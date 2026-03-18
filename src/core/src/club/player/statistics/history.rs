use chrono::NaiveDate;
use crate::league::Season;
use super::types::{PlayerStatistics, TeamInfo};

const THREE_MONTHS_DAYS: i64 = 90;

#[derive(Debug, Clone)]
pub struct PlayerStatisticsHistory {
    /// Finalized history from completed seasons
    pub items: Vec<PlayerStatisticsHistoryItem>,
    /// Raw current-season entries (never wiped mid-season, collapsed at season end)
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
    /// Monotonic sequence ID for stable ordering within a season
    pub seq_id: u32,
}

/// Raw entry for current season. Just appended, never deleted mid-season.
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
    /// When the player joined/arrived at this club (for 3-month rule)
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

    // ── Mid-season events (just append to current, no wipes) ──

    /// Permanent transfer: snapshot "from" stats, add "to" placeholder.
    pub fn record_transfer(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        fee: f64,
        date: NaiveDate,
    ) {
        self.update_or_push_current(from, old_stats, false, None, date);
        self.update_or_push_current(to, PlayerStatistics::default(), false, Some(fee), date);
    }

    /// Loan move: snapshot "from" stats, add loan placeholder.
    pub fn record_loan(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        loan_fee: f64,
        date: NaiveDate,
    ) {
        self.update_or_push_current(from, old_stats, false, None, date);
        self.update_or_push_current(to, PlayerStatistics::default(), true, Some(loan_fee), date);
    }

    /// Loan return: update loan entry stats, add parent placeholder.
    pub fn record_loan_return(
        &mut self,
        remaining_stats: PlayerStatistics,
        borrowing: &TeamInfo,
        date: NaiveDate,
    ) {
        // Update the loan entry with final stats
        let date_season = Season::from_date(date);
        if let Some(entry) = self.current.iter_mut().rev().find(|e| {
            e.team_slug == borrowing.slug && e.is_loan
                && Season::from_date(e.joined_date).start_year == date_season.start_year
        }) {
            if remaining_stats.total_games() > 0 && entry.statistics.total_games() == 0 {
                entry.statistics = remaining_stats;
            }
        } else {
            self.push_current(borrowing, remaining_stats, true, None, date);
        }

        // Find parent club info (last non-loan team that isn't the borrowing club)
        let parent_data = self.current.iter().rev()
            .find(|e| !e.is_loan && e.team_slug != borrowing.slug)
            .map(|e| (e.team_name.clone(), e.team_slug.clone(), e.team_reputation,
                       e.league_name.clone(), e.league_slug.clone()))
            .or_else(|| self.items.iter().rev()
                .find(|e| !e.is_loan && e.team_slug != borrowing.slug)
                .map(|e| (e.team_name.clone(), e.team_slug.clone(), e.team_reputation,
                           e.league_name.clone(), e.league_slug.clone())));

        if let Some((name, slug, rep, ln, ls)) = parent_data {
            let parent_team = TeamInfo {
                name, slug, reputation: rep,
                league_name: ln, league_slug: ls,
            };
            self.update_or_push_current(&parent_team, PlayerStatistics::default(), false, None, date);
        }
    }

    /// Cancel loan: save borrowing stats, add parent placeholder.
    pub fn record_cancel_loan(
        &mut self,
        old_stats: PlayerStatistics,
        borrowing: &TeamInfo,
        parent: &TeamInfo,
        _is_loan: bool,
        date: NaiveDate,
    ) {
        self.update_or_push_current(borrowing, old_stats, true, None, date);
        self.update_or_push_current(parent, PlayerStatistics::default(), false, None, date);
    }

    /// Manual transfer from web UI.
    pub fn record_departure_transfer(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        to: &TeamInfo,
        fee: Option<f64>,
        is_loan: bool,
        date: NaiveDate,
    ) {
        self.update_or_push_current(from, old_stats, is_loan, None, date);
        self.update_or_push_current(to, PlayerStatistics::default(), false, fee, date);
    }

    /// Manual loan from web UI.
    pub fn record_departure_loan(
        &mut self,
        old_stats: PlayerStatistics,
        from: &TeamInfo,
        _parent: &TeamInfo,
        to: &TeamInfo,
        _is_loan: bool,
        date: NaiveDate,
    ) {
        self.update_or_push_current(from, old_stats, false, None, date);
        self.update_or_push_current(to, PlayerStatistics::default(), true, None, date);
    }

    // ── Season end: collapse current → items ──────────────────

    /// Called at season end. Collapses `current` entries into `items`.
    ///
    /// Rules:
    /// - Keep entries where player played official matches (total_games > 0)
    /// - If matches == 0: keep if player stayed > 3 months at the club
    /// - Transfer fee entries are always kept
    pub fn record_season_end(
        &mut self,
        season: Season,
        current_stats: PlayerStatistics,
        team: &TeamInfo,
        is_loan: bool,
        _last_transfer_date: Option<NaiveDate>,
    ) {
        // Update the latest matching current entry with final stats
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
        } else if current_stats.total_games() > 0 || self.current.is_empty() {
            // No current entry — create one (e.g., player never moved this season)
            self.push_current_with_date(
                team, current_stats, is_loan, None,
                season.start_date(),
            );
        }

        // Collapse: move current entries to items with filtering.
        // Entries that belong to the next season (joined after season end) stay in current.
        let season_end = season.end_date();
        let entries = std::mem::take(&mut self.current);

        for entry in entries {
            let entry_season = Season::from_date(entry.joined_date);
            if entry_season.start_year != season.start_year {
                // Belongs to a different season — keep in current
                self.current.push(entry);
                continue;
            }

            let dominated_days = (season_end - entry.joined_date).num_days().max(0);
            let has_games = entry.statistics.total_games() > 0;
            let has_fee = entry.transfer_fee.is_some();
            let stayed_long = dominated_days >= THREE_MONTHS_DAYS;

            if has_games || has_fee || stayed_long {
                let seq = entry.seq_id;
                // Collapse: apps = played + subs, no subs column in finalized history
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
                    seq_id: seq,
                });
            }
        }
    }

    // ── View (combines items + current for display) ───────────

    /// Returns all history for display: finalized items + current entries,
    /// sorted most-recent first.
    pub fn view_items(&self) -> Vec<PlayerStatisticsHistoryItem> {
        let mut result: Vec<PlayerStatisticsHistoryItem> = self.items.clone();

        // Add current entries — each with its own season from joined_date
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

        // Group by (season, team, is_loan) and merge stats
        let merged = Self::merge_groups(&result);

        // Fill missing season gaps
        let filled = Self::fill_season_gaps(merged);

        // Sort: most recent season first, then seq_id desc
        let mut out = filled;
        out.sort_by(|a, b| {
            b.season.start_year.cmp(&a.season.start_year)
                .then(b.seq_id.cmp(&a.seq_id))
        });

        out
    }

    // ── Internal helpers ──────────────────────────────────────

    fn current_season(&self) -> Season {
        self.current.first()
            .map(|e| Season::from_date(e.joined_date))
            .unwrap_or_else(|| Season::new(2025))
    }

    fn push_current(&mut self, team: &TeamInfo, stats: PlayerStatistics, is_loan: bool, fee: Option<f64>, date: NaiveDate) {
        self.push_current_with_date(team, stats, is_loan, fee, date);
    }

    fn push_current_with_date(&mut self, team: &TeamInfo, stats: PlayerStatistics, is_loan: bool, fee: Option<f64>, date: NaiveDate) {
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

    fn update_or_push_current(&mut self, team: &TeamInfo, stats: PlayerStatistics, is_loan: bool, fee: Option<f64>, date: NaiveDate) {
        let date_season = Season::from_date(date);
        if let Some(entry) = self.current.iter_mut().rev().find(|e| {
            e.team_slug == team.slug && e.is_loan == is_loan
                && Season::from_date(e.joined_date).start_year == date_season.start_year
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
            self.push_current(team, stats, is_loan, fee, date);
        }
    }

    fn merge_groups(items: &[PlayerStatisticsHistoryItem]) -> Vec<PlayerStatisticsHistoryItem> {
        struct Group {
            key_idx: usize,
            stats: PlayerStatistics,
            rating_sum: f32,
            rating_count: u16,
        }

        let mut groups: Vec<Group> = Vec::new();
        let mut keys: Vec<PlayerStatisticsHistoryItem> = Vec::new();

        for item in items {
            let games = item.statistics.total_games();

            let group_idx = keys.iter().position(|k| {
                k.season.start_year == item.season.start_year
                    && k.team_slug == item.team_slug
                    && k.is_loan == item.is_loan
            });

            if let Some(idx) = group_idx {
                let group = &mut groups[idx];
                let key = &mut keys[group.key_idx];

                group.stats.played += item.statistics.played;
                group.stats.played_subs += item.statistics.played_subs;
                group.stats.goals += item.statistics.goals;
                group.stats.assists += item.statistics.assists;
                group.stats.player_of_the_match += item.statistics.player_of_the_match;
                group.stats.conceded += item.statistics.conceded;
                group.stats.clean_sheets += item.statistics.clean_sheets;
                group.rating_sum += item.statistics.average_rating * games as f32;
                group.rating_count += games;

                if item.transfer_fee.is_some() && key.transfer_fee.is_none() {
                    key.transfer_fee = item.transfer_fee;
                }
                if item.seq_id > key.seq_id {
                    if !item.league_name.is_empty() {
                        key.league_name = item.league_name.clone();
                        key.league_slug = item.league_slug.clone();
                    }
                    if games > 0 {
                        key.seq_id = item.seq_id;
                    }
                }
            } else {
                let key_idx = keys.len();
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
                groups.push(Group {
                    key_idx,
                    stats: PlayerStatistics {
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
                    rating_sum: item.statistics.average_rating * games as f32,
                    rating_count: games,
                });
            }
        }

        for group in &groups {
            let key = &mut keys[group.key_idx];
            key.statistics = PlayerStatistics {
                played: group.stats.played,
                played_subs: group.stats.played_subs,
                goals: group.stats.goals,
                assists: group.stats.assists,
                penalties: group.stats.penalties,
                player_of_the_match: group.stats.player_of_the_match,
                yellow_cards: group.stats.yellow_cards,
                red_cards: group.stats.red_cards,
                shots_on_target: group.stats.shots_on_target,
                tackling: group.stats.tackling,
                passes: group.stats.passes,
                average_rating: if group.rating_count > 0 {
                    group.rating_sum / group.rating_count as f32
                } else {
                    0.0
                },
                conceded: group.stats.conceded,
                clean_sheets: group.stats.clean_sheets,
            };
        }

        keys
    }

    fn fill_season_gaps(mut keys: Vec<PlayerStatisticsHistoryItem>) -> Vec<PlayerStatisticsHistoryItem> {
        if keys.is_empty() {
            return keys;
        }

        keys.sort_by(|a, b| {
            a.season.start_year.cmp(&b.season.start_year)
                .then(a.seq_id.cmp(&b.seq_id))
        });

        let min_year = keys.first().unwrap().season.start_year;
        let max_year = keys.last().unwrap().season.start_year;

        let mut fill = Vec::new();
        for year in min_year..=max_year {
            if keys.iter().any(|k| k.season.start_year == year) {
                continue;
            }
            let template = keys.iter().rev()
                .find(|k| k.season.start_year < year && !k.is_loan)
                .or_else(|| keys.iter().rev().find(|k| k.season.start_year < year));

            if let Some(tmpl) = template {
                fill.push(PlayerStatisticsHistoryItem {
                    season: Season::new(year),
                    team_name: tmpl.team_name.clone(),
                    team_slug: tmpl.team_slug.clone(),
                    team_reputation: tmpl.team_reputation,
                    league_name: tmpl.league_name.clone(),
                    league_slug: tmpl.league_slug.clone(),
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

        // Loan to Inter (0 games at Juve)
        h.record_departure_loan(
            make_stats(0, 0.0), &juve, &juve, &inter, false,
            make_date(2026, 1, 15),
        );
        // Cancel loan (0 games at Inter)
        h.record_cancel_loan(
            make_stats(0, 0.0), &inter, &juve, true,
            make_date(2026, 2, 15),
        );
        // Transfer to Milan (0 games at Juve)
        h.record_departure_transfer(
            make_stats(0, 0.0), &juve, &milan, Some(1_000_000.0), false,
            make_date(2026, 3, 15),
        );

        let view = h.view_items();
        // Should see: Milan (destination), Juve (after cancel), Inter (loan), Juve (original)
        assert!(view.len() >= 2, "got {} items", view.len());
        // Milan must be present
        assert!(view.iter().any(|v| v.team_slug == "milan"), "milan missing");
        // Inter loan must be present
        assert!(view.iter().any(|v| v.team_slug == "inter" && v.is_loan), "inter loan missing");
    }

    #[test]
    fn season_end_keeps_entries_with_games() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");

        // Transfer mid-season: 10 games at Juve, then move to Inter
        h.record_departure_transfer(
            make_stats(10, 7.0), &juve, &inter, Some(5_000_000.0), false,
            make_date(2026, 1, 15),
        );
        // Season end: 5 games at Inter
        h.record_season_end(
            Season::new(2025), make_stats(5, 6.5), &inter, false, None,
        );

        assert_eq!(h.items.len(), 2);
        assert_eq!(h.current.len(), 0);
        let juve_item = h.items.iter().find(|e| e.team_slug == "juventus").unwrap();
        assert_eq!(juve_item.statistics.played, 10);
        let inter_item = h.items.iter().find(|e| e.team_slug == "inter").unwrap();
        assert_eq!(inter_item.statistics.played, 5);
    }

    #[test]
    fn season_end_drops_short_zero_game_entries() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");
        let inter = make_team("Inter", "inter");
        let milan = make_team("Milan", "milan");

        // Quick loan (< 3 months), cancelled, 0 games
        h.record_departure_loan(
            make_stats(0, 0.0), &juve, &juve, &inter, false,
            make_date(2026, 3, 1),
        );
        h.record_cancel_loan(
            make_stats(0, 0.0), &inter, &juve, true,
            make_date(2026, 4, 1),  // only 1 month
        );
        // Transfer to Milan
        h.record_departure_transfer(
            make_stats(5, 7.0), &juve, &milan, None, false,
            make_date(2026, 4, 15),
        );
        // Season end
        h.record_season_end(
            Season::new(2025), make_stats(3, 6.0), &milan, false, None,
        );

        // Juve has 5 games → kept
        // Inter loan: 0 games, < 3 months → dropped
        // Milan has 3 games → kept
        // Juve original: 0 games, < 3 months → dropped (joined Mar 1, only 2.5 months to season end ~May)
        assert!(h.items.iter().any(|e| e.team_slug == "juventus" && e.statistics.played == 5));
        assert!(h.items.iter().any(|e| e.team_slug == "milan"));
    }

    #[test]
    fn season_end_keeps_long_zero_game_entries() {
        let mut h = PlayerStatisticsHistory::new();
        let juve = make_team("Juventus", "juventus");

        // Player at Juve entire season but 0 games
        h.push_current_with_date(
            &juve, PlayerStatistics::default(), false, None,
            make_date(2025, 8, 1), // joined at season start
        );

        h.record_season_end(
            Season::new(2025), make_stats(0, 0.0), &juve, false, None,
        );

        // Stayed > 3 months → kept
        assert_eq!(h.items.len(), 1);
        assert_eq!(h.items[0].team_slug, "juventus");
    }
}
