use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

const GUARD_DISTANCE: f32 = 8.0; // Stay tight to the attacker
const MAX_GUARD_RANGE: f32 = 100.0; // Give up guarding if attacker moves too far
const TACKLE_TRANSITION_DISTANCE: f32 = 8.0; // Tackle if opponent receives ball
const STAMINA_THRESHOLD: f32 = 15.0;
const PREDICTION_TIME: f32 = 0.25;

#[derive(Default, Clone)]
pub struct MidfielderGuardingState {}

impl StateProcessingHandler for MidfielderGuardingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If we have the ball, run with it
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Team regained possession — support attack
        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Free ball nearby — claim it
        if ctx.ball().should_take_ball_immediately() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::TakeBall,
            ));
        }

        // Stamina check
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // Press opponent with ball if nearby
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let dist = opponent_with_ball.distance(ctx);
            if dist < 40.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }
            if dist < 100.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
        }

        // Find the opponent to guard
        let guard_target = self.find_guard_target(ctx);

        if let Some(opponent) = guard_target {
            let distance = opponent.distance(ctx);

            // Opponent received the ball — react
            if opponent.has_ball(ctx) {
                if distance < TACKLE_TRANSITION_DISTANCE {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Tackling,
                    ));
                }
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            // Ball coming toward guarded opponent — intercept
            if ctx.ball().distance() < 80.0
                && ctx.ball().is_towards_player_with_angle(0.7)
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }

            // Opponent too far — give up guarding
            if distance > MAX_GUARD_RANGE {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            // Ball far away on opponent's side — no need to guard
            if !ctx.ball().on_own_side() && ctx.ball().distance() > 300.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Running,
                ));
            }

            // Continue guarding
            None
        } else {
            // No one to guard — track back or press
            Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if let Some(opponent) = self.find_guard_target(ctx) {
            let opponent_velocity = opponent.velocity(ctx);
            let own_goal = ctx.ball().direction_to_own_goal();
            let ball_position = ctx.tick_context.positions.ball.position;

            // Predict where opponent is heading
            let opponent_future = opponent.position + opponent_velocity * PREDICTION_TIME;

            // Position between opponent and ball (deny passes)
            let to_ball = (ball_position - opponent_future).normalize();
            let ball_deny_offset = to_ball * GUARD_DISTANCE * 0.4;

            // Position between opponent and goal (deny shooting lane)
            let to_goal = (own_goal - opponent_future).normalize();
            let goal_side_offset = to_goal * GUARD_DISTANCE * 0.3;

            let desired_position = opponent_future + ball_deny_offset + goal_side_offset;

            let to_desired = desired_position - ctx.player.position;
            let distance = to_desired.magnitude();

            if distance < 1.0 {
                // Mirror opponent movement
                return Some(opponent_velocity * 0.9);
            }

            let direction = to_desired.normalize();
            let opponent_speed = opponent_velocity.norm();
            let chase_speed = ctx.player.skills.physical.pace;
            let speed = chase_speed.max(opponent_speed * 1.05);
            let urgency = (distance / GUARD_DISTANCE).clamp(0.6, 1.4);

            Some(direction * speed * urgency + ctx.player().separation_velocity() * 0.2)
        } else {
            Some(Vector3::zeros())
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Guarding requires constant movement — high intensity
        MidfielderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl MidfielderGuardingState {
    /// Find the best opponent to guard — attackers without ball trying to find space
    fn find_guard_target(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let own_goal = ctx.ball().direction_to_own_goal();
        let ball_position = ctx.tick_context.positions.ball.position;

        let mut best_target: Option<MatchPlayerLite> = None;
        let mut best_score = f32::MIN;

        for opponent in ctx.players().opponents().nearby(MAX_GUARD_RANGE) {
            // Skip the ball carrier
            if opponent.has_ball(ctx) {
                continue;
            }

            let mut score = 0.0;

            // Factor 1: Proximity to our goal
            let dist_to_goal = (opponent.position - own_goal).magnitude();
            score += (400.0 - dist_to_goal.min(400.0)) / 8.0;

            // Factor 2: Proximity to ball (can receive passes)
            let dist_to_ball = (opponent.position - ball_position).magnitude();
            score += (200.0 - dist_to_ball.min(200.0)) / 8.0;

            // Factor 3: Movement toward our goal
            let velocity = opponent.velocity(ctx);
            let speed = velocity.norm();
            if speed > 1.0 {
                let move_dir = velocity.normalize();
                let to_goal = (own_goal - opponent.position).normalize();
                let alignment = move_dir.dot(&to_goal);
                if alignment > 0.0 {
                    score += alignment * speed * 8.0;
                }
            }

            // Factor 4: Unmarked bonus — no defender or midfielder covering this attacker
            let has_nearby_cover = ctx.players().teammates().all()
                .any(|t| {
                    if t.id == ctx.player.id { return false; }
                    (t.position - opponent.position).magnitude() < 15.0
                });

            if !has_nearby_cover {
                score += 35.0;
            }

            // Factor 5: Closeness to us
            let dist_to_us = opponent.distance(ctx);
            score += (60.0 - dist_to_us.min(60.0)) / 3.0;

            if score > best_score {
                best_score = score;
                best_target = Some(opponent);
            }
        }

        best_target
    }
}
