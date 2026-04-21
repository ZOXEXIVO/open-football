use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{FoulSeverity, PlayerEvent};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;
use rand::RngExt;

const TACKLE_DISTANCE_THRESHOLD: f32 = 2.0; // Maximum distance to attempt a tackle (in meters)
const TACKLE_SUCCESS_BASE_CHANCE: f32 = 0.7; // Base chance of successful tackle for goalkeeper
const FOUL_CHANCE_BASE: f32 = 0.1; // Base chance of committing a foul for goalkeeper

#[derive(Default, Clone)]
pub struct GoalkeeperTacklingState {}

impl StateProcessingHandler for GoalkeeperTacklingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Shared tackle cooldown. Without it the keeper re-attempts every
        // tick while the attacker is in range, generating fouls and/or
        // compounding the inflated tackle count.
        if !ctx.player.can_attempt_tackle() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        let opponents = ctx.players().opponents();
        let mut opponents_with_ball = opponents.with_ball();

        if let Some(opponent) = opponents_with_ball.next() {
            let distance_to_opponent = (ctx.player.position - opponent.position).magnitude();

            if distance_to_opponent > TACKLE_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Standing,
                ));
            }

            let (tackle_success, committed_foul, foul_severity) = self.attempt_tackle(ctx);

            if tackle_success {
                let mut state_change =
                    StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall);
                state_change
                    .events
                    .add(Event::PlayerEvent(PlayerEvent::TacklingBall(ctx.player.id)));
                state_change.start_tackle_cooldown = true;
                Some(state_change)
            } else if committed_foul {
                let mut state_change =
                    StateChangeResult::with_goalkeeper_state(GoalkeeperState::Standing);
                state_change
                    .events
                    .add_player_event(PlayerEvent::CommitFoul(
                        ctx.player.id,
                        foul_severity,
                    ));
                state_change.start_tackle_cooldown = true;
                return Some(state_change);
            } else {
                let mut state_change = StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Standing,
                );
                state_change.start_tackle_cooldown = true;
                return Some(state_change);
            }
        } else {
            Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ))
        }
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
    fn attempt_tackle(&self, ctx: &StateProcessingContext) -> (bool, bool, FoulSeverity) {
        let mut rng = rand::rng();

        let tackling_skill = ctx.player.skills.technical.tackling as f32 / 20.0;
        let aggression = ctx.player.skills.mental.aggression as f32 / 20.0;
        let composure = ctx.player.skills.mental.composure as f32 / 20.0;

        let overall_skill = (tackling_skill + composure) / 2.0;
        let success_chance = overall_skill * TACKLE_SUCCESS_BASE_CHANCE;
        let tackle_success = rng.random::<f32>() < success_chance;

        let foul_chance = (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.05;
        let committed_foul = !tackle_success && rng.random::<f32>() < foul_chance;

        // A keeper tackling outside the area nearly always means a
        // last-man challenge — classify most fouls as Violent (straight red).
        let severity = if !committed_foul {
            FoulSeverity::Normal
        } else if rng.random::<f32>() < 0.65 {
            FoulSeverity::Violent
        } else {
            FoulSeverity::Reckless
        };

        (tackle_success, committed_foul, severity)
    }
}
