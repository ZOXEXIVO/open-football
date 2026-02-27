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

    /// Push a new history entry. If an entry already exists for the same season
    /// and team with 0 games played, replace it instead of creating a duplicate
    /// (avoids empty breaks in history when a player's club switches league
    /// or when a player transfers without playing).
    pub fn push_or_replace(&mut self, item: PlayerStatisticsHistoryItem) {
        let zero_games_idx = self.items.iter().position(|existing| {
            existing.season.start_year == item.season.start_year
                && existing.team_slug == item.team_slug
                && (existing.statistics.played + existing.statistics.played_subs) == 0
        });

        if let Some(idx) = zero_games_idx {
            self.items[idx] = item;
        } else {
            self.items.push(item);
        }
    }
}
