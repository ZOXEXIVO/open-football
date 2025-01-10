use crate::r#match::defenders::states::DefenderState;
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

#[derive(Default)]
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
                    .distances
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
        let optimal_position = self.calculate_optimal_pushing_up_position(ctx);

        Some(
            SteeringBehavior::Pursuit {
                target: optimal_position
            }
                .calculate(ctx.player)
                .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl DefenderPushingUpState {
    fn should_retreat(&self, ctx: &StateProcessingContext) -> bool {
        let field_width = ctx.context.field_size.width as f32;
        let max_push_up_x = field_width * (MAX_PUSH_UP_DISTANCE + PUSH_UP_HYSTERESIS);

        ctx.player.position.x > max_push_up_x || self.is_last_defender(ctx)
    }

    fn is_last_defender(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players()
            .teammates()
            .defenders()
            .all(|d| d.position.x <= ctx.player.position.x)
    }

    fn calculate_optimal_pushing_up_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        let attacking_third_center = Vector3::new(
            field_width * (1.0 - FIELD_THIRD_THRESHOLD / 2.0),
            field_height * 0.5,
            0.0,
        );

        let teammates = ctx.players().teammates();

        let attacking_teammates = teammates.all().into_iter()
            .filter(|p| p.position.x > field_width * 0.5)
            .collect::<Vec<_>>();

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
            (support_position * 0.5 + attacking_third_center * 0.3 + player_position * 0.2)
                .cap_magnitude(field_width * MAX_PUSH_UP_DISTANCE);

        Vector3::new(
            optimal_position
                .x
                .clamp(field_width * 0.5, field_width * MAX_PUSH_UP_DISTANCE),
            optimal_position.y.clamp(0.0, field_height),
            0.0,
        )
    }
}
