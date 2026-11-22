#[cfg(feature = "match-logs")]
use crate::r#match::engine::psychology::lapse::lapse_diag;
use crate::r#match::engine::psychology::{ActionLapse, LapseFamily};
use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::shared::{
    ActivityIntensity, ForwardCondition, ForwardRead,
};
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::defender_census::ChallengeOutcome;
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::forward_census::{ForwardCensus, FwdTackleTake};
use crate::r#match::player::events::{FoulCommitted, FoulSeverity, FoulSource, PlayerEvent};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::shared::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::player::strategies::shared::states::{
    BlockChallenge, ChallengeKind, ClosingPoint, RecoveryChallenge, TackleDecision,
    TackleEngagement, TackleOutcome,
};
use crate::r#match::{
    ConditionContext, MatchContext, MatchPlayerLite, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
#[cfg(feature = "match-logs")]
use crate::tackle_stats::{FWD_ATTEMPTS, FWD_SUCCESSES, FWD_TICKS_IN_STATE};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

const FOUL_CHANCE_BASE: f32 = 0.15; // Base chance of committing a foul

/// **The challenge.** Entered with the kind of challenge staged on him, it
/// rolls once, on entry, on the forward's own outcome model, and leaves in
/// the same tick: won, with the ball; missed, getting up; a foul, to the
/// whistle. A loose ball beside the duel is his side's election's, as from
/// any other state.
#[derive(Default, Clone)]
pub struct ForwardTacklingState {}

impl StateProcessingHandler for ForwardTacklingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        #[cfg(feature = "match-logs")]
        FWD_TICKS_IN_STATE.fetch_add(1, Ordering::Relaxed);

        let (Some(kind), Some(carrier)) = (
            ctx.player.staged_challenge,
            ctx.players().opponents().with_ball().next(),
        ) else {
            return Some(ForwardRead::change(ctx));
        };
        #[cfg(feature = "match-logs")]
        FWD_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
        let outcome = match kind {
            ChallengeKind::Block if MatchContext::forward_duel_off() => {
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
            ForwardCensus::note_challenge(
                ctx,
                carrier.position,
                kind,
                ChallengeOutcome::of(&outcome),
            );
            if outcome == TackleOutcome::Won {
                ForwardCensus::note_tackle_take(FwdTackleTake::Duel);
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
        // Tackling is very high intensity - explosive action
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl ForwardTacklingState {
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
                FWD_SUCCESSES.fetch_add(1, Ordering::Relaxed);
                StateChangeResult::with_forward_state_and_event(
                    ForwardState::Running,
                    Event::PlayerEvent(PlayerEvent::TacklingBall(ctx.player.id)),
                )
            }
            TackleOutcome::Foul(severity) => StateChangeResult::with_forward_state_and_event(
                ForwardState::HoldingShape,
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

    /// The block tackle on the duel's one model, priced with how he arrived:
    /// a man still diving in wins it less often and fouls more. A forward
    /// has no skill profile of his own; the midfielder's reads the same
    /// attributes for his discipline and condition.
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

        // Aggression and composure still feed the foul-risk path
        // (they should — composure protects, aggression escalates),
        // but the duel resolution itself routes through the duel
        // composites so the tackler/carrier read consistent with the
        // rest of the engine.
        let aggression = ctx.player.skills.mental.aggression / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;

        // Calculate relative positioning advantage
        let distance = ctx.tick_context.grid.get(ctx.player.id, opponent.id);
        let distance_factor = (TackleEngagement::CONTACT - distance) / TackleEngagement::CONTACT;
        let distance_factor = distance_factor.clamp(0.0, 1.0);

        // Calculate angle advantage (tackling from behind is harder but less likely to be seen)
        let opponent_velocity = ctx.tick_context.positions.players.velocity(opponent.id);
        let tackle_angle_factor = if opponent_velocity.magnitude() > 0.1 {
            let to_opponent = (opponent.position - ctx.player.position).normalize();
            let opponent_direction = opponent_velocity.normalize();
            let angle_dot = to_opponent.dot(&opponent_direction);

            // Tackling from the side (perpendicular) is most effective
            1.0 - angle_dot.abs()
        } else {
            0.8 // Stationary opponent - moderate advantage
        };

        // Duel resolution via shared composites. `defensive_duel`
        // (tackler) vs `dribble_attack` (carrier) — both are 0..1 and
        // already fatigue-folded, so the raw `base_success * 0.4`
        // mapping below stays inside its calibrated band.
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let player_tackle_ability = sc::defensive_duel(ctx.player, minute);
        let opponent_evasion_ability = match ctx.context.players.by_id(opponent.id) {
            Some(opp) => sc::dribble_attack(opp, minute),
            None => 0.50,
        };

        // Final success calculation. Forward counter-press tackle
        // success in real football is the lowest of the three roles —
        // ~15-25% — because forwards are ahead of the play, off-balance,
        // and don't drill defensive technique. Base 0.15.
        let base_success = player_tackle_ability - opponent_evasion_ability;
        let situational_bonus = distance_factor * 0.3 + tackle_angle_factor * 0.2;
        let success_chance = (0.15 + base_success * 0.4 + situational_bonus).clamp(0.03, 0.60);

        let tackle_success = rng.random::<f32>() < success_chance;

        // Calculate foul probability - more refined
        let foul_base_risk = FOUL_CHANCE_BASE;
        let aggression_risk = aggression * 0.1;
        let desperation_risk = if ctx.team().is_loosing() && ctx.context.is_running_out() {
            0.05 // More desperate when losing late in game
        } else {
            0.0
        };

        let skill_protection = composure * 0.05; // Better composure reduces foul risk
        let situation_risk = if tackle_angle_factor < 0.3 {
            0.08 // Higher risk when tackling from behind
        } else {
            0.0
        };

        let foul_chance = if tackle_success {
            // Lower foul chance for successful tackles, but still possible
            (foul_base_risk * 0.3) + aggression_risk + desperation_risk + situation_risk
                - skill_protection
        } else {
            // Higher foul chance for failed tackles
            foul_base_risk + aggression_risk + desperation_risk + situation_risk + 0.05
                - skill_protection
        };

        // Self-preservation on a booking, as in the other tackling states.
        let foul_chance = if ctx.player.yellow_cards > 0 {
            foul_chance * 0.70
        } else {
            foul_chance
        };
        // Cap, then his head scales what is left — see defenders/tackling.
        let foul_chance = ActionLapse::scaled(
            LapseFamily::Challenge,
            foul_chance.clamp(0.0, 0.4),
            ctx.context.psychology.get(ctx.player.id),
        );
        let committed_foul = rng.random::<f32>() < foul_chance;
        #[cfg(feature = "match-logs")]
        lapse_diag::note(LapseFamily::Challenge, committed_foul);

        // Forwards rarely go studs-up; tackling-from-behind (low angle factor)
        // or desperation-late-in-match pushes severity up.
        // Violent 0.10 → 0.02, Reckless gated at 0.35 — aligned with the
        // 2026-06 discipline recalibration (see defenders/tackling):
        // the engine produced ~1.0 reds/match vs real ~0.15 because most
        // failed aggressive contact escalated straight past Normal.
        let behind_tackle = tackle_angle_factor < 0.3;
        let severity = if !committed_foul {
            FoulSeverity::Normal
        } else if behind_tackle && aggression > 0.7 && rng.random::<f32>() < 0.02 {
            FoulSeverity::Violent
        } else if !tackle_success
            && (behind_tackle || aggression > 0.55)
            && rng.random::<f32>() < 0.35
        {
            FoulSeverity::Reckless
        } else {
            FoulSeverity::Normal
        };

        TackleOutcome::of(tackle_success, committed_foul, severity)
    }
}
