//! **A goalkeeper's claim of a high ball, decided once.** He chooses to
//! catch or punch it as he comes (`MatchPlayer::claim_intent`). The cross
//! contest decides a cross he came for as it is delivered, and any other
//! ball he came for is decided here as it reaches him. Either way the
//! decision is carried out here, on contact, and staged on him for his
//! claim states to finish: he holds it, throws the punch, or comes down
//! with nothing.

use crate::r#match::engine::ball::ball::KeeperClaimCall;
use crate::r#match::engine::engine::*;
use crate::r#match::engine::psychology::{ActionLapse, LapseFamily};
use crate::r#match::goalkeepers::states::shared::KeeperAerialClaim;
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::keeper_census::{ClaimEnd, ClaimSite};
use crate::r#match::player::events::gk_claim::GkClaimContest;
#[cfg(feature = "match-logs")]
use crate::r#match::player::events::gk_claim::gk_claim_diag;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{ClaimIntent, ClaimOutcome};
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// How far out from his goal a flapped ball is pushed (1.2–3.2 m), and
    /// how far across: the punched-cross landing, around the six-yard line
    /// and the penalty spot.
    pub(crate) const FLAP_OUT: (f32, f32) = (10.0, 26.0);
    pub(crate) const FLAP_ACROSS: f32 = 14.0;

    /// Carry out the claim of a ball that has reached the keeper coming for
    /// it: the decision the cross contest left on the ball, or one made now.
    pub(in crate::r#match::engine::engine) fn resolve_keeper_claim(
        field: &mut MatchField,
        context: &mut MatchContext,
    ) {
        let call = field.ball.keeper_claim;
        // The reservation it rode on has gone: somebody else touched the
        // ball, or it dropped out of the air unplayed. He never got to it,
        // and the claim was decided once already.
        if let Some(call) = call
            && field.ball.aerial_contest_winner != Some(call.keeper_id)
        {
            field.ball.keeper_claim = None;
            if let Some(keeper) = field.players.iter_mut().find(|p| p.id == call.keeper_id) {
                keeper.on_claim_lost();
            }
            #[cfg(feature = "match-logs")]
            context
                .keeper_census
                .get_mut()
                .claim_end(call.keeper_id, ClaimEnd::Lost);
            return;
        }
        // A shot is the save model's, however it came to be at him.
        if field.ball.current_owner.is_some()
            || field.ball.cached_shot_target.is_some()
            || field.ball.awaiting_restart.is_some()
            || field.ball.in_net.is_some()
        {
            return;
        }
        let keeper = match call {
            Some(call) => field.players.iter().position(|p| p.id == call.keeper_id),
            None => field.players.iter().position(|p| p.claim_intent.is_some()),
        };
        let Some(idx) = keeper else {
            return;
        };
        if !KeeperAerialClaim::within_reach(&field.players[idx], field.ball.position) {
            return;
        }
        let outcome = match call {
            Some(call) => call.outcome,
            None => {
                #[cfg(feature = "match-logs")]
                {
                    let tick = context.current_tick();
                    context.keeper_census.get_mut().claim_roll(
                        field.players[idx].id,
                        ClaimSite::Contest,
                        tick,
                    );
                }
                Self::decide_claim(field, context, idx)
            }
        };
        field.ball.keeper_claim = None;
        Self::carry_out_claim(field, context, idx, outcome);
    }

    /// The aerial contest's one decision on a keeper's claim: does what he
    /// chose to do with the ball come off? A catch is priced by the flap
    /// model, a punch by the fist's, both against the traffic around it.
    pub(in crate::r#match::engine::engine) fn decide_claim(
        field: &MatchField,
        context: &MatchContext,
        idx: usize,
    ) -> ClaimOutcome {
        let keeper = &field.players[idx];
        let intent = keeper
            .claim_intent
            .expect("a keeper on a claim chose how to play it");
        let situation = GkClaimContest::assess(field, keeper.id);
        let minute = sc::minute_from_ticks(field.ball.current_tick_cached);
        let fails = ActionLapse::scaled(
            LapseFamily::Claim,
            GkClaimContest::failure(keeper, intent, &situation, minute),
            context.psychology.get(keeper.id),
        )
        .min(0.95);
        if context.rng.unit_f32() < fails {
            return ClaimOutcome::Flapped;
        }
        match intent {
            ClaimIntent::Catch => ClaimOutcome::Caught,
            ClaimIntent::Punch => ClaimOutcome::Punched,
        }
    }

    /// The cross contest's call for a keeper who gets to a cross first, kept
    /// on the ball, which is his, until it reaches him.
    pub(in crate::r#match::engine::engine) fn reserve_claim(
        field: &mut MatchField,
        context: &MatchContext,
        idx: usize,
    ) {
        let outcome = Self::decide_claim(field, context, idx);
        let keeper_id = field.players[idx].id;
        field.ball.keeper_claim = Some(KeeperClaimCall { keeper_id, outcome });
        field.ball.aerial_contest_winner = Some(keeper_id);
        field.ball.pass_target_player_id = None;
    }

    fn carry_out_claim(
        field: &mut MatchField,
        context: &mut MatchContext,
        idx: usize,
        outcome: ClaimOutcome,
    ) {
        let (keeper_id, team_id) = (field.players[idx].id, field.players[idx].team_id);
        let command = GkClaimContest::assess(field, keeper_id).command_weight;
        match outcome {
            ClaimOutcome::Caught => {
                field
                    .ball
                    .take_into_hands(keeper_id, team_id, context.current_tick());
            }
            ClaimOutcome::Punched => field.ball.hold_for_punch(keeper_id),
            ClaimOutcome::Flapped => {
                let out = context.rng.range_f32(Self::FLAP_OUT.0, Self::FLAP_OUT.1);
                let across = context.rng.range_f32(-Self::FLAP_ACROSS, Self::FLAP_ACROSS);
                field.ball.flap_off(&field.players[idx], out, across);
            }
        }
        field.ball.cross_contest_resolved = true;
        // A claim taken in traffic is command of the area; the weight is the
        // chance it reads as one.
        if outcome != ClaimOutcome::Flapped && context.rng.unit_f32() < command {
            field.players[idx].statistics.note_gk_command_action();
        }
        field.players[idx].on_claim_outcome(outcome);
        #[cfg(feature = "match-logs")]
        gk_claim_diag::CLAIMS.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "match-logs")]
        match outcome {
            ClaimOutcome::Caught => context
                .keeper_census
                .get_mut()
                .claim_end(keeper_id, ClaimEnd::Caught),
            ClaimOutcome::Punched => context
                .keeper_census
                .get_mut()
                .claim_end(keeper_id, ClaimEnd::Punched),
            ClaimOutcome::Flapped => {
                gk_claim_diag::FLAPS.fetch_add(1, Ordering::Relaxed);
                context
                    .keeper_census
                    .get_mut()
                    .claim_end(keeper_id, ClaimEnd::Flapped);
            }
        }
    }
}
