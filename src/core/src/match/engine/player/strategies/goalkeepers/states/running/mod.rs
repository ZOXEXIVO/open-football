use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use crate::IntegerUtils;
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperRunningState {}

impl StateProcessingHandler for GoalkeeperRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            if let Some(teammate) = self.find_best_pass_option(ctx) {
                return Some(StateChangeResult::with_goalkeeper_state_and_event(
                    GoalkeeperState::Standing,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(teammate.id)
                            .build(ctx),
                    )),
                ));
            }
        } else {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if ctx.player.has_ball(ctx) {
            if let Some(nearest_opponent) = ctx.players().opponents().nearby(200.0).next() {
                let player_goal_velocity = SteeringBehavior::Evade {
                    target: nearest_opponent.position,
                }
                    .calculate(ctx.player)
                    .velocity;

                Some(player_goal_velocity)
            } else {
                Some(
                    SteeringBehavior::Wander {
                        target: ctx.player.start_position,
                        radius: IntegerUtils::random(5, 150) as f32,
                        jitter: IntegerUtils::random(0, 2) as f32,
                        distance: IntegerUtils::random(10, 150) as f32,
                        angle: IntegerUtils::random(0, 360) as f32,
                    }
                        .calculate(ctx.player)
                        .velocity,
                )
            }
        } else {
            let slowing_distance: f32 = {
                if ctx.player().goal_distance() < 200.0 {
                    200.0
                } else {
                    10.0
                }
            };
            let result = SteeringBehavior::Arrive {
                target: ctx.tick_context.positions.ball.position,
                slowing_distance,
            }
                .calculate(ctx.player)
                .velocity;

            Some(result + ctx.player().separation_velocity())
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl GoalkeeperRunningState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        PassEvaluator::find_best_pass_option(ctx, 500.0)
    }
}
