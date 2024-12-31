use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use rand::Rng;
use std::sync::LazyLock;

static DEFENDER_TACKLING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_tackling_data.json")));

const TACKLE_DISTANCE_THRESHOLD: f32 = 2.0; // Maximum distance to attempt a sliding tackle (in meters)
const TACKLE_SUCCESS_BASE_CHANCE: f32 = 0.6; // Base chance of successful tackle
const FOUL_CHANCE_BASE: f32 = 0.2; // Base chance of committing a foul
const STAMINA_THRESHOLD: f32 = 25.0; // Minimum stamina to attempt a sliding tackle

#[derive(Default)]
pub struct DefenderTacklingState {}

impl StateProcessingHandler for DefenderTacklingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            if ctx.team().is_control_ball() {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Running,
                ));
            }

            let is_far_from_start_position = match ctx.player().position_to_distance() {
                PlayerDistanceFromStartPosition::Big => Some(
                    StateChangeResult::with_defender_state(DefenderState::Returning),
                ), // Continue tracking back
                PlayerDistanceFromStartPosition::Medium => Some(
                    StateChangeResult::with_defender_state(DefenderState::Returning),
                ),
                PlayerDistanceFromStartPosition::Small => None,
            };

            if let Some(_is_far_from_start_position) = is_far_from_start_position {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Running,
                ));
            }
        }

        // 1. Check defender's stamina
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
            // Transition to Resting state if stamina is too low
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Resting,
            ));
        }

        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            if opponent.distance(ctx) > TACKLE_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // 4. Attempt the sliding tackle
            let (tackle_success, committed_foul) = self.attempt_sliding_tackle(ctx, &opponent);

            if tackle_success {
                // Tackle is successful
                let mut state_change =
                    StateChangeResult::with_defender_state(DefenderState::Standing);

                // Gain possession of the ball
                state_change
                    .events
                    .add(Event::PlayerEvent(PlayerEvent::GainBall(ctx.player.id)));

                return Some(state_change);
            } else if committed_foul {
                // Tackle resulted in a foul
                let mut state_change =
                    StateChangeResult::with_defender_state(DefenderState::Standing);

                // Generate a foul event
                state_change
                    .events
                    .add_player_event(PlayerEvent::CommitFoul);

                return Some(state_change);
            } else {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Standing,
                ));
            }
        }
        
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.tick_context.positions.ball.position,
                slowing_distance: 10.0,
            }
                .calculate(ctx.player)
                .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}

impl DefenderTacklingState {
    /// Attempts a sliding tackle and returns whether it was successful and if a foul was committed.
    fn attempt_sliding_tackle(
        &self,
        ctx: &StateProcessingContext,
        _opponent: &MatchPlayerLite,
    ) -> (bool, bool) {
        let mut rng = rand::thread_rng();

        // Get defender's tackling-related skills
        let tackling_skill = ctx.player.skills.technical.tackling  / 20.0; // Normalize to [0,1]
        let aggression = ctx.player.skills.mental.aggression  / 20.0;
        let composure = ctx.player.skills.mental.composure  / 20.0;

        let overall_skill = (tackling_skill + composure) / 2.0;

        // Calculate success chance
        let success_chance = overall_skill * TACKLE_SUCCESS_BASE_CHANCE;

        // Simulate tackle success
        let tackle_success = rng.gen::<f32>() < success_chance;

        // Calculate foul chance
        let foul_chance = (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.1;

        // Simulate foul
        let committed_foul = !tackle_success && rng.gen::<f32>() < foul_chance;

        (tackle_success, committed_foul)
    }
}
