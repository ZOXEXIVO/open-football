use std::sync::LazyLock;

use nalgebra::Vector3;

use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, VectorExtensions,
};

static DEFENDER_STANDING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_standing_data.json")));

const INTERCEPTION_DISTANCE: f32 = 100.0;
const CLEARING_DISTANCE: f32 = 50.0;
const TIRED_THRESHOLD: f32 = 30.0;
const STANDING_TIME_LIMIT: u64 = 300;
const WALK_DISTANCE_THRESHOLD: f32 = 15.0;
const MARKING_DISTANCE: f32 = 15.0;
const PRESSING_DISTANCE: f32 = 150.0;
const FIELD_THIRD_THRESHOLD: f32 = 0.33; // One-third of the field width

#[derive(Default)]
pub struct DefenderStandingState {}

impl StateProcessingHandler for DefenderStandingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_ops = ctx.ball();
        let team_ops = ctx.team();
        let player_ops = ctx.player();

        if ball_ops.distance() < 200.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Tackling,
            ));
        }

        if !team_ops.is_control_ball() {
            if ball_ops.is_towards_player_with_angle(0.8) && ball_ops.distance() < 250.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Intercepting,
                ));
            }
        }

        if ball_ops.on_own_side() {
            // Ball is on the defender's side
            if ball_ops.is_towards_player() && !ctx.team().is_control_ball() {
                if ball_ops.distance() < INTERCEPTION_DISTANCE {
                    // Move to intercept only if ball is moving slowly or player is close
                    if ball_ops.speed() < 20.0 || player_ops.distance_from_start_position() < 10.0 {
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Intercepting,
                        ));
                    }
                }

                // Track back if far from position and ball moving fast
                if player_ops.distance_from_start_position() > 20.0 && ball_ops.speed() > 20.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::TrackingBack,
                    ));
                }
            } else {
                // Ball is not towards the player
                if let Some(opponent) = ctx.players().opponents().nearby(PRESSING_DISTANCE).next() {
                    if opponent.has_ball(ctx)
                        && opponent.position.distance_to(&ctx.player.position) < PRESSING_DISTANCE
                    {
                        // Only press if opponent has ball and is very close
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Pressing,
                        ));
                    } else if opponent.position.distance_to(&ctx.player.position) < MARKING_DISTANCE
                    {
                        // Mark nearby opponents
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Marking,
                        ));
                    }
                }
            }
        }
        // Ball is on the attacking side
        else {
            // Implement more sophisticated behavior when the ball is on the attacking side
            if self.should_push_up(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::PushingUp,
                ));
            }

            if self.should_hold_defensive_line(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }

            if self.should_cover_space(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Covering,
                ));
            }

            // Walk or hold line more readily on attacking side
            if self.should_transition_to_walking(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Walking,
                ));
            }
        }

        if ctx.in_state_time > 100 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Walking,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // Implement condition processing if needed
    }
}

impl DefenderStandingState {
    fn should_transition_to_walking(&self, ctx: &StateProcessingContext) -> bool {
        let player_ops = ctx.player();
        let ball_ops = ctx.ball();

        let is_tired = player_ops.is_tired();
        let standing_too_long = ctx.in_state_time > STANDING_TIME_LIMIT;
        let ball_far_away = ball_ops.distance() > INTERCEPTION_DISTANCE * 2.0;

        let no_immediate_threat = ctx
            .players()
            .opponents()
            .nearby(CLEARING_DISTANCE)
            .next()
            .is_some();

        let close_to_optimal_position =
            player_ops.distance_from_start_position() < WALK_DISTANCE_THRESHOLD;
        let team_in_control = ctx.team().is_control_ball();

        (is_tired || standing_too_long)
            && (ball_far_away || close_to_optimal_position)
            && no_immediate_threat
            && team_in_control
    }

    fn should_push_up(&self, ctx: &StateProcessingContext) -> bool {
        let ball_ops = ctx.ball();
        let player_ops = ctx.player();

        let ball_in_attacking_third = ball_ops.distance_to_opponent_goal()
            < ctx.context.field_size.width as f32 * FIELD_THIRD_THRESHOLD;
        let team_in_possession = ctx.team().is_control_ball();
        let defender_not_last_man = !self.is_last_defender(ctx);

        ball_in_attacking_third
            && team_in_possession
            && defender_not_last_man
            && player_ops.distance_from_start_position()
                < ctx.context.field_size.width as f32 * 0.25
    }

    fn should_hold_defensive_line(&self, ctx: &StateProcessingContext) -> bool {
        let ball_ops = ctx.ball();

        let defenders: Vec<MatchPlayerLite> = ctx.players().teammates().defenders().collect();
        let avg_defender_x =
            defenders.iter().map(|d| d.position.x).sum::<f32>() / defenders.len() as f32;

        (ctx.player.position.x - avg_defender_x).abs() < 5.0
            && ball_ops.distance() > INTERCEPTION_DISTANCE
            && !ctx.team().is_control_ball()
    }

    fn should_cover_space(&self, ctx: &StateProcessingContext) -> bool {
        let ball_ops = ctx.ball();
        let player_ops = ctx.player();

        let ball_in_middle_third = ball_ops.distance_to_opponent_goal()
            > ctx.context.field_size.width as f32 * FIELD_THIRD_THRESHOLD
            && ball_ops.distance_to_own_goal()
                > ctx.context.field_size.width as f32 * FIELD_THIRD_THRESHOLD;
        let no_immediate_threat = ctx
            .players()
            .opponents()
            .nearby(MARKING_DISTANCE)
            .next()
            .is_some();

        let not_in_optimal_position =
            player_ops.distance_from_start_position() > WALK_DISTANCE_THRESHOLD;

        ball_in_middle_third && no_immediate_threat && not_in_optimal_position
    }

    fn is_last_defender(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players()
            .teammates()
            .defenders()
            .all(|d| d.position.x >= ctx.player.position.x)
    }
}
