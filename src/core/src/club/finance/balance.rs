use crate::context::GlobalContext;
use crate::shared::CurrencyValue;
use crate::{ClubFinanceResult, ClubFinancialBalanceHistory, ClubSponsorship, ClubSponsorshipContract};
use chrono::NaiveDate;
use log::debug;

#[derive(Debug, Clone)]
pub struct ClubFinances {
    pub balance: ClubFinancialBalance,
    pub history: ClubFinancialBalanceHistory,
    pub sponsorship: ClubSponsorship,
    pub transfer_budget: Option<CurrencyValue>,  // NEW FIELD
    pub wage_budget: Option<CurrencyValue>,      // NEW FIELD
}

impl ClubFinances {
    pub fn new(amount: i64, sponsorship_contract: Vec<ClubSponsorshipContract>) -> Self {
        ClubFinances {
            balance: ClubFinancialBalance::new(amount),
            history: ClubFinancialBalanceHistory::new(),
            sponsorship: ClubSponsorship::new(sponsorship_contract),
            transfer_budget: None,
            wage_budget: None,
        }
    }

    // New constructor with budgets
    pub fn with_budgets(
        amount: i64,
        sponsorship_contract: Vec<ClubSponsorshipContract>,
        transfer_budget: Option<CurrencyValue>,
        wage_budget: Option<CurrencyValue>,
    ) -> Self {
        ClubFinances {
            balance: ClubFinancialBalance::new(amount),
            history: ClubFinancialBalanceHistory::new(),
            sponsorship: ClubSponsorship::new(sponsorship_contract),
            transfer_budget,
            wage_budget,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ClubFinanceResult {
        let mut result = ClubFinanceResult::new();
        let club_name = ctx.club.as_ref().expect("no club found").name;
        let club_id = ctx.club.as_ref().map(|c| c.id).unwrap_or(0);
        result = result.with_club(club_id);

        if ctx.simulation.is_month_beginning() {
            debug!("club: {}, finance: start new month", club_name);
            self.start_new_month(club_name, ctx.simulation.date.date());

            // Check financial distress: balance deeply negative (more than 3 months of wages)
            let monthly_wages: i64 = self.balance.expense_player_wages.max(1);
            if self.balance.balance < -(monthly_wages * 3) {
                result.is_in_distress = true;
            }

            // Count expired sponsorships (get_sponsorship_incomes prunes expired ones internally)
            let before = self.sponsorship.sponsorship_contracts.len();
            let _ = self.sponsorship.get_sponsorship_incomes(ctx.simulation.date.date());
            let after = self.sponsorship.sponsorship_contracts.len();
            result.expired_sponsorships = (before - after) as u32;
        }

        result
    }

    fn start_new_month(&mut self, club_name: &str, date: NaiveDate) {
        debug!(
        "club: {}, finance: add history, date = {}, balance = {}, income={}, outcome={}",
        club_name, date, self.balance.balance, self.balance.income, self.balance.outcome
    );

        self.history.add(date, self.balance.clone());
        self.balance.clear();
    }

    pub fn push_salary(&mut self, club_name: &str, amount: i64) {
        debug!(
            "club: {}, finance: push salary, amount = {}",
            club_name, amount
        );

        self.balance.push_expense_player_wages(amount);
    }

    // Helper method to spend from transfer budget
    pub fn spend_from_transfer_budget(&mut self, amount: f64) -> bool {
        if let Some(ref mut budget) = self.transfer_budget {
            if budget.amount >= amount {
                budget.amount -= amount;
                self.balance.push_outcome(amount as i64);
                return true;
            }
        }
        false
    }

    // Helper method to add transfer income
    pub fn add_transfer_income(&mut self, amount: f64) {
        self.balance.push_income(amount as i64);

        // Add 50% of transfer income to transfer budget
        if let Some(ref mut budget) = self.transfer_budget {
            budget.amount += amount * 0.5;
        } else {
            self.transfer_budget = Some(CurrencyValue {
                amount: amount * 0.5,
                currency: crate::shared::Currency::Usd,
            });
        }
    }

    /// Net loss accumulated over the trailing three years, read from the
    /// monthly history snapshots. Profitable months offset loss-making
    /// ones; a non-positive return means the club is cash-neutral or
    /// profitable over the period.
    pub fn three_year_loss(&self, today: NaiveDate) -> i64 {
        let cutoff = today - chrono::Duration::days(365 * 3);
        let mut losses = 0i64;
        for (date, snap) in self.history.iter() {
            if *date < cutoff {
                continue;
            }
            losses += snap.outcome - snap.income;
        }
        losses
    }

    /// Player wages paid over the trailing twelve months. Used as the
    /// scale for the FFP breach threshold so wealthy clubs aren't flagged
    /// for the same absolute losses that would cripple a smaller side.
    pub fn trailing_annual_wages(&self, today: NaiveDate) -> u64 {
        let cutoff = today - chrono::Duration::days(365);
        let mut total = 0u64;
        for (date, snap) in self.history.iter() {
            if *date < cutoff {
                continue;
            }
            total += snap.expense_player_wages.max(0) as u64;
        }
        total
    }

    /// Have the trailing three years of football operations pushed the
    /// club into FFP breach territory? Threshold is twice the trailing
    /// annual wage bill, with a floor of $20M so empty-history clubs get
    /// a sensible default. Downstream code (transfer pipeline, board)
    /// reads this to gate big spends.
    pub fn is_ffp_breach(&self, today: NaiveDate) -> bool {
        let loss = self.three_year_loss(today);
        if loss <= 0 {
            return false;
        }
        let annual_wages = self.trailing_annual_wages(today);
        let threshold = ((annual_wages as i64).saturating_mul(2)).max(20_000_000);
        loss > threshold
    }
}

#[derive(Debug, Clone)]
pub struct ClubFinancialBalance {
    pub balance: i64,
    pub income: i64,
    pub outcome: i64,

    // Income categories
    pub income_tv: i64,
    pub income_matchday: i64,
    pub income_sponsorship: i64,
    pub income_merchandising: i64,
    pub income_prize_money: i64,
    /// Placement-based TV bonus layered on top of the reputation TV base.
    pub income_tv_placement: i64,
    /// Domestic cup prize money earned this period.
    pub income_cup_prize: i64,
    /// Continental (UCL/UEL) prize money earned this period.
    pub income_continental_prize: i64,

    // Expense categories
    pub expense_player_wages: i64,
    pub expense_staff_wages: i64,
    pub expense_facilities: i64,
    /// Amortized portion of player transfer fees charged this period.
    pub expense_amortization: i64,
    /// Interest charged on a negative balance this period.
    pub expense_debt_interest: i64,

    // Loan match fee tracking
    pub income_loan_fees: i64,
    pub expense_loan_fees: i64,
}

impl ClubFinancialBalance {
    pub fn new(balance: i64) -> Self {
        ClubFinancialBalance {
            balance,
            income: 0,
            outcome: 0,
            income_tv: 0,
            income_matchday: 0,
            income_sponsorship: 0,
            income_merchandising: 0,
            income_prize_money: 0,
            income_tv_placement: 0,
            income_cup_prize: 0,
            income_continental_prize: 0,
            expense_player_wages: 0,
            expense_staff_wages: 0,
            expense_facilities: 0,
            expense_amortization: 0,
            expense_debt_interest: 0,
            income_loan_fees: 0,
            expense_loan_fees: 0,
        }
    }

    pub fn push_income(&mut self, amount: i64) {
        self.balance += amount;
        self.income += amount;
    }

    pub fn push_outcome(&mut self, amount: i64) {
        self.balance -= amount;
        self.outcome += amount;
    }

    // Categorized income methods
    pub fn push_income_tv(&mut self, amount: i64) {
        self.income_tv += amount;
        self.push_income(amount);
    }

    pub fn push_income_matchday(&mut self, amount: i64) {
        self.income_matchday += amount;
        self.push_income(amount);
    }

    pub fn push_income_sponsorship(&mut self, amount: i64) {
        self.income_sponsorship += amount;
        self.push_income(amount);
    }

    pub fn push_income_merchandising(&mut self, amount: i64) {
        self.income_merchandising += amount;
        self.push_income(amount);
    }

    pub fn push_income_prize_money(&mut self, amount: i64) {
        self.income_prize_money += amount;
        self.push_income(amount);
    }

    /// Placement bonus layered on top of the reputation-based TV base.
    pub fn push_income_tv_placement(&mut self, amount: i64) {
        self.income_tv_placement += amount;
        self.push_income(amount);
    }

    /// Domestic cup prize money — per round.
    pub fn push_income_cup_prize(&mut self, amount: i64) {
        self.income_cup_prize += amount;
        self.income_prize_money += amount;
        self.push_income(amount);
    }

    /// Continental (UCL/UEL) prize money — per round.
    pub fn push_income_continental_prize(&mut self, amount: i64) {
        self.income_continental_prize += amount;
        self.income_prize_money += amount;
        self.push_income(amount);
    }

    /// Amortized slice of a transfer fee this month.
    pub fn push_expense_amortization(&mut self, amount: i64) {
        self.expense_amortization += amount;
        self.push_outcome(amount);
    }

    /// Interest cost on a negative balance.
    pub fn push_expense_debt_interest(&mut self, amount: i64) {
        self.expense_debt_interest += amount;
        self.push_outcome(amount);
    }

    // Categorized expense methods
    pub fn push_expense_player_wages(&mut self, amount: i64) {
        self.expense_player_wages += amount;
        self.push_outcome(amount);
    }

    pub fn push_expense_staff_wages(&mut self, amount: i64) {
        self.expense_staff_wages += amount;
        self.push_outcome(amount);
    }

    pub fn push_expense_facilities(&mut self, amount: i64) {
        self.expense_facilities += amount;
        self.push_outcome(amount);
    }

    // Loan match fee methods
    pub fn push_income_loan_fees(&mut self, amount: i64) {
        self.income_loan_fees += amount;
        self.push_income(amount);
    }

    pub fn push_expense_loan_fees(&mut self, amount: i64) {
        self.expense_loan_fees += amount;
        self.push_outcome(amount);
    }

    pub fn clear(&mut self) {
        self.income = 0;
        self.outcome = 0;
        self.income_tv = 0;
        self.income_matchday = 0;
        self.income_sponsorship = 0;
        self.income_merchandising = 0;
        self.income_prize_money = 0;
        self.income_tv_placement = 0;
        self.income_cup_prize = 0;
        self.income_continental_prize = 0;
        self.expense_player_wages = 0;
        self.expense_staff_wages = 0;
        self.expense_facilities = 0;
        self.expense_amortization = 0;
        self.expense_debt_interest = 0;
        self.income_loan_fees = 0;
        self.expense_loan_fees = 0;
    }
}

#[cfg(test)]
mod ffp_tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, 1).unwrap()
    }

    fn finances_with_history(months: Vec<(NaiveDate, i64, i64, i64)>) -> ClubFinances {
        let mut f = ClubFinances::new(0, vec![]);
        for (date, income, outcome, wages) in months {
            let mut snap = ClubFinancialBalance::new(0);
            snap.income = income;
            snap.outcome = outcome;
            snap.expense_player_wages = wages;
            f.history.add(date, snap);
        }
        f
    }

    #[test]
    fn no_history_means_no_breach() {
        let f = ClubFinances::new(0, vec![]);
        assert!(!f.is_ffp_breach(d(2025, 1)));
        assert_eq!(f.three_year_loss(d(2025, 1)), 0);
    }

    #[test]
    fn profitable_club_is_not_in_breach() {
        let f = finances_with_history(vec![
            (d(2024, 6), 5_000_000, 3_000_000, 2_500_000),
            (d(2024, 7), 5_000_000, 3_000_000, 2_500_000),
            (d(2024, 8), 5_000_000, 3_000_000, 2_500_000),
        ]);
        assert!(f.three_year_loss(d(2025, 1)) <= 0);
        assert!(!f.is_ffp_breach(d(2025, 1)));
    }

    #[test]
    fn loss_under_threshold_is_not_breach() {
        // ~$15M loss, wage base $100M/yr → threshold $200M. Not a breach.
        let f = finances_with_history(vec![
            (d(2024, 6), 2_000_000, 7_000_000, 8_000_000),
            (d(2024, 7), 2_000_000, 7_000_000, 8_000_000),
            (d(2024, 8), 2_000_000, 7_000_000, 8_000_000),
        ]);
        assert!(f.three_year_loss(d(2025, 1)) > 0);
        assert!(!f.is_ffp_breach(d(2025, 1)));
    }

    #[test]
    fn loss_above_threshold_trips_breach() {
        // Zero wage bill → threshold floors at $20M. Accumulate $24M loss.
        let months: Vec<_> = (1..=12)
            .map(|m| (d(2024, m), 0_i64, 2_000_000_i64, 0_i64))
            .collect();
        let f = finances_with_history(months);
        let loss = f.three_year_loss(d(2025, 1));
        assert!(loss > 20_000_000, "loss={loss}");
        assert!(f.is_ffp_breach(d(2025, 1)));
    }

    #[test]
    fn old_history_outside_three_year_window_is_ignored() {
        let f = finances_with_history(vec![
            (d(2020, 6), 0, 50_000_000, 0), // >3 years old — shouldn't count
            (d(2024, 6), 1_000_000, 1_500_000, 100_000),
        ]);
        let loss = f.three_year_loss(d(2025, 1));
        assert!(loss < 1_000_000, "old loss leaked in: {loss}");
    }
}