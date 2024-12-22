use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PlayerEvent, ShootingEventModel};
use crate::r#match::player::PlayerSide;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

static MIDFIELDER_SHOOTING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_shooting_data.json")));

#[derive(Default)]
pub struct MidfielderShootingState {}

impl StateProcessingHandler for MidfielderShootingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Pressing
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        // Create an event to change the ball's velocity
        let mut state_change = StateChangeResult::with_midfielder_state(MidfielderState::Standing);

        state_change
            .events
            .add_player_event(PlayerEvent::Shoot(ShootingEventModel::build()
                .with_player_id(ctx.player.id)
                .with_target(ctx.player().opponent_goal_position())
                .with_force(ctx.player().shoot_goal_power())
                .build()));

        // Transition to the next appropriate state (e.g., Standing)
        Some(state_change)
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // No slow processing needed
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Midfielder remains stationary while taking the shot
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderShootingState {
    /// Calculates the shot direction towards the opponent's goal.
    fn calculate_shot_direction(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        ctx.ball().direction_to_opponent_goal()
    }

    /// Calculates the shot power based on player attributes and distance to goal.
    fn calculate_shot_power(&self, ctx: &StateProcessingContext) -> f32 {
        // Get player's shooting power attribute
        let shooting_power = ctx.player.skills.technical.tackling as f32 / 100.0;

        // Adjust power based on distance to goal
        let distance_to_goal =
            (ctx.player.position - self.get_opponent_goal_position(ctx)).magnitude();
        let max_distance = 30.0; // Maximum effective shooting distance
        let distance_factor = (max_distance - distance_to_goal) / max_distance;

        let base_power = MAX_SHOT_POWER * distance_factor;

        // Final shot power is a combination of base power and player's shooting power attribute
        base_power * (0.8 + 0.2 * shooting_power)
    }

    /// Gets the position of the opponent's goal.
    fn get_opponent_goal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_length = ctx.context.field_size.width as f32;
        let field_width = ctx.context.field_size.width as f32;

        if ctx.player.side == Some(PlayerSide::Left) {
            // Attacking towards the right (positive x)
            Vector3::new(field_length, field_width / 2.0, 0.0)
        } else {
            // Attacking towards the left (negative x)
            Vector3::new(0.0, field_width / 2.0, 0.0)
        }
    }
}

// Constants used in shot calculations
const MAX_SHOT_POWER: f32 = 30.0; // Maximum shot power
const STAMINA_COST_SHOT: f32 = 5.0; // Stamina cost of taking a shot
