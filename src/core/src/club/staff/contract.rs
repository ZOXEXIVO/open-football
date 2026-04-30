use crate::context::SimulationContext;
pub use chrono::prelude::{DateTime, Datelike, NaiveDate, Utc};

#[derive(Debug, Clone, PartialEq)]
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
    /// Modern analytics role — reads match data, informs tactical prep
    /// and recruitment shortlists.
    DataAnalyst,
    /// Regulator of the player recruitment pipeline.
    HeadOfRecruitment,
}

impl StaffPosition {
    /// Is this role involved in day-to-day player coaching?
    pub fn is_coaching(&self) -> bool {
        matches!(
            self,
            Self::Coach
                | Self::FirstTeamCoach
                | Self::YouthCoach
                | Self::GoalkeeperCoach
                | Self::FitnessCoach
                | Self::AssistantManager
                | Self::Manager
        )
    }

    /// Medical roles — govern injury recovery speed and training risk.
    pub fn is_medical(&self) -> bool {
        matches!(self, Self::Physio | Self::HeadOfPhysio)
    }

    /// Scouting roles — govern player report accuracy and pool expansion.
    pub fn is_scouting(&self) -> bool {
        matches!(
            self,
            Self::Scout | Self::ChiefScout | Self::HeadOfRecruitment | Self::DataAnalyst
        )
    }

    /// Executive roles — non-coaching decision makers above the manager.
    pub fn is_executive(&self) -> bool {
        matches!(
            self,
            Self::Chairman
                | Self::Director
                | Self::ManagingDirector
                | Self::DirectorOfFootball
                | Self::GeneralManager
        )
    }

    /// Youth pipeline roles — academy development and intake.
    pub fn is_youth(&self) -> bool {
        matches!(
            self,
            Self::HeadOfYouthDevelopment | Self::YouthCoach | Self::U19Manager | Self::U21Manager
        )
    }

    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            Self::Manager => "staff_manager",
            Self::AssistantManager => "staff_assistant_manager",
            Self::CaretakerManager => "staff_caretaker_manager",
            Self::Coach => "staff_coach",
            Self::FirstTeamCoach => "staff_first_team_coach",
            Self::FitnessCoach => "staff_fitness_coach",
            Self::GoalkeeperCoach => "staff_goalkeeper_coach",
            Self::YouthCoach => "staff_youth_coach",
            Self::U21Manager => "staff_u21_manager",
            Self::U19Manager => "staff_u19_manager",
            Self::Scout => "staff_scout",
            Self::ChiefScout => "staff_chief_scout",
            Self::Physio => "staff_physio",
            Self::HeadOfPhysio => "staff_head_of_physio",
            Self::Chairman => "staff_chairman",
            Self::Director => "staff_director",
            Self::ManagingDirector => "staff_managing_director",
            Self::DirectorOfFootball => "staff_director_of_football",
            Self::GeneralManager => "staff_general_manager",
            Self::HeadOfYouthDevelopment => "staff_head_of_youth_dev",
            Self::MediaPundit => "staff_media_pundit",
            Self::DataAnalyst => "staff_data_analyst",
            Self::HeadOfRecruitment => "staff_head_of_recruitment",
            Self::Free => "staff_free",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StaffStatus {
    Active,
    ExpiredContract,
}

#[derive(Debug, Clone)]
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
