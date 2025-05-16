use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct MidfielderPressingState {}

impl StateProcessingHandler for MidfielderPressingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.in_state_time > 60 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        if ctx.ball().distance() < 15.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Tackling,
            ));
        }

        // If ball is far away or team has possession, stop pressing
        if ctx.ball().distance() > 100.0 || ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // If team has possession, contribute to attack
        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::AttackSupporting,
            ));
        }

        // Check if the pressing is ineffective (opponent still has ball after some time)
        if ctx.in_state_time > 30 && !self.is_making_progress(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Pursuit {
                target: ctx.tick_context.positions.ball.position,
            }
            .calculate(ctx.player)
            .velocity
                + ctx.player().separation_velocity(),
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions to process in this state
    }
}

impl MidfielderPressingState {
    // New helper function to determine if pressing is making progress
    fn is_making_progress(&self, ctx: &StateProcessingContext) -> bool {
        let player_velocity = ctx.player.velocity;

        // Calculate dot product between player velocity and direction to ball
        let to_ball = ctx.tick_context.positions.ball.position - ctx.player.position;
        let to_ball_normalized = if to_ball.magnitude() > 0.0 {
            to_ball / to_ball.magnitude()
        } else {
            Vector3::new(0.0, 0.0, 0.0)
        };

        let moving_toward_ball = player_velocity.dot(&to_ball_normalized) > 0.0;

        // Check if other teammates are better positioned to press
        let other_pressing_teammates = ctx.players().teammates().all()
            .filter(|t| {
                let dist = (t.position - ctx.tick_context.positions.ball.position).magnitude();
                dist < ctx.ball().distance() * 0.8 // 20% closer than current player
            })
            .count();

        // Continue pressing if moving toward ball and not many better-positioned teammates
        moving_toward_ball && other_pressing_teammates < 2
    }
}