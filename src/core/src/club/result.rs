use crate::club::academy::result::ClubAcademyResult;
use crate::club::{BoardResult, ClubFinanceResult};
use crate::simulator::SimulatorData;
use crate::transfers::CompletedTransfer;
use crate::{
    Player, PlayerContractProposal, PlayerMessage, PlayerMessageType, PlayerResult,
    PlayerSquadStatus, PlayerStatusType, SimulationResult, StaffStatus, TeamResult,
};
use crate::utils::DateUtils;

enum UnresolvedSalaryDecision {
    TransferList,
    FreeTransfer,
}

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
        self.process_teams(data);
        self.board.process(data);
        self.academy.process(data);
    }

    fn process_teams(&self, data: &mut SimulatorData) {
        for team_result in &self.teams {
            for player_result in &team_result.players.players {
                if player_result.has_contract_actions() {
                    Self::process_player_contract_interaction(player_result, data, self.club_id);
                }
            }

            team_result.process(data);
        }
    }

    fn process_player_contract_interaction(result: &PlayerResult, data: &mut SimulatorData, club_id: u32) {
        let is_on_loan = data.player(result.player_id).map(|p| p.is_on_loan()).unwrap_or(false);

        // Contract rejected — club decides: keep trying, transfer list, or release
        // Loaned players can't be transfer-listed remotely by the parent club
        if result.contract.contract_rejected {
            if !is_on_loan {
                Self::handle_unresolved_salary(result.player_id, data, club_id);
            }
            return;
        }

        // Reactive renewal cooldown. process_contract sets
        // want_extend_contract every tick the contract is < 1 year from
        // expiry, and process_happiness sets want_improve_contract every
        // week the player is salary-unhappy. Without a gate here, the
        // club would send a fresh proposal each tick and pile up
        // rejection entries. Reuse the same retry rules as the proactive
        // ContractRenewalManager: 60-day cooldown after an offer, 120-day
        // cooldown after a rejection, max 3 attempts per rolling year.
        const OFFER_COOLDOWN_DAYS: i64 = 60;
        const REJECT_COOLDOWN_DAYS: i64 = 120;
        const MAX_ATTEMPTS_PER_YEAR: usize = 3;
        const OFFERED_LABEL: &str = "dec_contract_renewal_offered";
        const REJECTED_LABEL: &str = "dec_contract_renewal_rejected";
        let today = data.date.date();
        if let Some(player) = data.player(result.player_id) {
            let last = player
                .decision_history
                .items
                .iter()
                .rev()
                .find(|d| d.decision == OFFERED_LABEL || d.decision == REJECTED_LABEL);
            if let Some(d) = last {
                let days = (today - d.date).num_days();
                let cooldown = if d.decision == REJECTED_LABEL {
                    REJECT_COOLDOWN_DAYS
                } else {
                    OFFER_COOLDOWN_DAYS
                };
                if days < cooldown {
                    return;
                }
            }
            let attempts = player
                .decision_history
                .items
                .iter()
                .filter(|d| {
                    d.decision == OFFERED_LABEL
                        && (today - d.date).num_days() < 365
                })
                .count();
            if attempts >= MAX_ATTEMPTS_PER_YEAR {
                return;
            }
        }

        if result.contract.no_contract || result.contract.want_improve_contract || result.contract.want_extend_contract {
            // For loaned players, only handle parent contract extensions
            // Salary improvements are between the player and borrowing club — not relevant here
            if is_on_loan && !result.contract.want_extend_contract {
                return;
            }

            // Resolve which club handles the contract: parent club for loaned players
            let contract_club_id = if is_on_loan {
                match data.player(result.player_id)
                    .and_then(|p| p.contract_loan.as_ref())
                    .and_then(|c| c.loan_from_club_id)
                {
                    Some(id) => id,
                    None => return,
                }
            } else {
                club_id
            };

            // Step 1: Resolve contract renewal staff, wage budget, and current wage bill
            // Uses the parent club's context for loaned players
            let (negotiation_skill, judging_ability, wage_budget, current_wage_bill) = data.club(contract_club_id)
                .map(|club| {
                    let (neg, judge) = club.teams.teams.first()
                        .map(|team| {
                            let staff = team.staffs.responsibility.contract_renewal.handle_first_team_contracts
                                .and_then(|id| team.staffs.find(id));
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

                    let total_wages: u32 = club.teams.iter()
                        .map(|t| t.get_annual_salary())
                        .sum();

                    (neg, judge, wb, total_wages)
                })
                .unwrap_or((5, 5, 0, 0));

            // Step 2: Read player info (immutable)
            let player = match data.player(result.player_id) {
                Some(p) => p,
                None => return,
            };

            let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
            let ability = player.player_attributes.current_ability;
            let age = DateUtils::age(player.birth_date, data.date.date());
            let base_salary = ability_based_salary(ability);

            // Staff judging_ability affects how accurate the salary offer is
            // Low skill: offer 70-85% of fair value, high skill: offer 95-105%
            let accuracy = 0.70 + (judging_ability as f32 / 20.0) * 0.35;
            let adjusted_base = (base_salary as f32 * accuracy) as u32;

            // Loaned players always get the extension path (parent club renewing remotely)
            let mut offered_salary = if !is_on_loan && result.contract.want_improve_contract {
                // Staff evaluates whether this player deserves a raise
                let ability_f = ability as f32;
                let matches_played = player.statistics.played + player.statistics.played_subs;
                let is_not_needed = player.contract.as_ref()
                    .map(|c| matches!(c.squad_status, PlayerSquadStatus::NotNeeded))
                    .unwrap_or(false);

                // Not needed players don't get raises — transfer list or release instead
                if is_not_needed {
                    Self::handle_unresolved_salary(result.player_id, data, club_id);
                    return;
                }

                // Low ability with few appearances: smaller raise
                let raise_pct = if ability_f < 60.0 && matches_played < 5 {
                    1.05
                } else {
                    1.10 + (ability_f / 200.0) * 0.10 // 10-15% raise
                };

                // Escalate from whichever anchor is higher — player's
                // current wage or our band-based valuation. Escalating only
                // off the band meant players already above their band saw
                // no meaningful raise and their agent rejected every
                // attempt (see contract_renewal fix).
                let anchor = adjusted_base.max(current_salary);
                let raised = (anchor as f32 * raise_pct) as u32;
                raised.max(current_salary + current_salary / 20).max(current_salary + 1)
            } else {
                adjusted_base.max(current_salary + current_salary / 20).max(current_salary + 1)
            };

            // Converge toward the player's own ask when we have it — same
            // split-the-gap heuristic as the proactive path.
            if let Some(ask) = &player.pending_contract_ask {
                if ask.desired_salary > offered_salary {
                    offered_salary = (offered_salary + ask.desired_salary) / 2;
                }
            }

            // Wage budget enforcement: don't offer salary that would bust the budget
            // The new salary replaces the current one, so check the net increase
            let salary_increase = offered_salary.saturating_sub(current_salary);
            let final_salary = if wage_budget > 0 && current_wage_bill + salary_increase > wage_budget {
                // Over budget: cap the offer to what the budget allows
                let remaining = wage_budget.saturating_sub(current_wage_bill);
                let capped_salary = current_salary + remaining;
                // If we can't even match current salary, decide what to do with the player
                // Loaned players can't be transfer-listed remotely
                if capped_salary <= current_salary && result.contract.want_improve_contract && !is_on_loan {
                    Self::handle_unresolved_salary(result.player_id, data, club_id);
                    return;
                }
                capped_salary.max(current_salary)
            } else {
                offered_salary
            };

            let years = negotiate_contract_years(player, age, negotiation_skill);

            // Reactive path stays lean on sweeteners — the player has
            // asked for the renewal themselves, so greed is usually not
            // the blocker. Attach loyalty bonus for veterans to cover the
            // same case where a 30+ player hesitates on length.
            let loyalty_bonus = if age >= 30 {
                (final_salary as f32 * 0.10) as u32
            } else {
                0
            };

            Self::deliver_message(data, club_id, result.player_id, PlayerMessage {
                message_type: PlayerMessageType::ContractProposal(PlayerContractProposal::basic(
                    final_salary,
                    years,
                    negotiation_skill,
                    0,
                    loyalty_bonus,
                    None,
                )),
            });

            // Record the offer so the cooldown + attempt cap at the top
            // of this function can see it next tick. Without this the
            // reactive path would chain proposals every cycle while the
            // player kept signalling want_improve/extend_contract.
            let movement = format!(
                "{}y · ${}/y",
                years,
                crate::utils::FormattingUtils::format_money(final_salary as f64)
            );
            let offer_date = data.date.date();
            if let Some(player_mut) = data.player_mut(result.player_id) {
                player_mut.decision_history.add(
                    offer_date,
                    movement,
                    "dec_contract_renewal_offered".to_string(),
                    String::new(),
                );
            }
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
            player: &Player,
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

    /// When club can't resolve salary unhappiness (rejected proposal, over budget, not needed):
    /// decide whether to transfer list, release on free transfer, or do nothing.
    fn handle_unresolved_salary(player_id: u32, data: &mut SimulatorData, club_id: u32) {
        let date = data.date.date();

        // Gather decision info from immutable access
        let decision = {
            let squad_avg = data.club(club_id)
                .and_then(|club| club.teams.teams.first())
                .map(|team| team.players.current_ability_avg() as i16)
                .unwrap_or(0);

            let player = match data.player(player_id) {
                Some(p) => p,
                None => return,
            };

            // Already listed — don't re-process
            let statuses = player.statuses.get();
            if statuses.contains(&PlayerStatusType::Lst) || statuses.contains(&PlayerStatusType::Frt) {
                return;
            }

            let ability = player.player_attributes.current_ability as i16;
            let age = DateUtils::age(player.birth_date, date);
            let loyalty = player.attributes.loyalty;
            let is_key = player.contract.as_ref()
                .map(|c| matches!(c.squad_status,
                    PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular))
                .unwrap_or(false);

            // Key players and first-team regulars: club keeps trying — don't list them
            // unless they're well below squad average
            if is_key && ability >= squad_avg - 10 {
                return;
            }

            // Loyal players get more patience
            if loyalty > 14.0 && ability >= squad_avg - 10 {
                return;
            }

            // Competitive players (within 15 CA of squad avg): keep trying regardless of age
            // A 32-year-old with ability near the squad average is still valuable
            if ability >= squad_avg - 15 && age < 35 {
                return;
            }

            // Low ability: release on free transfer
            // Only release for age if truly past it (35+) or far below average
            if ability < squad_avg - 25 || (age >= 35 && ability < squad_avg - 10) {
                UnresolvedSalaryDecision::FreeTransfer
            } else {
                UnresolvedSalaryDecision::TransferList
            }
        };

        // Apply decision with mutable access through club
        let club = match data.club_mut(club_id) {
            Some(c) => c,
            None => return,
        };

        for team in &mut club.teams.teams {
            if let Some(player) = team.players.players.iter_mut().find(|p| p.id == player_id) {
                match decision {
                    UnresolvedSalaryDecision::FreeTransfer => {
                        player.statuses.add(date, PlayerStatusType::Frt);
                        player.contract = None;
                    }
                    UnresolvedSalaryDecision::TransferList => {
                        player.statuses.add(date, PlayerStatusType::Lst);
                        if let Some(ref mut contract) = player.contract {
                            contract.is_transfer_listed = true;
                        }
                    }
                }
                break;
            }
        }
    }

    fn deliver_message(data: &mut SimulatorData, club_id: u32, player_id: u32, message: PlayerMessage) {
        let club = match data.club_mut(club_id) {
            Some(c) => c,
            None => return,
        };

        for team in &mut club.teams.teams {
            if let Some(player) = team.players.players.iter_mut().find(|p| p.id == player_id) {
                player.mailbox.push(message);
                return;
            }
        }
    }
}
