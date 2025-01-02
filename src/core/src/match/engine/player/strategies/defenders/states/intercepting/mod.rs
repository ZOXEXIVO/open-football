use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{ConditionContext, PlayerDistanceFromStartPosition, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;
use rand::Rng;
use std::sync::LazyLock;

static _DEFENDER_INTERCEPTING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_intercepting_data.json")));

#[derive(Default)]
pub struct DefenderInterceptingState {}

impl StateProcessingHandler for DefenderInterceptingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        if ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }


        if ctx.ball().distance() < 15.0 {
            if ctx.tick_context.ball.is_owned {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            } else if self.calculate_tackling_success(ctx) {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Running,
                    Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                ));
            }
        }

        if ctx.team().is_control_ball() {
            return if ctx.ball().on_own_side() {
                Some(StateChangeResult::with_defender_state(
                    DefenderState::Running,
                ))
            } else {
                Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ))
            }
        } else {
            if ctx.ball().on_own_side() {
                if ctx.ball().distance() < 150.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
            } else {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }
        }

        if !self.can_reach_before_opponent(ctx) {
            // If not, transition to Pressing or HoldingLine state
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Pursuit {
                target: ctx.tick_context.positions.ball.position,
            }
                .calculate(ctx.player)
                .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}

impl DefenderInterceptingState {
    fn calculate_tackling_success(&self, ctx: &StateProcessingContext) -> bool {
        let player_skills = &ctx.player.skills;

        // Factors affecting tackling success
        let tackling = player_skills.technical.tackling;
        let aggression = player_skills.mental.aggression;
        let anticipation = player_skills.mental.anticipation;

        // Combine skills to create a tackling score
        let tackling_score = (tackling * 0.5) + (aggression * 0.3) + (anticipation * 0.2);

        // Normalize the score to a value between 0 and 1
        let normalized_score = tackling_score / 20.0;

        // Generate a random value to determine if the tackle is successful
        let mut rng = rand::thread_rng();
        let random_value: f32 = rng.gen_range(0.0..1.0);

        // Tackle is successful if the normalized score is higher than the random value
        normalized_score > random_value
    }

    /// Determines if the defender can reach the interception point before any opponent
    fn can_reach_before_opponent(&self, ctx: &StateProcessingContext) -> bool {
        // Calculate time for defender to reach interception point
        let interception_point = self.calculate_interception_point(ctx);
        let defender_distance = (interception_point - ctx.player.position).magnitude();
        let defender_speed = ctx.player.skills.physical.pace.max(0.1); // Avoid division by zero
        let defender_time = defender_distance / defender_speed;

        // Find the minimum time for any opponent to reach the interception point
        let opponent_time = ctx
            .players()
            .opponents()
            .all()
            .map(|opponent| {
                let player = ctx.player();
                let skills = player.skills(opponent.id);

                let opponent_speed = skills.physical.pace.max(0.1);
                let opponent_distance = (interception_point - opponent.position).magnitude();
                opponent_distance / opponent_speed
            })
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(f32::MAX);

        // Return true if defender can reach before any opponent
        defender_time < opponent_time
    }

    /// Calculates the interception point of the ball
    fn calculate_interception_point(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // Get ball position and velocity
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        // Defender's speed
        let defender_speed = ctx.player.skills.physical.pace.max(0.1);

        // Relative position and velocity
        let relative_position = ball_position - ctx.player.position;
        let relative_velocity = ball_velocity;

        // Time to intercept
        let time_to_intercept = relative_position.magnitude()
            / (defender_speed + relative_velocity.magnitude()).max(0.1);

        // Predict ball position after time_to_intercept
        ball_position + ball_velocity * time_to_intercept
    }
}
