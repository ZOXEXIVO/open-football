//! **What KIND of chance this is.**
//!
//! The taxonomy a shot is classified into, and the conversion multiplier
//! that goes with each kind. Real xG models separate header from foot,
//! free kick from open play, rebound from build-up: at identical
//! distance and angle those convert at meaningfully different rates, and
//! the difference is the shot type, not the geometry.
//!
//! # There is exactly one xG model, and it is not here
//!
//! [`ShotSkillProfile::expected_xg`] is it. This file used to carry a
//! second, complete one — `ShotQualityEvaluator`, with its own distance
//! curve, angle factor, keeper factor, pressure buckets and skill
//! multiplier — and it had **zero live callers**: every gate, every
//! recorded stat and the in-flight dispatcher all read the profile. Two
//! parallel xG models is how a future change gets wired into the dead one
//! and measures as a null result, so the evaluator is gone and its one
//! genuinely useful part, the per-type multiplier below, now feeds the
//! live profile.
//!
//! [`ShotSkillProfile::expected_xg`]: crate::r#match::player::strategies::players::ops::shot_skill::ShotSkillProfile::expected_xg

use crate::r#match::PassOriginRestart;

/// Type of the shot being taken. Drives the per-type xG multiplier and,
/// for the types that are a different ACTION rather than a different
/// look at goal (a penalty, a free kick, a header), which skills the
/// strike is executed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotType {
    FootOpenPlay,
    Header,
    Volley,
    OneVOne,
    Cutback,
    SetPieceHeader,
    LongShot,
    Rebound,
    Penalty,
    DirectFreeKick,
}

impl ShotType {
    /// The set-piece shot type implied by the ball's restart origin, or
    /// `None` for open play. The single definition of "is this strike a
    /// set piece", shared by the event builder and the profile builder so
    /// the decision-time and in-flight views of the same shot can't
    /// disagree.
    ///
    /// `IndirectFreeKick` deliberately does NOT map: it cannot be shot
    /// into the goal, so a strike from one is not a free-kick chance.
    /// Nor does the legacy generic `FreeKick`, which the offside
    /// fallback sets — only the restart a foul actually awards
    /// (`DirectFreeKick`) is a free kick somebody stands over.
    pub fn from_restart(restart: PassOriginRestart) -> Option<ShotType> {
        match restart {
            PassOriginRestart::Penalty => Some(ShotType::Penalty),
            PassOriginRestart::DirectFreeKick => Some(ShotType::DirectFreeKick),
            _ => None,
        }
    }

    /// A strike from a stationary ball with the referee's whistle behind
    /// it. Both of these have their own well-established real-world
    /// conversion bands and neither is described by "an unpressured shot
    /// from this distance", so the profile substitutes those bands for
    /// the geometry entirely.
    pub fn is_dead_ball(self) -> bool {
        matches!(self, ShotType::Penalty | ShotType::DirectFreeKick)
    }

    /// Struck with the head. A different action from a foot shot, aimed
    /// by the neck rather than by the standing foot, so the profile
    /// executes it off [`sc::header_finish`] instead of the finishing-led
    /// open-play composites.
    ///
    /// [`sc::header_finish`]: crate::r#match::player::strategies::players::ops::skill_composites::header_finish
    pub fn is_header(self) -> bool {
        matches!(self, ShotType::Header | ShotType::SetPieceHeader)
    }

    /// Multiplier on the location's xG for this kind of chance.
    ///
    /// Real-world reference rates at matched distance/angle: headers
    /// convert at roughly half a foot shot's rate, a clean one-v-one and
    /// a cutback well above it, a rebound slightly above (the keeper is
    /// already committed and displaced).
    ///
    /// `Penalty` and `DirectFreeKick` are NOT scale factors — the profile
    /// short-circuits both onto absolute conversion bands before this is
    /// consulted — but they carry their band's centre here so the table
    /// reads as one list of conversion rates rather than a list with two
    /// holes in it.
    pub fn xg_multiplier(self) -> f32 {
        match self {
            ShotType::FootOpenPlay => 1.00,
            ShotType::Header => 0.55,
            ShotType::Volley => 0.75,
            ShotType::OneVOne => 1.20,
            ShotType::Cutback => 1.25,
            ShotType::SetPieceHeader => 0.55,
            ShotType::LongShot => 1.00,
            ShotType::Rebound => 1.15,
            ShotType::Penalty => 0.76,
            ShotType::DirectFreeKick => 0.55,
        }
    }
}

#[cfg(test)]
mod shot_type_tests {
    use super::*;

    #[test]
    fn header_xg_is_lower_than_open_play() {
        assert!(ShotType::Header.xg_multiplier() < ShotType::FootOpenPlay.xg_multiplier());
    }

    #[test]
    fn cutback_xg_higher_than_one_v_one() {
        // Cutbacks are the highest-quality chance type.
        assert!(ShotType::Cutback.xg_multiplier() > ShotType::OneVOne.xg_multiplier());
    }

    #[test]
    fn rebound_xg_above_open_play() {
        assert!(ShotType::Rebound.xg_multiplier() > ShotType::FootOpenPlay.xg_multiplier());
    }

    #[test]
    fn penalty_xg_close_to_real_world() {
        // Real-world penalty conversion ~76%.
        let m = ShotType::Penalty.xg_multiplier();
        assert!((m - 0.76).abs() < 0.01);
    }

    /// The open-play default must be exactly neutral, or classifying a
    /// shot at all would move the population conversion rate.
    #[test]
    fn open_play_and_long_shots_are_multiplier_neutral() {
        assert_eq!(ShotType::FootOpenPlay.xg_multiplier(), 1.00);
        assert_eq!(ShotType::LongShot.xg_multiplier(), 1.00);
    }

    #[test]
    fn both_header_variants_are_headers_and_neither_is_a_dead_ball() {
        for t in [ShotType::Header, ShotType::SetPieceHeader] {
            assert!(t.is_header(), "{t:?}");
            assert!(!t.is_dead_ball(), "{t:?}");
        }
        assert!(!ShotType::FootOpenPlay.is_header());
    }

    #[test]
    fn dead_balls_are_exactly_the_two_restarts_somebody_stands_over() {
        assert!(ShotType::Penalty.is_dead_ball());
        assert!(ShotType::DirectFreeKick.is_dead_ball());
        for t in [
            ShotType::FootOpenPlay,
            ShotType::Header,
            ShotType::SetPieceHeader,
            ShotType::Volley,
            ShotType::OneVOne,
            ShotType::Cutback,
            ShotType::LongShot,
            ShotType::Rebound,
        ] {
            assert!(!t.is_dead_ball(), "{t:?}");
        }
    }
}

#[cfg(test)]
mod set_piece_shot_type_tests {
    use super::*;

    #[test]
    fn dead_ball_restarts_classify_as_their_set_piece() {
        assert_eq!(
            ShotType::from_restart(PassOriginRestart::Penalty),
            Some(ShotType::Penalty)
        );
        assert_eq!(
            ShotType::from_restart(PassOriginRestart::DirectFreeKick),
            Some(ShotType::DirectFreeKick)
        );
    }

    #[test]
    fn open_play_and_non_shooting_restarts_are_not_set_piece_shots() {
        // A corner or a throw-in is not a shot type — the strike that
        // eventually comes off one is an open-play (or header) chance.
        for restart in [
            PassOriginRestart::OpenPlay,
            PassOriginRestart::Corner,
            PassOriginRestart::ThrowIn,
            PassOriginRestart::GoalKick,
        ] {
            assert_eq!(
                ShotType::from_restart(restart),
                None,
                "{restart:?} should not classify as a set-piece shot"
            );
        }
    }

    #[test]
    fn a_direct_free_kick_is_a_worse_chance_than_an_open_play_strike() {
        // The whole reason for classifying it: a wall and a set keeper
        // make it a far poorer chance than the same distance in open
        // play. (`Penalty` is not comparable on the multiplier — its
        // 0.76 is an absolute conversion rate the penalty branch
        // substitutes for the geometry, not a scale factor.)
        assert!(ShotType::DirectFreeKick.xg_multiplier() < ShotType::FootOpenPlay.xg_multiplier());
    }
}
