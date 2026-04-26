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

    /// Year of the most recent yearly-wage-rise application. The clause is
    /// recurring (not consumed) and the apply helper is invoked daily, so
    /// without a per-year memo a same-day double call would double-apply
    /// the rise. Cleared by `new()` to None.
    pub last_yearly_rise_year: Option<i32>,
    /// Year in which the loyalty bonus was last paid. Loyalty pays once
    /// per contract anniversary; this guards against duplicate payments
    /// when end-of-period or anniversary checks fire more than once.
    pub last_loyalty_paid_year: Option<i32>,
    /// Anchor year for one-shot signing-bonus payment. Set when the
    /// contract is first installed; if the bonus has been paid out the
    /// flag is `Some(year)` and we don't pay again on subsequent ticks.
    pub signing_bonus_paid: bool,
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
            last_yearly_rise_year: None,
            last_loyalty_paid_year: None,
            signing_bonus_paid: false,
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
            last_yearly_rise_year: None,
            last_loyalty_paid_year: None,
            signing_bonus_paid: false,
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
            last_yearly_rise_year: None,
            last_loyalty_paid_year: None,
            signing_bonus_paid: false,
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

    /// Apply the yearly wage rise clause if a contract anniversary falls
    /// in the supplied window. The clause is consumed-and-replayed each
    /// year because the new salary becomes the baseline; the clause
    /// itself stays on the contract for future anniversaries.
    /// Returns the new salary if a rise was applied.
    ///
    /// Idempotency: the caller passes a window (typically once per day);
    /// if the anniversary date falls in that window we apply once. We
    /// guard against double-application by demanding the date equals the
    /// anniversary exactly — running this every day across the year only
    /// fires on one matching day.
    pub fn try_apply_yearly_wage_rise(&mut self, today: NaiveDate) -> Option<u32> {
        let started = self.started?;
        if today.month() != started.month() || today.day() != started.day() {
            return None;
        }
        if today.year() <= started.year() {
            return None; // Anniversary at year of signing is the signing itself.
        }
        // Per-year memo: the daily caller would otherwise re-apply the
        // rise on every same-day re-entry. Skip if we already applied this
        // calendar year.
        if self.last_yearly_rise_year == Some(today.year()) {
            return None;
        }
        let pct = self.clauses.iter().find_map(|c| match c.bonus_type {
            ContractClauseType::YearlyWageRise => Some(c.value.max(0) as u32),
            _ => None,
        })?;
        if pct == 0 {
            return None;
        }
        let bump = (self.salary as u64 * pct as u64 / 100) as u32;
        self.salary = self.salary.saturating_add(bump.max(1));
        self.last_yearly_rise_year = Some(today.year());
        Some(self.salary)
    }

    /// Apply the promotion wage increase. Consumes the clause so it
    /// can't fire twice in subsequent seasons (the new salary is
    /// permanent). Returns the new salary if applied.
    pub fn apply_promotion_wage_increase(&mut self) -> Option<u32> {
        let pos = self.clauses.iter().position(|c| {
            matches!(
                c.bonus_type,
                ContractClauseType::PromotionWageIncrease
                    | ContractClauseType::TopDivisionPromotionWageRise
            )
        })?;
        let pct = self.clauses[pos].value.max(0) as u32;
        if pct > 0 {
            let bump = (self.salary as u64 * pct as u64 / 100) as u32;
            self.salary = self.salary.saturating_add(bump.max(1));
        }
        self.clauses.remove(pos);
        Some(self.salary)
    }

    /// Apply the relegation wage decrease. Consumes the clause; symmetric
    /// to `apply_promotion_wage_increase`.
    pub fn apply_relegation_wage_decrease(&mut self) -> Option<u32> {
        let pos = self.clauses.iter().position(|c| {
            matches!(
                c.bonus_type,
                ContractClauseType::RelegationWageDecrease
                    | ContractClauseType::TopDivisionRelegationWageDrop
            )
        })?;
        let pct = self.clauses[pos].value.max(0) as u32;
        if pct > 0 {
            let drop = (self.salary as u64 * pct as u64 / 100) as u32;
            self.salary = self.salary.saturating_sub(drop);
        }
        self.clauses.remove(pos);
        Some(self.salary)
    }

    /// Activate a relegation release clause. Returns the threshold fee.
    /// Consumes the clause — once relegation has happened it has either
    /// triggered a sale or expired with the season.
    pub fn take_relegation_release(&mut self) -> Option<i32> {
        let pos = self
            .clauses
            .iter()
            .position(|c| matches!(c.bonus_type, ContractClauseType::RelegationFeeRelease))?;
        let value = self.clauses[pos].value;
        // Convert the relegation-release into a generic minimum-fee
        // release for the upcoming window so the transfer pipeline picks
        // it up. It's then consumed normally on sale or season end.
        self.clauses[pos].bonus_type = ContractClauseType::MinimumFeeRelease;
        Some(value)
    }

    /// Activate a non-promotion release clause. Same pattern as
    /// `take_relegation_release`.
    pub fn take_non_promotion_release(&mut self) -> Option<i32> {
        let pos = self
            .clauses
            .iter()
            .position(|c| matches!(c.bonus_type, ContractClauseType::NonPromotionRelease))?;
        let value = self.clauses[pos].value;
        self.clauses[pos].bonus_type = ContractClauseType::MinimumFeeRelease;
        Some(value)
    }

    /// Exercise the optional contract extension by the club. Adds the
    /// clause's stored years to expiration and removes the clause.
    pub fn exercise_optional_extension(&mut self) -> Option<NaiveDate> {
        let pos = self.clauses.iter().position(|c| {
            matches!(
                c.bonus_type,
                ContractClauseType::OptionalContractExtensionByClub
            )
        })?;
        let years = self.clauses[pos].value.max(0) as i32;
        if years == 0 {
            self.clauses.remove(pos);
            return None;
        }
        // Calendar-year shift preserves month/day so the contract still
        // has a coherent anniversary; falls back to 365-day arithmetic
        // only on impossible dates (Feb 29 across a non-leap target year).
        let target_year = self.expiration.year() + years;
        let new_exp = self
            .expiration
            .with_year(target_year)
            .or_else(|| self.expiration.checked_add_signed(chrono::Duration::days(365 * years as i64)))?;
        self.expiration = new_exp;
        self.clauses.remove(pos);
        Some(self.expiration)
    }

    /// One-year auto-extension after final-season league games threshold.
    /// `apps_this_season` is supplied by caller; clause holds the
    /// threshold. Fires only in the final season — guarded by checking
    /// that fewer than 365 days remain.
    pub fn try_apply_appearance_extension(
        &mut self,
        apps_this_season: u16,
        today: NaiveDate,
    ) -> Option<NaiveDate> {
        if (self.expiration - today).num_days() > 365 {
            return None;
        }
        let pos = self.clauses.iter().position(|c| {
            matches!(
                c.bonus_type,
                ContractClauseType::OneYearExtensionAfterLeagueGamesFinalSeason
            )
        })?;
        let threshold = self.clauses[pos].resolved_threshold();
        if apps_this_season < threshold {
            return None;
        }
        // Calendar-year shift: keep the same month/day, advance the year.
        // 365-day arithmetic drifts a day every leap year and breaks the
        // "contract anniversary" property the yearly_wage_rise helper
        // relies on.
        let new_exp = self
            .expiration
            .with_year(self.expiration.year() + 1)
            .or_else(|| self.expiration.checked_add_signed(chrono::Duration::days(365)))?;
        self.expiration = new_exp;
        self.clauses.remove(pos);
        Some(self.expiration)
    }

    /// Wage rise after a club-career league-games threshold. Reads
    /// `clause.threshold` + `clause.percentage` when present; falls back
    /// to plain `value` as the threshold and a 20% default rise for
    /// legacy clauses. Consumes the clause once fired.
    pub fn try_apply_wage_after_career_apps(&mut self, career_apps: u32) -> Option<u32> {
        let pos = self.clauses.iter().position(|c| {
            matches!(
                c.bonus_type,
                ContractClauseType::WageAfterReachingClubCareerLeagueGames
            )
        })?;
        let threshold = self.clauses[pos].resolved_threshold();
        let pct = self.clauses[pos].resolved_percentage(20);
        if career_apps < threshold as u32 {
            return None;
        }
        let bump = (self.salary as u64 * pct as u64 / 100) as u32;
        self.salary = self.salary.saturating_add(bump.max(1));
        self.clauses.remove(pos);
        Some(self.salary)
    }

    /// Wage rise after international caps cross a threshold. Same shape
    /// as `try_apply_wage_after_career_apps`; default rise is 15% when
    /// the negotiated percentage isn't recorded.
    pub fn try_apply_wage_after_caps(&mut self, caps: u16) -> Option<u32> {
        let pos = self.clauses.iter().position(|c| {
            matches!(
                c.bonus_type,
                ContractClauseType::WageAfterReachingInternationalCaps
            )
        })?;
        let threshold = self.clauses[pos].resolved_threshold();
        let pct = self.clauses[pos].resolved_percentage(15);
        if caps < threshold {
            return None;
        }
        let bump = (self.salary as u64 * pct as u64 / 100) as u32;
        self.salary = self.salary.saturating_add(bump.max(1));
        self.clauses.remove(pos);
        Some(self.salary)
    }

    /// Match-highest-earner: lift the salary up to the supplied top
    /// earner if the player's clause is in force. Anti-loop: the new
    /// salary itself is what becomes the new top earner only if the
    /// caller passes the *current* top earner *excluding this player*.
    /// We never raise above the supplied top, so feeding back this
    /// player's just-raised wage cannot escalate further on the next call.
    pub fn try_apply_match_highest_earner(&mut self, top_earner_excl_self: u32) -> Option<u32> {
        let has_clause = self
            .clauses
            .iter()
            .any(|c| matches!(c.bonus_type, ContractClauseType::MatchHighestEarner));
        if !has_clause {
            return None;
        }
        if top_earner_excl_self <= self.salary {
            return None;
        }
        self.salary = top_earner_excl_self;
        Some(self.salary)
    }
}

/// True for bonus types that have no payout site in the simulation.
/// Acceptance/install code filters these out so the renewal or transfer
/// AI can't accidentally install a decorative bonus that never costs
/// the club anything but inflates the player's perceived package value.
///
/// To make a type "active" again, implement the payout (in
/// `process_contract_bonuses` or `settle_lump_sum_bonuses`) and remove
/// it from this list.
pub fn is_inert_bonus(bonus_type: &ContractBonusType) -> bool {
    matches!(
        bonus_type,
        ContractBonusType::TeamOfTheYear | ContractBonusType::TopGoalscorer
    )
}

/// True for clause types with no apply/lifecycle hook in the
/// simulation. Acceptance filters them out for the same reason as
/// `is_inert_bonus`.
pub fn is_inert_clause(clause_type: &ContractClauseType) -> bool {
    matches!(
        clause_type,
        ContractClauseType::SellOnFee
            | ContractClauseType::SellOnFeeProfit
            | ContractClauseType::SeasonalLandmarkGoalBonus
            | ContractClauseType::StaffJobRelease
            | ContractClauseType::MinimumFeeReleaseToHigherDivisionClubs
            | ContractClauseType::TopDivisionPromotionWageRise
            | ContractClauseType::TopDivisionRelegationWageDrop
    )
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
    /// Single-number payload — release fee, percentage, or extension years
    /// depending on `bonus_type`. Kept for backward compatibility and as
    /// the dominant carrier for clauses that only need one number.
    pub value: i32,
    pub bonus_type: ContractClauseType,
    /// Optional appearances/caps threshold for clauses gated by a count
    /// (WageAfterReachingClubCareerLeagueGames,
    /// WageAfterReachingInternationalCaps,
    /// OneYearExtensionAfterLeagueGamesFinalSeason). When `Some`, this is
    /// authoritative — `value` is ignored as the threshold.
    pub threshold: Option<u16>,
    /// Optional negotiated percentage for percentage-based clauses (yearly
    /// wage rise size, app-threshold rise size, cap-threshold rise size).
    /// When `Some`, this is authoritative — `value` is ignored as the pct.
    pub percentage: Option<u8>,
}

impl ContractClause {
    pub fn new(value: i32, bonus_type: ContractClauseType) -> Self {
        ContractClause {
            value,
            bonus_type,
            threshold: None,
            percentage: None,
        }
    }

    /// Threshold + negotiated percentage carrier. Keep `value` synced with
    /// the legacy encoding so any code path that still inspects `value`
    /// directly sees the same threshold.
    pub fn new_threshold_pct(
        threshold: u16,
        percentage: u8,
        bonus_type: ContractClauseType,
    ) -> Self {
        ContractClause {
            value: threshold as i32,
            bonus_type,
            threshold: Some(threshold),
            percentage: Some(percentage),
        }
    }

    /// Read the threshold for a count-gated clause. Prefers the explicit
    /// `threshold` field; otherwise falls back to `value` (legacy data).
    pub fn resolved_threshold(&self) -> u16 {
        self.threshold
            .unwrap_or_else(|| self.value.max(0).min(u16::MAX as i32) as u16)
    }

    /// Read the percentage for a percentage-bearing clause. Prefers the
    /// explicit `percentage` field; falls back to `default_pct` when not
    /// set. (We never reinterpret `value` as both threshold and percentage
    /// any more — the brittle `* 100 + pct` encoding is gone.)
    pub fn resolved_percentage(&self, default_pct: u8) -> u8 {
        self.percentage
            .filter(|p| *p > 0)
            .unwrap_or(default_pct)
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

#[cfg(test)]
mod clause_lifecycle_tests {
    use super::*;

    fn fresh(salary: u32) -> PlayerClubContract {
        let mut c = PlayerClubContract::new(
            salary,
            NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
        );
        c.started = Some(NaiveDate::from_ymd_opt(2025, 7, 1).unwrap());
        c
    }

    #[test]
    fn yearly_wage_rise_applies_on_anniversary_only() {
        let mut c = fresh(100_000);
        c.clauses
            .push(ContractClause::new(10, ContractClauseType::YearlyWageRise));
        // Same date as start — no rise (signing day).
        assert!(c
            .try_apply_yearly_wage_rise(NaiveDate::from_ymd_opt(2025, 7, 1).unwrap())
            .is_none());
        // Day after — no rise.
        assert!(c
            .try_apply_yearly_wage_rise(NaiveDate::from_ymd_opt(2025, 7, 2).unwrap())
            .is_none());
        // First anniversary — applies.
        let new_salary = c
            .try_apply_yearly_wage_rise(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap())
            .unwrap();
        assert_eq!(new_salary, 110_000);
        // Same day called again immediately — does not apply twice
        // because the salary has changed, but the helper has no per-day
        // memo. The simulation invokes once per day; we assert that the
        // per-day call DOES apply (mathematically idempotent across
        // years not days). The protection is that the loop runs
        // process_contract once per day and the 2nd anniversary is a
        // year later. So called the same day -> applies again. The
        // operational guard is the daily granularity.
    }

    #[test]
    fn yearly_wage_rise_no_clause_returns_none() {
        let mut c = fresh(100_000);
        assert!(c
            .try_apply_yearly_wage_rise(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap())
            .is_none());
    }

    #[test]
    fn promotion_wage_increase_applies_once_then_clause_gone() {
        let mut c = fresh(100_000);
        c.clauses.push(ContractClause::new(
            20,
            ContractClauseType::PromotionWageIncrease,
        ));
        let new_salary = c.apply_promotion_wage_increase().unwrap();
        assert_eq!(new_salary, 120_000);
        // Second call: nothing to apply.
        assert!(c.apply_promotion_wage_increase().is_none());
        assert_eq!(c.salary, 120_000);
    }

    #[test]
    fn relegation_wage_decrease_applies_once_then_clause_gone() {
        let mut c = fresh(100_000);
        c.clauses.push(ContractClause::new(
            25,
            ContractClauseType::RelegationWageDecrease,
        ));
        let new_salary = c.apply_relegation_wage_decrease().unwrap();
        assert_eq!(new_salary, 75_000);
        assert!(c.apply_relegation_wage_decrease().is_none());
        assert_eq!(c.salary, 75_000);
    }

    #[test]
    fn relegation_release_converts_to_minimum_fee() {
        let mut c = fresh(100_000);
        c.clauses.push(ContractClause::new(
            5_000_000,
            ContractClauseType::RelegationFeeRelease,
        ));
        let value = c.take_relegation_release().unwrap();
        assert_eq!(value, 5_000_000);
        // Now the release acts as a generic minimum-fee release.
        assert!(matches!(
            c.release_clause_triggered(5_000_000.0, false),
            Some(ContractClauseType::MinimumFeeRelease)
        ));
    }

    #[test]
    fn optional_extension_pushes_expiration_and_consumes_clause() {
        let mut c = fresh(100_000);
        let original = c.expiration;
        c.clauses.push(ContractClause::new(
            2,
            ContractClauseType::OptionalContractExtensionByClub,
        ));
        let new_exp = c.exercise_optional_extension().unwrap();
        assert!(new_exp > original);
        assert!(c.exercise_optional_extension().is_none());
    }

    #[test]
    fn appearance_extension_only_in_final_year() {
        let mut c = fresh(100_000);
        // Expiration far away — clause shouldn't fire even with apps.
        c.expiration = NaiveDate::from_ymd_opt(2030, 6, 30).unwrap();
        c.clauses.push(ContractClause::new(
            25,
            ContractClauseType::OneYearExtensionAfterLeagueGamesFinalSeason,
        ));
        let today = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        assert!(c.try_apply_appearance_extension(30, today).is_none());

        // Move into final-year window.
        c.expiration = NaiveDate::from_ymd_opt(2027, 4, 1).unwrap();
        // Below threshold.
        assert!(c.try_apply_appearance_extension(20, today).is_none());
        // At threshold — extension applies (~+365 days; leap-year drift OK).
        let new_exp = c.try_apply_appearance_extension(25, today).unwrap();
        assert!(new_exp >= NaiveDate::from_ymd_opt(2028, 3, 31).unwrap());
        assert!(new_exp <= NaiveDate::from_ymd_opt(2028, 4, 1).unwrap());
        assert!(c.try_apply_appearance_extension(50, today).is_none());
    }

    #[test]
    fn match_highest_earner_lifts_to_top_but_does_not_loop() {
        let mut c = fresh(80_000);
        c.clauses.push(ContractClause::new(
            1,
            ContractClauseType::MatchHighestEarner,
        ));
        let new_salary = c.try_apply_match_highest_earner(120_000).unwrap();
        assert_eq!(new_salary, 120_000);
        // Repeating with the same top — no further raise (anti-loop).
        assert!(c.try_apply_match_highest_earner(120_000).is_none());
        assert_eq!(c.salary, 120_000);
    }

    #[test]
    fn wage_after_career_apps_fires_at_threshold_only() {
        let mut c = fresh(100_000);
        c.clauses.push(ContractClause::new_threshold_pct(
            100,
            20,
            ContractClauseType::WageAfterReachingClubCareerLeagueGames,
        ));
        assert!(c.try_apply_wage_after_career_apps(99).is_none());
        let new_salary = c.try_apply_wage_after_career_apps(100).unwrap();
        assert_eq!(new_salary, 120_000); // +20%
        assert!(c.try_apply_wage_after_career_apps(200).is_none());
    }

    #[test]
    fn wage_after_career_apps_uses_negotiated_percentage() {
        let mut c = fresh(100_000);
        c.clauses.push(ContractClause::new_threshold_pct(
            50,
            35,
            ContractClauseType::WageAfterReachingClubCareerLeagueGames,
        ));
        let new_salary = c.try_apply_wage_after_career_apps(50).unwrap();
        assert_eq!(new_salary, 135_000);
    }

    #[test]
    fn wage_after_caps_fires_at_threshold_only() {
        let mut c = fresh(100_000);
        c.clauses.push(ContractClause::new_threshold_pct(
            10,
            15,
            ContractClauseType::WageAfterReachingInternationalCaps,
        ));
        assert!(c.try_apply_wage_after_caps(9).is_none());
        let new_salary = c.try_apply_wage_after_caps(10).unwrap();
        assert_eq!(new_salary, 115_000); // +15%
        assert!(c.try_apply_wage_after_caps(20).is_none());
    }

    #[test]
    fn wage_after_caps_uses_negotiated_percentage() {
        let mut c = fresh(100_000);
        c.clauses.push(ContractClause::new_threshold_pct(
            5,
            25,
            ContractClauseType::WageAfterReachingInternationalCaps,
        ));
        let new_salary = c.try_apply_wage_after_caps(5).unwrap();
        assert_eq!(new_salary, 125_000);
    }

    #[test]
    fn yearly_wage_rise_is_idempotent_within_a_day() {
        let mut c = fresh(100_000);
        c.clauses
            .push(ContractClause::new(10, ContractClauseType::YearlyWageRise));
        let anniversary = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let s1 = c.try_apply_yearly_wage_rise(anniversary).unwrap();
        assert_eq!(s1, 110_000);
        assert!(c.try_apply_yearly_wage_rise(anniversary).is_none());
        assert_eq!(c.salary, 110_000);
        let next = NaiveDate::from_ymd_opt(2027, 7, 1).unwrap();
        let s2 = c.try_apply_yearly_wage_rise(next).unwrap();
        assert_eq!(s2, 121_000);
    }

    #[test]
    fn legacy_threshold_clauses_still_decode_as_plain_threshold() {
        // Pre-refactor clauses had only `value` set to the raw threshold
        // (50, 100, 150 etc). The new resolved_threshold/percentage path
        // must still treat them as raw thresholds and use the default
        // percentage rather than dividing by 100.
        for raw in [50, 100, 150] {
            let mut c = fresh(100_000);
            c.clauses.push(ContractClause::new(
                raw,
                ContractClauseType::WageAfterReachingClubCareerLeagueGames,
            ));
            let just_under = (raw - 1) as u32;
            let exactly_at = raw as u32;
            assert!(c.try_apply_wage_after_career_apps(just_under).is_none());
            let new_salary = c.try_apply_wage_after_career_apps(exactly_at).unwrap();
            // Default fallback rise is +20% → 120_000.
            assert_eq!(new_salary, 120_000, "legacy threshold {} broke", raw);
        }
    }

    #[test]
    fn optional_extension_calendar_year_shift_preserves_anniversary() {
        // Calendar-year shift (`with_year(year + N)`) keeps month/day so
        // the anniversary stays clean, unlike `+ 365 days` which drifts
        // a day every leap year.
        let mut c = fresh(100_000);
        c.expiration = NaiveDate::from_ymd_opt(2030, 6, 30).unwrap();
        c.clauses.push(ContractClause::new(
            2,
            ContractClauseType::OptionalContractExtensionByClub,
        ));
        let new_exp = c.exercise_optional_extension().unwrap();
        assert_eq!(new_exp, NaiveDate::from_ymd_opt(2032, 6, 30).unwrap());
    }

    #[test]
    fn appearance_extension_calendar_year_shift_preserves_anniversary() {
        // Inside the final-365-day window so the helper's gate passes.
        let mut c = fresh(100_000);
        c.expiration = NaiveDate::from_ymd_opt(2027, 4, 1).unwrap();
        c.clauses.push(ContractClause::new(
            25,
            ContractClauseType::OneYearExtensionAfterLeagueGamesFinalSeason,
        ));
        let today = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let new_exp = c.try_apply_appearance_extension(30, today).unwrap();
        // Year + 1, same month/day — calendar shift, no leap-year drift.
        assert_eq!(new_exp, NaiveDate::from_ymd_opt(2028, 4, 1).unwrap());
    }

    #[test]
    fn inert_bonuses_and_clauses_are_recognised() {
        // The is_inert_* lists are the source of truth used by the
        // accept-contract install path to strip decorative types.
        assert!(is_inert_bonus(&ContractBonusType::TeamOfTheYear));
        assert!(is_inert_bonus(&ContractBonusType::TopGoalscorer));
        assert!(!is_inert_bonus(&ContractBonusType::SigningBonus));
        assert!(!is_inert_bonus(&ContractBonusType::AppearanceFee));

        assert!(is_inert_clause(&ContractClauseType::SellOnFee));
        assert!(is_inert_clause(&ContractClauseType::SellOnFeeProfit));
        assert!(is_inert_clause(&ContractClauseType::SeasonalLandmarkGoalBonus));
        assert!(is_inert_clause(&ContractClauseType::StaffJobRelease));
        assert!(is_inert_clause(
            &ContractClauseType::MinimumFeeReleaseToHigherDivisionClubs
        ));
        assert!(is_inert_clause(
            &ContractClauseType::TopDivisionPromotionWageRise
        ));
        assert!(is_inert_clause(
            &ContractClauseType::TopDivisionRelegationWageDrop
        ));
        // Sanity: the active types are NOT marked inert.
        assert!(!is_inert_clause(&ContractClauseType::MinimumFeeRelease));
        assert!(!is_inert_clause(&ContractClauseType::YearlyWageRise));
        assert!(!is_inert_clause(
            &ContractClauseType::OptionalContractExtensionByClub
        ));
    }
}
