use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::events::EventCollection;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

static GOALKEEPER_THROWING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_throwing_data.json")));

const THROW_DISTANCE_THRESHOLD: f32 = 30.0; // Minimum distance to consider for throwing
const THROW_POWER_MULTIPLIER: f32 = 10.0; // Multiplier for throw power calculation

#[derive(Default)]
pub struct GoalkeeperThrowingState {}

impl StateProcessingHandler for GoalkeeperThrowingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the goalkeeper has the ball
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 2. Find the best teammate to throw the ball to
        let players = ctx.players();
        let teammates = players.teammates();

        let teammates = teammates.all();
        let best_teammate = teammates
            .filter(|teammate| {
                let distance = (teammate.position - ctx.player.position).magnitude();
                distance >= THROW_DISTANCE_THRESHOLD
            })
            .max_by(|a, b| {
                let dist_a = (a.position - ctx.player.position).magnitude();
                let dist_b = (b.position - ctx.player.position).magnitude();
                dist_a
                    .partial_cmp(&dist_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        if let Some(teammate) = best_teammate {
            let mut events = EventCollection::new();

            events.add_player_event(PlayerEvent::PassTo(
                PassingEventContext::build()
                    .with_player_id(ctx.player.id)
                    .with_target(teammate.position)
                    .with_force(ctx.player().throw_teammate_power(teammate.id))
                    .build()
            ));

            return Some(StateChangeResult::with_events(events));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Remain stationary while throwing the ball
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions to process in this state
    }
}
