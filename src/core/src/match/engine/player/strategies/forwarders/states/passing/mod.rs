use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::{PassingEventModel, PlayerEvent};
use crate::r#match::result::VectorExtensions;
use crate::r#match::{
    ConditionContext, MatchPlayer, MatchPlayerLite, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use rand::prelude::IteratorRandom;
use std::sync::LazyLock;

static FORWARD_PASSING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_passing_data.json")));

#[derive(Default)]
pub struct ForwardPassingState {}

impl StateProcessingHandler for ForwardPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the player has the ball
        if !ctx.player.has_ball(ctx) {
            // Transition to Running state if the player doesn't have the ball
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Find the best passing option
        if let Some(teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_forward_state_and_event(
                ForwardState::Running,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventModel::build()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.tick_context.positions.players.position(teammate.id))
                        .with_force(ctx.player().pass_teammate_power(teammate.id))
                        .build(),
                )),
            ));
        }

        // Check if there's space to dribble forward
        if self.space_to_dribble(ctx) {
            // Transition to Dribbling state if there's space to dribble
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        // Check if there's an opportunity to shoot
        if self.can_shoot(ctx) {
            // Transition to Shooting state if there's an opportunity to shoot
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Shooting,
            ));
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

impl ForwardPassingState {
    pub fn calculate_pass_power(&self, teammate_id: u32, ctx: &StateProcessingContext) -> f64 {
        let distance = ctx.tick_context.distances.get(ctx.player.id, teammate_id);

        let pass_skill = ctx.player.skills.technical.passing;

        (distance / pass_skill as f32 * 10.0) as f64
    }

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
            .min_by(|a, b| {
                let dist_a = (a.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                let dist_b = (b.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });

        nearest_to_goal
    }

    fn find_best_pass_option_middle_third(
        &self,
        ctx: &StateProcessingContext<'_>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        teammates
            .nearby(200.0)
            .filter(|p| !p.tactical_positions.is_forward() && !p.tactical_positions.is_goalkeeper())
            .choose(&mut rand::thread_rng())
    }

    fn find_best_pass_option_defensive_third(
        &self,
        ctx: &StateProcessingContext<'_>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        let nearest_teammate = teammates.nearby(300.0).min_by(|a, b| {
            let dist_a = (a.position - ctx.player.position).magnitude();
            let dist_b = (b.position - ctx.player.position).magnitude();
            dist_a.partial_cmp(&dist_b).unwrap()
        });

        nearest_teammate
    }

    fn is_open_for_pass(&self, ctx: &StateProcessingContext, teammate: &MatchPlayer) -> bool {
        let max_distance = 20.0; // Adjust based on your game's scale

        let players = ctx.players();
        let opponents = players.opponents();

        let distance = ctx.tick_context.distances.get(ctx.player.id, teammate.id);

        if distance > max_distance {
            return false;
        }

        let mut all_opponents = opponents.all();

        all_opponents.all(|opponent| opponent.position.distance_to(&teammate.position) > 5.0)
    }

    fn in_passing_lane(&self, ctx: &StateProcessingContext, teammate: &MatchPlayer) -> bool {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_to_ball = (ball_position - ctx.player.position).normalize();
        let player_to_teammate = (teammate.position - ctx.player.position).normalize();

        // Check if the teammate is in the passing lane
        player_to_ball.dot(&player_to_teammate) > 0.8
    }

    fn scoring_chance(&self, ctx: &StateProcessingContext, teammate: &MatchPlayer) -> f32 {
        let goal_position = match teammate.side {
            Some(PlayerSide::Left) => ctx.context.goal_positions.right,
            Some(PlayerSide::Right) => ctx.context.goal_positions.left,
            _ => Vector3::new(0.0, 0.0, 0.0),
        };

        let distance_to_goal = teammate.position.distance_to(&goal_position);
        let angle_to_goal = self.angle_to_goal(ctx, teammate);

        // Calculate the scoring chance based on distance and angle to the goal
        (1.0 - distance_to_goal / ctx.context.field_size.width as f32) * angle_to_goal
    }

    fn angle_to_goal(&self, ctx: &StateProcessingContext, player: &MatchPlayer) -> f32 {
        let goal_position = match player.side {
            Some(PlayerSide::Left) => ctx.context.goal_positions.right,
            Some(PlayerSide::Right) => ctx.context.goal_positions.left,
            _ => Vector3::new(0.0, 0.0, 0.0),
        };

        let player_to_goal = (goal_position - player.position).normalize();
        let player_velocity = player.velocity.normalize();

        player_velocity.dot(&player_to_goal).acos()
    }

    fn space_to_dribble(&self, ctx: &StateProcessingContext) -> bool {
        let dribble_distance = 10.0; // Adjust based on your game's scale
        let players = ctx.players();
        let opponents = players.opponents();

        !opponents.exists(dribble_distance)
    }

    fn can_shoot(&self, ctx: &StateProcessingContext) -> bool {
        let shot_distance = 25.0; // Adjust based on your game's scale

        // Check if the player is within shooting distance and has a clear shot
        ctx.ball().distance_to_opponent_goal() < shot_distance && self.has_clear_shot(ctx)
    }

    fn has_clear_shot(&self, ctx: &StateProcessingContext) -> bool {
        let opponent_goal_position = match ctx.player.side {
            // swap for opponents
            Some(PlayerSide::Left) => ctx.context.goal_positions.left,
            Some(PlayerSide::Right) => ctx.context.goal_positions.right,
            _ => Vector3::new(0.0, 0.0, 0.0),
        };

        let players = ctx.players();
        let opponents = players.opponents();
        let mut opponents_all = opponents.all();

        // Check if there are no opponents blocking the shot
        opponents_all.all(|opponent| {
            let opponent_to_goal = (opponent_goal_position - opponent.position).normalize();
            let player_to_goal = (opponent_goal_position - ctx.player.position).normalize();
            opponent_to_goal.dot(&player_to_goal) < 0.9
        })
    }
}
