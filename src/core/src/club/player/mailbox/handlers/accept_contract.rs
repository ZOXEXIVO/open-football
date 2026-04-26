use crate::club::player::contract::contract::encode_threshold_pct;
use crate::{
    ContractBonus, ContractBonusType, ContractClause, ContractClauseType, ContractType, Player,
    PlayerClubContract, PlayerContractProposal, PlayerSquadStatus,
};
use chrono::NaiveDate;

pub struct AcceptContractHandler;

impl AcceptContractHandler {
    pub fn process(player: &mut Player, proposal: PlayerContractProposal, now: NaiveDate) {
        let expiration = now
            .checked_add_signed(chrono::Duration::days(365 * proposal.years as i64))
            .unwrap_or(now);

        // Preserve existing shirt number; squad status is taken from the
        // proposal promise when supplied (negotiated role), otherwise we
        // inherit the current role.
        let (shirt_number, current_status) = player
            .contract
            .as_ref()
            .map(|c| (c.shirt_number, c.squad_status.clone()))
            .unwrap_or((None, PlayerSquadStatus::FirstTeamRegular));

        let squad_status = proposal
            .squad_status_promise
            .clone()
            .unwrap_or(current_status);

        let bonuses = build_bonuses(&proposal);
        let clauses = build_clauses(&proposal);

        // `signing_bonus_paid = false` defers the actual cash transfer to
        // the next monthly finance pass — it scans contracts for unpaid
        // signing bonuses, pushes them as expenses, and flips the flag.
        // Storing the bonus inside `contract.bonuses` keeps it visible to
        // happiness / package-value scoring throughout the contract.
        player.contract = Some(PlayerClubContract {
            shirt_number,
            salary: proposal.salary,
            contract_type: ContractType::FullTime,
            squad_status,
            is_transfer_listed: false,
            transfer_status: Option::None,
            started: Some(now),
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
            bonuses,
            clauses,
            last_yearly_rise_year: None,
            last_loyalty_paid_year: None,
            signing_bonus_paid: false,
        });
    }
}

fn build_bonuses(p: &PlayerContractProposal) -> Vec<ContractBonus> {
    let mut bonuses = Vec::new();

    if p.signing_bonus > 0 {
        bonuses.push(ContractBonus::new(
            p.signing_bonus as i32,
            ContractBonusType::SigningBonus,
        ));
    }
    if p.loyalty_bonus > 0 {
        bonuses.push(ContractBonus::new(
            p.loyalty_bonus as i32,
            ContractBonusType::LoyaltyBonus,
        ));
    }
    push_optional(&mut bonuses, p.appearance_fee, ContractBonusType::AppearanceFee);
    push_optional(
        &mut bonuses,
        p.unused_sub_fee,
        ContractBonusType::UnusedSubstitutionFee,
    );
    push_optional(&mut bonuses, p.goal_bonus, ContractBonusType::GoalFee);
    push_optional(
        &mut bonuses,
        p.clean_sheet_bonus,
        ContractBonusType::CleanSheetFee,
    );
    push_optional(&mut bonuses, p.promotion_bonus, ContractBonusType::PromotionFee);
    push_optional(
        &mut bonuses,
        p.avoid_relegation_bonus,
        ContractBonusType::AvoidRelegationFee,
    );
    push_optional(
        &mut bonuses,
        p.international_cap_bonus,
        ContractBonusType::InternationalCapFee,
    );

    bonuses
}

fn build_clauses(p: &PlayerContractProposal) -> Vec<ContractClause> {
    let mut clauses = Vec::new();

    if let Some(release) = p.release_clause {
        clauses.push(ContractClause::new(
            release as i32,
            ContractClauseType::MinimumFeeRelease,
        ));
    }
    if let Some(release) = p.relegation_release {
        clauses.push(ContractClause::new(
            release as i32,
            ContractClauseType::RelegationFeeRelease,
        ));
    }
    if let Some(release) = p.non_promotion_release {
        clauses.push(ContractClause::new(
            release as i32,
            ContractClauseType::NonPromotionRelease,
        ));
    }
    if let Some(pct) = p.yearly_wage_rise_pct {
        if pct > 0 {
            clauses.push(ContractClause::new(
                pct as i32,
                ContractClauseType::YearlyWageRise,
            ));
        }
    }
    if let Some(pct) = p.promotion_wage_increase_pct {
        if pct > 0 {
            clauses.push(ContractClause::new(
                pct as i32,
                ContractClauseType::PromotionWageIncrease,
            ));
        }
    }
    if let Some(pct) = p.relegation_wage_decrease_pct {
        if pct > 0 {
            clauses.push(ContractClause::new(
                pct as i32,
                ContractClauseType::RelegationWageDecrease,
            ));
        }
    }
    if let Some(years) = p.optional_extension_years {
        if years > 0 {
            clauses.push(ContractClause::new(
                years as i32,
                ContractClauseType::OptionalContractExtensionByClub,
            ));
        }
    }
    if let Some(threshold) = p.appearance_extension_threshold {
        if threshold > 0 {
            clauses.push(ContractClause::new(
                threshold as i32,
                ContractClauseType::OneYearExtensionAfterLeagueGamesFinalSeason,
            ));
        }
    }
    if let Some((threshold, pct)) = p.wage_after_apps {
        // Pack threshold and negotiated rise percentage into a single
        // i32 (`threshold * 100 + pct`) so we don't widen ContractClause
        // for two thresholded clauses. Decoded at apply-time.
        if threshold > 0 {
            clauses.push(ContractClause::new(
                encode_threshold_pct(threshold, pct),
                ContractClauseType::WageAfterReachingClubCareerLeagueGames,
            ));
        }
    }
    if let Some((threshold, pct)) = p.wage_after_caps {
        if threshold > 0 {
            clauses.push(ContractClause::new(
                encode_threshold_pct(threshold, pct),
                ContractClauseType::WageAfterReachingInternationalCaps,
            ));
        }
    }
    if p.match_highest_earner {
        clauses.push(ContractClause::new(
            1,
            ContractClauseType::MatchHighestEarner,
        ));
    }

    clauses
}

fn push_optional(out: &mut Vec<ContractBonus>, value: Option<u32>, kind: ContractBonusType) {
    if let Some(v) = value {
        if v > 0 {
            out.push(ContractBonus::new(v as i32, kind));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerContractProposal;

    fn rich_proposal() -> PlayerContractProposal {
        let mut p = PlayerContractProposal::basic(150_000, 4, 12, 30_000, 15_000, Some(25_000_000));
        p.appearance_fee = Some(2_000);
        p.unused_sub_fee = Some(500);
        p.goal_bonus = Some(5_000);
        p.clean_sheet_bonus = Some(0); // 0 -> not installed
        p.promotion_bonus = Some(50_000);
        p.avoid_relegation_bonus = Some(40_000);
        p.international_cap_bonus = Some(3_000);
        p.relegation_release = Some(8_000_000);
        p.non_promotion_release = Some(15_000_000);
        p.yearly_wage_rise_pct = Some(8);
        p.promotion_wage_increase_pct = Some(15);
        p.relegation_wage_decrease_pct = Some(20);
        p.optional_extension_years = Some(1);
        p.appearance_extension_threshold = Some(25);
        p.wage_after_apps = Some((100, 20));
        p.wage_after_caps = Some((10, 15));
        p.match_highest_earner = true;
        p
    }

    #[test]
    fn proposal_installs_all_negotiated_bonuses() {
        let proposal = rich_proposal();
        let bonuses = build_bonuses(&proposal);
        let kinds: Vec<_> = bonuses
            .iter()
            .map(|b| std::mem::discriminant(&b.bonus_type))
            .collect();
        // Signing + loyalty + appearance + unused_sub + goal + promotion +
        // avoid_relegation + international_cap = 8 (clean_sheet=0 skipped)
        assert_eq!(bonuses.len(), 8, "got {bonuses:?}");
        let _ = kinds; // discriminant equality is hard; len + present checks below
        assert!(bonuses
            .iter()
            .any(|b| matches!(b.bonus_type, ContractBonusType::SigningBonus) && b.value == 30_000));
        assert!(bonuses.iter().any(|b| matches!(
            b.bonus_type,
            ContractBonusType::AppearanceFee
        )));
        assert!(bonuses.iter().any(|b| matches!(
            b.bonus_type,
            ContractBonusType::PromotionFee
        )));
        // clean_sheet_bonus value=0 must NOT install
        assert!(!bonuses
            .iter()
            .any(|b| matches!(b.bonus_type, ContractBonusType::CleanSheetFee)));
    }

    #[test]
    fn proposal_installs_all_negotiated_clauses() {
        let proposal = rich_proposal();
        let clauses = build_clauses(&proposal);
        // release + relegation_release + non_promotion_release +
        // yearly_wage_rise + promotion_wage_increase + relegation_wage_decrease +
        // optional_extension + appearance_extension + wage_after_apps +
        // wage_after_caps + match_highest_earner = 11
        assert_eq!(clauses.len(), 11, "got {clauses:?}");
        assert!(clauses
            .iter()
            .any(|c| matches!(c.bonus_type, ContractClauseType::MinimumFeeRelease)));
        assert!(clauses
            .iter()
            .any(|c| matches!(c.bonus_type, ContractClauseType::RelegationFeeRelease)));
        assert!(clauses
            .iter()
            .any(|c| matches!(c.bonus_type, ContractClauseType::NonPromotionRelease)));
        assert!(clauses
            .iter()
            .any(|c| matches!(c.bonus_type, ContractClauseType::MatchHighestEarner)));
    }

    #[test]
    fn zero_or_none_optionals_do_not_install() {
        let proposal = PlayerContractProposal::basic(100_000, 3, 10, 0, 0, None);
        let bonuses = build_bonuses(&proposal);
        let clauses = build_clauses(&proposal);
        assert_eq!(bonuses.len(), 0);
        assert_eq!(clauses.len(), 0);
    }
}
