use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

const PRESS_DISTANCE: f32 = 20.0; // Distance within which to press opponents

#[derive(Default, Clone)]
pub struct ForwardStandingState {}

impl StateProcessingHandler for ForwardStandingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the forward still has the ball
        if ctx.player.has_ball(ctx) {
            let distance_to_goal = ctx.ball().distance_to_opponent_goal();

            // PRIORITY: Close to goal — shoot immediately (no cooldown)
            // Forwards should ALWAYS shoot when in range rather than pass
            if distance_to_goal <= 60.0 && ctx.player().shooting().in_shooting_range() {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ));
            }

            // Cooldown for medium/long range shots to prevent rapid-fire spam
            const SHOOTING_COOLDOWN: u64 = 20;

            if ctx.player().should_attempt_shot() && ctx.in_state_time > SHOOTING_COOLDOWN {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ));
            }

            if let Some(_) = self.find_best_teammate_to_pass(ctx) {
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }

            // If unable to shoot or pass, decide to dribble or hold position
            if self.should_dribble(ctx) {
                Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ))
            } else {
                // Transition to Running to approach goal
                Some(StateChangeResult::with_forward_state(
                    ForwardState::Running,
                ))
            }
        } else {
            // If notified by ball system, always respond (only 1 per team gets notified)
            if ctx.ball().is_player_notified() && !ctx.ball().is_owned() {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::TakeBall,
                ));
            }

            // Emergency: ball nearby and unowned - only chase if nearest teammate
            if ctx.ball().distance() < 50.0 && !ctx.ball().is_owned() {
                let ball_velocity = ctx.tick_context.positions.ball.velocity.norm();
                if ball_velocity < 3.0 {
                    let ball_pos = ctx.tick_context.positions.ball.position;
                    let my_dist = ctx.ball().distance();
                    let closer_teammate = ctx.players().teammates().all()
                        .any(|t| t.id != ctx.player.id && (t.position - ball_pos).magnitude() < my_dist - 5.0);

                    if !closer_teammate {
                        return Some(StateChangeResult::with_forward_state(
                            ForwardState::TakeBall,
                        ));
                    }
                }
            }

            // Minimum time in standing state to prevent rapid state oscillation
            if ctx.in_state_time < 10 {
                return None;
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

        // Standing = completely still. Separation is handled by transitioning
        // to Running state, not by applying forces while stationary.
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Standing is recovery - minimal movement
        ForwardCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}

impl ForwardStandingState {
    /// Finds the best teammate to pass to based on proximity and position.
    /// Filters out close recent passers to prevent ping-pong passing.
    fn find_best_teammate_to_pass<'a>(
        &'a self,
        ctx: &'a StateProcessingContext<'a>,
    ) -> Option<u32> {
        // Look for teammates within range, but skip anyone who just passed to us
        // and is still very close (prevents ping-pong)
        for (teammate_id, distance) in ctx.players().teammates().nearby_ids(100.0) {
            let recency = ctx.ball().passer_recency_penalty(teammate_id);
            // If most recent or second-most-recent passer AND within 40 units, skip
            if recency <= 0.3 && distance < 40.0 {
                continue;
            }
            return Some(teammate_id);
        }

        None
    }

    /// Decides whether the forward should dribble based on game context.
    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        // Only dribble when there are opponents to beat nearby
        let nearby_opponents = ctx.players().opponents().exists(15.0);

        // No opponents — just run, don't dribble
        if !nearby_opponents {
            return false;
        }

        // Dribble to beat nearby defenders if skilled enough
        let dribbling_skill = ctx.player.skills.technical.dribbling / 20.0;
        dribbling_skill > 0.5
    }

    /// Decides whether the forward should press the opponent.
    fn should_press(&self, ctx: &StateProcessingContext) -> bool {
        // Only press if opponent has the ball AND is close AND we're best positioned
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let dist = opponent.distance(ctx);
            // Very close — anyone reacts
            if dist < 10.0 {
                return true;
            }
            // Otherwise only the best chaser presses
            dist < PRESS_DISTANCE && ctx.team().is_best_player_to_chase_ball()
        } else {
            false
        }
    }

}
