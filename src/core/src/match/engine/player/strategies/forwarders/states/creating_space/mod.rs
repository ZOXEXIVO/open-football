use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

static FORWARD_CREATING_SPACE_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_creating_space_data.json")));

const CREATING_SPACE_THRESHOLD: f32 = 50.0;
const OPPONENT_DISTANCE_THRESHOLD: f32 = 20.0;

#[derive(Default)]
pub struct ForwardCreatingSpaceState {}

impl StateProcessingHandler for ForwardCreatingSpaceState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Check if the player has created enough space
        if self.has_created_space(ctx) {
            // If space is created, transition to the assisting state
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Assisting,
            ));
        }

        // Check if the player is too close to an opponent
        if self.is_too_close_to_opponent(ctx) {
            // If too close to an opponent, try to dribble away
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let direction = {
            // if let Some(empty_zone) = self.find_empty_zone_between_opponents(ctx) {
            //     return Some(empty_zone);
            // }

            ctx.ball().direction_to_opponent_goal()
        };

        return Some(
            SteeringBehavior::Arrive {
                target: direction,
                slowing_distance: 50.0,
            }
                .calculate(ctx.player)
                .velocity,
        );
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No specific conditions to process
    }
}

impl ForwardCreatingSpaceState {
    fn has_created_space(&self, ctx: &StateProcessingContext) -> bool {
        !ctx.players().opponents().exists(CREATING_SPACE_THRESHOLD)
    }

    fn is_too_close_to_opponent(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players()
            .opponents()
            .exists(OPPONENT_DISTANCE_THRESHOLD)
    }

    fn has_space_between_opponents(&self, ctx: &StateProcessingContext) -> bool {
        let players = ctx.players();
        let opponents = players.opponents();

        let mut opponents_all = opponents.all();

        if let Some(first) = opponents_all.next() {
            if let Some(second) = opponents_all.next() {
                let opponent1_position = first.position;
                let opponent2_position = second.position;

                let distance_between_opponents =
                    (opponent1_position - opponent2_position).magnitude();

                return distance_between_opponents > CREATING_SPACE_THRESHOLD;
            }
        }

        false
    }

    fn find_empty_zone_between_opponents(
        &self,
        ctx: &StateProcessingContext,
    ) -> Option<Vector3<f32>> {
        let players = ctx.players();
        let opponents = players.opponents();

        let mut opponents_positions: Vec<Vector3<f32>> =
            opponents.all().map(|opponent| opponent.position).collect();
        opponents_positions.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());

        let mut largest_gap = 0.0;
        let mut largest_gap_position = None;

        for window in opponents_positions.windows(2) {
            let gap = (window[1] - window[0]).magnitude();
            if gap > largest_gap {
                largest_gap = gap;
                largest_gap_position = Some((window[0] + window[1]) / 2.0);
            }
        }

        if let Some(position) = largest_gap_position {
            // Clamp the position within the field boundaries
            let field_width = ctx.context.field_size.width as f32;
            let field_height = ctx.context.field_size.height as f32;
            let clamped_position = Vector3::new(
                position.x.clamp(0.0, field_width),
                position.y.clamp(0.0, field_height),
                0.0,
            );
            Some(clamped_position)
        } else {
            None
        }
    }
}
