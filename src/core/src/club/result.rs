use crate::club::academy::result::ClubAcademyResult;
use crate::club::{BoardResult, ClubFinanceResult};
use crate::simulator::SimulatorData;
use crate::transfers::CompletedTransfer;
use crate::{
    PlayerContractProposal, PlayerMessage, PlayerMessageType, PlayerResult, PlayerStatusType,
    SimulationResult, StaffStatus, TeamResult,
};
use crate::utils::DateUtils;

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
            // Step 1: Resolve contract renewal staff, wage budget, and current wage bill
            let (negotiation_skill, judging_ability, wage_budget, current_wage_bill) = data.club(club_id)
                .map(|club| {
                    let (neg, judge) = club.teams.teams.first()
                        .map(|team| {
                            let staff = team.staffs.responsibility.contract_renewal.handle_first_team_contracts
                                .and_then(|id| team.staffs.staffs.iter().find(|s| s.id == id));
                            match staff {
                                Some(s) => {
                                    let is_active = s.contract.as_ref()
                                        .map(|c| matches!(c.status, StaffStatus::Active))
                                        .unwrap_or(false);
                                    if is_active {
                                        (
                                            s.staff_attributes.mental.man_management,
                                            s.staff_attributes.knowledge.judging_player_ability,
                                        )
                                    } else {
                                        (3u8, 3u8)
                                    }
                                }
                                None => (5u8, 5u8),
                            }
                        })
                        .unwrap_or((5, 5));

                    let wb = club.board.season_targets.as_ref()
                        .map(|t| t.wage_budget as u32)
                        .unwrap_or(0);

                    let total_wages: u32 = club.teams.teams.iter()
                        .map(|t| t.get_annual_salary())
                        .sum();

                    (neg, judge, wb, total_wages)
                })
                .unwrap_or((5, 5, 0, 0));

            // Step 2: Look up the player (may have been released since the result was generated)
            let player = match data.player(result.player_id) {
                Some(p) => p,
                None => return,
            };

            // Don't auto-renew contracts for players on loan — those expire and the player returns
            if player.is_on_loan() {
                return;
            }

            let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
            let ability = player.player_attributes.current_ability;
            let age = DateUtils::age(player.birth_date, data.date.date());
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

            // Wage budget enforcement: don't offer salary that would bust the budget
            // The new salary replaces the current one, so check the net increase
            let salary_increase = offered_salary.saturating_sub(current_salary);
            if wage_budget > 0 && current_wage_bill + salary_increase > wage_budget {
                // Over budget: cap the offer to what the budget allows
                let remaining = wage_budget.saturating_sub(current_wage_bill);
                let capped_salary = current_salary + remaining;
                // If we can't even match current salary, skip the offer entirely
                if capped_salary <= current_salary && result.contract.want_improve_contract {
                    return;
                }
                let final_salary = capped_salary.max(current_salary);

                let years = negotiate_contract_years(player, age, negotiation_skill);

                player.mailbox.push(PlayerMessage {
                    message_type: PlayerMessageType::ContractProposal(PlayerContractProposal {
                        salary: final_salary,
                        years,
                        negotiation_skill,
                    }),
                });
                return;
            }

            let years = negotiate_contract_years(player, age, negotiation_skill);

            player.mailbox.push(PlayerMessage {
                message_type: PlayerMessageType::ContractProposal(PlayerContractProposal {
                    salary: offered_salary,
                    years,
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

        /// Contract duration negotiation.
        ///
        /// Player wants: long contract (job security, commitment signal)
        /// Club wants: shorter contract (flexibility if player declines)
        ///
        /// Factors:
        /// - Age: clubs offer shorter deals to older players; young stars get longer
        /// - Ability/reputation: high-profile players demand and get longer deals
        /// - Loyalty: loyal players accept shorter deals (trust the club)
        /// - Ambition: ambitious players push for longer deals (higher commitment)
        /// - Other club interest (Wnt/Enq/Bid statuses): gives player leverage for longer deals
        /// - Staff negotiation skill: better negotiator → result closer to club's preference
        fn negotiate_contract_years(
            player: &crate::Player,
            age: u8,
            negotiation_skill: u8,
        ) -> u8 {
            let ability = player.player_attributes.current_ability;
            let reputation = player.player_attributes.current_reputation;
            let loyalty = player.attributes.loyalty;
            let ambition = player.attributes.ambition;

            // --- Player desired years (what the agent demands) ---
            let mut player_years: f32 = 3.0;

            // High reputation players demand longer contracts (security)
            if reputation > 7000 {
                player_years += 2.0;
            } else if reputation > 4000 {
                player_years += 1.0;
            }

            // High ability players want commitment
            if ability > 150 {
                player_years += 1.0;
            } else if ability > 120 {
                player_years += 0.5;
            }

            // Young players with high potential want long-term deals
            if age < 24 && player.player_attributes.potential_ability > ability + 20 {
                player_years += 1.0;
            }

            // Ambitious players push for longer contracts
            // ambition is 0-20
            if ambition > 15.0 {
                player_years += 1.0;
            } else if ambition > 10.0 {
                player_years += 0.5;
            }

            // Low loyalty = wants flexibility to move, shorter preferred
            if loyalty < 5.0 {
                player_years -= 1.0;
            } else if loyalty < 10.0 {
                player_years -= 0.5;
            }

            // Other club interest gives player leverage → pushes for longer commitment
            let has_interest = player.statuses.get().iter().any(|s| {
                matches!(s, PlayerStatusType::Wnt | PlayerStatusType::Enq | PlayerStatusType::Bid)
            });
            if has_interest {
                player_years += 1.0;
            }

            // Older players know they can't demand as much
            if age >= 34 {
                player_years -= 2.0;
            } else if age >= 32 {
                player_years -= 1.0;
            } else if age >= 30 {
                player_years -= 0.5;
            }

            // --- Club desired years (what the club wants to offer) ---
            let mut club_years: f32 = 3.0;

            // Club wants shorter deals for older players (decline risk)
            if age >= 34 {
                club_years = 1.0;
            } else if age >= 32 {
                club_years = 1.5;
            } else if age >= 30 {
                club_years = 2.0;
            }

            // Club wants to lock in young prospects (protect investment)
            if age < 22 && ability > 80 {
                club_years += 2.0;
            } else if age < 24 {
                club_years += 1.0;
            }

            // Club wants to lock in star players
            if ability > 150 {
                club_years += 1.5;
            } else if ability > 120 {
                club_years += 1.0;
            }

            // Low ability/rotation players: club wants short deals
            if ability < 70 {
                club_years -= 1.0;
            }

            // --- Negotiation: compromise between player and club ---
            // Staff negotiation skill (0-20) determines how much the club gets its way
            // 0 skill → 50/50 split, 20 skill → 80% club's preference
            let staff_weight = 0.5 + (negotiation_skill as f32 / 20.0) * 0.3; // 0.5 to 0.8
            let negotiated = club_years * staff_weight + player_years * (1.0 - staff_weight);

            // Clamp to realistic range: 1-5 years
            (negotiated.round() as u8).clamp(1, 5)
        }
    }
}
