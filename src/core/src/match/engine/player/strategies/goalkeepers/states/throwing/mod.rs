use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperRelease,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperThrowingState {}

impl StateProcessingHandler for GoalkeeperThrowingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the goalkeeper has the ball
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 2. Find the best teammate to throw the ball to
        if let Some((teammate, _reason)) = self.find_best_pass_option(ctx) {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::KeeperReleaseDiag::note_throw(
                (teammate.position - ctx.player.position).norm(),
            );
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate.id)
                        .with_reason("GK_THROWING")
                        .build(ctx),
                )),
            ));
        }

        // Nobody in throwing range. A keeper doesn't stand holding the
        // ball indefinitely (and the referee wouldn't let them) — switch
        // to booting it clear. Mirrors the Distributing timeout so no
        // release path can stall with the ball in hand.
        if ctx.in_state_time > 20 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Clearing,
            ));
        }

        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Remain stationary while throwing the ball
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Throwing requires moderate intensity with focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl GoalkeeperThrowingState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<(MatchPlayerLite, &'static str)> {
        // Throwing has limited range, but still prefer longer throws.
        //
        // The search radius was a private 150u — 18.75 m — while the
        // decision to throw at all was taken against a 110u "throw range"
        // in `HoldingBall`. Neither was in metres and neither matched the
        // other. A keeper's throw reaches the best part of forty metres,
        // which is the whole reason it competes with a kick; measured at
        // 150u the average throw travelled **9.5 m**, a roll to the nearest
        // full-back. One shared range now decides both.
        PassEvaluator::find_best_pass_option(ctx, KeeperRelease::THROW_RANGE)
    }
}
