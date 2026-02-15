use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardDribblingState {}

impl StateProcessingHandler for ForwardDribblingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            // Transition to Running state if the player doesn't have the ball
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // PRIORITY 0: Near opponent goalkeeper - MUST shoot immediately
        if let Some(gk) = ctx.players().opponents().goalkeeper().next() {
            let distance_to_gk = (ctx.player.position - gk.position).magnitude();
            if distance_to_gk < 25.0 && distance_to_goal < 120.0 {
                return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
            }
        }

        // PRIORITY 1: Very close to goal - ALWAYS try to shoot (inside 6-yard box)
        if distance_to_goal < 60.0 {
            return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
        }

        // PRIORITY 2: Inside penalty area - shoot aggressively
        if distance_to_goal < 165.0 {
            let finishing = ctx.player.skills.technical.finishing;
            let has_clear_shot = ctx.player().has_clear_shot();
            let close_blockers = ctx.players().opponents().nearby(5.0).count();

            // Shoot if clear shot OR good finishing skill with minimal blocking
            if has_clear_shot || (finishing > 12.0 && close_blockers <= 1) {
                return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
            }
        }

        // PRIORITY 3: Edge of box - shoot with good skills
        if distance_to_goal < 250.0 && ctx.player().has_clear_shot() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
        }

        // Prevent infinite dribbling - timeout after 40 ticks to reassess
        if ctx.in_state_time > 40 {
            // Check for shooting opportunity first - be aggressive
            if distance_to_goal < 300.0 {
                let finishing = ctx.player.skills.technical.finishing;
                let close_blockers = ctx.players().opponents().nearby(6.0).count();
                if ctx.player().has_clear_shot() || (finishing > 13.0 && close_blockers <= 1) {
                    return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
                }
            }
            // Otherwise try passing
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Check if the player is under pressure from multiple defenders
        let close_defenders = ctx.players().opponents().nearby(8.0).count();
        if close_defenders >= 2 {
            // Under heavy pressure in dangerous area - try to shoot anyway
            if distance_to_goal < 200.0 {
                let finishing = ctx.player.skills.technical.finishing;
                if finishing > 11.0 {
                    return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
                }
            }
            // Transition to Passing state if under pressure from multiple close defenders
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Check if there's space to dribble forward
        if !self.has_space_to_dribble(ctx) {
            // In dangerous area with no space - try shooting
            if distance_to_goal < 250.0 {
                return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
            }
            // Transition to HoldingUpPlay state if there's no space to dribble
            return Some(StateChangeResult::with_forward_state(
                ForwardState::HoldingUpPlay,
            ));
        }

        // Check if there's an opportunity to shoot
        if self.can_shoot(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Shooting,
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

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
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

    fn can_shoot(&self, ctx: &StateProcessingContext) -> bool {
        let shot_distance = 80.0;

        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // Check if the player is within shooting distance and has a clear shot
        distance_to_goal < shot_distance && ctx.player().has_clear_shot()
    }
}
