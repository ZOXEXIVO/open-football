//! **Holding the touchline** — the wide forward's off-ball job.
//!
//! The forward-side half of [`MidfielderHoldingWidthState`], sharing the
//! same [`WideChannel`] so the two can never drift apart. See that
//! module for why a state exists for standing still on a touchline.
//!
//! The difference is where he goes when the job ends. A wide midfielder
//! drops back into support; a wide forward's next thought is the box —
//! either the far post, if the plan has given him a slot, or the space
//! behind the last defender.
//!
//! [`MidfielderHoldingWidthState`]: crate::r#match::midfielders::states::MidfielderHoldingWidthState

use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::strategies::common::team::WideChannel;
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
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        if !WideChannel::still_mine(ctx) {
            return Some(StateChangeResult::with_forward_state(
                if ctx.team().is_control_ball() {
                    // He has been given the box — which for a wide
                    // forward is the far post, and `CreatingSpace` is
                    // the state that steers at an assigned slot.
                    ForwardState::CreatingSpace
                } else {
                    ForwardState::Returning
                },
            ));
        }

        // A ball played into his channel is his, and nothing else would
        // pick it up — the loose-ball election in the dispatcher only
        // covers a ball nobody is receiving.
        if ctx.ball().is_towards_player() && ctx.ball().distance() < 120.0 && !ctx.ball().is_owned()
        {
            return Some(StateChangeResult::with_forward_state(ForwardState::TakeBall));
        }

        None
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
