use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior, VectorExtensions,
};
use nalgebra::Vector3;

const CLAIM_BALL_DISTANCE: f32 = 5.0; // Distance at which goalkeeper claims the ball
const MAX_COMING_OUT_DISTANCE: f32 = 120.0; // Maximum distance to pursue ball
const PREPARE_FOR_SAVE_DISTANCE: f32 = 50.0; // Distance to prepare for save instead of claiming

#[derive(Default)]
pub struct GoalkeeperComingOutState {}

impl StateProcessingHandler for GoalkeeperComingOutState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // Goalkeeper skills
        let anticipation = ctx.player.skills.mental.anticipation / 20.0;
        let decisions = ctx.player.skills.mental.decisions / 20.0;
        let command_of_area = ctx.player.skills.mental.vision / 20.0;
        let agility = ctx.player.skills.physical.agility / 20.0;

        // If goalkeeper has reached the ball, claim it
        if ball_distance < CLAIM_BALL_DISTANCE {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::HoldingBall,
            ));
        }

        // Check if ball is moving fast toward goalkeeper - prepare for save instead
        let ball_toward_keeper = ctx.ball().is_towards_player_with_angle(0.7);
        if ball_toward_keeper && ball_speed > 8.0 && ball_distance < PREPARE_FOR_SAVE_DISTANCE {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::PreparingForSave,
            ));
        }

        // Check if ball is too far - abort coming out
        if ball_distance > MAX_COMING_OUT_DISTANCE * (1.0 + command_of_area * 0.3) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // Check if ball is now on opponent's half - return to goal
        if !ctx.ball().on_own_side() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // FIXED: Check if opponent has the ball and is getting dangerous
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let opponent_distance = opponent.distance(ctx);
            let opponent_ball_distance = (opponent.position - ctx.tick_context.positions.ball.position).magnitude();

            // If opponent has good control and is close to ball
            if opponent_ball_distance < 3.0 && opponent_distance < 30.0 {
                // Decision: continue coming out if we're close enough, otherwise prepare for save
                let keeper_advantage = self.can_reach_ball_first(ctx, &opponent);

                if keeper_advantage {
                    // Continue pursuing - we can reach it first
                    return None; // Stay in ComingOut state
                } else {
                    // Opponent will reach first - prepare for shot/dribble
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::PreparingForSave,
                    ));
                }
            }
        } else {
            // Ball is loose - perfect for claiming!
            // Use anticipation to judge if we should continue
            if ball_distance < (MAX_COMING_OUT_DISTANCE * 0.7) {
                return None; // Continue coming out for loose ball
            }
        }

        // Check if we're getting too far from goal
        let goal_distance = ctx.player().distance_from_start_position();
        if goal_distance > 50.0 * (1.0 + command_of_area * 0.2) {
            // Too far from goal - reassess
            if !ctx.ball().is_owned() && ball_distance < 20.0 {
                return None; // Ball very close and loose, keep going
            } else {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ReturningToGoal,
                ));
            }
        }

        // If ball is stationary or slow and we're close, keep coming
        if ball_speed < 3.0 && ball_distance < 40.0 {
            return None; // Stay in ComingOut
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_distance = ctx.ball().distance();

        // Use acceleration skill to determine sprint speed
        let acceleration = ctx.player.skills.physical.acceleration / 20.0;
        let agility = ctx.player.skills.physical.agility / 20.0;
        let speed_multiplier = 0.8 + (acceleration * 0.4); // Range: 0.8 to 1.2

        // If ball is moving, predict where it will be
        if ball_velocity.norm() > 1.0 {
            // Predict ball position
            let time_to_reach = ball_distance / (ctx.player.skills.physical.pace * 0.5);
            let predicted_position = ball_position + ball_velocity * time_to_reach;

            Some(
                SteeringBehavior::Pursuit {
                    target: predicted_position,
                }
                .calculate(ctx.player)
                .velocity * speed_multiplier,
            )
        } else {
            // Ball is stationary - go directly to it with urgency
            if ball_distance < 10.0 {
                // Very close - slow down for control
                Some(
                    SteeringBehavior::Arrive {
                        target: ball_position,
                        slowing_distance: 5.0,
                    }
                    .calculate(ctx.player)
                    .velocity * (speed_multiplier * 0.7),
                )
            } else {
                // Sprint to ball
                Some(
                    SteeringBehavior::Pursuit {
                        target: ball_position,
                    }
                    .calculate(ctx.player)
                    .velocity * speed_multiplier,
                )
            }
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl GoalkeeperComingOutState {
    /// Check if goalkeeper can reach ball before opponent
    fn can_reach_ball_first(
        &self,
        ctx: &StateProcessingContext,
        opponent: &crate::r#match::MatchPlayerLite,
    ) -> bool {
        let ball_position = ctx.tick_context.positions.ball.position;
        let keeper_position = ctx.player.position;
        let opponent_position = opponent.position;

        // Distance calculations
        let keeper_to_ball = (ball_position - keeper_position).magnitude();
        let opponent_to_ball = (ball_position - opponent_position).magnitude();

        // Skills
        let keeper_acceleration = ctx.player.skills.physical.acceleration / 20.0;
        let keeper_anticipation = ctx.player.skills.mental.anticipation / 20.0;
        let opponent_pace = ctx.player().skills(opponent.id).physical.pace / 20.0;
        let opponent_acceleration = ctx.player().skills(opponent.id).physical.acceleration / 20.0;

        // Calculate effective speeds
        let keeper_speed = ctx.player.skills.physical.pace * (1.0 + keeper_acceleration * 0.3);
        let opponent_speed = ctx.player().skills(opponent.id).physical.pace * (1.0 + opponent_acceleration * 0.2);

        // Time estimates
        let keeper_time = keeper_to_ball / keeper_speed;
        let opponent_time = opponent_to_ball / opponent_speed;

        // Goalkeeper gets advantage from anticipation
        let anticipation_advantage = 1.0 + keeper_anticipation * 0.25;

        keeper_time < (opponent_time * anticipation_advantage)
    }
}
