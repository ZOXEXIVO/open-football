use crate::r#match::defenders::states::shared::{
    ActivityIntensity, DefenderCondition, DefenderRead,
};
use crate::r#match::events::Event;
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct DefenderShootingState {}

impl StateProcessingHandler for DefenderShootingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Must have possession to shoot
        if !ctx.player.has_ball(ctx) {
            return Some(DefenderRead::change(ctx));
        }

        // Last-second quality check: defenders should almost never shoot from distance
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        if distance_to_goal > 35.0 && !ctx.player().has_clear_shot() {
            return Some(DefenderRead::change(ctx));
        }

        Some(StateChangeResult::with_defender_state_and_event(
            DefenderRead::released(ctx),
            Event::PlayerEvent(PlayerEvent::Shoot(
                ShootingEventContext::builder()
                    .with_player_id(ctx.player.id)
                    .with_target(ctx.player().shooting_direction())
                    .with_reason("DEF_SHOOTING")
                    .build(ctx),
            )),
        ))
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Shooting is explosive - powerful leg action requires significant energy
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderShootingState {}
