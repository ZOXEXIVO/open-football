use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

const GUARD_DISTANCE: f32 = 20.0; // Keep a realistic marking distance (don't sit on top of opponent)
const MAX_GUARD_RANGE: f32 = 80.0; // Give up guarding if attacker moves too far
const TACKLE_TRANSITION_DISTANCE: f32 = 5.0; // Tackle if opponent receives ball and is close
const STAMINA_THRESHOLD: f32 = 15.0; // Guarding is tiring — need minimum stamina
const HEADING_HEIGHT: f32 = 1.5;
const HEADING_DISTANCE: f32 = 5.0;
const PREDICTION_TIME: f32 = 0.25; // Look ahead 250ms to mirror movement
const MAX_DISTANCE_FROM_START: f32 = 120.0; // Don't follow opponent too far from tactical zone
const BOUNDARY_MARGIN: f32 = 15.0; // Stay away from field edges

#[derive(Default, Clone)]
pub struct DefenderGuardingState {}

impl StateProcessingHandler for DefenderGuardingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // 1. Stamina check — guarding is demanding
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Resting,
            ));
        }

        // Check for aerial ball
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_distance = ctx.ball().distance();

        if ball_position.z > HEADING_HEIGHT
            && ball_distance < HEADING_DISTANCE
            && ctx.ball().is_towards_player_with_angle(0.6)
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Heading,
            ));
        }

        // 2. Find the opponent we should guard
        let guard_target = self.find_guard_target(ctx);

        if let Some(opponent) = guard_target {
            let distance_to_opponent = opponent.distance(ctx);

            // 3. If the guarded opponent receives the ball — react immediately
            if opponent.has_ball(ctx) {
                if distance_to_opponent < TACKLE_TRANSITION_DISTANCE {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // 4. Ball coming towards our guarded opponent — try to intercept
            if ball_distance < 80.0
                && ctx.ball().is_towards_player_with_angle(0.7)
                && ball_distance < distance_to_opponent + 10.0
            {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Intercepting,
                ));
            }

            // 5. If opponent is too far away, stop guarding
            if distance_to_opponent > MAX_GUARD_RANGE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }

            // 6. If ball is very far and on opponent's side, no need to guard
            if !ctx.ball().on_own_side() && ball_distance > 300.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::PushingUp,
                ));
            }

            // 7. Don't follow opponent too far from our tactical position
            let dist_from_start = (ctx.player.position - ctx.player.start_position).magnitude();
            if dist_from_start > MAX_DISTANCE_FROM_START {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }

            // 8. Don't get stuck at the boundary following an opponent
            let field_width = ctx.context.field_size.width as f32;
            let field_height = ctx.context.field_size.height as f32;
            let pos = ctx.player.position;
            let at_boundary = pos.x < BOUNDARY_MARGIN
                || pos.x > field_width - BOUNDARY_MARGIN
                || pos.y < BOUNDARY_MARGIN
                || pos.y > field_height - BOUNDARY_MARGIN;

            if at_boundary {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }

            // Continue guarding from distance
            None
        } else {
            // No one to guard — return to holding line
            Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
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

            // Predict where the opponent is heading
            let opponent_future = opponent.position + opponent_velocity * PREDICTION_TIME;

            // Position between opponent and our goal at GUARD_DISTANCE away
            // This keeps us goal-side of the opponent at a realistic marking distance
            let to_goal = (own_goal - opponent_future).normalize();
            let desired_position = opponent_future + to_goal * GUARD_DISTANCE;

            // Blend with tactical position to avoid straying too far
            let tether_strength = 0.15;
            let desired_position = desired_position * (1.0 - tether_strength)
                + ctx.player.start_position * tether_strength;

            let to_desired = desired_position - ctx.player.position;
            let distance = to_desired.magnitude();

            if distance < 2.0 {
                // Close enough — mirror opponent movement gently
                return Some(opponent_velocity * 0.5 + ctx.player().separation_velocity() * 0.3);
            }

            let direction = to_desired.normalize();

            // Speed based on how far off position we are
            let base_speed = ctx.player.skills.physical.pace * 0.4;
            let urgency = (distance / GUARD_DISTANCE).clamp(0.4, 1.0);

            Some(direction * base_speed * urgency + ctx.player().separation_velocity() * 0.3)
        } else {
            Some(Vector3::zeros())
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Guarding requires constant movement mirroring the opponent — high intensity
        DefenderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl DefenderGuardingState {
    /// Find the best opponent to guard — focus on attackers without the ball
    /// who are trying to find space near our goal
    fn find_guard_target(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let own_goal = ctx.ball().direction_to_own_goal();
        let ball_position = ctx.tick_context.positions.ball.position;

        let mut best_target: Option<MatchPlayerLite> = None;
        let mut best_score = f32::MIN;

        for opponent in ctx.players().opponents().nearby(MAX_GUARD_RANGE) {
            // Skip the ball carrier — that's for pressing/tackling
            if opponent.has_ball(ctx) {
                continue;
            }

            let mut score = 0.0;

            // Factor 1: Proximity to our goal (closer = more dangerous to leave open)
            let dist_to_goal = (opponent.position - own_goal).magnitude();
            score += (400.0 - dist_to_goal.min(400.0)) / 8.0; // Max 50 points

            // Factor 2: Proximity to ball (could receive a pass)
            let dist_to_ball = (opponent.position - ball_position).magnitude();
            score += (200.0 - dist_to_ball.min(200.0)) / 8.0; // Max 25 points

            // Factor 3: Movement toward our goal (trying to get open)
            let velocity = opponent.velocity(ctx);
            let speed = velocity.norm();
            if speed > 1.0 {
                let move_dir = velocity.normalize();
                let to_goal = (own_goal - opponent.position).normalize();
                let alignment = move_dir.dot(&to_goal);
                if alignment > 0.0 {
                    score += alignment * speed * 8.0; // Max ~30 points
                }
            }

            // Factor 4: Is this opponent unmarked? (no other defender nearby)
            let has_nearby_defender = ctx.players().teammates().defenders()
                .any(|def| {
                    if def.id == ctx.player.id { return false; }
                    let dist = (def.position - opponent.position).magnitude();
                    dist < 15.0
                });

            if !has_nearby_defender {
                score += 30.0; // Big bonus for unmarked attackers
            }

            // Factor 5: Closeness to this defender (prefer guarding nearby opponents)
            let dist_to_us = opponent.distance(ctx);
            score += (60.0 - dist_to_us.min(60.0)) / 3.0; // Max 20 points

            // Factor 6: Opponent attacking skill
            let player_ops = ctx.player();
            let skills = player_ops.skills(opponent.id);
            let attacking_quality = (skills.physical.pace + skills.technical.finishing
                + skills.mental.off_the_ball) / 3.0;
            score += attacking_quality / 4.0; // Max ~5 points

            if score > best_score {
                best_score = score;
                best_target = Some(opponent);
            }
        }

        best_target
    }
}
