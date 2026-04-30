use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const JUMP_DURATION: u64 = 25; // Duration of jump animation in ticks (faster reaction)
const JUMP_HEIGHT: f32 = 3.0; // Maximum jump height (more explosive)
const MIN_DIVING_DISTANCE: f32 = 1.0; // Minimum distance to dive
const MAX_DIVING_DISTANCE: f32 = 8.0; // Maximum distance to dive (extended reach)

#[derive(Default, Clone)]
pub struct GoalkeeperJumpingState {}

impl StateProcessingHandler for GoalkeeperJumpingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if jump duration is complete
        if ctx.in_state_time >= JUMP_DURATION {
            // After jump, transition to appropriate state
            if ctx.player.has_ball(ctx) {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::HoldingBall,
                ));
            } else {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Standing,
                ));
            }
        }

        // During jump, check if we can catch the ball
        if self.can_catch_ball(ctx) {
            let mut result = StateChangeResult::with_goalkeeper_state(GoalkeeperState::Catching);

            // Add catch attempt event
            result
                .events
                .add_player_event(PlayerEvent::RequestBallReceive(ctx.player.id));
            return Some(result);
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Calculate base jump vector
        let jump_vector = self.calculate_jump_vector(ctx);

        // Add diving motion if needed
        let diving_vector = if self.should_dive(ctx) {
            self.calculate_diving_vector(ctx)
        } else {
            Vector3::zeros()
        };

        // Calculate vertical component based on jump phase
        let vertical_component = self.calculate_vertical_motion(ctx);

        // Combine all motion components
        let combined_velocity =
            jump_vector + diving_vector + Vector3::new(0.0, 0.0, vertical_component);

        // Explosive scaling — jumping/diving must be very fast
        let attribute_scaling = (ctx.player.skills.physical.jumping as f32
            + ctx.player.skills.physical.agility as f32)
            / 25.0; // was /40.0 — 60% faster

        Some(combined_velocity * attribute_scaling)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Jumping is a very high intensity activity requiring significant energy expenditure
        GoalkeeperCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl GoalkeeperJumpingState {
    /// Check if the goalkeeper can reach and catch the ball — skill-based probability
    fn can_catch_ball(&self, ctx: &StateProcessingContext) -> bool {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let distance = (ball_pos - keeper_pos).magnitude();

        let jumping = ctx.player.skills.physical.jumping / 20.0;
        let agility = ctx.player.skills.physical.agility / 20.0;
        let handling = ctx.player.skills.goalkeeping.handling / 20.0;
        let reflexes = ctx.player.skills.goalkeeping.reflexes / 20.0;

        // Reach based on jumping + agility
        let max_reach = JUMP_HEIGHT * (jumping * 0.6 + agility * 0.4);
        let vertical_reach = (ball_pos.z - keeper_pos.z).abs() <= max_reach;
        let horizontal_reach = distance <= MAX_DIVING_DISTANCE + agility * 4.0;

        if !vertical_reach || !horizontal_reach {
            return false;
        }

        // Skill-based catch probability
        let skill_blend = handling * 0.35 + reflexes * 0.30 + agility * 0.20 + jumping * 0.15;

        // Distance penalty — further stretch = harder catch
        let stretch_ratio = distance / (MAX_DIVING_DISTANCE + agility * 4.0);
        let distance_penalty = stretch_ratio * 0.25;

        // Ball speed penalty — fast shots are harder to hold
        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        let speed_penalty = (ball_speed / 5.0).min(0.3) * (1.0 - reflexes * 0.5);

        // Elite: 0.20 + 0.95*0.75 = 0.91, mediocre: 0.20 + 0.47*0.75 = 0.55
        let catch_probability =
            (0.20 + skill_blend * 0.75 - distance_penalty - speed_penalty).clamp(0.10, 0.95);

        rand::random::<f32>() < catch_probability
    }

    /// Calculate the base jump vector towards the ball
    fn calculate_jump_vector(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let to_ball = ball_pos - keeper_pos;

        if to_ball.magnitude() > 0.0 {
            to_ball.normalize() * ctx.player.skills.physical.acceleration
        } else {
            Vector3::zeros()
        }
    }

    /// Determine if the goalkeeper should dive
    fn should_dive(&self, ctx: &StateProcessingContext) -> bool {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let distance = (ball_pos - keeper_pos).magnitude();

        // Check if the ball is at a distance that requires diving
        if distance < MIN_DIVING_DISTANCE || distance > MAX_DIVING_DISTANCE {
            return false;
        }

        // Check if the ball is moving towards goal
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let to_goal = ctx.ball().direction_to_own_goal() - ball_pos;

        ball_velocity.dot(&to_goal) > 0.0
    }

    /// Calculate the diving motion vector
    fn calculate_diving_vector(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let to_ball = ball_pos - keeper_pos;

        if to_ball.magnitude() > 0.0 {
            // Calculate diving direction considering goalkeeper's diving ability
            let diving_direction = to_ball.normalize();
            let diving_power = ctx.player.skills.physical.jumping as f32 / 20.0;

            diving_direction * diving_power * 2.0
        } else {
            Vector3::zeros()
        }
    }

    /// Calculate vertical motion based on jump phase
    fn calculate_vertical_motion(&self, ctx: &StateProcessingContext) -> f32 {
        let jump_phase = ctx.in_state_time as f32 / JUMP_DURATION as f32;
        let jump_curve = (std::f32::consts::PI * jump_phase).sin(); // Smooth jump curve

        // Scale jump height based on goalkeeper's jumping ability
        let max_height = JUMP_HEIGHT * (ctx.player.skills.physical.jumping as f32 / 20.0);

        jump_curve * max_height
    }
}
