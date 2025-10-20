use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior, VectorExtensions,
};
use nalgebra::Vector3;

const DANGER_ZONE_RADIUS: f32 = 30.0;
const CLOSE_DANGER_DISTANCE: f32 = 80.0;
const MEDIUM_THREAT_DISTANCE: f32 = 150.0;
const FAR_THREAT_DISTANCE: f32 = 250.0;

#[derive(Default)]
pub struct GoalkeeperStandingState {}

impl StateProcessingHandler for GoalkeeperStandingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If goalkeeper has the ball, decide whether to pass or run
        if ctx.player.has_ball(ctx) {
            return if ctx.players().opponents().exists(DANGER_ZONE_RADIUS) {
                Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Passing,
                ))
            } else {
                Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Running,
                ))
            }
        }

        let ball_distance = ctx.ball().distance();
        let ball_on_own_side = ctx.ball().on_own_side();

        // Skill-based threat assessment
        let anticipation = ctx.player.skills.mental.anticipation / 20.0;
        let positioning = ctx.player.skills.mental.positioning / 20.0;
        let command_of_area = ctx.player.skills.mental.vision / 20.0;

        // Check for immediate threats requiring urgent action
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let opponent_distance = opponent.distance(ctx);

            // Opponent very close with ball - prepare for save or come out
            if opponent_distance < CLOSE_DANGER_DISTANCE {
                // Check if should come out or prepare for shot
                if self.should_rush_out_for_ball(ctx, &opponent) {
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::ComingOut,
                    ));
                } else {
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::PreparingForSave,
                    ));
                }
            }

            // Opponent approaching - be attentive
            if opponent_distance < MEDIUM_THREAT_DISTANCE && ball_on_own_side {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Attentive,
                ));
            }
        }

        // Check if ball is coming toward goal
        if ctx.ball().is_towards_player_with_angle(0.7) && ball_on_own_side {
            let ball_speed = ctx.tick_context.positions.ball.velocity.norm();

            if ball_speed > 5.0 {
                // Fast ball coming - prepare for save
                if ball_distance < MEDIUM_THREAT_DISTANCE * (1.0 + anticipation * 0.5) {
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::PreparingForSave,
                    ));
                }
            }

            // Ball coming slowly - be attentive
            if ball_distance < FAR_THREAT_DISTANCE {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Attentive,
                ));
            }
        }

        // Check for loose ball in dangerous area
        if !ctx.ball().is_owned() && ball_on_own_side && ball_distance < CLOSE_DANGER_DISTANCE * (1.0 + command_of_area * 0.5) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ComingOut,
            ));
        }

        // Ball on own side - be attentive
        if ball_on_own_side && ball_distance < FAR_THREAT_DISTANCE {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Attentive,
            ));
        }

        // Check positioning
        match ctx.player().position_to_distance() {
            PlayerDistanceFromStartPosition::Small => {
                // Good positioning - check for specific threats
                if self.is_opponent_in_danger_zone(ctx) {
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::UnderPressure,
                    ));
                }
            }
            PlayerDistanceFromStartPosition::Medium => {
                // Need to adjust position - walk to better spot
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Walking,
                ));
            }
            PlayerDistanceFromStartPosition::Big => {
                // Far from position - walk back
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Walking,
                ));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        // For now, return None to indicate no state change
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Calculate optimal position based on ball and goal
        let optimal_position = self.calculate_optimal_position(ctx);
        let distance_to_optimal = ctx.player.position.distance_to(&optimal_position);

        // If we're close to optimal position, make small adjustments
        if distance_to_optimal < 5.0 {
            // Very small movements to stay alert and ready
            Some(
                SteeringBehavior::Wander {
                    target: optimal_position,
                    radius: 2.0,
                    jitter: 0.5,
                    distance: 2.0,
                    angle: (ctx.in_state_time % 360) as f32,
                }
                .calculate(ctx.player)
                .velocity * 0.2, // Very slow movement
            )
        } else if distance_to_optimal < 15.0 {
            // Small repositioning needed
            Some(
                SteeringBehavior::Arrive {
                    target: optimal_position,
                    slowing_distance: 5.0,
                }
                .calculate(ctx.player)
                .velocity * 0.4, // Moderate speed
            )
        } else {
            // Need to move to better position
            Some(
                SteeringBehavior::Arrive {
                    target: optimal_position,
                    slowing_distance: 10.0,
                }
                .calculate(ctx.player)
                .velocity * 0.6, // Faster movement
            )
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions to process in this state
    }
}

impl GoalkeeperStandingState {
    fn is_opponent_in_danger_zone(&self, ctx: &StateProcessingContext) -> bool {
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let opponent_distance = ctx
                .tick_context
                .distances
                .get(ctx.player.id, opponent_with_ball.id);

            return opponent_distance < DANGER_ZONE_RADIUS;
        }

        false
    }

    /// Determine if goalkeeper should rush out for the ball
    fn should_rush_out_for_ball(&self, ctx: &StateProcessingContext, opponent: &MatchPlayerLite) -> bool {
        let ball_position = ctx.tick_context.positions.ball.position;
        let keeper_position = ctx.player.position;
        let opponent_position = opponent.position;

        // Distance calculations
        let keeper_to_ball = (ball_position - keeper_position).magnitude();
        let opponent_to_ball = (ball_position - opponent_position).magnitude();

        // Goalkeeper skills
        let anticipation = ctx.player.skills.mental.anticipation / 20.0;
        let decisions = ctx.player.skills.mental.decisions / 20.0;
        let rushing_out = (anticipation + decisions) / 2.0;

        // Opponent skills
        let opponent_control = ctx.player().skills(opponent.id).technical.first_touch / 20.0;
        let opponent_pace = ctx.player().skills(opponent.id).physical.pace / 20.0;

        // Calculate time to reach ball (rough estimate)
        let keeper_speed = ctx.player.skills.physical.acceleration * (1.0 + rushing_out * 0.3);
        let opponent_speed = opponent_pace * 20.0;

        let keeper_time = keeper_to_ball / keeper_speed;
        let opponent_time = opponent_to_ball / opponent_speed;

        // Factors favoring rushing out:
        // 1. Keeper can reach ball first (with skill advantage)
        // 2. Ball is loose or opponent has poor control
        // 3. Ball is within reasonable distance

        let can_reach_first = keeper_time < opponent_time * (1.0 + rushing_out * 0.2);
        let ball_loose_or_poor_control = !ctx.ball().is_owned() || opponent_control < 0.5;
        let reasonable_distance = keeper_to_ball < CLOSE_DANGER_DISTANCE * 1.5;

        can_reach_first && ball_loose_or_poor_control && reasonable_distance
    }

    /// Calculate optimal goalkeeper position based on ball and goal
    fn calculate_optimal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_center = ctx.ball().direction_to_own_goal();
        let ball_position = ctx.tick_context.positions.ball.position;

        // Goalkeeper skills affecting positioning
        let positioning_skill = ctx.player.skills.mental.positioning / 20.0;
        let command_of_area = ctx.player.skills.mental.vision / 20.0;

        // Calculate distance from goal to ball
        let goal_to_ball = ball_position - goal_center;
        let distance_to_ball = goal_to_ball.magnitude();

        // Base distance from goal line (in meters/units)
        let mut optimal_distance_from_goal = 4.0; // Start about 4 units from goal line

        // Adjust based on ball position
        if ctx.ball().on_own_side() {
            // Ball on defensive half - position based on threat level
            let threat_distance = distance_to_ball.min(300.0) / 300.0; // Normalize to 0-1

            // Closer ball = come out more (but not too far)
            optimal_distance_from_goal += (1.0 - threat_distance) * 8.0 * command_of_area;

            // Better positioning = more accurate placement
            optimal_distance_from_goal *= 0.9 + positioning_skill * 0.2;

            // Narrow the angle - position on line between goal and ball
            let direction_to_ball = if distance_to_ball > 1.0 {
                goal_to_ball.normalize()
            } else {
                Vector3::new(1.0, 0.0, 0.0) // Fallback if ball too close to goal
            };

            let mut new_position = goal_center + direction_to_ball * optimal_distance_from_goal;

            // Lateral adjustment for angle coverage
            let ball_y_offset = ball_position.y - goal_center.y;
            let lateral_adjustment = ball_y_offset * 0.05 * positioning_skill;
            new_position.y += lateral_adjustment;

            // Keep within penalty area
            self.clamp_to_penalty_area(ctx, new_position)
        } else {
            // Ball on opponent's half - stay closer to goal but ready
            optimal_distance_from_goal = 6.0 + command_of_area * 4.0;

            let mut new_position = goal_center;
            new_position.x += optimal_distance_from_goal * (if ctx.player.side == Some(PlayerSide::Left) { 1.0 } else { -1.0 });

            self.clamp_to_penalty_area(ctx, new_position)
        }
    }

    fn clamp_to_penalty_area(
        &self,
        ctx: &StateProcessingContext,
        position: Vector3<f32>,
    ) -> Vector3<f32> {
        let penalty_area = ctx
            .context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left));
        Vector3::new(
            position.x.clamp(penalty_area.min.x, penalty_area.max.x),
            position.y.clamp(penalty_area.min.y, penalty_area.max.y),
            0.0,
        )
    }
}
