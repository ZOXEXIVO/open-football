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
    }

    // ── Mid-season events ─────────────────────────────────
    //
    // The current club always exists in `current` (created at season end or first event).
    // Mid-season events just save stats on existing entry + add destination.

    pub fn record_transfer(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, to: &TeamInfo, fee: f64, date: NaiveDate) {
        self.upsert_current(from, old_stats, false, None, date);
        self.mark_departed(&from.slug, false, date);
        self.upsert_current(to, PlayerStatistics::default(), false, Some(fee), date);
    }

    pub fn record_loan(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, to: &TeamInfo, loan_fee: f64, date: NaiveDate) {
        self.upsert_current(from, old_stats, false, None, date);
        self.mark_departed(&from.slug, false, date);
        self.upsert_current(to, PlayerStatistics::default(), true, Some(loan_fee), date);
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

    pub fn record_cancel_loan(&mut self, old_stats: PlayerStatistics, borrowing: &TeamInfo, _parent: &TeamInfo, _is_loan: bool, _date: NaiveDate) {
        self.upsert_current(borrowing, old_stats, true, None, _date);
    }

    pub fn record_departure_transfer(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, to: &TeamInfo, fee: Option<f64>, is_loan: bool, date: NaiveDate) {
        self.upsert_current(from, old_stats, is_loan, None, date);
        self.upsert_current(to, PlayerStatistics::default(), false, fee, date);
    }

    pub fn record_departure_loan(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, _parent: &TeamInfo, to: &TeamInfo, _is_loan: bool, date: NaiveDate) {
        self.upsert_current(from, old_stats, false, None, date);
        self.upsert_current(to, PlayerStatistics::default(), true, None, date);
    }

    // ── Season end: drain current → frozen items, then seed new season ──

    pub fn record_season_end(
        &mut self,
        season: Season,
        current_stats: PlayerStatistics,
        team: &TeamInfo,
        is_loan: bool,
        _last_transfer_date: Option<NaiveDate>,
    ) {
        // Apply live stats to the current club entry
        self.upsert_current(team, current_stats, is_loan, None, season.start_date());

        // Drain everything into frozen items
        let season_end = season.end_date();
        let entries = std::mem::take(&mut self.current);

        for entry in entries {
            let games = entry.statistics.total_games();
            let end_date = entry.departed_date.unwrap_or(season_end);
            let days_at_club = (end_date - entry.joined_date).num_days().max(0);
            let season_days = (season_end - season.start_date()).num_days().max(1);
            let time_pct = (days_at_club as f64 / season_days as f64) * 100.0;

            // Drop entries where the player barely stayed and never played:
            // - Loan entries with 0 games and no fee are stale seeds (phantom entries)
            // - Any entry with 0 games and no fee that covers < 3% of the season is noise
            //   (e.g. returned from loan 5 days before season end, 0 apps at parent)
            // Always keep: entries with games, entries with transfer fees, or
            // entries where the player was at the club for a meaningful portion of the season
            let has_fee = entry.transfer_fee.is_some();
            let trivial_stint = games == 0 && !has_fee && time_pct < 3.0;
            let stale_loan_seed = entry.is_loan && games == 0 && !has_fee;

            let keep = !stale_loan_seed && !trivial_stint;

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

    // ── Query: pure read, no mutation ──────────────────────

    /// Get the transfer fee for a team in the current season.
    /// Checks live current-season entries first, falls back to frozen history.
    pub fn current_transfer_fee(&self, team_slug: &str, season_year: u16) -> Option<f64> {
        self.current.iter().rev()
            .find(|e| e.team_slug == team_slug)
            .and_then(|e| e.transfer_fee)
            .or_else(|| {
                self.items.iter()
                    .find(|h| h.season.start_year == season_year && h.team_slug == team_slug)
                    .and_then(|h| h.transfer_fee)
            })
    }

    // ── View: pure read, no mutation ────────────────────────

    /// Returns all history (past seasons) + current season entries,
    /// sorted by season desc, then seq_id desc.
    pub fn view_items(&self) -> Vec<PlayerStatisticsHistoryItem> {
        let current_season = self.current.first()
            .map(|e| Season::from_date(e.joined_date))
            .unwrap_or_else(|| Season::new(0));

        self.view_items_with_season(current_season)
    }

    pub fn view_items_with_season(&self, current_season: Season) -> Vec<PlayerStatisticsHistoryItem> {
        let mut result: Vec<PlayerStatisticsHistoryItem> = self.items.clone();

        for entry in &self.current {
            result.push(PlayerStatisticsHistoryItem {
                season: current_season.clone(),
                team_name: entry.team_name.clone(),
                team_slug: entry.team_slug.clone(),
                team_reputation: entry.team_reputation,
                league_name: entry.league_name.clone(),
                league_slug: entry.league_slug.clone(),
                is_loan: entry.is_loan,
                transfer_fee: entry.transfer_fee,
                statistics: entry.statistics.clone(),
                seq_id: entry.seq_id,
            });
        }

        result.sort_by(|a, b| {
            b.season.start_year.cmp(&a.season.start_year)
                .then(b.seq_id.cmp(&a.seq_id))
        });
        result
    }

    // ── Private helpers ─────────────────────────────────────

    /// Find parent club info from current or frozen history.
    fn find_parent_info(&self, exclude_slug: &str) -> Option<TeamInfo> {
        self.current.iter().rev()
            .find(|e| !e.is_loan && e.team_slug != exclude_slug)
            .map(|e| TeamInfo {
                name: e.team_name.clone(),
                slug: e.team_slug.clone(),
                reputation: e.team_reputation,
                league_name: e.league_name.clone(),
                league_slug: e.league_slug.clone(),
            })
            .or_else(|| self.items.iter().rev()
                .find(|e| !e.is_loan && e.team_slug != exclude_slug)
                .map(|e| TeamInfo {
                    name: e.team_name.clone(),
                    slug: e.team_slug.clone(),
                    reputation: e.team_reputation,
                    league_name: e.league_name.clone(),
                    league_slug: e.league_slug.clone(),
                }))
    }
}
