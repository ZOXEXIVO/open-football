use crate::r#match::events::Event;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const HEADING_HEIGHT_THRESHOLD: f32 = 1.5;
const HEADING_DISTANCE_THRESHOLD: f32 = 4.0;

#[derive(Default, Clone)]
pub struct ForwardHeadingState {}

impl StateProcessingHandler for ForwardHeadingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_position = ctx.tick_context.positions.ball.position;

        // Ball too far — transition back to running
        if ctx.ball().distance() > HEADING_DISTANCE_THRESHOLD {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Ball too low to head — transition to running
        if ball_position.z < HEADING_HEIGHT_THRESHOLD {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Attempt the header
        if self.attempt_heading(ctx) {
            // Success — shoot toward opponent goal
            Some(StateChangeResult::with_forward_state_and_event(
                ForwardState::Running,
                Event::PlayerEvent(PlayerEvent::Shoot(
                    ShootingEventContext::new()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.player().shooting_direction())
                        .with_reason("FWD_HEADING_ON_GOAL")
                        .build(ctx),
                )),
            ))
        } else {
            // Failed header — transition to running
            Some(StateChangeResult::with_forward_state(ForwardState::Running))
        }
    }


    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        Some(
            SteeringBehavior::Arrive {
                target: ball_position,
                slowing_distance: 3.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Heading is very high intensity - explosive jumping action
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl ForwardHeadingState {
    /// Determines if the forward successfully heads the ball based on skills and random chance.
    fn attempt_heading(&self, ctx: &StateProcessingContext) -> bool {
        let heading_skill = ctx.player.skills.technical.heading / 20.0;
        let jumping_skill = ctx.player.skills.physical.jumping / 20.0;
        let overall_skill = (heading_skill + jumping_skill) / 2.0;

        let random_value: f32 = rand::random();
        random_value < overall_skill
    }
}
