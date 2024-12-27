use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::{Event, EventCollection};
use crate::r#match::player::events::{PassingEventModel, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler, VectorExtensions};
use nalgebra::Vector3;
use rand::prelude::IteratorRandom;
use std::sync::LazyLock;
use crate::r#match::midfielders::states::MidfielderState;

static DEFENDER_PASSING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_passing_data.json")));

#[derive(Default)]
pub struct DefenderPassingState {}

impl StateProcessingHandler for DefenderPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        if let Some(teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventModel::build()
                        .with_player_id(ctx.player.id)
                        .with_target(teammate.position)
                        .with_force(ctx.player().pass_teammate_power(teammate.id))
                        .build()
                )),
            ));
        }
        
        let mut best_player_id = None;
        let mut highest_score = 0.0;

        for (player_id, teammate_distance) in ctx.players().teammates().nearby_ids(200.0) {
            let score = 1.0 / (teammate_distance + 1.0);
            if score > highest_score {
                highest_score = score;
                best_player_id = Some(player_id);
            }
        }

        if let Some(teammate_id) = best_player_id {
            let events = EventCollection::with_event(Event::PlayerEvent(PlayerEvent::PassTo(
                PassingEventModel::build()
                    .with_player_id(ctx.player.id)
                    .with_target(ctx.tick_context.positions.players.position(teammate_id))
                    .with_force(ctx.player().pass_teammate_power(teammate_id))
                    .build(),
            )));

            return Some(StateChangeResult::with_events(events));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        None
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl DefenderPassingState {
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
            .filter(|p| !p.tactical_positions.is_goalkeeper())
            .filter(|teammate| {
                // Check if the teammate is in a dangerous position near the opponent's goal
                let goal_distance_threshold = ctx.context.field_size.width as f32 * 0.2;
                (teammate.position - ctx.ball().direction_to_opponent_goal()).magnitude()
                    < goal_distance_threshold
            })
            .min_by(|a, b| {
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
            .filter(|p| !p.tactical_positions.is_goalkeeper())
            .min_by(|a, b| {
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
            .filter(|p| !p.tactical_positions.is_goalkeeper())
            .filter(|teammate| {
                // Check if the teammate is in a dangerous position near the opponent's goal
                let goal_distance_threshold = ctx.context.field_size.width as f32 * 0.2;
                (teammate.position - ctx.ball().direction_to_opponent_goal()).magnitude()
                    < goal_distance_threshold
            })
            .min_by(|a, b| {
                let dist_a = (a.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                let dist_b = (b.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });

        nearest_to_goal
    }

    pub fn calculate_pass_power(&self, teammate_id: u32, ctx: &StateProcessingContext) -> f64 {
        let distance = ctx.tick_context.distances.get(ctx.player.id, teammate_id);

        let pass_skill = ctx.player.skills.technical.passing;

        (distance / pass_skill as f32 * 10.0) as f64
    }
}
