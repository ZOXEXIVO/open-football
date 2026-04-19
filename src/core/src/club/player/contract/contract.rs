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

    /// Percentage (0-100) of the player's wage the BORROWING club covers.
    /// The remainder is paid by the parent club. Defaults to 100 (full
    /// wage paid by borrower) when omitted.
    pub loan_wage_contribution_pct: Option<u8>,
    /// Optional future fee agreed at loan signing (obligation or option).
    /// Triggered at loan end via a separate transfer record.
    pub loan_future_fee: Option<u32>,
    /// Whether the `loan_future_fee` is an obligation (true) or an option (false).
    pub loan_future_fee_obligation: bool,
    /// Parent club may recall the loan at any time after this date.
    pub loan_recall_available_after: Option<NaiveDate>,
    /// Minimum number of official matches the borrowing club has to give
    /// the player; breaching it allows recall and/or waives the match fee.
    pub loan_min_appearances: Option<u16>,

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
            loan_wage_contribution_pct: None,
            loan_future_fee: None,
            loan_future_fee_obligation: false,
            loan_recall_available_after: None,
            loan_min_appearances: None,
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
            loan_wage_contribution_pct: None,
            loan_future_fee: None,
            loan_future_fee_obligation: false,
            loan_recall_available_after: None,
            loan_min_appearances: None,
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
            loan_wage_contribution_pct: None,
            loan_future_fee: None,
            loan_future_fee_obligation: false,
            loan_recall_available_after: None,
            loan_min_appearances: None,
            bonuses: vec![],
            clauses: vec![],
        }
    }

    pub fn with_loan_match_fee(mut self, fee: u32) -> Self {
        self.loan_match_fee = Some(fee);
        self
    }

    pub fn with_loan_wage_contribution(mut self, pct: u8) -> Self {
        self.loan_wage_contribution_pct = Some(pct.min(100));
        self
    }

    pub fn with_loan_future_fee(mut self, fee: u32, obligation: bool) -> Self {
        self.loan_future_fee = Some(fee);
        self.loan_future_fee_obligation = obligation;
        self
    }

    pub fn with_loan_recall(mut self, after: NaiveDate) -> Self {
        self.loan_recall_available_after = Some(after);
        self
    }

    pub fn with_loan_min_appearances(mut self, min: u16) -> Self {
        self.loan_min_appearances = Some(min);
        self
    }

    /// Share of the player's wage the parent club still pays (0-100).
    pub fn parent_wage_share_pct(&self) -> u8 {
        100u8.saturating_sub(self.loan_wage_contribution_pct.unwrap_or(100))
    }

    pub fn is_expired(&self, now: NaiveDateTime) -> bool {
        self.expiration < now.date()
    }

    pub fn days_to_expiration(&self, now: NaiveDateTime) -> i64 {
        (self.expiration - now.date()).num_days()
    }

    /// Severance the club must pay to tear this contract up today — the
    /// cost of a mutual termination. Mirrors FM: cheap to exit youth and
    /// part-time deals, fraction of the remaining wages for a full
    /// professional deal (player accepts a haircut in exchange for
    /// immediate freedom), zero for anything already expired.
    ///
    /// Returns 0 for loan contracts — those are recalled, not terminated.
    pub fn termination_cost(&self, date: NaiveDate) -> u32 {
        let days_remaining = (self.expiration - date).num_days();
        if days_remaining <= 0 {
            return 0;
        }

        let settlement_factor = match self.contract_type {
            ContractType::Loan | ContractType::Amateur | ContractType::NonContract => return 0,
            ContractType::Youth => 0.25,
            ContractType::PartTime => 0.35,
            ContractType::FullTime => 0.5,
        };

        let months_remaining = (days_remaining as f32 / 30.0).min(18.0);
        let monthly_wage = self.salary as f32 / 12.0;
        (months_remaining * monthly_wage * settlement_factor).max(0.0) as u32
    }

    /// Does an incoming bid match a release-clause threshold that forces
    /// the selling club to accept? Returns the clause type that triggered,
    /// or `None` if no clause applies. The club can still veto; callers
    /// override negotiation chance to "guaranteed" when Some.
    ///
    /// Division-tier variants require richer context and are deferred — the
    /// cross-country variants are the common ones in real football.
    pub fn release_clause_triggered(
        &self,
        offer_amount: f64,
        buyer_is_foreign: bool,
    ) -> Option<ContractClauseType> {
        for clause in &self.clauses {
            if offer_amount < clause.value as f64 {
                continue;
            }
            match clause.bonus_type {
                ContractClauseType::MinimumFeeRelease => {
                    return Some(ContractClauseType::MinimumFeeRelease);
                }
                ContractClauseType::MinimumFeeReleaseToForeignClubs if buyer_is_foreign => {
                    return Some(ContractClauseType::MinimumFeeReleaseToForeignClubs);
                }
                ContractClauseType::MinimumFeeReleaseToDomesticClubs if !buyer_is_foreign => {
                    return Some(ContractClauseType::MinimumFeeReleaseToDomesticClubs);
                }
                _ => {}
            }
        }
        None
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
    /// One-off payment on signature — opens closed doors in renewal talks.
    SigningBonus,
    /// Yearly loyalty bonus — paid for each full contract year served.
    LoyaltyBonus,
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

#[cfg(test)]
mod release_clause_tests {
    use super::*;

    fn base_contract() -> PlayerClubContract {
        PlayerClubContract::new(
            500_000,
            NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
        )
    }

    #[test]
    fn no_clause_means_no_trigger() {
        let c = base_contract();
        assert!(c.release_clause_triggered(50_000_000.0, false).is_none());
    }

    #[test]
    fn universal_clause_triggers_when_offer_meets_threshold() {
        let mut c = base_contract();
        c.clauses.push(ContractClause::new(30_000_000, ContractClauseType::MinimumFeeRelease));
        assert!(matches!(
            c.release_clause_triggered(30_000_000.0, false),
            Some(ContractClauseType::MinimumFeeRelease)
        ));
        assert!(matches!(
            c.release_clause_triggered(50_000_000.0, true),
            Some(ContractClauseType::MinimumFeeRelease)
        ));
    }

    #[test]
    fn universal_clause_does_not_trigger_below_threshold() {
        let mut c = base_contract();
        c.clauses.push(ContractClause::new(30_000_000, ContractClauseType::MinimumFeeRelease));
        assert!(c.release_clause_triggered(29_999_999.0, false).is_none());
    }

    #[test]
    fn foreign_only_clause_rejects_domestic_bidder() {
        let mut c = base_contract();
        c.clauses.push(ContractClause::new(
            20_000_000,
            ContractClauseType::MinimumFeeReleaseToForeignClubs,
        ));
        assert!(c.release_clause_triggered(25_000_000.0, false).is_none());
        assert!(matches!(
            c.release_clause_triggered(25_000_000.0, true),
            Some(ContractClauseType::MinimumFeeReleaseToForeignClubs)
        ));
    }

    #[test]
    fn domestic_only_clause_rejects_foreign_bidder() {
        let mut c = base_contract();
        c.clauses.push(ContractClause::new(
            20_000_000,
            ContractClauseType::MinimumFeeReleaseToDomesticClubs,
        ));
        assert!(c.release_clause_triggered(25_000_000.0, true).is_none());
        assert!(matches!(
            c.release_clause_triggered(25_000_000.0, false),
            Some(ContractClauseType::MinimumFeeReleaseToDomesticClubs)
        ));
    }

    #[test]
    fn multiple_clauses_first_match_wins() {
        let mut c = base_contract();
        c.clauses.push(ContractClause::new(
            50_000_000,
            ContractClauseType::MinimumFeeReleaseToDomesticClubs,
        ));
        c.clauses.push(ContractClause::new(30_000_000, ContractClauseType::MinimumFeeRelease));
        // Domestic bidder meeting the universal clause triggers it even when
        // domestic-only comes first in the list but its threshold is higher.
        assert!(matches!(
            c.release_clause_triggered(35_000_000.0, false),
            Some(ContractClauseType::MinimumFeeRelease)
        ));
    }

    #[test]
    fn unhandled_clause_types_do_not_trigger() {
        let mut c = base_contract();
        c.clauses.push(ContractClause::new(1_000_000, ContractClauseType::SellOnFee));
        c.clauses.push(ContractClause::new(1_000_000, ContractClauseType::RelegationFeeRelease));
        assert!(c.release_clause_triggered(100_000_000.0, false).is_none());
    }
}
