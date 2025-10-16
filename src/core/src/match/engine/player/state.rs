use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::EventCollection;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer};
use crate::PlayerFieldPositionGroup;
use log::error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerState {
    Injured,
    Goalkeeper(GoalkeeperState),
    Defender(DefenderState),
    Midfielder(MidfielderState),
    Forward(ForwardState),
}

impl Display for PlayerState {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            PlayerState::Injured => write!(f, "Injured"),
            PlayerState::Goalkeeper(state) => write!(f, "Goalkeeper: {}", state),
            PlayerState::Defender(state) => write!(f, "Defender: {}", state),
            PlayerState::Midfielder(state) => write!(f, "Midfielder: {}", state),
            PlayerState::Forward(state) => write!(f, "Forward: {}", state),
        }
    }
}

pub struct PlayerMatchState;

impl PlayerMatchState {
    pub fn process(
        player: &mut MatchPlayer,
        context: &MatchContext,
        tick_context: &GameTickContext,
    ) -> EventCollection {
        let player_position_group = player.tactical_position.current_position.position_group();

        let state_change_result =
            player_position_group.process(player.in_state_time, player, context, tick_context);

        if let Some(state) = state_change_result.state {
            #[cfg(debug_assertions)]
            {
                if !Self::validate_state(state, &player_position_group) {
                    error!(
                        "invalid state change {:?} -> {:?} for {:?}",
                        player.state, state, player_position_group
                    );
                }
            }

            Self::change_state(player, state);
        } else {
            player.in_state_time += 1;
        }

        if let Some(velocity) = state_change_result.velocity {
            player.velocity = velocity;
        }

        state_change_result.events
    }

    fn change_state(player: &mut MatchPlayer, state: PlayerState) {
        player.in_state_time = 0;
        player.state = state;
    }

    fn validate_state(
        player_state: PlayerState,
        position_group: &PlayerFieldPositionGroup,
    ) -> bool {
        match (player_state, position_group) {
            (PlayerState::Injured, _) => true, // Injured state is valid for all position groups
            (PlayerState::Goalkeeper(_), PlayerFieldPositionGroup::Goalkeeper) => true,
            (PlayerState::Defender(_), PlayerFieldPositionGroup::Defender) => true,
            (PlayerState::Midfielder(_), PlayerFieldPositionGroup::Midfielder) => true,
            (PlayerState::Forward(_), PlayerFieldPositionGroup::Forward) => true,
            _ => false, // Any other combination is invalid
        }
    }
}
