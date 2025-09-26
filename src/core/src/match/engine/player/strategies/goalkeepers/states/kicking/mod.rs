use crate::r#match::events::EventCollection;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const KICK_DISTANCE_THRESHOLD: f32 = 30.0; // Maximum distance to consider for kicking

#[derive(Default)]
pub struct GoalkeeperKickingState {}

impl StateProcessingHandler for GoalkeeperKickingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the goalkeeper has the ball
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 2. Find the best teammate to kick the ball to
        let players = ctx.players();
        let teammates = players.teammates();

        if let Some(teammate) =  teammates.nearby(KICK_DISTANCE_THRESHOLD).next() {
            let mut events = EventCollection::new();

            events.add_player_event(PlayerEvent::PassTo(
                PassingEventContext::new()
                    .with_from_player_id(ctx.player.id)
                    .with_to_player_id(teammate.id)                   
                    .build(ctx)
            ));

            return Some(StateChangeResult::with_events(events));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
    }
}
