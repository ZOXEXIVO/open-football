use crate::club::academy::result::ClubAcademyResult;
use crate::club::{BoardResult, ClubFinanceResult};
use crate::simulator::SimulatorData;
use crate::transfers::CompletedTransfer;
use crate::{
    PlayerContractProposal, PlayerMessage, PlayerMessageType, PlayerResult, SimulationResult,
    TeamResult,
};

pub struct ClubResult {
    pub club_id: u32,
    pub finance: ClubFinanceResult,
    pub teams: Vec<TeamResult>,
    pub board: BoardResult,
    pub academy: ClubAcademyResult,
    pub academy_transfers: Vec<CompletedTransfer>,
}

impl ClubResult {
    pub fn new(
        club_id: u32,
        finance: ClubFinanceResult,
        teams: Vec<TeamResult>,
        board: BoardResult,
        academy: ClubAcademyResult,
    ) -> Self {
        ClubResult {
            club_id,
            finance,
            teams,
            board,
            academy,
            academy_transfers: Vec::new(),
        }
    }

    pub fn process(self, data: &mut SimulatorData, _result: &mut SimulationResult) {
        self.finance.process(data);

        for team_result in &self.teams {
            for player_result in &team_result.players.players {
                if player_result.has_contract_actions() {
                    Self::process_player_contract_interaction(player_result, data);
                }
            }

            team_result.process(data);
        }

        self.board.process(data);
        self.academy.process(data);
    }

    fn process_player_contract_interaction(result: &PlayerResult, data: &mut SimulatorData) {
        if result.contract.no_contract || result.contract.want_improve_contract || result.contract.want_extend_contract {
            let player = data.player(result.player_id).expect(&format!("player {} not found", result.player_id));

            // Don't auto-renew loan contracts — those expire and the player returns
            if let Some(ref contract) = player.contract {
                if contract.contract_type == crate::ContractType::Loan {
                    return;
                }
            }

            let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
            let player_growth_potential = player.growth_potential(data.date.date());
            let base_salary = get_contract_salary(player_growth_potential);

            let offered_salary = if result.contract.want_improve_contract {
                // Staff evaluates whether this player deserves a raise
                let ability = player.player_attributes.current_ability as f32;
                let matches_played = player.statistics.played + player.statistics.played_subs;
                let is_not_needed = player.contract.as_ref()
                    .map(|c| matches!(c.squad_status, crate::PlayerSquadStatus::NotNeeded))
                    .unwrap_or(false);

                // Staff blocks: low ability backup with few appearances, or not needed
                if is_not_needed || (ability < 60.0 && matches_played < 5) {
                    return;
                }

                // Staff offers a raise scaled by ability
                let raise_pct = 1.10 + (ability / 200.0) * 0.10; // 10-15% raise
                let raised = (current_salary as f32 * raise_pct) as u32;
                raised.max(base_salary).max(current_salary + 1)
            } else {
                // Extension/no-contract: offer at least their current salary
                base_salary.max(current_salary)
            };

            player.mailbox.push(PlayerMessage {
                message_type: PlayerMessageType::ContractProposal(PlayerContractProposal {
                    salary: offered_salary,
                    years: 3,
                }),
            })
        }

        fn get_contract_salary(player_growth_potential: u8) -> u32 {
            match player_growth_potential as u32 {
                0..=3 => 1000u32,
                4 => 2000u32,
                5 => 3000u32,
                _ => 1000u32,
            }
        }
    }
}
