use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

const MAX_SHOOTING_DISTANCE: f32 = 250.0; // Maximum distance to attempt a shot
const MIN_SHOOTING_DISTANCE: f32 = 1.0; // Minimum distance to attempt a shot (very close to goal)
const PRESS_DISTANCE: f32 = 20.0; // Distance within which to press opponents

#[derive(Default)]
pub struct ForwardStandingState {}

impl StateProcessingHandler for ForwardStandingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the forward still has the ball
        if ctx.player.has_ball(ctx) {
            // CRITICAL: Add cooldown before allowing another shot to prevent rapid-fire goal spam
            // Must wait at least 40 ticks (~0.7 seconds) after entering Standing before shooting again
            const SHOOTING_COOLDOWN: u64 = 10;

            // Decide next action based on game context
            if self.is_in_shooting_range(ctx) && ctx.in_state_time > SHOOTING_COOLDOWN {
                // Transition to Shooting state (only after cooldown)
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ));
            }

            if let Some(_) = self.find_best_teammate_to_pass(ctx) {
                // Transition to Passing state
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }

            // If unable to shoot or pass, decide to dribble or hold position
            if self.should_dribble(ctx) {
                Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ))
            } else {
                None
                // Hold possession
                //return Some(StateChangeResult::with_forward_state(ForwardState::HoldingPossession));
            }
        } else {
            // Emergency: if ball is nearby, slow-moving, and unowned, go for it immediately
            // OR if player is notified to take the ball (no distance limit when notified)
            let is_nearby = ctx.ball().distance() < 50.0;
            let is_notified = ctx.ball().is_player_notified();

            if (is_nearby || is_notified) && !ctx.ball().is_owned() {
                let ball_velocity = ctx.tick_context.positions.ball.velocity.norm();
                if ball_velocity < 3.0 { // Increased from 1.0 to catch slow rolling balls
                    // Ball is stopped or slow-moving - take it directly
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::TakeBall,
                    ));
                }
            }

            // If the forward doesn't have the ball, decide to move or press
            if self.should_press(ctx) {
                // Transition to Pressing state
                Some(StateChangeResult::with_forward_state(
                    ForwardState::Pressing,
                ))
            } else if ctx.ball().distance() > 200.0 && !ctx.team().is_control_ball() {
                // Ball is far and team doesn't have it - walk to conserve energy
                Some(StateChangeResult::with_forward_state(ForwardState::Walking))
            } else {
                Some(StateChangeResult::with_forward_state(ForwardState::Running))
            }
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic for advanced decision-making if necessary
        // For example, adjust positioning based on opponent movement
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Check if player should follow waypoints even when standing
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();

            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: 3.0,
                    }
                    .calculate(ctx.player)
                    .velocity * 0.5, // Slower speed when standing
                );
            }
        }

        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Standing is recovery - minimal movement
        ForwardCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}

impl ForwardStandingState {
    /// Determines if the forward is within shooting range of the opponent's goal.
    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = self.distance_to_opponent_goal(ctx);
        distance_to_goal <= MAX_SHOOTING_DISTANCE && distance_to_goal >= MIN_SHOOTING_DISTANCE
    }

    /// Finds the best teammate to pass to based on proximity and position.
    fn find_best_teammate_to_pass<'a>(
        &'a self,
        ctx: &'a StateProcessingContext<'a>,
    ) -> Option<u32> {
        if let Some((teammate_id, _)) = ctx.players().teammates().nearby_ids(100.0).next() {
            return Some(teammate_id)
        }

        None
    }

    /// Decides whether the forward should dribble based on game context.
    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        // Example logic: dribble if no immediate threat and space is available
        let safe_distance = 10.0;

        !ctx.players().opponents().exists(safe_distance)
    }

    /// Decides whether the forward should press the opponent.
    fn should_press(&self, ctx: &StateProcessingContext) -> bool {
        // Only press if opponent has the ball AND is close
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            opponent.distance(ctx) < PRESS_DISTANCE
        } else {
            false
        }
    }

    /// Calculates the distance from the forward to the opponent's goal.
    fn distance_to_opponent_goal(&self, ctx: &StateProcessingContext) -> f32 {
        ctx.ball().distance_to_opponent_goal()
    }
}
