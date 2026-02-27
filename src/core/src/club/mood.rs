#[derive(Debug, Clone)]
pub struct ClubMood {
    pub state: ClubMoodState,
}

impl ClubMood {
    pub fn default() -> Self {
        ClubMood {
            state: ClubMoodState::Normal,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ClubMoodState {
    Poor,
    Normal,
    Good,
    Excellent,
}
