use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::player::PlayerSide;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct DefenderClearingState {}

impl StateProcessingHandler for DefenderClearingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Wait a few ticks before clearing to allow the player to reach the ball
        if ctx.in_state_time < 5 {
            return None;
        }

        let mut state = StateChangeResult::with(PlayerState::Defender(DefenderState::Standing));

        // Get ball's current position
        let ball_position = ctx.tick_context.positions.ball.position;

        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let field_center_y = field_height / 2.0;

        // Check if ball is at or near a boundary
        const BOUNDARY_THRESHOLD: f32 = 5.0;
        let at_left_boundary = ball_position.x <= BOUNDARY_THRESHOLD;
        let at_right_boundary = ball_position.x >= field_width - BOUNDARY_THRESHOLD;
        let at_top_boundary = ball_position.y >= field_height - BOUNDARY_THRESHOLD;
        let at_bottom_boundary = ball_position.y <= BOUNDARY_THRESHOLD;
        let at_boundary =
            at_left_boundary || at_right_boundary || at_top_boundary || at_bottom_boundary;

        // Determine clearance direction based on player's side (always clear AWAY from own goal)
        let is_left_side = ctx.player.side == Some(PlayerSide::Left);

        // Profile-driven clearance: technique/composure/decisions/passing
        // blend governs accuracy, strength scales distance, and a
        // poor_clearance_chance roll occasionally produces a short /
        // miscued / sliced clearance. Replaces a fully deterministic
        // halfway-line target.
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        let rng = &ctx.context.rng;
        let poor_clearance = rng.random::<f32>() < def_profile.poor_clearance_chance.min(0.95);

        let halfway_x = field_width * 0.5;
        let nominal_target_x = if is_left_side {
            halfway_x.max(ball_position.x + 30.0)
        } else {
            halfway_x.min(ball_position.x - 30.0)
        };

        // Distance multiplier — strong + technique extend the kick;
        // poor clearances land short.
        let distance_mult = (0.75
            + (ctx.player.skills.physical.strength / 20.0).powf(1.20) * 0.25
            + (ctx.player.skills.technical.technique / 20.0).powf(1.30) * 0.15)
            .clamp(0.55, 1.20);
        let distance_mult = if poor_clearance {
            distance_mult * 0.55
        } else {
            distance_mult
        };
        let signed_dx = nominal_target_x - ball_position.x;
        let target_x = ball_position.x + signed_dx * distance_mult;

        // Y error scales inversely with clearance_profile and is
        // amplified for poor clearances. A clean clearance lands near
        // the centre line; a poor one drifts toward the wing.
        let y_error_scale = (1.25 - def_profile.clearance_profile * 0.75).max(0.30);
        let y_jitter: f32 = rng.random::<f32>() * 2.0 - 1.0;
        let extra_y_error = if poor_clearance { 22.0 } else { 6.0 };
        let center_pull = 0.6;
        let target_y = ball_position.y
            + (field_center_y - ball_position.y) * center_pull
            + y_jitter * extra_y_error * y_error_scale;

        let target_position = Vector3::new(target_x, target_y, 0.0);
        let to_target = target_position - ball_position;
        let to_target_dist = to_target.norm().max(0.1);
        let direction_to_target = to_target / to_target_dist;

        // Lofted clearance, solved the same way a lofted pass is: pick how
        // high the hoof goes (in metres), which fixes its hang time, and
        // the horizontal speed then follows from the distance it has to
        // cover. A clean clearance is a controlled outlet that finds the
        // touchline area; a poor one is a weak skewed strike that drops
        // short and rolls back into trouble.
        //
        // Previously the two components were independent constants — 4-5
        // u/tick horizontally and 5-6 "z" — fitted to a gravity that was
        // 160× too strong in metres. That produced a 40 m hoof with a
        // 0.6 s hang time, and the numbers only worked as a pair, so
        // neither could be reasoned about on its own.
        let apex_metres = if at_boundary { 12.0 } else { 9.0 };
        let apex_mult = 0.85 + (ctx.player.skills.technical.technique / 20.0).powf(1.30) * 0.15;
        let z_velocity = Ball::launch_speed_for_apex(
            apex_metres * apex_mult * if poor_clearance { 0.55 } else { 1.0 },
        );

        let speed_mult =
            (0.85 + def_profile.clearance_profile * 0.30) * def_profile.clearance_condition_mult;
        // Reach the aim point within the hang time, then trim by execution.
        // Clamped to a realistic hoof: 2.6 u/tick = 32 m/s.
        let hang = Ball::hang_ticks(z_velocity).max(1.0);
        let clear_speed =
            ((to_target_dist / hang) * speed_mult * if poor_clearance { 0.65 } else { 1.0 })
                .clamp(0.30, 2.6);
        let horizontal_velocity = direction_to_target * clear_speed;

        let ball_velocity = Vector3::new(horizontal_velocity.x, horizontal_velocity.y, z_velocity);

        // Add the clear ball event with the calculated velocity
        state
            .events
            .add_player_event(PlayerEvent::ClearBall(ball_velocity));

        // Return the updated state with the clearing event
        Some(state)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        Some(
            SteeringBehavior::Arrive {
                target: ball_position,
                slowing_distance: 5.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Clearing involves powerful kicking action - explosive effort
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
