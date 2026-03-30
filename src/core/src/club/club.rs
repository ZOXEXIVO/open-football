use crate::club::academy::ClubAcademy;
use crate::club::board::{BoardContext, ClubBoard};
use crate::club::facilities::ClubFacilities;
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

    pub facilities: ClubFacilities,

    pub rivals: Vec<u32>,
}

impl Club {
    pub fn is_rival(&self, other_club_id: u32) -> bool {
        self.rivals.contains(&other_club_id)
    }

    pub fn new(
        id: u32,
        name: String,
        location: Location,
        finance: ClubFinances,
        academy: ClubAcademy,
        status: ClubStatus,
        colors: ClubColors,
        teams: TeamCollection,
        facilities: ClubFacilities,
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
            facilities,
            rivals: Vec::new(),
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

        // Build club context with facility data for training/academy
        let club_ctx = ctx.with_club(self.id, &self.name);
        let club_ctx = {
            let mut c = club_ctx;
            if let Some(ref mut cc) = c.club {
                *cc = cc.clone().with_facilities(
                    self.facilities.training.multiplier(),
                    self.facilities.youth.multiplier(),
                    self.facilities.academy.multiplier(),
                    self.facilities.recruitment.multiplier(),
                );
            }
            c
        };

        let mut result = ClubResult::new(
            self.id,
            self.finance.simulate(ctx.with_finance()),
            self.teams.simulate(club_ctx.clone()),
            self.board.simulate(ctx.with_board_data(board_ctx)),
            self.academy.simulate(club_ctx.clone()),
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

        // Season start: reset player states and graduate academy players
        let season = ctx.country.as_ref().map(|c| c.season_dates).unwrap_or_default();
        if ctx.simulation.is_season_start(&season) {
            // Sync budgets from board targets to finance system
            if let Some(targets) = &self.board.season_targets {
                self.finance.transfer_budget = Some(crate::shared::CurrencyValue {
                    amount: targets.transfer_budget as f64,
                    currency: crate::shared::Currency::Usd,
                });
                self.finance.wage_budget = Some(crate::shared::CurrencyValue {
                    amount: targets.wage_budget as f64,
                    currency: crate::shared::Currency::Usd,
                });
            }

            self.process_pre_season_reset();
            result.academy_transfers = self.process_academy_graduations(date);
            self.trim_positional_surplus(date);
        }

        result
    }

    /// Pre-season reset: restore player conditions and clear lingering statuses.
    /// Called once at season start so teams begin with full healthy squads.
    fn process_pre_season_reset(&mut self) {
        for team in &mut self.teams.teams {
            for player in &mut team.players.players {
                // Restore condition to pre-season fitness level (85%)
                if player.player_attributes.condition < 8500 && !player.player_attributes.is_injured {
                    player.player_attributes.condition = 8500;
                }

                // Clear stale Int status (should have been released by national team,
                // but safety net in case tournament release was missed)
                player.statuses.remove(PlayerStatusType::Int);

                // Reset ban flags for new season
                player.player_attributes.is_banned = false;

                // NOTE: Do NOT reset player.statistics here!
                // The season-end snapshot (snapshot_player_season_statistics) takes
                // stats via std::mem::take in on_season_end. If we reset here first,
                // the snapshot captures zeroed stats and the season's history is lost.

                // Reset days since last match (pre-season training counts)
                player.player_attributes.days_since_last_match = 7;
            }
        }
    }

    /// Graduate best academy players to U18 team (5-10 per year).
    /// Move overage youth players to main team.
    /// Aged-out academy players disappear.
    /// Returns completed transfer records for graduated players.
    fn process_academy_graduations(&mut self, date: NaiveDate) -> Vec<CompletedTransfer> {
        let mut transfers = Vec::new();

        // Find the lowest youth team to graduate into (U18 → U19 → U20 → U21 → U23)
        let youth_idx = TeamType::YOUTH_PROGRESSION.iter()
            .find_map(|tt| self.teams.teams.iter().position(|t| t.team_type == *tt));

        // Graduate best academy players BEFORE releasing aged-out ones,
        // so 16+ year olds get a chance to graduate instead of being deleted
        if let Some(idx) = youth_idx {
            let youth_count = self.teams.teams[idx].players.players.len();
            let target = 20usize;
            let space = target.saturating_sub(youth_count);
            let to_graduate = space.max(5).min(10);

            // Main team name for contract registration
            let main_team_name = self.teams.teams.iter()
                .find(|t| t.team_type == TeamType::Main)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| self.name.clone());

            let youth_team_type = self.teams.teams[idx].team_type;
            let graduated = self.academy.graduate_to_youth(date, to_graduate);
            if !graduated.is_empty() {
                debug!("academy {}: {} players graduated (contract: {}, assigned: {:?}, was {})",
                    self.name, graduated.len(), main_team_name, youth_team_type, youth_count);
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
                    self.teams.teams[idx].players.add(player);
                }
            }
        }

        // Release aged-out academy players (16+) that were NOT graduated
        let released = self.academy.release_aged_out(date);
        if released > 0 {
            debug!("academy {}: {} aged-out players released", self.name, released);
        }

        // Move overage players from youth teams to main team
        self.enforce_youth_team_age_limits(date);

        // Fill main team if still short
        self.promote_youth_to_main_if_needed(date);

        transfers
    }

    /// Move players who exceed their youth team's max age to the next youth team,
    /// or to the main team if no eligible youth team exists.
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
            let player_age = self.teams.teams[team_idx].players.players
                .iter()
                .find(|p| p.id == player_id)
                .map(|p| p.age(date))
                .unwrap_or(99);

            let current_team_type = self.teams.teams[team_idx].team_type;

            // Find the next youth team in progression that can accept this player
            let next_youth_idx = self.find_next_youth_team(current_team_type, player_age);

            let target_idx = next_youth_idx.unwrap_or(main_idx);

            if let Some(mut player) = self.teams.teams[team_idx].players.take_player(&player_id) {
                // Give full contract only when promoting to main team
                if target_idx == main_idx {
                    if player.contract.as_ref().map(|c| c.contract_type == crate::ContractType::Youth).unwrap_or(false) {
                        let expiration = NaiveDate::from_ymd_opt(
                            date.year() + 3, date.month(), date.day().min(28),
                        ).unwrap_or(date);
                        let club_rep = self.teams.teams[main_idx].reputation.world;
                        let salary = graduation_salary(player.player_attributes.current_ability, club_rep);
                        player.contract = Some(PlayerClubContract::new(salary, expiration));
                    }
                }

                debug!("overage promotion: {} (age {}) from {} -> {}",
                    player.full_name, player.age(date),
                    self.teams.teams[team_idx].name, self.teams.teams[target_idx].name);
                self.teams.teams[target_idx].players.add(player);
            }
        }
    }

    /// Find the next youth team in progression (U18→U19→U20→U21→U23)
    /// that exists in this club and can accept a player of the given age.
    fn find_next_youth_team(&self, current_type: TeamType, player_age: u8) -> Option<usize> {
        let progression = TeamType::YOUTH_PROGRESSION;

        // Find current position in progression
        let current_pos = progression.iter().position(|t| *t == current_type)?;

        // Look through subsequent youth team types
        for next_type in &progression[current_pos + 1..] {
            if let Some(max_age) = next_type.max_age() {
                if player_age <= max_age {
                    // Check if this club has this team type
                    if let Some(idx) = self.teams.teams.iter().position(|t| t.team_type == *next_type) {
                        return Some(idx);
                    }
                }
            }
        }

        None
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

    /// Release excess players at over-represented positions across all teams.
    ///
    /// Real clubs maintain positional balance: 3 GKs in the first team, ~6-8
    /// defenders, ~6-8 midfielders, ~4-6 forwards. When academy graduations or
    /// failed transfers cause bloat (e.g. 13 GKs), the worst surplus players
    /// are released to free agents.
    fn trim_positional_surplus(&mut self, date: NaiveDate) {
        use crate::PlayerFieldPositionGroup;

        // Positional limits across ALL teams combined
        // (GK: max 4, DEF: max 20, MID: max 20, FWD: max 16)
        let limits: [(PlayerFieldPositionGroup, usize); 4] = [
            (PlayerFieldPositionGroup::Goalkeeper, 4),
            (PlayerFieldPositionGroup::Defender, 20),
            (PlayerFieldPositionGroup::Midfielder, 20),
            (PlayerFieldPositionGroup::Forward, 16),
        ];

        for (group, max_count) in &limits {
            // Collect all players at this position across all teams
            let mut players_at_pos: Vec<(usize, u32, u8)> = Vec::new(); // (team_idx, player_id, ability)
            for (ti, team) in self.teams.teams.iter().enumerate() {
                for p in &team.players.players {
                    if p.position().position_group() == *group {
                        players_at_pos.push((ti, p.id, p.player_attributes.current_ability));
                    }
                }
            }

            if players_at_pos.len() <= *max_count {
                continue;
            }

            // Sort by ability ascending — release the worst first
            players_at_pos.sort_by_key(|&(_, _, ca)| ca);

            let to_release = players_at_pos.len() - max_count;
            for &(team_idx, player_id, _) in players_at_pos.iter().take(to_release) {
                if let Some(player) = self.teams.teams[team_idx].players.take_player(&player_id) {
                    log::debug!(
                        "positional surplus release: {} ({:?}, CA={}) from {}",
                        player.full_name, group, player.player_attributes.current_ability,
                        self.teams.teams[team_idx].name
                    );
                    // Player is simply removed — becomes a free agent
                    drop(player);
                }
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
        // Skip youth teams that would drop below minimum viable squad (11 players)
        let min_youth_squad = 11usize;
        let youth_team_indices: Vec<usize> = self.teams.teams.iter()
            .enumerate()
            .filter(|(i, t)| *i != main_idx && t.team_type != TeamType::Main)
            .map(|(i, _)| i)
            .collect();

        // Gather youth candidates, but protect teams from going below minimum
        let mut candidates: Vec<(usize, u32, u8, u8)> = Vec::new(); // (team_idx, player_id, ability, age)
        for &ti in &youth_team_indices {
            let team_size = self.teams.teams[ti].players.players.len();
            // Only take from teams that have players to spare
            if team_size <= min_youth_squad && self.teams.teams[ti].team_type.max_age().is_some() {
                continue;
            }
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
                    let club_rep = self.teams.teams[main_idx].reputation.world;
                    let salary = graduation_salary(player.player_attributes.current_ability, club_rep);
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

        let rep_level = self.teams.teams[main_idx].reputation.level();

        // Wealthy clubs are more patient with underutilized players
        let (idle_threshold, games_threshold) = match rep_level {
            ReputationLevel::Elite => (120u16, 5u16),
            ReputationLevel::Continental => (90, 4),
            ReputationLevel::National => (60, 3),
            ReputationLevel::Regional => (45, 2),
            _ => (30, 1),
        };

        // Wealthy clubs within squad targets don't need to aggressively list
        let total_squad: usize = self.teams.teams.iter()
            .map(|t| t.players.players.len()).sum();
        let max_squad = self.board.season_targets
            .as_ref()
            .map(|t| t.max_squad_size as usize)
            .unwrap_or(50);
        let wealthy_within_limits = matches!(rep_level, ReputationLevel::Elite | ReputationLevel::Continental)
            && total_squad < max_squad;

        // Collect underutilized player decisions
        let mut loan_players: Vec<(usize, u32, String)> = Vec::new();
        let mut transfer_players: Vec<(usize, u32, String)> = Vec::new();

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

                let days_idle = player.player_attributes.days_since_last_match;
                let total_games = player.statistics.total_games();

                // Reputation-scaled underutilization threshold
                if days_idle < idle_threshold || total_games >= games_threshold {
                    continue;
                }

                let age = player.age(date);
                let ca = player.player_attributes.current_ability;
                let pa = player.player_attributes.potential_ability;

                // Wealthy clubs within squad limits: only list truly unwanted players
                if wealthy_within_limits && ca >= 50 && age < 32 {
                    continue;
                }

                // Decision: choose Lst vs Loa based on player profile and club context
                if age <= 23 && pa > ca.saturating_add(5) {
                    loan_players.push((ti, player.id, "dec_reason_young_develop".to_string()));
                } else if age >= 30 || (ca < 60 && pa < 70) {
                    let reason = if age >= 30 {
                        "dec_reason_aging_surplus"
                    } else {
                        "dec_reason_low_ability_surplus"
                    };
                    transfer_players.push((ti, player.id, reason.to_string()));
                } else if matches!(rep_level, ReputationLevel::Elite | ReputationLevel::Continental) && age <= 29 {
                    loan_players.push((ti, player.id, "dec_reason_underutilized_top_club".to_string()));
                } else {
                    transfer_players.push((ti, player.id, "dec_reason_underutilized".to_string()));
                }
            }
        }

        self.process_underutilized_players(date, main_idx, &loan_players, &transfer_players);
    }

    fn process_underutilized_players(
        &mut self,
        date: NaiveDate,
        main_idx: usize,
        loan_players: &[(usize, u32, String)],
        transfer_players: &[(usize, u32, String)],
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
        for (team_idx, player_id, reason) in loan_players {
            let team_idx = *team_idx;
            let player_id = *player_id;
            let team_name = self.teams.teams[team_idx].name.clone();

            let loan_fee = if rep_multiplier > 0.0 {
                let player_value = self.teams.teams[team_idx].players.players.iter()
                    .find(|p| p.id == player_id)
                    .map(|p| p.value(date, 0, 0))
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
                "dec_board_loan_listed".to_string(),
                reason.clone(),
                "dec_decided_board".to_string(),
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
        for (team_idx, player_id, reason) in transfer_players {
            let team_idx = *team_idx;
            let player_id = *player_id;
            let team_name = self.teams.teams[team_idx].name.clone();

            let asking_price = {
                let player = match self.teams.teams[team_idx].players.players.iter()
                    .find(|p| p.id == player_id)
                {
                    Some(p) => p,
                    None => continue,
                };
                player.value(date, 0, 0) * 0.5
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
                "dec_board_transfer_listed".to_string(),
                reason.clone(),
                "dec_decided_board".to_string(),
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

        // Economic factors from country
        let tv_multiplier = ctx.country.as_ref()
            .map(|c| c.tv_revenue_multiplier)
            .unwrap_or(1.0);
        let attendance_factor = ctx.country.as_ref()
            .map(|c| c.stadium_attendance_factor)
            .unwrap_or(1.0);
        let sponsorship_strength = ctx.country.as_ref()
            .map(|c| c.sponsorship_market_strength)
            .unwrap_or(1.0);

        // 1. Player wages: annual salary / 12
        for team in &self.teams.teams {
            let annual_salary = team.get_annual_salary();
            let monthly_salary = annual_salary / 12;
            self.finance.push_salary(club_name, monthly_salary as i64);
        }

        // 2. Staff wages: coaching, medical, scouting staff
        for team in &self.teams.teams {
            let staff_annual: u32 = team.staffs.staffs.iter()
                .filter_map(|s| s.contract.as_ref())
                .map(|c| c.salary)
                .sum();
            let staff_monthly = staff_annual / 12;
            if staff_monthly > 0 {
                self.finance.balance.push_expense_staff_wages(staff_monthly as i64);
            }
        }

        // 3. Sponsorship income
        let sponsorship_income: i64 = self.finance.sponsorship
            .get_sponsorship_incomes(date)
            .iter()
            .map(|c| (c.wage / 12) as i64)
            .sum();
        if sponsorship_income > 0 {
            self.finance.balance.push_income_sponsorship(sponsorship_income);
        }

        // 4. TV, matchday, merchandising, facility costs — from main team reputation
        let main_team = self.teams.teams.iter().find(|t| t.team_type == TeamType::Main);
        if let Some(team) = main_team {
            // TV revenue (reputation-based, scaled by country TV multiplier)
            let tv_base: i64 = match team.reputation.level() {
                crate::ReputationLevel::Elite => 2_000_000,
                crate::ReputationLevel::Continental => 800_000,
                crate::ReputationLevel::National => 300_000,
                crate::ReputationLevel::Regional => 70_000,
                crate::ReputationLevel::Local => 20_000,
                crate::ReputationLevel::Amateur => 5_000,
            };
            let tv_revenue = (tv_base as f64 * tv_multiplier as f64) as i64;
            self.finance.balance.push_income_tv(tv_revenue);

            // Matchday revenue (dynamic attendance)
            let base_attendance = self.facilities.average_attendance as f64;
            let dynamic_attendance = (base_attendance * attendance_factor as f64) as i64;
            let ticket_price: i64 = match team.reputation.level() {
                crate::ReputationLevel::Elite => 55,
                crate::ReputationLevel::Continental => 40,
                crate::ReputationLevel::National => 28,
                crate::ReputationLevel::Regional => 15,
                crate::ReputationLevel::Local => 8,
                crate::ReputationLevel::Amateur => 4,
            };
            let matchday_revenue = dynamic_attendance * ticket_price * 2;
            self.finance.balance.push_income_matchday(matchday_revenue);

            // Merchandising (reputation-based, scaled by sponsorship market)
            let merch_base: i64 = match team.reputation.level() {
                crate::ReputationLevel::Elite => 500_000,
                crate::ReputationLevel::Continental => 150_000,
                crate::ReputationLevel::National => 50_000,
                crate::ReputationLevel::Regional => 10_000,
                crate::ReputationLevel::Local => 2_000,
                crate::ReputationLevel::Amateur => 500,
            };
            let merch_revenue = (merch_base as f64 * sponsorship_strength as f64) as i64;
            self.finance.balance.push_income_merchandising(merch_revenue);
        }

        // 5. Facility maintenance costs
        let facility_cost: i64 = (
            self.facilities.training.to_rating() as i64 +
            self.facilities.youth.to_rating() as i64 +
            self.facilities.academy.to_rating() as i64
        ) * 5_000;
        self.finance.balance.push_expense_facilities(facility_cost);
    }
}

/// Graduation salary: ability sets the tier, club reputation scales it.
/// A youth graduate at Man City earns 50x what the same ability player earns in Chad.
fn graduation_salary(current_ability: u8, club_reputation: u16) -> u32 {
    let base = match current_ability {
        0..=60 => 2_000,
        61..=80 => 5_000,
        81..=100 => 12_000,
        101..=120 => 30_000,
        121..=150 => 80_000,
        _ => 200_000,
    };

    // Club reputation multiplier: cubic curve
    //   rep 1000 (amateur)    → ~0.01 → base * 0.10 (floor)
    //   rep 3000 (Malta)      → ~0.03 → base * 0.17
    //   rep 5000 (mid)        → ~0.13 → base * 0.47
    //   rep 7000 (good)       → ~0.34 → base * 1.05
    //   rep 8500 (top)        → ~0.61 → base * 1.73
    //   rep 10000 (elite)     → ~1.00 → base * 3.00
    let norm = (club_reputation as f64 / 10000.0).clamp(0.0, 1.0);
    let multiplier = 0.10 + 2.90 * norm * norm * norm;

    (base as f64 * multiplier).max(500.0) as u32
}
