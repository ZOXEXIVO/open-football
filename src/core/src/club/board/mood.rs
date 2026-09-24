#[derive(Debug, Clone)]
pub struct BoardMood {
    pub state: BoardMoodState,
}

impl Default for BoardMood {
    fn default() -> Self {
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
