use crate::r#match::events::Event;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::players::ShotQualityEvaluator;
use crate::r#match::player::strategies::players::MIN_XG_THRESHOLD;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct MidfielderShootingState {}

impl StateProcessingHandler for MidfielderShootingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Pressing
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        // Check shot cooldown
        let current_tick = ctx.current_tick();
        if !ctx.memory().can_shoot(current_tick) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Passing,
            ));
        }

        // Evaluate xG
        let xg = ShotQualityEvaluator::evaluate(ctx);
        if xg < MIN_XG_THRESHOLD {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Passing,
            ));
        }

        Some(StateChangeResult::with_midfielder_state_and_event(MidfielderState::Standing, Event::PlayerEvent(PlayerEvent::Shoot(
            ShootingEventContext::new()
                .with_player_id(ctx.player.id)
                .with_target(ctx.player().shooting_direction())
                .with_reason("MID_SHOOTING")
                .build(ctx)
        ))))
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // No slow processing needed
        None
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
