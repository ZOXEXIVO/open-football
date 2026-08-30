//! The standard of football being played in a match, and the amount by
//! which it differs from the division every constant in the engine was
//! fitted in.
//!
//! # Why this exists
//!
//! An attribute value is not a quantity of football on its own — it is a
//! quantity *relative to the people on the pitch*. `tackling 14/20` is an
//! ordinary centre-half in the division `dev_match stats 300 14 14`
//! calibrates against and an outstanding one three tiers below it. Every
//! place in the engine that compares an attribute against a fixed number
//! therefore prices the DIVISION rather than the player, and the engine
//! plays a different sport at each end of the pyramid.
//!
//! Measured, 2026-08-21, `dev_match levels 300 4 20 2` (equal squads at
//! every point, so nothing here is a mismatch effect):
//!
//! | level | goals/m | shots/tm | on-target | save% | box-shot share |
//! |---|---|---|---|---|---|
//! | 6  | 3.06 | 13.8 | 26.6% | 58.2% | 84.1% |
//! | 8  | 3.34 | 14.6 | 27.4% | 58.3% | 80.9% |
//! | 12 | 2.32 | 11.0 | 27.5% | 61.8% | 52.9% |
//! | 14 | 2.73 | 13.0 | 29.8% | 64.9% | 44.4% |
//! | 20 | 2.84 | 11.0 | 42.5% | 69.7% | 44.4% |
//!
//! Real football holds ~2.65 goals, ~33% on target and ~68% saved at
//! every level of every pyramid. The three columns that slide are the
//! three families of absolute read: how often a defender challenges, how
//! near goal a carrier drives, and how much of the goal a keeper covers.
//!
//! # The primitive
//!
//! [`MatchStandard::of`] is the standard of football in this match, on
//! the same 0..1 scale as a normalised attribute. [`MatchStandard::shift`]
//! is its distance from the calibration division: **zero there by
//! construction**, negative below it, positive above.
//!
//! A site is made divisionally flat by reading `attribute - shift`
//! instead of `attribute`. That is exactly calibration-neutral — at the
//! level every constant in the engine was titrated on, the shift is 0 and
//! not a single number moves — and it leaves the WITHIN-division spread
//! completely intact: a good tackler is still a good tackler, he is just
//! measured against the football around him instead of against a
//! yardstick from another league.
//!
//! # Why a team composite can stand in for an attribute
//!
//! Every composite in [`TeamSkillAggregates`] is a
//! weight-1 blend of `sc::n(skill)`, and `n` is linear. A squad whose
//! attributes all sit δ higher therefore moves every composite by exactly
//! δ/20 — the same amount a normalised attribute moves. So the deviation
//! of a composite from its own calibration value IS the deviation of the
//! underlying attributes, in attribute units, with no rescaling. Four of
//! them are averaged across both sides rather than one being trusted,
//! so a lopsided squad (all attack, no defence) does not drag the
//! estimate around.
//!
//! The goalkeeping attributes live on their own scale and get their own
//! reading — see [`MatchStandard::keeper_shift`].

use crate::r#match::MatchContext;
use crate::r#match::engine::teamplay::tactical::TeamSkillAggregates;
use std::sync::OnceLock;

/// The two standards, read once and then held for the whole match.
///
/// **Latched deliberately.** The team composites are recomputed every
/// ~100 ticks off `effective_skill`, so they sag as the match is played;
/// letting the standard sag with them would mean a tiring side quietly
/// re-levelling every constant it is measured against — a feedback loop,
/// and one that would also break the profile memos, which are keyed on
/// the assumption that everything but condition is frozen. The standard
/// of football in a fixture is a property of the two squads that turned
/// up, so it is read at kickoff and left alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StandardReading {
    /// [`MatchStandard::of`] at kickoff.
    pub outfield: f32,
    /// [`MatchStandard::keeper_of`] at kickoff.
    pub keeper: f32,
}

/// The standard of football in a match, and its distance from the
/// division the engine's constants were fitted in.
pub struct MatchStandard;

impl MatchStandard {
    /// Value of [`Self::of`] in the calibration division.
    ///
    /// **Measured, not chosen.** `dev_match stats 80 14 14` prints it as
    /// `STANDARD` in the willingness table — 0.6630 over 80 matches at
    /// level 14, which is the division `stats 300 14 14` and every
    /// constant titrated on it belong to. It reads the same in all six
    /// distance bands (0.66275-0.66368), because it is a property of the
    /// squads and not of the shot. Re-read it if the composite weights or
    /// the generator move; `OF_STANDARD_REF` overrides it so a re-fit
    /// costs no rebuild.
    ///
    /// The generator is linear in the level — measured 0.36937 at level 4
    /// through 0.84075 at level 20, a slope of 0.0295 a level against the
    /// 0.02875 a uniform +0.575 attribute shift predicts — so the shift
    /// below is a straight reading of how far this match sits from the
    /// calibration division, in attribute units.
    pub const CALIBRATION: f32 = 0.6630;

    /// Value of [`Self::keeper_of`] in the same division, printed as
    /// `gk_std` by the same table. Separate because `gk_quality` blends
    /// the goalkeeping attributes, which the generator does not hand the
    /// same population mean as the outfield ones — measured 0.588 against
    /// the outfield 0.663.
    pub const KEEPER_CALIBRATION: f32 = 0.5877;

    /// How far the two sides can be pulled apart before the standard
    /// stops describing either of them.
    ///
    /// A mismatch is not a different standard of football, it is a good
    /// side playing a bad one, and both must keep their own edge over the
    /// other. Averaging the two is what preserves that: the stronger
    /// side's players sit ABOVE the match's standard and get their lift,
    /// the weaker side's sit below and pay for it.
    ///
    /// The rail is wide enough to leave the whole generator inside it —
    /// the bottom of the pyramid measures a shift of −0.39 and the top
    /// +0.18 — because a clamp that BITES is a step in the football
    /// rather than in the model, and `ArrivingRunner::TIGHTNESS_EXPONENT`
    /// records what that costs (level 4 collapsing onto `MAX_REQUIRED`
    /// instead of onto anything a footballer would recognise). It is here
    /// only so a synthetic fixture with no real squad behind it cannot
    /// push the shift somewhere no constant in the engine was ever
    /// measured.
    const MAX_SHIFT: f32 = 0.45;

    /// The standard of football in this match, in normalised-attribute
    /// units. Both sides, four outfield composites each.
    #[inline]
    pub fn of(home: &TeamSkillAggregates, away: &TeamSkillAggregates) -> f32 {
        let side = |t: &TeamSkillAggregates| {
            t.build_up_quality + t.press_quality + t.defensive_quality + t.attacking_quality
        };
        (side(home) + side(away)) * 0.125
    }

    /// The standard of GOALKEEPING in this match, on the goalkeeping
    /// attributes' own scale.
    #[inline]
    pub fn keeper_of(home: &TeamSkillAggregates, away: &TeamSkillAggregates) -> f32 {
        (home.gk_quality + away.gk_quality) * 0.5
    }

    /// Take the kickoff reading, once. Called from the first
    /// skill-aggregate pass (`TeamShape::refresh`); a second call is a
    /// no-op, which is what makes the reading a property of the fixture
    /// rather than of the minute.
    #[inline]
    pub fn latch(ctx: &mut MatchContext) {
        if ctx.standard.is_some() {
            return;
        }
        ctx.standard = Some(StandardReading {
            outfield: Self::of(&ctx.home_skill_aggregates, &ctx.away_skill_aggregates),
            keeper: Self::keeper_of(&ctx.home_skill_aggregates, &ctx.away_skill_aggregates),
        });
    }

    /// Distance from the calibration division. Add this to a fixed
    /// reference, or subtract it from an attribute — the two are the same
    /// operation and both are exactly neutral at the calibration level.
    ///
    /// Zero before the first aggregate pass has run, which is the honest
    /// answer: with both sides still reading `TeamSkillAggregates::neutral`
    /// there is nothing to measure the match against yet.
    #[inline]
    pub fn shift(ctx: &MatchContext) -> f32 {
        match ctx.standard {
            Some(r) if !Self::disabled() => {
                (r.outfield - Self::reference()).clamp(-Self::MAX_SHIFT, Self::MAX_SHIFT)
            }
            _ => 0.0,
        }
    }

    /// A normalised (0..1) attribute read against the standard of
    /// football in this match rather than against the whole game.
    ///
    /// The operation is `norm01 - shift`, which is nothing more than
    /// [`Self::shift`]'s own doc restated — but it is the operation every
    /// caller wants and it was open-coded at each of them, so a new one
    /// could read the raw attribute and nothing would say it had gone
    /// wrong. `ShotSkillProfile` applies it to eleven bands;
    /// `PlayerOperationsImpl::shoot_goal_power` did not apply it at all,
    /// and that single omission was worth **38% of shot power across the
    /// pyramid** (measured 1.415 at level 6 against 1.953 at level 18,
    /// `dev_match levels`) with every other term in the shooting chain
    /// coming out flat.
    ///
    /// Exactly neutral at the calibration level, where `shift` is zero.
    #[inline]
    pub fn peer(norm01: f32, shift: f32) -> f32 {
        (norm01 - shift).clamp(0.0, 1.0)
    }

    /// The goalkeeping equivalent of [`Self::shift`].
    #[inline]
    pub fn keeper_shift(ctx: &MatchContext) -> f32 {
        match ctx.standard {
            Some(r) if !Self::disabled() => {
                (r.keeper - Self::keeper_reference()).clamp(-Self::MAX_SHIFT, Self::MAX_SHIFT)
            }
            _ => 0.0,
        }
    }

    /// `OF_STANDARD_OFF=1` pins every shift to zero, which restores the
    /// pre-2026-08-21 engine exactly. The A/B control for the whole
    /// family — see the module note.
    #[inline]
    pub fn disabled() -> bool {
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| {
            std::env::var("OF_STANDARD_OFF")
                .map(|v| v != "0" && !v.is_empty())
                .unwrap_or(false)
        })
    }

    #[inline]
    fn reference() -> f32 {
        static R: OnceLock<f32> = OnceLock::new();
        *R.get_or_init(|| {
            std::env::var("OF_STANDARD_REF")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(Self::CALIBRATION)
        })
    }

    #[inline]
    fn keeper_reference() -> f32 {
        static R: OnceLock<f32> = OnceLock::new();
        *R.get_or_init(|| {
            std::env::var("OF_STANDARD_GK_REF")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(Self::KEEPER_CALIBRATION)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aggregates(v: f32) -> TeamSkillAggregates {
        TeamSkillAggregates {
            build_up_quality: v,
            press_quality: v,
            defensive_quality: v,
            attacking_quality: v,
            gk_quality: v,
            concentration_teamwork_avg: v,
            top_leadership: v,
            keeper_voice: v,
        }
    }

    /// The whole point: a squad whose attributes all sit δ higher moves
    /// the standard by δ, so `attribute - shift` is the same number in
    /// every division.
    #[test]
    fn the_standard_tracks_a_uniform_attribute_shift_one_for_one() {
        let base = MatchStandard::of(&aggregates(0.40), &aggregates(0.40));
        let up = MatchStandard::of(&aggregates(0.60), &aggregates(0.60));
        assert!((up - base - 0.20).abs() < 1e-5, "{base} -> {up}");
    }

    /// A mismatch is a good side playing a bad one, not a different
    /// standard: the average is what leaves each side its own edge.
    #[test]
    fn a_mismatch_sits_between_the_two_sides() {
        let s = MatchStandard::of(&aggregates(0.35), &aggregates(0.75));
        assert!((s - 0.55).abs() < 1e-5, "{s}");
    }

    /// Nothing recalibrates at the level everything was fitted at.
    #[test]
    fn the_calibration_division_shifts_by_nothing() {
        let s = MatchStandard::of(
            &aggregates(MatchStandard::CALIBRATION),
            &aggregates(MatchStandard::CALIBRATION),
        );
        assert!((s - MatchStandard::CALIBRATION).abs() < 1e-5, "{s}");
    }
}
