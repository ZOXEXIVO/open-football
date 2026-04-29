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
    pub transfer_budget: Option<CurrencyValue>,
    pub wage_budget: Option<CurrencyValue>,
    /// Outstanding amortization slices owed on previously bought players.
    /// Each tick of `process_monthly_finances` charges one month from each.
    pub transfer_obligations: Vec<TransferObligation>,
    /// Home matches played this month — drives matchday revenue. Reset by
    /// the monthly tick, incremented when a home match concludes.
    pub home_matches_this_month: u32,
}

/// One amortization stream: a transfer fee spread across the contract
/// length so each month the buying club's P&L recognises its share.
#[derive(Debug, Clone)]
pub struct TransferObligation {
    pub monthly_amount: i64,
    pub months_remaining: u32,
}

/// Severity of debt — drives the monthly interest rate and is consumed by
/// the result-stage to decide how aggressively to cut budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistressLevel {
    None,
    Distress,
    Severe,
    Insolvency,
}

impl ClubFinances {
    pub fn new(amount: i64, sponsorship_contract: Vec<ClubSponsorshipContract>) -> Self {
        ClubFinances {
            balance: ClubFinancialBalance::new(amount),
            history: ClubFinancialBalanceHistory::new(),
            sponsorship: ClubSponsorship::new(sponsorship_contract),
            transfer_budget: None,
            wage_budget: None,
            transfer_obligations: Vec::new(),
            home_matches_this_month: 0,
        }
    }

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
            transfer_obligations: Vec::new(),
            home_matches_this_month: 0,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ClubFinanceResult {
        let mut result = ClubFinanceResult::new();
        let club_name = ctx.club.as_ref().expect("no club found").name;
        let club_id = ctx.club.as_ref().map(|c| c.id).unwrap_or(0);
        result = result.with_club(club_id);

        if ctx.simulation.is_month_beginning() {
            debug!("club: {}, finance: start new month", club_name);
            // Distress check uses the trailing wage average — read it
            // BEFORE clearing the in-progress month, otherwise the
            // post-clear `expense_player_wages` is zero and every club
            // looks one dollar from administration.
            let avg_wages = self.trailing_avg_monthly_wages(ctx.simulation.date.date());
            let level = classify_distress(self.balance.balance, avg_wages);
            result.is_in_distress = !matches!(level, DistressLevel::None);
            result.distress_level = level;

            self.start_new_month(club_name, ctx.simulation.date.date());

            result.expired_sponsorships = self
                .sponsorship
                .remove_expired(ctx.simulation.date.date());
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
        self.home_matches_this_month = 0;
    }

    /// Average monthly player wages charged across the trailing window of
    /// completed-month snapshots. Falls back to the live (in-progress)
    /// month's wages, then to the current annualized-wage estimate via
    /// `current_monthly_wage_estimate`. A floor of $1 keeps comparisons
    /// well-formed for a brand-new club with no history.
    pub fn trailing_avg_monthly_wages(&self, today: NaiveDate) -> i64 {
        let cutoff = today - chrono::Duration::days(95);
        let mut total = 0i64;
        let mut months = 0i64;
        for (date, snap) in self.history.iter() {
            if *date < cutoff {
                continue;
            }
            if snap.expense_player_wages <= 0 {
                continue;
            }
            total += snap.expense_player_wages;
            months += 1;
        }
        if months > 0 {
            return (total / months).max(1);
        }
        if self.balance.expense_player_wages > 0 {
            return self.balance.expense_player_wages;
        }
        1
    }

    /// Schedule a home match for the current month. Called from the match
    /// pipeline when a non-friendly home fixture concludes.
    pub fn record_home_match(&mut self) {
        self.home_matches_this_month = self.home_matches_this_month.saturating_add(1);
    }

    /// Pull and reset the month's home-match count. Used by
    /// `process_monthly_finances` so the matchday revenue line scales with
    /// actual fixtures rather than a hardcoded `* 2`.
    pub fn take_home_match_count(&mut self) -> u32 {
        let n = self.home_matches_this_month;
        self.home_matches_this_month = 0;
        n
    }

    /// Tick all outstanding amortization streams: each charges one month's
    /// slice as `expense_amortization`. Streams that reach zero remaining
    /// months are dropped.
    pub fn tick_amortization(&mut self) -> i64 {
        let mut total = 0i64;
        for ob in self.transfer_obligations.iter_mut() {
            if ob.months_remaining == 0 {
                continue;
            }
            total += ob.monthly_amount;
            ob.months_remaining -= 1;
        }
        self.transfer_obligations.retain(|o| o.months_remaining > 0);
        if total > 0 {
            self.balance.push_expense_amortization(total);
        }
        total
    }

    pub fn push_salary(&mut self, club_name: &str, amount: i64) {
        debug!(
            "club: {}, finance: push salary, amount = {}",
            club_name, amount
        );

        self.balance.push_expense_player_wages(amount);
    }

    /// Buying-side bookkeeping for a permanent transfer. Cash leaves the
    /// balance immediately; the P&L impact is spread across `contract_years`
    /// as monthly amortization. Returns `false` when the transfer budget is
    /// configured and can't cover the fee — caller should not proceed.
    pub fn register_transfer_purchase(&mut self, amount: f64, contract_years: u8) -> bool {
        let amount = amount.max(0.0);
        if amount <= 0.0 {
            return true;
        }
        if let Some(ref mut budget) = self.transfer_budget {
            if budget.amount < amount {
                return false;
            }
            budget.amount -= amount;
        }
        self.balance.push_cash_outflow(amount as i64);
        let years = contract_years.max(1) as u32;
        let months = years * 12;
        let monthly = (amount as i64) / months as i64;
        if monthly > 0 {
            self.transfer_obligations.push(TransferObligation {
                monthly_amount: monthly,
                months_remaining: months,
            });
        }
        true
    }

    /// Buying-side loan fee payment — immediate cash + immediate P&L,
    /// classified as `expense_loan_fees`. Loans use small fees and don't
    /// amortize like a permanent purchase.
    pub fn pay_loan_fee(&mut self, amount: f64) {
        let amount = amount.max(0.0) as i64;
        if amount <= 0 {
            return;
        }
        if let Some(ref mut budget) = self.transfer_budget {
            budget.amount = (budget.amount - amount as f64).max(0.0);
        }
        self.balance.push_expense_loan_fees(amount);
    }

    /// Selling-side loan fee receipt — immediate cash + immediate P&L,
    /// classified as `income_loan_fees`.
    pub fn receive_loan_fee(&mut self, amount: f64) {
        let amount = amount.max(0.0) as i64;
        if amount <= 0 {
            return;
        }
        self.balance.push_income_loan_fees(amount);
    }

    /// Reverse a previously credited loan fee — used when the
    /// borrowing-side rejects the move and the player has to come back.
    pub fn refund_loan_fee(&mut self, amount: f64) {
        let amount = amount.max(0.0) as i64;
        if amount <= 0 {
            return;
        }
        self.balance.income_loan_fees -= amount;
        self.balance.income -= amount;
        self.balance.balance -= amount;
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

    /// Trailing twelve months of total income across the history snapshots.
    /// Used by the board to size next season's transfer/wage budgets from
    /// projected revenue rather than current cash.
    pub fn trailing_annual_income(&self, today: NaiveDate) -> i64 {
        let cutoff = today - chrono::Duration::days(365);
        let mut total = 0i64;
        for (date, snap) in self.history.iter() {
            if *date < cutoff {
                continue;
            }
            total += snap.income;
        }
        total
    }

    /// Trailing twelve months of total operating expenses across the
    /// history snapshots — counterpart to `trailing_annual_income`.
    pub fn trailing_annual_outcome(&self, today: NaiveDate) -> i64 {
        let cutoff = today - chrono::Duration::days(365);
        let mut total = 0i64;
        for (date, snap) in self.history.iter() {
            if *date < cutoff {
                continue;
            }
            total += snap.outcome;
        }
        total
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

    /// Soft FFP signal — the club has booked losses but still inside the
    /// breach threshold. Used by the board to throttle spend before legal
    /// trouble arrives.
    pub fn is_ffp_watchlist(&self, today: NaiveDate) -> bool {
        if self.is_ffp_breach(today) {
            return false;
        }
        let loss = self.three_year_loss(today);
        if loss <= 0 {
            return false;
        }
        let annual_wages = self.trailing_annual_wages(today);
        let breach_threshold = ((annual_wages as i64).saturating_mul(2)).max(20_000_000);
        loss * 2 > breach_threshold
    }
}

/// Classify the club's distress from cash balance and trailing wage scale.
/// Wealth-relative — a small club is distressed at smaller absolute debt
/// than a Premier League side.
pub fn classify_distress(balance: i64, avg_monthly_wages: i64) -> DistressLevel {
    let scale = avg_monthly_wages.max(1);
    if balance < -(scale.saturating_mul(12)) {
        DistressLevel::Insolvency
    } else if balance < -(scale.saturating_mul(6)) {
        DistressLevel::Severe
    } else if balance < -(scale.saturating_mul(3)) {
        DistressLevel::Distress
    } else {
        DistressLevel::None
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

    /// Cash leaves the bank account but the cost is recognised over time
    /// (amortization), not immediately as a P&L expense. Used for the
    /// upfront leg of a permanent transfer purchase.
    pub fn push_cash_outflow(&mut self, amount: i64) {
        self.balance -= amount;
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

    /// Amortized slice of a transfer fee this month. The full fee already
    /// left the cash balance at purchase via `push_cash_outflow`, so this
    /// only recognises the P&L leg — `outcome` and the categorised
    /// `expense_amortization` bucket. Touching `balance` here would
    /// double-debit the cash that was already paid upfront.
    pub fn push_expense_amortization(&mut self, amount: i64) {
        self.expense_amortization += amount;
        self.outcome += amount;
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

#[cfg(test)]
mod finance_tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, 1).unwrap()
    }

    #[test]
    fn distress_uses_trailing_wage_average_not_cleared_current_month() {
        // The bug we're guarding against: previously the distress check
        // ran AFTER the monthly clear, so `expense_player_wages` was zero
        // and any non-trivial debt tripped the alarm. With trailing
        // history available, distress must scale with the actual wage
        // bill — a club with $5M/month wages tolerates more debt than
        // one with $200K/month.
        let mut f = ClubFinances::new(-2_000_000, vec![]);
        let mut prev = ClubFinancialBalance::new(0);
        prev.expense_player_wages = 5_000_000;
        f.history.add(d(2026, 3), prev);

        let avg = f.trailing_avg_monthly_wages(d(2026, 4));
        assert_eq!(avg, 5_000_000);
        let level = classify_distress(f.balance.balance, avg);
        // -$2M < -3 * $5M? No (since 3*5M=15M, -2M is not below -15M).
        assert_eq!(level, DistressLevel::None);

        // Now drop the cash deeper.
        f.balance.balance = -50_000_000;
        let level = classify_distress(f.balance.balance, avg);
        // -50M < -3 * 5M=-15M (yes); < -6*5M=-30M (yes); < -12*5M=-60M (no)
        assert_eq!(level, DistressLevel::Severe);
    }

    #[test]
    fn distress_falls_back_to_floor_for_brand_new_club() {
        // No history, no in-progress wages → floor of $1 keeps the
        // comparison well-formed without tripping every fresh club into
        // distress on day one.
        let f = ClubFinances::new(0, vec![]);
        let avg = f.trailing_avg_monthly_wages(d(2026, 4));
        assert_eq!(avg, 1);
    }

    #[test]
    fn classify_distress_thresholds() {
        assert_eq!(classify_distress(0, 1_000_000), DistressLevel::None);
        // Just above the distress line: -3 * 1M = -3M cutoff.
        assert_eq!(
            classify_distress(-2_999_999, 1_000_000),
            DistressLevel::None
        );
        assert_eq!(
            classify_distress(-3_500_000, 1_000_000),
            DistressLevel::Distress
        );
        assert_eq!(
            classify_distress(-7_000_000, 1_000_000),
            DistressLevel::Severe
        );
        assert_eq!(
            classify_distress(-13_000_000, 1_000_000),
            DistressLevel::Insolvency
        );
    }

    #[test]
    fn home_match_counter_records_and_resets() {
        let mut f = ClubFinances::new(0, vec![]);
        f.record_home_match();
        f.record_home_match();
        assert_eq!(f.home_matches_this_month, 2);
        let n = f.take_home_match_count();
        assert_eq!(n, 2);
        assert_eq!(f.home_matches_this_month, 0);
    }

    #[test]
    fn register_transfer_purchase_decrements_cash_and_stages_amortization() {
        let mut f = ClubFinances::new(100_000_000, vec![]);
        let ok = f.register_transfer_purchase(48_000_000.0, 4);
        assert!(ok);
        // Cash dropped by full fee.
        assert_eq!(f.balance.balance, 100_000_000 - 48_000_000);
        // P&L (outcome) untouched at upfront.
        assert_eq!(f.balance.outcome, 0);
        assert_eq!(f.balance.expense_amortization, 0);
        // One obligation: 48M / (4 * 12) = 1M/month for 48 months.
        assert_eq!(f.transfer_obligations.len(), 1);
        assert_eq!(f.transfer_obligations[0].monthly_amount, 1_000_000);
        assert_eq!(f.transfer_obligations[0].months_remaining, 48);
    }

    #[test]
    fn tick_amortization_charges_pl_without_double_debiting_balance() {
        let mut f = ClubFinances::new(100_000_000, vec![]);
        f.register_transfer_purchase(24_000_000.0, 2);
        let cash_after_purchase = f.balance.balance;

        let charged = f.tick_amortization();
        assert_eq!(charged, 1_000_000); // 24M / 24 months
        // P&L charged.
        assert_eq!(f.balance.outcome, 1_000_000);
        assert_eq!(f.balance.expense_amortization, 1_000_000);
        // Cash NOT touched again — already paid upfront.
        assert_eq!(f.balance.balance, cash_after_purchase);
        assert_eq!(f.transfer_obligations[0].months_remaining, 23);
    }

    #[test]
    fn tick_amortization_drops_finished_obligations() {
        let mut f = ClubFinances::new(0, vec![]);
        f.transfer_obligations.push(TransferObligation {
            monthly_amount: 100,
            months_remaining: 1,
        });
        let charged = f.tick_amortization();
        assert_eq!(charged, 100);
        assert!(f.transfer_obligations.is_empty());
    }

    #[test]
    fn loan_fee_payment_is_immediate_pl_classified() {
        let mut f = ClubFinances::new(10_000_000, vec![]);
        f.pay_loan_fee(500_000.0);
        assert_eq!(f.balance.balance, 10_000_000 - 500_000);
        assert_eq!(f.balance.expense_loan_fees, 500_000);
        assert_eq!(f.balance.outcome, 500_000);
    }

    #[test]
    fn loan_fee_receipt_is_immediate_pl_classified() {
        let mut f = ClubFinances::new(0, vec![]);
        f.receive_loan_fee(500_000.0);
        assert_eq!(f.balance.balance, 500_000);
        assert_eq!(f.balance.income_loan_fees, 500_000);
        assert_eq!(f.balance.income, 500_000);

        f.refund_loan_fee(500_000.0);
        assert_eq!(f.balance.balance, 0);
        assert_eq!(f.balance.income_loan_fees, 0);
        assert_eq!(f.balance.income, 0);
    }

    #[test]
    fn trailing_annual_income_sums_history_within_window() {
        let mut f = ClubFinances::new(0, vec![]);
        let mut snap = ClubFinancialBalance::new(0);
        snap.income = 5_000_000;
        f.history.add(d(2025, 6), snap);
        let mut snap_old = ClubFinancialBalance::new(0);
        snap_old.income = 99_000_000;
        // > 365 days old, must be ignored.
        f.history.add(d(2024, 1), snap_old);
        assert_eq!(f.trailing_annual_income(d(2026, 1)), 5_000_000);
    }
}