//! Goalkeeper cross-claim contest.
//!
//! When a keeper gathers an opponent's ball that is not a shot, the
//! engine used to do two things unconditionally: hand him the ball, and
//! credit a `gk_command_action`. Both were wrong.
//!
//! **The claim always succeeded.** A keeper coming for a cross is one of
//! the few moments in a match where goalkeeping ability is nakedly
//! exposed — an elite keeper takes it cleanly over a crowd, a nervous
//! 18-year-old flaps at it and puts it back into the six-yard box. With
//! no contest, that failure mode did not exist: `gk_failed_claims_to_shot`
//! and `_to_goal` were read by the rating model and never produced by
//! anything.
//!
//! **Every gather was credited as commanding the box.** The counter fired
//! ~15 times per match (measured `dev_match gap`, level 14) against a
//! real-football 1-3. That is not command of the area, it is a keeper
//! picking up back-passes and tame through-balls. It mattered far beyond
//! the small `command` credit: `gk_command_actions >= 3` promotes a
//! keeper to the `GkBusy` evidence tier and `>= 2` to the top clean-sheet
//! tier, so EVERY keeper cleared both bars every week regardless of how
//! he played.
//!
//! Both are fixed here, at the producer, rather than at the rating
//! boundary — an over-emitting counter is an engine gap, and the divisor
//! table exists for volume, not for meaning.
//!
//! Continuity note: there is no "is this contested" boolean. The
//! situation produces a `command_weight` in 0..1 (how much of a genuine
//! command-of-area moment this is: height, pressure, whether it came from
//! a cross, pace) which is used as the PROBABILITY that the moment is
//! worth recording — a statistician's judgement call, modelled as the
//! chance it reads as one. Routine collections fall out with low weight
//! and no roll, without a threshold anywhere.

/// Claim-contest diagnostic counters — how many balls the keepers
/// gathered, how many of those read as command-of-area moments, and how
/// many were flapped. Without these the contest is unobservable: the
/// stat sheet only records a flap that a shot followed, so a
/// mis-calibrated rate looks identical to a correctly-calibrated one.
/// `match-logs` only; the dev harness prints them.
#[cfg(feature = "match-logs")]
pub mod gk_claim_diag {
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Non-shot opponent balls a keeper gathered.
    pub static GATHERS: AtomicU64 = AtomicU64::new(0);
    /// Of those, the ones that read as a genuine command-of-area claim.
    pub static COMMAND_MOMENTS: AtomicU64 = AtomicU64::new(0);
    /// Of those, the ones the keeper flapped.
    pub static FLAPS: AtomicU64 = AtomicU64::new(0);
    /// Shot events that fired while a flap was still pending — the
    /// numerator of "does a spilled cross actually become a chance".
    pub static FLAP_SHOTS_SEEN: AtomicU64 = AtomicU64::new(0);
    /// Of those, the ones charged to the keeper.
    pub static FLAP_TO_SHOT: AtomicU64 = AtomicU64::new(0);
    /// Pending flaps dropped because the shot came too late or from the
    /// keeper's own side.
    pub static FLAP_DROPPED: AtomicU64 = AtomicU64::new(0);

    pub fn reset() {
        for c in [
            &GATHERS,
            &COMMAND_MOMENTS,
            &FLAPS,
            &FLAP_SHOTS_SEEN,
            &FLAP_TO_SHOT,
            &FLAP_DROPPED,
        ] {
            c.store(0, Ordering::Relaxed);
        }
    }

    pub fn snapshot() -> (u64, u64, u64) {
        (
            GATHERS.load(Ordering::Relaxed),
            COMMAND_MOMENTS.load(Ordering::Relaxed),
            FLAPS.load(Ordering::Relaxed),
        )
    }

    /// `(shots seen while pending, charged, dropped)`.
    pub fn resolution() -> (u64, u64, u64) {
        (
            FLAP_SHOTS_SEEN.load(Ordering::Relaxed),
            FLAP_TO_SHOT.load(Ordering::Relaxed),
            FLAP_DROPPED.load(Ordering::Relaxed),
        )
    }
}

use crate::PlayerFieldPositionGroup;
use crate::r#match::MatchField;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffSkillCtx, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;

/// How the situation reads at the moment the keeper reaches the ball.
pub struct ClaimSituation {
    /// 0..1 — how much this is a genuine command-of-area claim rather
    /// than a routine gather. Doubles as the probability the moment is
    /// recorded as a command action.
    pub command_weight: f32,
    /// 0..1 — how hard the claim is, given height, traffic and pace.
    pub difficulty: f32,
}

pub struct GkClaimContest;

impl GkClaimContest {
    /// Radius (game units, 1u = 0.125m) inside which an opponent counts
    /// as putting the keeper under pressure. 24u ≈ 3m — close enough to
    /// challenge for the ball in the air.
    const PRESSURE_RADIUS: f32 = 24.0;
    /// Ball height (metres) at which a claim is unambiguously an aerial
    /// one. Above this the weight saturates.
    const AERIAL_HEIGHT: f32 = 2.0;
    /// Scale of the failure curve. Set so an ordinary contested claim
    /// (difficulty ≈ 0.5) is flapped ~13% of the time by a weak young
    /// keeper and ~3% by an elite one — a handful of dropped crosses a
    /// season for the young keeper, one or two for the international.
    const FAILURE_SCALE: f32 = 0.53;
    /// Exponent on the keeper's shortfall. Superlinear so the spread
    /// between a poor keeper and a good one is wider than the spread
    /// between a good keeper and a great one, which is how the real
    /// distribution of flapped crosses looks.
    const FAILURE_EXPONENT: f32 = 1.5;
    const FAILURE_MIN: f32 = 0.005;
    const FAILURE_MAX: f32 = 0.35;

    /// Read the situation around the keeper as he reaches the ball.
    pub fn assess(field: &MatchField, gk_id: u32) -> ClaimSituation {
        let ball_pos = field.ball.position;
        let ball_speed = field.ball.velocity.norm();
        let from_cross = field.ball.pending_pass_was_cross;

        let gk_team = field.players.iter().find(|p| p.id == gk_id).map(|p| p.team_id);

        // Traffic: opponents close enough to challenge. Saturating so a
        // crowded six-yard box doesn't run away with the weight.
        let mut opponents_near = 0.0f32;
        if let Some(team) = gk_team {
            for p in field.players.iter() {
                if p.team_id == team || p.is_sent_off {
                    continue;
                }
                let dx = p.position.x - ball_pos.x;
                let dy = p.position.y - ball_pos.y;
                if dx * dx + dy * dy <= Self::PRESSURE_RADIUS * Self::PRESSURE_RADIUS {
                    opponents_near += 1.0;
                }
            }
        }
        let pressure = 1.0 - (-opponents_near / 1.5).exp();

        // Height: a ball above head height is a claim; one rolling along
        // the ground is a gather.
        let aerial = (ball_pos.z / Self::AERIAL_HEIGHT).clamp(0.0, 1.0);
        let pace = (ball_speed / 4.0).clamp(0.0, 1.0);
        let cross = if from_cross { 1.0 } else { 0.0 };

        // Command weight — what makes a gather read as commanding the
        // box. Height and traffic dominate; a cross is the archetypal
        // case, pace is a minor contributor.
        let command_weight =
            (aerial * 0.42 + pressure * 0.30 + cross * 0.20 + pace * 0.08).clamp(0.0, 1.0);

        // Difficulty shares the same inputs but weights pace higher (a
        // driven cross is hard to hold whatever the traffic) and adds a
        // floor so even an uncontested high ball can be fumbled.
        let difficulty =
            (0.12 + aerial * 0.30 + pressure * 0.26 + pace * 0.22 + cross * 0.10).clamp(0.0, 1.0);

        ClaimSituation {
            command_weight,
            difficulty,
        }
    }

    /// Probability the keeper flaps this claim.
    ///
    /// `minute` routes the composites through `effective_skill`, so a
    /// tired keeper late in the game is more likely to spill — the same
    /// fatigue channel the save model uses.
    pub fn failure_probability(field: &MatchField, gk_id: u32, situation: &ClaimSituation, minute: u32) -> f32 {
        let Some(keeper) = field.players.iter().find(|p| p.id == gk_id) else {
            return 0.0;
        };
        if keeper.tactical_position.current_position.position_group()
            != PlayerFieldPositionGroup::Goalkeeper
        {
            return 0.0;
        }
        // Claiming a cross is `gk_claim_cross` (aerial reach, command of
        // area, handling, positioning, anticipation, jumping) held
        // together by the head: a keeper who loses his nerve in traffic
        // drops balls his reach says he should hold.
        let craft = sc::gk_claim_cross(keeper, minute);
        let mental_ctx = EffSkillCtx::mental(minute);
        let composure = (effective_skill(keeper, keeper.skills.mental.composure, mental_ctx) / 20.0)
            .clamp(0.0, 1.0);
        let concentration = (effective_skill(
            keeper,
            keeper.skills.mental.concentration,
            mental_ctx,
        ) / 20.0)
        .clamp(0.0, 1.0);
        let skill = (craft * 0.62 + composure * 0.20 + concentration * 0.18).clamp(0.0, 1.0);

        let shortfall = (1.0 - skill).powf(Self::FAILURE_EXPONENT);
        (Self::FAILURE_SCALE * situation.difficulty * shortfall)
            .clamp(Self::FAILURE_MIN, Self::FAILURE_MAX)
    }
}
