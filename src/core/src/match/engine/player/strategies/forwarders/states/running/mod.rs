use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;
use crate::IntegerUtils;

const MAX_SHOOTING_DISTANCE: f32 = 300.0; // Maximum distance to attempt a shot
const MIN_SHOOTING_DISTANCE: f32 = 20.0; // Minimum distance to attempt a shot (e.g., edge of penalty area)

#[derive(Default)]
pub struct ForwardRunningState {}

const PRESSING_DISTANCE_THRESHOLD: f32 = 50.0;
const SHOOTING_DISTANCE_THRESHOLD: f32 = 250.0;
const PASSING_DISTANCE_THRESHOLD: f32 = 400.0;
const ASSISTING_DISTANCE_THRESHOLD: f32 = 200.0;

impl StateProcessingHandler for ForwardRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        if ctx.player.has_ball(ctx) {
            if self.has_clear_shot(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ));
            }

            if self.is_in_shooting_range(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ));
            }

            if ctx.players().opponents().exists(70.0) {
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }

            if distance_to_goal > PASSING_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }
        } else {
            if ctx.team().is_control_ball() {
                if self.should_support_attack(ctx) || !self.is_leading_forward(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Assisting,
                    ));
                }

                if self.should_create_space(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::CreatingSpace,
                    ));
                }
            }

            if ctx.ball().distance() < 200.0
                && !ctx.team().is_control_ball()
                && ctx.ball().is_towards_player_with_angle(0.85)
            {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Intercepting,
                ));
            }

            if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
                let opponent_distance = ctx.player().distance_to_player(opponent_with_ball.id);

                if opponent_distance < PRESSING_DISTANCE_THRESHOLD {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Pressing,
                    ));
                }
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
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
                        path_offset: IntegerUtils::random(1, 10) as f32,
                    }
                        .calculate(ctx.player)
                        .velocity,
                );
            }
        }
        
        if ctx.player.has_ball(ctx) {
            let goal_direction = ctx.player().opponent_goal_position();

            let player_goal_velocity = SteeringBehavior::Arrive {
                target: goal_direction + ctx.player().separation_velocity(),
                slowing_distance: 100.0,
            }
            .calculate(ctx.player)
            .velocity;

            Some(player_goal_velocity)
        } else {
            if ctx.ball().is_owned() {
                let players = ctx.players();
                let opponents = players.opponents();

                if let Some(goalkeeper) = opponents.goalkeeper().next() {
                    let result = SteeringBehavior::Arrive {
                        target: goalkeeper.position + ctx.player().separation_velocity(),
                        slowing_distance: 200.0
                    }
                        .calculate(ctx.player)
                        .velocity;

                    return Some(result + ctx.player().separation_velocity());
                };
            } else {
                let players = ctx.players();
                let opponents = players.opponents();

                if let Some(goalkeeper) = opponents.goalkeeper().next() {
                    let result = SteeringBehavior::Arrive {
                        target: ctx.tick_context.positions.ball.position,
                        slowing_distance: 0.0
                    }
                        .calculate(ctx.player)
                        .velocity;

                    return Some(result + ctx.player().separation_velocity());
                };
            };

            None
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl ForwardRunningState {
    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        (MIN_SHOOTING_DISTANCE..=MAX_SHOOTING_DISTANCE).contains(&distance_to_goal)
    }

    fn has_clear_shot(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.ball().distance_to_opponent_goal() < SHOOTING_DISTANCE_THRESHOLD {
            return ctx.player().has_clear_shot();
        }

        false
    }

    fn is_leading_forward(&self, ctx: &StateProcessingContext) -> bool {
        let players = ctx.players();
        let teammates = players.teammates();

        let forwards = teammates.forwards();

        let (leading_forward, _) =
            forwards.fold((None, f32::MIN), |(leading_player, max_score), player| {
                let distance =
                    (player.position - ctx.tick_context.positions.ball.position).magnitude();

                let players = ctx.player();
                let skills = players.skills(player.id);

                let speed = skills.max_speed();
                let time_to_ball = distance / speed;

                let score = skills.technical.average() + skills.mental.average() - time_to_ball;

                if score > max_score {
                    (Some(player), score)
                } else {
                    (leading_player, max_score)
                }
            });

        if let Some(leading_forward) = leading_forward {
            if leading_forward.id == ctx.player.id {
                // The current player is the leading forward
                true
            } else {
                // Check if the current player is within a certain range of the leading forward
                let distance_to_leading_forward =
                    (ctx.player.position - leading_forward.position).magnitude();
                if distance_to_leading_forward <= ASSISTING_DISTANCE_THRESHOLD {
                    // The current player is close enough to the leading forward to be considered assisting
                    false
                } else {
                    // Check if the current player has a better score than the leading forward
                    let player_distance = (ctx.player.position
                        - ctx.tick_context.positions.ball.position)
                        .magnitude();

                    let player = ctx.player();
                    let skills = player.skills(leading_forward.id);

                    let player_speed = skills.max_speed();
                    let player_time_to_ball = player_distance / player_speed;

                    let player_score =
                        skills.technical.average() + skills.mental.average() - player_time_to_ball;

                    let leading_forward_distance = (leading_forward.position
                        - ctx.tick_context.positions.ball.position)
                        .magnitude();
                    let leading_forward_speed = skills.max_speed();
                    let leading_forward_time_to_ball =
                        leading_forward_distance / leading_forward_speed;

                    let leading_forward_score = skills.technical.average()
                        + skills.mental.average()
                        - leading_forward_time_to_ball;

                    player_score > leading_forward_score
                }
            }
        } else {
            // No other forwards, so the current player is the leading forward
            true
        }
    }

    fn should_support_attack(&self, ctx: &StateProcessingContext) -> bool {
        let player_position = ctx.player.position;
        let goal_position = ctx.player().opponent_goal_position();
        let distance_to_goal = (player_position - goal_position).magnitude();

        // Check if the player is in the attacking half of the field
        let in_attacking_half = player_position.x > ctx.context.field_size.width as f32 / 2.0;

        // Check if the player is within a certain distance from the goal
        let goal_distance_threshold = ctx.context.field_size.width as f32 * 0.3; // Adjust the threshold as needed

        in_attacking_half && distance_to_goal < goal_distance_threshold
    }

    fn should_create_space(&self, ctx: &StateProcessingContext) -> bool {
        ctx.team().is_control_ball() && ctx.players().opponents().exists(50.0)
    }
}
