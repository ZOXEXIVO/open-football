use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::events::Event;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperShootingState {}

impl StateProcessingHandler for GoalkeeperShootingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        let event = Event::PlayerEvent(PlayerEvent::Shoot(ShootingEventContext::new()
            .with_player_id(ctx.player.id)
            .with_target(ctx.player().shooting_direction())
            .with_reason("GK_SHOOTING")
            .build(ctx)));

        // Transition to Standing immediately after shooting to prevent repeated shots
        Some(StateChangeResult::with_goalkeeper_state_and_event(
            GoalkeeperState::Standing,
            event,
        ))
    }


    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Remain stationary while shooting
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Shooting requires moderate intensity with focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}
