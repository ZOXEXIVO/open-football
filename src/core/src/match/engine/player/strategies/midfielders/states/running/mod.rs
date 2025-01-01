use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use crate::IntegerUtils;
use nalgebra::Vector3;
use std::sync::LazyLock;

static MIDFIELDER_RUNNING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_running_data.json")));

const MAX_SHOOTING_DISTANCE: f32 = 300.0; // Maximum distance to attempt a shot
const MIN_SHOOTING_DISTANCE: f32 = 10.0; // Minimum distance to attempt a shot (e.g., edge of penalty area)

#[derive(Default)]
pub struct MidfielderRunningState {}

impl StateProcessingHandler for MidfielderRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            if self.has_clear_shot(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }

            // If the player has the ball, consider shooting, passing, or dribbling
            if self.in_shooting_range(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::DistanceShooting,
                ));
            }

            if self.should_dribble(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Dribbling,
                ));
            }

            if self.is_under_pressure(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
        } else {
            if ctx.ball().distance() < 200.0 && ctx.ball().is_towards_player_with_angle(0.8) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }

            // If the player doesn't have the ball, check if they should press, support attack, or return
            if self.should_press(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            if self.should_support_attack(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::AttackSupporting,
                ));
            }

            if self.should_return_to_position(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            if self.is_under_pressure(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let separation_velocity = ctx.player().separation_velocity();

        if let Some(target_position) = self.find_space_between_opponents(ctx) {
            Some(
                SteeringBehavior::Arrive {
                    target: target_position + separation_velocity,
                    slowing_distance: 10.0,
                }
                .calculate(ctx.player)
                .velocity,
            )
        } else if ctx.player.has_ball(ctx) {
            Some(
                SteeringBehavior::Arrive {
                    target: ctx.ball().direction_to_opponent_goal() + separation_velocity,
                    slowing_distance: 100.0,
                }
                .calculate(ctx.player)
                .velocity,
            )
        } else if ctx.team().is_control_ball() {
            Some(
                SteeringBehavior::Arrive {
                    target: ctx.ball().direction_to_opponent_goal() + separation_velocity,
                    slowing_distance: 100.0,
                }
                .calculate(ctx.player)
                .velocity,
            )
        } else {
            Some(
                SteeringBehavior::Wander {
                    target: ctx.player.start_position + separation_velocity,
                    radius: IntegerUtils::random(5, 150) as f32,
                    jitter: IntegerUtils::random(0, 2) as f32,
                    distance: IntegerUtils::random(10, 150) as f32,
                    angle: IntegerUtils::random(0, 360) as f32,
                }
                .calculate(ctx.player)
                .velocity,
            )
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderRunningState {
    fn in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        (MIN_SHOOTING_DISTANCE..=MAX_SHOOTING_DISTANCE)
            .contains(&ctx.ball().distance_to_opponent_goal())
    }

    fn has_clear_shot(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.ball().distance_to_opponent_goal() < MAX_SHOOTING_DISTANCE {
            return ctx.player().has_clear_shot();
        }

        false
    }

    fn find_open_teammate<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<u32> {
        let open_teammates = ctx
            .players()
            .opponents()
            .all()
            .min_by(|a, b| {
                // Prefer teammates closer to the opponent's goal
                let a_distance = (a.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                let b_distance = (b.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                a_distance.partial_cmp(&b_distance).unwrap()
            })
            .map(|p| p.id);

        open_teammates
    }

    fn should_press(&self, ctx: &StateProcessingContext) -> bool {
        let pressing_distance = 100.0;

        !ctx.team().is_control_ball()
            && ctx.ball().distance() < pressing_distance
            && ctx.ball().is_towards_player_with_angle(0.8)
    }

    fn find_space_between_opponents(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let players = ctx.players();
        let opponents = players.opponents();

        let mut nearest_opponents = opponents.nearby_raw(200.0);

        if let Some((first_id, _)) = nearest_opponents.next() {
            while let Some((second_id, _)) = nearest_opponents.next() {
                if first_id == second_id {
                    continue;
                }
                let distance_between_opponents =
                    ctx.tick_context.distances.get(first_id, second_id);
                if distance_between_opponents > 10.0 {
                    let first_position = ctx.tick_context.positions.players.position(first_id);
                    let second_position = ctx.tick_context.positions.players.position(second_id);

                    let midpoint = (first_position + second_position) * 0.5;

                    return Some(midpoint);
                }
            }
        }

        None
    }

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        // Check if there is space to dribble and no immediate pressure from opponents
        let space_ahead = self.space_ahead(ctx);
        let under_pressure = self.is_under_pressure(ctx);

        space_ahead && !under_pressure
    }

    fn should_support_attack(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the team is in possession and the player is in a good position to support the attack
        let team_in_possession = ctx.team().is_control_ball();
        let in_attacking_half = ctx.player.position.x > ctx.context.field_size.width as f32 / 2.0;

        team_in_possession && in_attacking_half && ctx.ball().distance() < 200.0
    }

    fn should_return_to_position(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the player is far from their starting position and the team is not in possession
        let distance_from_start = ctx.player().distance_from_start_position();
        let team_in_possession = ctx.team().is_control_ball();

        distance_from_start > 20.0 && !team_in_possession
    }

    fn space_ahead(&self, ctx: &StateProcessingContext) -> bool {
        // Check if there is open space ahead of the player
        let space_threshold = 10.0;
        let player_direction = ctx.player.velocity.normalize();
        let space_ahead = ctx.tick_context.space.cast_ray(
            ctx.player.position,
            player_direction,
            space_threshold,
            true,
        );

        space_ahead.is_none()
    }

    fn is_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(25.0)
    }
}
