use super::types::{PlayerStatistics, TeamInfo};
use crate::league::Season;
use chrono::NaiveDate;

const ZERO_APP_TRIVIAL_SEASON_SHARE: f64 = 0.35;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    InitialSeed,
    SeasonSeed,
    TransferIn,
    LoanIn,
    LoanReturn,
    FreeAgentSigning,
    SourceSnapshot,
}

#[derive(Debug, Clone)]
pub struct PlayerStatisticsHistory {
    pub items: Vec<PlayerStatisticsHistoryItem>,
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
    pub departed_date: Option<NaiveDate>,
    pub seq_id: u32,
    pub season_start_year: u16,
    pub kind: EntryKind,
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

    pub fn needs_current_season_seed(&self) -> bool {
        self.current.is_empty()
    }

    fn next_seq(&mut self) -> u32 {
        let s = self.next_seq;
        self.next_seq += 1;
        s
    }

    fn season_year_for(date: NaiveDate) -> u16 {
        Season::from_date(date).start_year
    }

    fn find_active_mut(
        &mut self,
        season_start_year: u16,
        team_slug: &str,
        is_loan: bool,
    ) -> Option<&mut CurrentSeasonEntry> {
        self.current.iter_mut().rev().find(|e| {
            e.season_start_year == season_start_year
                && e.team_slug == team_slug
                && e.is_loan == is_loan
                && e.departed_date.is_none()
        })
    }

    fn find_closed_parent_mut(
        &mut self,
        season_start_year: u16,
        team_slug: &str,
    ) -> Option<&mut CurrentSeasonEntry> {
        self.current.iter_mut().rev().find(|e| {
            e.season_start_year == season_start_year
                && e.team_slug == team_slug
                && !e.is_loan
                && e.departed_date.is_some()
        })
    }

    fn merge_or_set_stats(target: &mut PlayerStatistics, incoming: PlayerStatistics) {
        if incoming.total_games() == 0 {
            return;
        }
        if target.total_games() == 0 {
            *target = incoming;
        } else {
            target.merge_from(&incoming);
        }
    }

    fn push_entry(
        &mut self,
        team: &TeamInfo,
        season_start_year: u16,
        stats: PlayerStatistics,
        is_loan: bool,
        fee: Option<f64>,
        joined_date: NaiveDate,
        kind: EntryKind,
    ) -> u32 {
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
            joined_date,
            departed_date: None,
            seq_id: seq,
            season_start_year,
            kind,
        });
        seq
    }

    /// Locate the active entry for `(season_start_year, team_slug, is_loan)`
    /// and merge `stats` / install `fee` into it. If none exists, create one.
    fn upsert_active(
        &mut self,
        team: &TeamInfo,
        season_start_year: u16,
        stats: PlayerStatistics,
        is_loan: bool,
        fee: Option<f64>,
        joined_date: NaiveDate,
        kind: EntryKind,
    ) {
        if let Some(entry) = self.find_active_mut(season_start_year, &team.slug, is_loan) {
            Self::merge_or_set_stats(&mut entry.statistics, stats);
            if fee.is_some() && entry.transfer_fee.is_none() {
                entry.transfer_fee = fee;
            }
        } else {
            self.push_entry(
                team,
                season_start_year,
                stats,
                is_loan,
                fee,
                joined_date,
                kind,
            );
        }
    }

    fn close_active(
        &mut self,
        season_start_year: u16,
        team_slug: &str,
        is_loan: bool,
        date: NaiveDate,
    ) {
        if let Some(e) = self.find_active_mut(season_start_year, team_slug, is_loan) {
            e.departed_date = Some(date);
        }
    }

    /// Decide whether an entry's data is meaningful enough to survive collapse.
    /// `is_first_career` short-circuits: the player's very first entry is
    /// always preserved, even with 0 games and no fee.
    /// `has_loan_peer` covers the parent-club-of-loaned-player case: a
    /// 0-app permanent row is the player's parent-club row for the season
    /// when there's a same-season loan at a different club, and must be
    /// kept even if the pre-loan stint is below the time-share threshold.
    fn is_meaningful(
        entry: &CurrentSeasonEntry,
        is_first_career: bool,
        has_loan_peer: bool,
    ) -> bool {
        if is_first_career {
            return true;
        }
        let games = entry.statistics.total_games();
        if games > 0 {
            return true;
        }
        if entry.transfer_fee.is_some() {
            return true;
        }
        if entry.is_loan {
            return false;
        }
        if has_loan_peer {
            return true;
        }
        let season = Season::new(entry.season_start_year);
        let season_start = season.start_date();
        let season_end = season.end_date();
        let end_date = entry
            .departed_date
            .unwrap_or(season_end)
            .max(entry.joined_date);
        let days_at_club = (end_date - entry.joined_date).num_days().max(0) as f64;
        let season_days = (season_end - season_start).num_days().max(1) as f64;
        let share = days_at_club / season_days;
        share >= ZERO_APP_TRIVIAL_SEASON_SHARE
    }

    /// Collect `(season_start_year, team_slug)` for every loan spell visible
    /// to the player — frozen items, current entries, and any in-flight
    /// buffer the caller passes in. Used to drive `has_loan_peer`.
    fn collect_loan_marks(
        items: &[PlayerStatisticsHistoryItem],
        current: &[CurrentSeasonEntry],
        extra: &[CurrentSeasonEntry],
    ) -> Vec<(u16, String)> {
        let mut out = Vec::new();
        for item in items {
            if item.is_loan {
                out.push((item.season.start_year, item.team_slug.clone()));
            }
        }
        for entry in current {
            if entry.is_loan {
                out.push((entry.season_start_year, entry.team_slug.clone()));
            }
        }
        for entry in extra {
            if entry.is_loan {
                out.push((entry.season_start_year, entry.team_slug.clone()));
            }
        }
        out
    }

    fn has_loan_peer(marks: &[(u16, String)], year: u16, slug: &str) -> bool {
        marks.iter().any(|(y, s)| *y == year && s != slug)
    }

    /// Single freeze decision: merge into an existing frozen row when one
    /// already covers `(season, slug, is_loan)`, skip if `seq_id` was
    /// already processed, otherwise push a new frozen item iff meaningful.
    fn freeze_or_merge(
        items: &mut Vec<PlayerStatisticsHistoryItem>,
        entry: CurrentSeasonEntry,
        is_first_career: bool,
        has_loan_peer: bool,
    ) {
        if items
            .iter()
            .any(|i| i.season.start_year == entry.season_start_year && i.seq_id == entry.seq_id)
        {
            return;
        }
        if let Some(existing) = items.iter_mut().rev().find(|i| {
            i.season.start_year == entry.season_start_year
                && i.team_slug == entry.team_slug
                && i.is_loan == entry.is_loan
        }) {
            if entry.statistics.total_games() > 0 {
                let mut more = entry.statistics;
                more.played += more.played_subs;
                more.played_subs = 0;
                existing.statistics.merge_from(&more);
            }
            if existing.transfer_fee.is_none() && entry.transfer_fee.is_some() {
                existing.transfer_fee = entry.transfer_fee;
            }
            return;
        }
        if Self::is_meaningful(&entry, is_first_career, has_loan_peer) {
            items.push(Self::freeze_entry(entry));
        }
    }

    fn freeze_entry(entry: CurrentSeasonEntry) -> PlayerStatisticsHistoryItem {
        let mut stats = entry.statistics;
        stats.played += stats.played_subs;
        stats.played_subs = 0;
        PlayerStatisticsHistoryItem {
            season: Season::new(entry.season_start_year),
            team_name: entry.team_name,
            team_slug: entry.team_slug,
            team_reputation: entry.team_reputation,
            league_name: entry.league_name,
            league_slug: entry.league_slug,
            is_loan: entry.is_loan,
            transfer_fee: entry.transfer_fee,
            statistics: stats,
            seq_id: entry.seq_id,
        }
    }

    /// Drain entries whose `season_start_year < before_year`. Entries
    /// still active are closed at their (now-elapsed) season end so the
    /// collapse rule can score them. Called at the head of every event
    /// (mid-season *and* season-end) so a delayed country snapshot leaves
    /// nothing co-mingled across seasons.
    fn flush_stale_to(&mut self, before_year: u16) {
        let mut stale: Vec<CurrentSeasonEntry> = Vec::new();
        let mut remaining: Vec<CurrentSeasonEntry> = Vec::with_capacity(self.current.len());
        for entry in std::mem::take(&mut self.current) {
            if entry.season_start_year < before_year {
                stale.push(entry);
            } else {
                remaining.push(entry);
            }
        }
        self.current = remaining;

        let is_first_season = self.items.is_empty();
        let first_seq = stale.iter().map(|e| e.seq_id).min();
        let marks = Self::collect_loan_marks(&self.items, &self.current, &stale);

        for mut entry in stale {
            if entry.departed_date.is_none() {
                entry.departed_date = Some(Season::new(entry.season_start_year).end_date());
            }
            let is_first_career = is_first_season && first_seq == Some(entry.seq_id);
            let has_peer = Self::has_loan_peer(&marks, entry.season_start_year, &entry.team_slug);
            Self::freeze_or_merge(&mut self.items, entry, is_first_career, has_peer);
        }
    }

    fn flush_stale_entries(&mut self, current_date: NaiveDate) {
        let before_year = Self::season_year_for(current_date);
        self.flush_stale_to(before_year);
    }

    // ── Mid-season events ─────────────────────────────────

    pub fn record_transfer(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        fee: f64,
        date: NaiveDate,
    ) {
        self.flush_stale_entries(date);
        let year = Self::season_year_for(date);
        self.upsert_active(
            from,
            year,
            old_stats,
            false,
            None,
            Season::new(year).start_date(),
            EntryKind::SourceSnapshot,
        );
        self.close_active(year, &from.slug, false, date);
        self.push_entry(
            to,
            year,
            PlayerStatistics::default(),
            false,
            Some(fee),
            date,
            EntryKind::TransferIn,
        );
    }

    pub fn record_loan(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        loan_fee: f64,
        date: NaiveDate,
    ) {
        self.flush_stale_entries(date);
        let year = Self::season_year_for(date);
        self.upsert_active(
            from,
            year,
            old_stats,
            false,
            None,
            Season::new(year).start_date(),
            EntryKind::SourceSnapshot,
        );
        self.close_active(year, &from.slug, false, date);
        self.push_entry(
            to,
            year,
            PlayerStatistics::default(),
            true,
            Some(loan_fee),
            date,
            EntryKind::LoanIn,
        );
    }

    pub fn record_loan_return(
        &mut self,
        remaining_stats: PlayerStatistics,
        borrowing: &TeamInfo,
        parent: &TeamInfo,
        date: NaiveDate,
    ) {
        self.flush_stale_entries(date);
        let year = Self::season_year_for(date);

        // Snapshot remaining stats onto the loan entry that owned this spell.
        // Prefer the active loan entry for the date's season; if the borrowing
        // country already snapshotted that season, fall back to any active loan
        // entry at borrowing (a forward-looking SeasonSeed).
        let target_year = if self.find_active_mut(year, &borrowing.slug, true).is_some() {
            Some(year)
        } else if remaining_stats.total_games() > 0 {
            // No same-season active spell — but real games were played, so
            // they need to land somewhere belonging to the borrowing club.
            // Materialise a fresh same-season loan entry to receive them.
            self.push_entry(
                borrowing,
                year,
                PlayerStatistics::default(),
                true,
                None,
                date,
                EntryKind::LoanIn,
            );
            Some(year)
        } else {
            self.current
                .iter()
                .rev()
                .find(|e| e.team_slug == borrowing.slug && e.is_loan && e.departed_date.is_none())
                .map(|e| e.season_start_year)
        };

        if let Some(ty) = target_year {
            if let Some(entry) = self.find_active_mut(ty, &borrowing.slug, true) {
                Self::merge_or_set_stats(&mut entry.statistics, remaining_stats);
                entry.departed_date = Some(date);
            }
        }

        // Drop forward-looking phantom seeds (loan entry, 0 games, no fee, no
        // departed date) at the borrowing club — they were auto-seeded by an
        // earlier season-end and the player is leaving now, so they'd never
        // accumulate anything.
        self.current.retain(|e| {
            !(e.team_slug == borrowing.slug
                && e.is_loan
                && e.statistics.total_games() == 0
                && e.transfer_fee.is_none()
                && e.departed_date.is_none())
        });

        // Reopen the parent's same-season closed spell, or push a fresh
        // LoanReturn entry. Reopening avoids splitting one continuous parent
        // spell into two rows when the player is loaned out and back inside
        // the same season.
        if let Some(parent_entry) = self.find_closed_parent_mut(year, &parent.slug) {
            parent_entry.departed_date = None;
            if parent_entry.statistics.total_games() == 0 && parent_entry.transfer_fee.is_none() {
                parent_entry.joined_date = date;
            }
        } else if !self
            .current
            .iter()
            .any(|e| !e.is_loan && e.season_start_year == year && e.departed_date.is_none())
        {
            self.push_entry(
                parent,
                year,
                PlayerStatistics::default(),
                false,
                None,
                date,
                EntryKind::LoanReturn,
            );
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
        self.record_loan_return(old_stats, borrowing, parent, date);
    }

    pub fn record_release(
        &mut self,
        last_stats: PlayerStatistics,
        from: &TeamInfo,
        date: NaiveDate,
    ) {
        self.flush_stale_entries(date);
        let year = Self::season_year_for(date);
        self.upsert_active(
            from,
            year,
            last_stats,
            false,
            None,
            Season::new(year).start_date(),
            EntryKind::SourceSnapshot,
        );
        self.close_active(year, &from.slug, false, date);
    }

    pub fn record_free_agent_signing(
        &mut self,
        last_stats: PlayerStatistics,
        to: &TeamInfo,
        date: NaiveDate,
    ) {
        self.flush_stale_entries(date);
        let year = Self::season_year_for(date);

        if last_stats.total_games() > 0 {
            // First try: fold into the still-active source spell (release wasn't
            // recorded separately). Otherwise: attach to the most recent same-
            // season departed source row that still has zero stats — that's the
            // row those games actually belonged to.
            let attached_to_active = if let Some(active) =
                self.current.iter_mut().rev().find(|e| {
                    !e.is_loan && e.season_start_year == year && e.departed_date.is_none()
                }) {
                Self::merge_or_set_stats(&mut active.statistics, last_stats.clone());
                active.departed_date = Some(date);
                true
            } else {
                false
            };
            if !attached_to_active {
                if let Some(departed) = self.current.iter_mut().rev().find(|e| {
                    !e.is_loan
                        && e.season_start_year == year
                        && e.departed_date.is_some()
                        && e.statistics.total_games() == 0
                }) {
                    departed.statistics = last_stats;
                }
            }
        }

        self.push_entry(
            to,
            year,
            PlayerStatistics::default(),
            false,
            Some(0.0),
            date,
            EntryKind::FreeAgentSigning,
        );
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
        let year = Self::season_year_for(date);
        self.upsert_active(
            from,
            year,
            old_stats,
            is_loan,
            None,
            Season::new(year).start_date(),
            EntryKind::SourceSnapshot,
        );
        self.close_active(year, &from.slug, is_loan, date);
        self.push_entry(
            to,
            year,
            PlayerStatistics::default(),
            false,
            fee,
            date,
            EntryKind::TransferIn,
        );
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
        let year = Self::season_year_for(date);
        self.upsert_active(
            from,
            year,
            old_stats,
            false,
            None,
            Season::new(year).start_date(),
            EntryKind::SourceSnapshot,
        );
        self.close_active(year, &from.slug, false, date);
        self.push_entry(
            to,
            year,
            PlayerStatistics::default(),
            true,
            Some(0.0),
            date,
            EntryKind::LoanIn,
        );
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
        let target_year = season.start_year;

        // First, freeze anything from prior seasons that lingers in current
        // (e.g. an out-of-band loan-return that ran after a mid-season
        // snapshot). They'll be merged into existing frozen rows or pushed
        // fresh by `freeze_or_merge`.
        self.flush_stale_to(target_year);

        let frozen_match_exists = self.items.iter().any(|i| {
            i.season.start_year == target_year && i.team_slug == team.slug && i.is_loan == is_loan
        });

        // Three-way apply for current_stats:
        // 1. An active current entry exists for (year, team, is_loan) — merge.
        // 2. No active entry, but a frozen row already covers this row
        //    (multi-league duplicate snapshot) — merge any leftover games into
        //    the frozen row; never push a duplicate.
        // 3. Neither — create a fresh active entry so the games are recorded.
        if self
            .find_active_mut(target_year, &team.slug, is_loan)
            .is_some()
        {
            if let Some(entry) = self.find_active_mut(target_year, &team.slug, is_loan) {
                Self::merge_or_set_stats(&mut entry.statistics, current_stats);
            }
        } else if frozen_match_exists {
            if current_stats.total_games() > 0
                && let Some(existing) = self.items.iter_mut().rev().find(|i| {
                    i.season.start_year == target_year
                        && i.team_slug == team.slug
                        && i.is_loan == is_loan
                })
            {
                let mut remaining = current_stats.clone();
                remaining.played += remaining.played_subs;
                remaining.played_subs = 0;
                existing.statistics.merge_from(&remaining);
            }
        } else {
            let join_date = last_transfer_date.unwrap_or_else(|| season.start_date());
            self.push_entry(
                team,
                target_year,
                current_stats,
                is_loan,
                None,
                join_date,
                EntryKind::SeasonSeed,
            );
        }

        // Drain only entries whose season_start_year matches the season
        // being ended. Entries from later seasons (e.g. mid-season transfer
        // destinations recorded after a late-running first snapshot) stay
        // in current and will be frozen by their own season's end.
        let is_first_season = self.items.is_empty();
        let mut to_drain: Vec<CurrentSeasonEntry> = Vec::new();
        let mut remaining: Vec<CurrentSeasonEntry> = Vec::with_capacity(self.current.len());
        for entry in std::mem::take(&mut self.current) {
            if entry.season_start_year == target_year {
                to_drain.push(entry);
            } else {
                remaining.push(entry);
            }
        }
        self.current = remaining;

        let first_seq = to_drain.iter().map(|e| e.seq_id).min();
        let marks = Self::collect_loan_marks(&self.items, &self.current, &to_drain);
        for entry in to_drain {
            let is_first_career = is_first_season && first_seq == Some(entry.seq_id);
            let has_peer = Self::has_loan_peer(&marks, entry.season_start_year, &entry.team_slug);
            Self::freeze_or_merge(&mut self.items, entry, is_first_career, has_peer);
        }

        let next_year = target_year + 1;
        if !self.current.iter().any(|e| {
            e.season_start_year == next_year && e.team_slug == team.slug && e.is_loan == is_loan
        }) {
            self.push_entry(
                team,
                next_year,
                PlayerStatistics::default(),
                is_loan,
                None,
                Season::new(next_year).start_date(),
                EntryKind::SeasonSeed,
            );
        }
    }

    // ── Initial seeding ───────────────────────────────────

    pub fn seed_initial_team(&mut self, team: &TeamInfo, date: NaiveDate, is_loan: bool) {
        if !self.current.is_empty() {
            return;
        }
        let year = Self::season_year_for(date);
        self.push_entry(
            team,
            year,
            PlayerStatistics::default(),
            is_loan,
            None,
            date,
            EntryKind::InitialSeed,
        );
    }

    // ── View: pure read, no mutation ────────────────────────

    pub fn view_items(
        &self,
        live_stats: Option<&PlayerStatistics>,
    ) -> Vec<PlayerStatisticsHistoryItem> {
        let mut result: Vec<PlayerStatisticsHistoryItem> = self.items.clone();

        let is_first_season = self.items.is_empty();
        let first_seq = self.current.iter().map(|e| e.seq_id).min();
        let marks = Self::collect_loan_marks(&self.items, &self.current, &[]);

        // Identify the single active entry to apply live_stats to.
        let active_seq: Option<u32> = self
            .current
            .iter()
            .find(|e| e.departed_date.is_none())
            .map(|e| e.seq_id);

        for entry in &self.current {
            let is_active = entry.departed_date.is_none();
            let is_first_career = is_first_season && first_seq == Some(entry.seq_id);
            let has_peer = Self::has_loan_peer(&marks, entry.season_start_year, &entry.team_slug);

            // Hide entries that wouldn't survive a season-end freeze. This
            // mirrors `is_meaningful` so a 0-app phantom doesn't render.
            if !is_active && !Self::is_meaningful(entry, is_first_career, has_peer) {
                continue;
            }

            let statistics = if is_active && Some(entry.seq_id) == active_seq {
                live_stats
                    .cloned()
                    .unwrap_or_else(|| entry.statistics.clone())
            } else {
                entry.statistics.clone()
            };

            result.push(PlayerStatisticsHistoryItem {
                season: Season::new(entry.season_start_year),
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
            b.season
                .start_year
                .cmp(&a.season.start_year)
                .then(b.seq_id.cmp(&a.seq_id))
        });

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

    pub fn career_totals(items: &[PlayerStatisticsHistoryItem]) -> PlayerStatistics {
        let mut totals = PlayerStatistics::default();
        for item in items {
            totals.merge_from(&item.statistics);
        }
        totals
    }

    pub fn active_team_slug(&self) -> Option<&str> {
        self.current
            .iter()
            .find(|e| e.departed_date.is_none())
            .map(|e| e.team_slug.as_str())
    }

    pub fn current_club_career_apps(&self, live_played: u16, live_played_subs: u16) -> u32 {
        let slug = match self.active_team_slug() {
            Some(s) => s,
            None => return live_played as u32 + live_played_subs as u32,
        };
        let mut total: u32 = 0;
        for item in &self.items {
            if item.team_slug == slug {
                total = total
                    .saturating_add(item.statistics.played as u32)
                    .saturating_add(item.statistics.played_subs as u32);
            }
        }
        total = total
            .saturating_add(live_played as u32)
            .saturating_add(live_played_subs as u32);
        total
    }
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
            season_start_year: 2025,
            kind: EntryKind::SeasonSeed,
        }
    }

    #[test]
    fn club_career_apps_sums_history_at_current_club_plus_live() {
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
        let mut hist = PlayerStatisticsHistory::from_items(vec![
            frozen(2022, "torino", 60, 0),
            frozen(2023, "juventus", 25, 5),
        ]);
        hist.current.push(current("juventus", 0));
        let apps = hist.current_club_career_apps(10, 0);
        assert_eq!(apps, 30 + 10);
    }

    #[test]
    fn club_career_apps_falls_back_to_live_only_with_no_active_spell() {
        let hist = PlayerStatisticsHistory::new();
        let apps = hist.current_club_career_apps(5, 2);
        assert_eq!(apps, 7);
    }
}
