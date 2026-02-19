use crate::context::SimulationContext;
pub use chrono::prelude::{DateTime, Datelike, NaiveDate, Utc};
use chrono::NaiveDateTime;

#[derive(Debug, PartialEq)]
pub enum ContractType {
    PartTime,
    FullTime,
    Amateur,
    Youth,
    NonContract,
    Loan,
}

#[derive(Debug)]
pub enum PlayerSquadStatus {
    Invalid,
    NotYetSet,
    KeyPlayer,
    FirstTeamRegular,
    FirstTeamSquadRotation,
    MainBackupPlayer,
    HotProspectForTheFuture,
    DecentYoungster,
    NotNeeded,
    SquadStatusCount,
}

#[derive(Debug)]
pub enum PlayerTransferStatus {
    TransferListed,
    LoadListed,
    TransferAndLoadListed,
}

#[derive(Debug)]
pub struct PlayerClubContract {
    pub shirt_number: Option<u8>,

    pub salary: u32,
    pub contract_type: ContractType,
    pub squad_status: PlayerSquadStatus,

    pub is_transfer_listed: bool,
    pub transfer_status: Option<PlayerTransferStatus>,

    pub started: Option<NaiveDate>,
    pub expiration: NaiveDate,

    pub bonuses: Vec<ContractBonus>,
    pub clauses: Vec<ContractClause>,
}

impl PlayerClubContract {
    pub fn new(salary: u32, expired: NaiveDate) -> Self {
        PlayerClubContract {
            shirt_number: None,
            salary,
            contract_type: ContractType::FullTime,
            squad_status: PlayerSquadStatus::NotYetSet,
            transfer_status: None,
            is_transfer_listed: false,
            started: Option::None,
            expiration: expired,
            bonuses: vec![],
            clauses: vec![],
        }
    }

    pub fn new_youth(salary: u32, expiration: NaiveDate) -> Self {
        PlayerClubContract {
            shirt_number: None,
            salary,
            contract_type: ContractType::Youth,
            squad_status: PlayerSquadStatus::NotYetSet,
            transfer_status: None,
            is_transfer_listed: false,
            started: Option::None,
            expiration,
            bonuses: vec![],
            clauses: vec![],
        }
    }

    pub fn new_loan(salary: u32, expiration: NaiveDate) -> Self {
        PlayerClubContract {
            shirt_number: None,
            salary,
            contract_type: ContractType::Loan,
            squad_status: PlayerSquadStatus::NotYetSet,
            transfer_status: None,
            is_transfer_listed: false,
            started: Option::None,
            expiration,
            bonuses: vec![],
            clauses: vec![],
        }
    }

    pub fn is_expired(&self, now: NaiveDateTime) -> bool {
        self.expiration >= now.date()
    }

    pub fn days_to_expiration(&self, now: NaiveDateTime) -> i64 {
        let diff = self.expiration - now.date();
        diff.num_days().abs()
    }

    pub fn simulate(&mut self, context: &mut SimulationContext) {
        if context.check_contract_expiration() && self.is_expired(context.date) {}
    }
}

// Bonuses
#[derive(Debug)]
pub enum ContractBonusType {
    AppearanceFee,
    GoalFee,
    CleanSheetFee,
    TeamOfTheYear,
    TopGoalscorer,
    PromotionFee,
    AvoidRelegationFee,
    InternationalCapFee,
    UnusedSubstitutionFee,
}

#[derive(Debug)]
pub struct ContractBonus {
    pub value: i32,
    pub bonus_type: ContractBonusType,
}

impl ContractBonus {
    pub fn new(value: i32, bonus_type: ContractBonusType) -> Self {
        ContractBonus { value, bonus_type }
    }
}

// Clauses
#[derive(Debug)]
pub enum ContractClauseType {
    MinimumFeeRelease,
    RelegationFeeRelease,
    NonPromotionRelease,
    YearlyWageRise,
    PromotionWageIncrease,
    RelegationWageDecrease,
    StaffJobRelease,
    SellOnFee,
    SellOnFeeProfit,
    SeasonalLandmarkGoalBonus,
    OneYearExtensionAfterLeagueGamesFinalSeason,
    MatchHighestEarner,
    WageAfterReachingClubCareerLeagueGames,
    TopDivisionPromotionWageRise,
    TopDivisionRelegationWageDrop,
    MinimumFeeReleaseToForeignClubs,
    MinimumFeeReleaseToHigherDivisionClubs,
    MinimumFeeReleaseToDomesticClubs,
    WageAfterReachingInternationalCaps,
    OptionalContractExtensionByClub,
}

#[derive(Debug)]
pub struct ContractClause {
    pub value: i32,
    pub bonus_type: ContractClauseType,
}

impl ContractClause {
    pub fn new(value: i32, bonus_type: ContractClauseType) -> Self {
        ContractClause { value, bonus_type }
    }
}
