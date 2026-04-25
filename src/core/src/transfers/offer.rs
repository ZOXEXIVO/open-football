use crate::shared::CurrencyValue;
use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub struct TransferOffer {
    pub base_fee: CurrencyValue,
    pub clauses: Vec<TransferClause>,
    pub salary_contribution: Option<CurrencyValue>, // For loans
    pub contract_length: Option<u8>,                // Years
    pub offering_club_id: u32,
    pub offered_date: NaiveDate,
}

#[derive(Debug, Clone)]
pub enum TransferClause {
    AppearanceFee(CurrencyValue, u32),  // Money after X appearances
    GoalBonus(CurrencyValue, u32),      // Money after X goals
    SellOnClause(f32),                  // Percentage of future transfer
    PromotionBonus(CurrencyValue),      // Money if buying club gets promoted
    Installments(CurrencyValue, u8),    // Money paid over N years
    LoanOptionToBuy(CurrencyValue),     // Optional future permanent fee
    LoanObligationToBuy(CurrencyValue), // Mandatory future permanent fee
}

impl Default for TransferOffer {
    fn default() -> Self {
        TransferOffer {
            base_fee: CurrencyValue {
                amount: 0.0,
                currency: crate::shared::Currency::Usd,
            },
            clauses: Vec::new(),
            salary_contribution: None,
            contract_length: None,
            offering_club_id: 0,
            offered_date: NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
        }
    }
}

impl TransferOffer {
    pub fn new(base_fee: CurrencyValue, offering_club_id: u32, offered_date: NaiveDate) -> Self {
        TransferOffer {
            base_fee,
            clauses: Vec::new(),
            salary_contribution: None,
            contract_length: None,
            offering_club_id,
            offered_date,
        }
    }

    pub fn with_clause(mut self, clause: TransferClause) -> Self {
        self.clauses.push(clause);
        self
    }

    pub fn with_salary_contribution(mut self, contribution: CurrencyValue) -> Self {
        self.salary_contribution = Some(contribution);
        self
    }

    pub fn with_contract_length(mut self, years: u8) -> Self {
        self.contract_length = Some(years);
        self
    }

    pub fn total_potential_value(&self) -> f64 {
        let mut total = self.base_fee.amount;

        for clause in &self.clauses {
            match clause {
                TransferClause::AppearanceFee(fee, _) => total += fee.amount * 0.7, // Assume 70% chance of meeting appearances
                TransferClause::GoalBonus(fee, _) => total += fee.amount * 0.5, // Assume 50% chance of meeting goal bonus
                TransferClause::SellOnClause(percentage) => {
                    total += total * (*percentage as f64) * 0.3
                } // Assume 30% chance of future sale
                TransferClause::PromotionBonus(fee) => total += fee.amount * 0.2, // Assume 20% chance of promotion
                TransferClause::Installments(fee, _) => total += fee.amount * 0.95,
                TransferClause::LoanOptionToBuy(fee) => total += fee.amount * 0.25,
                TransferClause::LoanObligationToBuy(fee) => total += fee.amount,
            }
        }

        total
    }

    pub fn loan_future_fee(&self) -> Option<(CurrencyValue, bool)> {
        self.clauses.iter().find_map(|clause| match clause {
            TransferClause::LoanOptionToBuy(fee) => Some((fee.clone(), false)),
            TransferClause::LoanObligationToBuy(fee) => Some((fee.clone(), true)),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::{Currency, CurrencyValue};

    fn money(amount: f64) -> CurrencyValue {
        CurrencyValue {
            amount,
            currency: Currency::Usd,
        }
    }

    #[test]
    fn extracts_loan_option_to_buy_as_future_fee() {
        let offer = TransferOffer::default()
            .with_clause(TransferClause::LoanOptionToBuy(money(5_000_000.0)));

        let future_fee = offer.loan_future_fee();

        assert_eq!(
            future_fee.map(|(fee, obligation)| (fee.amount, obligation)),
            Some((5_000_000.0, false))
        );
    }

    #[test]
    fn extracts_loan_obligation_to_buy_as_mandatory_future_fee() {
        let offer = TransferOffer::default()
            .with_clause(TransferClause::LoanObligationToBuy(money(7_000_000.0)));

        let future_fee = offer.loan_future_fee();

        assert_eq!(
            future_fee.map(|(fee, obligation)| (fee.amount, obligation)),
            Some((7_000_000.0, true))
        );
    }
}
