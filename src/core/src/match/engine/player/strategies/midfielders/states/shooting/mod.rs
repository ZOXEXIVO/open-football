use crate::r#match::events::Event;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
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
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
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
        if distance_to_goal > 80.0 && !ctx.player().has_clear_shot() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        let reason = if distance_to_goal <= 30.0 {
            "MID_SHOOTING_CLOSE"
        } else if distance_to_goal <= 60.0 {
            "MID_SHOOTING_MEDIUM"
        } else {
            "MID_SHOOTING_LONG"
        };

        Some(StateChangeResult::with_midfielder_state_and_event(MidfielderState::Standing, Event::PlayerEvent(PlayerEvent::Shoot(
            ShootingEventContext::new()
                .with_player_id(ctx.player.id)
                .with_target(ctx.player().shooting_direction())
                .with_reason(reason)
                .build(ctx)
        ))))
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
