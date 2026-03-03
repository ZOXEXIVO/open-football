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
            // preserve original created_at and transfer info so sort order
            // and transfer markers stay correct
            if new_games > 0 {
                let original_created_at = self.items[idx].created_at;
                let transfer_fee = self.items[idx].transfer_fee;
                self.items[idx] = item;
                self.items[idx].created_at = original_created_at;
                self.items[idx].transfer_fee = transfer_fee;
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
