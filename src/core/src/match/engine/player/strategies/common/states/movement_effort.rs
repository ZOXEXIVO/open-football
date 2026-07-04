use super::activity_intensity::ActivityIntensity;

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
