use crate::PlayerFieldPositionGroup;
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperPunt,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

/// The goalkeeper's long ball.
///
/// Two things reach this state and they are struck differently:
///
/// * **From his gloves** — the punt. He drops it and hits it, and it is
///   aimed at a channel at his own kicking range rather than at a man's
///   feet. [`KeeperPunt`] owns the whole model, including why this cannot
///   be a `PassTo`.
/// * **From the floor** — a long goal kick, still resolved as a pass at a
///   chosen receiver, because a dead ball struck off the deck is aimed
///   rather than launched.
///
/// Before the punt existed this state ran the pass search for both, and
/// the search's distance bands were written in units while its comments
/// read them as metres. Its "extreme kick" tier needed a distribution
/// composite over 0.62 against a population value of 0.34, so for almost
/// every keeper in the game the top-scoring option was a midfielder
/// 100-200u — twelve to twenty-five METRES — away. A state entered around
/// ten times a keeper a match, named `Kicking`, played a short pass every
/// single time.
#[derive(Default, Clone)]
pub struct GoalkeeperKickingState {}

impl StateProcessingHandler for GoalkeeperKickingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the goalkeeper has the ball
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 2. In his hands — punt it. No receiver, no lead, no claim
        //    privilege: a ball dropped on the halfway line belongs to
        //    whoever gets up highest.
        if KeeperPunt::from_hands(ctx) {
            if let Some(plan) = KeeperPunt::plan(ctx) {
                #[cfg(feature = "match-logs")]
                crate::mid_run_diag::KeeperReleaseDiag::note_punt(
                    plan.is_drop_kick(),
                    (plan.target - ctx.player.position).norm(),
                    plan.apex,
                    plan.target_man.is_some(),
                );
                return Some(StateChangeResult::with_goalkeeper_state_and_event(
                    GoalkeeperState::ReturningToGoal,
                    Event::PlayerEvent(PlayerEvent::ClearBall(plan.velocity)),
                ));
            }
        }

        // 3. Off the deck: find the best teammate to kick the ball to
        if let Some((teammate, _reason)) = self.find_best_pass_option(ctx) {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::KeeperReleaseDiag::note_goal_kick(
                (teammate.position - ctx.player.position).norm(),
            );
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate.id)
                        .with_reason("GK_KICKING")
                        .build(ctx),
                )),
            ));
        }

        // No target worth aiming at — hoof it clear rather than hold the
        // ball. Mirrors the Distributing / Throwing timeout so no release
        // path can stall with the ball in hand.
        if ctx.in_state_time > 20 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Clearing,
            ));
        }

        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Kicking requires moderate intensity with focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl GoalkeeperKickingState {
    /// Target search for a long GOAL KICK — a placed ball struck off the
    /// floor at a chosen man. The punt does not come through here.
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<(MatchPlayerLite, &'static str)> {
        // Kicking range scales with the unified distribution profile —
        // weak keepers can't reliably reach the far end, so cap the
        // search. `field_width` is 840u = 105 m, so the band runs roughly
        // 126 m for the weakest keeper to 294 m for the best — well past
        // the pitch either way, which is deliberate: the SCORING below is
        // what decides how far he actually looks.
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let max_distance = ctx.context.field_size.width as f32 * (1.2 + prof.distribution * 1.6);

        let vision_skill = ctx.player.skills.mental.vision / 20.0;
        let kicking_skill = ctx.player.skills.goalkeeping.kicking / 20.0;

        // How far up the pitch this keeper can put a goal kick, in UNITS.
        // A goal kick off the deck carries a little less than a punt from
        // the hands; `KeeperPunt` owns the range model and this is a
        // fraction of it, so the two cannot drift apart.
        //
        // Every band below used to be a raw literal — 60 / 100 / 200 /
        // 300 units — annotated in metres, which put the "long kick" band
        // at 7.5-12.5 m and the "extreme" one at 37.5. Expressed against
        // the keeper's own reach they mean what they say.
        let reach = KeeperPunt::goal_kick_reach(ctx, &prof);

        // Extreme-kick capability is now the unified distribution
        // composite — concentration / decisions / composure already
        // baked in.
        let extreme_capability = prof.distribution;
        let prefers_extreme = extreme_capability > 0.62;

        let mut best_option: Option<MatchPlayerLite> = None;
        let mut best_score = 0.0;

        for teammate in ctx.players().teammates().nearby(max_distance) {
            // GRADUATED RECENCY PENALTY: Penalize recent passers instead of hard-skipping
            let recency_penalty = ctx.ball().passer_recency_penalty(teammate.id);

            let distance = (teammate.position - ctx.player.position).norm();

            // Calculate base score using vision-weighted evaluation
            // (side-aware: "upfield" flips for Right teams)
            let forward_progress = ctx
                .player
                .side
                .map_or(0.0, |s| {
                    s.forward_delta(ctx.player.position.x, teammate.position.x)
                })
                .max(0.0);
            let field_progress = forward_progress / ctx.context.field_size.width as f32;

            // Check if receiver is a forward
            let is_forward = matches!(
                teammate.tactical_positions.position_group(),
                PlayerFieldPositionGroup::Forward
            );

            // Check space around receiver
            let nearby_opponents = ctx.tick_context.grid.opponents(teammate.id, 15.0).count();
            let space_factor = match nearby_opponents {
                0 => 3.0, // Completely free
                1 => 1.8,
                2 => 1.0,
                _ => 0.4,
            };

            // Distance scoring against WHAT THIS KEEPER CAN HIT. A ball at
            // the very end of his range is his best long option; past it
            // he cannot reach, and well short of it he is wasting the kick.
            let of_reach = distance / reach.max(1.0);
            let distance_score = if of_reach > 1.15 {
                // Beyond the leg. Only a keeper who really can strike one
                // should be tempted, and even then it is a poor option.
                if prefers_extreme && is_forward {
                    0.9 + (extreme_capability - 0.62) * 3.0
                } else {
                    0.2
                }
            } else if of_reach > 0.75 {
                // The proper long goal kick — landing where he meant it.
                if is_forward {
                    3.0 + vision_skill * 1.5
                } else {
                    2.2 + kicking_skill * 0.8
                }
            } else if of_reach > 0.40 {
                // Into midfield. Fine, but it is not what this state is.
                if is_forward { 2.0 } else { 1.6 }
            } else {
                // Short. `Distributing` is the state for this.
                0.6
            };

            // Position bonus
            let position_bonus = match teammate.tactical_positions.position_group() {
                PlayerFieldPositionGroup::Forward => {
                    if of_reach > 0.75 {
                        2.2 // The target man at the end of the kick
                    } else {
                        1.5
                    }
                }
                PlayerFieldPositionGroup::Midfielder => {
                    if of_reach > 1.15 {
                        0.7 // Don't over-hit it past midfield
                    } else {
                        1.2
                    }
                }
                PlayerFieldPositionGroup::Defender => 0.3, // Avoid kicking to defenders
                PlayerFieldPositionGroup::Goalkeeper => 0.1,
            };

            // Combine all factors with vision-based weighting and recency penalty
            let score = distance_score
                * space_factor
                * position_bonus
                * (1.0 + field_progress)
                * (0.5 + vision_skill * 0.5)
                * recency_penalty;

            if score > best_score {
                best_score = score;
                best_option = Some(teammate);
            }
        }

        // Fallback to standard evaluator if no good option found
        if best_option.is_none() || best_score < 1.0 {
            PassEvaluator::find_best_pass_option(ctx, max_distance)
        } else {
            best_option.map(|teammate| (teammate, "GK_KICKING_CUSTOM_EVALUATION"))
        }
    }
}
