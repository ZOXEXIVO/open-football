use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct ForwardShootingState {}

impl StateProcessingHandler for ForwardShootingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Must have possession to shoot
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Per-player + team shot cooldowns. A forward who struck within
        // the last 1.5 s is still recovering — balance disturbed, ball
        // gone, stance reset. Team cooldown (~500ms) prevents the whole
        // front line rifling shots every tick during sustained pressure.
        // Abort and drop back to Running; next tick they'll reassess
        // and most often pass or reposition. The entry gates in other
        // forward states already check these, but belt-and-braces here
        // catches any branch that slipped through (e.g. point-blank in
        // Running, which intentionally bypasses team cooldown above).
        if !ctx.player().can_shoot() || !ctx.team().can_shoot() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // Abort for very long range with no clear shot
        if distance_to_goal > 150.0 && !ctx.player().has_clear_shot() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Medium-long range: need decent finishing or clear shot to commit
        let finishing = ctx.player.skills.technical.finishing / 20.0;
        if distance_to_goal > 80.0 && finishing < 0.5 && !ctx.player().has_clear_shot() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Use the transition-point reason if one was tagged. The
        // caller (e.g. forwarders/running Priority 0.5) sets
        // `pending_shot_reason` so we can tell which code path fired
        // the shot in the per-match log — matches the pass-reason
        // pattern. Fall back to distance-bucketed reason if nothing
        // was tagged (rare — only direct-to-Shooting paths).
        let reason = ctx.player.pending_shot_reason.unwrap_or_else(|| {
            if distance_to_goal <= 30.0 {
                "FWD_SHOOTING_CLOSE"
            } else if distance_to_goal <= 60.0 {
                "FWD_SHOOTING_MEDIUM"
            } else {
                "FWD_SHOOTING_LONG"
            }
        });

        Some(StateChangeResult::with_forward_state_and_event(
            ForwardState::Running,
            Event::PlayerEvent(PlayerEvent::Shoot(
                ShootingEventContext::new()
                    .with_player_id(ctx.player.id)
                    .with_target(ctx.player().shooting_direction())
                    .with_reason(reason)
                    .build(ctx),
            )),
        ))
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Shooting is very high intensity - explosive action
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
