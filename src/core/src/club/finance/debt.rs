//! Club debt, insolvency, and administration.
//!
//! The previous model charged 0.6–1.5% *per month* on whatever the cash
//! balance happened to be, added the interest straight back onto that same
//! balance, and charged it again next month. Nothing capped it and nothing
//! ever resolved it: there was no bankruptcy, no owner bailout, no
//! restructuring. A club that dipped into the red compounded at 7–20% a
//! year forever, which is how clubs reached balances of minus one and a half
//! billion — a number no football club has ever approached because in
//! reality something always intervenes first.
//!
//! This module supplies the interventions, in the order a real club meets
//! them:
//!
//! 1. **Overdraft.** Debt inside an agreed facility is ordinary borrowing at
//!    an ordinary rate.
//! 2. **Owner injection.** Past the facility, a solvent owner puts money in
//!    rather than watch the club fail. Wealth and risk appetite decide how
//!    much and how willingly.
//! 3. **Emergency measures.** No transfer spending, forced wage reduction,
//!    players listed. The club is trading its way out.
//! 4. **Administration.** The terminal state: a points deduction, a transfer
//!    embargo, and a write-down of the unpayable debt to something the club
//!    can actually service. Painful and slow to exit — but finite.

use super::balance::DistressLevel;

/// Where a club sits on the debt ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DebtStanding {
    /// Positive balance, or borrowing comfortably inside the facility.
    #[default]
    Solvent,
    /// Inside the facility but deep enough that the board is watching.
    Leveraged,
    /// Past the facility. The owner is being asked for money.
    OwnerFunded,
    /// Owner won't or can't cover it. Emergency trading measures.
    Emergency,
    /// In administration: points deducted, embargoed, debt being written
    /// down.
    Administration,
}

impl DebtStanding {
    /// True when the club may not spend a fee on a player.
    pub fn blocks_transfer_spending(self) -> bool {
        matches!(self, DebtStanding::Emergency | DebtStanding::Administration)
    }

    /// True when the club must actively shed wages rather than merely
    /// hold them flat.
    pub fn forces_wage_reduction(self) -> bool {
        matches!(
            self,
            DebtStanding::OwnerFunded | DebtStanding::Emergency | DebtStanding::Administration
        )
    }

    /// Ceiling on wages as a share of revenue at this standing. The board's
    /// budget calculation takes the tighter of this and its own target.
    pub fn wage_ratio_ceiling(self) -> f64 {
        match self {
            DebtStanding::Solvent => 0.70,
            DebtStanding::Leveraged => 0.62,
            DebtStanding::OwnerFunded => 0.55,
            DebtStanding::Emergency => 0.45,
            DebtStanding::Administration => 0.35,
        }
    }
}

/// An active administration: what it costs and how long it lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdministrationState {
    /// League points deducted this season as a result of entering.
    pub points_deduction: u8,
    /// Months left before the club can exit.
    pub months_remaining: u8,
    /// Whether the points deduction has been handed to the league yet.
    pub deduction_applied: bool,
}

impl AdministrationState {
    /// Standard sanction on entry: a points deduction and a year under
    /// supervision.
    pub fn enter(league_tier: u8) -> Self {
        // Top flights sanction harder — the competitive advantage of
        // outspending everyone and then writing the debt off is larger.
        let points_deduction = if league_tier <= 1 { 12 } else { 9 };
        AdministrationState {
            points_deduction,
            months_remaining: 12,
            deduction_applied: false,
        }
    }

    /// Tick one month. Returns true once the club may exit.
    pub fn tick(&mut self) -> bool {
        self.months_remaining = self.months_remaining.saturating_sub(1);
        self.months_remaining == 0
    }
}

/// A club's debt position, evaluated fresh each month.
#[derive(Debug, Clone, Copy, Default)]
pub struct DebtProfile {
    pub standing: DebtStanding,
    pub administration: Option<AdministrationState>,
}

impl DebtProfile {
    /// Borrowing a club can carry against its revenue before the facility is
    /// exhausted. Real clubs run leveraged — 1.5× annual turnover is a heavy
    /// but survivable book — and the absolute floor keeps a tiny club from
    /// being declared insolvent over a rounding error.
    const FACILITY_REVENUE_MULTIPLE: f64 = 1.5;
    const FACILITY_FLOOR: i64 = 2_000_000;

    /// Agreed overdraft facility for a club with this trailing revenue.
    pub fn facility_limit(trailing_annual_income: i64) -> i64 {
        ((trailing_annual_income.max(0) as f64 * Self::FACILITY_REVENUE_MULTIPLE) as i64)
            .max(Self::FACILITY_FLOOR)
    }

    /// Annual interest rate on borrowing. Rises with distress the way a real
    /// facility reprices, but stays in single-to-low-double digits — the old
    /// model's effective 20%/yr on an uncapped balance was the engine of the
    /// spiral.
    fn annual_rate(standing: DebtStanding) -> f64 {
        match standing {
            DebtStanding::Solvent | DebtStanding::Leveraged => 0.055,
            DebtStanding::OwnerFunded => 0.070,
            DebtStanding::Emergency => 0.090,
            // Administration freezes interest: that is the point of the
            // process. Charging a club in administration is what made the
            // old model unable to ever resolve a debt.
            DebtStanding::Administration => 0.0,
        }
    }

    /// Monthly interest charge on a negative balance.
    ///
    /// Interest is charged only on debt **within** the facility. Borrowing
    /// beyond the facility is not a bank loan the club is servicing — it is
    /// a shortfall the owner covers or the club goes into administration
    /// over. Charging interest on it was the compounding term that let the
    /// balance run away.
    pub fn monthly_interest(&self, balance: i64, trailing_annual_income: i64) -> i64 {
        if balance >= 0 {
            return 0;
        }
        let debt = -balance;
        let serviced = debt.min(Self::facility_limit(trailing_annual_income));
        let rate = Self::annual_rate(self.standing) / 12.0;
        (serviced as f64 * rate) as i64
    }

    /// Classify the club's standing from its balance, revenue, and any
    /// administration already in force.
    pub fn classify(
        balance: i64,
        trailing_annual_income: i64,
        distress: DistressLevel,
        administration: Option<AdministrationState>,
    ) -> DebtStanding {
        if administration.is_some() {
            return DebtStanding::Administration;
        }
        if balance >= 0 {
            return DebtStanding::Solvent;
        }
        let facility = Self::facility_limit(trailing_annual_income);
        let debt = -balance;

        if debt <= facility / 2 {
            return DebtStanding::Solvent;
        }
        if debt <= facility {
            return DebtStanding::Leveraged;
        }
        // Past the facility. How far past, and how bad the cash burn is,
        // decides whether the owner is still willing to fund it.
        if debt <= facility * 2 && !matches!(distress, DistressLevel::Insolvency) {
            return DebtStanding::OwnerFunded;
        }
        DebtStanding::Emergency
    }

    /// Cash the owner is prepared to inject this month to close a shortfall
    /// beyond the facility.
    ///
    /// `injection_appetite` is [`crate::OwnershipModel::injection_appetite`]
    /// (0..1). A committed billionaire covers the whole gap; a reluctant or
    /// cash-poor owner covers a slice, and the club slides towards
    /// administration.
    pub fn owner_injection(
        balance: i64,
        trailing_annual_income: i64,
        injection_appetite: f32,
        standing: DebtStanding,
    ) -> i64 {
        if !matches!(
            standing,
            DebtStanding::OwnerFunded | DebtStanding::Emergency
        ) {
            return 0;
        }
        let facility = Self::facility_limit(trailing_annual_income);
        let shortfall = (-balance) - facility;
        if shortfall <= 0 {
            return 0;
        }
        // Appetite scales both the willingness and the pace: nobody wires
        // the whole gap in one month, but a wealthy owner clears it quickly.
        let appetite = injection_appetite.clamp(0.0, 1.0) as f64;
        let pace = 0.15 + appetite * 0.55;
        (shortfall as f64 * pace * appetite) as i64
    }

    /// Whether a club in [`DebtStanding::Emergency`] must now enter
    /// administration.
    ///
    /// The trigger is inability to meet the wage bill from revenue while
    /// already past twice its facility — the real-world test, rather than an
    /// arbitrary balance threshold.
    pub fn should_enter_administration(
        balance: i64,
        trailing_annual_income: i64,
        trailing_annual_wages: i64,
        standing: DebtStanding,
    ) -> bool {
        if !matches!(standing, DebtStanding::Emergency) {
            return false;
        }
        let facility = Self::facility_limit(trailing_annual_income);
        if -balance <= facility * 2 {
            return false;
        }
        // Revenue cannot cover the wage bill: the club is trading while
        // insolvent.
        trailing_annual_wages > trailing_annual_income
    }

    /// Debt written down on entering administration.
    ///
    /// Creditors take a haircut and the club emerges with a balance it can
    /// service: borrowing is cut back to its facility. Returns the amount
    /// written off (a credit to the balance).
    pub fn administration_write_down(balance: i64, trailing_annual_income: i64) -> i64 {
        if balance >= 0 {
            return 0;
        }
        let target = -Self::facility_limit(trailing_annual_income);
        if balance >= target {
            return 0;
        }
        target - balance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REVENUE: i64 = 400_000_000;

    #[test]
    fn interest_is_capped_at_the_facility() {
        let profile = DebtProfile {
            standing: DebtStanding::Emergency,
            administration: None,
        };
        let facility = DebtProfile::facility_limit(REVENUE);

        // Debt far beyond the facility must not scale interest with it —
        // that unbounded term is what compounded balances to -1.5bn.
        let at_limit = profile.monthly_interest(-facility, REVENUE);
        let way_past = profile.monthly_interest(-facility * 10, REVENUE);
        assert_eq!(
            at_limit, way_past,
            "interest must stop growing once debt passes the facility"
        );
    }

    #[test]
    fn administration_freezes_interest() {
        let profile = DebtProfile {
            standing: DebtStanding::Administration,
            administration: Some(AdministrationState::enter(1)),
        };
        assert_eq!(profile.monthly_interest(-900_000_000, REVENUE), 0);
    }

    #[test]
    fn interest_rate_stays_in_a_survivable_band() {
        // The old model charged up to 18%/yr on an uncapped balance. Even
        // the worst standing here must stay in single digits annually.
        let profile = DebtProfile {
            standing: DebtStanding::Emergency,
            administration: None,
        };
        let facility = DebtProfile::facility_limit(REVENUE);
        let annual = profile.monthly_interest(-facility, REVENUE) * 12;
        let rate = annual as f64 / facility as f64;
        assert!(rate < 0.10, "annual rate of {rate:.3} is punitive");
    }

    #[test]
    fn standing_escalates_with_debt() {
        let facility = DebtProfile::facility_limit(REVENUE);
        let none = DistressLevel::None;
        assert_eq!(
            DebtProfile::classify(1_000, REVENUE, none, None),
            DebtStanding::Solvent
        );
        assert_eq!(
            DebtProfile::classify(-facility, REVENUE, none, None),
            DebtStanding::Leveraged
        );
        assert_eq!(
            DebtProfile::classify(-facility - 1, REVENUE, none, None),
            DebtStanding::OwnerFunded
        );
        assert_eq!(
            DebtProfile::classify(-facility * 3, REVENUE, DistressLevel::Insolvency, None),
            DebtStanding::Emergency
        );
    }

    #[test]
    fn administration_overrides_every_other_standing() {
        let admin = Some(AdministrationState::enter(1));
        assert_eq!(
            DebtProfile::classify(500_000_000, REVENUE, DistressLevel::None, admin),
            DebtStanding::Administration
        );
    }

    #[test]
    fn wealthy_owner_covers_the_shortfall_faster() {
        let facility = DebtProfile::facility_limit(REVENUE);
        let balance = -facility - 100_000_000;
        let committed =
            DebtProfile::owner_injection(balance, REVENUE, 0.95, DebtStanding::OwnerFunded);
        let reluctant =
            DebtProfile::owner_injection(balance, REVENUE, 0.10, DebtStanding::OwnerFunded);
        assert!(committed > reluctant * 5);
        assert!(committed <= 100_000_000);
    }

    #[test]
    fn solvent_clubs_get_no_injection() {
        assert_eq!(
            DebtProfile::owner_injection(-1_000, REVENUE, 1.0, DebtStanding::Solvent),
            0
        );
    }

    #[test]
    fn administration_writes_debt_down_to_serviceable() {
        let facility = DebtProfile::facility_limit(REVENUE);
        let balance = -1_500_000_000i64;
        let written = DebtProfile::administration_write_down(balance, REVENUE);
        assert_eq!(balance + written, -facility);
    }

    #[test]
    fn administration_is_only_for_clubs_that_cannot_pay_wages() {
        let facility = DebtProfile::facility_limit(REVENUE);
        let deep = -facility * 3;
        // Revenue comfortably covers wages → trade on, no administration.
        assert!(!DebtProfile::should_enter_administration(
            deep,
            REVENUE,
            REVENUE / 2,
            DebtStanding::Emergency
        ));
        // Wage bill exceeds revenue → insolvent trading.
        assert!(DebtProfile::should_enter_administration(
            deep,
            REVENUE,
            REVENUE * 2,
            DebtStanding::Emergency
        ));
    }

    #[test]
    fn debt_cannot_diverge_over_a_decade() {
        // The headline regression. Run a badly-run club for ten years with
        // a persistent operating deficit and assert the balance converges
        // to a bounded figure instead of compounding away.
        let revenue: i64 = 60_000_000;
        let wages: i64 = 90_000_000; // hopelessly overcommitted
        let mut balance: i64 = -10_000_000;
        let mut administration: Option<AdministrationState> = None;

        for _ in 0..120 {
            let standing =
                DebtProfile::classify(balance, revenue, DistressLevel::Insolvency, administration);
            let profile = DebtProfile {
                standing,
                administration,
            };

            // Operating cash flow, then interest.
            balance += revenue / 12;
            balance -= wages / 12;
            balance -= profile.monthly_interest(balance, revenue);

            // Owner steps in.
            balance += DebtProfile::owner_injection(balance, revenue, 0.5, standing);

            // Administration entry / exit.
            if administration.is_none()
                && DebtProfile::should_enter_administration(balance, revenue, wages, standing)
            {
                balance += DebtProfile::administration_write_down(balance, revenue);
                administration = Some(AdministrationState::enter(1));
            } else if let Some(mut state) = administration {
                if state.tick() {
                    administration = None;
                } else {
                    administration = Some(state);
                }
            }
        }

        let facility = DebtProfile::facility_limit(revenue);
        assert!(
            balance > -facility * 4,
            "debt diverged to {balance} despite administration (facility {facility})"
        );
    }
}
