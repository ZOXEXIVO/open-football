use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{
    ConditionContext, PlayerDistanceFromStartPosition, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior, MATCH_TIME_MS,
};
use nalgebra::Vector3;

const CLOSE_TO_START_DISTANCE: f32 = 10.0;
const BALL_INTERCEPTION_DISTANCE: f32 = 30.0;

#[derive(Default)]
pub struct DefenderTrackingBackState {}

impl StateProcessingHandler for DefenderTrackingBackState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the defender has reached their starting position
        if ctx.player().distance_from_start_position() < CLOSE_TO_START_DISTANCE {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        // Check if the ball is close and moving towards the player
        if ctx.ball().distance() < BALL_INTERCEPTION_DISTANCE && ctx.ball().is_towards_player() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting,
            ));
        }

        // If the team is losing and there's little time left, consider a more aggressive stance
        if ctx.team().is_loosing() && ctx.context.total_match_time > (MATCH_TIME_MS - 300) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

        // If the player is tired, switch to a less demanding state
        if ctx.player().is_tired() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ));
        }

        // If the ball is on the team's own side, prioritize defensive positioning
        if ctx.ball().on_own_side() {
            match ctx.player().position_to_distance() {
                PlayerDistanceFromStartPosition::Big => None, // Continue tracking back
                PlayerDistanceFromStartPosition::Medium => Some(
                    StateChangeResult::with_defender_state(DefenderState::HoldingLine),
                ),
                PlayerDistanceFromStartPosition::Small => Some(
                    StateChangeResult::with_defender_state(DefenderState::Standing),
                ),
            }
        } else {
            None // Continue tracking back if the ball is on the opponent's side
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let start_position = ctx.player.start_position;
        let distance_from_start = ctx.player().distance_from_start_position();

        // Calculate urgency based on game situation
        let urgency = if ctx.ball().on_own_side() {
            // Ball on own side - more urgent to get back
            1.5
        } else if ctx.team().is_loosing() && ctx.context.total_match_time > (MATCH_TIME_MS - 300) {
            // Losing late in game - less urgent to defend
            0.8
        } else {
            1.0
        };

        // Use Arrive behavior with slowing distance based on how far we are
        let slowing_distance = if distance_from_start > 50.0 {
            15.0 // Far away - longer slowing distance
        } else {
            10.0 // Close - shorter slowing distance
        };

        Some(
            SteeringBehavior::Arrive {
                target: start_position,
                slowing_distance,
            }
            .calculate(ctx.player)
            .velocity
            * urgency,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}
