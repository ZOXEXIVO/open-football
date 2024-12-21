use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use rand::Rng;
use std::sync::LazyLock;

static MIDFIELDER_STANDING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_standing_data.json")));

const PASSING_DISTANCE_THRESHOLD: f32 = 30.0; // Adjust as needed
const PRESSING_DISTANCE_THRESHOLD: f32 = 50.0; // Adjust as needed

#[derive(Default)]
pub struct MidfielderStandingState {}

impl StateProcessingHandler for MidfielderStandingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            // Decide whether to hold possession or distribute the ball
            return if self.should_hold_possession(ctx) {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::HoldingPossession,
                ))
            } else {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Distributing,
                ))
            };
        }

        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }
        else {
            if ctx.ball().distance() < 150.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }

            if ctx.ball().distance() < 250.0 && ctx.ball().is_towards_player_with_angle(0.8) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }

            if ctx.ball().distance() < PRESSING_DISTANCE_THRESHOLD {
                // Transition to Tackling state to try and win the ball
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
        }

        // 3. Check if an opponent is nearby and pressing is needed
        if self.is_opponent_nearby(ctx) {
            // Transition to Pressing state to apply pressure
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        // 4. Check if a teammate is making a run and needs support
        if self.should_support_attack(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::AttackSupporting,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}

impl MidfielderStandingState {
    /// Checks if the midfielder should hold possession based on game context.
    fn should_hold_possession(&self, ctx: &StateProcessingContext) -> bool {
        // For simplicity, let's assume the midfielder holds possession if there are no immediate passing options
        !self.has_passing_options(ctx)
    }

    fn calculate_movement_probability(
        &self,
        rng: &mut impl Rng,
        ctx: &StateProcessingContext,
    ) -> bool {
        let positioning_probability = (ctx.player.skills.mental.positioning as f64) / 20.0;
        let concentration_probability = (ctx.player.skills.mental.concentration as f64) / 20.0;

        let positioning_roll = rng.gen_range(0.0..1.0);
        let concentration_roll = rng.gen_range(0.0..1.0);

        positioning_roll < positioning_probability && concentration_roll < concentration_probability
    }

    /// Determines if the midfielder has passing options.
    fn has_passing_options(&self, ctx: &StateProcessingContext) -> bool {
        const PASSING_DISTANCE_THRESHOLD: f32 = 30.0;
        ctx.players().teammates().exists(PASSING_DISTANCE_THRESHOLD)
    }

    const PRESSING_DISTANCE_THRESHOLD: f32 = 10.0;

    /// Checks if an opponent player is nearby within the pressing threshold.
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
