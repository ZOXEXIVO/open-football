use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::strategies::processor::StateChangeResult;
use crate::r#match::{
    ConditionContext, StateProcessingContext, StateProcessingHandler, SteeringBehavior,
    VectorExtensions,
};
use crate::IntegerUtils;
use nalgebra::Vector3;
use std::sync::LazyLock;

static _GOALKEEPER_ATTENTIVE_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_attentive_data.json")));

#[derive(Default)]
pub struct GoalkeeperAttentiveState {}

impl StateProcessingHandler for GoalkeeperAttentiveState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Distributing,
            ));
        }

        if ctx.ball().on_own_side() {
            if self.should_come_out(ctx) && ctx.ball().distance() < 200.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ComingOut,
                ));
            } else if ctx.ball().is_towards_player_with_angle(0.8) && ctx.ball().distance() < 250.0
            {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
        } else {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Walking,
            ));
        }

        // Check if the goalkeeper is out of position
        if self.is_out_of_position(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // Check if there's an immediate threat
        if self.is_under_threat(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::UnderPressure,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Wander {
                target: ctx.player.start_position,
                radius: IntegerUtils::random(5, 100) as f32,
                jitter: IntegerUtils::random(0, 2) as f32,
                distance: IntegerUtils::random(10, 100) as f32,
                angle: IntegerUtils::random(0, 360) as f32,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl GoalkeeperAttentiveState {
    fn is_out_of_position(&self, ctx: &StateProcessingContext) -> bool {
        let optimal_position = self.calculate_optimal_position(ctx);
        ctx.player.position.distance_to(&optimal_position) > 100.0 // Reduced threshold for more frequent adjustments
    }

    fn is_under_threat(&self, ctx: &StateProcessingContext) -> bool {
        let players = ctx.players();
        let opponents = players.opponents();
        let mut opponents_with_ball = opponents.with_ball();

        if let Some(opponent) = opponents_with_ball.next() {
            let distance_to_opponent = opponent.position.distance_to(&ctx.player.position);
            distance_to_opponent < 30.0 // Adjust this value based on your game's scale
        } else {
            false
        }
    }

    fn should_come_out(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let goalkeeper_skills = &ctx.player.skills;

        // Decision based on ball distance and goalkeeper's skills
        ball_distance < 50.0
            && goalkeeper_skills.mental.decisions > 10.0
            && goalkeeper_skills.physical.acceleration > 10.0
    }

    fn calculate_optimal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_position = ctx.ball().direction_to_own_goal();
        let ball_position = ctx.tick_context.positions.ball.position;

        // Calculate the angle between the ball and the goal
        let angle_to_ball = (ball_position - goal_position).normalize();

        // Determine the optimal distance based on ball position and goalkeeper skills
        let optimal_distance = self.calculate_optimal_distance(ctx);

        // Calculate the new position, taking into account the angle to the ball
        let new_position = goal_position + angle_to_ball * optimal_distance;

        // Limit the goalkeeper's movement to stay within the penalty area
        self.limit_to_penalty_area(new_position, ctx)
    }

    fn calculate_optimal_distance(&self, ctx: &StateProcessingContext) -> f32 {
        let ball_distance = ctx.ball().distance();
        let goalkeeper_skills = &ctx.player.skills;

        // Base distance
        let mut optimal_distance = 2.0;

        // Adjust distance based on ball position
        if ball_distance < 30.0 {
            optimal_distance += (30.0 - ball_distance) * 0.1;
        }

        // Adjust distance based on goalkeeper's positioning skill
        optimal_distance += goalkeeper_skills.mental.positioning * 0.05;

        // Limit the distance to a reasonable range
        optimal_distance.clamp(1.0, 5.0)
    }

    fn limit_to_penalty_area(
        &self,
        position: Vector3<f32>,
        ctx: &StateProcessingContext,
    ) -> Vector3<f32> {
        // Assume penalty area dimensions (adjust as needed)
        let penalty_area_width = 40.0;
        let penalty_area_depth = 16.5;

        let goal_position = ctx.ball().direction_to_own_goal();

        let mut limited_position = position;

        // Limit x-coordinate
        limited_position.x = limited_position.x.clamp(
            goal_position.x - penalty_area_width / 2.0,
            goal_position.x + penalty_area_width / 2.0,
        );

        // Limit z-coordinate (assuming z is depth)
        limited_position.z = limited_position
            .z
            .clamp(goal_position.z, goal_position.z + penalty_area_depth);

        limited_position
    }
}
