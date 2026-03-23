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
        let result = ClubFinanceResult::new();
        let club_name = ctx.club.as_ref().expect("no club found").name;

        if ctx.simulation.is_month_beginning() {
            debug!("club: {}, finance: start new month", club_name);
            self.start_new_month(club_name, ctx.simulation.date.date());
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

    // Expense categories
    pub expense_player_wages: i64,
    pub expense_staff_wages: i64,
    pub expense_facilities: i64,
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
            expense_player_wages: 0,
            expense_staff_wages: 0,
            expense_facilities: 0,
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

    pub fn clear(&mut self) {
        self.income = 0;
        self.outcome = 0;
        self.income_tv = 0;
        self.income_matchday = 0;
        self.income_sponsorship = 0;
        self.income_merchandising = 0;
        self.income_prize_money = 0;
        self.expense_player_wages = 0;
        self.expense_staff_wages = 0;
        self.expense_facilities = 0;
    }
}