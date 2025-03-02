use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
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
                        PassingEventContext::build()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(teammate.id)
                            .with_target(teammate.position)
                            .with_force(ctx.player().pass_teammate_power(teammate.id))
                            .build(),
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
            if let Some(nearest_opponent) = ctx.players().opponents().nearby(100.0).next() {
                let player_goal_velocity = SteeringBehavior::Evade {
                    target: nearest_opponent.position,
                }
                .calculate(ctx.player)
                .velocity;

                Some(player_goal_velocity)
            } else {
                Some(
                    SteeringBehavior::Wander {
                        target: ctx.player.start_position + ctx.player().separation_velocity(),
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
                target: ctx.tick_context.positions.ball.position
                    + ctx.player().separation_velocity(),
                slowing_distance,
            }
            .calculate(ctx.player)
            .velocity;

            Some(result)
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl GoalkeeperRunningState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let vision_range = ctx.player.skills.mental.vision * 15.0;
        let open_teammates: Vec<MatchPlayerLite> = ctx
            .players()
            .teammates()
            .nearby(vision_range)
            .filter(|t| self.is_teammate_open(ctx, t) && ctx.player().has_clear_pass(t.id))
            .collect();

        if !open_teammates.is_empty() {
            open_teammates
                .iter()
                .min_by(|a, b| {
                    let risk_a = self.estimate_interception_risk(ctx, a);
                    let risk_b = self.estimate_interception_risk(ctx, b);
                    risk_a
                        .partial_cmp(&risk_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned()
        } else {
            None
        }
    }

    fn is_teammate_open(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let opponent_distance_threshold = 5.0;
        ctx.players().opponents().all()
            .filter(|o| (o.position - teammate.position).magnitude() <= opponent_distance_threshold)
            .count() == 0
    }

    fn estimate_interception_risk(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> f32 {
        let max_interception_distance = 10.0;
        let player_position = ctx.player.position;
        let pass_direction = (teammate.position - player_position).normalize();

        ctx.players().opponents().all()
            .filter(|o| (o.position - player_position).dot(&pass_direction) > 0.0)
            .map(|o| (o.position - player_position).magnitude())
            .filter(|d| *d <= max_interception_distance)
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(max_interception_distance)
    }
}
