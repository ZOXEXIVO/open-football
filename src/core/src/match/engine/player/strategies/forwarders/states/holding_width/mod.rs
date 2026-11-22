//! **Holding the touchline** — the wide forward's off-ball job.
//!
//! The forward-side half of [`MidfielderHoldingWidthState`], sharing the
//! same [`WideChannel`] so the two can never drift apart. See that
//! module for why a state exists for standing still on a touchline.
//!
//! When the job ends the read sends him on, to whatever the attack gives
//! him next or to his place.
//!
//! [`MidfielderHoldingWidthState`]: crate::r#match::midfielders::states::MidfielderHoldingWidthState

use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::shared::{
    ActivityIntensity, ForwardCondition, ForwardRead,
};
use crate::r#match::player::strategies::shared::team::WideChannel;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// See `MidfielderHoldingWidthState::SETTLE` — the touchline is a
/// corridor, not a coordinate.
const SETTLE: f32 = 40.0;

#[derive(Default, Clone)]
pub struct ForwardHoldingWidthState {}

impl StateProcessingHandler for ForwardHoldingWidthState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        ForwardRead::hold(ctx, ForwardState::HoldingWidth)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        WideChannel::note_tick(ctx);
        let target = WideChannel::target(ctx, WideChannel::intent(ctx));
        Some(
            SteeringBehavior::Arrive {
                target,
                slowing_distance: SETTLE,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // See the note in `MidfielderHoldingWidthState`: the tier is a
        // speed cap as much as a fatigue price, and `High` is the one
        // that neither loses the byline race nor charges a man standing
        // on a touchline as a sprinter.
        ForwardCondition::new(ActivityIntensity::High).process(ctx);
    }
}
