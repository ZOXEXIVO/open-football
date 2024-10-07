use std::sync::LazyLock;
use nalgebra::Vector3;
use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::IntegerUtils;
use crate::r#match::{ConditionContext, MatchPlayer, PlayerDistanceFromStartPosition, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior, VectorExtensions};
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::player::events::PlayerUpdateEvent;

static DEFENDER_WALKING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_walking_data.json")));

const INTERCEPTION_DISTANCE: f32 = 150.0;
const MARKING_DISTANCE: f32 = 50.0;
const PRESSING_DISTANCE: f32 = 30.0;

#[derive(Default)]
pub struct DefenderWalkingState {}

impl StateProcessingHandler for DefenderWalkingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let mut result = StateChangeResult::new();

        // Check if the ball is moving towards the player and is close
        if ctx.ball().is_towards_player_with_angle(0.8) && ctx.ball().distance() < INTERCEPTION_DISTANCE {
            return Some(StateChangeResult::with_defender_state(DefenderState::Intercepting));
        }

        // Check if the defender needs to return to their position
        if ctx.player().position_to_distance() != PlayerDistanceFromStartPosition::Small {
            return Some(StateChangeResult::with_defender_state(DefenderState::Returning));
        }

        // Check if there's an opponent to mark
        if let Some(opponent_to_mark) = self.find_opponent_to_mark(ctx) {
            if ctx.player.position.distance_to(&opponent_to_mark.position) < MARKING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(DefenderState::Marking));
            }
        }

        // Check if there's an opponent to press
        if let Some(opponent_to_press) = self.find_opponent_to_press(ctx) {
            if ctx.player.position.distance_to(&opponent_to_press.position) < PRESSING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(DefenderState::Pressing));
            }
        }

        // Adjust position if needed
        let optimal_position = self.calculate_optimal_position(ctx);
        if ctx.player.position.distance_to(&optimal_position) > 2.0 {
            result.events.add(PlayerUpdateEvent::MovePlayer(ctx.player.id, optimal_position));
            return Some(result);
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // 1. If this is the first tick in the state, initialize wander behavior
        if ctx.in_state_time % 100 == 0 {
            return Some(SteeringBehavior::Wander {
                target: ctx.player.start_position,
                radius: IntegerUtils::random(5, 15) as f32,
                jitter: IntegerUtils::random(1, 5) as f32,
                distance: IntegerUtils::random(10, 20) as f32,
                angle: IntegerUtils::random(0, 360) as f32,
            }.calculate(ctx.player).velocity);
        }

        // Fallback to moving towards optimal position
        let optimal_position = self.calculate_optimal_position(ctx);
        let direction = (optimal_position - ctx.player.position).normalize();
        let speed = ctx.player.skills.walking_speed().norm();
        Some(direction * speed)
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}

impl DefenderWalkingState {
    fn find_opponent_to_mark<'a>(&self, ctx: &'a StateProcessingContext<'a>) -> Option<&'a MatchPlayer> {
        ctx.tick_context
            .object_positions
            .player_distances
            .find_closest_opponent(ctx.player)
            .and_then(|(opponent_id, _)| {
                ctx.context.players.get(opponent_id)
                    .filter(|opponent| !opponent.has_ball)
            })
    }

    fn find_opponent_to_press<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<&'a MatchPlayer> {
        ctx.tick_context
            .object_positions
            .player_distances
            .find_closest_opponent(ctx.player)
            .and_then(|(opponent_id, _)| {
                ctx.context.players.get(opponent_id)
                    .filter(|opponent| opponent.has_ball)
            })
    }

    fn calculate_optimal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // This is a simplified calculation. You might want to make it more sophisticated
        // based on team formation, tactics, and the current game situation.
        let team_center = self.calculate_team_center(ctx);
        let ball_position = ctx.tick_context.object_positions.ball_position;

        // Position between team center and ball, slightly closer to team center
        (team_center * 0.7 + ball_position * 0.3).into()
    }

    fn calculate_team_center(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let teammates = ctx.context.players.get_by_team(ctx.player.team_id);
        let sum: Vector3<f32> = teammates.iter().map(|p| p.position).sum();
        sum / teammates.len() as f32
    }
}