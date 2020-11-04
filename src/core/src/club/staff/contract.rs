pub use chrono::prelude::{DateTime, Datelike, NaiveDate, Utc};
use crate::context::SimulationContext;

#[derive(Debug, PartialEq)]
pub enum StaffPosition {
    SportDirector,
    MainCoach,
    Coach,
    Physio,
}

#[derive(Debug, PartialEq)]
pub enum StaffStatus {
    Active,
    ExpiredContract,
}

#[derive(Debug)]
pub struct StaffClubContract {
    expired: NaiveDate,
    pub salary: f64,
    pub position: StaffPosition,
    pub status: StaffStatus,
}

impl StaffClubContract {
    pub fn new(salary: f64, expired: NaiveDate, position: StaffPosition, status: StaffStatus) -> Self {
        StaffClubContract {
            salary,
            expired,
            position,
            status,
        }
    }

    pub fn is_expired(&self, context: &SimulationContext) -> bool {
        self.expired >= context.date.date()
    }

    pub fn simulate(&mut self, context: &SimulationContext) {
        if context.check_contract_expiration() && self.is_expired(context) {
            self.status = StaffStatus::ExpiredContract;
        }
    }
}