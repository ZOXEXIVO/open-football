use chrono::NaiveDate;
use core::Player;
use core::Staff;
use core::club::staff::perception::{EstimationContext, PotentialEstimator};

/// Star-rating projector for the web crate. Absolute scale only —
/// CA stars come from `current_ability` / 200; potential stars come
/// from staff-assessed projections (`PotentialEstimator`), never the
/// hidden biological `potential_ability`. No club-relative baselines:
/// the same player reads the same stars for the same observer and
/// date regardless of which squad you're viewing them in.
pub struct PotentialStarsView;

impl PotentialStarsView {
    /// Stars (0..5) from the player's absolute current ability.
    pub fn current(player: &Player) -> u8 {
        (5.0 * (player.player_attributes.current_ability as f32 / 200.0))
            .round()
            .clamp(0.0, 5.0) as u8
    }

    /// Staff-assessed potential stars: the observer's *credible*
    /// projection (uncertainty-discounted believed ceiling) on the
    /// 1..200 scale. The hidden biological PA never leaks onto the
    /// page — a sharper judge shows a tighter belief, not a truer
    /// one. Floored at the current-ability star count: staff don't
    /// tell you the ceiling is below where the player already plays.
    pub fn potential_by_staff(player: &Player, staff: &Staff, date: NaiveDate) -> u8 {
        let estimate = PotentialEstimator::estimate_for_staff(
            player,
            staff,
            &EstimationContext::default(),
            date,
        );
        let stars = (5.0 * (estimate.credible_potential as f32 / 200.0))
            .round()
            .clamp(0.0, 5.0) as u8;
        stars.max(Self::current(player))
    }

    /// Observer-free potential stars — the "any reasonable observer"
    /// ceiling for free-agent and retired views where no employing
    /// club exists. Same floor as the staff read.
    pub fn potential_absolute(player: &Player, date: NaiveDate) -> u8 {
        let ceiling = PotentialEstimator::observable_ceiling(player, date);
        let raw = (5.0 * (ceiling as f32 / 200.0)).round().clamp(0.0, 5.0) as u8;
        raw.max(Self::current(player))
    }
}
