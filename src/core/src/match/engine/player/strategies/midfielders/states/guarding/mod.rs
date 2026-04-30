use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

const GUARD_DISTANCE: f32 = 25.0; // Keep a realistic marking distance (don't sit on top of opponent)
const MAX_GUARD_RANGE: f32 = 100.0; // Give up guarding if attacker moves too far
const TACKLE_TRANSITION_DISTANCE: f32 = 15.0; // Tackle if opponent receives ball nearby
const STAMINA_THRESHOLD: f32 = 15.0;
const PREDICTION_TIME: f32 = 0.25;
const MAX_DISTANCE_FROM_START: f32 = 150.0; // Don't follow opponent too far from tactical zone
const BOUNDARY_MARGIN: f32 = 15.0; // Stay away from field edges

#[derive(Default, Clone)]
pub struct MidfielderGuardingState {}

impl StateProcessingHandler for MidfielderGuardingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
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

        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
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

        // Press opponent with ball if nearby — midfielders must engage
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let dist = opponent_with_ball.distance(ctx);
            // Close — tackle aggressively
            if dist < 25.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }
            // Only the best-positioned player presses further out
            if dist < 100.0 && ctx.team().is_best_player_to_chase_ball() {
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
            if ctx.ball().distance() < 80.0 && ctx.ball().is_towards_player_with_angle(0.7) {
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

            // Don't follow opponent too far from tactical position
            let dist_from_start = (ctx.player.position - ctx.player.start_position).magnitude();
            if dist_from_start > MAX_DISTANCE_FROM_START {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            // Don't get stuck at the boundary following an opponent
            let field_width = ctx.context.field_size.width as f32;
            let field_height = ctx.context.field_size.height as f32;
            let pos = ctx.player.position;
            let at_boundary = pos.x < BOUNDARY_MARGIN
                || pos.x > field_width - BOUNDARY_MARGIN
                || pos.y < BOUNDARY_MARGIN
                || pos.y > field_height - BOUNDARY_MARGIN;

            if at_boundary {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            // Continue guarding from distance
            None
        } else {
            // No one to guard — go to Running (NOT Returning, which would loop back here)
            Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ))
        }
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if let Some(opponent) = self.find_guard_target(ctx) {
            let opponent_velocity = opponent.velocity(ctx);
            let own_goal = ctx.ball().direction_to_own_goal();

            // Predict where opponent is heading
            let opponent_future = opponent.position + opponent_velocity * PREDICTION_TIME;

            // Position between opponent and our goal at GUARD_DISTANCE away
            let to_goal = (own_goal - opponent_future).normalize();
            let desired_position = opponent_future + to_goal * GUARD_DISTANCE;

            // Blend with tactical position to avoid straying too far
            let tether_strength = 0.2;
            let desired_position = desired_position * (1.0 - tether_strength)
                + ctx.player.start_position * tether_strength;

            let to_desired = desired_position - ctx.player.position;
            let distance = to_desired.magnitude();

            // Dead zone: close enough — hold position, no jitter
            if distance < 8.0 {
                return Some(Vector3::zeros());
            }

            let direction = to_desired.normalize();

            // Speed based on how far off position we are
            let base_speed = ctx.player.skills.physical.pace * 0.4;
            let urgency = (distance / GUARD_DISTANCE).clamp(0.4, 1.0);

            Some(direction * base_speed * urgency)
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
            // From the opponent's POV, our teammates are their "opponents"
            let has_nearby_cover = ctx
                .tick_context
                .grid
                .opponents(opponent.id, 15.0)
                .any(|(t_id, _)| t_id != ctx.player.id);

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
