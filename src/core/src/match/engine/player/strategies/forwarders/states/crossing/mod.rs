use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::passing::CrossModel;
#[cfg(feature = "match-logs")]
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::{
    CORNER_CROSS_SENT, CORNER_CROSS_TO_CB, CrossDiag,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

const CROSS_EXECUTION_TIME: u64 = 5;
/// Max ticks the taker holds an attacking corner waiting for the box to
/// load before delivering anyway (the dead-ball set-up window).
const CORNER_SETUP_MAX: u64 = 200;

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

        // CORNER SET-UP HOLD: on our corner, hold the delivery until the
        // box is loaded (centre-backs need ~1-2s to sprint up) or the
        // set-up window expires. Without this the taker crosses in 5 ticks
        // — long before the CBs arrive — so they never get to attack it.
        if ctx.ball().is_team_attacking_corner()
            && !CrossModel::box_loaded_for_corner(ctx)
            && ctx.in_state_time < CORNER_SETUP_MAX
        {
            return None;
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
                    if ctx.ball().is_team_attacking_corner() {
                        CORNER_CROSS_SENT.fetch_add(1, Ordering::Relaxed);
                        if let Some(t) = ctx.context.players.by_id(decision.target_id) {
                            if t.tactical_position.current_position.is_central_defender() {
                                CORNER_CROSS_TO_CB.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
                return Some(StateChangeResult::with_forward_state_and_event(
                    ForwardState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
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
