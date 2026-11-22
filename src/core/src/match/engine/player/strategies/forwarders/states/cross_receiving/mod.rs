use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::shared::{
    ActivityIntensity, ForwardCondition, ForwardRead,
};
use crate::r#match::player::strategies::shared::players::ops::box_movement::BoxMovement;
use crate::r#match::{
    ConditionContext, MatchContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// **Attacking his place in the box**, once the attack on it is staged: he
/// waits off it on his marker's blind side and darts onto it when the
/// delivery is struck (`BoxMovement`). A delivery he has won is the read's
/// staged level, one played to his feet the dispatcher's reception.
#[derive(Default, Clone)]
pub struct ForwardCrossReceivingState {}

impl StateProcessingHandler for ForwardCrossReceivingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        ForwardRead::hold(ctx, ForwardState::CrossReceiving)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let team = ctx.team();
        Some(match team.my_box_slot() {
            Some(slot) => BoxMovement::steer(ctx, slot),
            None => {
                SteeringBehavior::Arrive {
                    target: team.my_anchor(),
                    slowing_distance: 25.0,
                }
                .calculate(ctx.player)
                .velocity
            }
        })
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // `BoxMovement` waits off the place and darts onto it when the
        // delivery is struck; the dart is a sprint, which `Moderate` capped
        // at a jog.
        let tier = if MatchContext::forward_box_pace_off() {
            ActivityIntensity::Moderate
        } else {
            ActivityIntensity::VeryHigh
        };
        ForwardCondition::new(tier).process(ctx);
    }
}
