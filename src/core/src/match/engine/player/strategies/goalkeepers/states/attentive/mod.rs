use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::processor::StateChangeResult;
use crate::r#match::player::strategies::processor::{StateProcessingContext, StateProcessingHandler};
use crate::r#match::{
    ConditionContext, PlayerSide, SteeringBehavior,
    VectorExtensions,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperAttentiveState {}

impl StateProcessingHandler for GoalkeeperAttentiveState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // First, handle if goalkeeper already has the ball
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Distributing,
            ));
        }

        // Check if the ball is on the goalkeeper's side of the field
        if ctx.ball().on_own_side() {
            // Calculate actual distance to ball rather than using a fixed threshold
            let ball_distance = ctx.ball().distance();

            // If the ball is close and the goalkeeper should come out, transition to ComingOut
            if self.should_come_out(ctx) && ball_distance < 300.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ComingOut,
                ));
            }
            // If the ball is moving toward the goalkeeper, prepare for save
            else if ctx.ball().is_towards_player_with_angle(0.8) && ball_distance < 200.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
            // If the ball is very close to the goalkeeper, attempt to intercept it
            else if ball_distance < 50.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ComingOut,
                ));
            }
        } else {
            // If the ball is on the opponent's side, transition to Walking state
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Walking,
            ));
        }

        // Check if the goalkeeper is out of position and needs to return
        if self.is_out_of_position(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // Check if there's an immediate threat
        if self.is_under_threat(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::UnderPressure,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.tick_context.positions.ball.position,
                slowing_distance: 50.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl GoalkeeperAttentiveState {
    fn is_out_of_position(&self, ctx: &StateProcessingContext) -> bool {
        let optimal_position = self.calculate_optimal_position(ctx);
        // Reduced threshold for more responsive positioning
        ctx.player.position.distance_to(&optimal_position) > 70.0
    }

    fn is_under_threat(&self, ctx: &StateProcessingContext) -> bool {
        // Check if any opponent with the ball is near
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent.position.distance_to(&ctx.player.position);
            let penalty_area_threshold = 40.0;

            // If opponent with ball is in or near penalty area
            if distance_to_opponent < penalty_area_threshold {
                return true;
            }
        }

        // Also check if ball is moving quickly toward goal
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();
        let is_toward_goal = ctx.ball().is_towards_player_with_angle(0.8);

        if ball_speed > 15.0 && is_toward_goal && ctx.ball().distance() < 150.0 {
            return true;
        }

        false
    }

    fn should_come_out(&self, ctx: &StateProcessingContext) -> bool {
        // Ball distance and movement factors
        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();
        let is_ball_moving_towards_goal = ctx.ball().is_towards_player_with_angle(0.6);

        // Goalkeeper skill factors
        let goalkeeper_skills = &ctx.player.skills;
        let decision_skill = goalkeeper_skills.mental.decisions;
        let positioning_skill = goalkeeper_skills.mental.positioning;
        let rushing_out_skill = (decision_skill + positioning_skill) / 2.0;

        // Distance thresholds - adjusted by goalkeeper skill
        let base_threshold = 100.0;
        let skill_factor = rushing_out_skill / 20.0; // Normalize to 0-1
        let adjusted_threshold = base_threshold * (0.7 + skill_factor * 0.6); // Range: 70-130

        // Case 1: Ball is very close and no one has possession
        if ball_distance < 100.0 && !ctx.ball().is_owned() {
            return true;
        }

        // Case 2: Ball is moving toward goal at speed
        if is_ball_moving_towards_goal && ball_speed > 5.0 && ball_distance < adjusted_threshold {
            return true;
        }

        // Case 3: Ball is loose in dangerous area
        if !ctx.ball().is_owned()
            && ball_distance < adjusted_threshold
            && self.is_ball_in_danger_area(ctx)
        {
            return true;
        }

        // Case 4: Opponent with ball is approaching but still at interceptable distance
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            if ctx.player().distance_to_player(opponent.id) < adjusted_threshold * 0.8 {
                // Check if goalkeeper can reach ball before opponent
                if self.can_reach_before_opponent(ctx, &opponent) {
                    return true;
                }
            }
        }

        false
    }

    fn can_reach_before_opponent(
        &self,
        ctx: &StateProcessingContext,
        opponent: &crate::r#match::MatchPlayerLite,
    ) -> bool {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let opponent_pos = opponent.position;

        // Estimate distance and time for both keeper and opponent
        let dist_keeper_to_ball = (ball_pos - keeper_pos).magnitude();
        let dist_opponent_to_ball = (ball_pos - opponent_pos).magnitude();

        // Use skill-adjusted speeds
        let keeper_speed = ctx.player.skills.physical.acceleration * 1.2; // Boost for urgency
        let opponent_speed = ctx.player().skills(opponent.id).physical.pace;

        // Calculate estimated times
        let time_keeper = dist_keeper_to_ball / keeper_speed;
        let time_opponent = dist_opponent_to_ball / opponent_speed;

        // Add skill-based advantage for goalkeeper decisions
        let decision_advantage = ctx.player.skills.mental.decisions / 40.0; // 0-0.5 range

        // Return true if goalkeeper can reach ball first
        time_keeper < (time_opponent * (1.0 - decision_advantage))
    }

    fn is_ball_in_danger_area(&self, ctx: &StateProcessingContext) -> bool {
        // Get relevant positions
        let ball_position = ctx.tick_context.positions.ball.position;
        let goal_position = ctx.ball().direction_to_own_goal();

        // Calculate distance from ball to goal
        let distance_to_goal = (ball_position - goal_position).magnitude();

        // Define danger area threshold based on field size
        let field_width = ctx.context.field_size.width as f32;
        let danger_threshold = field_width * 0.2; // 20% of field width

        // Check if ball is in danger area
        distance_to_goal < danger_threshold
    }

    fn calculate_optimal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_position = ctx.ball().direction_to_own_goal();
        let ball_position = ctx.tick_context.positions.ball.position;

        // Calculate vector from goal to ball
        let goal_to_ball = ball_position - goal_position;

        // Normalize the vector and scale it based on goalkeeper positioning skill
        let positioning_skill = ctx.player.skills.mental.positioning / 20.0; // Normalize to 0-1

        // Determine optimal distance - better goalkeepers position more optimally
        let base_distance = 5.0;
        let skill_adjusted_distance = base_distance * (0.8 + positioning_skill * 0.4);

        // Position is on the line between goal and ball, but closer to goal
        let new_position = if goal_to_ball.magnitude() > 0.1 {
            goal_position + goal_to_ball.normalize() * skill_adjusted_distance
        } else {
            // Fallback if ball is too close to goal center
            goal_position
        };

        // Limit the goalkeeper's movement to stay within the penalty area
        self.limit_to_penalty_area(new_position, ctx)
    }

    fn limit_to_penalty_area(
        &self,
        position: Vector3<f32>,
        ctx: &StateProcessingContext,
    ) -> Vector3<f32> {
        // Get penalty area dimensions
        let penalty_area = ctx
            .context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left));

        // Clamp position to stay within penalty area
        Vector3::new(
            position.x.clamp(penalty_area.min.x, penalty_area.max.x),
            position.y.clamp(penalty_area.min.y, penalty_area.max.y),
            0.0,
        )
    }
}
