//! **The overlap** — the full-back's run beyond the man on the touchline.
//!
//! # Why this is its own state
//!
//! The behaviour existed, buried inside [`DefenderPushingUpState`] as an
//! `is_overlap_run` branch of its `velocity`, and it was reached through
//! an eight-condition gate in `DefenderRunningState::should_overlap`.
//! Measured over 200 matches at level 14, that gate was asked 256 000
//! times a match and **committed 260** — one tenth of one per cent — and
//! `Defender: Pushing Up` held 0.4% of all AI ticks. The run was
//! effectively absent from the game.
//!
//! Three things were wrong and all three are structural rather than
//! tuning:
//!
//! * **The decision was made by the wrong body.** Eight independent
//!   conditions, each individually reasonable, are a conjunction that a
//!   coin-flip term ("is the ball on my flank?") can halve at any point.
//!   Whether a side can afford to send a full-back is a TEAM question
//!   with one answer, and it is now answered once, by name, in
//!   [`WidePlan`](crate::r#match::WidePlan) — which also guarantees both
//!   full-backs cannot go on the same possession.
//! * **Rest defence held him home.** The plan ranked rest defence on raw
//!   depth, and a back four's two full-backs stand at the same depth as
//!   its centre-backs, so both were always picked. See
//!   `PlanBuilder::rest_fit`.
//! * **He was capped at a jog.** `PushingUp` declares
//!   `ActivityIntensity::Moderate`, and the tier a state declares is a
//!   hard *speed ceiling* (`MovementEffort::speed_fraction`) — 0.52 of
//!   top speed. A man limited to 52% cannot overtake a winger running at
//!   78%, so even a committed overlap could never actually get beyond
//!   the man it was overlapping. This state declares the sprint it is.
//!
//! # What he does
//!
//! Runs the outside lane past the width holder and gets to the byline,
//! sharing [`WideChannel`] with the two `HoldingWidth` states so the man
//! overlapping and the man being overlapped agree about where the
//! channel is. What he does when the ball arrives is decided by where he
//! is standing, in `DefenderRunningState`'s wide-area ladder — not here,
//! and not by his position on the team sheet.

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::player::strategies::common::team::WideChannel;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// He is running at a point, not standing on one — a short slowing
/// distance so he arrives at the byline still moving.
const SETTLE: f32 = 24.0;

#[derive(Default, Clone)]
pub struct DefenderOverlappingState {}

impl StateProcessingHandler for DefenderOverlappingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            // `Running` carries the wide-area ladder — release, deliver,
            // or drive on — and it decides on position rather than on
            // the fact that a defender is holding the ball.
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // The run is over when the plan says so, and the plan says so
        // the moment we lose the ball. Reading it from one place is what
        // stops this state and the one that sent him here disagreeing —
        // the failure that had `PushingUp` bouncing a committed
        // full-back back to `TrackingBack` on the same tick he set off.
        if !WideChannel::still_mine(ctx) {
            return Some(StateChangeResult::with_defender_state(
                if ctx.team().is_control_ball() {
                    DefenderState::PushingUp
                } else {
                    DefenderState::TrackingBack
                },
            ));
        }

        // A ball rolled into the channel ahead of him is what the run was
        // for.
        if ctx.ball().is_towards_player() && ctx.ball().distance() < 120.0 && !ctx.ball().is_owned()
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
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
        // An overlap is one of the four or five genuine sprints a
        // full-back makes in a match, and the tier is a speed cap as
        // much as a fatigue price — see the module docs. It is also
        // self-limiting: the run is 40 m, it happens a handful of times
        // a half, and `WideChannel::still_mine` ends it the instant
        // possession turns over.
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
