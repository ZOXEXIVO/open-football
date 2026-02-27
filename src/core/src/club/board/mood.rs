#[derive(Debug, Clone)]
pub struct BoardMood {
    pub state: BoardMoodState,
}

impl BoardMood {
    pub fn default() -> Self {
        BoardMood {
            state: BoardMoodState::Normal,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BoardMoodState {
    Poor,
    Normal,
    Good,
    Excellent,
}
