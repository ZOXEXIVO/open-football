use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::strategies::common::players::MatchPlayerIteratorExt;
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

        // Early exit if a teammate is significantly closer to avoid circular running
        let ball_distance = ctx.ball().distance();
        let ball_position = ctx.tick_context.positions.ball.position;

        if let Some(closest_teammate) = ctx.players().teammates().all()
            .filter(|t| t.id != ctx.player.id)
            .min_by(|a, b| {
                let dist_a = (a.position - ball_position).magnitude();
                let dist_b = (b.position - ball_position).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            })
        {
            let teammate_distance = (closest_teammate.position - ball_position).magnitude();

            // If teammate is closer by 10+ units, give up pressing
            if teammate_distance < ball_distance - 10.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Running,
                ));
            }
        }

        // Loose ball nearby — go claim it directly instead of pressing thin air
        if !ctx.ball().is_owned() && ball_distance < 50.0 && ctx.ball().speed() < 3.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::TakeBall,
            ));
        }

        // CRITICAL: Tackle if an opponent has the ball nearby
        // Using new chaining syntax: nearby(30.0).with_ball(ctx)
        if let Some(opponent) = ctx.players().opponents().nearby(30.0).with_ball(ctx).next() {
            let opponent_distance = (opponent.position - ctx.player.position).magnitude();

            // This prevents the midfielder from just circling without tackling
            if opponent_distance < 30.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }
        }

        // If ball is far away, stop pressing
        // Don't exit pressing just because ball is stationary (speed < 0.5 makes is_towards return false)
        if ctx.ball().distance() > 250.0 || (ctx.ball().speed() > 0.5 && !ctx.ball().is_towards_player_with_angle(0.8)) {
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
        // Only pursue if opponent has the ball
        if let Some(_opponent) = ctx.players().opponents().nearby(500.0).with_ball(ctx).next() {
            // Pursue the ball (which is with the opponent)
            Some(
                SteeringBehavior::Pursuit {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity
                    + ctx.player().separation_velocity(),
            )
        } else if !ctx.ball().is_owned() && ctx.ball().distance() < 80.0 {
            // Loose ball — pursue it
            Some(
                SteeringBehavior::Pursuit {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity
                    + ctx.player().separation_velocity(),
            )
        } else {
            // Teammate has ball — maintain position
            Some(Vector3::zeros())
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Pressing is high intensity - sustained running and pressure
        MidfielderCondition::with_velocity(ActivityIntensity::High).process(ctx);
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