use chrono::NaiveDate;
use core::Player;
use core::Staff;
use core::club::staff::perception::{AbilityEstimator, EstimationContext, PotentialEstimator};

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
/// no club-relative baselines. Both rows are *perception*, never the
/// hidden digits: ability comes from the coach-observable level
/// (visible skills + match results + training + reputation), potential
/// from staff-assessed projections. The biological `current_ability` /
/// `potential_ability` numbers never leak onto a page.
pub struct PotentialStarsView;

impl PotentialStarsView {
    /// Stars from the coach-observable current level — what any
    /// competent observer concludes from watching the player play,
    /// train, and perform. Never the hidden CA digit.
    pub fn current(player: &Player) -> StarRating {
        StarRating::from_ability_scale(AbilityEstimator::observable_level(player))
    }

    /// Staff-assessed potential stars: the observer's *credible*
    /// projection (uncertainty-discounted believed ceiling) on the
    /// 1..200 scale. The employing club's coach watches this player
    /// every day — a saturated observation count, not a scout's cold
    /// first read — so the error band is tight and the stars stay
    /// stable. `is_main_team` marks whether the player is in the
    /// observer's daily training group (false for academy kids and
    /// loaned-out players assessed from the parent club). Floored at
    /// the current-ability stars: staff don't tell you the ceiling is
    /// below where the player already plays.
    pub fn potential_by_staff(
        player: &Player,
        staff: &Staff,
        is_main_team: bool,
        date: NaiveDate,
    ) -> StarRating {
        // A vacant bench resolves to the stub staff (id 0) — one shared
        // phantom judge whose noise seed is identical world-wide. Fall
        // back to the observer-free ceiling instead of pretending a
        // coach exists.
        if staff.id == 0 {
            return Self::potential_absolute(player, date);
        }
        let ctx = EstimationContext {
            observation_count: 20,
            is_main_team,
            ..EstimationContext::default()
        };
        let estimate = PotentialEstimator::estimate_for_staff(player, staff, &ctx, date);
        StarRating::from_ability_scale(estimate.credible_potential).max(Self::current(player))
    }

    /// Observer-free potential stars — the "any reasonable observer"
    /// ceiling for free-agent and retired views where no employing
    /// club exists. Same floor as the staff read.
    pub fn potential_absolute(player: &Player, date: NaiveDate) -> StarRating {
        let ceiling = PotentialEstimator::observable_ceiling(player, date);
        StarRating::from_ability_scale(ceiling).max(Self::current(player))
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
