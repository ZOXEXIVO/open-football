use crate::context::SimulationContext;
pub use chrono::prelude::{DateTime, Datelike, NaiveDate, Utc};
use chrono::NaiveDateTime;

#[derive(Debug, Clone, PartialEq)]
pub enum ContractType {
    PartTime,
    FullTime,
    Amateur,
    Youth,
    NonContract,
    Loan,
}

#[derive(Debug, Clone, PartialEq)]
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

impl PlayerSquadStatus {
    /// Squad status based on player CA rank within the team.
    /// `team_cas` should be sorted descending (best first).
    pub fn calculate(player_ca: u8, player_age: u8, team_cas: &[u8]) -> Self {
        let squad_size = team_cas.len();
        if squad_size == 0 {
            return PlayerSquadStatus::FirstTeamRegular;
        }

        // Youth players get youth-specific statuses
        if player_age <= 19 {
            let avg_ca = team_cas.iter().map(|&c| c as u32).sum::<u32>() / squad_size as u32;
            return if (player_ca as u32) >= avg_ca {
                PlayerSquadStatus::HotProspectForTheFuture
            } else {
                PlayerSquadStatus::DecentYoungster
            };
        }

        // Find player's rank in the squad (0 = best)
        let rank = team_cas.iter().filter(|&&ca| ca > player_ca).count();
        let percentile = rank as f32 / squad_size as f32;

        // Thresholds:
        // Top ~15% = Key Player (typically 3-4 players in a 25-man squad)
        // Next ~25% = First Team Regular
        // Next ~20% = Squad Rotation
        // Next ~20% = Backup
        // Bottom ~20% = Not Needed
        if percentile < 0.15 {
            PlayerSquadStatus::KeyPlayer
        } else if percentile < 0.40 {
            PlayerSquadStatus::FirstTeamRegular
        } else if percentile < 0.60 {
            PlayerSquadStatus::FirstTeamSquadRotation
        } else if percentile < 0.80 {
            PlayerSquadStatus::MainBackupPlayer
        } else {
            PlayerSquadStatus::NotNeeded
        }
    }
}

#[derive(Debug, Clone)]
pub enum PlayerTransferStatus {
    TransferListed,
    LoadListed,
    TransferAndLoadListed,
}

#[derive(Debug, Clone)]
pub struct PlayerClubContract {
    pub shirt_number: Option<u8>,

    pub salary: u32,
    pub contract_type: ContractType,
    pub squad_status: PlayerSquadStatus,

    pub is_transfer_listed: bool,
    pub transfer_status: Option<PlayerTransferStatus>,

    pub started: Option<NaiveDate>,
    pub expiration: NaiveDate,

    pub loan_from_club_id: Option<u32>,
    pub loan_from_team_id: Option<u32>,
    pub loan_to_club_id: Option<u32>,

    /// Fee the parent club pays the borrowing club per official match played.
    /// Incentivises the borrowing club to give the player minutes.
    pub loan_match_fee: Option<u32>,

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
            loan_from_club_id: None,
            loan_from_team_id: None,
            loan_to_club_id: None,
            loan_match_fee: None,
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
            loan_from_club_id: None,
            loan_from_team_id: None,
            loan_to_club_id: None,
            loan_match_fee: None,
            bonuses: vec![],
            clauses: vec![],
        }
    }

    pub fn new_loan(salary: u32, expiration: NaiveDate, from_club_id: u32, from_team_id: u32, to_club_id: u32) -> Self {
        PlayerClubContract {
            shirt_number: None,
            salary,
            contract_type: ContractType::Loan,
            squad_status: PlayerSquadStatus::NotYetSet,
            transfer_status: None,
            is_transfer_listed: false,
            started: Option::None,
            expiration,
            loan_from_club_id: Some(from_club_id),
            loan_from_team_id: Some(from_team_id),
            loan_to_club_id: Some(to_club_id),
            loan_match_fee: None,
            bonuses: vec![],
            clauses: vec![],
        }
    }

    pub fn with_loan_match_fee(mut self, fee: u32) -> Self {
        self.loan_match_fee = Some(fee);
        self
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
#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct ContractClause {
    pub value: i32,
    pub bonus_type: ContractClauseType,
}

impl ContractClause {
    pub fn new(value: i32, bonus_type: ContractClauseType) -> Self {
        ContractClause { value, bonus_type }
    }
}
