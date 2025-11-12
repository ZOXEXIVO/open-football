use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const PRESSING_DISTANCE_THRESHOLD: f32 = 50.0; // Adjust as needed

#[derive(Default)]
pub struct MidfielderStandingState {}

impl StateProcessingHandler for MidfielderStandingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Add timeout to prevent getting stuck in standing state
        if ctx.in_state_time > 30 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        if ctx.player.has_ball(ctx) {
            // Decide whether to hold possession or distribute the ball
            return if self.should_hold_possession(ctx) {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::HoldingPossession,
                ))
            } else {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Distributing,
                ))
            };
        }
        else {
            // Emergency: if ball is nearby, stopped, and unowned, go for it immediately
            if ctx.ball().should_take_ball_immediately() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::TakeBall,
                ));
            }

            if ctx.team().is_control_ball() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Running,
                ));
            }
            else {
                // Only press/tackle if an OPPONENT has the ball
                if let Some(_opponent) = ctx.players().opponents().with_ball().next() {
                    if ctx.ball().distance() < PRESSING_DISTANCE_THRESHOLD {
                        // Transition to Pressing state to try and win the ball
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::Pressing,
                        ));
                    }

                    if ctx.ball().distance() < 100.0 {
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::Tackling,
                        ));
                    }
                }

                // Only intercept if ball is loose (not owned by anyone)
                if !ctx.ball().is_owned()
                    && ctx.ball().distance() < 250.0
                    && ctx.ball().is_towards_player_with_angle(0.8) {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Intercepting,
                    ));
                }
            }
        }

        // Only press if opponent is nearby AND has the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            if opponent.distance(ctx) < PRESSING_DISTANCE_THRESHOLD {
                // Transition to Pressing state to apply pressure
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
        }

        // 4. Check if a teammate is making a run and needs support
        if self.should_support_attack(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::AttackSupporting,
            ));
        }

        // If nothing else is happening, start moving again after a brief pause
        if ctx.in_state_time > 15 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
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
        MidfielderCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}

impl MidfielderStandingState {
    /// Checks if the midfielder should hold possession based on game context.
    fn should_hold_possession(&self, ctx: &StateProcessingContext) -> bool {
        // For simplicity, let's assume the midfielder holds possession if there are no immediate passing options
        !self.has_passing_options(ctx)
    }

    /// Determines if the midfielder has passing options.
    fn has_passing_options(&self, ctx: &StateProcessingContext) -> bool {
        const PASSING_DISTANCE_THRESHOLD: f32 = 30.0;
        ctx.players().teammates().exists(PASSING_DISTANCE_THRESHOLD)
    }

    /// Checks if an opponent player is nearby within the pressing threshold.
    fn is_opponent_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players()
            .opponents()
            .exists(PRESSING_DISTANCE_THRESHOLD)
    }

    /// Determines if the midfielder should support an attacking play.
    fn should_support_attack(&self, ctx: &StateProcessingContext) -> bool {
        // For simplicity, assume the midfielder supports the attack if the ball is in the attacking third
        let field_length = ctx.context.field_size.width as f32;
        let attacking_third_start = if ctx.player.side == Some(PlayerSide::Left) {
            field_length * (2.0 / 3.0)
        } else {
            field_length / 3.0
        };

        let ball_position_x = ctx.tick_context.positions.ball.position.x;

        if ctx.player.side == Some(PlayerSide::Left) {
            ball_position_x > attacking_third_start
        } else {
            ball_position_x < attacking_third_start
        }
    }
}
