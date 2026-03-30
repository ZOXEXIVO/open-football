use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::EventCollection;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer};
use crate::PlayerFieldPositionGroup;

use std::fmt::Display;
use std::fmt::Formatter;

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
        // Decay memory every 100 ticks
        let current_tick = context.current_tick();
        if current_tick > 0 && current_tick % 100 == 0 {
            player.memory.decay(current_tick);
        }

        let player_position_group = player.tactical_position.current_position.position_group();

        let state_change_result =
            player_position_group.process(player.in_state_time, player, context, tick_context);

        if let Some(state) = state_change_result.state {
            Self::change_state(player, state);
        } else {
            player.in_state_time += 1;
        }

        if let Some(velocity) = state_change_result.velocity {
            let max_speed = if player_position_group == PlayerFieldPositionGroup::Goalkeeper {
                let speed_context = match player.state {
                    PlayerState::Goalkeeper(GoalkeeperState::Diving)
                    | PlayerState::Goalkeeper(GoalkeeperState::PreparingForSave)
                    | PlayerState::Goalkeeper(GoalkeeperState::Jumping) => {
                        GoalkeeperSpeedContext::Explosive
                    }
                    PlayerState::Goalkeeper(GoalkeeperState::Catching)
                    | PlayerState::Goalkeeper(GoalkeeperState::ComingOut)
                    | PlayerState::Goalkeeper(GoalkeeperState::UnderPressure) => {
                        GoalkeeperSpeedContext::Active
                    }
                    PlayerState::Goalkeeper(GoalkeeperState::Attentive)
                    | PlayerState::Goalkeeper(GoalkeeperState::Standing)
                    | PlayerState::Goalkeeper(GoalkeeperState::ReturningToGoal) => {
                        GoalkeeperSpeedContext::Positioning
                    }
                    _ => GoalkeeperSpeedContext::Casual,
                };
                player.skills.goalkeeper_max_speed(
                    player.player_attributes.condition,
                    speed_context,
                )
            } else {
                player.skills.max_speed_with_condition(
                    player.player_attributes.condition,
                )
            };

            let velocity_sq = velocity.norm_squared();
            let max_speed_sq = max_speed * max_speed;

            if velocity_sq > max_speed_sq && velocity_sq > 0.0 {
                // Velocity is too high, clamp it to max_speed
                let velocity_magnitude = velocity_sq.sqrt();
                player.velocity = velocity * (max_speed / velocity_magnitude);
            } else {
                player.velocity = velocity;
            }
        }

        state_change_result.events
    }

    fn change_state(player: &mut MatchPlayer, state: PlayerState) {
        player.in_state_time = 0;
        player.state = state;
    }
}
