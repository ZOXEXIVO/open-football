use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const TRACKING_DISTANCE_THRESHOLD: f32 = 30.0; // Maximum distance to track the runner
const STAMINA_THRESHOLD: f32 = 50.0; // Minimum stamina required to continue tracking
const BALL_INTERCEPTION_DISTANCE: f32 = 15.0; // Distance to switch to intercepting ball

#[derive(Default)]
pub struct MidfielderTrackingRunnerState {}

impl StateProcessingHandler for MidfielderTrackingRunnerState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check for ball interception opportunities first
        let ball_distance = ctx.ball().distance();
        if ball_distance < BALL_INTERCEPTION_DISTANCE && ctx.ball().is_towards_player() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Intercepting,
            ));
        }

        // Check if opponent with ball is nearby - switch to tackling
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let distance = opponent_with_ball.distance(ctx);
            if distance < 5.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            } else if distance < 20.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
        }

        let nearest_forward = ctx.players().opponents().forwards().min_by(|a, b| {
            let dist_a = (a.position - ctx.player.position).magnitude();
            let dist_b = (b.position - ctx.player.position).magnitude();
            dist_a.partial_cmp(&dist_b).unwrap()
        });

        if let Some(runner) = nearest_forward {
            // Check if the midfielder has enough stamina to continue tracking
            if ctx.player.player_attributes.condition_percentage() < STAMINA_THRESHOLD as u32 {
                // If stamina is low, transition to the Defending state
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            // Check if the runner is within tracking distance
            let distance_to_runner = (ctx.player.position - runner.position).magnitude();
            if distance_to_runner > TRACKING_DISTANCE_THRESHOLD {
                // If the runner is too far, transition to the Defending state
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            // Continue tracking the runner
            None
        } else {
            // If no opponent runner is found, transition to the Defending state
            Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let nearest_forward = ctx.players().opponents().forwards().min_by(|a, b| {
            let dist_a = (a.position - ctx.player.position).magnitude();
            let dist_b = (b.position - ctx.player.position).magnitude();
            dist_a.partial_cmp(&dist_b).unwrap()
        });

        // Move towards the opponent runner
        if let Some(runner) = nearest_forward {
            let steering = SteeringBehavior::Pursuit {
                target: runner.position,
                target_velocity: Vector3::zeros(), // Opponent velocity not available in lite struct
            }
            .calculate(ctx.player);

            Some(steering.velocity)
        } else {
            // If no runner is found, stay in the current position
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tracking runner is moderate intensity - sustained tracking
        MidfielderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}

impl MidfielderTrackingRunnerState {}
