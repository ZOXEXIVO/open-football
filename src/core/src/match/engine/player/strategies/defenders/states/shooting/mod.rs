use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::events::Event;
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::players::ShotQualityEvaluator;
use crate::r#match::player::strategies::players::MIN_XG_THRESHOLD;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct DefenderShootingState {}

impl StateProcessingHandler for DefenderShootingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check shot cooldown
        let current_tick = ctx.current_tick();
        if !ctx.memory().can_shoot(current_tick) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        // Evaluate xG
        let xg = ShotQualityEvaluator::evaluate(ctx);
        if xg < MIN_XG_THRESHOLD {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        Some(StateChangeResult::with_defender_state_and_event(
            DefenderState::Standing,
            Event::PlayerEvent(PlayerEvent::Shoot(
                ShootingEventContext::new()
                    .with_player_id(ctx.player.id)
                    .with_target(ctx.player().shooting_direction())
                    .with_reason("DEF_SHOOTING")
                    .build(ctx)
            )),
        ))
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
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
