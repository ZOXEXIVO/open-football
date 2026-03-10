/// Info about a team context for recording history events.
#[derive(Debug, Clone)]
pub struct TeamInfo {
    pub name: String,
    pub slug: String,
    pub reputation: u16,
    pub league_name: String,
    pub league_slug: String,
}

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

    /// Merge another stat set into this one (FM-style: combine stints at same club in one season).
    /// Weighted-averages the rating, sums everything else.
    pub fn merge_from(&mut self, other: &PlayerStatistics) {
        let self_games = self.total_games() as f32;
        let other_games = other.total_games() as f32;
        let total = self_games + other_games;

        // Weighted average rating (compute before mutating counts)
        if total > 0.0 {
            self.average_rating =
                (self.average_rating * self_games + other.average_rating * other_games) / total;
        }

        self.played += other.played;
        self.played_subs += other.played_subs;
        self.goals += other.goals;
        self.assists += other.assists;
        self.penalties += other.penalties;
        self.player_of_the_match += other.player_of_the_match;
        self.yellow_cards += other.yellow_cards;
        self.red_cards += other.red_cards;
        self.shots_on_target += other.shots_on_target;
        self.tackling += other.tackling;
        self.passes += other.passes;
        self.conceded += other.conceded;
        self.clean_sheets += other.clean_sheets;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stats(played: u16, played_subs: u16, goals: u16, rating: f32) -> PlayerStatistics {
        let mut s = PlayerStatistics::default();
        s.played = played;
        s.played_subs = played_subs;
        s.goals = goals;
        s.average_rating = rating;
        s
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
        assert_eq!(a.combined_rating_str(&b), "6.50");
    }

    #[test]
    fn combined_rating_str_unequal_games() {
        let a = make_stats(30, 0, 0, 7.0);
        let b = make_stats(10, 0, 0, 6.0);
        assert_eq!(a.combined_rating_str(&b), "6.75");
    }

    #[test]
    fn combined_rating_str_includes_subs() {
        let a = make_stats(8, 2, 0, 7.0);
        let b = make_stats(5, 5, 0, 6.0);
        assert_eq!(a.combined_rating_str(&b), "6.50");
    }
}
