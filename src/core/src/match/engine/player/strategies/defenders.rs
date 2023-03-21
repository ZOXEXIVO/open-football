﻿use crate::r#match::position::FieldPosition;
use crate::r#match::{
    BallState, MatchObjectsPositions, MatchPlayer, MatchState, PlayerUpdateEvent, SteeringBehavior,
    SteeringOutput,
};
use nalgebra::Vector2;

pub struct DefenderStrategies {}

impl DefenderStrategies {
    pub fn detect_velocity(
        current_time: u64,
        player: &MatchPlayer,
        result: &mut Vec<PlayerUpdateEvent>,
        objects_positions: &MatchObjectsPositions,
        state: &MatchState,
    ) -> Vector2<f32> {
        let behavior = match state.ball_state {
            Some(ball_state) => match ball_state {
                BallState::HomeSide => {
                    if player.is_home {
                        DefenderBehavior::Defend
                    } else {
                        DefenderBehavior::Support
                    }
                }
                BallState::AwaySide => {
                    if player.is_home {
                        DefenderBehavior::Support
                    } else {
                        DefenderBehavior::Defend
                    }
                }
            },
            None => DefenderBehavior::Idle,
        };

        let steering_output = match behavior {
            DefenderBehavior::Defend => {
                if DefenderStrategies::is_on_defending_half(player, state) {
                    SteeringBehavior::Seek {
                        target: objects_positions.ball_positions,
                    }
                    .calculate(player)
                } else {
                    SteeringBehavior::Arrive {
                        target: FieldPosition::new(0.0, 0.0),
                        slowing_distance: 2.0,
                    }
                    .calculate(player)
                }
            }
            DefenderBehavior::Support => SteeringBehavior::Arrive {
                target: player.start_position,
                slowing_distance: 2.0,
            }
            .calculate(player),
            DefenderBehavior::Idle => SteeringOutput {
                velocity: FieldPosition::new(0.0, 0.0),
                rotation: 0.0,
            },
        };

        Vector2::new(steering_output.velocity.x, steering_output.velocity.y)
    }

    fn is_on_defending_half(player: &MatchPlayer, state: &MatchState) -> bool {
        match state.ball_state {
            Some(ball_state) => ball_state == BallState::HomeSide && player.is_home,
            None => false,
        }
    }
}

enum DefenderBehavior {
    Defend,
    Support,
    Idle,
}