use crate::r#match::player::strategies::common::team::WideChannel;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// Fraction of full speed a walking player moves at. This is the
/// recovery gear — the state exists so a forward can reorganise without
/// burning condition, so he covers ground but does not sprint.
const WALK_PACE: f32 = 0.45;

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
                if WideChannel::still_mine(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::HoldingWidth,
                    ));
                }
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::CreatingSpace,
                ));
            } else {
                return Some(StateChangeResult::with_forward_state(ForwardState::Running));
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

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();

            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        crowd_offset: ctx.player().separation_offset(),
                    }
                    .calculate(ctx.player)
                    .velocity,
                );
            }
        }

        // Walking is the low-energy off-ball state, and it used to be a
        // literal `Wander` around the kickoff dot: a random walk, on a
        // point fixed before the match started. That is the single line
        // most responsible for "the forward is strolling about not playing
        // football" — a striker whose team had the ball 60 m away spent
        // the possession describing small circles on the spot he was
        // drawn on.
        //
        // A forward off the ball is doing the opposite of nothing: he is
        // getting into the shape his team needs him in, at a jog. Steer at
        // the anchor and stop when he is there.
        Some(
            SteeringBehavior::Arrive {
                target: ctx.team().my_anchor(),
                slowing_distance: 30.0,
            }
            .calculate(ctx.player)
            .velocity
                * WALK_PACE,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Walking is low intensity - minimal fatigue
        ForwardCondition::with_velocity(ActivityIntensity::Low).process(ctx);
    }
}
