use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;

const MAX_SHOOTING_DISTANCE: f32 = 30.0; // Maximum distance to attempt a shot
const MIN_SHOOTING_DISTANCE: f32 = 16.5; // Minimum distance to attempt a shot (e.g., edge of penalty area)
const PRESS_DISTANCE: f32 = 20.0; // Distance within which to press opponents

#[derive(Default)]
pub struct ForwardStandingState {}

impl StateProcessingHandler for ForwardStandingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the forward still has the ball
        if ctx.player.has_ball(ctx) {
            // Decide next action based on game context
            if self.is_in_shooting_range(ctx) {
                // Transition to Shooting state
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
            // If the forward doesn't have the ball, decide to move or press
            if self.should_press(ctx) {
                // Transition to Pressing state
                Some(StateChangeResult::with_forward_state(
                    ForwardState::Pressing,
                ))
            } else {
                Some(StateChangeResult::with_forward_state(ForwardState::Running))
                // Transition to Positioning state
                //Some(StateChangeResult::with_forward_state(ForwardState::Positioning))
            }
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic for advanced decision-making if necessary
        // For example, adjust positioning based on opponent movement
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // Handle additional conditions or triggers if necessary
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
        ctx.ball().distance() < PRESS_DISTANCE && ctx.player.has_ball(ctx)
    }

    /// Calculates the distance from the forward to the opponent's goal.
    fn distance_to_opponent_goal(&self, ctx: &StateProcessingContext) -> f32 {
        ctx.ball().distance_to_opponent_goal()
    }
}
