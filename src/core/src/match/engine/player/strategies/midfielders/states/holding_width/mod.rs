//! **Holding the touchline** — the wide midfielder's off-ball job.
//!
//! The engine had no state for the commonest thing a wide player does.
//! `AttackSupporting` (13% of all AI ticks) walks him toward the ball,
//! `CreatingSpace` looks for a gap between defenders, `Returning` takes
//! him home — and every one of those pulls him *infield*, because they
//! are all written from the ball outward. So the flank was empty, and
//! [`CrossModel::is_in_wide_position`](crate::r#match::player::strategies::passing::CrossModel::is_in_wide_position)
//! — which every crossing state guards itself with — was false
//! essentially always.
//!
//! Standing on a touchline while the ball is 50 m away looks like doing
//! nothing, and it is the opposite. It is the only instruction in
//! football whose whole value is in where you are NOT: the full-back
//! marking you cannot tuck in, so the centre-back cannot cover the
//! channel, so the space the striker runs into exists. A side with
//! nobody holding width is a side defending itself.
//!
//! What he actually does tick to tick is [`WideChannel`]'s business, and
//! the forward's [`ForwardHoldingWidthState`] shares it — a 4-4-2's wide
//! midfielder and a 4-3-3's wide forward are the same footballer with a
//! different label, and this engine has a long history of two roles
//! keeping separate copies of one job until they contradict each other.
//!
//! [`ForwardHoldingWidthState`]: crate::r#match::forwarders::states::ForwardHoldingWidthState

use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, MidfielderCondition, MidfielderRead,
};
use crate::r#match::player::strategies::common::team::WideChannel;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// How close to the target he settles rather than chases. 40u = 5 m —
/// a touchline is a corridor, not a coordinate, and a man who chases a
/// point inside his own stride length oscillates on it.
const SETTLE: f32 = 40.0;

#[derive(Default, Clone)]
pub struct MidfielderHoldingWidthState {}

impl StateProcessingHandler for MidfielderHoldingWidthState {
    /// The read answers the touchline only while his side has the ball and
    /// `WideChannel::still_mine`, so entry and exit are one question and a
    /// lost ball moves him on the tick it is lost, not at the tactical
    /// refresh that `still_mine`'s flag waits for.
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        MidfielderRead::hold(ctx, MidfielderState::HoldingWidth)
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
        // `High`, not the tier the intent asks for.
        //
        // The exertion a state declares is also a hard SPEED CAP
        // (`MovementEffort::speed_fraction`), and `ConditionContext`
        // cannot see the intent — so the choice is one tier for all
        // three. `Moderate` (0.52) would lose the byline race to the
        // full-back tracking him, which is the one race this state
        // exists to run; `VeryHigh` would price a man jogging on a
        // touchline as a sprinter. Fatigue is 75% velocity-driven
        // anyway, so the honest cost of holding still is charged by the
        // steering, not by the label.
        MidfielderCondition::new(ActivityIntensity::High).process(ctx);
    }
}
