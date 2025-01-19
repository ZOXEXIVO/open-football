use crate::r#match::defenders::states::DefenderState;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperRunningState {}

impl StateProcessingHandler for GoalkeeperRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            if self.should_pass(ctx) {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Passing,
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
            let goal_direction = ctx.player().opponent_goal_position();

            let player_goal_velocity = SteeringBehavior::Arrive {
                target: goal_direction + ctx.player().separation_velocity(),
                slowing_distance: 100.0,
            }
            .calculate(ctx.player)
            .velocity;

            Some(player_goal_velocity)
        } else if ctx.player().goal_distance() < 150.0 && ctx.players().opponents().exists(50.0) {
            let players = ctx.players();
            let opponents = players.opponents();

            if let Some(goalkeeper) = opponents.goalkeeper().next() {
                let result = SteeringBehavior::Evade {
                    target: goalkeeper.position + ctx.player().separation_velocity(),
                }
                .calculate(ctx.player)
                .velocity;

                return Some(result + ctx.player().separation_velocity());
            }

            None
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
    pub fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.players().opponents().exists(50.0) {
            return true;
        }

        let game_vision_skill = ctx.player.skills.mental.vision;
        let game_vision_threshold = 14.0; // Adjust this value based on your game balance

        if game_vision_skill >= game_vision_threshold {
            if let Some(_) = self.find_open_teammate_on_opposite_side(ctx) {
                return true;
            }
        }

        false
    }

    fn find_open_teammate_on_opposite_side(
        &self,
        ctx: &StateProcessingContext,
    ) -> Option<MatchPlayerLite> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let opposite_side_x = match ctx.player.side {
            Some(PlayerSide::Left) => field_width * 0.75,
            Some(PlayerSide::Right) => field_width * 0.25,
            None => return None,
        };

        let mut open_teammates: Vec<MatchPlayerLite> = ctx
            .players()
            .teammates()
            .nearby(200.0)
            .filter(|teammate| {
                let is_on_opposite_side = match ctx.player.side {
                    Some(PlayerSide::Left) => teammate.position.x > opposite_side_x,
                    Some(PlayerSide::Right) => teammate.position.x < opposite_side_x,
                    None => false,
                };
                let is_open = !ctx
                    .players()
                    .opponents()
                    .nearby(20.0)
                    .any(|opponent| opponent.id == teammate.id);
                is_on_opposite_side && is_open
            })
            .collect();

        if open_teammates.is_empty() {
            None
        } else {
            open_teammates.sort_by(|a, b| {
                let dist_a = (a.position - player_position).magnitude();
                let dist_b = (b.position - player_position).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });
            Some(open_teammates[0])
        }
    }
}
