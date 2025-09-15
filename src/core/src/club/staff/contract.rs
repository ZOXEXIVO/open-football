use crate::context::SimulationContext;
pub use chrono::prelude::{DateTime, Datelike, NaiveDate, Utc};

#[derive(Debug, PartialEq)]
pub enum StaffPosition {
    Free,
    Coach,
    Chairman,
    Director,
    ManagingDirector,
    DirectorOfFootball,
    Physio,
    Scout,
    Manager,
    AssistantManager,
    MediaPundit,
    GeneralManager,
    FitnessCoach,
    GoalkeeperCoach,
    U21Manager,
    ChiefScout,
    YouthCoach,
    HeadOfPhysio,
    U19Manager,
    FirstTeamCoach,
    HeadOfYouthDevelopment,
    CaretakerManager,
}

#[derive(Debug, PartialEq)]
pub enum StaffStatus {
    Active,
    ExpiredContract,
}

#[derive(Debug)]
pub struct StaffClubContract {
    pub expired: NaiveDate,
    pub salary: u32,
    pub position: StaffPosition,
    pub status: StaffStatus,
}

impl StaffClubContract {
    pub fn new(
        salary: u32,
        expired: NaiveDate,
        position: StaffPosition,
        status: StaffStatus,
    ) -> Self {
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
