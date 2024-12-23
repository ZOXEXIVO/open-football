use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;
use std::sync::LazyLock;

const MAX_SHOOTING_DISTANCE: f32 = 300.0; // Maximum distance to attempt a shot
const MIN_SHOOTING_DISTANCE: f32 = 20.0; // Minimum distance to attempt a shot (e.g., edge of penalty area)

static DEFENDER_RUNNING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_running_data.json")));

#[derive(Default)]
pub struct DefenderRunningState {}

impl StateProcessingHandler for DefenderRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let distance_to_ball = ctx.ball().distance();

        if ctx.player.has_ball(ctx) {
            if self.is_in_shooting_range(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Shooting,
                ));
            }

            if self.should_clear(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }

            if self.should_pass(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Passing,
                ));
            }

        } else {
            if ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }

            if !ctx.team().is_control_ball() {
                if ctx.ball().is_towards_player_with_angle(0.9) && distance_to_ball < 250.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Intercepting,
                    ));
                }

                if ctx.ball().distance() < 200.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.ball().direction_to_opponent_goal(),
                slowing_distance: if ctx.player.has_ball(ctx) { 150.0 } else { 100.0 },
            }
                .calculate(ctx.player)
                .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {

    }
}

impl DefenderRunningState {
    pub fn should_clear(&self, ctx: &StateProcessingContext) -> bool {
        ctx.ball().in_own_penalty_area() && ctx.players().opponents().exists(100.0)
    }

    pub fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.players().opponents().exists(50.0) {
            return true;
        }

        let game_vision_skill = ctx.player.skills.mental.vision;
        let game_vision_threshold = 14.0; // Adjust this value based on your game balance

        if game_vision_skill >= game_vision_threshold {
            if let Some(_) = self.find_open_teammate_on_opposite_side(ctx) {
                return true;
            }
        }

        false
    }

    fn find_open_teammate_on_opposite_side(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let opposite_side_x = match ctx.player.side {
            Some(PlayerSide::Left) => field_width * 0.75,
            Some(PlayerSide::Right) => field_width * 0.25,
            None => return None,
        };

        let mut open_teammates: Vec<MatchPlayerLite> = ctx
            .players()
            .teammates()
            .nearby(200.0)
            .filter(|teammate| {
                let is_on_opposite_side = match ctx.player.side {
                    Some(PlayerSide::Left) => teammate.position.x > opposite_side_x,
                    Some(PlayerSide::Right) => teammate.position.x < opposite_side_x,
                    None => false,
                };
                let is_open = !ctx.players().opponents().nearby(20.0).any(|opponent| opponent.id == teammate.id);
                is_on_opposite_side && is_open
            })
            .collect();

        if open_teammates.is_empty() {
            None
        } else {
            open_teammates.sort_by(|a, b| {
                let dist_a = (a.position - player_position).magnitude();
                let dist_b = (b.position - player_position).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });
            Some(open_teammates[0])
        }
    }

    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        distance_to_goal <= MAX_SHOOTING_DISTANCE && distance_to_goal >= MIN_SHOOTING_DISTANCE
    }
}
