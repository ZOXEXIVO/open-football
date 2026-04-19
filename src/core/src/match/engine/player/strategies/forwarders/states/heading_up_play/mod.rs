use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct ForwardHeadingUpPlayState {}

impl StateProcessingHandler for ForwardHeadingUpPlayState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the player has the ball
        if !ctx.player.has_ball(ctx) {
            // Transition to Running state if the player doesn't have the ball
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Check if there's support from teammates
        if !self.has_support(ctx) {
            // Transition to Dribbling state if there's no support
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        None
    }


    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Instead of standing completely still, shield the ball with subtle movement
        // Move away from nearest defender to protect possession
        if let Some(nearest_opponent) = ctx.players().opponents().nearby(10.0).next() {
            let away_from_opponent = (ctx.player.position - nearest_opponent.position).normalize();
            // Slow, controlled movement to shield the ball (like a real forward holding up play)
            return Some(away_from_opponent * 1.0);
        }

        // Holding up play = stationary with ball
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Heading up play is low intensity - holding and distributing
        ForwardCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl ForwardHeadingUpPlayState {
    fn has_support(&self, ctx: &StateProcessingContext) -> bool {
        let min_support_distance = 10.0;

        ctx.players().teammates().exists(min_support_distance)
    }
}
