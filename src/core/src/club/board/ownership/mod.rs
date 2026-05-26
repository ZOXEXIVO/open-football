//! Ownership model — who actually owns the club and how they exercise
//! power. This sits *alongside* the legacy `ChairmanProfile` (which keeps
//! `ambition`, `patience`, and `manager_loyalty` for backward-compatible
//! callers) and adds the richer governance knobs a real board needs:
//! wealth, interference, risk appetite, and exit pressure.
//!
//! Every field here feeds at least one downstream calculation — budget
//! sizing, transfer governance, facility approvals, pressure response, or
//! takeover behaviour. Nothing is inert.

/// Who owns the club. Each archetype biases governance differently:
/// member-owned clubs answer to supporters, state-backed owners chase
/// trophies regardless of cash, private equity obsesses over resale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OwnershipType {
    /// Fan/member owned (Socios model). Reacts hardest to supporter mood,
    /// allergic to debt and unpopular sales.
    MemberOwned,
    /// A single local businessperson. Prudent, modest means.
    #[default]
    LocalBusiness,
    /// A group of investors. Pragmatic, balance-sheet focused.
    Consortium,
    /// Sovereign / state-backed wealth. Deep pockets, trophy-hungry,
    /// tolerant of short-term losses.
    StateBacked,
    /// Private-equity / leveraged owner. Resale value and wage control
    /// above all; willing to load debt.
    PrivateEquity,
    /// Old-money family dynasty. Stability prized, slow to change.
    FamilyOwned,
}

impl OwnershipType {
    /// How strongly the owner weights supporter sentiment when forming
    /// confidence and calling meetings. 0.0 = ignores fans entirely.
    pub fn supporter_sensitivity(self) -> f32 {
        match self {
            OwnershipType::MemberOwned => 1.0,
            OwnershipType::FamilyOwned => 0.75,
            OwnershipType::LocalBusiness => 0.7,
            OwnershipType::Consortium => 0.5,
            OwnershipType::PrivateEquity => 0.35,
            OwnershipType::StateBacked => 0.4,
        }
    }

    /// True when the owner expects silverware as the baseline and will
    /// bankroll short-term losses to get it.
    pub fn trophy_hungry(self) -> bool {
        matches!(self, OwnershipType::StateBacked)
    }

    /// True when resale value / wage discipline dominate transfer thinking.
    pub fn resale_driven(self) -> bool {
        matches!(self, OwnershipType::PrivateEquity)
    }
}

/// Persistent ownership submodel. Knobs are 0-100 so they compose into
/// smooth multipliers rather than hard switches.
#[derive(Debug, Clone)]
pub struct OwnershipModel {
    pub ownership_type: OwnershipType,
    /// Spending power independent of current cash — a rich owner can
    /// inject funds, a poor one cannot. Drives budget injections and
    /// facility approvals.
    pub wealth: u8,
    /// How much the owner meddles: forced signings, overriding the
    /// manager, demanding star buys. High interference lowers manager
    /// autonomy.
    pub interference: u8,
    /// Appetite for debt, high wage ratios, and speculative fees.
    pub risk_tolerance: u8,
    /// Desire to sell up / walk away. Rises with sustained losses and
    /// fan revolt; gates takeover rumours.
    pub exit_pressure: u8,
}

impl Default for OwnershipModel {
    fn default() -> Self {
        // Neutral owner — every multiplier resolves to ~1.0 so legacy
        // budget/governance tests built on `ClubBoard::new()` are
        // unaffected.
        OwnershipModel {
            ownership_type: OwnershipType::LocalBusiness,
            wealth: 50,
            interference: 35,
            risk_tolerance: 50,
            exit_pressure: 10,
        }
    }
}

impl OwnershipModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Budget multiplier from wealth + risk appetite. Neutral (wealth 50,
    /// risk 50) returns 1.0; a deep-pocketed risk-taker can roughly double
    /// the war chest, a cautious pauper halves it. Multiplicative so it
    /// never resurrects a zero free-cash budget.
    pub fn budget_multiplier(&self) -> f64 {
        let wealth = self.wealth as f64 / 50.0; // 0..2, neutral 1.0
        let risk = 0.75 + (self.risk_tolerance as f64 / 100.0) * 0.5; // 0.75..1.25
        // Blend so neither knob alone dominates; clamp to a sane band.
        ((wealth * 0.6 + 0.4) * risk).clamp(0.4, 2.2)
    }

    /// Extra wage-to-revenue headroom the owner will sanction, in ratio
    /// points. A risk-loving wealthy owner lets wages run hotter.
    pub fn wage_ratio_bonus(&self) -> f64 {
        let risk = (self.risk_tolerance as f64 - 50.0) / 50.0; // -1..1
        (risk * 0.08).clamp(-0.06, 0.10)
    }

    /// Manager transfer/selection autonomy after interference. 1.0 = full
    /// autonomy, lower = the owner pulls strings.
    pub fn autonomy_factor(&self) -> f32 {
        1.0 - (self.interference as f32 / 100.0) * 0.6
    }

    /// Probability weight (0..1) that the owner injects cash on a strong
    /// season rather than banking it. Wealth + risk driven.
    pub fn injection_appetite(&self) -> f32 {
        ((self.wealth as f32 * 0.6 + self.risk_tolerance as f32 * 0.4) / 100.0).clamp(0.0, 1.0)
    }

    /// Derive a coherent ownership archetype from durable club signals.
    /// Deterministic given the same inputs — `seed` (use the club id)
    /// spreads clubs of similar size across plausible archetypes without
    /// any hard-coded names.
    ///
    /// * `reputation` — main-team `overall_score()` 0..1.
    /// * `balance` — current cash; large negatives hint at leveraged or
    ///   distressed ownership.
    /// * `economic_factor` — country TV/wealth multiplier; richer leagues
    ///   attract richer owners.
    pub fn derive(reputation: f32, balance: i64, economic_factor: f32, seed: u32) -> Self {
        let bucket = seed % 5;

        // Elite clubs in wealthy leagues skew towards moneyed owners.
        let ownership_type = if reputation >= 0.8 {
            match bucket {
                0 | 1 => OwnershipType::StateBacked,
                2 => OwnershipType::PrivateEquity,
                3 => OwnershipType::Consortium,
                _ => OwnershipType::FamilyOwned,
            }
        } else if reputation >= 0.55 {
            match bucket {
                0 => OwnershipType::Consortium,
                1 => OwnershipType::PrivateEquity,
                2 => OwnershipType::FamilyOwned,
                3 => OwnershipType::LocalBusiness,
                _ => OwnershipType::MemberOwned,
            }
        } else {
            match bucket {
                0 | 1 => OwnershipType::LocalBusiness,
                2 => OwnershipType::MemberOwned,
                3 => OwnershipType::FamilyOwned,
                _ => OwnershipType::Consortium,
            }
        };

        // Wealth tracks reputation and league money, nudged by archetype.
        let rep_w = (reputation * 60.0) as i32;
        let eco_w = ((economic_factor - 1.0) * 25.0) as i32;
        let type_w = match ownership_type {
            OwnershipType::StateBacked => 35,
            OwnershipType::PrivateEquity => 20,
            OwnershipType::Consortium => 12,
            OwnershipType::FamilyOwned => 5,
            OwnershipType::LocalBusiness => -5,
            OwnershipType::MemberOwned => -10,
        };
        let wealth = (25 + rep_w + eco_w + type_w).clamp(5, 100) as u8;

        let risk_tolerance = match ownership_type {
            OwnershipType::StateBacked => 85,
            OwnershipType::PrivateEquity => 70,
            OwnershipType::Consortium => 55,
            OwnershipType::LocalBusiness => 45,
            OwnershipType::FamilyOwned => 35,
            OwnershipType::MemberOwned => 25,
        };

        let interference = match ownership_type {
            OwnershipType::StateBacked => 75,
            OwnershipType::PrivateEquity => 55,
            OwnershipType::LocalBusiness => 45,
            OwnershipType::Consortium => 35,
            OwnershipType::FamilyOwned => 40,
            OwnershipType::MemberOwned => 20,
        };

        // Distressed balance (deep negative relative to nothing else we
        // know here) primes exit pressure a little.
        let exit_pressure = if balance < -50_000_000 {
            30
        } else if balance < 0 {
            18
        } else {
            10
        };

        OwnershipModel {
            ownership_type,
            wealth,
            interference,
            risk_tolerance,
            exit_pressure,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_owner_is_budget_identity() {
        let m = OwnershipModel::default();
        let mult = m.budget_multiplier();
        assert!(
            (mult - 1.0).abs() < 0.06,
            "neutral owner should be ~1.0 budget multiplier, got {mult}"
        );
    }

    #[test]
    fn wealthy_risk_taker_spends_more_than_cautious_pauper() {
        let rich = OwnershipModel {
            wealth: 95,
            risk_tolerance: 90,
            ..Default::default()
        };
        let poor = OwnershipModel {
            wealth: 15,
            risk_tolerance: 20,
            ..Default::default()
        };
        assert!(rich.budget_multiplier() > poor.budget_multiplier() * 1.5);
    }

    #[test]
    fn interference_lowers_autonomy() {
        let meddler = OwnershipModel {
            interference: 100,
            ..Default::default()
        };
        let hands_off = OwnershipModel {
            interference: 0,
            ..Default::default()
        };
        assert!(meddler.autonomy_factor() < hands_off.autonomy_factor());
        assert!(hands_off.autonomy_factor() >= 0.99);
    }

    #[test]
    fn elite_wealthy_league_derives_rich_owner() {
        // Sample all seed buckets — elite clubs should always land a
        // high-wealth owner regardless of the archetype bucket.
        for seed in 0..5u32 {
            let m = OwnershipModel::derive(0.9, 100_000_000, 1.5, seed);
            assert!(m.wealth >= 70, "elite owner wealth too low: {}", m.wealth);
        }
    }

    #[test]
    fn small_club_derives_modest_owner() {
        let m = OwnershipModel::derive(0.2, 0, 0.8, 0);
        assert!(m.wealth <= 55, "small club owner too rich: {}", m.wealth);
        assert!(matches!(
            m.ownership_type,
            OwnershipType::LocalBusiness | OwnershipType::MemberOwned | OwnershipType::FamilyOwned
        ));
    }

    #[test]
    fn derive_is_deterministic() {
        let a = OwnershipModel::derive(0.6, 5_000_000, 1.2, 42);
        let b = OwnershipModel::derive(0.6, 5_000_000, 1.2, 42);
        assert_eq!(a.ownership_type, b.ownership_type);
        assert_eq!(a.wealth, b.wealth);
        assert_eq!(a.risk_tolerance, b.risk_tolerance);
    }
}
