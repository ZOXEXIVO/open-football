use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;
use std::sync::LazyLock;

static FORWARD_RUNNING_IN_BEHIND_STATE_NETWORK: LazyLock<NeuralNetwork> = LazyLock::new(|| {
    DefaultNeuralNetworkLoader::load(include_str!("nn_running_in_behind_data.json"))
});

#[derive(Default)]
pub struct ForwardRunningInBehindState {}

impl StateProcessingHandler for ForwardRunningInBehindState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_ops = ctx.ball();
        let player_ops = ctx.player();

        if ctx.player.has_ball(ctx) {
            // Transition to Dribbling or Shooting based on position
            return if ball_ops.distance_to_opponent_goal() < 25.0 {
                Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ))
            } else {
                Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ))
            };
        }

        // Check if the player is offside
        if !player_ops.on_own_side() {
            // Transition to Standing state when offside
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Standing,
            ));
        }

        // Check if the run is still viable
        if !self.is_run_viable(ctx) {
            // If the run is no longer viable, transition to Creating Space
            return Some(StateChangeResult::with_forward_state(
                ForwardState::CreatingSpace,
            ));
        }

        // Check if there's an opportunity to break the offside trap
        if self.can_break_offside_trap(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::OffsideTrapBreaking,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl ForwardRunningInBehindState {
    fn is_run_viable(&self, ctx: &StateProcessingContext) -> bool {
        // Check if there's still space to run into
        let space_ahead = self.space_ahead(ctx);

        // Check if the player is still in a good position to receive a pass
        let in_passing_lane = self.in_passing_lane(ctx);

        // Check if the player has the stamina to continue the run
        let has_stamina = !ctx.player().is_tired();

        space_ahead && in_passing_lane && has_stamina
    }

    fn space_ahead(&self, ctx: &StateProcessingContext) -> bool {
        let space_threshold = 10.0;
        let players = ctx.players();
        let opponents = players.opponents();

        !opponents.exists(space_threshold)
    }

    fn in_passing_lane(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the player is in a good position to receive a pass
        // This is a simplified version and may need to be more complex in practice
        let teammate_with_ball = ctx
            .tick_context
            .positions
            .players
            .items
            .iter()
            .find(|p| {
                p.side == ctx.player.side.unwrap() && ctx.ball().owner_id() == Some(p.player_id)
            });

        if let Some(teammate) = teammate_with_ball {
            let direction_to_player = (ctx.player.position - teammate.position).normalize();
            let direction_to_goal =
                (ctx.ball().direction_to_own_goal() - teammate.position).normalize();

            // Check if the player is running towards the goal
            direction_to_player.dot(&direction_to_goal) > 0.7
        } else {
            false
        }
    }

    fn can_break_offside_trap(&self, ctx: &StateProcessingContext) -> bool {
        let player_ops = ctx.player();
        let ball_ops = ctx.ball();

        // Check if the player is currently offside
        if player_ops.on_own_side() {
            return false;
        }

        // Check if the ball is moving towards the player
        if !ball_ops.is_towards_player() {
            return false;
        }

        // Check if the player has enough space to run into
        if !self.space_ahead(ctx) {
            return false;
        }

        // Check if the player has the speed to break the offside trap
        let player_speed = ctx.player.skills.physical.acceleration;
        let speed_threshold = 80.0; // Adjust based on your game's balance
        if player_speed < speed_threshold {
            return false;
        }

        // Check if the player's team is losing
        if !ctx.team().is_loosing() {
            return false;
        }

        // If all conditions are met, the player can break the offside trap
        true
    }
}
