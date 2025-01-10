use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use rand::Rng;

const TACKLE_DISTANCE_THRESHOLD: f32 = 2.0; // Maximum distance to attempt a sliding tackle (in meters)
const FOUL_CHANCE_BASE: f32 = 0.2; // Base chance of committing a foul

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
        }

        if !ctx.ball().is_owned() && ctx.ball().distance() < 150.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting,
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
        }

        if ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
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
        let mut rng = rand::thread_rng();

        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let aggression = ctx.player.skills.mental.aggression / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;

        let overall_skill = (tackling_skill + composure) / 2.0;

        let opponent_dribbling = ctx.player().skills(opponent.id).technical.dribbling / 20.0;
        let opponent_agility = ctx.player().skills(opponent.id).physical.agility / 20.0;

        let skill_difference = overall_skill - (opponent_dribbling + opponent_agility) / 2.0;

        let success_chance = 0.5 + skill_difference * 0.3;
        let clamped_success_chance = success_chance.clamp(0.1, 0.9);

        let tackle_success = rng.gen::<f32>() < clamped_success_chance;

        let foul_chance = if tackle_success {
            (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.05
        } else {
            (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.15
        };

        let committed_foul = rng.gen::<f32>() < foul_chance;

        (tackle_success, committed_foul)
    }
}
