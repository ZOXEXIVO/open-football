use super::activity_intensity::ActivityIntensity;
use crate::r#match::MatchPlayer;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;

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
