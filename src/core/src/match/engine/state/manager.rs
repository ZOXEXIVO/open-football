use crate::r#match::engine::goal::assign_kickoff;
use crate::r#match::{MatchContext, MatchField, MatchState, PlayMatchStateResult, PlayerSide, Score, TeamsTactics};

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

    /// Advance to the next state. Needs `score` + `is_knockout` so we can
    /// decide whether SecondHalf / ExtraTime lead to the End or into the
    /// tiebreak branch (extra time → shootout).
    pub fn next(&mut self, score: &Score, is_knockout: bool) -> Option<MatchState> {
        let next_state: MatchState = Self::get_next_state(self.current_state, score, is_knockout);

        match next_state {
            MatchState::End => None,
            _ => {
                self.current_state = next_state;
                Some(self.current_state)
            }
        }
    }

    fn get_next_state(current_state: MatchState, score: &Score, is_knockout: bool) -> MatchState {
        match current_state {
            MatchState::Initial => MatchState::FirstHalf,
            MatchState::FirstHalf => MatchState::HalfTime,
            MatchState::HalfTime => MatchState::SecondHalf,
            MatchState::SecondHalf => {
                // League / friendly matches always end here — draws are fine.
                // Knockout ties that are level after 90 min go to extra time.
                if is_knockout && score.is_tied() {
                    MatchState::ExtraTime
                } else {
                    MatchState::End
                }
            }
            MatchState::ExtraTime => {
                // Still level after 120 min → penalty shootout.
                if score.is_tied() {
                    MatchState::PenaltyShootout
                } else {
                    MatchState::End
                }
            }
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
            context.tactics = TeamsTactics::from_field(field);
        }

        if play_result.additional_time > 0 {
            context.add_time(play_result.additional_time);
        }

        match context.state.match_state {
            MatchState::Initial => {}
            MatchState::FirstHalf => {
                Self::play_rest_time(field);

                field.reset_players_positions();
                field.ball.reset();
            }
            MatchState::HalfTime => {
                // Half-time finished - reset time for second half
                context.reset_period_time();
                field.reset_players_positions();
                field.ball.reset();
                // Second half kicks off — Away team (now playing Left
                // after the halftime swap) takes it.
                assign_kickoff(field, PlayerSide::Left);
            }
            MatchState::SecondHalf => {
                // Second half finished. If the tie rolls to extra time the
                // engine loop will call this state next; rest the squad a bit.
                if context.is_knockout && context.score.is_tied() {
                    Self::play_rest_time(field);
                    context.reset_period_time();
                    field.reset_players_positions();
                    field.ball.reset();
                    // Extra time kicks off — pick Left by convention.
                    assign_kickoff(field, PlayerSide::Left);
                }
            }
            MatchState::ExtraTime => {
                // ET complete — positions reset only matters if shootout follows,
                // but the shootout resolver rebuilds everything it needs.
                context.reset_period_time();
            }
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

    fn tied_score() -> Score {
        Score::new(1, 2)
    }

    fn decided_score() -> Score {
        use crate::r#match::engine::result::TeamScore;
        Score {
            home_team: TeamScore::new_with_score(1, 1),
            away_team: TeamScore::new_with_score(2, 0),
            details: Vec::new(),
            home_shootout: 0,
            away_shootout: 0,
        }
    }

    #[test]
    fn test_state_manager_new() {
        let state_manager = StateManager::new();
        assert_eq!(state_manager.current(), MatchState::Initial);
    }

    #[test]
    fn league_match_ends_after_second_half_even_when_tied() {
        let mut state_manager = StateManager::new();
        let score = tied_score();
        assert_eq!(state_manager.next(&score, false), Some(MatchState::FirstHalf));
        assert_eq!(state_manager.next(&score, false), Some(MatchState::HalfTime));
        assert_eq!(state_manager.next(&score, false), Some(MatchState::SecondHalf));
        assert_eq!(state_manager.next(&score, false), None);
    }

    #[test]
    fn knockout_tie_triggers_extra_time_then_shootout() {
        let mut state_manager = StateManager::new();
        let score = tied_score();
        assert_eq!(state_manager.next(&score, true), Some(MatchState::FirstHalf));
        assert_eq!(state_manager.next(&score, true), Some(MatchState::HalfTime));
        assert_eq!(state_manager.next(&score, true), Some(MatchState::SecondHalf));
        assert_eq!(state_manager.next(&score, true), Some(MatchState::ExtraTime));
        assert_eq!(state_manager.next(&score, true), Some(MatchState::PenaltyShootout));
        assert_eq!(state_manager.next(&score, true), None);
    }

    #[test]
    fn knockout_decided_in_regulation_ends_early() {
        let mut state_manager = StateManager::new();
        let score = decided_score();
        assert_eq!(state_manager.next(&score, true), Some(MatchState::FirstHalf));
        assert_eq!(state_manager.next(&score, true), Some(MatchState::HalfTime));
        assert_eq!(state_manager.next(&score, true), Some(MatchState::SecondHalf));
        assert_eq!(state_manager.next(&score, true), None);
    }
}
