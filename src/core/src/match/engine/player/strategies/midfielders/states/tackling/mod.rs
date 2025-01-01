use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use rand::Rng;
use std::sync::LazyLock;

static MIDFIELDER_TACKLING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_tackling_data.json")));

const TACKLE_DISTANCE_THRESHOLD: f32 = 2.0; // Maximum distance to attempt a tackle (in meters)
const TACKLE_SUCCESS_BASE_CHANCE: f32 = 0.3; // Base chance of successful tackle
const FOUL_CHANCE_BASE: f32 = 0.2; // Base chance of committing a foul
const STAMINA_THRESHOLD: f32 = 30.0; // Minimum stamina to attempt a tackle

#[derive(Default)]
pub struct MidfielderTacklingState {}

impl StateProcessingHandler for MidfielderTacklingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {

        }

        let players = ctx.players();
        let opponents = players.opponents();
        let mut opponents_with_ball = opponents.with_ball();

        if let Some(opponent) = opponents_with_ball.next() {
            // 3. Calculate the distance to the opponent
            let distance_to_opponent = (ctx.player.position - opponent.position).magnitude();

            if distance_to_opponent > TACKLE_DISTANCE_THRESHOLD {
                // Opponent is too far to attempt a tackle
                // Transition back to appropriate state (e.g., Pressing)
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            // 4. Attempt the tackle
            let (tackle_success, committed_foul) = self.attempt_tackle(ctx, &opponent);

            let option = if tackle_success {
                // Tackle is successful
                let mut state_change =
                    StateChangeResult::with_midfielder_state(MidfielderState::HoldingPossession);

                // Gain possession of the ball
                state_change
                    .events
                    .add_player_event(PlayerEvent::GainBall(ctx.player.id));

                // Update opponent's state to reflect loss of possession
                // You may need to send an event or directly modify the opponent's state

                // Optionally reduce midfielder's stamina
                // ctx.player.player_attributes.reduce_stamina(tackle_stamina_cost);

                Some(state_change)
            } else if committed_foul {
                // Tackle resulted in a foul
                let mut state_change =
                    StateChangeResult::with_midfielder_state(MidfielderState::Standing);

                // Generate a foul event
                state_change
                    .events
                    .add_player_event(PlayerEvent::CommitFoul);

                // Transition to appropriate state (e.g., ReactingToFoul)
                // You may need to define additional states for handling fouls

                return Some(state_change);
            } else {
                // Tackle failed without committing a foul
                // Transition back to appropriate state
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Standing,
                ));
            };
            option
        } else {
            // No opponent with the ball found
            // Transition back to appropriate state
            Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Standing,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Pursuit {
                target: ctx.tick_context.positions.ball.position
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}

impl MidfielderTacklingState {
    /// Attempts a tackle and returns whether it was successful and if a foul was committed.
    fn attempt_tackle(
        &self,
        ctx: &StateProcessingContext,
        _opponent: &MatchPlayerLite,
    ) -> (bool, bool) {
        let mut rng = rand::thread_rng();

        // Get midfielder's tackling-related skills
        let tackling_skill = ctx.player.skills.technical.tackling as f32 / 20.0;
        let aggression = ctx.player.skills.mental.aggression as f32 / 20.0;
        let composure = ctx.player.skills.mental.composure as f32 / 20.0;

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
