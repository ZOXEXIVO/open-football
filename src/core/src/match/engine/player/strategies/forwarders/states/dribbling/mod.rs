use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::shared::{
    ActivityIntensity, ForwardCondition, ForwardRead, ForwardShot,
};
use crate::r#match::player::strategies::shared::players::ops::forward_shot_decision::BallCarry;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct ForwardDribblingState {}

impl StateProcessingHandler for ForwardDribblingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardRead::released(ctx),
            ));
        }

        // No opponents nearby — just run, dribbling is for beating defenders
        if !ctx.players().opponents().exists(25.0) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // PRIORITY 0: Near opponent goalkeeper.
        if let Some(gk) = ctx.players().opponents().goalkeeper().next() {
            let distance_to_gk = (ctx.player.position - gk.position).magnitude();
            if distance_to_gk < 25.0
                && distance_to_goal < 120.0
                && let Some(result) = ForwardShot::dispatch(ctx, "FWD_DRIB_NEAR_GK")
            {
                return Some(result);
            }
        }

        // PRIORITY 1: In shooting range with a clear lane.
        if ctx.player().shooting().in_shooting_range()
            && ctx.player().has_clear_shot()
            && let Some(result) = ForwardShot::dispatch(ctx, "FWD_DRIB_CLEAR")
        {
            return Some(result);
        }

        // (A separate "PRIORITY 1b" range fallback used to re-test the
        // IDENTICAL predicate — `should_attempt_shot()` is literally
        // `in_shooting_range()` — doubling the per-tick willingness
        // roll to 1-(1-w)^2. One look, one roll.)

        // Prevent infinite dribbling - timeout after 40 ticks to reassess.
        if ctx.in_state_time > 40 {
            if distance_to_goal < 60.0
                && ctx.player().has_clear_shot()
                && let Some(result) = ForwardShot::dispatch(ctx, "FWD_DRIB_TIMEOUT")
            {
                return Some(result);
            }
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Under REAL pressure from multiple defenders — pass.
        // The flicker bug: the old "no space to dribble" check below
        // (opponents within 15u) fired against Passing's "opponents
        // within 20u → back to Dribbling" rule, so a lone chaser at
        // 17u produced Dribbling → Passing → Dribbling every few
        // ticks. Now we require two real pressers OR a long commit
        // window before abandoning the dribble.
        let close_defenders = ctx.players().opponents().nearby(24.0).count();
        if close_defenders >= 2 {
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Only abandon dribbling for a pass when genuinely boxed in —
        // opponent within 10u AND we've been dribbling long enough to
        // commit to the decision (≥15 ticks). The old `has_space_to_dribble`
        // (15u threshold) fired too eagerly against a single chaser.
        if ctx.in_state_time >= 15 && ctx.players().opponents().nearby(10.0).next().is_some() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Cross from wide position in attacking third
        if self.should_cross(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Crossing,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Past the defender's shoulder, and no closer than a position he
        // could shoot from — see `BallCarry::target`. This used to `Arrive` at
        // the goal centre, i.e. run at the goalkeeper.
        Some(
            SteeringBehavior::Arrive {
                target: BallCarry::target(ctx),
                slowing_distance: 30.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Dribbling is high intensity - sustained movement with ball
        ForwardCondition::new(ActivityIntensity::High).process(ctx);
    }
}

impl ForwardDribblingState {
    fn should_cross(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let y = ctx.player.position.y;
        let wide_margin = field_height * 0.2;

        let is_wide = y < wide_margin || y > field_height - wide_margin;
        if !is_wide {
            return false;
        }

        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        // In attacking area but not too close to goal
        if distance_to_goal > 300.0 || distance_to_goal < 60.0 {
            return false;
        }

        // Has teammates in the box
        let goal_pos = ctx.player().opponent_goal_position();
        let teammates_in_box = ctx.players().teammates().nearby_at(goal_pos, 120.0).count();

        let crossing = ctx.player.skills.technical.crossing / 20.0;
        teammates_in_box >= 1 && crossing > 0.4
    }
}
