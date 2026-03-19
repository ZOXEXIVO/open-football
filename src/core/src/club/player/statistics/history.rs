use chrono::NaiveDate;
use crate::league::Season;
use super::types::{PlayerStatistics, TeamInfo};

const THREE_MONTHS_DAYS: i64 = 90;

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
        self.upsert_current(to, PlayerStatistics::default(), false, Some(fee), date);
    }

    pub fn record_loan(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, to: &TeamInfo, loan_fee: f64, date: NaiveDate) {
        self.upsert_current(from, old_stats, false, None, date);
        self.upsert_current(to, PlayerStatistics::default(), true, Some(loan_fee), date);
    }

    pub fn record_loan_return(&mut self, remaining_stats: PlayerStatistics, borrowing: &TeamInfo, _date: NaiveDate) {
        self.upsert_current(borrowing, remaining_stats, true, None, _date);
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
            let days = (season_end - entry.joined_date).num_days().max(0);
            // Drop only short loan stints with 0 games and no fee
            let keep = entry.statistics.total_games() > 0
                || entry.transfer_fee.is_some()
                || !entry.is_loan
                || days >= THREE_MONTHS_DAYS;

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
                .then(a.seq_id.cmp(&b.seq_id))
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
