use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::events::Event;
use crate::r#match::player::events::{FoulSeverity, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;
use rand::RngExt;

const TACKLE_DISTANCE_THRESHOLD: f32 = 12.0; // Sliding tackles have longer reach
const TACKLE_SUCCESS_BASE_CHANCE: f32 = 0.75; // Skilled defenders should win most slide tackles
const FOUL_CHANCE_BASE: f32 = 0.15; // Better-trained defenders commit fewer fouls
const STAMINA_THRESHOLD: f32 = 20.0; // Slide tackle even when tired — it's a commitment

#[derive(Default, Clone)]
pub struct DefenderSlidingTackleState {}

impl StateProcessingHandler for DefenderSlidingTackleState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check defender's stamina
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
            // Transition to Resting state if stamina is too low
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Resting,
            ));
        }

        // 2. Back off during foul protection
        if ctx.ball().is_in_flight() && ctx.ball().is_owned() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        // 3. Identify the opponent player with the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            // 3. Calculate the distance to the opponent
            let distance_to_opponent = (ctx.player.position - opponent.position).magnitude();

            if distance_to_opponent > TACKLE_DISTANCE_THRESHOLD {
                // Opponent is too far to attempt a sliding tackle
                // Transition back to appropriate state (e.g., Pressing)
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // 4. Attempt the sliding tackle
            let (tackle_success, committed_foul, foul_severity) =
                self.attempt_sliding_tackle(ctx, &opponent);

            if tackle_success {
                // Tackle is successful
                let mut state_change =
                    StateChangeResult::with_defender_state(DefenderState::Standing);

                // Gain possession of the ball
                state_change
                    .events
                    .add(Event::PlayerEvent(PlayerEvent::GainBall(ctx.player.id)));

                // Update opponent's state to reflect loss of possession
                // This assumes you have a mechanism to update other players' states
                // You may need to send an event or directly modify the opponent's state

                // Optionally reduce defender's stamina
                // ctx.player.player_attributes.reduce_stamina(tackle_stamina_cost);

                Some(state_change)
            } else if committed_foul {
                // Tackle resulted in a foul
                let mut state_change =
                    StateChangeResult::with_defender_state(DefenderState::Standing);

                // Generate a foul event — sliding tackles skew more reckless.
                state_change
                    .events
                    .add(Event::PlayerEvent(PlayerEvent::CommitFoul(
                        ctx.player.id,
                        foul_severity,
                    )));

                // Transition to appropriate state (e.g., ReactingToFoul)
                // You may need to define additional states for handling fouls

                Some(state_change)
            } else {
                // Tackle failed without committing a foul
                // Transition back to appropriate state
                Some(StateChangeResult::with_defender_state(
                    DefenderState::Standing,
                ))
            }
        } else {
            // No opponent with the ball found
            // Transition back to appropriate state
            Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ))
        }
    }


    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the opponent to attempt the sliding tackle

         // Get the opponent with the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            // Calculate direction towards the opponent
            let direction = (opponent.position - ctx.player.position).normalize();
            // Set speed based on player's pace, increased slightly for the slide
            let speed = ctx.player.skills.physical.pace * 1.1; // Increase speed by 10%
            Some(direction * speed)
        } else {
            // No opponent with the ball found
            // Remain stationary or move back to position
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Sliding tackle is explosive and very demanding - high energy output
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderSlidingTackleState {
    /// Attempts a sliding tackle and returns whether it was successful and if a foul was committed.
    fn attempt_sliding_tackle(
        &self,
        ctx: &StateProcessingContext,
        _opponent: &MatchPlayerLite,
    ) -> (bool, bool, FoulSeverity) {
        let mut rng = rand::rng();

        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let aggression = ctx.player.skills.mental.aggression / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;

        let overall_skill = (tackling_skill + composure) / 2.0;
        let success_chance = overall_skill * TACKLE_SUCCESS_BASE_CHANCE;
        let tackle_success = rng.random::<f32>() < success_chance;

        let foul_chance = (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.1;
        let committed_foul = !tackle_success && rng.random::<f32>() < foul_chance;

        // Slide tackles that miss are typically reckless. Violent studs-up
        // challenges are rare but possible for very aggressive players.
        let severity = if !committed_foul {
            FoulSeverity::Normal
        } else if aggression > 0.78 && rng.random::<f32>() < 0.14 {
            FoulSeverity::Violent
        } else {
            FoulSeverity::Reckless
        };

        (tackle_success, committed_foul, severity)
    }
}
