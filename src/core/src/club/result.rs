use crate::club::academy::result::ClubAcademyResult;
use crate::club::{BoardResult, ClubFinanceResult};
use crate::simulator::SimulatorData;
use crate::transfers::CompletedTransfer;
use crate::{
    PlayerContractProposal, PlayerMessage, PlayerMessageType, PlayerResult, SimulationResult,
    StaffStatus, TeamResult,
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
                    Self::process_player_contract_interaction(player_result, data, self.club_id);
                }
            }

            team_result.process(data);
        }

        self.board.process(data);
        self.academy.process(data);
    }

    fn process_player_contract_interaction(result: &PlayerResult, data: &mut SimulatorData, club_id: u32) {
        if result.contract.no_contract || result.contract.want_improve_contract || result.contract.want_extend_contract {
            // Step 1: Resolve contract renewal staff and extract their skills
            let (negotiation_skill, judging_ability) = data.club(club_id)
                .and_then(|club| club.teams.teams.first())
                .map(|team| {
                    let staff = team.staffs.responsibility.contract_renewal.handle_first_team_contracts
                        .and_then(|id| team.staffs.staffs.iter().find(|s| s.id == id));
                    match staff {
                        Some(s) => {
                            // Check if staff has active contract (not resigned/expired)
                            let is_active = s.contract.as_ref()
                                .map(|c| matches!(c.status, StaffStatus::Active))
                                .unwrap_or(false);
                            if is_active {
                                (
                                    s.staff_attributes.mental.man_management,
                                    s.staff_attributes.knowledge.judging_player_ability,
                                )
                            } else {
                                // Staff resigned or contract expired — poor negotiation
                                (3u8, 3u8)
                            }
                        }
                        None => (5u8, 5u8), // No staff assigned — below average
                    }
                })
                .unwrap_or((5, 5));

            // Step 2: Look up the player
            let player = data.player(result.player_id).expect(&format!("player {} not found", result.player_id));

            // Don't auto-renew contracts for players on loan — those expire and the player returns
            if player.is_on_loan() {
                return;
            }

            let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
            let ability = player.player_attributes.current_ability;
            let base_salary = ability_based_salary(ability);

            // Staff judging_ability affects how accurate the salary offer is
            // Low skill: offer 70-85% of fair value, high skill: offer 95-105%
            let accuracy = 0.70 + (judging_ability as f32 / 20.0) * 0.35;
            let adjusted_base = (base_salary as f32 * accuracy) as u32;

            let offered_salary = if result.contract.want_improve_contract {
                // Staff evaluates whether this player deserves a raise
                let ability_f = ability as f32;
                let matches_played = player.statistics.played + player.statistics.played_subs;
                let is_not_needed = player.contract.as_ref()
                    .map(|c| matches!(c.squad_status, crate::PlayerSquadStatus::NotNeeded))
                    .unwrap_or(false);

                // Staff blocks: not needed players don't get raises
                if is_not_needed {
                    return;
                }

                // Low ability with few appearances: smaller raise
                let raise_pct = if ability_f < 60.0 && matches_played < 5 {
                    1.05
                } else {
                    1.10 + (ability_f / 200.0) * 0.10 // 10-15% raise
                };

                let raised = (current_salary as f32 * raise_pct) as u32;
                raised.max(adjusted_base).max(current_salary + 1)
            } else {
                // Extension/no-contract: offer at least their current salary
                adjusted_base.max(current_salary)
            };

            player.mailbox.push(PlayerMessage {
                message_type: PlayerMessageType::ContractProposal(PlayerContractProposal {
                    salary: offered_salary,
                    years: 3,
                    negotiation_skill,
                }),
            })
        }

        /// Ability-based salary that reflects realistic player market value
        fn ability_based_salary(ability: u8) -> u32 {
            match ability {
                0..=50 => 5_000,
                51..=70 => 20_000,
                71..=90 => 50_000,
                91..=110 => 100_000,
                111..=130 => 200_000,
                131..=150 => 350_000,
                _ => 500_000,
            }
        }
    }
}
