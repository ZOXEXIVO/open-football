use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use crate::IntegerUtils;
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardWalkingState {}

impl StateProcessingHandler for ForwardWalkingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Emergency: if ball is nearby, stopped, and unowned, go for it immediately
        if ctx.ball().distance() < 50.0 && !ctx.ball().is_owned() {
            let ball_velocity = ctx.tick_context.positions.ball.velocity.norm();
            if ball_velocity < 1.0 {
                // Ball is stopped or nearly stopped - take it directly
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::TakeBall,
                ));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>>  {
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();

            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: IntegerUtils::random(1, 10) as f32,
                    }
                    .calculate(ctx.player)
                    .velocity,
                );
            }
        }

        Some(
            SteeringBehavior::Wander {
                target: ctx.player.start_position,
                radius: IntegerUtils::random(5, 15) as f32,
                jitter: IntegerUtils::random(1, 5) as f32,
                distance: IntegerUtils::random(10, 20) as f32,
                angle: IntegerUtils::random(0, 360) as f32,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Walking is low intensity - minimal fatigue
        ForwardCondition::with_velocity(ActivityIntensity::Low).process(ctx);
    }
}
