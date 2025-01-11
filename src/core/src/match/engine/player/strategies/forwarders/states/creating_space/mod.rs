use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const CREATING_SPACE_THRESHOLD: f32 = 150.0;
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
        if self.should_dribble(ctx) {
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

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player.has_ball(ctx) && ctx
                .players()
                .opponents()
                .exists(OPPONENT_DISTANCE_THRESHOLD)
    }
}
