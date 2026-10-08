use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::passing::CrossModel;
#[cfg(feature = "match-logs")]
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::CrossDiag;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const CROSS_EXECUTION_TIME: u64 = 5;

#[derive(Default, Clone)]
pub struct ForwardCrossingState {}

impl StateProcessingHandler for ForwardCrossingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Lost possession - transition out
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Not in a wide position - should pass instead
        if !CrossModel::is_in_wide_position(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // After windup time, deliver the cross
        if ctx.in_state_time > CROSS_EXECUTION_TIME {
            // The cross model picks the delivery type AND the patch of the
            // box it is aimed at — the ball is struck at a space, not at a
            // pair of feet, so more than one player can attack it.
            if let Some(decision) = CrossModel::pick(ctx) {
                #[cfg(feature = "match-logs")]
                {
                    CrossDiag::note(decision.cross_type);
                    {
                        let goal = ctx.player().opponent_goal_position();
                        crate::mid_run_diag::WideDiag::note_delivery(
                            2,
                            (goal.x - ctx.player.position.x).abs(),
                            (ctx.player.position.y - goal.y).abs() < 165.0,
                        );
                    }
                }
                return Some(StateChangeResult::with_forward_state_and_event(
                    ForwardState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::builder()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(decision.target_id)
                            .with_cross_type(decision.cross_type)
                            .with_target_point(decision.aim_point)
                            .with_reason("FWD_CROSS")
                            .build(ctx),
                    )),
                ));
            }

            // No target found — fall back to generic passing
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Stationary while preparing the cross
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
