use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use rand::Rng;

const TACKLE_DISTANCE_THRESHOLD: f32 = 5.0; // Maximum distance to attempt a tackle (in meters)
const FOUL_CHANCE_BASE: f32 = 0.2; // Base chance of committing a foul

#[derive(Default)]
pub struct MidfielderTacklingState {}

impl StateProcessingHandler for MidfielderTacklingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        if ctx.ball().distance() > 50.0 && !ctx.ball().is_towards_player_with_angle(0.8) {
            return if ctx.team().is_control_ball() {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::AttackSupporting,
                ))
            } else {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ))
            };
        }

        let players = ctx.players();
        let opponents = players.opponents();
        let mut opponents_with_ball = opponents.with_ball();

        if let Some(opponent) = opponents_with_ball.next() {
            let opponent_distance = ctx.tick_context.distances.get(ctx.player.id, opponent.id);
            if opponent_distance <= TACKLE_DISTANCE_THRESHOLD {
                let (tackle_success, committed_foul) = self.attempt_tackle(ctx, &opponent);
                if tackle_success {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::HoldingPossession,
                        Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                    ));
                } else if committed_foul {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Standing,
                        Event::PlayerEvent(PlayerEvent::CommitFoul),
                    ));
                }
            }
        } else if self.can_intercept_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Running,
                Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Pursuit {
                target: ctx.tick_context.positions.ball.position + ctx.player().separation_velocity(),
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
        opponent: &MatchPlayerLite,
    ) -> (bool, bool) {
        let mut rng = rand::thread_rng();

        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let aggression = ctx.player.skills.mental.aggression / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;

        let overall_skill = (tackling_skill + composure) / 2.0;

        // Calculate opponent's dribbling and agility skills
        let opponent_dribbling = ctx.player().skills(opponent.id).technical.dribbling / 20.0;
        let opponent_agility = ctx.player().skills(opponent.id).physical.agility / 20.0;

        // Calculate the relative skill difference between the tackler and the opponent
        let skill_difference = overall_skill - (opponent_dribbling + opponent_agility) / 2.0;

        // Calculate success chance based on the skill difference
        let success_chance = 0.5 + skill_difference * 0.3;
        let clamped_success_chance = success_chance.clamp(0.1, 0.9);

        // Simulate tackle success
        let tackle_success = rng.gen::<f32>() < clamped_success_chance;

        // Calculate foul chance
        let foul_chance = if tackle_success {
            // Lower foul chance for successful tackles
            (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.05
        } else {
            // Higher foul chance for unsuccessful tackles
            (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.15
        };

        // Simulate foul
        let committed_foul = rng.gen::<f32>() < foul_chance;

        (tackle_success, committed_foul)
    }

    fn can_intercept_ball(&self, ctx: &StateProcessingContext) -> bool {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let player_position = ctx.player.position;
        let player_speed = ctx.player.skills.physical.pace;

        if !ctx.tick_context.ball.is_owned && ball_velocity.magnitude() > 0.1 {
            let time_to_ball = (ball_position - player_position).magnitude() / player_speed;
            let ball_travel_distance = ball_velocity.magnitude() * time_to_ball;
            let ball_intercept_position =
                ball_position + ball_velocity.normalize() * ball_travel_distance;
            let player_intercept_distance = (ball_intercept_position - player_position).magnitude();

            player_intercept_distance <= TACKLE_DISTANCE_THRESHOLD
        } else {
            false
        }
    }
}
