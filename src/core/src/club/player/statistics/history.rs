use chrono::NaiveDate;
use crate::league::Season;
use super::types::{PlayerStatistics, TeamInfo};

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

    fn next_seq(&mut self) -> u32 {
        let s = self.next_seq;
        self.next_seq += 1;
        s
    }

    /// Mark the most recent entry for a team as departed on the given date.
    fn mark_departed(&mut self, team_slug: &str, is_loan: bool, date: NaiveDate) {
        if let Some(entry) = self.current.iter_mut().rev()
            .find(|e| e.team_slug == team_slug && e.is_loan == is_loan)
        {
            entry.departed_date = Some(date);
        }
    }

    /// Add or update a current-season entry for (team_slug, is_loan).
    /// If an entry already exists: replace stats (if new has games, or old has none), keep fee.
    /// If no entry exists: push new row.
    fn upsert_current(&mut self, team: &TeamInfo, stats: PlayerStatistics, is_loan: bool, fee: Option<f64>, date: NaiveDate) {
        if let Some(entry) = self.current.iter_mut().rev().find(|e| {
            e.team_slug == team.slug && e.is_loan == is_loan
        }) {
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
    fn push_new_entry(&mut self, team: &TeamInfo, stats: PlayerStatistics, is_loan: bool, fee: Option<f64>, date: NaiveDate) {
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

        for entry in stale {
            let entry_season = Season::from_date(entry.joined_date);
            let season_end = entry_season.end_date();

            let games = entry.statistics.total_games();
            let has_fee = entry.transfer_fee.is_some();
            let is_initial_record = is_first_season && first_seq == Some(entry.seq_id);
            let stale_loan_seed = entry.is_loan && games == 0 && !has_fee;

            let end_date = entry.departed_date.unwrap_or(season_end);
            let days_at_club = (end_date - entry.joined_date).num_days().max(0);
            let season_days = (season_end - entry_season.start_date()).num_days().max(1);
            let time_pct = (days_at_club as f64 / season_days as f64) * 100.0;
            let trivial_stint = games == 0 && !has_fee && time_pct < 35.0;

            if is_initial_record || (!stale_loan_seed && !trivial_stint) {
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

    pub fn record_transfer(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, to: &TeamInfo, fee: f64, date: NaiveDate) {
        self.upsert_current(from, old_stats, false, None, date);
        self.mark_departed(&from.slug, false, date);
        self.push_new_entry(to, PlayerStatistics::default(), false, Some(fee), date);
    }

    pub fn record_loan(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, to: &TeamInfo, loan_fee: f64, date: NaiveDate) {
        self.upsert_current(from, old_stats, false, None, date);
        self.mark_departed(&from.slug, false, date);
        self.push_new_entry(to, PlayerStatistics::default(), true, Some(loan_fee), date);
    }

    pub fn record_loan_return(&mut self, remaining_stats: PlayerStatistics, borrowing: &TeamInfo, date: NaiveDate) {
        self.upsert_current(borrowing, remaining_stats, true, None, date);

        // Clean up stale loan entries: after a loan return, any loan entry
        // with 0 games and no fee is a leftover seed from season-end processing.
        // Keeping it would create phantom history entries in the next season.
        self.current.retain(|e| {
            !(e.is_loan && e.statistics.total_games() == 0 && e.transfer_fee.is_none())
        });

        // Clear departed_date on parent entry — the player is back
        if let Some(parent) = self.current.iter_mut().rev()
            .find(|e| !e.is_loan && e.departed_date.is_some())
        {
            parent.departed_date = None;
            // Reset joined_date to return date for post-loan time calculation
            if parent.statistics.total_games() == 0 && parent.transfer_fee.is_none() {
                parent.joined_date = date;
            }
        }
    }

    pub fn record_cancel_loan(&mut self, old_stats: PlayerStatistics, borrowing: &TeamInfo, _parent: &TeamInfo, _is_loan: bool, date: NaiveDate) {
        self.upsert_current(borrowing, old_stats, true, None, date);

        // Mirror record_loan_return cleanup: clear parent departed_date
        // so the parent entry correctly represents the post-return stint
        if let Some(parent) = self.current.iter_mut().rev()
            .find(|e| !e.is_loan && e.departed_date.is_some())
        {
            parent.departed_date = None;
            if parent.statistics.total_games() == 0 && parent.transfer_fee.is_none() {
                parent.joined_date = date;
            }
        }
    }

    pub fn record_departure_transfer(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, to: &TeamInfo, fee: Option<f64>, is_loan: bool, date: NaiveDate) {
        self.flush_stale_entries(date);
        self.upsert_current(from, old_stats, is_loan, None, date);
        self.mark_departed(&from.slug, is_loan, date);
        self.push_new_entry(to, PlayerStatistics::default(), false, fee, date);
    }

    pub fn record_departure_loan(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, _parent: &TeamInfo, to: &TeamInfo, _is_loan: bool, date: NaiveDate) {
        self.flush_stale_entries(date);
        self.upsert_current(from, old_stats, false, None, date);
        self.mark_departed(&from.slug, false, date);
        // Use Some(0.0) for fee so the loan entry survives stale_loan_seed filter
        // even with 0 games (consistent with record_loan which always sets Some(fee))
        self.push_new_entry(to, PlayerStatistics::default(), true, Some(0.0), date);
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
        // Guard: if this season was already frozen (multi-league country where
        // different leagues start new seasons on different dates, or cross-country
        // loan where both countries snapshot the same player), avoid duplicates.
        // Merge any remaining stats into the existing frozen entry and re-seed.
        if self.items.iter().any(|i| i.season.start_year == season.start_year) {
            // Merge remaining stats (games played between first and second snapshot)
            if current_stats.total_games() > 0 {
                if let Some(existing) = self.items.iter_mut().rev()
                    .find(|i| i.season.start_year == season.start_year
                        && i.team_slug == team.slug
                        && i.is_loan == is_loan)
                {
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
            // Re-seed for next season
            let new_season_start = Season::new(season.start_year + 1).start_date();
            self.upsert_current(team, PlayerStatistics::default(), is_loan, None, new_season_start);
            return;
        }

        // When the player has no tracked entry for this team (e.g. returned from
        // loan mid-season), use last_transfer_date as joined_date so the trivial
        // stint filter can accurately measure time at this club.
        let has_existing = self.current.iter().any(|e| e.team_slug == team.slug && e.is_loan == is_loan);
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

        for entry in entries {
            let games = entry.statistics.total_games();
            let end_date = entry.departed_date.unwrap_or(season_end);
            let days_at_club = (end_date - entry.joined_date).num_days().max(0);
            let season_days = (season_end - season.start_date()).num_days().max(1);
            let time_pct = (days_at_club as f64 / season_days as f64) * 100.0;

            // Drop entries where the player barely stayed and never played:
            // - Loan entries with 0 games and no fee are stale seeds (phantom entries)
            // - Any entry with 0 games and no fee that covers < 35% of the season is noise
            //   (e.g. returned from loan near season end, 0 apps at parent club)
            // Always keep: entries with games, entries with transfer fees,
            // entries where the player was at the club for a meaningful portion of the season,
            // or the player's first-ever career record (initial club).
            //
            let has_fee = entry.transfer_fee.is_some();
            let is_initial_record = is_first_season && first_seq == Some(entry.seq_id);
            let trivial_stint = games == 0 && !has_fee && time_pct < 35.0;
            let stale_loan_seed = entry.is_loan && games == 0 && !has_fee;

            let keep = is_initial_record || (!stale_loan_seed && !trivial_stint);

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

        // Seed the new season with an empty entry for the current club
        let new_season_start = Season::new(season.start_year + 1).start_date();
        self.upsert_current(team, PlayerStatistics::default(), is_loan, None, new_season_start);
    }

    // ── Initial seeding ───────────────────────────────────

    /// Seed the player's history with their initial team when the game starts.
    /// Only seeds if history is completely empty (no current entries).
    pub fn seed_initial_team(&mut self, team: &TeamInfo, date: NaiveDate) {
        if self.current.is_empty() && self.items.is_empty() {
            self.upsert_current(team, PlayerStatistics::default(), false, None, date);
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
    pub fn view_items(&self, live_stats: Option<&PlayerStatistics>) -> Vec<PlayerStatisticsHistoryItem> {
        let current_season = self.current.first()
            .map(|e| Season::from_date(e.joined_date))
            .unwrap_or_else(|| Season::new(0));

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

            result.push(PlayerStatisticsHistoryItem {
                season: current_season.clone(),
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

        result.sort_by(|a, b| {
            b.season.start_year.cmp(&a.season.start_year)
                .then(b.seq_id.cmp(&a.seq_id))
        });
        result
    }
}
