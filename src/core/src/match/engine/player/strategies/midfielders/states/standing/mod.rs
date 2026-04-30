use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const PRESSING_DISTANCE_THRESHOLD: f32 = 80.0; // Midfielders press from further out

#[derive(Default, Clone)]
pub struct MidfielderStandingState {}

impl StateProcessingHandler for MidfielderStandingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Offside discipline — attack-minded midfielders (AM, wingers)
        // can drift beyond the opposing defensive line. If our team
        // doesn't have the ball, drop back to Returning or any pass
        // upfield will catch us offside.
        if !ctx.player.has_ball(ctx) && ctx.player().defensive().is_stranded_offside() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        if ctx.player.has_ball(ctx) {
            // Go directly to Passing state — it has the best pass evaluation logic
            // Only hold possession if under no pressure and no teammates nearby
            // With the ball and no passing options, stay in Standing —
            // the top-of-function idle logic will refresh next tick.
            // HoldingPossession did nothing extra beyond that.
            return if self.has_passing_options(ctx) {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ))
            } else {
                None
            };
        } else {
            // Loose-ball claim lives in the dispatcher.

            if ctx.team().is_control_ball() {
                // If teammates are clustered nearby, create space instead of running
                let nearby_teammates = ctx.players().teammates().nearby(25.0).count();
                if nearby_teammates >= 2 && ctx.ball().distance() > 30.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::CreatingSpace,
                    ));
                }
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Running,
                ));
            } else {
                // Only press/tackle if an OPPONENT has the ball AND we're the best chaser
                if let Some(_opponent) = ctx.players().opponents().with_ball().next() {
                    if ctx.ball().distance() < PRESSING_DISTANCE_THRESHOLD
                        && ctx.team().is_best_player_to_chase_ball()
                    {
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::Pressing,
                        ));
                    }

                    // Second closest can press from very short range only
                    if ctx.ball().distance() < 20.0 {
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::Pressing,
                        ));
                    }
                }

                // Ball in flight (clearance, long pass) — go contest the
                // landing zone. Without this, clearances to midfield always
                // end up at the opposing team's feet because we only
                // intercept when the ball is already headed directly at
                // us. Midfielders are the default contester of loose
                // balls in the middle third; the predicted landing
                // position gives them a runway to reach it.
                if !ctx.ball().is_owned() && ctx.ball().is_in_flight() {
                    let landing = ctx.tick_context.positions.ball.landing_position;
                    let dist_to_landing = (landing - ctx.player.position).magnitude();
                    if dist_to_landing < 100.0 {
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::Intercepting,
                        ));
                    }
                }

                // Loose + heading toward us — stay with the original tight
                // trigger (angle gate filters passes that aren't coming
                // our way).
                if !ctx.ball().is_owned()
                    && ctx.ball().distance() < 250.0
                    && ctx.ball().is_towards_player_with_angle(0.8)
                {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Intercepting,
                    ));
                }

                // Guard unmarked attackers on our side
                if ctx.ball().on_own_side() {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Guarding,
                    ));
                }
            }
        }

        // Only press if opponent is nearby AND has the ball AND we're closest
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            if opponent.distance(ctx) < PRESSING_DISTANCE_THRESHOLD
                && ctx.team().is_best_player_to_chase_ball()
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
        }

        // Check if a teammate is making a run and needs support
        if self.should_support_attack(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::AttackSupporting,
            ));
        }

        // Midfielders should not stand still for long — get moving quickly
        if ctx.in_state_time > 8 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Standing = completely still. No separation, no drift.
        // Midfielders transition out of Standing within 8 ticks anyway.
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Standing is recovery - minimal movement
        MidfielderCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}

impl MidfielderStandingState {
    /// Determines if the midfielder has passing options.
    fn has_passing_options(&self, ctx: &StateProcessingContext) -> bool {
        const PASSING_DISTANCE_THRESHOLD: f32 = 30.0;
        ctx.players().teammates().exists(PASSING_DISTANCE_THRESHOLD)
    }

    /// Checks if an opponent player is nearby within the pressing threshold.
    #[allow(dead_code)]
    fn is_opponent_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players()
            .opponents()
            .exists(PRESSING_DISTANCE_THRESHOLD)
    }

    /// Determines if the midfielder should support an attacking play.
    fn should_support_attack(&self, ctx: &StateProcessingContext) -> bool {
        // For simplicity, assume the midfielder supports the attack if the ball is in the attacking third
        let field_length = ctx.context.field_size.width as f32;
        let attacking_third_start = if ctx.player.side == Some(PlayerSide::Left) {
            field_length * (2.0 / 3.0)
        } else {
            field_length / 3.0
        };

        let ball_position_x = ctx.tick_context.positions.ball.position.x;

        if ctx.player.side == Some(PlayerSide::Left) {
            ball_position_x > attacking_third_start
        } else {
            ball_position_x < attacking_third_start
        }
    }
}
