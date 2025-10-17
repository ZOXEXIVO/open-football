use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use rand::Rng;

const TACKLE_DISTANCE_THRESHOLD: f32 = 3.0;
const FOUL_CHANCE_BASE: f32 = 0.2;

#[derive(Default)]
pub struct DefenderTacklingState {}

impl StateProcessingHandler for DefenderTacklingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) || ctx.team().is_control_ball(){
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        if ctx.ball().distance() > 200.0 && !ctx.ball().is_towards_player_with_angle(0.8){
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }
        
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            if opponent.distance(ctx) > TACKLE_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            let (tackle_success, committed_foul) = self.attempt_sliding_tackle(ctx, &opponent);

            return if tackle_success {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::GainBall(ctx.player.id)),
                ));
            } else if committed_foul {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::CommitFoul),
                ));
            } else {
                Some(StateChangeResult::with_defender_state(
                    DefenderState::Standing,
                ))
            };
        } else if self.can_intercept_ball(ctx) {
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderState::Running,
                Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
            ));
        }

        // Fallback: if ball is loose and very close, try to claim it
        let ball_distance = ctx.ball().distance();
        if !ctx.tick_context.ball.is_owned && ball_distance < 5.0 {
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderState::Running,
                Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
            ));
        }

        // Timeout fallback: if stuck in tackling state too long, transition out
        if ctx.in_state_time > 30 {
            return Some(StateChangeResult::with_defender_state(
                if ball_distance < 50.0 {
                    DefenderState::Pressing
                } else {
                    DefenderState::Returning
                }
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
            SteeringBehavior::Arrive {
                target: ctx.tick_context.positions.ball.position,
                slowing_distance: 0.0,
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
    fn attempt_sliding_tackle(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> (bool, bool) {
        let mut rng = rand::rng();

        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let aggression = ctx.player.skills.mental.aggression / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;

        let overall_skill = (tackling_skill + composure) / 2.0;

        let opponent_dribbling = ctx.player().skills(opponent.id).technical.dribbling / 20.0;
        let opponent_agility = ctx.player().skills(opponent.id).physical.agility / 20.0;

        let skill_difference = overall_skill - (opponent_dribbling + opponent_agility) / 2.0;

        let success_chance = 0.5 + skill_difference * 0.3;
        let clamped_success_chance = success_chance.clamp(0.1, 0.9);

        let tackle_success = rng.random::<f32>() < clamped_success_chance;

        let foul_chance = if tackle_success {
            (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.05
        } else {
            (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.15
        };

        let committed_foul = rng.random::<f32>() < foul_chance;

        (tackle_success, committed_foul)
    }

    fn exists_nearby(&self, ctx: &StateProcessingContext) -> bool {
        const DISTANCE: f32 = 30.0;

        ctx.players().opponents().exists(DISTANCE) || ctx.players().teammates().exists(DISTANCE)
    }

    fn can_intercept_ball(&self, ctx: &StateProcessingContext) -> bool {
        if self.exists_nearby(ctx){
            return false;
        }

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
