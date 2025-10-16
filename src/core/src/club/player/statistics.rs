use crate::league::Season;

#[derive(Debug, Default)]
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
}

#[derive(Debug)]
pub struct PlayerStatisticsHistory {
    pub items: Vec<PlayerStatisticsHistoryItem>,
}

#[derive(Debug)]
pub struct PlayerStatisticsHistoryItem {
    pub season: Season,
    pub statistics: PlayerStatistics,
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
}
