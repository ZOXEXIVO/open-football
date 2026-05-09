use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::{
    ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct ForwardFinishingState {}

impl StateProcessingHandler for ForwardFinishingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Check if the player is within shooting range
        if !self.is_within_shooting_range(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        // Centralised shot decision. The previous Finishing state fired
        // a Shoot event with no skill / clarity / xG / pass-EV checks
        // beyond a 150u distance gate — which is why mediocre finishers
        // converted at near-elite rates from any cross or rebound.
        match evaluate_forward_shot_decision(ctx, "FWD_FINISHING") {
            ShotDecision::Shoot { reason } => {
                let (shooting_direction, _) = self.calculate_shooting_parameters(ctx);
                Some(StateChangeResult::with_forward_state_and_event(
                    ForwardState::Running,
                    Event::PlayerEvent(PlayerEvent::Shoot(
                        ShootingEventContext::new()
                            .with_player_id(ctx.player.id)
                            .with_target(shooting_direction)
                            .with_reason(reason)
                            .build(ctx),
                    )),
                ))
            }
            ShotDecision::Pass => Some(StateChangeResult::with_forward_state(
                ForwardState::Passing,
            )),
            ShotDecision::Hold => Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            )),
        }
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
