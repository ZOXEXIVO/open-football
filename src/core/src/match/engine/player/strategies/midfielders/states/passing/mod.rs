use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;
use std::sync::LazyLock;

static _MIDFIELDER_LONG_PASSING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_passing_data.json")));

#[derive(Default)]
pub struct MidfielderPassingState {}

impl StateProcessingHandler for MidfielderPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Pressing
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        // Determine the best teammate to pass to
        if let Some(target_teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::build()
                        .with_player_id(ctx.player.id)
                        .with_target(target_teammate.position)
                        .with_force(ctx.player().pass_teammate_power(target_teammate.id))
                        .build()
                )),
            ));
        }

        if ctx.ball().distance_to_opponent_goal() < 200.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Shooting,
            ))
        }
        
        if ctx.in_state_time > 10 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Distributing,
            ))
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if ctx.in_state_time % 10 == 0 {
            if let Some(nearest_teammate) = ctx.players().teammates().nearby_to_opponent_goal() {
                return Some(
                    SteeringBehavior::Arrive {
                        target: nearest_teammate.position,
                        slowing_distance: 30.0,
                    }
                    .calculate(ctx.player)
                    .velocity,
                );
            }
        }

        None
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderPassingState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;

        let attacking_third_start = if ctx.player.side == Some(PlayerSide::Left) {
            field_width * (2.0 / 3.0)
        } else {
            field_width / 3.0
        };

        if player_position.x >= attacking_third_start {
            // Player is in the attacking third, prioritize teammates near the opponent's goal
            self.find_best_pass_option_attacking_third(ctx)
        } else if player_position.x >= field_width / 3.0
            && player_position.x <= field_width * (2.0 / 3.0)
        {
            // Player is in the middle third, prioritize teammates in advanced positions
            self.find_best_pass_option_middle_third(ctx)
        } else {
            // Player is in the defensive third, prioritize safe passes to nearby teammates
            self.find_best_pass_option_defensive_third(ctx)
        }
    }

    fn find_best_pass_option_attacking_third(
        &self,
        ctx: &StateProcessingContext<'_>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        let nearest_to_goal = teammates
            .all()
            .filter(|teammate| {
                // Check if the teammate is in a dangerous position near the opponent's goal
                let goal_distance_threshold = ctx.context.field_size.width as f32 * 0.2;
                (teammate.position - ctx.ball().direction_to_opponent_goal()).magnitude()
                    < goal_distance_threshold
            })
            .max_by(|a, b| {
                let dist_a = (a.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                let dist_b = (b.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });

        nearest_to_goal
    }

    fn find_best_pass_option_defensive_third<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        let nearest_teammate = teammates
            .nearby(200.0)
            .max_by(|a, b| {
                let dist_a = (a.position - ctx.player.position).magnitude();
                let dist_b = (b.position - ctx.player.position).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });

        nearest_teammate
    }

    fn find_best_pass_option_middle_third(
        &self,
        ctx: &StateProcessingContext<'_>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        let nearest_to_goal = teammates
            .all()
            .filter(|teammate| {
                // Check if the teammate is in a dangerous position near the opponent's goal
                let goal_distance_threshold = ctx.context.field_size.width as f32 * 0.2;
                (teammate.position - ctx.ball().direction_to_opponent_goal()).magnitude()
                    < goal_distance_threshold
            })
            .max_by(|a, b| {
                let dist_a = (a.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                let dist_b = (b.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });

        nearest_to_goal
    }
}
