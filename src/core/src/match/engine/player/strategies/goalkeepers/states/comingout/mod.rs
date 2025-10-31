use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const CLAIM_BALL_DISTANCE: f32 = 15.0; // Distance at which goalkeeper claims the ball
const MAX_COMING_OUT_DISTANCE: f32 = 120.0; // Maximum distance to pursue ball

#[derive(Default)]
pub struct GoalkeeperComingOutState {}

impl StateProcessingHandler for GoalkeeperComingOutState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        if self.should_dive(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Diving,
            ));
        }

        // If goalkeeper has reached the ball, claim it immediately
        if ball_distance < CLAIM_BALL_DISTANCE {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        // Check if ball is moving very fast toward goalkeeper at close range - prepare for save
        let ball_toward_keeper = ctx.ball().is_towards_player_with_angle(0.7);
        if ball_toward_keeper && ball_distance < 150.0 {
            // Only switch to save for very fast shots at close range
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::PreparingForSave,
            ));
        }

        // Check if ball is too far - be more generous with command of area
        let command_of_area = ctx.player.skills.mental.vision / 20.0;
        let max_pursuit_distance = MAX_COMING_OUT_DISTANCE * (1.0 + command_of_area * 0.5);
        if ball_distance > max_pursuit_distance {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // Check if ball is on opponent's half - return to goal
        if !ctx.ball().on_own_side() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // Check if opponent has the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let opponent_distance = opponent.distance(ctx);
            let opponent_ball_distance = (opponent.position - ctx.tick_context.positions.ball.position).magnitude();

            // If opponent has control and is very close
            if opponent_ball_distance < 2.0 && opponent_distance < 20.0 {
                // Close opponent with ball - prepare for save/1v1
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            } else if opponent_ball_distance < 4.0 {
                // Opponent is near ball but we might intercept
                let keeper_advantage = self.can_reach_ball_first(ctx, &opponent);

                if keeper_advantage && ball_distance < 30.0 {
                    // We can reach it first - stay aggressive!
                    return None;
                } else if opponent_distance < 25.0 {
                    // Too risky - prepare for save
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::PreparingForSave,
                    ));
                }
            }
        }

        // Ball is loose - be aggressive!
        if !ctx.ball().is_owned() {
            // Loose ball within reasonable distance - continue pursuit
            if ball_distance < max_pursuit_distance * 0.8 {
                return None; // Keep going!
            }

            // Even if far, continue if ball is moving towards us
            if ball_toward_keeper && ball_distance < max_pursuit_distance {
                return None; // Ball coming to us - keep pursuing
            }
        }

        // Check distance from goal - allow more freedom based on command of area
        let goal_distance = ctx.player().distance_from_start_position();
        let max_goal_distance = 60.0 * (1.0 + command_of_area * 0.4);

        if goal_distance > max_goal_distance {
            // Getting far from goal - only continue if ball is very close and loose
            if !ctx.ball().is_owned() && ball_distance < 15.0 {
                return None; // Ball very close and loose, commit!
            } else {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ReturningToGoal,
                ));
            }
        }

        // For moving balls, be more aggressive about pursuing
        if ball_speed > 1.0 {
            // Moving ball - check if we can intercept
            if ball_distance < 50.0 {
                return None; // Pursue moving ball
            }
        } else {
            // Stationary ball - pursue if within range
            if ball_distance < 60.0 {
                return None; // Pursue stationary ball
            }
        }

        // Default: continue pursuit if we haven't hit any abort conditions
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
        let ball_speed = ball_velocity.norm();

        // Use acceleration skill to determine sprint speed
        let acceleration = ctx.player.skills.physical.acceleration / 20.0;

        // Goalkeepers should sprint aggressively when coming out
        // Speed multiplier: 1.4 to 2.0 (much faster than before)
        let speed_multiplier = 1.4 + (acceleration * 0.6);

        // Add urgency bonus based on ball distance and speed
        let urgency_multiplier = if ball_distance < 20.0 {
            1.4 // Very close - maximum urgency
        } else if ball_distance < 40.0 {
            1.2 // Medium distance - high urgency
        } else {
            1.1 // Far distance - moderate urgency
        };

        // Calculate interception point for moving balls
        let target_position = if ball_speed > 1.0 {
            // Ball is moving - predict interception point
            let keeper_sprint_speed = ctx.player.skills.physical.pace * (1.0 + acceleration * 0.5);
            let time_to_intercept = ball_distance / keeper_sprint_speed.max(1.0);

            // Predict where ball will be, with slight underprediction for safety (0.8x)
            ball_position + ball_velocity * time_to_intercept * 0.8
        } else {
            // Ball is stationary - go directly to it
            ball_position
        };

        // Decide steering behavior based on distance
        if ball_distance < 5.0 {
            // Very close - use Arrive for controlled claiming
            Some(
                SteeringBehavior::Arrive {
                    target: target_position,
                    slowing_distance: 1.0,
                }
                .calculate(ctx.player)
                .velocity * (speed_multiplier * 0.9), // Still fast but controllable
            )
        } else if ball_distance < 15.0 {
            // Close - sprint with slight deceleration zone
            Some(
                SteeringBehavior::Arrive {
                    target: target_position,
                    slowing_distance: 6.0,
                }
                .calculate(ctx.player)
                .velocity * (speed_multiplier * urgency_multiplier),
            )
        } else {
            // Far - full sprint using Pursuit for maximum speed
            // For fast-moving balls, add extra boost
            let final_multiplier = if ball_speed > 5.0 {
                speed_multiplier * urgency_multiplier * 1.15
            } else {
                speed_multiplier * urgency_multiplier
            };

            Some(
                SteeringBehavior::Pursuit {
                    target: target_position,
                    target_velocity: Vector3::zeros(), // Static target position
                }
                .calculate(ctx.player)
                .velocity * final_multiplier,
            )
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
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let keeper_position = ctx.player.position;
        let opponent_position = opponent.position;

        // Distance calculations
        let keeper_to_ball = (ball_position - keeper_position).magnitude();
        let opponent_to_ball = (ball_position - opponent_position).magnitude();

        // Goalkeeper skills
        let keeper_acceleration = ctx.player.skills.physical.acceleration / 20.0;
        let keeper_anticipation = ctx.player.skills.mental.anticipation / 20.0;
        let keeper_agility = ctx.player.skills.physical.agility / 20.0;
        let keeper_pace = ctx.player.skills.physical.pace;

        // Opponent skills
        let opponent_pace = ctx.player().skills(opponent.id).physical.pace;
        let opponent_acceleration = ctx.player().skills(opponent.id).physical.acceleration / 20.0;

        // Calculate effective sprint speeds (using our improved calculation)
        let keeper_sprint_speed = keeper_pace * (1.0 + keeper_acceleration * 0.5);
        let opponent_sprint_speed = opponent_pace * (1.0 + opponent_acceleration * 0.3);

        // If ball is moving, predict interception point
        let (keeper_distance, opponent_distance) = if ball_velocity.norm() > 1.0 {
            // Ball is moving - calculate interception distances

            // Simple prediction: where will ball be when keeper/opponent reaches it
            let keeper_intercept_time = keeper_to_ball / keeper_sprint_speed.max(1.0);
            let opponent_intercept_time = opponent_to_ball / opponent_sprint_speed.max(1.0);

            let keeper_intercept_pos = ball_position + ball_velocity * keeper_intercept_time;
            let opponent_intercept_pos = ball_position + ball_velocity * opponent_intercept_time;

            (
                (keeper_intercept_pos - keeper_position).magnitude(),
                (opponent_intercept_pos - opponent_position).magnitude()
            )
        } else {
            // Ball is stationary
            (keeper_to_ball, opponent_to_ball)
        };

        // Time estimates with actual distances
        let keeper_time = keeper_distance / keeper_sprint_speed.max(1.0);
        let opponent_time = opponent_distance / opponent_sprint_speed.max(1.0);

        // Goalkeeper advantages:
        // 1. Anticipation - better reading of the game
        // 2. Agility - quicker reactions
        // 3. Can use hands - easier to claim
        let anticipation_bonus = 1.0 + (keeper_anticipation * 0.3);
        let agility_bonus = 1.0 + (keeper_agility * 0.15);
        let hand_advantage = 1.2; // Can use hands to claim from further away

        let total_advantage = anticipation_bonus * agility_bonus * hand_advantage;

        // Keeper wins if their time is less than opponent's time adjusted for advantages
        keeper_time < (opponent_time * total_advantage)
    }

    fn should_dive(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();
        let ball_position = ctx.tick_context.positions.ball.position;
        let keeper_position = ctx.player.position;

        // Goalkeeper skills
        let reflexes = ctx.player.skills.mental.concentration / 20.0;
        let agility = ctx.player.skills.physical.agility / 20.0;
        let bravery = ctx.player.skills.mental.bravery / 20.0;
        let anticipation = ctx.player.skills.mental.anticipation / 20.0;
        let positioning = ctx.player.skills.technical.first_touch / 20.0; // Use first_touch as positioning proxy

        // Ball must be moving with reasonable speed to consider diving
        if ball_speed < 2.5 {
            return false;
        }

        // Don't dive if ball is too far - even skilled keepers have limits
        const MAX_DIVE_DISTANCE: f32 = 8.0;
        let skill_adjusted_max_dive = MAX_DIVE_DISTANCE + (agility * 4.0) + (reflexes * 3.0);

        if ball_distance > skill_adjusted_max_dive {
            return false;
        }

        // Check if ball is moving towards the keeper (important - don't dive at balls going away!)
        let ball_towards_keeper = ctx.ball().is_towards_player_with_angle(0.5);
        if !ball_towards_keeper {
            // Ball moving away or parallel - no need to dive
            return false;
        }

        // Calculate how close the ball will get if keeper doesn't dive
        let keeper_to_ball_dir = (ball_position - keeper_position).normalize();
        let ball_vel_normalized = if ball_speed > 0.01 {
            ball_velocity.normalize()
        } else {
            return false;
        };

        // Dot product to see if ball trajectory will bring it close
        let trajectory_alignment = keeper_to_ball_dir.dot(&ball_vel_normalized);

        // If ball is moving towards keeper but not directly, adjust dive threshold
        let trajectory_factor = trajectory_alignment.max(0.0);

        // Calculate time until ball reaches keeper's position
        let time_to_reach = ball_distance / ball_speed.max(1.0);

        // Predict where ball will be in near future
        let predicted_ball_pos = ball_position + ball_velocity * time_to_reach.min(2.0);
        let predicted_distance = (predicted_ball_pos - keeper_position).magnitude();

        // Decision factors:
        // 1. Ball is getting closer (predicted distance < current distance)
        // 2. Ball will be very close soon
        // 3. Keeper doesn't have time to run there

        let ball_getting_closer = predicted_distance < ball_distance;

        // Calculate if keeper can reach the ball by running
        let keeper_sprint_speed = ctx.player.skills.physical.pace * (1.0 + ctx.player.skills.physical.acceleration / 40.0);
        let time_to_run_to_ball = ball_distance / keeper_sprint_speed.max(1.0);

        // Dive if:
        // - Ball is close and getting closer
        // - Ball speed suggests keeper can't reach by running
        // - Keeper's skills support the dive decision

        let urgency_threshold = 0.8 + (anticipation * 0.3); // Better anticipation = earlier dive decision
        let dive_urgency = time_to_reach < urgency_threshold;

        // Different scenarios based on ball speed
        if ball_speed > 15.0 {
            // Very fast shot - need quick reflexes
            let reflex_threshold = 0.5 - (reflexes * 0.3);
            ball_getting_closer
                && ball_distance < (6.0 + reflexes * 5.0)
                && time_to_reach < reflex_threshold
                && trajectory_factor > 0.6
        } else if ball_speed > 10.0 {
            // Fast shot - need good positioning and reflexes
            let can_catch_running = time_to_run_to_ball < time_to_reach * 1.3;

            if can_catch_running {
                // Can probably catch it running - only dive if very close or excellent skills
                ball_distance < 4.0 && bravery > 0.6 && ball_getting_closer
            } else {
                // Need to dive to reach
                ball_distance < (8.0 + agility * 3.0 + positioning * 2.0)
                    && dive_urgency
                    && trajectory_factor > 0.5
                    && bravery > 0.4
            }
        } else if ball_speed > 5.0 {
            // Medium speed - keeper has more time to decide
            let can_catch_running = time_to_run_to_ball < time_to_reach * 1.5;

            if can_catch_running {
                // Prefer to run and catch - only dive if skills are high and ball is very close
                ball_distance < 3.0
                    && (reflexes + agility) > 1.3
                    && bravery > 0.7
                    && ball_getting_closer
            } else {
                // Ball will pass before keeper can run - dive needed
                ball_distance < (6.0 + agility * 4.0)
                    && predicted_distance < 8.0
                    && trajectory_factor > 0.4
                    && bravery > 0.5
            }
        } else {
            // Slow ball - generally shouldn't dive, should run and catch
            // Only dive if extremely close and keeper is brave/skilled
            ball_distance < 2.5
                && ball_getting_closer
                && bravery > 0.8
                && (reflexes + agility + positioning) > 2.0
                && trajectory_factor > 0.7
        }
    }
}
