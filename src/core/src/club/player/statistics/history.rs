use chrono::NaiveDate;
use crate::league::Season;
use super::types::{PlayerStatistics, TeamInfo};

const THREE_MONTHS_DAYS: i64 = 90;

#[derive(Debug, Clone)]
pub struct PlayerStatisticsHistory {
    /// Frozen history from completed seasons. Never modified after write.
    pub items: Vec<PlayerStatisticsHistoryItem>,
    /// Raw current-season entries. Append-only during season, drained at season end.
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

    // ── Mid-season: append to current ─────────────────────────

    pub fn record_transfer(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, to: &TeamInfo, fee: f64, date: NaiveDate) {
        self.upsert_current(from, old_stats, false, None, date);
        self.upsert_current(to, PlayerStatistics::default(), false, Some(fee), date);
    }

    pub fn record_loan(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, to: &TeamInfo, loan_fee: f64, date: NaiveDate) {
        self.upsert_current(from, old_stats, false, None, date);
        self.upsert_current(to, PlayerStatistics::default(), true, Some(loan_fee), date);
    }

    pub fn record_loan_return(&mut self, remaining_stats: PlayerStatistics, borrowing: &TeamInfo, date: NaiveDate) {
        self.upsert_current(borrowing, remaining_stats, true, None, date);

        // Add parent placeholder
        let parent = self.find_parent_info(&borrowing.slug);
        if let Some(info) = parent {
            self.upsert_current(&info, PlayerStatistics::default(), false, None, date);
        }
    }

    pub fn record_cancel_loan(&mut self, old_stats: PlayerStatistics, borrowing: &TeamInfo, parent: &TeamInfo, _is_loan: bool, date: NaiveDate) {
        self.upsert_current(borrowing, old_stats, true, None, date);
        self.upsert_current(parent, PlayerStatistics::default(), false, None, date);
    }

    pub fn record_departure_transfer(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, to: &TeamInfo, fee: Option<f64>, is_loan: bool, date: NaiveDate) {
        self.upsert_current(from, old_stats, is_loan, None, date);
        self.upsert_current(to, PlayerStatistics::default(), false, fee, date);
    }

    pub fn record_departure_loan(&mut self, old_stats: PlayerStatistics, from: &TeamInfo, _parent: &TeamInfo, to: &TeamInfo, _is_loan: bool, date: NaiveDate) {
        self.upsert_current(from, old_stats, false, None, date);
        self.upsert_current(to, PlayerStatistics::default(), true, None, date);
    }

    // ── Season end: drain current → frozen items ──────────────

    /// Drain ALL current entries into frozen items.
    /// Rules for keeping 0-game entries:
    ///   - has official matches → keep
    ///   - has transfer fee → keep
    ///   - stayed > 3 months → keep
    ///   - otherwise → drop
    /// Apps are collapsed: played = played + played_subs, played_subs = 0
    pub fn record_season_end(
        &mut self,
        season: Season,
        current_stats: PlayerStatistics,
        team: &TeamInfo,
        is_loan: bool,
        _last_transfer_date: Option<NaiveDate>,
    ) {
        // Merge final stats into matching current entry, or create one
        if let Some(entry) = self.current.iter_mut().rev().find(|e| {
            e.team_slug == team.slug && e.is_loan == is_loan
        }) {
            if current_stats.total_games() > 0 {
                if entry.statistics.total_games() == 0 {
                    entry.statistics = current_stats;
                } else {
                    entry.statistics.merge_from(&current_stats);
                }
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
                transfer_fee: None,
                statistics: current_stats,
                joined_date: season.start_date(),
                seq_id: seq,
            });
        }

        // Drain everything. After this, current is empty.
        let season_end = season.end_date();
        let entries = std::mem::take(&mut self.current);

        for entry in entries {
            let entry_season = Season::from_date(entry.joined_date);
            let entry_end = entry_season.end_date();
            let days = (entry_end - entry.joined_date).num_days().max(0);
            let keep = entry.statistics.total_games() > 0
                || entry.transfer_fee.is_some()
                || days >= THREE_MONTHS_DAYS;

            if keep {
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

    // ── View: pure read, no mutation ──────────────────────────

    pub fn view_items(&self) -> Vec<PlayerStatisticsHistoryItem> {
        let mut result: Vec<PlayerStatisticsHistoryItem> = self.items.clone();

        for entry in &self.current {
            result.push(PlayerStatisticsHistoryItem {
                season: Season::from_date(entry.joined_date),
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

        let merged = Self::merge_view(&result);
        let filled = Self::fill_gaps(merged);

        let mut out = filled;
        out.sort_by(|a, b| {
            b.season.start_year.cmp(&a.season.start_year)
                .then(b.seq_id.cmp(&a.seq_id))
        });
        out
    }

    // ── Private helpers ───────────────────────────────────────

    /// Find or update existing current entry for same team+loan, or push new.
    fn upsert_current(&mut self, team: &TeamInfo, stats: PlayerStatistics, is_loan: bool, fee: Option<f64>, date: NaiveDate) {
        if let Some(entry) = self.current.iter_mut().rev().find(|e| {
            e.team_slug == team.slug && e.is_loan == is_loan
        }) {
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

    /// Group by (season, team, is_loan), merge stats.
    fn merge_view(items: &[PlayerStatisticsHistoryItem]) -> Vec<PlayerStatisticsHistoryItem> {
        let mut keys: Vec<PlayerStatisticsHistoryItem> = Vec::new();
        let mut sums: Vec<(PlayerStatistics, f32, u16)> = Vec::new(); // (stats, rating_sum, rating_count)

        for item in items {
            let games = item.statistics.total_games();
            let pos = keys.iter().position(|k| {
                k.season.start_year == item.season.start_year
                    && k.team_slug == item.team_slug
                    && k.is_loan == item.is_loan
            });

            if let Some(idx) = pos {
                let (ref mut s, ref mut rsum, ref mut rcnt) = sums[idx];
                s.played += item.statistics.played;
                s.played_subs += item.statistics.played_subs;
                s.goals += item.statistics.goals;
                s.assists += item.statistics.assists;
                s.player_of_the_match += item.statistics.player_of_the_match;
                s.conceded += item.statistics.conceded;
                s.clean_sheets += item.statistics.clean_sheets;
                *rsum += item.statistics.average_rating * games as f32;
                *rcnt += games;
                if item.transfer_fee.is_some() && keys[idx].transfer_fee.is_none() {
                    keys[idx].transfer_fee = item.transfer_fee;
                }
                if item.seq_id > keys[idx].seq_id && games > 0 {
                    keys[idx].seq_id = item.seq_id;
                }
            } else {
                keys.push(PlayerStatisticsHistoryItem {
                    season: item.season.clone(),
                    team_name: item.team_name.clone(),
                    team_slug: item.team_slug.clone(),
                    team_reputation: item.team_reputation,
                    league_name: item.league_name.clone(),
                    league_slug: item.league_slug.clone(),
                    is_loan: item.is_loan,
                    transfer_fee: item.transfer_fee,
                    statistics: PlayerStatistics::default(),
                    seq_id: item.seq_id,
                });
                sums.push((
                    PlayerStatistics {
                        played: item.statistics.played,
                        played_subs: item.statistics.played_subs,
                        goals: item.statistics.goals,
                        assists: item.statistics.assists,
                        penalties: item.statistics.penalties,
                        player_of_the_match: item.statistics.player_of_the_match,
                        yellow_cards: item.statistics.yellow_cards,
                        red_cards: item.statistics.red_cards,
                        shots_on_target: item.statistics.shots_on_target,
                        tackling: item.statistics.tackling,
                        passes: item.statistics.passes,
                        average_rating: 0.0,
                        conceded: item.statistics.conceded,
                        clean_sheets: item.statistics.clean_sheets,
                    },
                    item.statistics.average_rating * games as f32,
                    games,
                ));
            }
        }

        for (i, (s, rsum, rcnt)) in sums.into_iter().enumerate() {
            keys[i].statistics = PlayerStatistics {
                played: s.played,
                played_subs: s.played_subs,
                goals: s.goals,
                assists: s.assists,
                penalties: s.penalties,
                player_of_the_match: s.player_of_the_match,
                yellow_cards: s.yellow_cards,
                red_cards: s.red_cards,
                shots_on_target: s.shots_on_target,
                tackling: s.tackling,
                passes: s.passes,
                average_rating: if rcnt > 0 { rsum / rcnt as f32 } else { 0.0 },
                conceded: s.conceded,
                clean_sheets: s.clean_sheets,
            };
        }

        keys
    }

    /// Fill missing season gaps with 0-game placeholders.
    fn fill_gaps(mut keys: Vec<PlayerStatisticsHistoryItem>) -> Vec<PlayerStatisticsHistoryItem> {
        if keys.is_empty() {
            return keys;
        }

        keys.sort_by_key(|k| k.season.start_year);

        let min = keys.first().unwrap().season.start_year;
        let max = keys.last().unwrap().season.start_year;

        let mut fill = Vec::new();
        for year in min..=max {
            if keys.iter().any(|k| k.season.start_year == year) {
                continue;
            }
            let tmpl = keys.iter().rev()
                .find(|k| k.season.start_year < year && !k.is_loan)
                .or_else(|| keys.iter().rev().find(|k| k.season.start_year < year));

            if let Some(t) = tmpl {
                fill.push(PlayerStatisticsHistoryItem {
                    season: Season::new(year),
                    team_name: t.team_name.clone(),
                    team_slug: t.team_slug.clone(),
                    team_reputation: t.team_reputation,
                    league_name: t.league_name.clone(),
                    league_slug: t.league_slug.clone(),
                    is_loan: false,
                    transfer_fee: None,
                    statistics: PlayerStatistics::default(),
                    seq_id: 0,
                });
            }
        }

        keys.extend(fill);
        keys
    }

    // Keep for backward compat with tests that push directly
    pub fn push_current_with_date(&mut self, team: &TeamInfo, stats: PlayerStatistics, is_loan: bool, fee: Option<f64>, date: NaiveDate) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn make_team(name: &str, slug: &str) -> TeamInfo {
        TeamInfo {
            name: name.to_string(),
            slug: slug.to_string(),
            reputation: 100,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
        }
    }

    fn make_stats(played: u16, rating: f32) -> PlayerStatistics {
        let mut s = PlayerStatistics::default();
        s.played = played;
        s.average_rating = rating;
        s
    }

    #[test]
    fn loan_cancel_then_transfer_preserves_all() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");
        let milan = make_team("Milan", "milan");

        h.record_departure_loan(make_stats(0, 0.0), &juve, &juve, &inter, false, make_date(2026, 1, 15));
        h.record_cancel_loan(make_stats(0, 0.0), &inter, &juve, true, make_date(2026, 2, 15));
        h.record_departure_transfer(make_stats(0, 0.0), &juve, &milan, Some(1_000_000.0), false, make_date(2026, 3, 15));

        // All in current, nothing lost
        assert!(h.current.iter().any(|e| e.team_slug == "milan"), "milan missing");
        assert!(h.current.iter().any(|e| e.team_slug == "inter" && e.is_loan), "inter loan missing");
    }

    #[test]
    fn season_end_keeps_entries_with_games() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");

        h.record_departure_transfer(make_stats(10, 7.0), &juve, &inter, Some(5_000_000.0), false, make_date(2026, 1, 15));
        h.record_season_end(Season::new(2025), make_stats(5, 6.5), &inter, false, None);

        assert_eq!(h.items.len(), 2);
        assert_eq!(h.current.len(), 0);
        assert!(h.items.iter().any(|e| e.team_slug == "juventus" && e.statistics.played == 10));
        assert!(h.items.iter().any(|e| e.team_slug == "inter" && e.statistics.played == 5));
    }

    #[test]
    fn season_end_collapses_subs_into_played() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");

        let mut stats = PlayerStatistics::default();
        stats.played = 10;
        stats.played_subs = 5;
        h.push_current_with_date(&juve, PlayerStatistics::default(), false, None, make_date(2025, 8, 1));
        h.record_season_end(Season::new(2025), stats, &juve, false, None);

        assert_eq!(h.items[0].statistics.played, 15); // 10 + 5
        assert_eq!(h.items[0].statistics.played_subs, 0);
    }

    #[test]
    fn season_end_drops_short_zero_game_entries() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");
        let milan = make_team("Milan", "milan");

        h.record_departure_loan(make_stats(0, 0.0), &juve, &juve, &inter, false, make_date(2026, 3, 1));
        h.record_cancel_loan(make_stats(0, 0.0), &inter, &juve, true, make_date(2026, 4, 1));
        h.record_departure_transfer(make_stats(5, 7.0), &juve, &milan, None, false, make_date(2026, 4, 15));
        h.record_season_end(Season::new(2025), make_stats(3, 6.0), &milan, false, None);

        assert!(h.items.iter().any(|e| e.team_slug == "juventus" && e.statistics.played == 5));
        assert!(h.items.iter().any(|e| e.team_slug == "milan"));
        assert_eq!(h.current.len(), 0);
    }

    #[test]
    fn season_end_keeps_long_zero_game_entries() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");

        h.push_current_with_date(&juve, PlayerStatistics::default(), false, None, make_date(2025, 8, 1));
        h.record_season_end(Season::new(2025), make_stats(0, 0.0), &juve, false, None);

        assert_eq!(h.items.len(), 1);
        assert_eq!(h.items[0].team_slug, "juventus");
    }

    #[test]
    fn season_end_drains_current_completely() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");

        h.record_loan(make_stats(5, 7.0), &juve, &inter, 1000.0, make_date(2026, 1, 15));
        assert!(!h.current.is_empty());

        h.record_season_end(Season::new(2025), make_stats(10, 7.0), &inter, true, None);

        assert_eq!(h.current.len(), 0, "current must be empty after season end");
        assert!(!h.items.is_empty());
    }
}
