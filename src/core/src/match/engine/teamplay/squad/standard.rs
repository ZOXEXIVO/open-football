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
//! # The measurement
//!
//! [`MatchStandard::of`] is the standard of football in a match, on the
//! same 0..1 scale as a normalised attribute; its distance from the
//! calibration division is **zero there by construction**, negative below
//! it, positive above. [`MatchStandard::measure`] moves every attribute
//! of both squads by that distance before kickoff, so the whole engine
//! reads the players against the football around them. That is exactly
//! calibration-neutral — at the level every constant was titrated on, not
//! a single number moves — and it leaves the WITHIN-division spread
//! intact: a good tackler is still a good tackler, measured against his
//! own league instead of against a yardstick from another.
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
//! reading — see `MatchStandard::keeper_shift_of`.

use crate::club::player::mind::MindSwitch;
use crate::r#match::MatchSquad;
use crate::r#match::engine::teamplay::tactical::TeamSkillAggregates;
use std::sync::OnceLock;

/// The two standards as the squads brought them, read once at kickoff:
/// the standard of football in a fixture is a property of the two squads
/// that turned up, not of how tired they are by the 80th minute.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StandardReading {
    /// [`MatchStandard::of`] at kickoff.
    pub outfield: f32,
    /// [`MatchStandard::keeper_of`] at kickoff.
    pub keeper: f32,
}

/// The standard of football in a match, and the measurement of both
/// squads against it.
pub struct MatchStandard;

impl MatchStandard {
    /// The standard every constant in the engine was fitted against: the
    /// reading the per-read corrections centred on when `stats 300 14 14`
    /// titrated them (0.6630 then). Squads are measured onto it at
    /// kickoff, so the willingness table's `STANDARD` column reads it back
    /// at every level; the generator's own levels read off the raw squads
    /// in `MatchResultRaw::standard_of_football` — level 4 ~0.40, level 14
    /// ~0.69, level 18 ~0.80. `OF_STANDARD_REF` overrides it so a re-fit
    /// costs no rebuild.
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

    /// The standard one side plays at, on the scale of [`Self::of`], which
    /// is the mean of the two sides' readings.
    #[inline]
    pub fn of_side(t: &TeamSkillAggregates) -> f32 {
        (t.build_up_quality + t.press_quality + t.defensive_quality + t.attacking_quality) * 0.25
    }

    /// The standard of GOALKEEPING in this match, on the goalkeeping
    /// attributes' own scale.
    #[inline]
    pub fn keeper_of(home: &TeamSkillAggregates, away: &TeamSkillAggregates) -> f32 {
        (home.gk_quality + away.gk_quality) * 0.5
    }

    /// Read both squads against the standard of football they bring, once,
    /// before a ball is kicked: every attribute moves by the match's
    /// distance from the calibration division, so every read in the engine
    /// — a decision's gate, a contest, a stride — is against the football
    /// around the player. Returns the standard as the squads brought it.
    /// One place rather than a correction at each read, because the reads
    /// left uncorrected were the whole of the chance-volume slope up the
    /// pyramid. `OF_STANDARD_OFF=1` leaves the squads as they came.
    pub fn measure(home: &mut MatchSquad, away: &mut MatchSquad) -> StandardReading {
        let home_side = TeamSkillAggregates::at_kickoff(&home.main_squad);
        let away_side = TeamSkillAggregates::at_kickoff(&away.main_squad);
        let reading = StandardReading {
            outfield: Self::of(&home_side, &away_side),
            keeper: Self::keeper_of(&home_side, &away_side),
        };
        if !Self::disabled() {
            let outfield = -20.0 * Self::shift_of(reading);
            let goalkeeping = -20.0 * Self::keeper_shift_of(reading);
            home.measure_against(outfield, goalkeeping);
            away.measure_against(outfield, goalkeeping);
        }
        reading
    }

    /// Distance from the calibration division, in normalised-attribute
    /// units: zero there by construction, negative below it, positive
    /// above.
    fn shift_of(r: StandardReading) -> f32 {
        (r.outfield - Self::reference()).clamp(-Self::MAX_SHIFT, Self::MAX_SHIFT)
    }

    /// Keeper standard per unit of outfield standard, across the levels of
    /// the generator. Measured, not chosen: `dev_match stats 40 L L` at
    /// L = 6, 10, 14, 18, 20 printed `STANDARD` 0.462 .. 0.854 against
    /// `gk_std` 0.397 .. 0.795, a least-squares slope of 1.02.
    /// `OF_STANDARD_GK_SLOPE` overrides it so a re-fit costs no rebuild.
    pub const KEEPER_SLOPE: f32 = 1.02;

    /// The goalkeeping equivalent of [`Self::shift_of`]: the standard of
    /// goalkeeping the football in this match implies.
    ///
    /// Derived from the OUTFIELD standard, not read off the two keepers on
    /// the pitch. Averaging the two keepers graded each against half of
    /// himself — a weak keeper dragged the standard down and won back half
    /// his gap on reach, read and hands — and against the man in the other
    /// goal, who has nothing to do with the shots he faces. `OF_MIND_OFF`
    /// restores the two-keeper reading as the A/B control.
    fn keeper_shift_of(r: StandardReading) -> f32 {
        let shift = if MindSwitch::armed() {
            (r.outfield - Self::reference()) * Self::keeper_slope()
        } else {
            r.keeper - Self::keeper_reference()
        };
        shift.clamp(-Self::MAX_SHIFT, Self::MAX_SHIFT)
    }

    #[inline]
    fn keeper_slope() -> f32 {
        static S: OnceLock<f32> = OnceLock::new();
        *S.get_or_init(|| {
            std::env::var("OF_STANDARD_GK_SLOPE")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(Self::KEEPER_SLOPE)
        })
    }

    /// `OF_STANDARD_OFF=1` leaves the squads unmeasured — the A/B control.
    fn disabled() -> bool {
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

    #[test]
    fn the_keeper_at_the_other_end_does_not_move_the_standard() {
        let with = |keeper: f32| StandardReading {
            outfield: 0.70,
            keeper,
        };
        assert_eq!(
            MatchStandard::keeper_shift_of(with(0.40)),
            MatchStandard::keeper_shift_of(with(0.80))
        );
    }

    #[test]
    fn the_keeper_shift_is_zero_at_the_calibration_division() {
        let reading = StandardReading {
            outfield: MatchStandard::CALIBRATION,
            keeper: 0.30,
        };
        assert_eq!(MatchStandard::keeper_shift_of(reading), 0.0);
    }

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
