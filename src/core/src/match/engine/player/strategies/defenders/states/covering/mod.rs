use crate::r#match::defenders::states::common::{
    ActivityIntensity, DefenderCondition, DefenderRead, DefensiveLine, ReadLevel,
};
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::common::states::StationKeeping;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// **Covering the man engaging the carrier**: the diagonal behind him, so
/// the carrier who beats him runs into the cover. Entered on the plan's
/// cover duty and left when it goes.
#[derive(Default, Clone)]
pub struct DefenderCoveringState {}

impl StateProcessingHandler for DefenderCoveringState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if let Some(change) = DefenderRead::change_above(ctx, ReadLevel::Cover) {
            return Some(change);
        }
        if DefenderRead::has_cover_duty(ctx) {
            return None;
        }
        Some(DefenderRead::change(ctx))
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // The plan's cover anchor — a stride and a half behind the point
        // of attack on the line to goal — held inside the shape: cover is
        // a line role, and sitting behind the presser is exactly the
        // `DEPTH_DROP` the constraint allows.
        let Some(anchor) = ctx.team().my_duty_anchor() else {
            return Some(Vector3::zeros());
        };
        let target = DefensiveLine::hold_shape(ctx, anchor);
        let to_target = target - ctx.player.position;
        let distance = to_target.magnitude();
        if distance < 2.0 {
            let point_velocity = ctx
                .players()
                .opponents()
                .with_ball()
                .next()
                .map_or(ctx.tick_context.positions.ball.velocity, |c| c.velocity(ctx));
            return Some(
                StationKeeping::hold(point_velocity, to_target)
                    + ctx.player().separation_velocity() * 0.3,
            );
        }
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        let speed = ctx.player.skills.physical.pace * 0.9 * def_profile.recovery_run_mult;
        let urgency = (distance / 20.0).clamp(0.7, 1.4);
        Some(to_target.normalize() * speed * urgency + ctx.player().separation_velocity() * 0.2)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Covering a gap during a live attack is a hard recovery run —
        // a sprint, not a jog. See the note in `DefenderPressingState`.
        DefenderCondition::new(ActivityIntensity::chase()).process(ctx);
    }
}
