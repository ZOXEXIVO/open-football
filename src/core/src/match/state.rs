use crate::r#match::MatchState;

pub struct GameState {
    pub match_state: MatchState,
}

impl GameState {
    pub fn new() -> Self {
        GameState {
            match_state: MatchState::Initial,
        }
    }

    pub fn set(&mut self, match_state: MatchState) {
        self.match_state = match_state;
    }
}
