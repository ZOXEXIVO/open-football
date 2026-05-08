//! Decision math for the free-agent decay model. Pure functions —
//! `Player` and `TransferConfig` already own the *state*; this module
//! owns the *formulas* that turn state + buyer context into gates,
//! wage demands, and acceptance scores.
//!
//! Bundled on a unit struct (`FreeAgentMarketCalculator`) so callers
//! see one namespace rather than free `fn`s scattered through the
//! transfer module — the project convention is "no global helpers".

use crate::transfers::pipeline::PipelineProcessor;
use crate::PlayerFieldPositionGroup;

/// Inferred role the buyer is signing the player for. Drives wage
/// asks, role-fit scoring, and acceptance. The matcher rarely knows
/// the buyer's intended role explicitly, so we read it off the
/// player's CA relative to the buyer's tier-anchored starter / ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuyerRoleFit {
    KeyPlayer,
    Starter,
    Rotation,
    Backup,
    Emergency,
}

pub struct FreeAgentMarketCalculator;

impl FreeAgentMarketCalculator {
    /// Sliding country-rep tolerance. The buyer's country can be this
    /// much weaker than the player's reference reputation before the
    /// move is implausible. At pressure 0 a Russian free agent rejects
    /// Malta; by pressure 1.0 even a 4–5k rep gap is acceptable.
    pub fn rep_drop_allowed(career_pressure: f32, age: u8, ca: u8) -> i32 {
        let cp = career_pressure.clamp(0.0, 1.0);
        let age_bonus: i32 = if age < 24 {
            0
        } else if age < 31 {
            300
        } else if age < 35 {
            700
        } else {
            1200
        };
        let quality_bonus: i32 = if ca >= 130 {
            -500
        } else if ca >= 100 {
            0
        } else if ca >= 70 {
            500
        } else {
            1000
        };
        let base = 400.0 + 2800.0 * cp;
        base.round() as i32 + age_bonus + quality_bonus
    }

    /// Sliding region-prestige tolerance. Player's home region can be
    /// this much more prestigious than the buyer's region before the
    /// move is blocked. At pressure 0 only neighbours pass; at 1.0
    /// almost every region is reachable.
    pub fn region_drop_allowed(career_pressure: f32) -> f32 {
        let cp = career_pressure.clamp(0.0, 1.0);
        0.10 + 0.55 * cp
    }

    /// Minimum CA the buyer will sign at this tier, slackened by the
    /// player's career pressure. Very pressured players will be
    /// considered well below the buyer's nominal starter standard.
    pub fn min_acceptable_ca(club_reputation_score: f32, group: PlayerFieldPositionGroup, career_pressure: f32) -> u8 {
        let starter = PipelineProcessor::tier_starter_ca_score(club_reputation_score, group) as i16;
        let tolerance = PipelineProcessor::tier_quality_tolerance_score(club_reputation_score);
        let slack = (4.0 + 10.0 * career_pressure.clamp(0.0, 1.0)).round() as i16;
        (starter - tolerance - slack).clamp(20, 200) as u8
    }

    /// Maximum CA the buyer will accept (above the tier ceiling we'd
    /// be talking about a star randomly slumming it at a small club).
    /// Pressure widens the band: a desperate 160-CA player can credibly
    /// land at a Continental club, but not at an Amateur side.
    pub fn max_acceptable_ca(club_reputation_score: f32, group: PlayerFieldPositionGroup, career_pressure: f32) -> u8 {
        let ceiling = PipelineProcessor::tier_target_ceiling_score(club_reputation_score, group) as i16;
        let overreach = (5.0 + 18.0 * career_pressure.clamp(0.0, 1.0)).round() as i16;
        (ceiling + overreach).clamp(20, 200) as u8
    }

    /// Quality fit score in [0,1]: 1.0 inside the band, falling off
    /// linearly outside. Asymmetric — being too good for the buyer
    /// decays slower than being too weak (a star slumming is rarer but
    /// still plausible at high pressure; a player below the floor is
    /// just not a realistic signing).
    pub fn quality_fit_score(ca: u8, min_acceptable: u8, max_acceptable: u8) -> f32 {
        let ca_i = ca as i16;
        let min_i = min_acceptable as i16;
        let max_i = max_acceptable as i16;
        if ca_i >= min_i && ca_i <= max_i {
            1.0
        } else if ca_i < min_i {
            (1.0 - (min_i - ca_i) as f32 / 20.0).clamp(0.0, 1.0)
        } else {
            (1.0 - (ca_i - max_i) as f32 / 35.0).clamp(0.0, 1.0)
        }
    }

    /// Floor on professional wages — below this we treat the offer as
    /// semi-pro. Scales gently with the buyer's country reputation so
    /// poor countries don't get unrealistic ask prices.
    pub fn minimum_professional_wage(buyer_country_reputation: u16) -> u32 {
        let by_country = 4_000u32 + (buyer_country_reputation as u32) * 3;
        by_country.max(2_000)
    }

    /// Wage the player would settle for at this buyer. Decays from
    /// previous salary toward the destination's market wage as career
    /// pressure climbs.
    pub fn reservation_wage(
        market_wage: u32,
        last_salary: u32,
        career_pressure: f32,
        buyer_country_reputation: u16,
    ) -> u32 {
        let cp = career_pressure.clamp(0.0, 1.0);
        let demand_multiplier = 1.20 - 0.55 * cp;
        let previous_weight = (0.75 - 0.65 * cp).clamp(0.10, 0.75);

        let previous_anchor = if last_salary > 0 {
            last_salary as f32
        } else {
            market_wage as f32 * 1.25
        };

        let from_market = market_wage as f32 * demand_multiplier;
        let from_previous = previous_anchor * previous_weight;
        let floor = Self::minimum_professional_wage(buyer_country_reputation) as f32;
        from_market.max(from_previous).max(floor) as u32
    }

    /// Buyer's offer wage. Role-weighted off the market wage, nudged by
    /// the negotiator's skill, then clamped by the country's wage
    /// floor and an upper sanity bound. The very-desperate branch lets
    /// the buyer lowball below the player's reservation.
    pub fn offer_wage(
        market_wage: u32,
        role: BuyerRoleFit,
        negotiator_skill: u8,
        buyer_country_reputation: u16,
        reservation_wage: u32,
        career_pressure: f32,
    ) -> u32 {
        let role_factor = Self::role_factor(role);
        let negotiation_factor = 0.92 + (negotiator_skill as f32 / 100.0).clamp(0.0, 0.20);
        let mut offer = (market_wage as f32) * role_factor * negotiation_factor;

        if career_pressure >= 0.75 {
            // Lowball discount kicks in only when the player has clear
            // pressure to accept. Caps at 15% below reservation.
            let floor = (reservation_wage as f32) * 0.85;
            offer = offer.max(floor);
        }

        let floor = Self::minimum_professional_wage(buyer_country_reputation) as f32;
        let ceiling = (market_wage as f32) * 2.5; // sanity, no club tops 2.5× market
        offer.clamp(floor, ceiling) as u32
    }

    pub fn role_factor(role: BuyerRoleFit) -> f32 {
        match role {
            BuyerRoleFit::KeyPlayer => 1.15,
            BuyerRoleFit::Starter => 1.00,
            BuyerRoleFit::Rotation => 0.82,
            BuyerRoleFit::Backup => 0.65,
            BuyerRoleFit::Emergency => 0.50,
        }
    }

    pub fn role_score(role: BuyerRoleFit) -> f32 {
        match role {
            BuyerRoleFit::KeyPlayer => 1.00,
            BuyerRoleFit::Starter => 0.85,
            BuyerRoleFit::Rotation => 0.60,
            BuyerRoleFit::Backup => 0.35,
            BuyerRoleFit::Emergency => 0.20,
        }
    }

    /// Infer the buyer's role intent from CA versus their tier's
    /// starter and ceiling. The matcher rarely passes an explicit role
    /// for free agents, so this gives every signing a defensible
    /// classification without requiring upstream changes.
    pub fn infer_buyer_role(ca: u8, club_reputation_score: f32, group: PlayerFieldPositionGroup) -> BuyerRoleFit {
        let starter = PipelineProcessor::tier_starter_ca_score(club_reputation_score, group) as i16;
        let ceiling = PipelineProcessor::tier_target_ceiling_score(club_reputation_score, group) as i16;
        let headroom = ceiling - starter;
        let high_anchor = starter + (headroom * 2 / 3);
        let ca_i = ca as i16;

        if ca_i >= high_anchor {
            BuyerRoleFit::KeyPlayer
        } else if ca_i >= starter {
            BuyerRoleFit::Starter
        } else if ca_i >= starter - 8 {
            BuyerRoleFit::Rotation
        } else if ca_i >= starter - 18 {
            BuyerRoleFit::Backup
        } else {
            BuyerRoleFit::Emergency
        }
    }

    /// Wage component of the acceptance score. 0 below `0.75 *
    /// reservation`, 1 at or above reservation, linear in between.
    pub fn wage_score(offer_wage: u32, reservation_wage: u32) -> f32 {
        if reservation_wage == 0 {
            return 1.0;
        }
        let ratio = (offer_wage as f32) / (reservation_wage as f32);
        ((ratio - 0.75) / 0.50).clamp(0.0, 1.0)
    }

    /// Prestige component of the acceptance score. Maps the gap between
    /// the buyer's adjusted reputation (with `rep_drop_allowed` slack)
    /// and the player's reference reputation onto [0,1]. Comfortable
    /// matches score 1.0; the buyer being borderline implausible
    /// scores near zero.
    pub fn prestige_score(
        buyer_country_reputation: u16,
        player_reference_reputation: u16,
        rep_drop_allowed: i32,
    ) -> f32 {
        let buyer = buyer_country_reputation as i32 + rep_drop_allowed;
        let gap = buyer - player_reference_reputation as i32;
        ((gap as f32) / 2500.0).clamp(0.0, 1.0)
    }

    /// Aggregate acceptance score in [0,1]. Weights mirror the design
    /// doc (wage 0.34, role 0.22, prestige 0.18, quality 0.16,
    /// pressure 0.10) — wages dominate but the smaller signals stop
    /// elite players signing with hopeless clubs just because the
    /// money's right.
    pub fn acceptance_score(
        wage_score: f32,
        role_score: f32,
        prestige_score: f32,
        quality_fit_score: f32,
        career_pressure: f32,
    ) -> f32 {
        0.34 * wage_score.clamp(0.0, 1.0)
            + 0.22 * role_score.clamp(0.0, 1.0)
            + 0.18 * prestige_score.clamp(0.0, 1.0)
            + 0.16 * quality_fit_score.clamp(0.0, 1.0)
            + 0.10 * career_pressure.clamp(0.0, 1.0)
    }

    /// Threshold the score must clear to accept. Drops as pressure
    /// rises so a desperate player accepts on weaker compositions.
    pub fn acceptance_threshold(career_pressure: f32) -> f32 {
        0.72 - 0.22 * career_pressure.clamp(0.0, 1.0)
    }

    /// Sigmoid-smoothed acceptance probability. Returns the chance
    /// (0..1) that the offer is accepted given the score and
    /// threshold. Steepness 8.0 matches the design doc — strong
    /// scores accept reliably, weak ones reliably refuse, with a
    /// short transition band.
    pub fn acceptance_probability(score: f32, threshold: f32) -> f32 {
        let z = (score - threshold) * 8.0;
        1.0 / (1.0 + (-z).exp())
    }

    /// Daily signing chance percentage (clamped 0.5..35.0). Replaces the
    /// CA-band-only `TransferConfig::daily_signing_chance` for free
    /// agents because elite players still move quickly while weak
    /// players eventually move when career pressure lifts the floor.
    pub fn daily_signing_chance(career_pressure: f32, ca: u8, urgency_bonus: f32) -> f32 {
        let cp = career_pressure.clamp(0.0, 1.0);
        let raw = 0.5 + 8.0 * cp + 0.04 * (ca as f32) + urgency_bonus.max(0.0);
        raw.clamp(0.5, 35.0)
    }

    /// Per-month retirement probability after 12 months of unemployment.
    /// Caller passes the count of months *beyond* twelve (so a player
    /// 14 months free has months_after_12 = 2). Always 0 below the
    /// threshold — that gate lives at the call site so the formula
    /// here stays a pure curve.
    pub fn retirement_probability_per_month(
        months_after_12: u32,
        age: u8,
        ca: u8,
        world_reputation: i16,
    ) -> f32 {
        let base = 0.02 * (months_after_12 as f32);
        let age_factor = if age < 28 {
            0.00
        } else if age < 32 {
            0.01
        } else if age < 35 {
            0.04
        } else {
            0.10
        };
        let quality_factor = if ca < 50 {
            0.08
        } else if ca < 70 {
            0.04
        } else {
            0.00
        };
        let rep_factor = (world_reputation.max(0) as f32 / 10_000.0) * 0.08;
        (base + age_factor + quality_factor - rep_factor).clamp(0.0, 0.35)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rep_drop_widens_with_pressure_and_age() {
        // Young 130 CA freshly free: tight tolerance.
        let fresh = FreeAgentMarketCalculator::rep_drop_allowed(0.0, 23, 130);
        // Old 60 CA at full pressure: very wide tolerance.
        let desperate = FreeAgentMarketCalculator::rep_drop_allowed(1.0, 38, 60);
        assert!(desperate > fresh + 4000, "fresh={fresh} desperate={desperate}");
    }

    #[test]
    fn region_drop_increases_with_pressure() {
        let low = FreeAgentMarketCalculator::region_drop_allowed(0.0);
        let high = FreeAgentMarketCalculator::region_drop_allowed(1.0);
        assert!(high > low + 0.40);
    }

    #[test]
    fn reservation_wage_decays_with_pressure() {
        let market = 200_000u32;
        let last = 500_000u32;
        let fresh = FreeAgentMarketCalculator::reservation_wage(market, last, 0.0, 5000);
        let desperate = FreeAgentMarketCalculator::reservation_wage(market, last, 1.0, 5000);
        assert!(
            desperate < fresh,
            "fresh={fresh} desperate={desperate} — pressure should decay demand"
        );
    }

    #[test]
    fn reservation_wage_respects_country_floor() {
        let market = 1_000u32; // unrealistically low
        let last = 0u32;
        let res = FreeAgentMarketCalculator::reservation_wage(market, last, 1.0, 6000);
        assert!(res >= FreeAgentMarketCalculator::minimum_professional_wage(6000));
    }

    #[test]
    fn quality_fit_in_band_is_one() {
        let s = FreeAgentMarketCalculator::quality_fit_score(120, 100, 140);
        assert!((s - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn quality_fit_below_band_decays_linearly() {
        let s = FreeAgentMarketCalculator::quality_fit_score(80, 100, 140);
        // 100-80 = 20, 1 - 20/20 = 0
        assert!(s <= 0.01, "s={s}");
    }

    #[test]
    fn acceptance_threshold_drops_with_pressure() {
        let lo = FreeAgentMarketCalculator::acceptance_threshold(1.0);
        let hi = FreeAgentMarketCalculator::acceptance_threshold(0.0);
        assert!(lo < hi - 0.20);
    }

    #[test]
    fn acceptance_probability_is_monotonic_in_score() {
        let t = 0.6;
        let a = FreeAgentMarketCalculator::acceptance_probability(0.4, t);
        let b = FreeAgentMarketCalculator::acceptance_probability(0.7, t);
        assert!(b > a);
    }

    #[test]
    fn daily_chance_floor_is_positive() {
        let c = FreeAgentMarketCalculator::daily_signing_chance(0.0, 30, 0.0);
        assert!(c >= 0.5);
    }

    #[test]
    fn daily_chance_max_is_clamped() {
        let c = FreeAgentMarketCalculator::daily_signing_chance(1.0, 200, 50.0);
        assert!(c <= 35.0);
    }

    #[test]
    fn retirement_probability_zero_under_twelve_months_calls_with_zero() {
        // Caller passes 0 below the gate; our curve respects that.
        let p = FreeAgentMarketCalculator::retirement_probability_per_month(0, 30, 80, 1000);
        assert!(p < 0.05);
    }

    #[test]
    fn retirement_probability_lifts_for_old_low_quality_player() {
        let young_strong =
            FreeAgentMarketCalculator::retirement_probability_per_month(6, 26, 120, 5000);
        let old_weak =
            FreeAgentMarketCalculator::retirement_probability_per_month(6, 36, 50, 0);
        assert!(old_weak > young_strong);
    }

    #[test]
    fn role_inference_buckets() {
        // Continental-tier midfielder: starter ~ 95-110 range.
        let group = PlayerFieldPositionGroup::Midfielder;
        let club_score = 0.7;
        let starter_role = FreeAgentMarketCalculator::infer_buyer_role(150, club_score, group);
        assert_eq!(starter_role, BuyerRoleFit::KeyPlayer);
        let backup_role = FreeAgentMarketCalculator::infer_buyer_role(70, club_score, group);
        assert!(matches!(
            backup_role,
            BuyerRoleFit::Backup | BuyerRoleFit::Emergency
        ));
    }
}
