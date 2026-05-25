use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::passing::box_loaded_for_corner;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

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
        if !self.is_in_wide_position(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // CORNER SET-UP HOLD: on our corner, hold the delivery until the
        // box is loaded (centre-backs need ~1-2s to sprint up) or the
        // set-up window expires. Without this the taker crosses in 5 ticks
        // — long before the CBs arrive — so they never get to attack it.
        if ctx.ball().is_team_attacking_corner()
            && !box_loaded_for_corner(ctx)
            && ctx.in_state_time < CORNER_SETUP_MAX
        {
            return None;
        }

        // After windup time, deliver the cross
        if ctx.in_state_time > CROSS_EXECUTION_TIME {
            // Find a target in the box
            if let Some(target) = self.find_cross_target(ctx) {
                #[cfg(feature = "match-logs")]
                if ctx.ball().is_team_attacking_corner() {
                    use std::sync::atomic::Ordering;
                    use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag;
                    mid_run_diag::CORNER_CROSS_SENT.fetch_add(1, Ordering::Relaxed);
                    if target.tactical_positions.is_central_defender() {
                        mid_run_diag::CORNER_CROSS_TO_CB.fetch_add(1, Ordering::Relaxed);
                    }
                }
                return Some(StateChangeResult::with_forward_state_and_event(
                    ForwardState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target.id)
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

impl ForwardCrossingState {
    fn is_in_wide_position(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let y = ctx.player.position.y;
        let wide_margin = field_height * 0.2;

        y < wide_margin || y > field_height - wide_margin
    }

    /// Find the best teammate in or near the penalty area to cross to.
    fn find_cross_target<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        let goal_pos = ctx.player().opponent_goal_position();

        let mut best_target: Option<(MatchPlayerLite, f32)> = None;

        for teammate in ctx.players().teammates().all() {
            // Skip self
            if teammate.id == ctx.player.id {
                continue;
            }

            let dist_to_goal = (teammate.position - goal_pos).magnitude();

            // Must be within 150 units of opponent goal (in/near the box)
            if dist_to_goal > 150.0 {
                continue;
            }

            // Must have a clear passing lane — EXCEPT on a corner, where the
            // delivery is lofted over the packed defenders, so a blocked
            // ground lane doesn't disqualify a central target.
            if !ctx.ball().is_team_attacking_corner() && !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

            // Score: prefer players with good heading skill and proximity to goal center
            let heading_skill = if let Some(player) = ctx.context.players.by_id(teammate.id) {
                player.skills.technical.heading
            } else {
                10.0
            };

            // On a corner, prefer the pushed-up centre-back (the designated
            // aerial target). Inert in open play (CBs aren't in the box).
            let corner_cb_bonus = if ctx.ball().is_team_attacking_corner()
                && teammate.tactical_positions.is_central_defender()
            {
                12.0
            } else {
                0.0
            };

            let score = heading_skill + corner_cb_bonus + (150.0 - dist_to_goal) / 10.0;

            if let Some((_, best_score)) = &best_target {
                if score > *best_score {
                    best_target = Some((teammate, score));
                }
            } else {
                best_target = Some((teammate, score));
            }
        }

        best_target.map(|(t, _)| t)
    }
}
