use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
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
pub struct MidfielderCrossingState {}

impl StateProcessingHandler for MidfielderCrossingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Running
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // After windup time, deliver the cross
        if ctx.in_state_time > CROSS_EXECUTION_TIME {
            // Shared cross model — type, aim point and lane quality. Aimed
            // at a zone of the box rather than at a teammate's feet, which
            // is what lets the delivery be contested.
            if let Some(decision) = CrossModel::pick(ctx) {
                #[cfg(feature = "match-logs")]
                {
                    CrossDiag::note(decision.cross_type);
                    {
                        let goal = ctx.player().opponent_goal_position();
                        crate::mid_run_diag::WideDiag::note_delivery(
                            1,
                            (goal.x - ctx.player.position.x).abs(),
                            (ctx.player.position.y - goal.y).abs() < 165.0,
                        );
                    }
                }
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::builder()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(decision.target_id)
                            .with_cross_type(decision.cross_type)
                            .with_target_point(decision.aim_point)
                            .with_reason("MID_CROSS")
                            .build(ctx),
                    )),
                ));
            }

            // No target found — fall back to generic passing
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Passing,
            ));
        }

        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Stationary while preparing the cross
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Crossing is very high intensity - explosive action
        MidfielderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
