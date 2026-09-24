#[derive(Debug, Clone)]
pub struct ClubMood {
    pub state: ClubMoodState,
}

impl Default for ClubMood {
    fn default() -> Self {
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
