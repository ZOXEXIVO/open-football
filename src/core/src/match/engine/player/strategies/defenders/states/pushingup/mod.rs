use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const TACKLING_DISTANCE_THRESHOLD: f32 = 2.0;
const PRESSING_DISTANCE_THRESHOLD: f32 = 20.0;
const STAMINA_THRESHOLD: f32 = 30.0;
const FIELD_THIRD_THRESHOLD: f32 = 0.33;
const MAX_PUSH_UP_DISTANCE: f32 = 0.7;
const PUSH_UP_HYSTERESIS: f32 = 0.05; // Hysteresis to prevent rapid state changes

#[derive(Default, Clone)]
pub struct DefenderPushingUpState {}

impl StateProcessingHandler for DefenderPushingUpState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_ops = ctx.ball();

        if ball_ops.on_own_side() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TrackingBack,
            ));
        }

        if !ctx.team().is_control_ball() {
            if let Some(opponent) = ctx.players().opponents().nearby(TACKLING_DISTANCE_THRESHOLD).next() {
                let distance_to_opponent = ctx
                    .tick_context
                    .grid
                    .get(opponent.id, ctx.player.id);

                if distance_to_opponent <= TACKLING_DISTANCE_THRESHOLD {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }

                if distance_to_opponent <= PRESSING_DISTANCE_THRESHOLD
                    && ctx.player.skills.physical.stamina > STAMINA_THRESHOLD
                {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
            }

            // Instead of immediately switching to Covering, introduce a transition state
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Covering,
            ));
        }

        if self.should_retreat(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TrackingBack,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // OVERLAPPING RUN: If wide defender with teammate on ball on same flank,
        // sprint ahead of ball carrier along touchline
        if self.is_overlap_run(ctx) {
            let target = self.calculate_overlap_target(ctx);
            let acceleration = ctx.player.skills.physical.acceleration / 20.0;
            return Some(
                SteeringBehavior::Pursuit {
                    target,
                    target_velocity: Vector3::zeros(),
                }
                    .calculate(ctx.player)
                    .velocity * (1.0 + acceleration * 0.3), // Sprint bonus
            );
        }

        let optimal_position = self.calculate_optimal_pushing_up_position(ctx);

        Some(
            SteeringBehavior::Pursuit {
                target: optimal_position,
                target_velocity: Vector3::zeros(), // Static target position
            }
                .calculate(ctx.player)
                .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Pushing up involves moving forward with the team - moderate intensity
        DefenderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}

impl DefenderPushingUpState {
    /// Check if this push-up is an overlapping run (wide defender, ball on same flank)
    fn is_overlap_run(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let start_y = ctx.player.start_position.y;
        let is_wide = start_y < field_height * 0.25 || start_y > field_height * 0.75;
        if !is_wide {
            return false;
        }

        // Team must have ball on same flank
        if !ctx.team().is_control_ball() {
            return false;
        }
        let ball_pos = ctx.tick_context.positions.ball.position;
        let player_on_left = start_y < field_height * 0.5;
        let ball_on_left = ball_pos.y < field_height * 0.5;

        player_on_left == ball_on_left
    }

    /// Calculate overlap target: ahead of ball carrier, wide on touchline
    fn calculate_overlap_target(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_height = ctx.context.field_size.height as f32;
        let field_width = ctx.context.field_size.width as f32;
        let ball_pos = ctx.tick_context.positions.ball.position;
        let is_left = ctx.player.side == Some(crate::r#match::player::PlayerSide::Left);

        // Target is ahead of ball position, on the touchline
        let wing_y = if ctx.player.start_position.y < field_height * 0.5 {
            field_height * 0.08 // Left touchline
        } else {
            field_height * 0.92 // Right touchline
        };

        // Push ahead of ball carrier toward opponent goal
        let ahead_x = if is_left {
            (ball_pos.x + 60.0).clamp(0.0, field_width * 0.75)
        } else {
            (ball_pos.x - 60.0).clamp(field_width * 0.25, field_width)
        };

        Vector3::new(ahead_x, wing_y, 0.0)
    }

    fn should_retreat(&self, ctx: &StateProcessingContext) -> bool {
        let field_width = ctx.context.field_size.width as f32;
        let is_left = ctx.player.side == Some(crate::r#match::player::PlayerSide::Left);

        if is_left {
            // Left team pushes right: retreat if past max push-up line
            let max_push_up_x = field_width * (MAX_PUSH_UP_DISTANCE + PUSH_UP_HYSTERESIS);
            ctx.player.position.x > max_push_up_x || self.is_last_defender(ctx)
        } else {
            // Right team pushes left: retreat if past max push-up line (from right side)
            let min_push_up_x = field_width * (1.0 - MAX_PUSH_UP_DISTANCE - PUSH_UP_HYSTERESIS);
            ctx.player.position.x < min_push_up_x || self.is_last_defender(ctx)
        }
    }

    fn is_last_defender(&self, ctx: &StateProcessingContext) -> bool {
        let is_left = ctx.player.side == Some(crate::r#match::player::PlayerSide::Left);

        if is_left {
            // Left team: last defender is the one furthest back (smallest x)
            ctx.players()
                .teammates()
                .defenders()
                .all(|d| d.position.x >= ctx.player.position.x)
        } else {
            // Right team: last defender is the one furthest back (largest x)
            ctx.players()
                .teammates()
                .defenders()
                .all(|d| d.position.x <= ctx.player.position.x)
        }
    }

    fn calculate_optimal_pushing_up_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let is_left = ctx.player.side == Some(crate::r#match::player::PlayerSide::Left);

        let (attacking_third_x, mid_x, clamp_min, clamp_max) = if is_left {
            (
                field_width * (1.0 - FIELD_THIRD_THRESHOLD / 2.0),
                field_width * 0.5,
                field_width * 0.5,
                field_width * MAX_PUSH_UP_DISTANCE,
            )
        } else {
            (
                field_width * (FIELD_THIRD_THRESHOLD / 2.0),
                field_width * 0.5,
                field_width * (1.0 - MAX_PUSH_UP_DISTANCE),
                field_width * 0.5,
            )
        };

        let attacking_third_center = Vector3::new(attacking_third_x, field_height * 0.5, 0.0);

        let teammates = ctx.players().teammates();

        let attacking_teammates: Vec<_> = teammates.all().into_iter()
            .filter(|p| {
                if is_left { p.position.x > mid_x } else { p.position.x < mid_x }
            })
            .collect();

        let avg_attacking_position = if !attacking_teammates.is_empty() {
            attacking_teammates
                .iter()
                .fold(Vector3::zeros(), |acc, p| acc + p.position)
                / attacking_teammates.len() as f32
        } else {
            attacking_third_center
        };

        let support_position = (ball_position + avg_attacking_position) * 0.5;

        let optimal_position =
            support_position * 0.5 + attacking_third_center * 0.3 + player_position * 0.2;

        Vector3::new(
            optimal_position.x.clamp(clamp_min, clamp_max),
            optimal_position.y.clamp(0.0, field_height),
            0.0,
        )
    }
}
