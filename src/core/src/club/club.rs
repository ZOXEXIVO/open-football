use crate::club::academy::ClubAcademy;
use crate::club::board::{BoardContext, ClubBoard};
use crate::club::status::ClubStatus;
use crate::club::{ClubFinances, ClubResult};
use crate::context::GlobalContext;
use crate::shared::{Currency, CurrencyValue, Location};
use crate::transfers::{CompletedTransfer, TransferType};
use crate::transfers::pipeline::{ClubTransferPlan, LoanOutCandidate, LoanOutReason, LoanOutStatus};
use crate::{ContractType, Person, PlayerClubContract, PlayerStatusType, ReputationLevel, TeamCollection, TeamType, TransferItem};
use chrono::{Datelike, NaiveDate};
use log::debug;

#[derive(Debug, Clone, PartialEq)]
pub enum ClubPhilosophy {
    /// Develop youth and sell for profit (Ajax, Benfica, Dortmund)
    DevelopAndSell,
    /// Sign established players, compete now (PSG, Chelsea, Man City)
    SignToCompete,
    /// Loan-heavy strategy, minimal spending (smaller clubs)
    LoanFocused,
    /// Balanced approach (most clubs)
    Balanced,
}

#[derive(Debug, Clone)]
pub struct ClubColors {
    pub background: String,
    pub foreground: String,
}

impl Default for ClubColors {
    fn default() -> Self {
        ClubColors {
            background: "#1e272d".to_string(),
            foreground: "#ffffff".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Club {
    pub id: u32,
    pub name: String,

    pub location: Location,

    pub board: ClubBoard,

    pub finance: ClubFinances,

    pub status: ClubStatus,

    pub academy: ClubAcademy,

    pub colors: ClubColors,

    pub teams: TeamCollection,

    pub transfer_plan: ClubTransferPlan,

    pub philosophy: ClubPhilosophy,
}

impl Club {
    pub fn new(
        id: u32,
        name: String,
        location: Location,
        finance: ClubFinances,
        academy: ClubAcademy,
        status: ClubStatus,
        colors: ClubColors,
        teams: TeamCollection,
    ) -> Self {
        let philosophy = Self::determine_philosophy(&teams);

        Club {
            id,
            name,
            location,
            finance,
            status,
            academy,
            colors,
            board: ClubBoard::new(),
            teams,
            transfer_plan: ClubTransferPlan::new(),
            philosophy,
        }
    }

    fn determine_philosophy(teams: &TeamCollection) -> ClubPhilosophy {
        let rep_level = teams.teams.iter()
            .find(|t| t.team_type == TeamType::Main)
            .map(|t| t.reputation.level())
            .unwrap_or(ReputationLevel::Amateur);

        match rep_level {
            ReputationLevel::Elite => ClubPhilosophy::SignToCompete,
            ReputationLevel::Continental => ClubPhilosophy::Balanced,
            ReputationLevel::National => ClubPhilosophy::Balanced,
            _ => ClubPhilosophy::LoanFocused,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ClubResult {
        let date = ctx.simulation.date.date();

        let board_ctx = self.build_board_context();

        let mut result = ClubResult::new(
            self.id,
            self.finance.simulate(ctx.with_finance()),
            self.teams.simulate(ctx.with_club(self.id, &self.name)),
            self.board.simulate(ctx.with_board_data(board_ctx)),
            self.academy.simulate(ctx.clone()),
        );

        if ctx.simulation.is_week_beginning() {
            self.teams.ensure_coach_state(date);
            self.teams.update_all_impressions(date);

            // Weekly: move loan returnees from main to reserve
            self.move_loan_returns_to_reserve(date);

            // Weekly: enforce youth team age limits and fill main squad
            self.enforce_youth_team_age_limits(date);
            self.promote_youth_to_main_if_needed(date);
        } else {
            self.teams.manage_critical_squad_moves(date);
        }

        if ctx.simulation.is_month_beginning() {
            self.teams.ensure_coach_state(date);
            for req in self.teams.prepare_ai_requests(date, self.id) {
                ctx.ai.push(req);
            }

            // Monthly: process wages (annual salary / 12) and income
            self.process_monthly_finances(ctx.clone());

            // Monthly: audit squad utilization and list underused players
            self.audit_squad_utilization(date);
        }

        // Academy graduations at season start
        let season = ctx.country.as_ref().map(|c| c.season_dates).unwrap_or_default();
        if ctx.simulation.is_season_start(&season) {
            result.academy_transfers = self.process_academy_graduations(date);
        }

        result
    }

    /// Graduate best academy players to U18 team (5-10 per year).
    /// Move overage youth players to main team.
    /// Aged-out academy players disappear.
    /// Returns completed transfer records for graduated players.
    fn process_academy_graduations(&mut self, date: NaiveDate) -> Vec<CompletedTransfer> {
        let mut transfers = Vec::new();

        // Release aged-out academy players first
        let released = self.academy.release_aged_out(date);
        if released > 0 {
            debug!("academy {}: {} aged-out players released", self.name, released);
        }

        // Find U18 team index
        let u18_idx = self.teams.teams.iter().position(|t| t.team_type == TeamType::U18);

        // Graduate best academy players — sign youth contract with main team, stay in U18
        if let Some(idx) = u18_idx {
            let u18_count = self.teams.teams[idx].players.players.len();
            let target = 20usize;
            let space = target.saturating_sub(u18_count);
            let to_graduate = space.max(5).min(10);

            // Main team name for contract registration
            let main_team_name = self.teams.teams.iter()
                .find(|t| t.team_type == TeamType::Main)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| self.name.clone());

            let graduated = self.academy.graduate_to_u18(date, to_graduate);
            if !graduated.is_empty() {
                debug!("academy {}: {} players graduated (contract: {}, assigned: U18, was {})",
                    self.name, graduated.len(), main_team_name, u18_count);
                for player in graduated {
                    transfers.push(CompletedTransfer::new(
                        player.id,
                        player.full_name.to_string(),
                        0,
                        0,
                        "Academy".to_string(),
                        self.id,
                        main_team_name.clone(),
                        date,
                        CurrencyValue::new(0.0, Currency::Usd),
                        TransferType::Free,
                    ).with_reason("Academy graduation — youth contract signed".to_string()));
                    // Player physically stays in U18 team
                    self.teams.teams[idx].players.add(player);
                }
            }
        }

        // Move overage players from youth teams to main team
        self.enforce_youth_team_age_limits(date);

        // Fill main team if still short
        self.promote_youth_to_main_if_needed(date);

        transfers
    }

    /// Move players who exceed their youth team's max age to the main team.
    fn enforce_youth_team_age_limits(&mut self, date: NaiveDate) {
        let main_idx = match self.teams.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return,
        };

        // Collect overage players from all youth teams
        let mut overage: Vec<(usize, u32)> = Vec::new(); // (team_idx, player_id)

        for (ti, team) in self.teams.teams.iter().enumerate() {
            if ti == main_idx {
                continue;
            }
            if let Some(max_age) = team.team_type.max_age() {
                for p in &team.players.players {
                    if p.age(date) > max_age {
                        overage.push((ti, p.id));
                    }
                }
            }
        }

        for (team_idx, player_id) in overage {
            if let Some(mut player) = self.teams.teams[team_idx].players.take_player(&player_id) {
                // Give full contract if on youth contract
                if player.contract.as_ref().map(|c| c.contract_type == crate::ContractType::Youth).unwrap_or(false) {
                    let expiration = NaiveDate::from_ymd_opt(
                        date.year() + 3, date.month(), date.day().min(28),
                    ).unwrap_or(date);
                    let salary = graduation_salary(player.player_attributes.current_ability);
                    player.contract = Some(PlayerClubContract::new(salary, expiration));
                }

                debug!("overage promotion: {} (age {}) from {} -> main team",
                    player.full_name, player.age(date), self.teams.teams[team_idx].name);
                self.teams.teams[main_idx].players.add(player);
            }
        }
    }

    /// Move players without a contract (loan returnees) from main team to reserve.
    /// Loan returns land on teams[0] (main) — staff then moves them to reserve for assessment.
    fn move_loan_returns_to_reserve(&mut self, _date: NaiveDate) {
        let main_idx = match self.teams.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return,
        };

        let reserve_idx = self.teams.teams.iter()
            .position(|t| t.team_type == TeamType::Reserve)
            .or_else(|| self.teams.teams.iter().position(|t| t.team_type == TeamType::B));

        let reserve_idx = match reserve_idx {
            Some(idx) => idx,
            None => return, // no reserve team, stay on main
        };

        // Find main team players with no contract (returned from loan)
        let to_move: Vec<u32> = self.teams.teams[main_idx].players.players.iter()
            .filter(|p| p.contract.is_none())
            .map(|p| p.id)
            .collect();

        for player_id in to_move {
            if let Some(player) = self.teams.teams[main_idx].players.take_player(&player_id) {
                debug!("loan return -> reserve: {} moved to {}",
                    player.full_name, self.teams.teams[reserve_idx].name);
                self.teams.teams[reserve_idx].players.add(player);
            }
        }
    }

    /// If main team has fewer than 22 players, promote best youth players up.
    fn promote_youth_to_main_if_needed(&mut self, date: NaiveDate) {
        let main_idx = match self.teams.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return,
        };

        let main_count = self.teams.teams[main_idx].players.players.len();
        let min_squad = 22usize;

        if main_count >= min_squad {
            return;
        }

        let deficit = min_squad - main_count;

        // Collect candidates from youth teams (U18, U19, U20, U21, U23, B, Reserve)
        let youth_team_indices: Vec<usize> = self.teams.teams.iter()
            .enumerate()
            .filter(|(i, t)| *i != main_idx && t.team_type != TeamType::Main)
            .map(|(i, _)| i)
            .collect();

        // Gather all youth candidates with their team index and ability
        let mut candidates: Vec<(usize, u32, u8, u8)> = Vec::new(); // (team_idx, player_id, ability, age)
        for &ti in &youth_team_indices {
            for p in &self.teams.teams[ti].players.players {
                candidates.push((ti, p.id, p.player_attributes.current_ability, p.age(date)));
            }
        }

        // Sort by ability descending
        candidates.sort_by(|a, b| b.2.cmp(&a.2));
        candidates.truncate(deficit);

        let mut promoted = 0;
        for (team_idx, player_id, _ca, _age) in candidates {
            if let Some(mut player) = self.teams.teams[team_idx].players.take_player(&player_id) {
                // Upgrade to full contract if on youth contract
                if player.contract.as_ref().map(|c| c.contract_type == crate::ContractType::Youth).unwrap_or(false) {
                    let expiration = NaiveDate::from_ymd_opt(
                        date.year() + 3, date.month(), date.day().min(28),
                    ).unwrap_or(date);
                    let salary = graduation_salary(player.player_attributes.current_ability);
                    player.contract = Some(PlayerClubContract::new(salary, expiration));
                }

                debug!("promote to main: {} (CA={}, age={}) from {}",
                    player.full_name, player.player_attributes.current_ability,
                    player.age(date), self.teams.teams[team_idx].name);
                self.teams.teams[main_idx].players.add(player);
                promoted += 1;
            }
        }

        if promoted > 0 {
            debug!("{}: promoted {} youth players to main team (now {})",
                self.name, promoted, self.teams.teams[main_idx].players.players.len());
        }
    }

    fn build_board_context(&self) -> BoardContext {
        let main_team = self.teams.teams.iter().find(|t| t.team_type == TeamType::Main);

        let main_squad_size = main_team.map(|t| t.players.players.len()).unwrap_or(0);

        let reserve_squad_size: usize = self.teams.teams.iter()
            .filter(|t| t.team_type != TeamType::Main)
            .map(|t| t.players.players.len())
            .sum();

        let total_annual_wages: u32 = self.teams.teams.iter()
            .map(|t| t.get_annual_salary())
            .sum();

        let reputation_score = main_team
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.0);

        BoardContext {
            balance: self.finance.balance.balance,
            total_annual_wages,
            reputation_score,
            main_squad_size,
            reserve_squad_size,
        }
    }

    /// Monthly audit: identify underutilized players in non-main teams and list them for loan/transfer.
    fn audit_squad_utilization(&mut self, date: NaiveDate) {
        let main_idx = match self.teams.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return,
        };

        // Collect underutilized player decisions: (team_idx, player_id, loan_or_transfer)
        let mut loan_players: Vec<(usize, u32)> = Vec::new();
        let mut transfer_players: Vec<(usize, u32)> = Vec::new();

        for (ti, team) in self.teams.teams.iter().enumerate() {
            if ti == main_idx {
                continue;
            }

            for player in &team.players.players {
                // Skip youth contracts
                if player.contract.as_ref()
                    .map(|c| c.contract_type == ContractType::Youth)
                    .unwrap_or(false)
                {
                    continue;
                }

                // Skip loan players
                if player.is_on_loan() {
                    continue;
                }

                // Skip already listed/loaned
                let statuses = player.statuses.get();
                if statuses.contains(&PlayerStatusType::Lst) || statuses.contains(&PlayerStatusType::Loa) {
                    continue;
                }

                // Underutilized: no matches in 60+ days AND fewer than 3 total games
                let days_idle = player.player_attributes.days_since_last_match;
                let total_games = player.statistics.total_games();

                if days_idle < 60 || total_games >= 3 {
                    continue;
                }

                let age = player.age(date);
                let ca = player.player_attributes.current_ability;
                let pa = player.player_attributes.potential_ability;

                // Decision: loan out young talent, transfer list older/low-ability players
                if age <= 23 && pa > ca.saturating_add(5) {
                    loan_players.push((ti, player.id));
                } else if age >= 28 || ca < 60 {
                    transfer_players.push((ti, player.id));
                } else {
                    loan_players.push((ti, player.id));
                }
            }
        }

        self.process_underutilized_players(date, main_idx, &loan_players, &transfer_players);
    }

    fn process_underutilized_players(
        &mut self,
        date: NaiveDate,
        main_idx: usize,
        loan_players: &[(usize, u32)],
        transfer_players: &[(usize, u32)],
    ) {
        // Reputation-based loan fee multiplier
        let rep_multiplier = match self.teams.teams[main_idx].reputation.level() {
            crate::ReputationLevel::Elite => 0.15,
            crate::ReputationLevel::Continental => 0.10,
            crate::ReputationLevel::National => 0.05,
            crate::ReputationLevel::Regional => 0.02,
            _ => 0.0, // Local/Amateur: free loan
        };

        // Process loan recommendations
        for &(team_idx, player_id) in loan_players {
            let team_name = self.teams.teams[team_idx].name.clone();

            let loan_fee = if rep_multiplier > 0.0 {
                let player_value = self.teams.teams[team_idx].players.players.iter()
                    .find(|p| p.id == player_id)
                    .map(|p| p.value(date))
                    .unwrap_or(0.0);
                crate::utils::FormattingUtils::round_fee(player_value * rep_multiplier)
            } else {
                0.0
            };

            let player = match self.teams.teams[team_idx].players.players.iter_mut()
                .find(|p| p.id == player_id)
            {
                Some(p) => p,
                None => continue,
            };

            player.statuses.add(date, PlayerStatusType::Loa);
            player.decision_history.add(
                date,
                "Board loan-listed".to_string(),
                "Underutilized in reserve squad".to_string(),
                "Board".to_string(),
            );

            debug!("Board loan-listed: {} (age {}, CA={}) from {}, loan fee: {}",
                player.full_name, player.age(date),
                player.player_attributes.current_ability,
                team_name, loan_fee);

            self.transfer_plan.loan_out_candidates.push(LoanOutCandidate {
                player_id,
                reason: LoanOutReason::LackOfPlayingTime,
                status: LoanOutStatus::Listed,
                loan_fee,
            });
        }

        // Process transfer recommendations
        for &(team_idx, player_id) in transfer_players {
            let team_name = self.teams.teams[team_idx].name.clone();

            let asking_price = {
                let player = match self.teams.teams[team_idx].players.players.iter()
                    .find(|p| p.id == player_id)
                {
                    Some(p) => p,
                    None => continue,
                };
                player.value(date) * 0.5
            };

            let player = match self.teams.teams[team_idx].players.players.iter_mut()
                .find(|p| p.id == player_id)
            {
                Some(p) => p,
                None => continue,
            };

            player.statuses.add(date, PlayerStatusType::Lst);
            player.decision_history.add(
                date,
                "Board transfer-listed".to_string(),
                "Underutilized, surplus to requirements".to_string(),
                "Board".to_string(),
            );

            debug!("Board transfer-listed: {} (age {}, CA={}) from {}, asking {}",
                player.full_name, player.age(date),
                player.player_attributes.current_ability,
                team_name,
                asking_price);

            self.teams.teams[main_idx].transfer_list.add(TransferItem::new(
                player_id,
                CurrencyValue::new(asking_price, Currency::Usd),
            ));
        }
    }

    fn process_monthly_finances(&mut self, ctx: GlobalContext<'_>) {
        let club_name = ctx.club.as_ref().expect("no club found").name;
        let date = ctx.simulation.date.date();

        // Monthly wage deduction: annual salary / 12
        for team in &self.teams.teams {
            let annual_salary = team.get_annual_salary();
            let monthly_salary = annual_salary / 12;
            self.finance.push_salary(club_name, monthly_salary as i32);
        }

        // Monthly sponsorship income
        let sponsorship_income: i32 = self.finance.sponsorship
            .get_sponsorship_incomes(date)
            .iter()
            .map(|c| c.wage / 12)
            .sum();

        if sponsorship_income > 0 {
            self.finance.balance.push_income(sponsorship_income);
        }

        // Monthly reputation-based revenue (TV deals, matchday, merchandise)
        let main_team = self.teams.teams.iter().find(|t| t.team_type == TeamType::Main);
        if let Some(team) = main_team {
            let monthly_revenue = match team.reputation.level() {
                crate::ReputationLevel::Elite => 2_500_000,
                crate::ReputationLevel::Continental => 1_000_000,
                crate::ReputationLevel::National => 400_000,
                crate::ReputationLevel::Regional => 100_000,
                crate::ReputationLevel::Local => 30_000,
                crate::ReputationLevel::Amateur => 10_000,
            };
            self.finance.balance.push_income(monthly_revenue);
        }
    }
}

fn graduation_salary(current_ability: u8) -> u32 {
    match current_ability {
        0..=60 => 15_000,
        61..=80 => 30_000,
        81..=100 => 60_000,
        101..=120 => 120_000,
        121..=150 => 250_000,
        _ => 500_000,
    }
}
