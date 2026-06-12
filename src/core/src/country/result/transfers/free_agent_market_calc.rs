//! Decision math for the free-agent decay model. Pure functions —
//! `Player` and `TransferConfig` already own the *state*; this module
//! owns the *formulas* that turn state + buyer context into gates,
//! wage demands, and acceptance scores.
//!
//! Bundled on a unit struct (`FreeAgentMarketCalculator`) so callers
//! see one namespace rather than free `fn`s scattered through the
//! transfer module — the project convention is "no global helpers".

use super::free_agents::{EmergencySignedTerms, FreeAgentCandidate};
use crate::PlayerFieldPositionGroup;
use crate::club::player::calculators::WageCalculator;
use crate::transfers::pipeline::PipelineProcessor;
use crate::transfers::squad_needs::EmergencyContractTermsPolicy;

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

/// One candidate priced for one buyer: inferred role, the player's
/// reservation wage, and the buyer's role-weighted offer. Single
/// implementation of the wage chain shared by the normal request-driven
/// matcher, the urgent emergency fill, and the staged depth negotiation
/// — so role inference, the wage model, and the contract-length policy
/// cannot drift between entry points.
pub(super) struct FreeAgentOfferPricing {
    pub role: BuyerRoleFit,
    pub reservation_wage: u32,
    pub offer_wage: u32,
}

impl FreeAgentOfferPricing {
    /// Price one candidate for one slot at one buyer: market wage →
    /// reservation (decays with career pressure) → role-weighted offer.
    pub(super) fn compute(
        candidate: &FreeAgentCandidate,
        group: PlayerFieldPositionGroup,
        buyer_club_score: f32,
        buyer_league_reputation: u16,
        buyer_negotiator_skill: u8,
        buyer_country_reputation: u16,
    ) -> Self {
        let role =
            FreeAgentMarketCalculator::infer_buyer_role(candidate.ability, buyer_club_score, group);
        Self::compute_with_role(
            candidate,
            group,
            role,
            buyer_club_score,
            buyer_league_reputation,
            buyer_negotiator_skill,
            buyer_country_reputation,
        )
    }

    /// Same wage chain with an explicit role override. The market-
    /// clearing pass prices its offers as Backup / Emergency squad
    /// roles regardless of how well the player's CA fits the buyer's
    /// tier — the pitch is "join the squad on a modest short deal",
    /// never a starter's package.
    pub(super) fn compute_with_role(
        candidate: &FreeAgentCandidate,
        group: PlayerFieldPositionGroup,
        role: BuyerRoleFit,
        buyer_club_score: f32,
        buyer_league_reputation: u16,
        buyer_negotiator_skill: u8,
        buyer_country_reputation: u16,
    ) -> Self {
        let market_wage = WageCalculator::expected_annual_wage_raw(
            candidate.ability,
            candidate.current_reputation,
            group == PlayerFieldPositionGroup::Forward,
            group == PlayerFieldPositionGroup::Goalkeeper,
            candidate.age,
            buyer_club_score,
            buyer_league_reputation,
        );
        let reservation_wage = FreeAgentMarketCalculator::reservation_wage(
            market_wage,
            candidate.last_salary,
            candidate.career_pressure,
            buyer_country_reputation,
        );
        let offer_wage = FreeAgentMarketCalculator::offer_wage(
            market_wage,
            role,
            buyer_negotiator_skill,
            buyer_country_reputation,
            reservation_wage,
            candidate.career_pressure,
        );
        FreeAgentOfferPricing {
            role,
            reservation_wage,
            offer_wage,
        }
    }

    /// Render the priced offer into the signed-terms shape execution
    /// installs (wage + length + role promise).
    pub(super) fn signed_terms(&self, candidate: &FreeAgentCandidate) -> EmergencySignedTerms {
        EmergencySignedTerms {
            annual_wage: self.offer_wage,
            contract_years: EmergencyContractTermsPolicy::contract_years(
                candidate.age,
                candidate.ability,
            ),
            role: self.role,
        }
    }
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

    /// Hard cross-continent gate. A free agent stepping down across
    /// continent boundaries into a markedly less prestigious region
    /// — Russian → Algerian, Brazilian → Vietnamese — is unrealistic
    /// at routine career pressure even when the sliding
    /// `region_drop_allowed` gate alone would let it through. Returns
    /// `true` when the move should be blocked. `min_pressure_to_cross`
    /// is the pressure floor below which the gate fires; pass `0.85`
    /// for non-urgent paths (normal matcher, Strict emergency depth),
    /// `0.75` for urgent group fills, and skip the call entirely for
    /// no-keeper desperation slots where any registered player is
    /// preferable to an empty position.
    pub fn cross_continent_blocked(
        same_continent: bool,
        player_region_prestige: f32,
        buyer_region_prestige: f32,
        career_pressure: f32,
        min_pressure_to_cross: f32,
    ) -> bool {
        if same_continent {
            return false;
        }
        let drop = player_region_prestige - buyer_region_prestige;
        if drop <= 0.10 {
            return false;
        }
        career_pressure < min_pressure_to_cross
    }

    /// Minimum CA the buyer will sign at this tier, slackened by the
    /// player's career pressure. Very pressured players will be
    /// considered well below the buyer's nominal starter standard.
    pub fn min_acceptable_ca(
        club_reputation_score: f32,
        group: PlayerFieldPositionGroup,
        career_pressure: f32,
    ) -> u8 {
        let starter = PipelineProcessor::tier_starter_ca_score(club_reputation_score, group) as i16;
        let tolerance = PipelineProcessor::tier_quality_tolerance_score(club_reputation_score);
        let slack = (4.0 + 10.0 * career_pressure.clamp(0.0, 1.0)).round() as i16;
        (starter - tolerance - slack).clamp(20, 200) as u8
    }

    /// Maximum CA the buyer will accept (above the tier ceiling we'd
    /// be talking about a star randomly slumming it at a small club).
    /// Pressure widens the band: a desperate 160-CA player can credibly
    /// land at a Continental club, but not at an Amateur side.
    pub fn max_acceptable_ca(
        club_reputation_score: f32,
        group: PlayerFieldPositionGroup,
        career_pressure: f32,
    ) -> u8 {
        let ceiling =
            PipelineProcessor::tier_target_ceiling_score(club_reputation_score, group) as i16;
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
        // Sanity ceiling never drops below the country's minimum professional
        // wage — a low-CA player whose 2.5× market wage sits below the local
        // pro-wage floor would otherwise produce floor > ceiling and panic.
        let ceiling = ((market_wage as f32) * 2.5).max(floor);
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
    pub fn infer_buyer_role(
        ca: u8,
        club_reputation_score: f32,
        group: PlayerFieldPositionGroup,
    ) -> BuyerRoleFit {
        let starter = PipelineProcessor::tier_starter_ca_score(club_reputation_score, group) as i16;
        let ceiling =
            PipelineProcessor::tier_target_ceiling_score(club_reputation_score, group) as i16;
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
    /// The 0.30 slope (was 0.22) makes a year-unemployed player
    /// accept the modest backup-role offers the market-clearing pass
    /// makes — at 0.22 those compositions sat just under the line
    /// and long sits never resolved.
    pub fn acceptance_threshold(career_pressure: f32) -> f32 {
        0.72 - 0.30 * career_pressure.clamp(0.0, 1.0)
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

    /// Combined priority score in [0,1] for ordering free-agent
    /// candidates against one transfer request. Replaces the legacy
    /// raw-quality `max_by_key` so a realistic, willing, affordable
    /// local journeyman outranks a stronger player who will never
    /// actually accept — the starvation pattern where one unrealistic
    /// star blocked the whole request day after day.
    ///
    /// Weights: quality fit 0.30, locality 0.20, career pressure
    /// 0.20, rep closeness 0.15, wage affordability 0.15. Quality
    /// still matters most, but the four "will this deal actually
    /// happen" signals together dominate it.
    pub fn candidate_priority_score(
        quality_fit: f32,
        domestic: bool,
        same_continent: bool,
        rep_mismatch: i32,
        career_pressure: f32,
        wage_affordability: f32,
    ) -> f32 {
        let locality = if domestic {
            1.0
        } else if same_continent {
            0.60
        } else {
            0.25
        };
        let rep_closeness = 1.0 - (rep_mismatch.unsigned_abs() as f32 / 5000.0).clamp(0.0, 1.0);
        0.30 * quality_fit.clamp(0.0, 1.0)
            + 0.20 * locality
            + 0.20 * career_pressure.clamp(0.0, 1.0)
            + 0.15 * rep_closeness
            + 0.15 * wage_affordability.clamp(0.0, 1.0)
    }

    /// Daily signing chance percentage (clamped 0.5..35.0). Replaces the
    /// CA-band-only `TransferConfig::daily_signing_chance` for free
    /// agents because elite players still move quickly while weak
    /// players eventually move when career pressure lifts the floor.
    /// Pressure slope 10.0 (was 8.0): a fully-pressured journeyman
    /// fields an approach roughly every week instead of every
    /// fortnight, which is what lets the 12-month resolution targets
    /// hold without touching the realism gates.
    pub fn daily_signing_chance(career_pressure: f32, ca: u8, urgency_bonus: f32) -> f32 {
        let cp = career_pressure.clamp(0.0, 1.0);
        let raw = 0.5 + 10.0 * cp + 0.04 * (ca as f32) + urgency_bonus.max(0.0);
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
            0.05
        } else {
            0.12
        };
        let quality_factor = if ca < 50 {
            0.10
        } else if ca < 70 {
            0.06
        } else {
            0.00
        };
        let rep_factor = (world_reputation.max(0) as f32 / 10_000.0) * 0.08;
        // Old + low-quality stacks to ~0.34/month by 18 months free,
        // so that cohort usually resolves inside the 18-24 month
        // window; the deterministic bound below is the backstop, not
        // the common path.
        (base + age_factor + quality_factor - rep_factor).clamp(0.0, 0.40)
    }

    /// Hard upper bound, in months of unemployment, after which a
    /// free agent retires deterministically — the backstop that
    /// guarantees no player sits in the pool for multiple seasons
    /// just because the monthly probability rolls kept missing.
    ///
    /// `observable_ceiling` is the staff-free potential proxy
    /// (`PotentialEstimator::observable_ceiling`), never the hidden
    /// biological PA — the bound is a market judgement, and the
    /// market only sees observable promise.
    ///
    ///   - young with visible growth room: 48 months (they keep
    ///     training and hoping; the market-clearing pass normally
    ///     resolves them long before this)
    ///   - renowned veterans (world rep ≥ 6000): 42 months — still
    ///     names, clubs keep calling, but they too eventually stop
    ///   - baseline: 36 months
    ///   - old (33+) or low-quality (CA < 60): −6 months each,
    ///     floored at 24
    pub fn deterministic_retirement_months(
        age: u8,
        ca: u8,
        observable_ceiling: u8,
        world_reputation: i16,
    ) -> u32 {
        if age < 24 && observable_ceiling >= ca.saturating_add(15) {
            return 48;
        }
        if world_reputation >= 6000 {
            return 42;
        }
        let mut months: i32 = 36;
        if age >= 33 {
            months -= 6;
        }
        if ca < 60 {
            months -= 6;
        }
        months.max(24) as u32
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
        assert!(
            desperate > fresh + 4000,
            "fresh={fresh} desperate={desperate}"
        );
    }

    #[test]
    fn region_drop_increases_with_pressure() {
        let low = FreeAgentMarketCalculator::region_drop_allowed(0.0);
        let high = FreeAgentMarketCalculator::region_drop_allowed(1.0);
        assert!(high > low + 0.40);
    }

    #[test]
    fn cross_continent_blocks_russian_to_algerian_at_low_pressure() {
        // EasternEurope prestige 0.50 → NorthAfrica prestige 0.25,
        // cross-continent, mid pressure. Must block.
        assert!(FreeAgentMarketCalculator::cross_continent_blocked(
            false, 0.50, 0.25, 0.4, 0.85,
        ));
    }

    #[test]
    fn cross_continent_passes_russian_to_algerian_at_very_high_pressure() {
        // Same step-down but the player is on the verge of retiring
        // — gate unlocks.
        assert!(!FreeAgentMarketCalculator::cross_continent_blocked(
            false, 0.50, 0.25, 0.90, 0.85,
        ));
    }

    #[test]
    fn cross_continent_allows_small_drop() {
        // EasternEurope 0.50 → MiddleEastEurope 0.40 is only a 0.10
        // step. Cross-continent but small drop — always allowed.
        assert!(!FreeAgentMarketCalculator::cross_continent_blocked(
            false, 0.50, 0.40, 0.0, 0.85,
        ));
    }

    #[test]
    fn cross_continent_does_not_block_same_continent() {
        // Big prestige drop but same continent (e.g. WesternEurope
        // 1.00 → EasternEurope 0.50) — gate stays silent.
        assert!(!FreeAgentMarketCalculator::cross_continent_blocked(
            true, 1.00, 0.50, 0.0, 0.85,
        ));
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
        let old_weak = FreeAgentMarketCalculator::retirement_probability_per_month(6, 36, 50, 0);
        assert!(old_weak > young_strong);
    }

    #[test]
    fn high_pressure_low_ca_player_can_sign_below_previous_reputation_tier() {
        // 34yo CA-55 journeyman a year on the market: a buyer country
        // 3000 points below his reference market must clear the rep
        // gate at his pressure...
        let desperate_drop = FreeAgentMarketCalculator::rep_drop_allowed(0.9, 34, 55);
        assert!(
            1500 + desperate_drop >= 4500,
            "desperate journeyman must reach 3000-point step-downs, drop={desperate_drop}"
        );
        // ...while the same gap stays closed for a fresh prime-age
        // quality player — pressure, not time alone, opens doors.
        let fresh_drop = FreeAgentMarketCalculator::rep_drop_allowed(0.05, 27, 110);
        assert!(
            1500 + fresh_drop < 4500,
            "fresh quality player must not step down 3000 points, drop={fresh_drop}"
        );
    }

    #[test]
    fn deterministic_retirement_bound_orders_cohorts_sensibly() {
        // Old + low quality resolves fastest.
        assert_eq!(
            FreeAgentMarketCalculator::deterministic_retirement_months(36, 50, 50, 0),
            24
        );
        // Baseline journeyman.
        assert_eq!(
            FreeAgentMarketCalculator::deterministic_retirement_months(29, 80, 85, 2000),
            36
        );
        // Renowned veterans wait longer but still resolve.
        assert_eq!(
            FreeAgentMarketCalculator::deterministic_retirement_months(33, 120, 120, 8000),
            42
        );
        // Young with visible growth room gets the longest leash.
        assert_eq!(
            FreeAgentMarketCalculator::deterministic_retirement_months(21, 70, 120, 1000),
            48
        );
    }

    #[test]
    fn candidate_priority_prefers_signable_local_over_raw_quality() {
        // The starvation case: a perfectly-fitting but unaffordable,
        // pressure-free foreigner versus a domestic journeyman under
        // real pressure with an affordable wage ask. The journeyman
        // must rank first even with a slightly worse quality fit.
        let unrealistic_star = FreeAgentMarketCalculator::candidate_priority_score(
            1.0,   // quality fit
            false, // domestic
            true,  // same continent
            1200,  // rep mismatch
            0.0,   // career pressure
            0.0,   // wage affordability
        );
        let signable_local =
            FreeAgentMarketCalculator::candidate_priority_score(1.0, true, true, 1500, 0.7, 0.8);
        assert!(
            signable_local > unrealistic_star,
            "local={signable_local} star={unrealistic_star}"
        );
    }

    #[test]
    fn offer_wage_does_not_panic_when_country_floor_exceeds_market_ceiling() {
        // Low-ability player whose computed market wage is below the buyer
        // country's minimum professional wage. Pre-fix this hit
        // `f32::clamp(floor, ceiling)` with floor > ceiling and panicked.
        let offer = FreeAgentMarketCalculator::offer_wage(
            4_329, // market_wage — 2.5× = 10_822.5
            BuyerRoleFit::Backup,
            10,
            4_768, // buyer_country_reputation → floor 18_304
            5_000,
            0.5,
        );
        assert!(offer >= 4_000);
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
