use crate::r#match::common_states::ShapeTempo;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{
    ActivityIntensity, DefenderCondition, DefenderRead, DefensiveSlot,
};
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// **Alert: the play is in reach of the line.** The same slot the relaxed
/// defender walks to, held on his toes; the read sends him on a job, into
/// a duel or back to a walk.
#[derive(Default, Clone)]
pub struct DefenderHoldingLineState {}

impl StateProcessingHandler for DefenderHoldingLineState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        DefenderRead::hold(ctx, DefenderState::HoldingLine)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let to_target = DefensiveSlot::point(ctx) - ctx.player.position;
        let distance = to_target.magnitude();

        // Define thresholds for movement
        const MIN_DISTANCE_THRESHOLD: f32 = 1.0;
        const SLOWING_DISTANCE: f32 = 5.0;

        // Base movement speed — line_holding_mult bakes condition,
        // concentration curve, and fatigue into a 0.75..1.03 multiplier.
        // Replaces the raw pace/20 clamp so a tired CB shuffles slower
        // and a fresh, concentrated CB holds the line crisply.
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        let pace_influence = (ctx.player.skills.physical.pace / 20.0).clamp(0.6, 1.2);
        let base_speed = 3.0 * pace_influence * def_profile.line_holding_mult;

        if distance > MIN_DISTANCE_THRESHOLD {
            let direction = to_target.normalize();

            // Speed factor - slow down as approaching target
            let speed_factor = if distance > SLOWING_DISTANCE {
                1.0
            } else {
                (distance / SLOWING_DISTANCE).clamp(0.25, 1.0)
            };

            Some(direction * base_speed * speed_factor)
        } else {
            // In position - stay still (no artificial jitter)
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Holding line involves minimal movement - allows for recovery
        DefenderCondition::new(ShapeTempo::declared(&ctx, ActivityIntensity::Recovery))
            .process(ctx);
    }
}
