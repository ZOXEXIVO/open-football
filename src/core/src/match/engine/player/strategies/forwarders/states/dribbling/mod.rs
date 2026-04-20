use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct ForwardDribblingState {}

impl StateProcessingHandler for ForwardDribblingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // No opponents nearby — just run, dribbling is for beating defenders
        if !ctx.players().opponents().exists(25.0) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // PRIORITY 0: Near opponent goalkeeper - shoot
        if let Some(gk) = ctx.players().opponents().goalkeeper().next() {
            let distance_to_gk = (ctx.player.position - gk.position).magnitude();
            if distance_to_gk < 25.0 && distance_to_goal < 120.0 {
                return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
            }
        }

        // PRIORITY 1: In shooting range — shoot
        if ctx.player().shooting().in_shooting_range() {
            if ctx.player().has_clear_shot() || distance_to_goal < 60.0 {
                return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
            }
        }

        // PRIORITY 1b: xG-based fallback (should_attempt_shot already checks both cooldowns)
        if ctx.player().should_attempt_shot() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
        }

      // Prevent infinite dribbling - timeout after 40 ticks to reassess.
        // Only force a shot if we're in a genuine shooting range (~30m).
        // The previous 300-unit fallback routed dribblers into Shooting from
        // the halfway line, which then bounced back to Running without firing
        // but kept the front line churning states instead of redistributing.
        if ctx.in_state_time > 40 {
            if distance_to_goal < 60.0 {
                return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
            }
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Check if the player is under pressure from multiple defenders
        let close_defenders = ctx.players().opponents().nearby(8.0).count();
        if close_defenders >= 2 {
            // Under heavy pressure - pass instead of forcing a shot
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // No space to dribble — lay off with a pass instead. Previously
        // dropped into HoldingUpPlay (a dead state); Passing is the
        // right football call.
        if !self.has_space_to_dribble(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Passing,
            ));
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
        Some(
            SteeringBehavior::Arrive {
                target: ctx.player().opponent_goal_position(),
                slowing_distance: 150.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Dribbling is high intensity - sustained movement with ball
        ForwardCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl ForwardDribblingState {
    fn has_space_to_dribble(&self, ctx: &StateProcessingContext) -> bool {
        let dribble_distance = 15.0;

        !ctx.players().opponents().exists(dribble_distance)
    }

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
        let teammates_in_box = ctx
            .players()
            .teammates()
            .all()
            .filter(|t| (t.position - goal_pos).magnitude() < 120.0)
            .count();

        let crossing = ctx.player.skills.technical.crossing / 20.0;
        teammates_in_box >= 1 && crossing > 0.4
    }
}
