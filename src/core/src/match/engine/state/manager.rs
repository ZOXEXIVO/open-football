use crate::r#match::{MatchContext, MatchField, MatchState, PlayMatchStateResult};

pub struct StateManager {
    current_state: MatchState,
}

impl Default for StateManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StateManager {
    pub fn new() -> Self {
        StateManager {
            current_state: MatchState::Initial,
        }
    }

    pub fn current(&self) -> MatchState {
        self.current_state
    }

    pub fn next(&mut self) -> Option<MatchState> {
        let next_state: MatchState = Self::get_next_state(self.current_state);

        match next_state {
            MatchState::End => None,
            _ => {
                self.current_state = next_state;
                Some(self.current_state)
            }
        }
    }

    fn get_next_state(current_state: MatchState) -> MatchState {
        match current_state {
            MatchState::Initial => MatchState::FirstHalf,
            MatchState::FirstHalf => MatchState::HalfTime,
            MatchState::HalfTime => MatchState::SecondHalf,
            MatchState::SecondHalf => MatchState::End,  // Regular matches end after second half
            MatchState::ExtraTime => MatchState::PenaltyShootout,
            MatchState::PenaltyShootout => MatchState::End,
            MatchState::End => MatchState::End,
        }
    }

    pub fn handle_state_finish(
        context: &mut MatchContext,
        field: &mut MatchField,
        play_result: PlayMatchStateResult,
    ) {
        if context.state.match_state.need_swap_squads() {
            field.swap_squads();
        }

        if play_result.additional_time > 0 {
            context.add_time(play_result.additional_time);
        }

        match context.state.match_state {
            MatchState::Initial => {}
            MatchState::FirstHalf => {
                Self::play_rest_time(field);
            }
            MatchState::HalfTime => {
                // Half-time finished - reset time for second half
                context.reset_period_time();
            }
            MatchState::SecondHalf => {
                // Second half finished - ready for extra time if needed
            }
            MatchState::ExtraTime => {}
            MatchState::PenaltyShootout => {}
            _ => {}
        }
    }

    fn play_rest_time(field: &mut MatchField) {
        field.players.iter_mut().for_each(|p| {
            p.player_attributes.rest(1000);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::MatchState;

    #[test]
    fn test_state_manager_new() {
        let state_manager = StateManager::new();
        assert_eq!(state_manager.current(), MatchState::Initial);
    }

    #[test]
    fn test_state_manager_next() {
        let mut state_manager = StateManager::new();
        assert_eq!(state_manager.next(), Some(MatchState::FirstHalf));
        assert_eq!(state_manager.next(), Some(MatchState::HalfTime));
        assert_eq!(state_manager.next(), Some(MatchState::SecondHalf));
        assert_eq!(state_manager.next(), None); // Regular match ends after second half
        assert_eq!(state_manager.next(), None); // No more states after match ends
    }
}
