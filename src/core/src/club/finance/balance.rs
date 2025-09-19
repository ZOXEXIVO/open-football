use crate::context::GlobalContext;
use crate::shared::CurrencyValue;
use crate::{ClubFinanceResult, ClubFinancialBalanceHistory, ClubSponsorship, ClubSponsorshipContract};
use chrono::NaiveDate;
use log::debug;

#[derive(Debug)]
pub struct ClubFinances {
    pub balance: ClubFinancialBalance,
    pub history: ClubFinancialBalanceHistory,
    pub sponsorship: ClubSponsorship,
    pub transfer_budget: Option<CurrencyValue>,  // NEW FIELD
    pub wage_budget: Option<CurrencyValue>,      // NEW FIELD
}

impl ClubFinances {
    pub fn new(amount: i32, sponsorship_contract: Vec<ClubSponsorshipContract>) -> Self {
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
        amount: i32,
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

            // Update budgets at month beginning
            self.update_budgets();
        }

        if ctx.simulation.is_year_beginning() {
            // ... sponsorship income code ...

            // Reset budgets for new year
            self.reset_annual_budgets();
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

    pub fn push_salary(&mut self, club_name: &str, amount: i32) {
        debug!(
            "club: {}, finance: push salary, amount = {}",
            club_name, amount
        );

        self.balance.push_outcome(amount);
    }

    fn update_budgets(&mut self) {
        // Update transfer and wage budgets based on current financial situation
        let available_funds = self.balance.balance as f64;

        // Allocate 30% of available funds to transfers if positive balance
        if available_funds > 0.0 {
            self.transfer_budget = Some(CurrencyValue {
                amount: available_funds * 0.3,
                currency: crate::shared::Currency::Usd,
            });
        }
    }

    fn reset_annual_budgets(&mut self) {
        // Reset budgets based on overall financial health
        let available_funds = self.balance.balance as f64;

        if available_funds > 0.0 {
            // Set annual transfer budget (40% of available funds)
            self.transfer_budget = Some(CurrencyValue {
                amount: available_funds * 0.4,
                currency: crate::shared::Currency::Usd,
            });

            // Set annual wage budget (50% of available funds)
            self.wage_budget = Some(CurrencyValue {
                amount: available_funds * 0.5,
                currency: crate::shared::Currency::Usd,
            });
        } else {
            // No budget if in debt
            self.transfer_budget = None;
            self.wage_budget = None;
        }
    }

    // Helper method to spend from transfer budget
    pub fn spend_from_transfer_budget(&mut self, amount: f64) -> bool {
        if let Some(ref mut budget) = self.transfer_budget {
            if budget.amount >= amount {
                budget.amount -= amount;
                self.balance.push_outcome(amount as i32);
                return true;
            }
        }
        false
    }

    // Helper method to add transfer income
    pub fn add_transfer_income(&mut self, amount: f64) {
        self.balance.push_income(amount as i32);

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
#[allow(dead_code)]
pub struct ClubFinancialBalance {
    pub balance: i32,
    pub income: i32,
    pub outcome: i32,
    highest_wage_paid: i32,
    latest_season_tickets: i32,
    remaining_budget: i32,
    season_transfer_funds: i32,
    transfer_income_percentage: i32,
    weekly_wage_budget: i32,
    highest_wage: i32,
    youth_grant_income: i32,
}

impl ClubFinancialBalance {
    pub fn new(balance: i32) -> Self {
        ClubFinancialBalance {
            balance,
            income: 0,
            outcome: 0,
            highest_wage_paid: 0,
            latest_season_tickets: 0,
            remaining_budget: 0,
            season_transfer_funds: 0,
            transfer_income_percentage: 0,
            weekly_wage_budget: 0,
            highest_wage: 0,
            youth_grant_income: 0,
        }
    }

    pub fn push_income(&mut self, wage: i32) {
        self.balance = self.balance + wage;
        self.income = self.income + wage;
    }

    pub fn push_outcome(&mut self, wage: i32) {
        self.balance = self.balance - wage;
        self.outcome = self.outcome + wage;
    }

    pub fn clear(&mut self) {
        self.income = 0;
        self.outcome = 0;
    }
}