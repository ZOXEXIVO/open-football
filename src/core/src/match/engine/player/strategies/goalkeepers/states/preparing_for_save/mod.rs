use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const DIVE_DISTANCE: f32 = 40.0; // Distance to attempt diving save
const CATCH_DISTANCE: f32 = 35.0; // Distance to attempt catching
const PUNCH_DISTANCE: f32 = 18.0; // Distance to attempt punching

#[derive(Default, Clone)]
pub struct GoalkeeperPreparingForSaveState {}

impl StateProcessingHandler for GoalkeeperPreparingForSaveState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If goalkeeper has the ball, transition to passing
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Passing,
            ));
        }

        // Check if we need to dive
        if self.should_dive(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Diving,
            ));
        }

        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // Check if we should attempt a save
        // IMPORTANT: Only catch if goalkeeper is reasonably close to their goal
        // This prevents catching balls at center field
        let distance_from_goal = ctx.player().distance_from_start_position();
        const MAX_DISTANCE_FROM_GOAL_TO_CATCH: f32 = 50.0; // Only catch near goal area

        // Shot in flight: enter Catching immediately — we need to be
        // moving toward the intercept line every tick, not waiting for
        // the ball to come within 35u first (by which point it's
        // already past the keeper).
        if ctx.tick_context.ball.cached_shot_target.is_some()
            && distance_from_goal < MAX_DISTANCE_FROM_GOAL_TO_CATCH
        {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        if ball_distance < CATCH_DISTANCE && distance_from_goal < MAX_DISTANCE_FROM_GOAL_TO_CATCH {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        // If ball is on opponent's half, return to goal
        if !ctx.ball().on_own_side() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // Our team now has the ball — drop back to Standing; no save
        // is imminent. (Previously routed to Attentive, which was a
        // no-op Standing.)
        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // Check if we should punch (for dangerous high balls)
        if self.should_punch(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Punching,
            ));
        }

        // Check if ball is moving away and we should come out
        let ball_toward_goal = self.is_ball_toward_goal(ctx);
        if !ball_toward_goal && ball_distance < 30.0 && ball_speed < 5.0 {
            // Loose ball not heading to goal - come out to claim
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ComingOut,
            ));
        }

        // Continue preparing - position for the save
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // Sprint speed boost — gated by explosive multiplier.
        let speed_boost = (1.6 + prof.shot_stopping * 0.6 + prof.dive_reach * 0.5)
            * prof.explosive_mult;

        // If a shot has been fired, the projected goal-line crossing is
        // cached on the ball. Commit to that line instead of chasing
        // the ball's current position — a real keeper picks a spot on
        // the line and dives there. Without this the keeper lost ground
        // tick-by-tick to the 5.6 u/tick shot and never saved anything.
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            let goal_pos = ctx.ball().direction_to_own_goal();
            // Arrive at (goal_line_x, target_y) — i.e. the post-to-post
            // line, Y offset is where the shot is going. Z ignored: we
            // move on the ground.
            let intercept_point = Vector3::new(goal_pos.x, target.goal_line_y, 0.0);
            return Some(
                SteeringBehavior::Arrive {
                    target: intercept_point,
                    slowing_distance: 3.0,
                }
                .calculate(ctx.player)
                .velocity
                    * speed_boost,
            );
        }

        // No shot cached — slow ball / through ball / loose ball: fall
        // back to the angle-narrowing behaviour.
        let ball_distance = ctx.ball().distance();
        let goal_pos = ctx.ball().direction_to_own_goal();
        let prediction_time = 0.2 + prof.shot_stopping * 0.4;
        let predicted_ball = ball_position + ball_velocity * prediction_time;
        let goal_to_predicted = predicted_ball - goal_pos;
        let intercept_distance = if ball_speed > 1.2 {
            10.0 + prof.shot_stopping * 8.0 + prof.dive_reach * 3.0
        } else {
            18.0 + prof.shot_stopping * 10.0 + prof.dive_reach * 4.0
        };
        let target = if goal_to_predicted.norm() > 1.0 {
            goal_pos + goal_to_predicted.normalize() * intercept_distance.min(ball_distance * 0.5)
        } else {
            goal_pos
        };

        Some(
            SteeringBehavior::Pursuit {
                target,
                target_velocity: ball_velocity * 0.3,
            }
            .calculate(ctx.player)
            .velocity
                * speed_boost,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Preparing for save requires high intensity as goalkeeper moves into position
        GoalkeeperCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl GoalkeeperPreparingForSaveState {
    /// Determine if goalkeeper should dive for the ball
    fn should_dive(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // Don't dive if ball is too far
        if ball_distance > DIVE_DISTANCE {
            return false;
        }

        // Check if ball is heading toward goal
        let toward_goal = self.is_ball_toward_goal(ctx);
        if !toward_goal {
            return false;
        }

        // Ball must be moving (shots have velocity ~1.0-2.0 per tick)
        if ball_speed < 0.3 {
            return false;
        }

        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let time_to_ball = ball_distance / ball_speed.max(0.5);

        // Skill-driven distances. Effective dive distance already
        // bakes in dive_reach + positioning so it can scale with
        // condition; ball-speed branches differ in reaction window.
        let effective = prof.effective_dive_distance;
        if ball_speed > 1.5 {
            ball_distance < effective
                && time_to_ball < (18.0 + prof.shot_stopping * 22.0)
        } else if ball_speed > 0.8 {
            ball_distance < (effective * 0.85)
        } else {
            ball_distance < (effective * 0.65)
        }
    }

    /// Determine if goalkeeper should punch the ball
    fn should_punch(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();
        let ball_position = ctx.tick_context.positions.ball.position;

        if ball_distance > PUNCH_DISTANCE {
            return false;
        }

        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let ball_height = ball_position.z;
        let is_high_ball = ball_height > 2.0;

        let crowd = if ball_distance < 10.0 {
            (ctx.players().opponents().nearby(8.0).count() as f32 / 4.0).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let power_factor = ((ball_speed - 4.0) / 8.0).clamp(0.0, 1.0);
        // Build a synthetic catch_prob: aerial command + handling
        // discounted by crowd + power.
        let synthetic_catch = (prof.handling_profile * 0.55
            + prof.aerial_command * 0.45
            - power_factor * 0.20
            - crowd * 0.20)
            .clamp(0.0, 1.0);

        if is_high_ball && ball_speed > 8.0 {
            return true;
        }
        if crowd >= 0.5 && ball_distance < 10.0 {
            return prof.should_punch(synthetic_catch, crowd, power_factor);
        }
        if prof.handling_profile < 0.5 && ball_speed > 6.0 && is_high_ball {
            return true;
        }
        false
    }

    /// Check if ball is moving toward goal
    fn is_ball_toward_goal(&self, ctx: &StateProcessingContext) -> bool {
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // Stationary ball is not moving toward goal
        if ball_speed < 0.5 {
            return false;
        }

        // Get goal direction from ball
        let goal_direction = ctx.ball().direction_to_own_goal();

        // Check if ball velocity is pointing toward goal
        // Use dot product: > 0 means moving in same general direction
        let toward_goal_dot = ball_velocity.normalize().dot(&goal_direction.normalize());

        // Consider it "toward goal" if angle is less than 90 degrees (dot > 0)
        // More strict for positioning: require at least 30 degree alignment
        toward_goal_dot > 0.5
    }

    /// Calculate the optimal position for making a save
    #[allow(dead_code)]
    fn calculate_optimal_save_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();
        let goal_position = ctx.ball().direction_to_own_goal();
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // If ball is moving, predict where it will be
        let predicted_ball_position = if ball_speed > 1.0 {
            let prediction_time = 0.3 + prof.positioning * 0.3;
            ball_position + ball_velocity * prediction_time
        } else {
            ball_position
        };

        let goal_line_position = goal_position;
        let positioning_ratio = 0.15 + prof.positioning * 0.15;
        let optimal_position =
            goal_line_position + (predicted_ball_position - goal_line_position) * positioning_ratio;
        let max_distance_from_goal = 8.0 + prof.positioning * 4.0;
        let distance_from_goal = (optimal_position - goal_line_position).magnitude();

        if distance_from_goal > max_distance_from_goal {
            // Clamp to max distance
            goal_line_position
                + (optimal_position - goal_line_position).normalize() * max_distance_from_goal
        } else {
            optimal_position
        }
    }
}
