use super::activity_intensity::ActivityIntensity;
use crate::r#match::MatchPlayer;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use nalgebra::Vector3;

/// Translates a player's current exertion level — the same
/// [`ActivityIntensity`] the fatigue model reads each tick — into a
/// target movement speed expressed as a fraction of their conditioned
/// max speed, then shades it down when the player is tired (self-pacing).
///
/// Why this exists: every off-ball steering behaviour (`Seek`, `Flee`,
/// `Evade`, `FollowPath`) asks for `direction * max_speed`, and the
/// velocity integrator clamps the result back to `max_speed`. So a
/// player merely holding shape or jogging back into position drifts at a
/// full sprint. The condition diagnostic confirmed it: ~77% of outfield
/// ticks sat in the top sprint band (real players sprint ~2-4% of a
/// match). That both inflated the fatigue drain — forcing an
/// order-of-magnitude cut to the rate to compensate — and flattened
/// every off-ball movement into the same all-out gallop.
///
/// A player jogs to reposition and only sprints to press, chase, or
/// break in behind — so the speed they move at IS a function of how hard
/// the current state says they are working. The fractions are anchored
/// so each intensity tier lands in the matching velocity-occupancy band
/// (jog 30-60%, run 60-85%, sprint >85%): the distribution of states now
/// paints the distribution of speeds, instead of everything pinning to
/// the sprint band.
///
/// This is a CAP, not a floor — states whose velocity is already below
/// the scaled ceiling (a near-target `Arrive` decelerating, an
/// intentionally slow walk vector) are left untouched.
pub struct MovementEffort;

/// Ground acceleration at `acceleration` = 1, in m/s².
///
/// ⚠ **The band is ~3× the sprint literature (20–30 against a real
/// 4.5–9), and that factor is a measured price, not a guess.** Three
/// stronger doses were built and run before it (n=250, level 14,
/// `OF_RAMP_LEGACY` as the control at goals 3.2 / tackles ~10.7 / pass
/// 86.6 / shots 14.7):
///
/// * flat 4.4–8.8 m/s² (the literature) → goals **6.1**, tackles
///   **5.2**, pass 91.2, shots 22.1;
/// * 6.5–12.0 with a −75% speed falloff (real first-step-strongest
///   shape) → WORSE: goals **7.3**, tackles **4.1**, shots 25.4 — a
///   chaser lives near top speed making in-run corrections, which is
///   exactly where the falloff cut his budget, so the "more physical"
///   shape starved the defensive phase hardest. Do not rebuild the
///   falloff without recalibrating defence first;
/// * flat 13–22 → goals 3.64, tackles 9.2, pass 89.2 — close, still
///   +0.4 goals over the control's own spread.
///
/// The chase census showed chasers still out-ran carriers under all of
/// them (actual speed +0.05 u/tick) — what collapsed was the CONTACT
/// economy: the old instant-velocity integrator manufactured challenge
/// windows out of unphysical direction flips, and every tackle window,
/// engagement range and press cadence downstream is calibrated against
/// that supply. An honest 5–9 m/s² ramp therefore costs a wholesale
/// defensive recalibration (the tackle ladder, engagement, pressing) —
/// a campaign of its own. Until then this band is the titrated dose:
/// the largest ramp the calibrated defence tolerates (at it, 3 refs:
/// goals 3.40 vs control 3.20±0.16, tackles ~10.1, H−A identical, and
/// every `OF_PIN` physical-attribute sensitivity intact). It still
/// turns a standing start into a ~0.21–0.27 s build (against the old
/// 20–40 ms), forbids within-tick velocity inversions, and gives
/// `acceleration` 6-vs-18 a ~2 u lead one second into a same-pace
/// standing start instead of ~2 cm.
const ACCEL_PEAK_FLOOR_MS2: f32 = 20.0;
/// Span to `acceleration` = 20 (floor + span = 30.0 m/s² burst).
const ACCEL_PEAK_SPAN_MS2: f32 = 10.0;
/// m/s² → Δ(u/tick) per AI tick. Velocity is written on full AI ticks
/// only (every second 10 ms sim tick → dt = 0.02 s), and 1 u/tick =
/// 12.5 m/s (0.125 m per 10 ms): `a × 0.02 / 12.5`.
const MS2_TO_DV: f32 = 0.02 / 12.5;
/// Braking / change-of-direction budget multiplier band over the PEAK
/// (not the speed-decayed gain): 1.8× at agility 0 to 2.6× at agility
/// 20. Eccentric (braking)
/// force genuinely exceeds concentric in sprinting humans (~1.3–2×); the
/// top of this band is deliberately generous so arrivals stay crisp —
/// every `Arrive`/settle in the engine was calibrated against instant
/// stops, and the brake budget is what keeps that calibration honest
/// while the forward ramp becomes physical.
const BRAKE_BASE: f32 = 1.8;
const BRAKE_AGILITY_SPAN: f32 = 0.8;

impl MovementEffort {
    /// Target speed as a fraction of conditioned max speed for the given
    /// exertion level and current `condition_pct` (0..100). See the type
    /// docs for the band-alignment rationale.
    pub fn speed_fraction(intensity: ActivityIntensity, condition_pct: u32) -> f32 {
        let base = match intensity {
            // Standing, resting, holding the line with minimal movement.
            ActivityIntensity::Recovery => 0.12,
            // Walking, casual short passing — barely above a stroll.
            ActivityIntensity::Low => 0.25,
            // Jogging into space, creating space, dribbling at tempo.
            ActivityIntensity::Moderate => 0.52,
            // Sustained running: pressing, marking, covering, tracking back.
            ActivityIntensity::High => 0.78,
            // Explosive: runs in behind, shooting, tackling, chasing loose balls.
            ActivityIntensity::VeryHigh => 0.95,
        };
        base * Self::self_pacing(intensity, condition_pct)
    }

    /// The speed ceiling for the man ON the ball, as a fraction of his
    /// conditioned max speed.
    ///
    /// Kept here, next to [`Self::speed_fraction`], rather than inline at
    /// the integration point, because the two are one decision: a chase
    /// is a comparison of these two numbers and they used to be set by
    /// unrelated code that never met. The diagnostic that measures the
    /// comparison (`dead_ball_diag::CHASE_SAMPLES`) reads this same
    /// function, so it cannot drift away from the live path — the first
    /// version of it re-derived the formula and immediately went stale.
    ///
    /// Two factors. The carrier is sprinting, so he gets the same
    /// flat-out ceiling anybody sprinting off the ball gets; and carrying
    /// costs him something on top, scaled by how good a carrier he is.
    /// An elite carrier therefore matches a defender running flat out,
    /// and everybody else is a little slower — which is the real
    /// relationship, and the inverse of what the engine used to do.
    ///
    /// The carry band is `0.80 + composite * 0.20`. The previous
    /// `0.78 + composite * 0.42` reached its 1.00 clamp at composite
    /// 0.524, below the population mean, so every professional resolved
    /// to the same number and none of the six attributes behind
    /// `movement_speed_with_ball` reached anything.
    /// `standard_shift` measures the carry composite against the standard
    /// of football in this match — see `MatchStandard`. The chaser has no
    /// skill term at all here, so read absolutely the carrier's handicap
    /// shrinks monotonically up the pyramid: measured on the harness's own
    /// curve, the ordinary carrier is 9.5% slower than a flat-out chaser
    /// at the bottom and 1.7% FASTER at the top, i.e. a top-flight carrier
    /// cannot be caught by anybody. Centred, the ordinary carrier gives up
    /// the same 3% in every division and the elite one still keeps his
    /// edge over the men he plays with.
    pub fn carrier_ceiling(
        player: &MatchPlayer,
        minute: u32,
        condition_pct: u32,
        standard_shift: f32,
    ) -> f32 {
        let composite =
            (sc::movement_speed_with_ball(player, minute) - standard_shift).clamp(0.0, 1.0);
        if Self::chase_legacy() {
            return (0.78 + composite * 0.42).clamp(0.75, 1.00);
        }
        let carry = (0.80 + composite * 0.20).clamp(0.75, 1.00);
        Self::speed_fraction(ActivityIntensity::VeryHigh, condition_pct) * carry
    }

    /// Diagnostic switch: with `OF_CHASE_LEGACY` set, the chase model
    /// reverts to what it was before 2026-08-18 — the man on the ball is
    /// exempt from every ceiling except his own carry band, and the
    /// states in which a player runs AT the ball declare `High` (0.78 of
    /// top speed) rather than a sprint.
    ///
    /// This is the A/B control for the defender-engagement work. Its
    /// effect reaches every player on every tick, so "did the chase model
    /// cause this?" cannot be answered from the diff — and it must not be
    /// answered by checking out an older revision either, because the
    /// working tree moves under you. Same pattern and same purpose as
    /// `MatchContext::shape_off` and `press_off`; read once per process.
    /// Debug infrastructure — do not remove.
    ///
    /// Measured across it (both arms otherwise identical, 120 fixtures at
    /// squad level 14): carrier speed ceiling 0.525 → 0.470 u/tick and his
    /// nearest chaser's 0.447 → 0.482, so the chaser goes from being the
    /// slower man on **90% of ticks to 31%**; the carrier's nearest
    /// opponent inside 2 m 37% → 55% of ticks; goals 4.74 → 3.11 and
    /// shots per team 18.2 → 12.3, against a real ~2.5 and ~13.
    ///
    /// ⚠ The legacy arm is NOT the pre-2026-08-18 engine. It reverts the
    /// chase model alone, while `TackleDecision::BASE` and the tackle
    /// ladder stay calibrated for the chase model being ON — so the
    /// legacy arm under-produces challenges (12.3 tackles per team
    /// against the live 17.7 and a real ~18). Read it as "what does the
    /// chase model do", not as "what did the engine used to score".
    pub fn chase_legacy() -> bool {
        use std::sync::OnceLock;
        static LEGACY: OnceLock<bool> = OnceLock::new();
        *LEGACY.get_or_init(|| std::env::var("OF_CHASE_LEGACY").is_ok())
    }

    /// One AI tick of the velocity ramp — the momentum model that makes
    /// `acceleration` mean what its name says.
    ///
    /// # The defect this replaces
    ///
    /// The integrator's old per-tick bound was `sprint_capability ×
    /// agility × 0.7` ≈ 0.25–0.55 u/tick — a FULL stop-to-sprint change
    /// in one or two AI ticks. In physical units that is 150–350 m/s²
    /// against a real footballer's 5–9, so every player on the pitch
    /// reached top speed in 20–40 ms from a standing start. Two
    /// consequences, both measured before this existed:
    ///
    /// * the `acceleration` attribute decided NOTHING kinematic — a 6 and
    ///   an 18 differed by one tick of ramp (~2 cm of ground); every race,
    ///   however short, was settled by top speed alone, where
    ///   `acceleration` is a 0.2 blend weight inside
    ///   [`PlayerSkills::max_speed`](crate::PlayerSkills). Real football
    ///   is the opposite: the first five metres belong to acceleration,
    ///   and only the long chase belongs to pace.
    /// * the bound read `agility`, not `acceleration` — the attribute
    ///   named for this exact quantity was consulted nowhere on the
    ///   integration path.
    ///
    /// # The model
    ///
    /// Per AI tick (20 ms — velocity is only written on full ticks) the
    /// velocity may move toward the desired vector by at most a budget
    /// expressed as a real ground acceleration:
    ///
    /// ```text
    /// gaining speed:  20 + accel01 × 10  m/s²   (accel01 fatigue-aware)
    /// braking/turning: that × (1.8 + agility01 × 0.8)
    /// ```
    ///
    /// The band is ~3× the sprint literature on purpose — the titrated
    /// dose; the two literature-faithful doses that were built and
    /// measured broke the calibrated defence, and the numbers live on
    /// [`ACCEL_PEAK_FLOOR_MS2`]. Even so, a standing start is now a
    /// ~0.21–0.27 s build to top speed instead of the old 20–40 ms, and
    /// `acceleration` 6-vs-18 opens a real gap over the first metres.
    /// Braking and redirecting get a larger budget than gaining speed,
    /// which is the real asymmetry (eccentric force beats concentric)
    /// and what keeps arrivals crisp; `agility` owns that multiplier, so
    /// change-of-direction is its kinematic channel while straight-line
    /// burst belongs to `acceleration`. The attribute is read through
    /// [`effective_skill`] in the explosive band, so a drained player
    /// loses burst before he loses top speed, and elite stamina keeps
    /// burst alive late — the same fatigue law every duel already reads.
    ///
    /// "Gaining speed" vs "braking" is decided by comparing speeds, not
    /// headings: a 90° cut at constant speed is braking work (shedding
    /// one direction while buying another), a standing start is pure
    /// gain, and a reversal transits the brake branch until the turn is
    /// through, then accelerates out — all without a special case.
    ///
    /// The `desired` vector is capped at `effort_ceiling` (the
    /// effort/carry-scaled max the caller already computed) BEFORE the
    /// ramp, and the ramped result is capped at `athletic_ceiling` (the
    /// conditioned top speed) — never at the effort ceiling, because a
    /// sprinter whose state drops to a walk does not stop in one tick:
    /// he decelerates through it at the brake budget, transiently above
    /// the new ceiling, which is what momentum IS. The athletic ceiling
    /// stays hard: nothing here ever makes a man faster than he can run.
    ///
    /// Goalkeepers are exempt (the caller keeps the legacy bound for
    /// them): their explosive lateral band and `SaveModel`'s reach are
    /// one calibrated budget, and re-deriving keeper dive travel through
    /// outfield sprint physics would silently shrink the goal he defends.
    ///
    /// `OF_RAMP_LEGACY=1` restores the old agility-twitch bound — the
    /// A/B control (same pattern as [`Self::chase_legacy`]).
    pub fn sprint_ramp(
        player: &MatchPlayer,
        minute: u32,
        desired: Vector3<f32>,
        effort_ceiling: f32,
        athletic_ceiling: f32,
    ) -> Vector3<f32> {
        let desired = Self::cap(desired, effort_ceiling);
        let current = player.velocity;
        let delta = desired - current;
        let delta_sq = delta.norm_squared();
        if delta_sq <= 1e-12 {
            return Self::cap(desired, athletic_ceiling);
        }

        let speeding_up = desired.norm_squared() > current.norm_squared();
        let budget = Self::accel_budget(player, minute, speeding_up);

        let ramped = if delta_sq > budget * budget {
            current + delta * (budget / delta_sq.sqrt())
        } else {
            desired
        };
        Self::cap(ramped, athletic_ceiling)
    }

    /// The per-AI-tick velocity-change budget behind [`Self::sprint_ramp`],
    /// in u/tick. Public so a diagnostic or test reads the SAME number the
    /// integrator enforces — a re-derived copy goes stale (see the history
    /// on [`Self::carrier_ceiling`]).
    pub fn accel_budget(player: &MatchPlayer, minute: u32, speeding_up: bool) -> f32 {
        let accel_eff = effective_skill(
            player,
            player.skills.physical.acceleration,
            ActionContext::explosive(minute),
        );
        let accel01 = (accel_eff / 20.0).clamp(0.0, 1.0);
        let peak_ms2 = ACCEL_PEAK_FLOOR_MS2 + accel01 * ACCEL_PEAK_SPAN_MS2;
        if speeding_up {
            peak_ms2 * MS2_TO_DV
        } else {
            let agility01 = (player.skills.physical.agility / 20.0).clamp(0.0, 1.0);
            peak_ms2 * (BRAKE_BASE + agility01 * BRAKE_AGILITY_SPAN) * MS2_TO_DV
        }
    }

    /// `OF_RAMP_LEGACY=1` reverts the velocity integrator to the
    /// pre-ramp agility-twitch bound (a full stop-to-sprint change in
    /// 1–2 AI ticks, `acceleration` kinematically inert). Debug
    /// infrastructure — do not remove.
    pub fn ramp_legacy() -> bool {
        use std::sync::OnceLock;
        static LEGACY: OnceLock<bool> = OnceLock::new();
        *LEGACY.get_or_init(|| std::env::var("OF_RAMP_LEGACY").is_ok())
    }

    #[inline]
    fn cap(v: Vector3<f32>, max: f32) -> Vector3<f32> {
        let n_sq = v.norm_squared();
        if n_sq > max * max && n_sq > 0.0 {
            v * (max / n_sq.sqrt())
        } else {
            v
        }
    }

    /// Self-pacing: a tired player can't keep flinging themselves into
    /// top-tier efforts — they shorten the sprint and jog the recovery.
    /// Below ~55% condition the high-effort tiers shade down toward a
    /// sustainable cruise (to a 0.82 floor at the 15% condition floor);
    /// the low tiers are untouched because anyone can keep walking, and
    /// fresh players (≥55%) are unaffected. Pairs with the corrected
    /// `is_tired` gate, which already stops an exhausted forward from
    /// even attempting a run in behind.
    ///
    /// Below 20% condition a separate hobbled regime takes over: that
    /// band is only reachable by a player whose in-match injury could
    /// not be substituted (bench spent), and a hobbled player cannot
    /// press or burst — high tiers collapse toward a walk and even
    /// jogging shortens, continuously down to the 15% floor. The team
    /// effectively plays around a passenger, exactly like real
    /// football when the subs are gone.
    fn self_pacing(intensity: ActivityIntensity, condition_pct: u32) -> f32 {
        if condition_pct < 20 {
            let c = condition_pct.max(15) as f32;
            // 0 at 20% condition, 1 at the 15% floor.
            let hobble = (20.0 - c) / 5.0;
            return match intensity {
                ActivityIntensity::High | ActivityIntensity::VeryHigh => {
                    let cruise = 0.82 + 0.18 * ((c - 15.0) / 40.0);
                    cruise * (1.0 - hobble) + 0.35 * hobble
                }
                ActivityIntensity::Moderate => 1.0 - 0.40 * hobble,
                _ => 1.0,
            };
        }
        match intensity {
            ActivityIntensity::High | ActivityIntensity::VeryHigh if condition_pct < 55 => {
                let c = condition_pct.max(15) as f32;
                0.82 + 0.18 * ((c - 15.0) / 40.0)
            }
            _ => 1.0,
        }
    }
}
