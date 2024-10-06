use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use std::sync::LazyLock;
use rand::Rng;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::player::events::PlayerUpdateEvent;

static DEFENDER_INTERCEPTING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_intercepting_data.json")));

#[derive(Default)]
pub struct DefenderInterceptingState {}

impl StateProcessingHandler for DefenderInterceptingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the ball is too far away, transition to Returning state
        let ball_ops = ctx.ball();

        // 3. If the defender has intercepted the ball, transition to appropriate state
        let ball_distance = ball_ops.distance();
        if ball_distance < 10.0 {
            if ctx.tick_context.ball.is_owned {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            } else {
                if self.calculate_tackling_success(ctx) {
                     let mut state = StateChangeResult::with_defender_state(
                        DefenderState::Running,
                    );

                    state.events.add(PlayerUpdateEvent::ClaimBall(ctx.player.id));

                    return Some(state);
                }
            }
        }

        if ball_ops.distance() > 150.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        // 2. Check if the defender can reach the interception point before any opponent
        if !self.can_reach_before_opponent(ctx) {
            // If not, transition to Pressing or HoldingLine state
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

        None
    }

    fn process_slow(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if ctx.in_state_time % 3 == 0 {
            // Calculate the interception point
            let interception_point = self.calculate_interception_point(ctx);

            // Direction towards the interception point
            let direction = (interception_point - ctx.player.position).normalize();

            // Retrieve player's current speed magnitude
            let current_speed = ctx.player.velocity.magnitude();

            // Player's physical attributes (scaled appropriately)
            let acceleration = ctx.player.skills.physical.acceleration / 10.0; // Scale down as needed
            let max_speed = ctx.player.skills.physical.pace / 10.0; // Scale down as needed

            // Ensure delta_time is available; if not, define it based on your simulation tick rate
            let delta_time = 1.0 / 60.0; // ctx.delta_time; // Time elapsed since last update in seconds

            // Calculate new speed, incrementing by acceleration, capped at max_speed
            let new_speed = (current_speed + acceleration * delta_time).min(max_speed);

            // Calculate new velocity vector
            let new_velocity = direction * new_speed;

            return Some(new_velocity);
        }

       None
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
        let opponent_time = ctx.context.players.raw_players()
            .iter()
            .filter(|p| p.team_id != ctx.player.team_id)
            .map(|opponent| {
                let opponent_speed = opponent.skills.physical.pace.max(0.1);
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
        let ball_position = ctx.tick_context.object_positions.ball_position;
        let ball_velocity = ctx.tick_context.object_positions.ball_velocity;

        // Defender's speed
        let defender_speed = ctx.player.skills.physical.pace.max(0.1);

        // Relative position and velocity
        let relative_position = ball_position - ctx.player.position;
        let relative_velocity = ball_velocity;

        // Time to intercept
        let time_to_intercept = relative_position.magnitude() / (defender_speed + relative_velocity.magnitude()).max(0.1);

        // Predict ball position after time_to_intercept
        ball_position + ball_velocity * time_to_intercept
    }
}
