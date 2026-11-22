use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::shared::{
    ActivityIntensity, DefenderCondition, DefenderRead, DefensiveSlot,
};
use crate::r#match::shared_states::ShapeTempo;
use crate::r#match::{
    ConditionContext, MATCH_TIME_MS, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// **The recovery run with the opposition on the ball**, or the ball loose:
/// back to his slot at full effort, taking the carrier on the way if he
/// becomes the man for him.
#[derive(Default, Clone)]
pub struct DefenderTrackingBackState {}

impl StateProcessingHandler for DefenderTrackingBackState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        DefenderRead::recovering(ctx, DefenderState::TrackingBack)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let station = DefensiveSlot::point(ctx);
        let distance_from_start = (ctx.player.position - station).magnitude();

        // Calculate urgency based on game situation
        let urgency = if ctx.ball().on_own_side() {
            // Ball on own side - more urgent to get back
            1.5
        } else if ctx.team().is_loosing()
            && ctx.context.total_match_time > MATCH_TIME_MS - MATCH_TIME_MS / 15
        {
            // Losing late in game - less urgent to defend
            0.8
        } else {
            1.0
        };

        // Use Arrive behavior with slowing distance based on how far we are
        let slowing_distance = if distance_from_start > 50.0 {
            15.0 // Far away - longer slowing distance
        } else {
            10.0 // Close - shorter slowing distance
        };

        Some(
            SteeringBehavior::Arrive {
                target: station,
                slowing_distance,
            }
            .calculate(ctx.player)
            .velocity
                * urgency,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tracking back is a hard recovery run to his place in the shape —
        // high intensity (the player is literally running back, not
        // jogging).
        DefenderCondition::new(ShapeTempo::declared(&ctx, ActivityIntensity::VeryHigh))
            .process(ctx);
    }
}
