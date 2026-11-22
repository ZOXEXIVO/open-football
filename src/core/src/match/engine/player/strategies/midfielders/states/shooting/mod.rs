use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::shared::{
    ActivityIntensity, MidfielderCondition, MidfielderRead,
};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct MidfielderShootingState {}

impl StateProcessingHandler for MidfielderShootingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Pressing
            return Some(MidfielderRead::change(ctx));
        }

        // Per-player cooldown — same reasoning as the forward
        // Shooting state. A midfielder who struck recently isn't
        // balanced to strike again within 1.5 s.
        if !ctx.player().can_shoot() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // Only abort for long range with no clear shot
        // Close and medium range: take the shot
        // Backstop only — the helper (or the Tier-1 gate) already
        // approved this strike; 80u = 10m re-vetoed nearly every
        // midfield shot from range.
        // Approved strikes are authoritative — see the forward Shooting
        // state for why re-vetoing here silently discards them.
        if ctx.player.pending_shot_reason.is_none()
            && distance_to_goal > 240.0
            && !ctx.player().has_clear_shot()
        {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Use the transition-point reason if one was tagged (matches
        // the forward Shooting pattern + the pass-reason convention).
        // Falls back to distance bucket only when the transition came
        // from a path that didn't tag.
        let reason = ctx.player.pending_shot_reason.unwrap_or({
            if distance_to_goal <= 30.0 {
                "MID_SHOOTING_CLOSE"
            } else if distance_to_goal <= 60.0 {
                "MID_SHOOTING_MEDIUM"
            } else {
                "MID_SHOOTING_LONG"
            }
        });

        Some(StateChangeResult::with_midfielder_state_and_event(
            MidfielderRead::released(ctx),
            Event::PlayerEvent(PlayerEvent::Shoot(
                ShootingEventContext::builder()
                    .with_player_id(ctx.player.id)
                    .with_target(ctx.player().shooting_direction())
                    .with_reason(reason)
                    .build(ctx),
            )),
        ))
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Midfielder remains stationary while taking the shot
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Shooting is very high intensity - explosive action
        MidfielderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
