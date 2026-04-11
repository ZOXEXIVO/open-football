use crate::{ContractType, Player, PlayerClubContract, PlayerContractProposal, PlayerSquadStatus};
use chrono::NaiveDate;

pub struct AcceptContractHandler;

impl AcceptContractHandler {
    pub fn process(player: &mut Player, proposal: PlayerContractProposal, now: NaiveDate) {
        let expiration = now
            .checked_add_signed(chrono::Duration::days(365 * proposal.years as i64))
            .unwrap_or(now);

        // Preserve existing shirt number and squad status when renewing
        let (shirt_number, squad_status) = player.contract.as_ref()
            .map(|c| (c.shirt_number, c.squad_status.clone()))
            .unwrap_or((None, PlayerSquadStatus::FirstTeamRegular));

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
            bonuses: vec![],
            clauses: vec![],
        });
    }
}
