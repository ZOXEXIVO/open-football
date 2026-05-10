use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, PlayerDistanceFromStartPosition, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperCatchingState {}

impl StateProcessingHandler for GoalkeeperCatchingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if self.is_catch_successful(ctx) {
            let mut holding_result =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall);

            holding_result
                .events
                .add_player_event(PlayerEvent::CaughtBall(ctx.player.id));

            return Some(holding_result);
        }

        // Shot is live: stay in Catching and keep sprinting toward the
        // intercept line. The old logic exited to Standing / ComingOut
        // the moment the ball was >12u away, which meant a keeper
        // aiming for the far post gave up the instant the shot was
        // fired. With a cached shot target the keeper commits.
        if ctx.tick_context.ball.cached_shot_target.is_some() {
            return None;
        }

        // Ball is moving away from the keeper at speed — only credit
        // a parry when the ball was actually within reach (the keeper
        // got a hand to it). Otherwise the shot just missed past the
        // keeper and "parry" would falsely credit a save for a wide
        // shot the GK never touched.
        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        let ball_distance = ctx.ball().distance();
        if ball_speed > 2.0 && !ctx.ball().is_towards_player_with_angle(0.6) {
            if ctx.tick_context.ball.cached_shot_target.is_some() && ball_distance < 25.0 {
                return Some(StateChangeResult::with_goalkeeper_state_and_event(
                    GoalkeeperState::Standing,
                    Event::PlayerEvent(PlayerEvent::ParriedBall(ctx.player.id)),
                ));
            }
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // If ball is too far, decide based on distance from goal
        if ctx.ball().distance() > 12.0 {
            // If already far from goal, return rather than chasing further
            if ctx.player().distance_from_start_position() > 40.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ReturningToGoal,
                ));
            }
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ComingOut,
            ));
        }

        if ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        if ctx.in_state_time > 30 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        // Sprint reaction speed: 1.6..2.6x band, gated by explosive_mult.
        let speed_boost = (1.6 + prof.shot_stopping * 0.5 + prof.dive_reach * 0.5)
            * prof.explosive_mult;

        // Shot in flight → commit to the intercept line, don't chase
        // the current ball position (it's moving at 5.6 u/tick and
        // outrunning the keeper's pursuit steering).
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            let goal_pos = ctx.ball().direction_to_own_goal();
            let intercept = Vector3::new(goal_pos.x, target.goal_line_y, 0.0);
            return Some(
                SteeringBehavior::Arrive {
                    target: intercept,
                    slowing_distance: 2.0,
                }
                .calculate(ctx.player)
                .velocity
                    * speed_boost,
            );
        }

        let ball_distance = ctx.ball().distance();
        if ball_distance > 3.0 {
            Some(
                SteeringBehavior::Pursuit {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity
                    * speed_boost,
            )
        } else {
            Some(
                SteeringBehavior::Arrive {
                    target: ctx.tick_context.positions.ball.position,
                    slowing_distance: 1.5,
                }
                .calculate(ctx.player)
                .velocity
                    * (speed_boost * 0.8),
            )
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Catching is a moderate intensity activity requiring focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl GoalkeeperCatchingState {
    fn is_catch_successful(&self, ctx: &StateProcessingContext) -> bool {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // Shot-in-flight: judge the save from the *intercept line*, not
        // from current ball distance. A ball aimed into the corner
        // passes the GK 8-15 units wide of their current position —
        // real keepers reach 3-4 m (6-8 u) diving, so the relevant
        // metric is "how far off the line am I?", not "am I touching
        // the ball right now?".
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            // Ball over the bar — no save attempt worth making.
            if target.goal_line_z > 2.44 {
                return false;
            }
            // Effective reach in game units: weak ~14u, elite ~30u.
            let reach =
                10.0 + prof.dive_reach * 12.0 + prof.shot_stopping * 4.0;
            let lateral_error = (ctx.player.position.y - target.goal_line_y).abs();
            if lateral_error > reach {
                return false;
            }

            // Build shot difficulty in 0..1 from placement, power,
            // reaction-window, and keeper-offline factors.
            let placement = (lateral_error / reach).clamp(0.0, 1.0);
            let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
            let power = ((ball_speed - 2.0) / 6.0).clamp(0.0, 1.0);
            let lateral_factor = placement; // already a 0..1 lateral error.
            let height_factor = (target.goal_line_z / 2.44).clamp(0.0, 1.0);
            let reaction = (1.0 - prof.shot_stopping).clamp(0.0, 1.0) * 0.4;

            let shot_difficulty = (power * 0.28
                + placement * 0.24
                + lateral_factor * 0.18
                + height_factor * 0.10
                + reaction * 0.10
                + (1.0 - prof.condition_mult) * 0.10)
                .clamp(0.0, 1.0);

            // Per-shot save probability, then converted to per-tick.
            // Calibrated for ~3 ticks of approach during a save.
            let save_prob = prof.save_probability(shot_difficulty);
            let per_tick = prof.per_tick_save(save_prob, 3.0);
            return rand::random::<f32>() < per_tick;
        }

        let distance_to_ball = ctx.ball().distance();
        let max_catch_distance = prof.effective_catch_distance;
        if distance_to_ball > max_catch_distance {
            return false;
        }

        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        if ball_speed > 0.5 && !ctx.ball().is_towards_player_with_angle(0.6) {
            return false;
        }

        let ball_height = ctx.tick_context.positions.ball.position.z;
        let stretch = (distance_to_ball / max_catch_distance).clamp(0.0, 1.0);
        let power = ((ball_speed - 1.5) / 6.0).clamp(0.0, 1.0);

        // Awkward-height penalty: ground or above-head balls are harder.
        let height_pen = if (0.5..=1.8).contains(&ball_height) {
            0.0
        } else if ball_height < 0.2 {
            0.18
        } else if ball_height > 2.5 {
            0.22
        } else {
            0.06
        };

        let direction_factor = if ctx.ball().is_towards_player_with_angle(0.7) {
            0.0
        } else {
            0.18
        };

        let catch_difficulty = (power * 0.28
            + stretch * 0.22
            + height_pen * 0.18
            + direction_factor * 0.12
            + (1.0 - prof.condition_mult) * 0.10
            + prof.poor_skill_penalty * 0.10)
            .clamp(0.0, 1.0);

        let catch_prob = prof.catch_probability(catch_difficulty);
        rand::random::<f32>() < catch_prob
    }
}
