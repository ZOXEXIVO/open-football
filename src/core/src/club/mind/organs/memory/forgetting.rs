//! How a memory fades.
//!
//! The existing morale-event decay is linear to zero over
//! `event_decay_halflife_days = 60`, with a hard drop at
//! `event_retention_days = 365`. That is correct for *mood* — how much a
//! thing still stings — and completely wrong for *memory*. A curve that
//! reaches zero has nothing left to recall, which is why the sim cannot
//! today express a player remembering anything about his career.
//!
//! Human long-term retention is a **power law**, not an exponential:
//! `R(t) ∝ (1 + t)^-β`. The distinction is the whole point — an
//! exponential decays through the noise floor within months, while a
//! power law has a heavy tail that still holds a real fraction of its
//! original strength a decade later. That tail *is* the ten-year memory.
//! No threshold, no special case, no "if years > 10 { remember }" hack —
//! the curve does it (see the project's realism rule: continuous curves,
//! never arbitrary thresholds).
//!
//! Three things modulate it, all continuous:
//!
//! * **Personality.** `professionalism` and `consistency` slow forgetting
//!   (a focused, even-keeled man carries a clearer record of his career);
//!   `temperament` slows the forgetting of *negative* episodes
//!   specifically — a short fuse nurses a grievance long after a calm
//!   man has let it go.
//! * **Rehearsal.** Every recall bumps encoding strength and resets the
//!   clock. Walking back into a club you left a decade ago rehearses
//!   everything tagged to it, which is exactly why the memories come
//!   back.
//! * **Protection.** Flashbulb episodes (debut, first trophy, relegation,
//!   a forced sale, a career-threatening injury) hold a retention floor.
//!   Real memory does this too, and the sim needs it for the handful of
//!   events that define a career.

use super::epoch::{EpochDay, MindClock};

/// The forgetting curve, its personality modulation, and the rehearsal
/// rule. Stateless — call sites pass the record's own fields.
pub struct ForgettingCurve;

impl ForgettingCurve {
    /// Baseline decay exponent β. At 0.18 a memory encoded at full
    /// strength retains ≈53% after a year and ≈24% after a decade:
    /// faded, clearly weaker than last week's, but unmistakably still
    /// there.
    pub const BETA_BASE: f32 = 0.18;

    /// Widest personality swing applied to β, in either direction. A
    /// maximally focused player forgets at β≈0.13 (≈31% at ten years);
    /// a scattered one at β≈0.23 (≈19%).
    pub const BETA_PERSONALITY_SPAN: f32 = 0.05;

    /// Extra β reduction applied to negative episodes at maximum
    /// `temperament`. Grievances outlive kindnesses for the hot-headed.
    pub const BETA_GRUDGE_SPAN: f32 = 0.04;

    /// Retention floor for a flashbulb episode. Career-defining moments
    /// stay vivid; they do not decay into the noise with everything else.
    pub const FLASHBULB_FLOOR: f32 = 0.55;

    /// Retention below which an episode is functionally forgotten — it
    /// no longer surfaces in recall and is dropped by the consolidation
    /// pass.
    ///
    /// Set against the curve rather than picked: with β≈0.16 a decade
    /// leaves ≈24% of encoding, so this line means **importance decides
    /// longevity**. A trivial event (a newspaper writing something nice,
    /// encoding ≈0.35) crosses it inside two years; anything encoded
    /// above ≈0.5 — the events that mattered to him — is still there a
    /// decade later. That is the whole shape of a real autobiographical
    /// memory, and it falls out of one number rather than a rule per
    /// event type.
    pub const FAINT: f32 = 0.12;

    /// Encoding strength added back by a single recall, as a fraction of
    /// the gap to full strength. Rehearsal strengthens, but one act of
    /// remembering never restores a memory to the vividness it had when
    /// it happened.
    pub const REHEARSAL_GAIN: f32 = 0.18;

    /// The decay exponent for this player and this episode's valence.
    ///
    /// `professionalism`, `consistency` and `temperament` are the
    /// FM-style 0–20 personality attributes from [`PersonAttributes`].
    ///
    /// [`PersonAttributes`]: crate::club::person::PersonAttributes
    pub fn beta(professionalism: f32, consistency: f32, temperament: f32, valence: f32) -> f32 {
        // Focus/steadiness centred at 10/20 → -1..+1, then scaled.
        let focus = ((professionalism + consistency) / 2.0 - 10.0) / 10.0;
        let mut beta = Self::BETA_BASE - focus.clamp(-1.0, 1.0) * Self::BETA_PERSONALITY_SPAN;

        // A grudge-holder forgets bad things more slowly. Scales with how
        // negative the episode was, so a mild annoyance is not nursed the
        // way a betrayal is.
        if valence < 0.0 {
            let heat = (temperament / 20.0).clamp(0.0, 1.0);
            beta -= heat * Self::BETA_GRUDGE_SPAN * valence.abs().min(1.0);
        }

        beta.max(0.05)
    }

    /// Live retention of a memory: how much of it is still there.
    ///
    /// `encoding` is the strength it was laid down with (0..1 — see
    /// `EpisodeSpec` for how that is computed at the moment of the
    /// event). `last_touched` is the later of when it happened and when
    /// it was last recalled, so rehearsal genuinely resets the clock.
    pub fn retention(encoding: f32, last_touched: EpochDay, now: EpochDay, beta: f32) -> f32 {
        let days = MindClock::elapsed_f32(last_touched, now);
        (encoding * (1.0 + days).powf(-beta)).clamp(0.0, 1.0)
    }

    /// [`Self::retention`] with the flashbulb floor applied. The floor
    /// scales with the memory's own encoding, so a flashbulb laid down
    /// weakly does not outshine one laid down at full force.
    pub fn retention_protected(
        encoding: f32,
        last_touched: EpochDay,
        now: EpochDay,
        beta: f32,
        flashbulb: bool,
    ) -> f32 {
        let raw = Self::retention(encoding, last_touched, now, beta);
        if flashbulb {
            raw.max(encoding * Self::FLASHBULB_FLOOR)
        } else {
            raw
        }
    }

    /// New encoding strength after an act of recall. Approaches full
    /// strength asymptotically — remembering something often keeps it
    /// alive, but never makes it more vivid than living it.
    pub fn rehearsed(encoding: f32) -> f32 {
        (encoding + (1.0 - encoding) * Self::REHEARSAL_GAIN).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YEAR: f32 = 365.0;

    fn retention_after(days: f32, beta: f32) -> f32 {
        (1.0f32 + days).powf(-beta)
    }

    #[test]
    fn a_decade_old_memory_is_faded_but_present() {
        let r = retention_after(YEAR * 10.0, ForgettingCurve::BETA_BASE);
        assert!(
            r > 0.20 && r < 0.30,
            "ten years should retain ~24% of encoding, got {r}"
        );
        assert!(
            r > ForgettingCurve::FAINT,
            "a decade-old memory must stay above the faint line — this is the whole point"
        );
    }

    #[test]
    fn the_power_law_tail_beats_an_exponential() {
        // The design claim, asserted: an exponential with the same
        // one-year retention is essentially gone at ten years, while the
        // power law is not.
        let power = retention_after(YEAR * 10.0, ForgettingCurve::BETA_BASE);
        let one_year = retention_after(YEAR, ForgettingCurve::BETA_BASE);
        let exponential = one_year.powf(10.0); // same 1y point, exponential shape
        assert!(
            power > exponential * 20.0,
            "power law ({power}) must dominate the exponential ({exponential}) at ten years"
        );
    }

    #[test]
    fn recent_memories_are_near_full_strength() {
        let r = retention_after(7.0, ForgettingCurve::BETA_BASE);
        assert!(r > 0.65, "last week should still be vivid, got {r}");
    }

    #[test]
    fn focused_players_forget_more_slowly() {
        let focused = ForgettingCurve::beta(18.0, 17.0, 10.0, 0.5);
        let scattered = ForgettingCurve::beta(4.0, 5.0, 10.0, 0.5);
        assert!(
            focused < scattered,
            "high professionalism/consistency must lower beta ({focused} vs {scattered})"
        );
        assert!(
            retention_after(YEAR * 10.0, focused) > retention_after(YEAR * 10.0, scattered),
            "and therefore retain more after a decade"
        );
    }

    #[test]
    fn hot_tempered_players_nurse_grievances() {
        let hot_bad = ForgettingCurve::beta(10.0, 10.0, 19.0, -0.9);
        let calm_bad = ForgettingCurve::beta(10.0, 10.0, 3.0, -0.9);
        assert!(
            hot_bad < calm_bad,
            "temperament must slow the forgetting of negatives ({hot_bad} vs {calm_bad})"
        );
    }

    #[test]
    fn temperament_does_not_touch_good_memories() {
        let hot_good = ForgettingCurve::beta(10.0, 10.0, 19.0, 0.9);
        let calm_good = ForgettingCurve::beta(10.0, 10.0, 3.0, 0.9);
        assert_eq!(
            hot_good, calm_good,
            "the grudge term is negative-valence only"
        );
    }

    #[test]
    fn rehearsal_resets_the_clock() {
        let beta = ForgettingCurve::BETA_BASE;
        let encoded_on: EpochDay = 1000;
        let now: EpochDay = 1000 + 3650;

        let untouched = ForgettingCurve::retention(1.0, encoded_on, now, beta);
        // Recalled a month ago instead.
        let rehearsed = ForgettingCurve::retention(1.0, now - 30, now, beta);
        assert!(
            rehearsed > untouched * 2.0,
            "a recently recalled memory must be far stronger ({rehearsed} vs {untouched})"
        );
    }

    #[test]
    fn rehearsal_strengthens_but_never_exceeds_full() {
        let mut encoding = 0.3;
        for _ in 0..50 {
            encoding = ForgettingCurve::rehearsed(encoding);
        }
        assert!(encoding > 0.9, "repeated recall consolidates: {encoding}");
        assert!(encoding <= 1.0, "but never past full strength");
    }

    #[test]
    fn flashbulb_memories_hold_their_floor() {
        let beta = ForgettingCurve::BETA_BASE;
        let now: EpochDay = 20_000;
        let ancient: EpochDay = 1_000;

        let ordinary = ForgettingCurve::retention_protected(1.0, ancient, now, beta, false);
        let flashbulb = ForgettingCurve::retention_protected(1.0, ancient, now, beta, true);
        assert!(flashbulb >= ForgettingCurve::FLASHBULB_FLOOR);
        assert!(flashbulb > ordinary);
    }

    #[test]
    fn a_weakly_encoded_flashbulb_does_not_outshine_a_strong_one() {
        let beta = ForgettingCurve::BETA_BASE;
        let now: EpochDay = 20_000;
        let weak = ForgettingCurve::retention_protected(0.3, 1_000, now, beta, true);
        let strong = ForgettingCurve::retention_protected(1.0, 1_000, now, beta, true);
        assert!(weak < strong, "the floor scales with encoding");
    }
}
