use crate::r#match::events::Event;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::players::ShotQualityEvaluator;
use crate::r#match::player::strategies::players::MIN_XG_THRESHOLD;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardFinishingState {}

impl StateProcessingHandler for ForwardFinishingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Check if the player is within shooting range
        if !self.is_within_shooting_range(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        // Check shot cooldown
        if !ctx.memory().can_shoot(ctx.current_tick()) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Evaluate xG - only shoot if above minimum threshold
        let xg = ShotQualityEvaluator::evaluate(ctx);
        if xg < MIN_XG_THRESHOLD {
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Calculate the shooting direction and power
        let (shooting_direction, _) = self.calculate_shooting_parameters(ctx);

        // Transition to Running state after taking the shot
        Some(StateChangeResult::with_forward_state_and_event(
            ForwardState::Running,
            Event::PlayerEvent(PlayerEvent::Shoot(
                ShootingEventContext::new()
                    .with_player_id(ctx.player.id)
                    .with_target(shooting_direction)
                    .with_reason("FWD_FINISHING")
                    .build(ctx)
            )),
        ))
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl ForwardFinishingState {
    fn is_within_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        ctx.ball().distance_to_opponent_goal() <= 150.0
    }

    fn calculate_shooting_parameters(&self, ctx: &StateProcessingContext) -> (Vector3<f32>, f32) {
        let goal_position = ctx.player().opponent_goal_position();
        let shooting_direction = (goal_position - ctx.player.position).normalize();
        let shooting_power = 1.0;

        (shooting_direction, shooting_power)
    }
}
