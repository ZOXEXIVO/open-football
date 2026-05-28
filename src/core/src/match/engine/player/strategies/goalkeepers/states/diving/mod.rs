use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Rotation3;
use nalgebra::Vector3;

const MAX_DIVE_TIME: f32 = 1.8; // Maximum time to stay in diving state (in seconds)
const BALL_CLAIM_DISTANCE: f32 = 14.0;

#[derive(Default, Clone)]
pub struct GoalkeeperDivingState {}

impl StateProcessingHandler for GoalkeeperDivingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Passing,
            ));
        }

        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_moving_away =
            ball_velocity.dot(&(ctx.player.position - ctx.ball().direction_to_own_goal())) > 0.0;

        // Ball is far away and moving away from goal — the dive missed
        // the line, the shot went wide, or the ball was already deflected
        // by something else. The keeper never touched the ball at this
        // distance, so this is NOT a parry; just exit the dive.
        // (A real parry happens at close range and is handled by the
        // is_ball_caught / is_ball_nearby branches below.)
        if ctx.ball().distance() > 100.0 && ball_moving_away {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        if ctx.in_state_time as f32 / 100.0 > MAX_DIVE_TIME {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        if self.is_ball_caught(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Standing,
                Event::PlayerEvent(PlayerEvent::CaughtBall(ctx.player.id)),
            ));
        } else if self.is_ball_nearby(ctx) {
            let mut result = StateChangeResult::with_goalkeeper_state(GoalkeeperState::Catching);
            result
                .events
                .add_player_event(PlayerEvent::ClaimBall(ctx.player.id));
            return Some(result);
        }

        if ctx.in_state_time > 90 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let dive_direction = self.calculate_dive_direction(ctx);
        let dive_speed = self.calculate_dive_speed(ctx);

        Some(dive_direction * dive_speed)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Diving is a very high intensity activity requiring maximum energy expenditure
        GoalkeeperCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl GoalkeeperDivingState {
    fn calculate_dive_direction(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // Prediction time scales with positioning + reaction quality.
        let prediction_time =
            (0.12 + prof.positioning * 0.26 + prof.shot_stopping * 0.22).clamp(0.10, 0.52);
        let future_ball_position = ball_position + ball_velocity * prediction_time;

        let to_future_ball = future_ball_position - ctx.player.position;
        let mut dive_direction = if to_future_ball.norm() > f32::EPSILON {
            to_future_ball.normalize()
        } else {
            Vector3::x()
        };

        // Direction error scales with poor skill and fatigue, eased by
        // good positioning. Range: ~0..0.6 rad band; elite stays tight.
        let direction_error =
            (0.05 + prof.poor_skill_penalty * 0.34 + (1.0 - prof.condition_mult) * 0.16
                - prof.positioning * 0.10)
                .clamp(0.0, 0.55);
        let random_angle = (ctx.context.rng.unit_f32() - 0.5) * direction_error;
        dive_direction = Rotation3::new(Vector3::z() * random_angle) * dive_direction;

        dive_direction
    }

    fn calculate_dive_speed(&self, ctx: &StateProcessingContext) -> f32 {
        let urgency = self.calculate_urgency(ctx);
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let base_speed =
            (ctx.player.skills.physical.acceleration + ctx.player.skills.physical.agility) * 0.5;
        // Elite: 0.70 + 0.95 = 1.65x, weak: ~0.75x. `explosive_mult` is
        // already folded into `dive_reach` so don't double-apply it.
        let skill_boost = 0.70 + prof.dive_reach * 0.95;
        base_speed * urgency * skill_boost
    }

    fn calculate_urgency(&self, ctx: &StateProcessingContext) -> f32 {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        let own_goal = ctx.ball().direction_to_own_goal();
        let distance_to_goal = (ball_position - own_goal).magnitude();
        let velocity_towards_goal = ball_velocity
            .dot(&(own_goal - ball_position).normalize())
            .max(0.0);

        // Scale for actual ball speeds (~1.0-2.0/tick)
        let urgency: f32 = (1.0 - distance_to_goal / 100.0) * (1.0 + velocity_towards_goal / 2.0);
        urgency.clamp(1.0, 2.5)
    }

    fn is_ball_caught(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        // Must be flying toward the GK or very close.
        if ball_distance > 5.0 && !ctx.ball().is_towards_player_with_angle(0.6) {
            return false;
        }

        let ball_speed = ctx.tick_context.positions.ball.velocity.magnitude();
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // Effective dive radius scales with skill: weak ~12u, elite
        // ~30u. Beyond that the keeper cannot get a hand on the ball.
        let catch_distance =
            6.0 + prof.dive_reach * 10.0 + prof.handling_profile * 4.0 + prof.shot_stopping * 4.0;
        if ball_distance > catch_distance {
            return false;
        }

        // Catch difficulty: stretch + power + fatigue + poor position.
        let stretch = (ball_distance / catch_distance).clamp(0.0, 1.0);
        let power = ((ball_speed - 1.5) / 6.0).clamp(0.0, 1.0);
        let catch_difficulty = (power * 0.36
            + stretch * 0.30
            + (1.0 - prof.condition_mult) * 0.14
            + (1.0 - prof.positioning) * 0.10
            + prof.poor_skill_penalty * 0.10)
            .clamp(0.0, 1.0);

        let catch_prob = prof.catch_probability(catch_difficulty);
        ctx.context.rng.unit_f32() < catch_prob
    }

    fn is_ball_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.ball().distance() < BALL_CLAIM_DISTANCE
    }
}
