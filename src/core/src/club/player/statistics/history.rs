use super::types::{PlayerStatistics, TeamInfo};
use crate::league::Season;
use chrono::NaiveDate;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct PlayerStatisticsHistory {
    /// Frozen history from completed seasons. Never modified after write.
    pub items: Vec<PlayerStatisticsHistoryItem>,
    /// Current-season entries. Append-only during season, drained at season end.
    pub current: Vec<CurrentSeasonEntry>,
    next_seq: u32,
}

#[derive(Debug, Clone)]
pub struct PlayerStatisticsHistoryItem {
    pub season: Season,
    pub team_name: String,
    pub team_slug: String,
    pub team_reputation: u16,
    pub league_name: String,
    pub league_slug: String,
    pub is_loan: bool,
    pub transfer_fee: Option<f64>,
    pub statistics: PlayerStatistics,
    pub seq_id: u32,
}

#[derive(Debug, Clone)]
pub struct CurrentSeasonEntry {
    pub team_name: String,
    pub team_slug: String,
    pub team_reputation: u16,
    pub league_name: String,
    pub league_slug: String,
    pub is_loan: bool,
    pub transfer_fee: Option<f64>,
    pub statistics: PlayerStatistics,
    pub joined_date: NaiveDate,
    /// Set when the player leaves (loan/transfer out). Used to calculate
    /// actual time at the club — without this, pre-loan stints look like
    /// full-season stays because joined_date is the season start.
    pub departed_date: Option<NaiveDate>,
    pub seq_id: u32,
}

impl Default for PlayerStatisticsHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerStatisticsHistory {
    pub fn new() -> Self {
        PlayerStatisticsHistory {
            items: Vec::new(),
            current: Vec::new(),
            next_seq: 0,
        }
    }

    /// Build a history pre-populated with frozen items from external data
    /// (e.g. the database loader). Caller is responsible for assigning
    /// `seq_id` in chronological order; `next_seq` is seeded past the max
    /// so future runtime events continue from a unique value.
    pub fn from_items(items: Vec<PlayerStatisticsHistoryItem>) -> Self {
        let next_seq = items.iter().map(|i| i.seq_id + 1).max().unwrap_or(0);
        PlayerStatisticsHistory {
            items,
            current: Vec::new(),
            next_seq,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.current.is_empty()
    }

    /// True when no current-season entry has been seeded yet, regardless of
    /// whether prior-season `items` are populated. Used by the simulator's
    /// initial-team seeding pass — players hydrated with historical `items`
    /// still need their current club seeded into `current`.
    pub fn needs_current_season_seed(&self) -> bool {
        self.current.is_empty()
    }

    fn next_seq(&mut self) -> u32 {
        let s = self.next_seq;
        self.next_seq += 1;
        s
    }

    /// Mark the most recent entry for a team as departed on the given date.
    fn mark_departed(&mut self, team_slug: &str, is_loan: bool, date: NaiveDate) {
        if let Some(entry) = self
            .current
            .iter_mut()
            .rev()
            .find(|e| e.team_slug == team_slug && e.is_loan == is_loan)
        {
            entry.departed_date = Some(date);
        }
    }

    /// Add or update a current-season entry for (team_slug, is_loan).
    /// If an entry already exists: replace stats (if new has games, or old has none), keep fee.
    /// If no entry exists: push new row.
    fn upsert_current(
        &mut self,
        team: &TeamInfo,
        stats: PlayerStatistics,
        is_loan: bool,
        fee: Option<f64>,
        date: NaiveDate,
    ) {
        if let Some(entry) = self
            .current
            .iter_mut()
            .rev()
            .find(|e| e.team_slug == team.slug && e.is_loan == is_loan)
        {
            // Update stats/fee on existing entry. Never change seq_id.
            if stats.total_games() > 0 {
                if entry.statistics.total_games() == 0 {
                    entry.statistics = stats;
                } else {
                    entry.statistics.merge_from(&stats);
                }
            }
            if fee.is_some() && entry.transfer_fee.is_none() {
                entry.transfer_fee = fee;
            }
        } else {
            self.push_new_entry(team, stats, is_loan, fee, date);
        }
    }

    /// Always create a new entry — never merge with an existing one.
    /// Used for destination clubs on transfers/loans so each stint is a
    /// separate record and the initial entry is never overridden.
    fn push_new_entry(
        &mut self,
        team: &TeamInfo,
        stats: PlayerStatistics,
        is_loan: bool,
        fee: Option<f64>,
        date: NaiveDate,
    ) {
        let seq = self.next_seq();
        self.current.push(CurrentSeasonEntry {
            team_name: team.name.clone(),
            team_slug: team.slug.clone(),
            team_reputation: team.reputation,
            league_name: team.league_name.clone(),
            league_slug: team.league_slug.clone(),
            is_loan,
            transfer_fee: fee,
            statistics: stats,
            joined_date: date,
            departed_date: None,
            seq_id: seq,
        });
    }

    /// Freeze entries from previous seasons into `items` before a manual action.
    /// When a user does a manual loan/transfer before the country's season-end
    /// snapshot has run, `current` may still hold entries from the prior season.
    /// Without flushing, `upsert_current` would reuse those old entries, merging
    /// stats from different seasons into one entry and losing history.
    fn flush_stale_entries(&mut self, current_date: NaiveDate) {
        let current_season = Season::from_date(current_date);

        let mut stale = Vec::new();
        self.current.retain(|e| {
            let entry_season = Season::from_date(e.joined_date);
            if entry_season.start_year < current_season.start_year {
                stale.push(e.clone());
                false
            } else {
                true
            }
        });

        let is_first_season = self.items.is_empty();
        let first_seq = stale.iter().map(|e| e.seq_id).min();

        // Years where another stale entry has real content (loan or
        // otherwise). Used for the sole-record carve-out so a U18..U23
        // player's only 0-game alias row for a season isn't dropped as
        // a trivial stint.
        let years_with_any_content: HashSet<u16> = stale
            .iter()
            .filter(|e| e.statistics.total_games() > 0 || e.transfer_fee.is_some())
            .map(|e| Season::from_date(e.joined_date).start_year)
            .collect();

        for entry in stale {
            let entry_season = Season::from_date(entry.joined_date);
            let entry_year = entry_season.start_year;
            let season_end = entry_season.end_date();

            let games = entry.statistics.total_games();
            let has_fee = entry.transfer_fee.is_some();
            let is_initial_record = is_first_season && first_seq == Some(entry.seq_id);
            let stale_loan_seed = entry.is_loan && games == 0 && !has_fee;

            let end_date = entry.departed_date.unwrap_or(season_end);
            let days_at_club = (end_date - entry.joined_date).num_days().max(0);
            let season_days = (season_end - entry_season.start_date()).num_days().max(1);
            let time_pct = (days_at_club as f64 / season_days as f64) * 100.0;
            let trivial_stint = games == 0 && !has_fee && time_pct < 45.0;

            let has_any_content_for_season = years_with_any_content.contains(&entry_year)
                || self.items.iter().any(|i| {
                    i.season.start_year == entry_year
                        && (i.statistics.total_games() > 0 || i.transfer_fee.is_some())
                });
            let sole_season_record =
                !entry.is_loan && games == 0 && !has_fee && !has_any_content_for_season;

            if is_initial_record || sole_season_record || (!stale_loan_seed && !trivial_stint) {
                let mut stats = entry.statistics;
                stats.played += stats.played_subs;
                stats.played_subs = 0;

                self.items.push(PlayerStatisticsHistoryItem {
                    season: entry_season,
                    team_name: entry.team_name,
                    team_slug: entry.team_slug,
                    team_reputation: entry.team_reputation,
                    league_name: entry.league_name,
                    league_slug: entry.league_slug,
                    is_loan: entry.is_loan,
                    transfer_fee: entry.transfer_fee,
                    statistics: stats,
                    seq_id: entry.seq_id,
                });
            }
        }
    }

    // ── Mid-season events ─────────────────────────────────
    //
    // The current club always exists in `current` (created at season end or first event).
    // Mid-season events just save stats on existing entry + add destination.

    pub fn record_transfer(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        fee: f64,
        date: NaiveDate,
    ) {
        self.upsert_current(from, old_stats, false, None, date);
        self.mark_departed(&from.slug, false, date);
        self.push_new_entry(to, PlayerStatistics::default(), false, Some(fee), date);
    }

    /// Player reassigned across teams of the same club (Main ↔ B / Second /
    /// Reserve / youth). Mirrors `record_transfer` but carries no fee, so
    /// the destination row doesn't render as "Free" — this isn't a market
    /// move.
    ///
    /// `from_senior` / `to_senior` gate per-side writes so non-senior
    /// squads (Reserve, U18..U23) never appear in career history. A
    /// promotion U21 → Main writes only the Main row; a demotion
    /// Main → U21 closes the Main spell; a youth-to-youth move writes
    /// nothing.
    ///
    /// When the destination is a senior team the player has already had a
    /// spell at this season (e.g. Main → U21 → Main bouncing), we
    /// reactivate the existing departed entry instead of creating a fresh
    /// row. Without this, the same team accumulates one entry per
    /// promotion cycle and the player history shows duplicate rows for
    /// the same season.
    pub fn record_intra_club_move(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        from_senior: bool,
        to_senior: bool,
        date: NaiveDate,
    ) {
        if from_senior {
            self.upsert_current(from, old_stats, false, None, date);
            self.mark_departed(&from.slug, false, date);
        }
        if to_senior {
            // Reactivate an existing senior entry for this team if one is
            // already present (departed or active) — otherwise push a fresh
            // row. Reactivation prevents the duplicate-row pattern when a
            // player bounces between Main and a non-senior squad inside the
            // same season.
            if let Some(existing) = self
                .current
                .iter_mut()
                .rev()
                .find(|e| e.team_slug == to.slug && !e.is_loan)
            {
                existing.departed_date = None;
            } else {
                self.push_new_entry(to, PlayerStatistics::default(), false, None, date);
            }
        }
    }

    pub fn record_loan(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        loan_fee: f64,
        date: NaiveDate,
    ) {
        self.upsert_current(from, old_stats, false, None, date);
        self.mark_departed(&from.slug, false, date);
        self.push_new_entry(to, PlayerStatistics::default(), true, Some(loan_fee), date);
    }

    pub fn record_loan_return(
        &mut self,
        remaining_stats: PlayerStatistics,
        borrowing: &TeamInfo,
        parent: &TeamInfo,
        date: NaiveDate,
    ) {
        self.upsert_current(borrowing, remaining_stats, true, None, date);

        // Mark loan entry as departed — the player has returned.
        // This prevents view_items from applying live_stats (parent club stats)
        // to the loan entry, which would show wrong stats for the loan row.
        self.mark_departed(&borrowing.slug, true, date);

        // Clean up stale loan entries: after a loan return, any loan entry
        // with 0 games and no fee is a leftover seed from season-end processing.
        // Keeping it would create phantom history entries in the next season.
        self.current.retain(|e| {
            !(e.is_loan && e.statistics.total_games() == 0 && e.transfer_fee.is_none())
        });

        // Clear departed_date on parent entry — the player is back
        if let Some(parent_entry) = self
            .current
            .iter_mut()
            .rev()
            .find(|e| !e.is_loan && e.departed_date.is_some())
        {
            parent_entry.departed_date = None;
            // Reset joined_date to return date for post-loan time calculation
            if parent_entry.statistics.total_games() == 0 && parent_entry.transfer_fee.is_none() {
                parent_entry.joined_date = date;
            }
        } else if !self.current.iter().any(|e| !e.is_loan) {
            // No parent entry exists — happens when season-end snapshot drained
            // current before the loan return ran. Create one so the parent club
            // has a current-season entry and view_items can show live stats.
            self.push_new_entry(parent, PlayerStatistics::default(), false, None, date);
        }
    }

    pub fn record_cancel_loan(
        &mut self,
        old_stats: PlayerStatistics,
        borrowing: &TeamInfo,
        parent: &TeamInfo,
        _is_loan: bool,
        date: NaiveDate,
    ) {
        self.upsert_current(borrowing, old_stats, true, None, date);

        // Mark loan entry as departed
        self.mark_departed(&borrowing.slug, true, date);

        // Mirror record_loan_return cleanup: clear parent departed_date
        // so the parent entry correctly represents the post-return stint
        if let Some(parent_entry) = self
            .current
            .iter_mut()
            .rev()
            .find(|e| !e.is_loan && e.departed_date.is_some())
        {
            parent_entry.departed_date = None;
            if parent_entry.statistics.total_games() == 0 && parent_entry.transfer_fee.is_none() {
                parent_entry.joined_date = date;
            }
        } else if !self.current.iter().any(|e| !e.is_loan) {
            // No parent entry exists — create one (same fix as record_loan_return)
            self.push_new_entry(parent, PlayerStatistics::default(), false, None, date);
        }
    }

    /// Record a release to the free-agent pool. Snapshots in-flight stats
    /// onto the source club's current-season entry and marks it as
    /// departed. Unlike `record_transfer`, no destination is written —
    /// the player will sit unaffiliated until a club picks them up. The
    /// "Free Agent" string belongs on the country-level market log only,
    /// not in a player's career history, so we never push a synthetic row
    /// for it here.
    pub fn record_release(
        &mut self,
        last_stats: PlayerStatistics,
        from: &TeamInfo,
        date: NaiveDate,
    ) {
        self.upsert_current(from, last_stats, false, None, date);
        self.mark_departed(&from.slug, false, date);
    }

    /// Record a free-agent signing. Unlike `record_departure_transfer`,
    /// there is no source club — only the destination — so we just freeze
    /// any prior-season entries and push one fresh row for the new club.
    /// `last_stats` is the player's pre-signing live `PlayerStatistics`,
    /// snapshotted onto the most recent unfinalised entry (e.g. a former
    /// club spell that hasn't been frozen yet) so its games aren't lost.
    pub fn record_free_agent_signing(
        &mut self,
        last_stats: PlayerStatistics,
        to: &TeamInfo,
        date: NaiveDate,
    ) {
        self.flush_stale_entries(date);
        if last_stats.total_games() > 0 {
            if let Some(entry) = self
                .current
                .iter_mut()
                .rev()
                .find(|e| e.statistics.total_games() == 0)
            {
                entry.statistics = last_stats;
            }
        }
        self.push_new_entry(to, PlayerStatistics::default(), false, Some(0.0), date);
    }

    pub fn record_departure_transfer(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        fee: Option<f64>,
        is_loan: bool,
        date: NaiveDate,
    ) {
        self.flush_stale_entries(date);
        self.upsert_current(from, old_stats, is_loan, None, date);
        self.mark_departed(&from.slug, is_loan, date);
        self.push_new_entry(to, PlayerStatistics::default(), false, fee, date);
    }

    pub fn record_departure_loan(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        _parent: &TeamInfo,
        to: &TeamInfo,
        _is_loan: bool,
        date: NaiveDate,
    ) {
        self.flush_stale_entries(date);
        self.upsert_current(from, old_stats, false, None, date);
        self.mark_departed(&from.slug, false, date);
        // Use Some(0.0) for fee so the loan entry survives stale_loan_seed filter
        // even with 0 games (consistent with record_loan which always sets Some(fee))
        self.push_new_entry(to, PlayerStatistics::default(), true, Some(0.0), date);
    }

    /// Drain any `current` entry whose `joined_date` season is earlier
    /// than the season we're about to close, pushing each under its
    /// OWN season label rather than letting it leak into the current
    /// season's drain. This recovers from missed snapshots — without
    /// it, a re-seed left over from a year whose snapshot never fired
    /// would silently collapse into the next season-end row.
    ///
    /// After flushing the entries, fill any gap years between the most
    /// recently flushed season and `target_season_start - 1` with an
    /// empty placeholder row for `fallback_team`. Aliased youth squads
    /// (U18..U23, Reserve) always carry the parent-club Main identity,
    /// so the gap-fill correctly continues the "career home" thread
    /// for a player who quietly spent multiple missed seasons in a
    /// non-owning team.
    fn flush_prior_season_seeds(
        &mut self,
        target_season_start: u16,
        fallback_team: &TeamInfo,
        fallback_is_loan: bool,
    ) {
        // Only consider entries that are *empty* re-seed leftovers — no
        // games, no fee, not yet departed. Mid-season actions
        // (`record_loan_return`, `record_intra_club_move`, etc.) can
        // legitimately create entries whose `joined_date` falls in an
        // earlier *calendar* season window (e.g. a loan return in June
        // sits in the season-ending-May window per `Season::from_date`)
        // even though their stats belong to the season we're now
        // closing. Flushing those would lose data — exactly the
        // regression the `lifecycle_two_consecutive_loans_no_phantom`
        // test guards against.
        let mut stale: Vec<CurrentSeasonEntry> = Vec::new();
        self.current.retain(|e| {
            let entry_year = Season::from_date(e.joined_date).start_year;
            let is_empty_seed = e.statistics.total_games() == 0
                && e.transfer_fee.is_none()
                && e.departed_date.is_none();
            if entry_year < target_season_start && is_empty_seed {
                stale.push(e.clone());
                false
            } else {
                true
            }
        });
        if stale.is_empty() {
            return;
        }

        let is_first_season = self.items.is_empty();
        let first_seq = stale.iter().map(|e| e.seq_id).min();

        // Precompute the set of season-years that have at least one
        // stale entry with real content (games or a transfer fee, loan
        // or otherwise). Combined with `self.items` checks below, this
        // drives the "sole season record" carve-out so a quiet U18..U23
        // season's single 0-game alias row isn't lost to the trivial-
        // stint filter when its seed date sits late in the season.
        let years_with_any_content: HashSet<u16> = stale
            .iter()
            .filter(|e| e.statistics.total_games() > 0 || e.transfer_fee.is_some())
            .map(|e| Season::from_date(e.joined_date).start_year)
            .collect();

        // Track the latest season the player demonstrably stayed at a
        // non-loan club; used to fill gap years for an unbroken career
        // thread (U18/U21 alias case). Initialised from frozen items
        // so a missed-snapshot recovery picks up where the last
        // recorded season left off.
        let mut last_thread_year: Option<u16> = self
            .items
            .iter()
            .filter(|i| !i.is_loan && i.team_slug == fallback_team.slug)
            .map(|i| i.season.start_year)
            .max();

        for entry in stale {
            let entry_season = Season::from_date(entry.joined_date);
            let entry_year = entry_season.start_year;

            // Already-frozen for this season? Merge stats/fee instead
            // of re-pushing — same-season duplicates are collapsed by
            // merge_same_season_team_items downstream, but we'd rather
            // not create the duplicate at all when avoidable.
            let already_frozen = self.items.iter().any(|i| {
                i.season.start_year == entry_year
                    && i.team_slug == entry.team_slug
                    && i.is_loan == entry.is_loan
            });

            let games = entry.statistics.total_games();
            let has_fee = entry.transfer_fee.is_some();
            let is_initial_record = is_first_season && first_seq == Some(entry.seq_id);
            let stale_loan_seed = entry.is_loan && games == 0 && !has_fee;

            let season_end = entry_season.end_date();
            let end_date = entry.departed_date.unwrap_or(season_end);
            let days_at_club = (end_date - entry.joined_date).num_days().max(0);
            let season_days = (season_end - entry_season.start_date()).num_days().max(1);
            let time_pct = (days_at_club as f64 / season_days as f64) * 100.0;
            let trivial_stint = games == 0 && !has_fee && time_pct < 45.0;

            // Sole-record exception (see `record_season_end` drain for
            // rationale): if no other entry for this season — stale OR
            // already-frozen — has real content, this 0-game-no-fee row
            // is the player's only record of that season and must
            // survive even when the seed date pushes time_pct below the
            // trivial-stint threshold.
            let has_any_content_for_season = years_with_any_content.contains(&entry_year)
                || self.items.iter().any(|i| {
                    i.season.start_year == entry_year
                        && (i.statistics.total_games() > 0 || i.transfer_fee.is_some())
                });
            let sole_season_record =
                !entry.is_loan && games == 0 && !has_fee && !has_any_content_for_season;

            let keep =
                is_initial_record || sole_season_record || (!stale_loan_seed && !trivial_stint);
            if !keep {
                continue;
            }

            if already_frozen {
                if games > 0 {
                    if let Some(existing) = self.items.iter_mut().rev().find(|i| {
                        i.season.start_year == entry_year
                            && i.team_slug == entry.team_slug
                            && i.is_loan == entry.is_loan
                    }) {
                        let mut remaining = entry.statistics.clone();
                        remaining.played += remaining.played_subs;
                        remaining.played_subs = 0;
                        existing.statistics.merge_from(&remaining);
                    }
                }
                if entry.transfer_fee.is_some() {
                    if let Some(existing) = self.items.iter_mut().rev().find(|i| {
                        i.season.start_year == entry_year
                            && i.team_slug == entry.team_slug
                            && i.is_loan == entry.is_loan
                            && i.transfer_fee.is_none()
                    }) {
                        existing.transfer_fee = entry.transfer_fee;
                    }
                }
            } else {
                let mut stats = entry.statistics.clone();
                stats.played += stats.played_subs;
                stats.played_subs = 0;
                self.items.push(PlayerStatisticsHistoryItem {
                    season: entry_season,
                    team_name: entry.team_name.clone(),
                    team_slug: entry.team_slug.clone(),
                    team_reputation: entry.team_reputation,
                    league_name: entry.league_name.clone(),
                    league_slug: entry.league_slug.clone(),
                    is_loan: entry.is_loan,
                    transfer_fee: entry.transfer_fee,
                    statistics: stats,
                    seq_id: entry.seq_id,
                });
            }

            // Only non-loan rows continue the "career home" thread. A
            // loan spell sits alongside the parent-club row; it
            // doesn't replace the parent club for gap-fill purposes.
            if !entry.is_loan {
                last_thread_year = Some(
                    last_thread_year
                        .map(|y| y.max(entry_year))
                        .unwrap_or(entry_year),
                );
            }
        }

        // Gap-fill: insert an empty placeholder row for every year
        // between (last_thread_year + 1) and (target_season_start - 1)
        // that has no non-loan row yet. Uses `fallback_team` so the
        // U18/U21 alias's parent-club Main identity continues
        // uninterrupted. Skip the fill entirely when there's no prior
        // thread year (first-time seed; the regular drain handles it).
        if let Some(start) = last_thread_year {
            let fill_from = start.saturating_add(1);
            for year in fill_from..target_season_start {
                let already_present = self
                    .items
                    .iter()
                    .any(|i| i.season.start_year == year && !i.is_loan);
                if already_present {
                    continue;
                }
                let seq = self.next_seq();
                self.items.push(PlayerStatisticsHistoryItem {
                    season: Season::new(year),
                    team_name: fallback_team.name.clone(),
                    team_slug: fallback_team.slug.clone(),
                    team_reputation: fallback_team.reputation,
                    league_name: fallback_team.league_name.clone(),
                    league_slug: fallback_team.league_slug.clone(),
                    is_loan: fallback_is_loan,
                    transfer_fee: None,
                    statistics: PlayerStatistics::default(),
                    seq_id: seq,
                });
            }
        }
    }

    // ── Season end: drain current → frozen items, then seed new season ──

    pub fn record_season_end(
        &mut self,
        season: Season,
        current_stats: PlayerStatistics,
        team: &TeamInfo,
        is_loan: bool,
        last_transfer_date: Option<NaiveDate>,
    ) {
        // Robustness: drain any *stale* seed entries — entries whose
        // `joined_date` falls in a season earlier than the one we're
        // closing now. They appear when a previous season-end snapshot
        // was skipped (e.g. a multi-league country where every league
        // briefly fails the `new_season_started` gate, a regen failure,
        // or simply a date computation that resolves the wrong
        // `ended_season`). Without this flush, the next-year drain
        // re-stamps the leftover seed under the current season label
        // and the missed year vanishes — exactly the user-reported
        // "missing 2026/27" pattern. For a U18/U21 player aliased to
        // their parent club's Main team, the gap-fill below also
        // inserts an empty Main row for each year that was skipped, so
        // the career table always has one row per season the player
        // existed at the club.
        self.flush_prior_season_seeds(season.start_year, team, is_loan);

        // Guard: if this season was already frozen (multi-league country where
        // different leagues start new seasons on different dates, or cross-country
        // loan where both countries snapshot the same player), avoid duplicates.
        // Merge any remaining stats into the existing frozen entry and re-seed.
        if self
            .items
            .iter()
            .any(|i| i.season.start_year == season.start_year)
        {
            // Merge remaining stats (games played between first and second snapshot)
            if current_stats.total_games() > 0 {
                if let Some(existing) = self.items.iter_mut().rev().find(|i| {
                    i.season.start_year == season.start_year
                        && i.team_slug == team.slug
                        && i.is_loan == is_loan
                }) {
                    let mut remaining = current_stats;
                    remaining.played += remaining.played_subs;
                    remaining.played_subs = 0;
                    existing.statistics.merge_from(&remaining);
                }
            }
            // Before clearing, freeze any current entries that carry meaningful
            // data (transfer fees or games) but don't yet exist in frozen items.
            // Without this, a cross-country season-end can silently drop entries
            // created by mid-season transfers (e.g. transfer fee lost).
            let entries = std::mem::take(&mut self.current);
            for entry in entries {
                let dominated_by_frozen = self.items.iter().any(|i| {
                    i.season.start_year == season.start_year
                        && i.team_slug == entry.team_slug
                        && i.is_loan == entry.is_loan
                });
                if dominated_by_frozen {
                    if entry.statistics.total_games() > 0 {
                        if let Some(existing) = self.items.iter_mut().rev().find(|i| {
                            i.season.start_year == season.start_year
                                && i.team_slug == entry.team_slug
                                && i.is_loan == entry.is_loan
                        }) {
                            let mut remaining = entry.statistics;
                            remaining.played += remaining.played_subs;
                            remaining.played_subs = 0;
                            existing.statistics.merge_from(&remaining);
                        }
                    }
                    if entry.transfer_fee.is_some() {
                        if let Some(existing) = self.items.iter_mut().rev().find(|i| {
                            i.season.start_year == season.start_year
                                && i.team_slug == entry.team_slug
                                && i.is_loan == entry.is_loan
                                && i.transfer_fee.is_none()
                        }) {
                            existing.transfer_fee = entry.transfer_fee;
                        }
                    }
                    continue;
                }
                let games = entry.statistics.total_games();
                let has_fee = entry.transfer_fee.is_some();
                if games > 0 || has_fee {
                    let mut stats = entry.statistics;
                    stats.played += stats.played_subs;
                    stats.played_subs = 0;
                    self.items.push(PlayerStatisticsHistoryItem {
                        season: season.clone(),
                        team_name: entry.team_name,
                        team_slug: entry.team_slug,
                        team_reputation: entry.team_reputation,
                        league_name: entry.league_name,
                        league_slug: entry.league_slug,
                        is_loan: entry.is_loan,
                        transfer_fee: entry.transfer_fee,
                        statistics: stats,
                        seq_id: entry.seq_id,
                    });
                }
            }
            // Same merge pass as the regular drain branch — see comment
            // there for rationale.
            merge_same_season_team_items(&mut self.items, season.start_year);

            // Re-seed for next season
            let new_season_start = Season::new(season.start_year + 1).start_date();
            self.upsert_current(
                team,
                PlayerStatistics::default(),
                is_loan,
                None,
                new_season_start,
            );
            return;
        }

        // When the player has no tracked entry for this team (e.g. returned from
        // loan mid-season), use last_transfer_date as joined_date so the trivial
        // stint filter can accurately measure time at this club.
        let has_existing = self
            .current
            .iter()
            .any(|e| e.team_slug == team.slug && e.is_loan == is_loan);
        let join_date = if has_existing {
            season.start_date()
        } else {
            last_transfer_date.unwrap_or(season.start_date())
        };

        // Apply live stats to the current club entry
        self.upsert_current(team, current_stats, is_loan, None, join_date);

        // Drain everything into frozen items
        let season_end = season.end_date();
        let entries = std::mem::take(&mut self.current);

        // The very first career record (no prior history) is always kept,
        // even with 0 games — it's the player's starting club.
        let is_first_season = self.items.is_empty();
        let first_seq = entries.iter().map(|e| e.seq_id).min();

        // Whether ANY entry in this drain has real content (games or a
        // transfer fee), loan or otherwise. Used to decide whether a
        // 0-game-no-fee row is the player's *only* record of the season.
        //
        // Per spec: every season the player existed at the club must
        // surface at least one row. A U18..U23 player who never gets a
        // senior callup has exactly one Main-alias row with no fee and
        // no games — that row must survive even when joined_date pushes
        // time_pct under the trivial-stint threshold (typical for a
        // late-in-season seed when the simulator starts mid-real-time).
        //
        // A loan row already represents the season, so the
        // post-loan-return parent-club row with 0 apps is still allowed
        // to fall through the trivial-stint filter — matching
        // `loan_return_no_phantom_parent_entry`'s expectation.
        let has_any_content = entries
            .iter()
            .any(|e| e.statistics.total_games() > 0 || e.transfer_fee.is_some());

        for entry in entries {
            let games = entry.statistics.total_games();
            let end_date = entry.departed_date.unwrap_or(season_end);
            let days_at_club = (end_date - entry.joined_date).num_days().max(0);
            let season_days = (season_end - season.start_date()).num_days().max(1);
            let time_pct = (days_at_club as f64 / season_days as f64) * 100.0;

            // Drop entries where the player barely stayed and never played:
            // - Loan entries with 0 games and no fee are stale seeds (phantom entries)
            // - Any entry with 0 games and no fee that covers < 45% of the season is noise
            //   (e.g. returned from loan near season end, 0 apps at parent club)
            // Always keep: entries with games, entries with transfer fees,
            // entries where the player was at the club for a meaningful portion of the season,
            // or the player's first-ever career record (initial club).
            //
            // Sole-record exception: when the drain has no other entry
            // with real content (games or fee, loan or otherwise), this
            // 0-game-no-fee row is the player's only record of the
            // season — typical for a U18..U23 player who never gets a
            // senior callup. The seed's joined_date often sits well
            // after the season start (game-start mid-real-time, youth
            // intake), which would otherwise trip the trivial-stint
            // filter and lose the entire season from career history.
            //
            // When a loan or transfer row already represents the season,
            // a 0-app parent-club row falls through to the trivial-stint
            // filter as before.
            let has_fee = entry.transfer_fee.is_some();
            let is_initial_record = is_first_season && first_seq == Some(entry.seq_id);
            let trivial_stint = games == 0 && !has_fee && time_pct < 45.0;
            let stale_loan_seed = entry.is_loan && games == 0 && !has_fee;
            let sole_season_record = !entry.is_loan && games == 0 && !has_fee && !has_any_content;

            let keep =
                is_initial_record || sole_season_record || (!stale_loan_seed && !trivial_stint);

            if keep {
                let mut stats = entry.statistics;
                stats.played += stats.played_subs;
                stats.played_subs = 0;

                self.items.push(PlayerStatisticsHistoryItem {
                    season: season.clone(),
                    team_name: entry.team_name,
                    team_slug: entry.team_slug,
                    team_reputation: entry.team_reputation,
                    league_name: entry.league_name,
                    league_slug: entry.league_slug,
                    is_loan: entry.is_loan,
                    transfer_fee: entry.transfer_fee,
                    statistics: stats,
                    seq_id: entry.seq_id,
                });
            }
        }

        // Collapse multiple same-team spells inside this season into one
        // row (e.g. Main → B → Main bouncing produces a single Main row
        // with summed stats, the same row a single uninterrupted spell
        // would have produced). Any phantom 0-game spells with no fee
        // are dropped during the merge.
        merge_same_season_team_items(&mut self.items, season.start_year);

        // Seed the new season with an empty entry for the current club
        let new_season_start = Season::new(season.start_year + 1).start_date();
        self.upsert_current(
            team,
            PlayerStatistics::default(),
            is_loan,
            None,
            new_season_start,
        );
    }

    // ── Initial seeding ───────────────────────────────────

    /// Seed the player's history with their initial team when the game starts.
    /// Seeds whenever there is no current-season entry — prior-season `items`
    /// loaded from the database still need a current-season row appended.
    /// `is_loan` flags the stint as a loan so the history UI labels it.
    pub fn seed_initial_team(&mut self, team: &TeamInfo, date: NaiveDate, is_loan: bool) {
        if self.current.is_empty() {
            self.upsert_current(team, PlayerStatistics::default(), is_loan, None, date);
        }
    }

    // ── View: pure read, no mutation ────────────────────────

    /// Returns all history (past seasons) + current season entries,
    /// sorted by season desc, then seq_id desc.
    ///
    /// `live_stats` — if provided, replaces the stats on the active current-season
    /// entry (the one without `departed_date`). This bridges the gap between
    /// `player.statistics` (continuously updated by matches) and the snapshot
    /// stored in `current` (only updated at event boundaries).
    ///
    /// `current_date` — today's game date. Used to label *active* current-season
    /// entries with the correct season. Without this, the season label would
    /// follow the entry's `joined_date`, which is set at the previous
    /// season-end snapshot and goes stale if the next snapshot was delayed
    /// (e.g. the league's new-season schedule hasn't been generated yet on
    /// the date the page is rendered).
    pub fn view_items(
        &self,
        live_stats: Option<&PlayerStatistics>,
        current_date: NaiveDate,
    ) -> Vec<PlayerStatisticsHistoryItem> {
        let today_season = Season::from_date(current_date);

        let mut result: Vec<PlayerStatisticsHistoryItem> = self.items.clone();

        let is_first_season = self.items.is_empty();
        let first_seq = self.current.iter().map(|e| e.seq_id).min();

        for entry in &self.current {
            let is_active = entry.departed_date.is_none();

            // Skip departed entries with 0 games and no transfer fee —
            // same logic as the trivial stint filter at season end,
            // so the UI doesn't show empty rows mid-season.
            // Exception: never skip the initial record (first-ever career entry).
            let is_initial_record = is_first_season && first_seq == Some(entry.seq_id);
            if !is_active
                && !is_initial_record
                && entry.statistics.total_games() == 0
                && entry.transfer_fee.is_none()
            {
                continue;
            }

            let statistics = if is_active {
                if let Some(stats) = live_stats {
                    stats.clone()
                } else {
                    entry.statistics.clone()
                }
            } else {
                entry.statistics.clone()
            };

            // Active rows track the actual game date so the player page shows
            // "this is their current season" no matter how stale joined_date is.
            // Departed rows keep their joined_date season — that's the spell
            // they actually played, regardless of when we render the page.
            let row_season = if is_active {
                today_season.clone()
            } else {
                let joined_season = Season::from_date(entry.joined_date);
                if joined_season.start_year > today_season.start_year {
                    today_season.clone()
                } else {
                    joined_season
                }
            };

            result.push(PlayerStatisticsHistoryItem {
                season: row_season,
                team_name: entry.team_name.clone(),
                team_slug: entry.team_slug.clone(),
                team_reputation: entry.team_reputation,
                league_name: entry.league_name.clone(),
                league_slug: entry.league_slug.clone(),
                is_loan: entry.is_loan,
                transfer_fee: entry.transfer_fee,
                statistics,
                seq_id: entry.seq_id,
            });
        }

        // Defensive merge for the view: collapse multiple same-team rows
        // inside the same season. New data goes through the merge at
        // `record_season_end`, but older data already in `items` (from
        // before this fix) needs to be cleaned up at render time too.
        merge_same_season_team_view(&mut result);

        result.sort_by(|a, b| {
            b.season
                .start_year
                .cmp(&a.season.start_year)
                .then(b.seq_id.cmp(&a.seq_id))
        });

        // Only the most recent entry (max seq_id) shows subs separately as "played (subs)".
        // All previous entries sum played + played_subs into played.
        if let Some(max_seq) = result.iter().map(|i| i.seq_id).max() {
            for item in &mut result {
                if item.seq_id != max_seq && item.statistics.played_subs > 0 {
                    item.statistics.played += item.statistics.played_subs;
                    item.statistics.played_subs = 0;
                }
            }
        }

        result
    }

    /// Compute career totals from view items.
    pub fn career_totals(items: &[PlayerStatisticsHistoryItem]) -> PlayerStatistics {
        let mut totals = PlayerStatistics::default();
        for item in items {
            totals.merge_from(&item.statistics);
        }
        totals
    }

    /// Slug of the player's currently active club spell — the entry in
    /// `current` without a `departed_date`. Used to identify which past
    /// items belong to the *current* club for career-apps clauses.
    pub fn active_team_slug(&self) -> Option<&str> {
        self.current
            .iter()
            .find(|e| e.departed_date.is_none())
            .map(|e| e.team_slug.as_str())
    }

    /// League stats accumulated across every current-season spell, with
    /// the live counter standing in for the still-active spell. The live
    /// `player.statistics` field is per-spell and gets drained on every
    /// intra-club move (Main ↔ B / Second), so reading it directly hides
    /// games the player accumulated before the move. This blends the
    /// drained spells (preserved on `current`) with the live counter so
    /// the player profile shows the full season tally.
    pub fn current_season_stats(&self, live_stats: &PlayerStatistics) -> PlayerStatistics {
        let mut total = PlayerStatistics::default();
        let mut found_active = false;
        for entry in &self.current {
            if entry.departed_date.is_none() && !found_active {
                total.merge_from(live_stats);
                found_active = true;
            } else {
                total.merge_from(&entry.statistics);
            }
        }
        if !found_active {
            total.merge_from(live_stats);
        }
        total
    }

    /// Total competitive (league + cup) apps the player has logged for
    /// their current club across all spells: prior frozen seasons +
    /// current-season snapshot. `live_played` / `live_played_subs` come
    /// from `player.statistics` because the current-season `current`
    /// entry isn't updated until event boundaries.
    ///
    /// Used by `WageAfterReachingClubCareerLeagueGames` so the threshold
    /// counts a player's full club tenure, not just this season.
    pub fn current_club_career_apps(&self, live_played: u16, live_played_subs: u16) -> u32 {
        let slug = match self.active_team_slug() {
            Some(s) => s,
            None => return live_played as u32 + live_played_subs as u32,
        };
        let mut total: u32 = 0;
        // Prior seasons at this club (frozen items).
        for item in &self.items {
            if item.team_slug == slug {
                total = total
                    .saturating_add(item.statistics.played as u32)
                    .saturating_add(item.statistics.played_subs as u32);
            }
        }
        // Current-season at this club uses LIVE stats — the snapshot in
        // `current` isn't updated continuously.
        total = total
            .saturating_add(live_played as u32)
            .saturating_add(live_played_subs as u32);
        total
    }
}

/// Collapse multiple rows for the same `(team_slug, is_loan)` inside a
/// single season into one row. Used by `record_season_end` so a player
/// who bounced between Main and a non-senior squad — or had several
/// short spells at the same senior team in one season — ends up with
/// one row per team rather than a duplicate stack.
///
/// Stats are summed (`merge_from`), the first non-`None` `transfer_fee`
/// is preserved, the highest `seq_id` wins for ordering. Drop rules
/// (applied after the merge):
///
/// - Rows with games or a transfer fee always survive.
/// - A 0-game-no-fee row is dropped if another non-loan team in the
///   same season has games or a fee — this is the intra-club move
///   phantom case (e.g. a seeded Main entry alongside a B spell where
///   the player actually played).
/// - Otherwise a 0-game-no-fee row is kept: U18/U21 players who never
///   get a senior callup still need a Main row for the season, and a
///   parent-club row must coexist with a loan-out spell.
fn merge_same_season_team_items(items: &mut Vec<PlayerStatisticsHistoryItem>, season_year: u16) {
    let (in_season, mut other): (Vec<_>, Vec<_>) = items
        .drain(..)
        .partition(|i| i.season.start_year == season_year);

    let mut merged: Vec<PlayerStatisticsHistoryItem> = Vec::with_capacity(in_season.len());
    for entry in in_season {
        if let Some(target) = merged
            .iter_mut()
            .find(|m| m.team_slug == entry.team_slug && m.is_loan == entry.is_loan)
        {
            target.statistics.merge_from(&entry.statistics);
            if target.transfer_fee.is_none() {
                target.transfer_fee = entry.transfer_fee;
            }
            target.seq_id = target.seq_id.max(entry.seq_id);
            if target.team_reputation == 0 && entry.team_reputation > 0 {
                target.team_reputation = entry.team_reputation;
            }
            if target.league_name.is_empty() && !entry.league_name.is_empty() {
                target.league_name = entry.league_name;
                target.league_slug = entry.league_slug;
            }
        } else {
            merged.push(entry);
        }
    }

    let merged_snapshot = merged.clone();
    merged.retain(|i| {
        let has_content = i.statistics.total_games() > 0 || i.transfer_fee.is_some();
        if has_content {
            return true;
        }
        // Drop only when a sibling NON-LOAN team in this season has
        // real content — that's the intra-club bounce that left this
        // row as a phantom seed. Loan siblings don't trigger the drop:
        // the parent-club row must survive alongside the loan spell.
        let phantom_alongside_other_senior_spell = merged_snapshot.iter().any(|other| {
            !other.is_loan
                && other.team_slug != i.team_slug
                && (other.statistics.total_games() > 0 || other.transfer_fee.is_some())
        });
        !phantom_alongside_other_senior_spell
    });

    other.extend(merged);
    *items = other;
}

/// View-time variant of [`merge_same_season_team_items`]. Operates on a
/// flat list rather than mutating in place per season — runs once across
/// every season the view contains so legacy duplicate rows already
/// frozen in `items` (from before the season-end merge existed) are
/// collapsed at render time.
fn merge_same_season_team_view(items: &mut Vec<PlayerStatisticsHistoryItem>) {
    let mut merged: Vec<PlayerStatisticsHistoryItem> = Vec::with_capacity(items.len());
    for entry in items.drain(..) {
        if let Some(target) = merged.iter_mut().find(|m| {
            m.season.start_year == entry.season.start_year
                && m.team_slug == entry.team_slug
                && m.is_loan == entry.is_loan
        }) {
            target.statistics.merge_from(&entry.statistics);
            if target.transfer_fee.is_none() {
                target.transfer_fee = entry.transfer_fee;
            }
            target.seq_id = target.seq_id.max(entry.seq_id);
            if target.team_reputation == 0 && entry.team_reputation > 0 {
                target.team_reputation = entry.team_reputation;
            }
            if target.league_name.is_empty() && !entry.league_name.is_empty() {
                target.league_name = entry.league_name;
                target.league_slug = entry.league_slug;
            }
        } else {
            merged.push(entry);
        }
    }

    let merged_snapshot = merged.clone();

    merged.retain(|i| {
        let has_content = i.statistics.total_games() > 0 || i.transfer_fee.is_some();
        if has_content {
            return true;
        }
        // Mirrors the season-end merge: drop a 0-game-no-fee row only
        // when a sibling NON-LOAN team in the same season actually
        // played — that's the intra-club phantom seed pattern. A loan
        // sibling doesn't trigger the drop (parent-club row must
        // coexist with the loan row), and a quiet season with just the
        // Main row is the U18/U21 "career home" record.
        let phantom_alongside_other_senior_spell = merged_snapshot.iter().any(|other| {
            other.season.start_year == i.season.start_year
                && !other.is_loan
                && other.team_slug != i.team_slug
                && (other.statistics.total_games() > 0 || other.transfer_fee.is_some())
        });
        !phantom_alongside_other_senior_spell
    });

    *items = merged;
}

#[cfg(test)]
mod club_career_apps_tests {
    use super::*;
    use crate::league::Season;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn frozen(
        season_start: u16,
        slug: &str,
        played: u16,
        played_subs: u16,
    ) -> PlayerStatisticsHistoryItem {
        let mut stats = PlayerStatistics::default();
        stats.played = played;
        stats.played_subs = played_subs;
        PlayerStatisticsHistoryItem {
            season: Season::new(season_start),
            team_name: slug.to_string(),
            team_slug: slug.to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: false,
            transfer_fee: None,
            statistics: stats,
            seq_id: season_start as u32,
        }
    }

    fn current(slug: &str, played: u16) -> CurrentSeasonEntry {
        let mut stats = PlayerStatistics::default();
        stats.played = played;
        CurrentSeasonEntry {
            team_name: slug.to_string(),
            team_slug: slug.to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: false,
            transfer_fee: None,
            statistics: stats,
            joined_date: d(2025, 8, 1),
            departed_date: None,
            seq_id: 999,
        }
    }

    fn team(slug: &str) -> TeamInfo {
        TeamInfo {
            name: slug.to_string(),
            slug: slug.to_string(),
            reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
        }
    }

    #[test]
    fn club_career_apps_sums_history_at_current_club_plus_live() {
        // Player has 80 historical apps at "juventus" (split across two
        // earlier seasons) plus 20 live apps this season at the same
        // club. Helper should report 100 — exactly the threshold a
        // 100-app clause would trigger on.
        let mut hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2023, "juventus", 30, 5),
            frozen(2024, "juventus", 40, 5),
        ]);
        hist.current.push(current("juventus", 0));
        let apps = hist.current_club_career_apps(20, 0);
        assert_eq!(apps, 35 + 45 + 20);
    }

    #[test]
    fn club_career_apps_excludes_other_clubs() {
        // Apps at other clubs (a previous spell at "torino") must NOT
        // count toward "career apps at the CURRENT club".
        let mut hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2022, "torino", 60, 0),
            frozen(2023, "juventus", 25, 5),
        ]);
        hist.current.push(current("juventus", 0));
        let apps = hist.current_club_career_apps(10, 0);
        // 30 (Juventus historical) + 10 (live) = 40 — Torino's 60 ignored.
        assert_eq!(apps, 30 + 10);
    }

    #[test]
    fn club_career_apps_falls_back_to_live_only_with_no_active_spell() {
        // Edge case: empty current vec (mid-transfer). Helper falls back
        // to live stats only so we don't crash and don't claim apps
        // never logged.
        let hist = PlayerStatisticsHistory::new();
        let apps = hist.current_club_career_apps(5, 2);
        assert_eq!(apps, 7);
    }

    #[test]
    fn view_items_labels_active_entry_with_current_game_date_season() {
        // Repro for: player history page shows the latest row stuck on a
        // past season (e.g. "2026/27") even though the game date is well
        // into a later season ("2027/28"). This happens when the next
        // season-end snapshot has been delayed for that league, so the
        // current-season entry's `joined_date` is still anchored to the
        // previous season's start. The view must label the active row
        // using today's game date, not the stale joined_date.
        let mut hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2025, "spartak", 28, 0),
            frozen(2026, "spartak", 30, 0),
        ]);
        // Stale current entry: joined_date is from the 2026/27 season
        // start — the next snapshot never re-seeded it.
        hist.current.push(CurrentSeasonEntry {
            team_name: "spartak".to_string(),
            team_slug: "spartak".to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics::default(),
            joined_date: d(2026, 8, 1),
            departed_date: None,
            seq_id: 99,
        });

        let mut live = PlayerStatistics::default();
        live.played = 18;

        let view = hist.view_items(Some(&live), d(2028, 5, 14));

        // Frozen rows kept as-is; the active row must surface as 2027/28
        // (the season containing today's date), not duplicate 2026/27.
        assert!(
            view.iter()
                .any(|i| i.season.start_year == 2027 && i.team_slug == "spartak"),
            "expected a 2027/28 spartak row reflecting current game date,\
             got seasons: {:?}",
            view.iter().map(|i| i.season.start_year).collect::<Vec<_>>()
        );
        let active_row = view
            .iter()
            .find(|i| i.season.start_year == 2027 && i.team_slug == "spartak")
            .unwrap();
        assert_eq!(active_row.statistics.played, 18);
        // Frozen 2026/27 row must remain untouched (single row, original 30 apps).
        let frozen_2026: Vec<_> = view
            .iter()
            .filter(|i| i.season.start_year == 2026 && i.team_slug == "spartak")
            .collect();
        assert_eq!(frozen_2026.len(), 1);
        assert_eq!(frozen_2026[0].statistics.played, 30);
    }

    #[test]
    fn view_items_keeps_departed_entry_in_its_own_season() {
        // A mid-season transfer leaves a *departed* current entry behind
        // (e.g. spartak → cska in April 2028). The departed row must keep
        // its joined_date season label, not adopt today's season —
        // otherwise both spells would collapse into one row.
        let mut hist = PlayerStatisticsHistory::from_items(vec![frozen(2025, "spartak", 28, 0)]);

        let mut spartak_stats = PlayerStatistics::default();
        spartak_stats.played = 22;
        hist.current.push(CurrentSeasonEntry {
            team_name: "spartak".to_string(),
            team_slug: "spartak".to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: false,
            transfer_fee: None,
            statistics: spartak_stats,
            joined_date: d(2026, 8, 1),
            departed_date: Some(d(2027, 4, 1)),
            seq_id: 10,
        });
        hist.current.push(CurrentSeasonEntry {
            team_name: "cska".to_string(),
            team_slug: "cska".to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: false,
            transfer_fee: Some(1_000_000.0),
            statistics: PlayerStatistics::default(),
            joined_date: d(2027, 4, 1),
            departed_date: None,
            seq_id: 11,
        });

        let mut live = PlayerStatistics::default();
        live.played = 5;
        let view = hist.view_items(Some(&live), d(2028, 5, 14));

        let spartak_row = view
            .iter()
            .find(|i| i.team_slug == "spartak" && i.seq_id == 10)
            .unwrap();
        assert_eq!(spartak_row.season.start_year, 2026);
        assert_eq!(spartak_row.statistics.played, 22);

        let cska_row = view.iter().find(|i| i.team_slug == "cska").unwrap();
        assert_eq!(cska_row.season.start_year, 2027);
        assert_eq!(cska_row.statistics.played, 5);
    }

    #[test]
    fn duplicate_season_guard_merges_dominated_current_loan_stats() {
        let mut frozen_stats = PlayerStatistics::default();
        frozen_stats.played = 0;

        let mut hist = PlayerStatisticsHistory::from_items(vec![PlayerStatisticsHistoryItem {
            season: Season::new(2026),
            team_name: "zabbar".to_string(),
            team_slug: "zabbar".to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: true,
            transfer_fee: Some(0.0),
            statistics: frozen_stats,
            seq_id: 1,
        }]);

        let mut current_stats = PlayerStatistics::default();
        current_stats.played = 12;
        hist.current.push(CurrentSeasonEntry {
            team_name: "zabbar".to_string(),
            team_slug: "zabbar".to_string(),
            team_reputation: 5_000,
            league_name: String::new(),
            league_slug: String::new(),
            is_loan: true,
            transfer_fee: Some(0.0),
            statistics: current_stats,
            joined_date: d(2026, 9, 1),
            departed_date: Some(d(2027, 5, 31)),
            seq_id: 2,
        });

        hist.record_season_end(
            Season::new(2026),
            PlayerStatistics::default(),
            &team("zabbar"),
            true,
            None,
        );

        let loan_row = hist
            .items
            .iter()
            .find(|i| i.season.start_year == 2026 && i.team_slug == "zabbar" && i.is_loan)
            .unwrap();
        assert_eq!(loan_row.statistics.played, 12);
    }

    // ─────────────────────────────────────────────────────────────────
    // User-reported bug coverage:
    //   "I have a player with duplicated statistics in same season."
    //   Player bounced Main ↔ U21 with the squad rebalance pipeline,
    //   producing more than one Main row inside the same season because
    //   `record_intra_club_move` always pushed a fresh entry on a
    //   non-senior → senior promotion. The fix reactivates the
    //   pre-existing senior entry instead, AND collapses leftover
    //   duplicates at season end.
    // ─────────────────────────────────────────────────────────────────

    fn season_team(slug: &str) -> TeamInfo {
        TeamInfo {
            name: slug.to_string(),
            slug: slug.to_string(),
            reputation: 5_000,
            league_name: "League".to_string(),
            league_slug: "league".to_string(),
        }
    }

    #[test]
    fn intra_club_promotion_reuses_existing_senior_row() {
        // Player demoted Main → U21, then promoted U21 → Main inside
        // one season must not end up with two Main rows.
        let mut hist = PlayerStatisticsHistory::new();
        let main = season_team("napoli");
        let u21 = season_team("napoli-u21");

        // Seed Main entry as if the player started the season there.
        hist.seed_initial_team(&main, d(2025, 8, 1), false);

        // Pre-demotion stats accumulated at Main.
        let mut pre_demotion = PlayerStatistics::default();
        pre_demotion.played = 10;
        pre_demotion.goals = 2;

        // Mid-season demotion to U21 (from_senior=true, to_senior=false).
        hist.record_intra_club_move(pre_demotion, &main, &u21, true, false, d(2025, 12, 15));

        // Plays at U21 — those stats are intentionally not tracked.
        // Mid-season promotion back to Main.
        hist.record_intra_club_move(
            PlayerStatistics::default(),
            &u21,
            &main,
            false,
            true,
            d(2026, 2, 1),
        );

        // Exactly one ACTIVE Main entry survives in `current` — the
        // earlier one was reactivated, no duplicate was pushed.
        let main_entries: Vec<&CurrentSeasonEntry> = hist
            .current
            .iter()
            .filter(|e| e.team_slug == "napoli" && !e.is_loan)
            .collect();
        assert_eq!(
            main_entries.len(),
            1,
            "expected a single reactivated Main entry, got {}: {:?}",
            main_entries.len(),
            main_entries
                .iter()
                .map(|e| (e.joined_date, e.departed_date, e.statistics.played))
                .collect::<Vec<_>>()
        );
        assert!(
            main_entries[0].departed_date.is_none(),
            "the reactivated entry should be active (no departed_date)"
        );
        assert_eq!(
            main_entries[0].statistics.played, 10,
            "first-spell stats must survive the bounce"
        );
    }

    #[test]
    fn season_end_after_main_u21_main_bounce_emits_single_main_row() {
        // The end-to-end repro of the user's report. The fix must
        // produce exactly one Main row for the season once stats are
        // frozen, with the combined apps from both spells.
        let mut hist = PlayerStatisticsHistory::new();
        let main = season_team("napoli");
        let u21 = season_team("napoli-u21");

        hist.seed_initial_team(&main, d(2025, 8, 1), false);

        // First Main spell: 10 apps.
        let mut spell_one = PlayerStatistics::default();
        spell_one.played = 10;
        hist.record_intra_club_move(spell_one, &main, &u21, true, false, d(2025, 12, 15));

        // Promotion back to Main, then more games (8 apps in spell two).
        hist.record_intra_club_move(
            PlayerStatistics::default(),
            &u21,
            &main,
            false,
            true,
            d(2026, 2, 1),
        );

        let mut spell_two = PlayerStatistics::default();
        spell_two.played = 8;
        hist.record_season_end(Season::new(2025), spell_two, &main, false, None);

        let main_rows_2025: Vec<&PlayerStatisticsHistoryItem> = hist
            .items
            .iter()
            .filter(|i| i.season.start_year == 2025 && i.team_slug == "napoli")
            .collect();
        assert_eq!(
            main_rows_2025.len(),
            1,
            "expected exactly one Main row for 2025/26, got {}",
            main_rows_2025.len()
        );
        assert_eq!(
            main_rows_2025[0].statistics.played, 18,
            "combined apps from both Main spells must add up"
        );
    }

    #[test]
    fn season_end_drops_zero_app_intra_club_spell_when_other_team_has_games() {
        // Main(10) → B(0) → Main(8): the empty B spell should be
        // collapsed at season end, leaving Main(18).
        let mut hist = PlayerStatisticsHistory::new();
        let main = season_team("ural");
        let b = season_team("ural-b");

        hist.seed_initial_team(&main, d(2025, 8, 1), false);

        let mut spell_one = PlayerStatistics::default();
        spell_one.played = 10;
        hist.record_intra_club_move(spell_one, &main, &b, true, true, d(2025, 11, 1));

        // Player joined B but never played a match before going back.
        hist.record_intra_club_move(
            PlayerStatistics::default(),
            &b,
            &main,
            true,
            true,
            d(2025, 12, 1),
        );

        let mut spell_two = PlayerStatistics::default();
        spell_two.played = 8;
        hist.record_season_end(Season::new(2025), spell_two, &main, false, None);

        let rows: Vec<&PlayerStatisticsHistoryItem> = hist
            .items
            .iter()
            .filter(|i| i.season.start_year == 2025)
            .collect();
        assert_eq!(rows.len(), 1, "B(0) row should be collapsed");
        assert_eq!(rows[0].team_slug, "ural");
        assert_eq!(rows[0].statistics.played, 18);
    }

    #[test]
    fn non_senior_only_season_emits_main_row_with_zero_games() {
        // A player who spent the season entirely on U21 still gets a
        // Main-team row (with 0 games) — the user's rule is that
        // non-owning team players always show a Main row each season,
        // even when they didn't play any senior games.
        let mut hist = PlayerStatisticsHistory::new();
        let main = season_team("napoli");

        // Seeder aliased the U21 player to Main on game start.
        hist.seed_initial_team(&main, d(2025, 8, 1), false);

        // Non-senior season-end path (driven by `Player::on_non_senior_season_end`):
        // empty current_stats, Main team_info.
        hist.record_season_end(
            Season::new(2025),
            PlayerStatistics::default(),
            &main,
            false,
            None,
        );

        let napoli_2025: Vec<&PlayerStatisticsHistoryItem> = hist
            .items
            .iter()
            .filter(|i| i.season.start_year == 2025 && i.team_slug == "napoli")
            .collect();
        assert_eq!(
            napoli_2025.len(),
            1,
            "U21-only player must still get a Main row for the season"
        );
        assert_eq!(napoli_2025[0].statistics.played, 0);
    }

    #[test]
    fn non_senior_season_end_flushes_departed_main_spell() {
        // Player started at Main, was demoted to U21 mid-season, and
        // ends the season on U21. The Main spell is frozen into career
        // history with the games from the pre-demotion spell; the U21
        // spell does not appear under its own slug.
        let mut hist = PlayerStatisticsHistory::new();
        let main = season_team("zenit");

        hist.seed_initial_team(&main, d(2025, 8, 1), false);

        // Stats from the Main spell get committed via the intra-club
        // move (from_senior=true).
        let mut main_stats = PlayerStatistics::default();
        main_stats.played = 14;
        main_stats.goals = 4;
        hist.record_intra_club_move(
            main_stats,
            &main,
            &season_team("zenit-u21"),
            true,
            false,
            d(2025, 12, 15),
        );

        // Player is now on U21. Season ends through the non-senior
        // path: empty stats, Main team_info.
        hist.record_season_end(
            Season::new(2025),
            PlayerStatistics::default(),
            &main,
            false,
            None,
        );

        // Exactly the Main row survives — no U21 row, no duplicate.
        let zenit_rows: Vec<&PlayerStatisticsHistoryItem> = hist
            .items
            .iter()
            .filter(|i| i.season.start_year == 2025 && i.team_slug == "zenit")
            .collect();
        assert_eq!(zenit_rows.len(), 1);
        assert_eq!(zenit_rows[0].statistics.played, 14);
        assert_eq!(zenit_rows[0].statistics.goals, 4);
    }

    #[test]
    fn consecutive_non_senior_seasons_preserve_main_row_each_year() {
        // User-reported bug: a U18 player with no senior callups loses
        // his Main row for every season after the first. The very first
        // season-end keeps the seed entry under the `is_initial_record`
        // gate, but every subsequent zero-game season-end row is wiped
        // out by the merge step because it isn't the career-first row.
        //
        // Expected behaviour: every season the player exists at the club
        // produces a Main row, even when they never break into the senior
        // squad. The third season here has a single senior callup to
        // confirm the path that actually records games still works.
        let mut hist = PlayerStatisticsHistory::new();
        let main = season_team("spartak");

        hist.seed_initial_team(&main, d(2025, 8, 1), false);

        // 2025/26 — U18 only, no senior callups.
        hist.record_season_end(
            Season::new(2025),
            PlayerStatistics::default(),
            &main,
            false,
            None,
        );

        // 2026/27 — U18 only again, no senior callups.
        hist.record_season_end(
            Season::new(2026),
            PlayerStatistics::default(),
            &main,
            false,
            None,
        );

        // 2027/28 — one senior callup (6 apps).
        let mut callups = PlayerStatistics::default();
        callups.played = 6;
        hist.record_season_end(Season::new(2027), callups, &main, false, None);

        let main_rows: Vec<&PlayerStatisticsHistoryItem> = hist
            .items
            .iter()
            .filter(|i| i.team_slug == "spartak")
            .collect();
        assert_eq!(
            main_rows.len(),
            3,
            "every consecutive non-senior season must keep its Main row, got seasons: {:?}",
            main_rows
                .iter()
                .map(|i| i.season.start_year)
                .collect::<Vec<_>>()
        );
        let row_2026 = hist
            .items
            .iter()
            .find(|i| i.season.start_year == 2026 && i.team_slug == "spartak")
            .expect("2026/27 Main row must survive");
        assert_eq!(row_2026.statistics.played, 0);
    }

    #[test]
    fn skipped_season_snapshot_does_not_collapse_rows() {
        // Repro hypothesis for the user's "missing 2026/27" report:
        // the regular season-end snapshot for 2026/27 doesn't fire
        // (e.g. because the country's leagues happened to have no rows
        // with played > 0 on the schedule-regen day, or some other gate
        // dropped `new_season_started` for the year). The next year's
        // snapshot then drains the seed entry that was meant for
        // 2026/27 and stamps it under 2027/28's label, leaving the
        // career table missing the middle season entirely.
        let mut hist = PlayerStatisticsHistory::new();
        let main = season_team("spartak");
        hist.seed_initial_team(&main, d(2025, 8, 1), false);

        // 2025/26 ended normally.
        hist.record_season_end(
            Season::new(2025),
            PlayerStatistics::default(),
            &main,
            false,
            None,
        );
        // 2026/27: NO snapshot fires (skipped year).
        // 2027/28 ends — snapshot finally fires for that year.
        hist.record_season_end(
            Season::new(2027),
            PlayerStatistics::default(),
            &main,
            false,
            None,
        );

        let years: Vec<u16> = hist
            .items
            .iter()
            .filter(|i| i.team_slug == "spartak")
            .map(|i| i.season.start_year)
            .collect();
        assert!(
            years.contains(&2025) && years.contains(&2026) && years.contains(&2027),
            "skipping a snapshot must not collapse the missed season; got: {:?}",
            years
        );
    }

    #[test]
    fn youth_to_main_promotion_via_history_layer_does_not_lose_stats() {
        // History-layer guard: `record_intra_club_move` with
        // `from_senior=false` historically passed `old_stats` to the
        // function and then ignored them — neither the from nor the
        // to branch wrote them anywhere. Callers must therefore avoid
        // handing over stats they care about preserving. This test
        // pins down that contract: passing default() into a
        // non-senior-to-senior move is harmless, and the existing
        // Main-aliased seed is reactivated for the player to
        // continue accumulating into.
        let mut hist = PlayerStatisticsHistory::new();
        let main = season_team("spartak");
        let u21 = season_team("spartak-u21");
        hist.seed_initial_team(&main, d(2025, 8, 1), false);

        // No stats handed over (the Player-layer fix in
        // `on_intra_club_move` skips the mem::take when the from
        // side is non-senior; player.statistics keeps the callup
        // games for the next season-end drain).
        hist.record_intra_club_move(
            PlayerStatistics::default(),
            &u21,
            &main,
            false,
            true,
            d(2025, 11, 15),
        );

        let main_entries: Vec<&CurrentSeasonEntry> = hist
            .current
            .iter()
            .filter(|e| e.team_slug == "spartak" && !e.is_loan)
            .collect();
        assert_eq!(
            main_entries.len(),
            1,
            "exactly one Main entry must be active after promotion, got {:?}",
            main_entries
                .iter()
                .map(|e| (e.team_slug.clone(), e.departed_date))
                .collect::<Vec<_>>()
        );
        assert!(
            main_entries[0].departed_date.is_none(),
            "the Main entry must be active so subsequent senior games \
             accumulate against it"
        );
    }

    #[test]
    fn multi_year_skipped_snapshot_fills_every_gap_year() {
        // Defensive case: if MORE than one snapshot is skipped in a
        // row, the flush still recovers one row per missed year via
        // the gap-fill so the career table stays unbroken even after
        // multiple successive failures of the season-end trigger.
        let mut hist = PlayerStatisticsHistory::new();
        let main = season_team("spartak");
        hist.seed_initial_team(&main, d(2025, 8, 1), false);

        hist.record_season_end(
            Season::new(2025),
            PlayerStatistics::default(),
            &main,
            false,
            None,
        );

        // 2026/27, 2027/28 BOTH skipped — snapshot finally fires for
        // 2028/29.
        hist.record_season_end(
            Season::new(2028),
            PlayerStatistics::default(),
            &main,
            false,
            None,
        );

        let years: Vec<u16> = {
            let mut v: Vec<u16> = hist
                .items
                .iter()
                .filter(|i| i.team_slug == "spartak" && !i.is_loan)
                .map(|i| i.season.start_year)
                .collect();
            v.sort();
            v.dedup();
            v
        };
        assert_eq!(
            years,
            vec![2025, 2026, 2027, 2028],
            "every season between the last recorded year and the snapshot \
             must have a Main row, got: {:?}",
            years
        );
    }

    #[test]
    fn multi_league_country_repeated_snapshots_keep_every_season_row() {
        // Real-game repro: a country with several leagues whose seasons
        // start on staggered days (e.g. Premier League Aug 1, second
        // division Aug 5, youth league Aug 10) triggers
        // `snapshot_player_season_statistics` THREE times across that
        // window — every league that flips into a new season fires the
        // country-wide snapshot. For a U21 player, each fire calls
        // `record_season_end` for the same `ended_season`. The first
        // call drains via the regular path; the next two hit the
        // duplicate-season guard branch.
        //
        // The user reports a 2026/27 row going missing after running the
        // sim through ~1.2 seasons. This test models the full sequence
        // for three consecutive seasons including the staggered re-fires
        // so any drop in the duplicate guard branch surfaces here.
        let mut hist = PlayerStatisticsHistory::new();
        let main = season_team("spartak");
        hist.seed_initial_team(&main, d(2025, 8, 1), false);

        let snapshot = |hist: &mut PlayerStatisticsHistory, ended_year: u16| {
            hist.record_season_end(
                Season::new(ended_year),
                PlayerStatistics::default(),
                &main,
                false,
                None,
            );
        };

        // End of 2025/26 — three staggered league snapshots.
        snapshot(&mut hist, 2025); // Premier League ticks first
        snapshot(&mut hist, 2025); // 2nd division
        snapshot(&mut hist, 2025); // youth premier league

        // End of 2026/27 — same staggered pattern.
        snapshot(&mut hist, 2026);
        snapshot(&mut hist, 2026);
        snapshot(&mut hist, 2026);

        // End of 2027/28 — same again.
        snapshot(&mut hist, 2027);
        snapshot(&mut hist, 2027);
        snapshot(&mut hist, 2027);

        let main_rows: Vec<&PlayerStatisticsHistoryItem> = hist
            .items
            .iter()
            .filter(|i| i.team_slug == "spartak")
            .collect();
        let years: Vec<u16> = main_rows.iter().map(|i| i.season.start_year).collect();
        assert!(
            years.contains(&2025) && years.contains(&2026) && years.contains(&2027),
            "every consecutive non-senior season must keep its Main row \
             across the multi-league snapshot pattern, got: {:?}",
            years
        );
        assert_eq!(
            main_rows.len(),
            3,
            "expected exactly 3 Main rows, got {}",
            main_rows.len()
        );
    }

    #[test]
    fn view_items_keeps_zero_game_row_for_middle_non_senior_season() {
        // View-time variant of the bug: a saved player history with three
        // Main rows (one with games, two with zero games) must keep all
        // three at render time. The legacy view merge dropped any 0-game
        // row that wasn't the career-first one.
        let mut games_only = PlayerStatistics::default();
        games_only.played = 6;
        let hist = PlayerStatisticsHistory::from_items(vec![
            PlayerStatisticsHistoryItem {
                season: Season::new(2025),
                team_name: "Spartak Moscow".to_string(),
                team_slug: "spartak".to_string(),
                team_reputation: 5_000,
                league_name: "Russian Premier League".to_string(),
                league_slug: "rpl".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: PlayerStatistics::default(),
                seq_id: 0,
            },
            PlayerStatisticsHistoryItem {
                season: Season::new(2026),
                team_name: "Spartak Moscow".to_string(),
                team_slug: "spartak".to_string(),
                team_reputation: 5_000,
                league_name: "Russian Premier League".to_string(),
                league_slug: "rpl".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: PlayerStatistics::default(),
                seq_id: 1,
            },
            PlayerStatisticsHistoryItem {
                season: Season::new(2027),
                team_name: "Spartak Moscow".to_string(),
                team_slug: "spartak".to_string(),
                team_reputation: 5_000,
                league_name: "Russian Premier League".to_string(),
                league_slug: "rpl".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: games_only,
                seq_id: 2,
            },
        ]);

        let view = hist.view_items(None, d(2028, 1, 15));
        let seasons: Vec<u16> = view
            .iter()
            .filter(|i| i.team_slug == "spartak")
            .map(|i| i.season.start_year)
            .collect();
        assert!(
            seasons.contains(&2025) && seasons.contains(&2026) && seasons.contains(&2027),
            "view must keep every Main row across consecutive seasons, got: {:?}",
            seasons
        );
    }

    #[test]
    fn view_items_collapses_legacy_duplicate_main_rows() {
        // Older saves carry phantom duplicate rows from the pre-fix
        // aliasing. The view-time merge collapses them at render so
        // existing player pages render cleanly without a data
        // migration.
        let mut frozen_a = PlayerStatistics::default();
        frozen_a.played = 12;
        let mut frozen_b = PlayerStatistics::default();
        frozen_b.played = 6;

        let hist = PlayerStatisticsHistory::from_items(vec![
            PlayerStatisticsHistoryItem {
                season: Season::new(2025),
                team_name: "Spartak Moscow".to_string(),
                team_slug: "spartak".to_string(),
                team_reputation: 5_000,
                league_name: "Russian Premier League".to_string(),
                league_slug: "rpl".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: frozen_a,
                seq_id: 0,
            },
            PlayerStatisticsHistoryItem {
                season: Season::new(2025),
                team_name: "Spartak Moscow".to_string(),
                team_slug: "spartak".to_string(),
                team_reputation: 5_000,
                league_name: "Russian Premier League".to_string(),
                league_slug: "rpl".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: frozen_b,
                seq_id: 1,
            },
        ]);

        let view = hist.view_items(None, d(2026, 1, 15));
        let spartak_2025: Vec<_> = view
            .iter()
            .filter(|i| i.season.start_year == 2025 && i.team_slug == "spartak")
            .collect();
        assert_eq!(
            spartak_2025.len(),
            1,
            "view must collapse legacy duplicate rows"
        );
        assert_eq!(spartak_2025[0].statistics.played, 18);
    }
}
