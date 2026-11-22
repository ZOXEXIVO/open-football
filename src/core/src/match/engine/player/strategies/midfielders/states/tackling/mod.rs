#[cfg(feature = "match-logs")]
use crate::r#match::engine::psychology::lapse::lapse_diag;
use crate::r#match::engine::psychology::{ActionLapse, LapseFamily};
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::shared::{
    ActivityIntensity, MidfielderCondition, MidfielderRead,
};
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::defender_census::ChallengeOutcome;
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::midfielder_census::{MidfielderCensus, TackleTake};
use crate::r#match::player::events::{FoulCommitted, FoulSeverity, FoulSource, PlayerEvent};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::shared::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::player::strategies::shared::states::{
    BlockChallenge, ChallengeKind, ClosingPoint, RecoveryChallenge, TackleDecision, TackleOutcome,
};
use crate::r#match::{
    ConditionContext, MatchContext, MatchPlayerLite, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
#[cfg(feature = "match-logs")]
use crate::tackle_stats::{MID_ATTEMPTS, MID_SUCCESSES, MID_TICKS_IN_STATE};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

/// **The challenge.** Entered with the kind of challenge staged on him —
/// the block tackle from containing, the stretch from being beaten — it
/// rolls once, on entry, and leaves in the same tick: won, with the ball;
/// missed, getting up; a foul, to the whistle. A loose ball beside the duel
/// is his side's election's, as from any other state.
#[derive(Default, Clone)]
pub struct MidfielderTacklingState {}

impl StateProcessingHandler for MidfielderTacklingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        #[cfg(feature = "match-logs")]
        MID_TICKS_IN_STATE.fetch_add(1, Ordering::Relaxed);

        let (Some(kind), Some(carrier)) = (
            ctx.player.staged_challenge,
            ctx.players().opponents().with_ball().next(),
        ) else {
            return Some(MidfielderRead::change(ctx));
        };
        #[cfg(feature = "match-logs")]
        MID_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
        let outcome = match kind {
            ChallengeKind::Block if MatchContext::midfielder_duel_off() => {
                self.attempt_tackle(ctx, &carrier)
            }
            ChallengeKind::Block => self.block_challenge(ctx, &carrier),
            _ => {
                let minute = sc::minute_from_ms(ctx.context.total_match_time);
                RecoveryChallenge::resolve(
                    ctx,
                    sc::defensive_duel(ctx.player, minute),
                    MidfielderSkillProfile::from_ctx(ctx).discipline,
                    TackleDecision::carrier_threat(ctx, &carrier),
                    carrier.distance(ctx),
                    RecoveryChallenge::lead(ctx, carrier.position),
                )
            }
        };
        #[cfg(feature = "match-logs")]
        {
            MidfielderCensus::note_challenge(
                ctx,
                carrier.position,
                kind,
                ChallengeOutcome::of(&outcome),
            );
            if outcome == TackleOutcome::Won {
                MidfielderCensus::note_tackle_take(TackleTake::Duel);
            }
        }
        let mut result = self.settle(ctx, outcome, kind, carrier.id);
        result.start_tackle_cooldown = true;
        Some(result)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let carrier = ctx.players().opponents().with_ball().next()?;
        Some(
            SteeringBehavior::Pursuit {
                target: ClosingPoint::for_carrier(ctx, &carrier),
                target_velocity: Vector3::zeros(),
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tackling is explosive and very demanding physically
        MidfielderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl MidfielderTacklingState {
    /// One resolved challenge, turned into the state change it implies.
    fn settle(
        &self,
        ctx: &StateProcessingContext,
        outcome: TackleOutcome,
        kind: ChallengeKind,
        carrier_id: u32,
    ) -> StateChangeResult {
        match outcome {
            TackleOutcome::Won => {
                #[cfg(feature = "match-logs")]
                MID_SUCCESSES.fetch_add(1, Ordering::Relaxed);
                StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::TacklingBall(ctx.player.id)),
                )
            }
            TackleOutcome::Foul(severity) => StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::HoldingShape,
                Event::PlayerEvent(PlayerEvent::CommitFoul(FoulCommitted {
                    fouler: ctx.player.id,
                    fouled: carrier_id,
                    severity,
                    source: FoulSource::Tackle,
                    challenge: Some(kind),
                })),
            ),
            TackleOutcome::Missed => StateChangeResult::with_missed_challenge(kind),
        }
    }

    /// `discipline` drives the foul model; the duel itself is the shared
    /// composite pair every role resolves — see `defenders/tackling` for
    /// why a peer-relative profile cannot be differenced against an
    /// absolute carry score.
    /// The block tackle on the duel's one model, priced with how he arrived:
    /// a man still diving in wins it less often and fouls more.
    fn block_challenge(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> TackleOutcome {
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let profile = MidfielderSkillProfile::from_ctx(ctx);
        BlockChallenge {
            tackle: sc::defensive_duel(ctx.player, minute),
            carry: TackleDecision::carrier_threat(ctx, opponent),
            aggression: ctx.player.skills.mental.aggression,
            discipline: profile.discipline,
            condition: profile.mid_condition_mult,
            in_own_box: ctx
                .context
                .penalty_area(ctx.player.side == Some(PlayerSide::Left))
                .contains(&ctx.tick_context.positions.ball.position),
            booked: ctx.player.yellow_cards > 0,
            rush: BlockChallenge::rush_of(
                ctx.player,
                opponent.position,
                ctx.tick_context.positions.players.velocity(opponent.id),
            ),
        }
        .resolve(&ctx.context.rng, ctx.context.psychology.get(ctx.player.id))
    }

    fn attempt_tackle(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> TackleOutcome {
        let rng = &ctx.context.rng;

        let mid_profile = MidfielderSkillProfile::from_ctx(ctx);
        let aggression01 = (ctx.player.skills.mental.aggression / 20.0).clamp(0.0, 1.0);

        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let raw_diff =
            sc::defensive_duel(ctx.player, minute) - TackleDecision::carrier_threat(ctx, opponent);
        let logistic = 1.0 / (1.0 + (-raw_diff * 2.4).exp());
        let success_chance = logistic.clamp(0.06, 0.55);
        let tackle_success = rng.random::<f32>() < success_chance;

        // Foul model driven by discipline (composure/decisions/tackling/
        // concentration blend) instead of raw composure/aggression.
        // Base 0.025 → 0.044 — see defenders/tackling for the 2026-06
        // discipline recalibration rationale (fouls ran at half the
        // real rate; reds at ~6× it).
        // 0.044 → 0.062 in the second lift — see defenders/tackling.
        // Lifted ~3× alongside the defender model — see the note there
        // for why the old rate was fitted against a tackle volume ten
        // times real, and starved the foul / card / free-kick chain once
        // the volume was corrected.
        let mut base_foul = 0.30 + aggression01 * 0.32 - mid_profile.discipline * 0.19;
        if !tackle_success {
            base_foul *= 1.75;
        }
        // Tired midfielders foul more.
        base_foul += (1.0 - mid_profile.mid_condition_mult).max(0.0) * 0.08;
        // Own-box restraint — same rationale as the defender model:
        // nobody dives in inside their own penalty area. Rectangle
        // check matches the restart-award geometry.
        let in_own_box = ctx
            .context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left))
            .contains(&ctx.tick_context.positions.ball.position);
        // 0.008 → 0.06 — see the defender model for why, including that
        // the old value sat below this model's own `max(0.005)` floor and
        // so was a saturating constant rather than a tendency.
        if in_own_box {
            base_foul *= 0.06;
        }
        // Self-preservation on a booking — see defenders/tackling.
        if ctx.player.yellow_cards > 0 {
            base_foul *= 0.70;
        }
        // His head scales what is left — see defenders/tackling.
        let foul_chance = ActionLapse::scaled(
            LapseFamily::Challenge,
            base_foul.max(0.005),
            ctx.context.psychology.get(ctx.player.id),
        );

        let committed_foul = rng.random::<f32>() < foul_chance;
        #[cfg(feature = "match-logs")]
        lapse_diag::note(LapseFamily::Challenge, committed_foul);

        // Violent 0.10 → 0.02, Reckless gated at 0.35 — most failed
        // contact is a plain foul, not a card-worthy lunge.
        let severity = if !committed_foul {
            FoulSeverity::Normal
        } else if aggression01 > 0.75 && !tackle_success && rng.random::<f32>() < 0.008 {
            FoulSeverity::Violent
        } else if !tackle_success && aggression01 > 0.55 && rng.random::<f32>() < 0.35 {
            FoulSeverity::Reckless
        } else {
            FoulSeverity::Normal
        };

        TackleOutcome::of(tackle_success, committed_foul, severity)
    }
}
