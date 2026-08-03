use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::{
    ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// Inside this distance of the goal a strike is a FINISH — a first-time
/// contact from inside or on the edge of the box, where hesitation costs
/// the chance. Matches `POINT_BLANK_DISTANCE` in the forward running
/// state, which is the branch that feeds this one.
const FINISHING_RANGE: f32 = 36.0;

/// The close-range strike.
///
/// Distinct from `Shooting`, which owns everything from the edge of the
/// box outwards and carries abort gates tuned for range (`> 150u without
/// a clear shot`, `> 80u with sub-0.5 finishing`). Those gates are simply
/// wrong for a tap-in: a poor finisher six yards out still shoots. This
/// state has no range aborts because it only ever runs inside
/// `FINISHING_RANGE`.
///
/// It had no inbound transition at all before — every chance, from a
/// six-yard tap-in to a 30-yard drive, went through `Shooting`.
#[derive(Default, Clone)]
pub struct ForwardFinishingState {}

impl StateProcessingHandler for ForwardFinishingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Drifted out of finishing range — hand back to the general shot
        // state, which owns range shooting and its own abort gates.
        if !self.is_within_shooting_range(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Shooting,
            ));
        }

        // A caller that already tagged `pending_shot_reason` has run the
        // gates and committed to the strike; re-running the helper here
        // would roll willingness a second time and throw away chances the
        // decision layer already approved. Same contract the Shooting
        // state uses.
        if let Some(reason) = ctx.player.pending_shot_reason {
            return Some(self.strike(ctx, reason));
        }

        // Direct entry (no tagged reason) — evaluate from scratch. The
        // centralised helper applies the skill / clarity / xG / pass-EV
        // checks; without it this state fired on a bare distance gate,
        // which is why mediocre finishers once converted at near-elite
        // rates from any cross or rebound.
        match evaluate_forward_shot_decision(ctx, "FWD_FINISHING") {
            ShotDecision::Shoot { reason } => Some(self.strike(ctx, reason)),
            ShotDecision::Pass => {
                Some(StateChangeResult::with_forward_state(ForwardState::Passing))
            }
            // Hesitated. Inside the box that means taking the touch that
            // opens the angle rather than backing out of the chance.
            ShotDecision::Hold => Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            )),
        }
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl ForwardFinishingState {
    /// Compose the strike and hand back to Running so the forward reacts
    /// to the rebound.
    ///
    /// The target comes from `shooting_direction()`, which — despite the
    /// name — returns a goal-mouth POSITION with skill-aware placement
    /// (corners for good finishers, centre-mass under pressure), the same
    /// helper every other shooting state passes to `with_target`. The
    /// state's original (never-reached) code built a normalized unit
    /// DIRECTION here instead, which the shot pipeline read as a target
    /// point one unit from the pitch origin: every finish flew toward the
    /// corner flag and recorded floor xG. Latent while the state was
    /// unreachable; wiring the state exposed it as a forward-conversion
    /// collapse (FWD goals-by-line 368 → ~150 per 200 matches).
    fn strike(&self, ctx: &StateProcessingContext, reason: &'static str) -> StateChangeResult {
        StateChangeResult::with_forward_state_and_event(
            ForwardState::Running,
            Event::PlayerEvent(PlayerEvent::Shoot(
                ShootingEventContext::new()
                    .with_player_id(ctx.player.id)
                    .with_target(ctx.player().shooting_direction())
                    .with_reason(reason)
                    .build(ctx),
            )),
        )
    }

    fn is_within_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        ctx.ball().distance_to_opponent_goal() <= FINISHING_RANGE
    }
}
