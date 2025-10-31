use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const MARKING_DISTANCE_THRESHOLD: f32 = 10.0; // Desired distance to maintain from the opponent
const TACKLING_DISTANCE_THRESHOLD: f32 = 3.0; // Distance within which the defender can tackle
const STAMINA_THRESHOLD: f32 = 20.0; // Minimum stamina to continue marking
const BALL_PROXIMITY_THRESHOLD: f32 = 10.0; // Distance to consider the ball as close
const HEADING_HEIGHT: f32 = 1.5;
const HEADING_DISTANCE: f32 = 5.0;

#[derive(Default)]
pub struct DefenderMarkingState {}

impl StateProcessingHandler for DefenderMarkingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the defender has enough stamina to continue marking
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
            // Transition to Resting state if stamina is low
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Resting,
            ));
        }

        // Check if ball is aerial and at heading height
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_distance = ctx.ball().distance();

        if ball_position.z > HEADING_HEIGHT
            && ball_distance < HEADING_DISTANCE
            && ctx.ball().is_towards_player_with_angle(0.6) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Heading,
            ));
        }

        // 2. Identify the most dangerous opponent player to mark
        if let Some(opponent_to_mark) = self.find_most_dangerous_opponent(ctx) {
            let distance_to_opponent = opponent_to_mark.distance(ctx);

            // 4. If the opponent has the ball and is within tackling distance, attempt a tackle
            if opponent_to_mark.has_ball(ctx) && distance_to_opponent < TACKLING_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            }

            // 5. If the opponent is beyond the marking distance threshold, switch to Running state to catch up
            if distance_to_opponent > MARKING_DISTANCE_THRESHOLD * 1.5 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Running,
                ));
            }

            // 6. If the ball is close to the defender, consider intercepting
            if ctx.ball().distance() < BALL_PROXIMITY_THRESHOLD && !opponent_to_mark.has_ball(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Intercepting,
                ));
            }

            // 7. Continue marking (no state change)
            None
        } else {
            // No opponent to mark found
            // Transition back to HoldingLine or appropriate state
            Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        // For now, return None to indicate no state change
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move to maintain position relative to the opponent being marked

        // Identify the most dangerous opponent player to mark
        if let Some(opponent_to_mark) = self.find_most_dangerous_opponent(ctx) {
            // Calculate desired position to maintain proper marking
            let opponent_future_position = opponent_to_mark.position + opponent_to_mark.velocity(ctx);
            let desired_position = opponent_future_position
                - (opponent_to_mark.velocity(ctx).normalize() * MARKING_DISTANCE_THRESHOLD);

            let direction = (desired_position - ctx.player.position).normalize();
            // Set speed based on player's pace
            let speed = ctx.player.skills.physical.pace; // Use pace attribute
            Some(direction * speed)
        } else {
            // No opponent to mark found
            // Remain stationary or return to default position
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions to process in this state
    }
}

impl DefenderMarkingState {
    /// Find the most dangerous opponent to mark based on multiple factors
    fn find_most_dangerous_opponent(&self, ctx: &StateProcessingContext) -> Option<crate::r#match::MatchPlayerLite> {
        let nearby_opponents: Vec<_> = ctx.players().opponents().nearby(100.0).collect();

        if nearby_opponents.is_empty() {
            return None;
        }

        let player_ops = ctx.player();

        // Calculate danger score for each opponent
        let mut best_opponent = None;
        let mut best_score = f32::MIN;

        for opponent in nearby_opponents {
            let mut danger_score = 0.0;

            // Factor 1: Has the ball (VERY dangerous)
            if opponent.has_ball(ctx) {
                danger_score += 100.0;
            }

            // Factor 2: Distance to our goal (closer = more dangerous)
            let own_goal_position = ctx.ball().direction_to_own_goal();
            let distance_to_own_goal = (opponent.position - own_goal_position).magnitude();
            danger_score += (500.0 - distance_to_own_goal) / 10.0; // Max 50 points

            // Factor 3: Distance to defender (closer = needs marking)
            let distance_to_defender = opponent.distance(ctx);
            danger_score += (100.0 - distance_to_defender.min(100.0)) / 5.0; // Max 20 points

            // Factor 4: Opponent facing our goal (attacking posture)
            let opponent_velocity = opponent.velocity(ctx);
            let to_our_goal = (own_goal_position - opponent.position).normalize();
            if opponent_velocity.norm() > 0.1 {
                let velocity_dir = opponent_velocity.normalize();
                let alignment = velocity_dir.dot(&to_our_goal);
                if alignment > 0.0 {
                    danger_score += alignment * 30.0; // Max 30 points for running towards goal
                }
            }

            // Factor 5: Ball proximity (if opponent doesn't have ball, closer to ball = more dangerous)
            if !opponent.has_ball(ctx) {
                let ball_position = ctx.tick_context.positions.ball.position;
                let distance_to_ball = (opponent.position - ball_position).magnitude();
                danger_score += (50.0 - distance_to_ball.min(50.0)) / 5.0; // Max 10 points
            }

            // Factor 6: Opponent skill level (better players are more dangerous)
            let opponent_skills = player_ops.skills(opponent.id);
            let attacking_skill = (opponent_skills.physical.pace
                + opponent_skills.technical.dribbling
                + opponent_skills.technical.finishing) / 3.0;
            danger_score += attacking_skill / 20.0; // Max ~5 points for elite attacker

            // Update best if this opponent is more dangerous
            if danger_score > best_score {
                best_score = danger_score;
                best_opponent = Some(opponent);
            }
        }

        best_opponent
    }
}

