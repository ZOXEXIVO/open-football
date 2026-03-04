use chrono::NaiveDate;
use crate::league::Season;

#[derive(Debug, Clone, Default)]
pub struct PlayerStatistics {
    pub played: u16,
    pub played_subs: u16,

    pub goals: u16,
    pub assists: u16,
    pub penalties: u16,
    pub player_of_the_match: u8,
    pub yellow_cards: u8,
    pub red_cards: u8,

    pub shots_on_target: f32,
    pub tackling: f32,
    pub passes: u8,

    pub average_rating: f32,

    pub conceded: u16,
    pub clean_sheets: u16,
}

impl PlayerStatistics {
    /// Total appearances (started + substitute)
    #[inline]
    pub fn total_games(&self) -> u16 {
        self.played + self.played_subs
    }

    /// Format any rating value for display (e.g. "6.75")
    #[inline]
    pub fn format_rating(value: f32) -> String {
        format!("{:.2}", value)
    }

    /// Average rating formatted for display (e.g. "6.75")
    #[inline]
    pub fn average_rating_str(&self) -> String {
        Self::format_rating(self.average_rating)
    }

    /// Combined average rating of two stat sets (official + friendly), formatted for display
    pub fn combined_rating_str(&self, other: &PlayerStatistics) -> String {
        let games_a = self.total_games();
        let games_b = other.total_games();
        let total = games_a + games_b;
        if total == 0 {
            return "-".to_string();
        }
        let combined = (self.average_rating * games_a as f32
            + other.average_rating * games_b as f32)
            / total as f32;
        format!("{:.2}", combined)
    }
}

#[derive(Debug, Clone)]
pub struct PlayerStatisticsHistory {
    pub items: Vec<PlayerStatisticsHistoryItem>,
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
    /// When this history entry was created (for ordering)
    pub created_at: NaiveDate,
}

impl Default for PlayerStatisticsHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerStatisticsHistory {
    pub fn new() -> Self {
        PlayerStatisticsHistory { items: Vec::new() }
    }

    /// Push a new history entry, deduplicating against existing entries for the
    /// same season + team to preserve correct created_at ordering.
    pub fn push_or_replace(&mut self, item: PlayerStatisticsHistoryItem) {
        let new_games = item.statistics.played + item.statistics.played_subs;

        // Find existing entry for same season + team with 0 games
        let zero_games_idx = self.items.iter().position(|existing| {
            existing.season.start_year == item.season.start_year
                && existing.team_slug == item.team_slug
                && (existing.statistics.played + existing.statistics.played_subs) == 0
        });

        if let Some(idx) = zero_games_idx {
            // Replace 0-game placeholder only if new entry has actual games;
            // preserve original created_at, transfer info, and loan flag so sort order,
            // transfer markers, and loan status stay correct
            if new_games > 0 {
                let original_created_at = self.items[idx].created_at;
                let transfer_fee = self.items[idx].transfer_fee;
                let original_is_loan = self.items[idx].is_loan;
                self.items[idx] = item;
                self.items[idx].created_at = original_created_at;
                self.items[idx].transfer_fee = transfer_fee;
                // The 0-game placeholder was created by the transfer system,
                // which is the authoritative source of loan status.
                // Preserve it so loan spells aren't lost when the contract
                // type changes before the season snapshot runs.
                self.items[idx].is_loan = original_is_loan;
            }
        } else if new_games == 0 {
            // New 0-game entry — only push if no entry exists at all for this season + team;
            // avoids duplicates that corrupt created_at when merged in display layer
            let any_existing = self.items.iter().any(|existing| {
                existing.season.start_year == item.season.start_year
                    && existing.team_slug == item.team_slug
            });
            if !any_existing {
                self.items.push(item);
            }
        } else {
            self.items.push(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn make_stats(played: u16, played_subs: u16, goals: u16, rating: f32) -> PlayerStatistics {
        let mut s = PlayerStatistics::default();
        s.played = played;
        s.played_subs = played_subs;
        s.goals = goals;
        s.average_rating = rating;
        s
    }

    fn make_history_item(
        start_year: u16,
        slug: &str,
        is_loan: bool,
        played: u16,
    ) -> PlayerStatisticsHistoryItem {
        let mut stats = PlayerStatistics::default();
        stats.played = played;
        PlayerStatisticsHistoryItem {
            season: Season::new(start_year),
            team_name: slug.to_string(),
            team_slug: slug.to_string(),
            team_reputation: 100,
            league_name: "League".to_string(),
            league_slug: "league".to_string(),
            is_loan,
            transfer_fee: None,
            statistics: stats,
            created_at: make_date(start_year as i32 + 1, 6, 1),
        }
    }

    // === PlayerStatistics ===

    #[test]
    fn total_games_sums_played_and_subs() {
        let s = make_stats(20, 5, 3, 7.0);
        assert_eq!(s.total_games(), 25);
    }

    #[test]
    fn total_games_zero_when_empty() {
        let s = PlayerStatistics::default();
        assert_eq!(s.total_games(), 0);
    }

    #[test]
    fn format_rating_two_decimals() {
        assert_eq!(PlayerStatistics::format_rating(6.5), "6.50");
        assert_eq!(PlayerStatistics::format_rating(7.123), "7.12");
        assert_eq!(PlayerStatistics::format_rating(0.0), "0.00");
    }

    #[test]
    fn average_rating_str_delegates_to_format_rating() {
        let s = make_stats(10, 0, 0, 6.75);
        assert_eq!(s.average_rating_str(), "6.75");
    }

    #[test]
    fn combined_rating_str_zero_games_returns_dash() {
        let a = PlayerStatistics::default();
        let b = PlayerStatistics::default();
        assert_eq!(a.combined_rating_str(&b), "-");
    }

    #[test]
    fn combined_rating_str_one_side_zero() {
        let a = make_stats(10, 0, 0, 7.0);
        let b = PlayerStatistics::default();
        assert_eq!(a.combined_rating_str(&b), "7.00");
    }

    #[test]
    fn combined_rating_str_weighted_average() {
        let a = make_stats(10, 0, 0, 7.0);
        let b = make_stats(10, 0, 0, 6.0);
        // (7.0*10 + 6.0*10) / 20 = 6.5
        assert_eq!(a.combined_rating_str(&b), "6.50");
    }

    #[test]
    fn combined_rating_str_unequal_games() {
        let a = make_stats(30, 0, 0, 7.0);
        let b = make_stats(10, 0, 0, 6.0);
        // (7.0*30 + 6.0*10) / 40 = 6.75
        assert_eq!(a.combined_rating_str(&b), "6.75");
    }

    #[test]
    fn combined_rating_str_includes_subs() {
        let a = make_stats(8, 2, 0, 7.0); // 10 total
        let b = make_stats(5, 5, 0, 6.0); // 10 total
        assert_eq!(a.combined_rating_str(&b), "6.50");
    }

    // === PlayerStatisticsHistory ===

    #[test]
    fn new_history_is_empty() {
        let h = PlayerStatisticsHistory::new();
        assert!(h.items.is_empty());
    }

    #[test]
    fn default_history_is_empty() {
        let h = PlayerStatisticsHistory::default();
        assert!(h.items.is_empty());
    }

    #[test]
    fn push_or_replace_basic_push() {
        let mut h = PlayerStatisticsHistory::new();
        h.push_or_replace(make_history_item(2031, "inter", false, 25));
        assert_eq!(h.items.len(), 1);
        assert_eq!(h.items[0].statistics.played, 25);
    }

    #[test]
    fn push_or_replace_zero_game_dedup() {
        let mut h = PlayerStatisticsHistory::new();
        h.push_or_replace(make_history_item(2031, "inter", false, 0));
        h.push_or_replace(make_history_item(2031, "inter", false, 0));
        assert_eq!(h.items.len(), 1);
    }

    #[test]
    fn push_or_replace_replaces_zero_game_entry() {
        let mut h = PlayerStatisticsHistory::new();

        let mut placeholder = make_history_item(2031, "inter", true, 0);
        placeholder.transfer_fee = Some(5_000_000.0);
        placeholder.created_at = make_date(2031, 8, 1);
        h.push_or_replace(placeholder);

        let actual = make_history_item(2031, "inter", false, 25);
        h.push_or_replace(actual);

        assert_eq!(h.items.len(), 1);
        assert_eq!(h.items[0].statistics.played, 25);
        // Preserves original fields
        assert_eq!(h.items[0].created_at, make_date(2031, 8, 1));
        assert_eq!(h.items[0].transfer_fee, Some(5_000_000.0));
        assert!(h.items[0].is_loan); // preserved from placeholder
    }

    #[test]
    fn push_or_replace_different_teams_same_season() {
        let mut h = PlayerStatisticsHistory::new();
        h.push_or_replace(make_history_item(2031, "inter", false, 10));
        h.push_or_replace(make_history_item(2031, "juventus", false, 15));
        assert_eq!(h.items.len(), 2);
    }

    #[test]
    fn push_or_replace_different_seasons_same_team() {
        let mut h = PlayerStatisticsHistory::new();
        h.push_or_replace(make_history_item(2030, "inter", false, 10));
        h.push_or_replace(make_history_item(2031, "inter", false, 15));
        assert_eq!(h.items.len(), 2);
    }

    #[test]
    fn push_or_replace_both_with_games_keeps_both() {
        let mut h = PlayerStatisticsHistory::new();
        h.push_or_replace(make_history_item(2031, "inter", false, 10));
        h.push_or_replace(make_history_item(2031, "inter", false, 5));
        assert_eq!(h.items.len(), 2);
    }

    // === Season ===

    #[test]
    fn season_new_and_display() {
        let s = Season::new(2031);
        assert_eq!(s.start_year, 2031);
        assert_eq!(s.display, "2031/32");
    }

    #[test]
    fn season_century_wrap() {
        let s = Season::new(2099);
        // 2100 % 100 = 0, displayed as single digit
        assert_eq!(s.display, "2099/0");
    }

    #[test]
    fn season_from_date_after_august() {
        // August 2032 → season 2032/33
        let s = Season::from_date(make_date(2032, 8, 15));
        assert_eq!(s.start_year, 2032);
    }

    #[test]
    fn season_from_date_before_august() {
        // March 2032 → season 2031/32
        let s = Season::from_date(make_date(2032, 3, 15));
        assert_eq!(s.start_year, 2031);
    }

    #[test]
    fn season_from_date_august_boundary() {
        // August 1 → new season starts
        let s = Season::from_date(make_date(2032, 8, 1));
        assert_eq!(s.start_year, 2032);
    }

    #[test]
    fn season_from_date_july_boundary() {
        // July 31 → previous season
        let s = Season::from_date(make_date(2032, 7, 31));
        assert_eq!(s.start_year, 2031);
    }
}
