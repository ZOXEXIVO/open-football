use crate::r#match::common_states::LooseBallChase;
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::player::events::{FoulSeverity, PlayerEvent};
use crate::r#match::player::strategies::common::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::player::strategies::common::states::{
    TackleDecision, TackleEngagement, TackleOutcome,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

#[derive(Default, Clone)]
pub struct MidfielderTacklingState {}

impl StateProcessingHandler for MidfielderTacklingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        #[cfg(feature = "match-logs")]
        crate::tackle_stats::MID_ENTRIES.fetch_add(1, Ordering::Relaxed);

        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        let ball_distance = ctx.ball().distance();

        if ball_distance > 150.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // If ball is moving away but opponent still nearby, keep pressing
        if ball_distance > 80.0 && !ctx.ball().is_towards_player_with_angle(0.8) {
            return if ctx.team().is_control_ball() {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::AttackSupporting,
                ))
            } else {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ))
            };
        }

        // Per-player tackle cooldown. Midfielders re-enter Tackling via
        // Pressing, Running, and Standing roles; without a shared cooldown
        // each one re-fires a tackle attempt next tick, driving fouls and
        // successful tackles 5-10× above real-football rates.
        if !ctx.player.can_attempt_tackle() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        let opponents = ctx.players().opponents();
        let mut opponents_with_ball = opponents.with_ball();

        if let Some(opponent) = opponents_with_ball.next() {
            let opponent_distance = ctx.tick_context.grid.get(ctx.player.id, opponent.id);

            // The shared break-off. This state's own exits are BALL
            // distances of 80u and 150u, neither of which a carrier who
            // has simply gone past his man ever trips.
            if opponent_distance > TackleEngagement::DISENGAGE {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
            if opponent_distance <= TackleEngagement::CONTACT {
                // JOCKEY FIRST — the same rule the defenders got, and for
                // the same reason. Reaching contact range is not a reason
                // to lunge; the cooldown alone was the limiter, so a
                // midfielder in range challenged roughly once a second for
                // as long as he stayed there. Measured: 7,411 attempts and
                // 2,846 successes against a back line that had already
                // been fixed down to 1,445 / 604 — midfielders were
                // essentially the entire remaining excess, 47.2 successful
                // tackles per team per match against a real ~18.
                //
                // Declining keeps him in the state containing, exactly as
                // it does for a defender.
                // The licence is asked of the CHALLENGE, not of the entry
                // tick — see `DefenderTacklingState`.
                if !TackleEngagement::may_engage_carrier(ctx)
                    || !TackleDecision::is_eligible(ctx)
                    || !ctx
                        .context
                        .rng
                        .bernoulli(TackleDecision::commits_now(ctx, opponent_distance))
                {
                    return None;
                }
                #[cfg(feature = "match-logs")]
                crate::tackle_stats::MID_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
                let mut result = self.settle(ctx, self.attempt_tackle(ctx, &opponent));
                result.start_tackle_cooldown = true;
                return Some(result);
            }
        } else if self.can_intercept_ball(ctx) {
            // can_intercept_ball already checks is_in_flight
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Running,
                Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let pace = ctx.player.skills.physical.acceleration / 20.0;
        // Explosive closing speed — skilled tacklers close gaps faster
        let speed_boost = 1.3 + tackling_skill * 0.3 + pace * 0.3; // 1.3x - 1.9x

        Some(
            SteeringBehavior::Pursuit {
                target: ctx.tick_context.positions.ball.position,
                target_velocity: ctx.tick_context.positions.ball.velocity,
            }
            .calculate(ctx.player)
            .velocity
                * speed_boost
                + ctx.player().separation_velocity() * 0.2,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tackling is explosive and very demanding physically
        MidfielderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl MidfielderTacklingState {
    /// One resolved challenge, turned into the state change it implies.
    fn settle(&self, ctx: &StateProcessingContext, outcome: TackleOutcome) -> StateChangeResult {
        match outcome {
            TackleOutcome::Won => {
                #[cfg(feature = "match-logs")]
                crate::tackle_stats::MID_SUCCESSES.fetch_add(1, Ordering::Relaxed);
                StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Standing,
                    Event::PlayerEvent(PlayerEvent::TacklingBall(ctx.player.id)),
                )
            }
            TackleOutcome::Foul(severity) => StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Standing,
                Event::PlayerEvent(PlayerEvent::CommitFoul(ctx.player.id, severity)),
            ),
            TackleOutcome::Missed => {
                StateChangeResult::with_midfielder_state(MidfielderState::Pressing)
            }
        }
    }

    /// `discipline` drives the foul model; the duel itself is the shared
    /// composite pair every role resolves — see `defenders/tackling` for
    /// why a peer-relative profile cannot be differenced against an
    /// absolute carry score.
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
        let foul_chance = base_foul.max(0.005);

        let committed_foul = rng.random::<f32>() < foul_chance;

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

    /// See `DefenderTacklingState::can_intercept_ball` — one prediction
    /// model, the one the movement layer uses.
    fn can_intercept_ball(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.tick_context.ball.is_owned {
            return false;
        }
        let meeting = LooseBallChase::meeting_point(
            ctx,
            ctx.tick_context.positions.ball.position,
            ctx.tick_context.positions.ball.velocity,
        );
        (meeting - ctx.player.position).magnitude() <= TackleEngagement::CONTACT
    }
}
