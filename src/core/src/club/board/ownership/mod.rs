//! Ownership model — who actually owns the club and how they exercise
//! power. This sits *alongside* the legacy `ChairmanProfile` (which keeps
//! `ambition`, `patience`, and `manager_loyalty` for backward-compatible
//! callers) and adds the richer governance knobs a real board needs:
//! wealth, interference, risk appetite, and exit pressure.
//!
//! Every field here feeds at least one downstream calculation — budget
//! sizing, transfer governance, facility approvals, pressure response, or
//! takeover behaviour. Nothing is inert.

pub mod benefactor;

pub use benefactor::ClubBenefactor;

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

    /// Appetite for debt, high wage ratios and speculative fees, by
    /// archetype. The one table, so a club whose ownership FLIPS gets the
    /// new archetype's risk rather than keeping the old one's.
    pub fn risk_tolerance(self) -> u8 {
        match self {
            OwnershipType::StateBacked => 85,
            OwnershipType::PrivateEquity => 70,
            OwnershipType::Consortium => 55,
            OwnershipType::LocalBusiness => 45,
            OwnershipType::FamilyOwned => 35,
            OwnershipType::MemberOwned => 25,
        }
    }

    /// How much the owner meddles, by archetype.
    pub fn interference(self) -> u8 {
        match self {
            OwnershipType::StateBacked => 75,
            OwnershipType::PrivateEquity => 55,
            OwnershipType::LocalBusiness => 45,
            OwnershipType::Consortium => 35,
            OwnershipType::FamilyOwned => 40,
            OwnershipType::MemberOwned => 20,
        }
    }

    /// Wealth points the archetype itself is worth, before reputation,
    /// league money and any owner cheque.
    pub fn wealth_bias(self) -> i32 {
        match self {
            OwnershipType::StateBacked => 35,
            OwnershipType::PrivateEquity => 20,
            OwnershipType::Consortium => 12,
            OwnershipType::FamilyOwned => 5,
            OwnershipType::LocalBusiness => -5,
            OwnershipType::MemberOwned => -10,
        }
    }
}

/// Persistent ownership submodel. Knobs are 0-100 so they compose into
/// smooth multipliers rather than hard switches.
#[derive(Debug, Clone)]
pub struct OwnershipModel {
    pub ownership_type: OwnershipType,
    /// Spending power the club's standing and its owner's archetype imply,
    /// before any cheque he is actually writing. Independent of current
    /// cash — a rich owner can inject funds, a poor one cannot.
    ///
    /// Two fields, not one running total: the owner's contribution
    /// ([`Self::benefactor_wealth`]) is DERIVED from
    /// [`Self::benefactor`] and added on read. Folding it in and
    /// subtracting it back out each season was lossy in both directions —
    /// a club deriving at 140 clamped to 100 and then gave back 40 to
    /// reach 60, although with no owner at all it would have been 100.
    pub base_wealth: u8,
    /// How much the owner meddles: forced signings, overriding the
    /// manager, demanding star buys. High interference lowers manager
    /// autonomy.
    pub interference: u8,
    /// Appetite for debt, high wage ratios, and speculative fees.
    pub risk_tolerance: u8,
    /// Desire to sell up / walk away. Rises with sustained losses and
    /// fan revolt; gates takeover rumours.
    pub exit_pressure: u8,
    /// How much of this club's spending its OWNER, rather than its
    /// revenue, is paying for — 0..1 from [`ClubBenefactor::signal`].
    ///
    /// Unlike the four knobs above this one moves: it is refreshed once a
    /// season as an EMA of the club's own cash-against-income ratio, so a
    /// club that spends its pile keeps the habit for a while and one whose
    /// owner walks away loses it slowly. Everything the money buys — the
    /// wage subsidy, the fee headroom, the state-backed archetype at a
    /// small league's reputation — hangs off this number and nothing else.
    pub benefactor: f32,
    /// Idle cash the owner's pile held when the club was first read as
    /// owner-funded — the high-water mark the yearly top-up refills
    /// towards. Re-stamped when a fresh owner arrives.
    ///
    /// Without it the subsidy was a ≈ 35 %/yr geometric drain plus fees:
    /// three windows, then an ordinary club still wearing a `StateBacked`
    /// label. A benefactor keeps writing cheques; that is what makes him
    /// one.
    pub idle_at_derive: f64,
}

impl Default for OwnershipModel {
    fn default() -> Self {
        // Neutral owner — every multiplier resolves to ~1.0 so legacy
        // budget/governance tests built on `ClubBoard::new()` are
        // unaffected.
        OwnershipModel {
            ownership_type: OwnershipType::LocalBusiness,
            base_wealth: 50,
            interference: 35,
            risk_tolerance: 50,
            exit_pressure: 10,
            benefactor: 0.0,
            idle_at_derive: 0.0,
        }
    }
}

impl OwnershipModel {
    /// Appetite an owner writing cheques adds directly, at full
    /// [`Self::benefactor`]. Sized to the +0.24 the old wealth loop
    /// produced, so the population number is unchanged while the loop is
    /// gone.
    const BENEFACTOR_APPETITE: f32 = 0.25;

    /// Share of the gap back to [`Self::idle_at_derive`] a committed owner
    /// refills each year. Half, so a club that spends its pile is topped
    /// up rather than made whole — the pile still falls under sustained
    /// spending, which is the point of "the drain".
    const TOP_UP_SHARE: f64 = 0.5;

    /// Exit pressure past which an owner is looking for the door and stops
    /// writing cheques.
    const TOP_UP_EXIT_BAR: u8 = 40;

    pub fn new() -> Self {
        Self::default()
    }

    /// Spending power all in: the archetype-and-standing part plus what
    /// the owner is actually funding. Clamped on read, never stored, so
    /// nothing ratchets and nothing is lost to a clamp.
    #[inline]
    pub fn wealth(&self) -> u8 {
        (self.base_wealth as i32 + Self::benefactor_wealth(self.benefactor)).clamp(5, 100) as u8
    }

    /// Budget multiplier from wealth + risk appetite. Neutral (wealth 50,
    /// risk 50) returns 1.0; a deep-pocketed risk-taker can roughly double
    /// the war chest, a cautious pauper halves it. Multiplicative so it
    /// never resurrects a zero free-cash budget.
    pub fn budget_multiplier(&self) -> f64 {
        let wealth = self.wealth() as f64 / 50.0; // 0..2, neutral 1.0
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

    /// How much of the owner's own money the subsidy may draw on, 0..1.
    ///
    /// Reads `base_wealth`, NOT [`Self::wealth`]. The subsidy is already
    /// `benefactor × appetite × idle × 0.35`; letting the appetite read a
    /// wealth figure the benefactor itself inflates put the ratio into the
    /// term twice — a +0.24 appetite bump feeding a +40 % subsidy on the
    /// same reading. The owner's own funding enters ONCE, as an explicit
    /// term of its own.
    pub fn injection_appetite(&self) -> f32 {
        ((self.base_wealth as f32 * 0.6 + self.risk_tolerance as f32 * 0.4) / 100.0
            + Self::BENEFACTOR_APPETITE * self.benefactor.clamp(0.0, 1.0))
        .clamp(0.0, 1.0)
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
    /// * `benefactor` — 0..1 from [`ClubBenefactor::signal`]. Cash the
    ///   club's own revenue cannot explain. Past
    ///   [`ClubBenefactor::STATE_BACKED_BAR`] it makes the club
    ///   state-backed at ANY reputation, which is the whole point: a
    ///   5200-reputation league's richest side is not a local business
    ///   just because its league is small, and the old
    ///   `reputation >= 0.8` rule said it was.
    pub fn derive(
        reputation: f32,
        balance: i64,
        economic_factor: f32,
        benefactor: f32,
        seed: u32,
    ) -> Self {
        let bucket = seed % 5;
        let benefactor = benefactor.clamp(0.0, 1.0);

        // Elite clubs in wealthy leagues skew towards moneyed owners.
        let ownership_type = if benefactor >= ClubBenefactor::STATE_BACKED_BAR {
            OwnershipType::StateBacked
        } else if reputation >= 0.8 {
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
        // The owner's own cheque is NOT folded in here — it is added on
        // read by [`Self::wealth`], so nothing ratchets and nothing is
        // clamped away.
        let rep_w = (reputation * 60.0) as i32;
        let eco_w = ((economic_factor - 1.0) * 25.0) as i32;
        let base_wealth =
            (25 + rep_w + eco_w + ownership_type.wealth_bias()).clamp(5, 100) as u8;

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
            base_wealth,
            interference: ownership_type.interference(),
            risk_tolerance: ownership_type.risk_tolerance(),
            exit_pressure,
            benefactor,
            idle_at_derive: 0.0,
        }
    }

    /// What the owner will put into the wage bill this year, in currency.
    #[inline]
    pub fn owner_subsidy_per_year(&self, idle_cash: f64) -> f64 {
        ClubBenefactor::subsidy_per_year(self.benefactor, self.injection_appetite(), idle_cash)
    }

    /// Fold this season's cash-against-income reading into the stored one,
    /// and let the archetype follow it up.
    ///
    /// **Flips persist.** An owner who starts funding a club is a
    /// state-backed owner from then on: ownership is derived once and
    /// persists, and a club that spends its pile does not stop having the
    /// owner who paid for it — that is what the three-season EMA half-life
    /// on [`Self::benefactor`] is for, and it is the number every
    /// consequence actually reads. What the flip must NOT leave behind is
    /// a half-derived model, so the archetype's own risk, interference and
    /// wealth bias are re-read from the same tables `derive` uses.
    ///
    /// Returns `true` when the archetype changed, so the board can re-map
    /// its chairman knobs from the new one.
    pub fn refresh_benefactor(
        &mut self,
        balance: i64,
        annual_wages: i64,
        annual_income: i64,
    ) -> bool {
        let signal = ClubBenefactor::signal(balance, annual_wages, annual_income);
        self.benefactor = ClubBenefactor::blend(self.benefactor, signal);
        if self.benefactor < ClubBenefactor::STATE_BACKED_BAR
            || self.ownership_type == OwnershipType::StateBacked
        {
            return false;
        }
        let was = self.ownership_type;
        self.ownership_type = OwnershipType::StateBacked;
        self.base_wealth = (self.base_wealth as i32 - was.wealth_bias()
            + OwnershipType::StateBacked.wealth_bias())
        .clamp(5, 100) as u8;
        self.risk_tolerance = OwnershipType::StateBacked.risk_tolerance();
        self.interference = OwnershipType::StateBacked.interference();
        // A new owner arrives with a pile of his own; the top-up refills
        // towards what he brought, not towards what the last one had.
        self.idle_at_derive = ClubBenefactor::idle_cash(balance, annual_wages);
        true
    }

    /// Record the pile the owner was first seen holding — the high-water
    /// mark [`Self::annual_top_up`] refills towards.
    #[inline]
    pub fn stamp_idle_at_derive(&mut self, idle_cash: f64) {
        self.idle_at_derive = idle_cash.max(0.0);
    }

    /// What a committed owner puts back into the pile this year.
    ///
    /// Bounded three ways — by how owner-funded the club is at all, by his
    /// appetite for writing cheques, and by half the gap back to the pile
    /// he arrived with — so it is a top-up, never free money: the balance
    /// can approach `idle_at_derive` and never passes it.
    ///
    /// Nothing for an owner heading for the exit, and nothing for a club
    /// in emergency measures: the shortfall there is already covered by
    /// [`crate::club::finance::DebtProfile::owner_injection`], and paying
    /// twice would be an owner bailing out a club he is walking away from.
    pub fn annual_top_up(&self, idle_now: f64, in_emergency: bool) -> f64 {
        if in_emergency
            || self.ownership_type != OwnershipType::StateBacked
            || self.benefactor < ClubBenefactor::STATE_BACKED_BAR
            || self.exit_pressure >= Self::TOP_UP_EXIT_BAR
        {
            return 0.0;
        }
        let gap = (self.idle_at_derive - idle_now.max(0.0)).max(0.0);
        (self.benefactor.clamp(0.0, 1.0) * self.injection_appetite()) as f64
            * gap
            * Self::TOP_UP_SHARE
    }

    /// Wealth points an owner's funding is worth. An owner writing cheques
    /// the revenue cannot cover IS wealth, and it is the only kind a small
    /// league's giant has.
    #[inline]
    fn benefactor_wealth(benefactor: f32) -> i32 {
        (40.0 * benefactor.clamp(0.0, 1.0)) as i32
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
            base_wealth: 95,
            risk_tolerance: 90,
            ..Default::default()
        };
        let poor = OwnershipModel {
            base_wealth: 15,
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
            let m = OwnershipModel::derive(0.9, 100_000_000, 1.5, 0.0, seed);
            assert!(m.wealth() >= 70, "elite owner wealth too low: {}", m.wealth());
        }
    }

    #[test]
    fn small_club_derives_modest_owner() {
        let m = OwnershipModel::derive(0.2, 0, 0.8, 0.0, 0);
        assert!(m.wealth() <= 55, "small club owner too rich: {}", m.wealth());
        assert!(matches!(
            m.ownership_type,
            OwnershipType::LocalBusiness | OwnershipType::MemberOwned | OwnershipType::FamilyOwned
        ));
    }

    #[test]
    fn derive_is_deterministic() {
        let a = OwnershipModel::derive(0.6, 5_000_000, 1.2, 0.0, 42);
        let b = OwnershipModel::derive(0.6, 5_000_000, 1.2, 0.0, 42);
        assert_eq!(a.ownership_type, b.ownership_type);
        assert_eq!(a.wealth(), b.wealth());
        assert_eq!(a.risk_tolerance, b.risk_tolerance);
    }
}
