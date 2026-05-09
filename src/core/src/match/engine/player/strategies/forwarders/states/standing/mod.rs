use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::{
    ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const PRESS_DISTANCE: f32 = 20.0; // Distance within which to press opponents

#[derive(Default, Clone)]
pub struct ForwardStandingState {}

impl StateProcessingHandler for ForwardStandingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the forward still has the ball
        if ctx.player.has_ball(ctx) {
            let distance_to_goal = ctx.ball().distance_to_opponent_goal();
            // Settle before striking: 30 ticks = ~300ms. Without this the
            // forward can receive + shoot in the same half-second, which
            // turns every possession into a strike.
            let ownership_ticks = ctx.tick_context.ball.ownership_duration;
            let has_settled = ownership_ticks >= 30;
            let can_shoot = ctx.team().can_shoot() && ctx.player().can_shoot();

            // Point-blank (≤24u / ~12m) — defer to the centralised helper
            // even though the geometric trigger is generous; the helper's
            // 1v1 / sprint-balance / xG-floor gates still apply, and a
            // panicked Composure-8 striker shouldn't auto-fire just
            // because they're inside the box. Without this dispatch the
            // legacy `with_shot_reason("FWD_STAND_POINT_BLANK")` path
            // skipped every gate in `evaluate_forward_shot_decision`.
            if distance_to_goal <= 24.0 && can_shoot && ctx.player().shooting().in_shooting_range()
            {
                if let Some(result) = dispatch_shot(ctx, "FWD_STAND_POINT_BLANK") {
                    return Some(result);
                }
            }

            // Clear-shot trigger inside the standard shooting range.
            // The helper performs the skill-aware willingness roll, so
            // the bespoke `finishing*0.55 + composure*0.15 + 0.25` gate
            // here was strictly redundant and used a different curve
            // (floor 0.45, ceil 0.95) — much more aggressive than the
            // helper's 0.10..0.60 range, which is exactly the kind of
            // local override the polish pass is removing.
            if has_settled
                && can_shoot
                && distance_to_goal <= 60.0
                && ctx.player().shooting().in_shooting_range()
                && ctx.player().has_clear_shot()
            {
                if let Some(result) = dispatch_shot(ctx, "FWD_STAND_CLEAR") {
                    return Some(result);
                }
            }

            // Cooldown for medium/long range shots to prevent rapid-fire spam.
            const SHOOTING_COOLDOWN: u64 = 20;

            if has_settled
                && can_shoot
                && ctx.player().should_attempt_shot()
                && ctx.in_state_time > SHOOTING_COOLDOWN
            {
                if let Some(result) = dispatch_shot(ctx, "FWD_STAND_RANGE") {
                    return Some(result);
                }
            }

            if let Some(_) = self.find_best_teammate_to_pass(ctx) {
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }

            // If unable to shoot or pass, decide to dribble or hold position
            if self.should_dribble(ctx) {
                Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ))
            } else {
                // Transition to Running to approach goal
                Some(StateChangeResult::with_forward_state(ForwardState::Running))
            }
        } else {
            // If notified by ball system, always respond (only 1 per team gets notified)
            if ctx.ball().is_player_notified() && !ctx.ball().is_owned() {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::TakeBall,
                ));
            }

            // Loose-ball claim lives in the dispatcher.

            // Minimum time in standing state to prevent rapid state oscillation
            if ctx.in_state_time < 10 {
                return None;
            }

            // Offside discipline — if we're stranded beyond the opposing
            // defensive line while our team doesn't have the ball, drop
            // back. Otherwise we stay camped near the opponent's goal
            // and every upfield clearance finds us offside.
            if ctx.player().defensive().is_stranded_offside() {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Returning,
                ));
            }

            // If the forward doesn't have the ball, decide to move or press
            if self.should_press(ctx) {
                // Transition to Pressing state
                Some(StateChangeResult::with_forward_state(
                    ForwardState::Pressing,
                ))
            } else if ctx.ball().distance() > 200.0 && !ctx.team().is_control_ball() {
                // Ball is far and team doesn't have it - walk to conserve energy
                Some(StateChangeResult::with_forward_state(ForwardState::Walking))
            } else {
                Some(StateChangeResult::with_forward_state(ForwardState::Running))
            }
        }
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
                    .velocity
                        * 0.5, // Slower speed when standing
                );
            }
        }

        // Standing = completely still. Separation is handled by transitioning
        // to Running state, not by applying forces while stationary.
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Standing is recovery - minimal movement
        ForwardCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}

impl ForwardStandingState {
    /// Finds the best teammate to pass to based on proximity and position.
    /// Filters out close recent passers to prevent ping-pong passing.
    fn find_best_teammate_to_pass<'a>(
        &'a self,
        ctx: &'a StateProcessingContext<'a>,
    ) -> Option<u32> {
        // Look for teammates within range, but skip anyone who just passed to us
        // and is still very close (prevents ping-pong)
        for (teammate_id, distance) in ctx.players().teammates().nearby_ids(100.0) {
            let recency = ctx.ball().passer_recency_penalty(teammate_id);
            // If most recent or second-most-recent passer AND within 40 units, skip
            if recency <= 0.3 && distance < 40.0 {
                continue;
            }
            return Some(teammate_id);
        }

        None
    }

    /// Decides whether the forward should dribble based on game context.
    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        // Only dribble when there are opponents to beat nearby
        let nearby_opponents = ctx.players().opponents().exists(15.0);

        // No opponents — just run, don't dribble
        if !nearby_opponents {
            return false;
        }

        // Dribble to beat nearby defenders if skilled enough
        let dribbling_skill = ctx.player.skills.technical.dribbling / 20.0;
        dribbling_skill > 0.5
    }

    /// Decides whether the forward should press the opponent.
    fn should_press(&self, ctx: &StateProcessingContext) -> bool {
        // Only press if opponent has the ball AND is close AND we're best positioned
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let dist = opponent.distance(ctx);
            // Very close — anyone reacts
            if dist < 10.0 {
                return true;
            }
            // Otherwise only the best chaser presses
            dist < PRESS_DISTANCE && ctx.team().is_best_player_to_chase_ball()
        } else {
            false
        }
    }
}

/// Route a candidate shot through the centralised gate stack. See the
/// twin helper in forwarders/states/dribbling/mod.rs for rationale.
fn dispatch_shot(
    ctx: &StateProcessingContext,
    tag: &'static str,
) -> Option<StateChangeResult> {
    match evaluate_forward_shot_decision(ctx, tag) {
        ShotDecision::Shoot { reason } => Some(
            StateChangeResult::with_forward_state(ForwardState::Shooting)
                .with_shot_reason(reason),
        ),
        ShotDecision::Pass => Some(StateChangeResult::with_forward_state(ForwardState::Passing)),
        ShotDecision::Hold => None,
    }
}
