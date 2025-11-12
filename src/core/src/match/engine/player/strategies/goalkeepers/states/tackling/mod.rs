use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;
use rand::Rng;

const TACKLE_DISTANCE_THRESHOLD: f32 = 2.0; // Maximum distance to attempt a tackle (in meters)
const TACKLE_SUCCESS_BASE_CHANCE: f32 = 0.7; // Base chance of successful tackle for goalkeeper
const FOUL_CHANCE_BASE: f32 = 0.1; // Base chance of committing a foul for goalkeeper

#[derive(Default)]
pub struct GoalkeeperTacklingState {}

impl StateProcessingHandler for GoalkeeperTacklingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let opponents = ctx.players().opponents();
        let mut opponents_with_ball = opponents.with_ball();

        if let Some(opponent) = opponents_with_ball.next() {
            // 3. Calculate the distance to the opponent
            let distance_to_opponent = (ctx.player.position - opponent.position).magnitude();

            if distance_to_opponent > TACKLE_DISTANCE_THRESHOLD {
                // Opponent is too far to attempt a tackle
                // Transition back to appropriate state (e.g., Standing)
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Standing,
                ));
            }

            // 4. Attempt the tackle
            let (tackle_success, committed_foul) = self.attempt_tackle(ctx);

            if tackle_success {
                // Tackle is successful
                let mut state_change =
                    StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall);

                // Gain possession of the ball
                state_change
                    .events
                    .add(Event::PlayerEvent(PlayerEvent::GainBall(ctx.player.id)));

                // Update opponent's state to reflect loss of possession
                // This assumes you have a mechanism to update other players' states
                // You may need to send an event or directly modify the opponent's state

                // Optionally reduce goalkeeper's stamina
                // ctx.player.player_attributes.reduce_stamina(tackle_stamina_cost);

                Some(state_change)
            } else if committed_foul {
                // Tackle resulted in a foul
                let mut state_change =
                    StateChangeResult::with_goalkeeper_state(GoalkeeperState::Standing);

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
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Standing,
                ));
            }
        } else {
            // No opponent with the ball found
            // Transition back to appropriate state
            Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the opponent to attempt the tackle

        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            // Calculate direction towards the opponent
            let direction = (opponent.position - ctx.player.position).normalize();
            // Set speed based on player's pace
            let speed = ctx.player.skills.physical.pace;
            Some(direction * speed)
        } else {
            // No opponent with the ball found
            // Remain stationary or move back to position
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tackling is a very high intensity activity requiring explosive effort
        GoalkeeperCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl GoalkeeperTacklingState {
    /// Attempts a tackle and returns whether it was successful and if a foul was committed.
    fn attempt_tackle(&self, ctx: &StateProcessingContext) -> (bool, bool) {
        let mut rng = rand::rng();

        // Get goalkeeper's tackling-related skills
        let tackling_skill = ctx.player.skills.technical.tackling as f32 / 20.0; // Normalize to [0,1]
        let aggression = ctx.player.skills.mental.aggression as f32 / 20.0;
        let composure = ctx.player.skills.mental.composure as f32 / 20.0;

        let overall_skill = (tackling_skill + composure) / 2.0;

        // Calculate success chance
        let success_chance = overall_skill * TACKLE_SUCCESS_BASE_CHANCE;

        // Simulate tackle success
        let tackle_success = rng.random::<f32>() < success_chance;

        // Calculate foul chance
        let foul_chance = (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.05;

        // Simulate foul
        let committed_foul = !tackle_success && rng.random::<f32>() < foul_chance;

        (tackle_success, committed_foul)
    }
}
