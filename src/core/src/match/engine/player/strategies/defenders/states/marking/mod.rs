use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const MARKING_DISTANCE_THRESHOLD: f32 = 8.0; // Reduced from 10.0 - tighter marking
const TACKLING_DISTANCE_THRESHOLD: f32 = 4.0; // Increased from 3.0 - tackle earlier
const STAMINA_THRESHOLD: f32 = 20.0; // Minimum stamina to continue marking
const BALL_PROXIMITY_THRESHOLD: f32 = 15.0; // Increased from 10.0 - react earlier to ball
const HEADING_HEIGHT: f32 = 1.5;
const HEADING_DISTANCE: f32 = 5.0;
const GOAL_SIDE_WEIGHT: f32 = 0.6; // How much to prioritize being goal-side
const SWITCH_OPPONENT_THRESHOLD: f32 = 50.0; // Distance to consider switching marking target

#[derive(Default)]
pub struct DefenderMarkingState {}

impl StateProcessingHandler for DefenderMarkingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Priority 0: Free ball nearby - go claim it
        if ctx.ball().should_take_ball_immediately() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // 1. Check if the defender has enough stamina to continue marking
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
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

        // 2. Find best opponent to mark using coordination system
        // First try to find an unmarked opponent if current target is being engaged by another defender
        let opponent_to_mark = self.find_best_marking_target(ctx);

        if let Some(opponent) = opponent_to_mark {
            let distance_to_opponent = opponent.distance(ctx);

            // Priority: If opponent with ball is very close, press/tackle immediately
            if opponent.has_ball(ctx) {
                if distance_to_opponent < TACKLING_DISTANCE_THRESHOLD {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }
                // Press the ball carrier if close enough
                if distance_to_opponent < 20.0 && ctx.player().defensive().is_best_defender_for_opponent(&opponent) {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
            }

            // If opponent is too far, switch to running to catch up
            if distance_to_opponent > MARKING_DISTANCE_THRESHOLD * 2.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Running,
                ));
            }

            // If ball is close and unmarked, consider intercepting
            if ctx.ball().distance() < BALL_PROXIMITY_THRESHOLD && !opponent.has_ball(ctx) {
                if ctx.ball().is_towards_player_with_angle(0.7) {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Intercepting,
                    ));
                }
            }

            // Check if another defender is already engaging this opponent
            // If so, look for unmarked threats
            if ctx.player().defensive().is_opponent_being_engaged(&opponent) {
                if let Some(_unmarked) = ctx.player().defensive().find_unmarked_opponent(SWITCH_OPPONENT_THRESHOLD) {
                    // Switch to covering to find better position
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Covering,
                    ));
                }
            }

            // Continue marking
            None
        } else {
            // No opponent to mark found
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
        // Move to maintain goal-side position relative to the opponent being marked

        if let Some(opponent_to_mark) = self.find_best_marking_target(ctx) {
            let own_goal = ctx.ball().direction_to_own_goal();
            let opponent_velocity = opponent_to_mark.velocity(ctx);

            // Predict opponent's future position
            let prediction_time = 0.3; // Look ahead 300ms
            let opponent_future_position = opponent_to_mark.position + opponent_velocity * prediction_time;

            // Calculate goal-side marking position
            // Position between opponent and own goal
            let to_goal = (opponent_future_position - own_goal).normalize();
            let goal_side_offset = to_goal * MARKING_DISTANCE_THRESHOLD * GOAL_SIDE_WEIGHT;

            // Also consider ball-side positioning
            let ball_position = ctx.tick_context.positions.ball.position;
            let to_ball = (ball_position - opponent_future_position).normalize();
            let ball_side_offset = to_ball * MARKING_DISTANCE_THRESHOLD * (1.0 - GOAL_SIDE_WEIGHT);

            // Blend goal-side and ball-side positioning
            let desired_position = opponent_future_position + goal_side_offset + ball_side_offset;

            let to_desired = desired_position - ctx.player.position;
            let distance = to_desired.magnitude();

            if distance < 1.0 {
                // Close enough, minimal adjustment
                return Some(to_desired * 0.5);
            }

            let direction = to_desired.normalize();
            // Speed based on urgency - faster if far, slower if close
            let urgency = (distance / MARKING_DISTANCE_THRESHOLD).clamp(0.5, 1.5);
            let speed = ctx.player.skills.physical.pace * urgency;

            Some(direction * speed + ctx.player().separation_velocity() * 0.3)
        } else {
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Marking involves constant movement following opponent - moderate intensity
        DefenderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}

impl DefenderMarkingState {
    /// Find the best marking target using coordination system
    /// Prefers unmarked opponents to avoid double-marking
    fn find_best_marking_target(&self, ctx: &StateProcessingContext) -> Option<crate::r#match::MatchPlayerLite> {
        // First, try to find an unmarked dangerous opponent
        if let Some(unmarked) = ctx.player().defensive().find_unmarked_opponent(100.0) {
            return Some(unmarked);
        }

        // If all dangerous opponents are marked, use the traditional scoring
        // but only if we're the best defender for them
        let dangerous = self.find_most_dangerous_opponent(ctx);

        if let Some(ref opp) = dangerous {
            // Only take over marking if we're significantly better positioned
            if ctx.player().defensive().is_best_defender_for_opponent(opp) {
                return dangerous;
            }
        }

        // Return the dangerous opponent anyway if no other option
        dangerous
    }

    /// Find the most dangerous opponent to mark based on multiple factors
    fn find_most_dangerous_opponent(&self, ctx: &StateProcessingContext) -> Option<crate::r#match::MatchPlayerLite> {
        // Extended search range to catch dangerous runs from distance
        let nearby_opponents: Vec<_> = ctx.players().opponents().nearby(150.0).collect();

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

            // Factor 4: Opponent facing our goal (attacking posture) - ENHANCED
            let opponent_velocity = opponent.velocity(ctx);
            let to_our_goal = (own_goal_position - opponent.position).normalize();
            let speed = opponent_velocity.norm();

            if speed > 0.1 {
                let velocity_dir = opponent_velocity.normalize();
                let alignment = velocity_dir.dot(&to_our_goal);

                if alignment > 0.0 {
                    // Base points for running towards goal
                    danger_score += alignment * 30.0;

                    // Bonus for dangerous runs: high speed + good alignment
                    if speed > 3.0 && alignment > 0.7 {
                        danger_score += 25.0; // Additional points for clear dangerous run
                    }
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

