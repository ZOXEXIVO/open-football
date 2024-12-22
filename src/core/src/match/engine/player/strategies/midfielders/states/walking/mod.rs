use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use crate::IntegerUtils;
use nalgebra::Vector3;
use std::sync::LazyLock;

static DEFENDER_WALKING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_walking_data.json")));

#[derive(Default)]
pub struct MidfielderWalkingState {}

impl StateProcessingHandler for MidfielderWalkingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }
        
        if ctx.team().is_control_ball() {
            if ctx.ball().distance() < 100.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }
            
            if ctx.ball().is_towards_player_with_angle(0.8) && ctx.ball().distance() < 250.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }
            
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        } else {
            if ctx.ball().distance() < 100.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }
            
            if ctx.ball().is_towards_player_with_angle(0.8) && ctx.ball().distance() < 250.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            let nearby_opponents = ctx.players().opponents().nearby(150.0).collect::<Vec<_>>();
            if !nearby_opponents.is_empty() {
                // If there are nearby opponents, assess the situation
                let ball_distance = ctx.ball().distance();

                let mut closest_opponent_distance = f32::MAX;
                for opponent in &nearby_opponents {
                    let distance = ctx.player().distance_to_player(opponent.id);
                    if distance < closest_opponent_distance {
                        closest_opponent_distance = distance;
                    }
                }

                if ball_distance < 100.0 && closest_opponent_distance < 50.0 {
                    // If the ball is close and an opponent is very close, transition to Tackling state
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Tackling,
                    ));
                } else if ball_distance < 200.0 {
                    // If the ball is moderately close, transition to Pressing state
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Pressing,
                    ));
                } else {
                    // If the ball is far, transition to Running state to get closer to the action
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Running,
                    ));
                }
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Impl ement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Wander {
                target: ctx.player.start_position,
                radius: IntegerUtils::random(5, 150) as f32,
                jitter: IntegerUtils::random(0, 2) as f32,
                distance: IntegerUtils::random(10, 150) as f32,
                angle: IntegerUtils::random(0, 180) as f32,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}
