use chrono::NaiveDate;
use core::Player;
use core::Staff;
use core::club::staff::perception::{AbilityEstimator, CoachEye, EstimationContext};

/// Star rating on a half-star scale — 0..=10 halves render as 0..=5 stars.
/// Precomputed into full/half/empty segment counts so templates stay a
/// dumb loop with no arithmetic. Field order carries the derived `Ord`:
/// more full stars ranks higher, a half star breaks the tie.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StarRating {
    pub full: u8,
    pub half: bool,
    pub empty: u8,
}

impl StarRating {
    /// Map a 1..200 ability-scale value onto the 10-step half-star scale.
    fn from_ability_scale(value: u8) -> Self {
        let halves = (((value as f32 / 200.0) * 10.0).round().clamp(0.0, 10.0) as u8).min(10);
        StarRating {
            full: halves / 2,
            half: halves % 2 == 1,
            empty: (10 - halves) / 2,
        }
    }
}

/// Star-rating projector for the web crate. Absolute scale only —
/// no club-relative baselines. Ability is the coach-observable level
/// (visible skills + match results + training + reputation), never the
/// hidden `current_ability` digit. Potential is the judge's eye: the
/// team's own head coach reads the player's ceiling and gets it wrong by
/// as much as his `judging_player_potential`, the player's age and his
/// own biases allow (see [`CoachEye`]). The viewer has no club of his
/// own, so every page shows what that club's coach believes — mistakes
/// included. The read is licensed for display only; club decisions keep
/// the observable estimator.
pub struct PotentialStarsView;

impl PotentialStarsView {
    /// The club's own coach has watched this player for a long time — a
    /// saturated observation count, not a scout's cold first read.
    const OWN_PLAYER_OBSERVATIONS: u8 = 20;

    /// Stars from the coach-observable current level — what any
    /// competent observer concludes from watching the player play,
    /// train, and perform. Never the hidden CA digit.
    pub fn current(player: &Player) -> StarRating {
        StarRating::from_ability_scale(AbilityEstimator::observable_level(player))
    }

    /// Potential stars as this coach reads them. `is_main_team` marks
    /// whether the player is in the coach's daily training group (false
    /// for academy kids and loaned-out players assessed from the parent
    /// club) — a coach sees less of those unless he has an eye for
    /// youth. Floored at the current-ability stars: nobody tells you the
    /// ceiling is below where the player already plays. A vacant bench
    /// resolves to the stub staff (id 0), one shared phantom judge; that
    /// falls back to the market consensus instead.
    pub fn potential_by_staff(
        player: &Player,
        staff: &Staff,
        is_main_team: bool,
        date: NaiveDate,
    ) -> StarRating {
        StarRating::from_ability_scale(Self::potential_value_by_staff(
            player,
            staff,
            is_main_team,
            date,
        ))
        .max(Self::current(player))
    }

    /// The 1..200 value behind [`Self::potential_by_staff`], for pages
    /// that sort by the same read they display.
    pub fn potential_value_by_staff(
        player: &Player,
        staff: &Staff,
        is_main_team: bool,
        date: NaiveDate,
    ) -> u8 {
        if staff.id == 0 {
            return CoachEye::consensus(player, date);
        }
        let ctx = EstimationContext {
            observation_count: Self::OWN_PLAYER_OBSERVATIONS,
            is_main_team,
            ..EstimationContext::default()
        };
        CoachEye::read(player, staff, &ctx, date)
    }

    /// Judge-free potential stars — the market's consensus read for
    /// free-agent and retired views where no employing club exists.
    /// Same floor as the coach read.
    pub fn potential_absolute(player: &Player, date: NaiveDate) -> StarRating {
        StarRating::from_ability_scale(CoachEye::consensus(player, date)).max(Self::current(player))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_star_scale_maps_ability_correctly() {
        let elite = StarRating::from_ability_scale(200);
        assert_eq!((elite.full, elite.half, elite.empty), (5, false, 0));

        let mid = StarRating::from_ability_scale(100);
        assert_eq!((mid.full, mid.half, mid.empty), (2, true, 2));

        let none = StarRating::from_ability_scale(0);
        assert_eq!((none.full, none.half, none.empty), (0, false, 5));

        // Half-star resolution separates players the whole-star scale
        // collapsed: CA 170 (4.5★) must outrank CA 150 (4★).
        assert!(StarRating::from_ability_scale(170) > StarRating::from_ability_scale(150));
    }

    #[test]
    fn segments_always_sum_to_five_stars() {
        for v in (0..=200).step_by(5) {
            let s = StarRating::from_ability_scale(v);
            assert_eq!(
                s.full + s.half as u8 + s.empty,
                5,
                "segments must fill the 5-star row for value {v}"
            );
        }
    }
}
