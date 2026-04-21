use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use crate::IntegerUtils;
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct ForwardWalkingState {}

impl StateProcessingHandler for ForwardWalkingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Offside discipline — a forward walking/jogging about with the
        // opponents in possession must not stray beyond the defensive
        // line, or every clearance lands offside. Drop back to Returning.
        if ctx.player().defensive().is_stranded_offside() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        if ctx.ball().is_owned() {
            if ctx.team().is_control_ball() {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::CreatingSpace
                ));
            } else {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Running
                ));
            }
        }

        // Loose-ball claim lives in the dispatcher.

        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::TakeBall,
            ));
        }

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
