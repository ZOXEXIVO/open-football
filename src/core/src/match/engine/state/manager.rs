use crate::r#match::engine::flow::field::ResetReason;
use crate::r#match::engine::goal::assign_kickoff;
use crate::r#match::{
    MatchContext, MatchField, MatchState, Score, TeamsTactics,
};

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
    /// decide whether the second half / extra time lead to the end or into
    /// the tiebreak branch (extra time → shootout).
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
                    MatchState::ExtraTimeFirst
                } else {
                    MatchState::End
                }
            }
            MatchState::ExtraTimeFirst => MatchState::ExtraTimeInterval,
            MatchState::ExtraTimeInterval => MatchState::ExtraTimeSecond,
            MatchState::ExtraTimeSecond => {
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

    pub fn handle_state_finish(context: &mut MatchContext, field: &mut MatchField) {
        if context.state.match_state.need_swap_squads() {
            field.swap_squads();
            context.tactics = TeamsTactics::from_field(field);
            // Side swap rebinds positional roles per side, so the
            // cached per-team composites must be recomputed before
            // the next tactical refresh.
            context.invalidate_skill_aggregates();
        }

        match context.state.match_state {
            MatchState::FirstHalf | MatchState::ExtraTimeFirst => {
                if context.state.match_state == MatchState::FirstHalf {
                    Self::play_rest_time(field);
                }

                // ⚠ Overwritten ten milliseconds later by the interval arm
                // below, after `swap_squads`, and never sampled in between —
                // see [`ResetReason::PeriodDead`].
                field.reset_players_positions(ResetReason::PeriodDead);
                field.ball.reset();
            }
            MatchState::HalfTime | MatchState::ExtraTimeInterval => {
                context.reset_period_time();
                field.reset_players_positions(ResetReason::Period);
                field.ball.reset();
                Self::kick_off_period(context, field);
            }
            MatchState::SecondHalf => {
                // Second half finished. If the tie rolls to extra time the
                // engine loop will call this state next; rest the squad a bit.
                if context.is_knockout && context.score.is_tied() {
                    Self::play_rest_time(field);
                    context.reset_period_time();
                    context.grant_extra_time_allowance();
                    field.reset_players_positions(ResetReason::Period);
                    field.ball.reset();
                    Self::kick_off_period(context, field);
                }
            }
            MatchState::ExtraTimeSecond => {
                // ET complete — positions reset only matters if shootout follows,
                // but the shootout resolver rebuilds everything it needs.
                context.reset_period_time();
            }
            MatchState::Initial | MatchState::PenaltyShootout | MatchState::End => {}
        }
    }

    /// The home side kicks off the match and the sides take turns after
    /// that, each period kicked off by the team that did not kick off the
    /// one before it — which is what the laws' coin toss and change of ends
    /// come to over a match.
    pub fn kick_off_period(context: &mut MatchContext, field: &mut MatchField) {
        let team = match context.period_kickoff_team {
            Some(last) if last == field.home_team_id => field.away_team_id,
            _ => field.home_team_id,
        };
        context.period_kickoff_team = Some(team);
        let side = field.side_of(team);
        assign_kickoff(field, side, None);
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
        assert_eq!(
            state_manager.next(&score, false),
            Some(MatchState::FirstHalf)
        );
        assert_eq!(
            state_manager.next(&score, false),
            Some(MatchState::HalfTime)
        );
        assert_eq!(
            state_manager.next(&score, false),
            Some(MatchState::SecondHalf)
        );
        assert_eq!(state_manager.next(&score, false), None);
    }

    #[test]
    fn knockout_tie_triggers_extra_time_then_shootout() {
        let mut state_manager = StateManager::new();
        let score = tied_score();
        assert_eq!(
            state_manager.next(&score, true),
            Some(MatchState::FirstHalf)
        );
        assert_eq!(state_manager.next(&score, true), Some(MatchState::HalfTime));
        assert_eq!(
            state_manager.next(&score, true),
            Some(MatchState::SecondHalf)
        );
        assert_eq!(
            state_manager.next(&score, true),
            Some(MatchState::ExtraTimeFirst)
        );
        assert_eq!(
            state_manager.next(&score, true),
            Some(MatchState::ExtraTimeInterval)
        );
        assert_eq!(
            state_manager.next(&score, true),
            Some(MatchState::ExtraTimeSecond)
        );
        assert_eq!(
            state_manager.next(&score, true),
            Some(MatchState::PenaltyShootout)
        );
        assert_eq!(state_manager.next(&score, true), None);
    }

    #[test]
    fn knockout_decided_in_regulation_ends_early() {
        let mut state_manager = StateManager::new();
        let score = decided_score();
        assert_eq!(
            state_manager.next(&score, true),
            Some(MatchState::FirstHalf)
        );
        assert_eq!(state_manager.next(&score, true), Some(MatchState::HalfTime));
        assert_eq!(
            state_manager.next(&score, true),
            Some(MatchState::SecondHalf)
        );
        assert_eq!(state_manager.next(&score, true), None);
    }
}
