use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;
use std::sync::LazyLock;

static _MIDFIELDER_HOLDING_POSSESSION_STATE_NETWORK: LazyLock<NeuralNetwork> = LazyLock::new(|| {
    DefaultNeuralNetworkLoader::load(include_str!("nn_holding_possession_data.json"))
});

const MAX_SHOOTING_DISTANCE: f32 = 300.0; // Maximum distance to attempt a shot
const MIN_SHOOTING_DISTANCE: f32 = 20.0; // Minimum distance to attempt a shot (e.g., edge of penalty area)

#[derive(Default)]
pub struct MidfielderHoldingPossessionState {}

impl StateProcessingHandler for MidfielderHoldingPossessionState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder has the ball
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        if self.is_in_shooting_range(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Shooting,
            ));
        }

        // Check if the midfielder is being pressured by opponents
        if self.is_under_pressure(ctx) {
            // If under pressure, decide whether to dribble or pass based on the situation
            return if self.has_space_to_dribble(ctx) {
                // If there is space to dribble, transition to the dribbling state
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Dribbling,
                ))
            } else {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing
                ))
            }
        }

        let players= ctx.players();
        let teammates = players.teammates();

        if let Some(_) = teammates
            .nearby(300.0)
            .filter(|teammate| self.is_teammate_open(ctx, teammate)).next() {
            // If there is an open teammate, transition to the passing state
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Passing
            ));
        }

        // If none of the above conditions are met, continue holding possession
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.ball().direction_to_opponent_goal(),
                slowing_distance: 30.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderHoldingPossessionState {
    pub fn is_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(50.0)
    }

    fn is_teammate_open(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // Check if a teammate is open to receive a pass
        let is_in_passing_range = (teammate.position - ctx.player.position).magnitude() <= 30.0;
        let has_clear_passing_lane = self.has_clear_passing_lane(ctx, teammate);

        is_in_passing_range && has_clear_passing_lane
    }

    fn has_space_to_dribble(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(10.0)
    }

    fn has_clear_passing_lane(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // Check if there is a clear passing lane to a teammate without any obstructing opponents
        let player_position = ctx.player.position;
        let teammate_position = teammate.position;
        let passing_direction = (teammate_position - player_position).normalize();

        let ray_cast_result = ctx.tick_context.space.cast_ray(
            player_position,
            passing_direction,
            (teammate_position - player_position).magnitude(),
            false,
        );

        ray_cast_result.is_none() // No collisions with opponents
    }

    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        distance_to_goal <= MAX_SHOOTING_DISTANCE && distance_to_goal >= MIN_SHOOTING_DISTANCE
    }
}
