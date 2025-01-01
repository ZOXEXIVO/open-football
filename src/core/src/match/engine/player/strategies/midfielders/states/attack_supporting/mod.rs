use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

static _MIDFIELDER_ATTACK_SUPPORTING_STATE_NETWORK: LazyLock<NeuralNetwork> = LazyLock::new(|| {
    DefaultNeuralNetworkLoader::load(include_str!("nn_attack_supporting_data.json"))
});

#[derive(Default)]
pub struct MidfielderAttackSupportingState {}

impl StateProcessingHandler for MidfielderAttackSupportingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        if ctx.ball().distance() < 15.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Tackling,
            ));
        }

        if !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        if ctx.ball().distance() > 200.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // 2. Check if the attack has broken down (e.g., ball lost)
        if self.attack_broken_down(ctx) {
            // Transition back to defensive state
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        // 3. If midfielder is in a good shooting position, consider shooting
        if self.is_in_shooting_position(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::DistanceShooting,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let target_position = self.calculate_support_position(ctx);

        Some(
            SteeringBehavior::Seek {
                target: target_position,
            }
            .calculate(&ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderAttackSupportingState {
    fn attack_broken_down(&self, ctx: &StateProcessingContext) -> bool {
        !ctx.team().is_control_ball()
    }

    /// Checks if the midfielder is in a good position to attempt a shot.
    fn is_in_shooting_position(&self, ctx: &StateProcessingContext) -> bool {
        let shooting_range = 25.0; // Distance from goal to consider shooting
        let player_position = ctx.player.position;
        let goal_position = self.get_opponent_goal_position(ctx);

        let distance_to_goal = (player_position - goal_position).magnitude();

        distance_to_goal <= shooting_range
    }

    /// Calculates the position the midfielder should move to in order to support the attack.
    fn calculate_support_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // For simplicity, position yourself slightly behind the forwards
        let forwards_positions: Vec<Vector3<f32>> = ctx
            .players()
            .teammates()
            .forwards()
            .map(|p| p.position)
            .collect();

        if forwards_positions.is_empty() {
            // No forwards found, move towards the penalty area
            self.get_penalty_area_position(ctx)
        } else {
            // Calculate average position of forwards
            let average_forward_position = forwards_positions
                .iter()
                .fold(Vector3::zeros(), |acc, pos| acc + *pos)
                / forwards_positions.len() as f32;

            // Position slightly behind the forwards
            let offset = if ctx.player.side.unwrap() == PlayerSide::Left {
                Vector3::new(-10.0, 0.0, 0.0)
            } else {
                Vector3::new(10.0, 0.0, 0.0)
            };

            average_forward_position + offset
        }
    }

    /// Gets the position of the opponent's goal.
    fn get_opponent_goal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_length = ctx.context.field_size.width as f32;
        let field_width = ctx.context.field_size.width as f32;

        if ctx.player.side.unwrap() == PlayerSide::Left {
            // Home team attacking towards the right (positive x)
            Vector3::new(field_length, field_width / 2.0, 0.0)
        } else {
            // Away team attacking towards the left (negative x)
            Vector3::new(0.0, field_width / 2.0, 0.0)
        }
    }

    /// Gets a position inside the opponent's penalty area.
    fn get_penalty_area_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_length = ctx.context.field_size.width as f32;
        let field_width = ctx.context.field_size.width as f32;
        let penalty_area_depth = 16.5; // Standard penalty area depth in meters

        let x = if ctx.player.side.unwrap() == PlayerSide::Left {
            field_length - penalty_area_depth / 2.0
        } else {
            penalty_area_depth / 2.0
        };

        let y = field_width / 2.0;

        Vector3::new(x, y, 0.0)
    }
}
