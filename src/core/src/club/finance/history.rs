use crate::club::ClubFinancialBalance;
use chrono::NaiveDate;
use std::collections::LinkedList;

#[derive(Debug, Clone)]
pub struct ClubFinancialBalanceHistory {
    history: LinkedList<(NaiveDate, ClubFinancialBalance)>,
}

impl ClubFinancialBalanceHistory {
    pub fn new() -> Self {
        ClubFinancialBalanceHistory {
            history: LinkedList::new(),
        }
    }

    pub fn get(&self, date: NaiveDate) -> Option<&ClubFinancialBalance> {
        for (history_date, item) in self.history.iter() {
            if *history_date == date {
                return Some(item);
            }
        }

        None
    }

    pub fn add(&mut self, date: NaiveDate, balance: ClubFinancialBalance) {
        self.history.push_front((date, balance))
    }

    pub fn iter(&self) -> impl Iterator<Item = &(NaiveDate, ClubFinancialBalance)> {
        self.history.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_date_not_found_none() {
        let history = ClubFinancialBalanceHistory::new();

        let date = NaiveDate::from_ymd_opt(2020, 2, 1).unwrap();

        let result = history.get(date);

        assert!(result.is_none());
    }

    #[test]
    fn get_date_exist_return_balance() {
        let mut history = ClubFinancialBalanceHistory::new();

        let balance = ClubFinancialBalance::new(123);
        let date = NaiveDate::from_ymd_opt(2020, 2, 1).unwrap();

        history.add(date, balance);

        let result = history.get(date);

        assert!(result.is_some());
        assert_eq!(123, result.unwrap().balance);
    }
}
