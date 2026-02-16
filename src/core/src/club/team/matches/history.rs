use crate::r#match::TeamScore;
use chrono::NaiveDateTime;

const DEFAULT_MATCH_LIST_SIZE: usize = 10;

#[derive(Debug)]
pub struct MatchHistory {
    items: Vec<MatchHistoryItem>,
}

impl Default for MatchHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl MatchHistory {
    pub fn new() -> Self {
        MatchHistory {
            items: Vec::with_capacity(DEFAULT_MATCH_LIST_SIZE),
        }
    }

    pub fn add(&mut self, item: MatchHistoryItem) {
        self.items.push(item);
    }

    pub fn items(&self) -> &[MatchHistoryItem] {
        &self.items
    }
}

#[derive(Debug)]
pub struct MatchHistoryItem {
    pub date: NaiveDateTime,
    pub rival_team_id: u32,
    pub score: (TeamScore, TeamScore),
}

impl MatchHistoryItem {
    pub fn new(date: NaiveDateTime, rival_team_id: u32, score: (TeamScore, TeamScore)) -> Self {
        MatchHistoryItem {
            date,
            rival_team_id,
            score,
        }
    }
}
