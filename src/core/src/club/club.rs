use crate::club::academy::ClubAcademy;
use crate::club::board::ClubBoard;
use crate::club::status::ClubStatus;
use crate::club::{ClubFinances, ClubResult};
use crate::context::GlobalContext;
use crate::shared::Location;
use crate::transfers::pipeline::ClubTransferPlan;
use crate::{Person, PlayerClubContract, TeamCollection, TeamType};
use chrono::{Datelike, NaiveDate};
use log::debug;

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
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ClubResult {
        let date = ctx.simulation.date.date();

        let result = ClubResult::new(
            self.id,
            self.finance.simulate(ctx.with_finance()),
            self.teams.simulate(ctx.with_club(self.id, &self.name)),
            self.board.simulate(ctx.with_board()),
            self.academy.simulate(ctx.clone()),
        );

        if ctx.simulation.is_week_beginning() {
            self.teams.ensure_coach_state(date);
            self.teams.update_all_impressions(date);
            self.process_salaries(ctx.clone());

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
        }

        // Academy graduations at season start
        if ctx.simulation.is_season_start() {
            self.process_academy_graduations(date);
        }

        result
    }

    /// Graduate best academy players to U18 team (5-10 per year).
    /// Move overage youth players to main team.
    /// Aged-out academy players disappear.
    fn process_academy_graduations(&mut self, date: NaiveDate) {
        // Release aged-out academy players first
        let released = self.academy.release_aged_out(date);
        if released > 0 {
            debug!("academy {}: {} aged-out players released", self.name, released);
        }

        // Find U18 team index
        let u18_idx = self.teams.teams.iter().position(|t| t.team_type == TeamType::U18);

        // Graduate best academy players to U18 team
        if let Some(idx) = u18_idx {
            let u18_count = self.teams.teams[idx].players.players.len();
            let target = 20usize;
            let space = target.saturating_sub(u18_count);
            let to_graduate = space.max(5).min(10);

            let graduated = self.academy.graduate_to_u18(date, to_graduate);
            if !graduated.is_empty() {
                debug!("academy {}: {} players graduated to U18 (was {})",
                    self.name, graduated.len(), u18_count);
                for player in graduated {
                    self.teams.teams[idx].players.add(player);
                }
            }
        }

        // Move overage players from youth teams to main team
        self.enforce_youth_team_age_limits(date);

        // Fill main team if still short
        self.promote_youth_to_main_if_needed(date);
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

    fn process_salaries(&mut self, ctx: GlobalContext<'_>) {
        for team in &self.teams.teams {
            let weekly_salary = team.get_week_salary();
            self.finance.push_salary(
                ctx.club.as_ref().expect("no club found").name,
                weekly_salary as i32,
            );
        }
    }
}

fn graduation_salary(current_ability: u8) -> u32 {
    match current_ability {
        0..=60 => 500,
        61..=80 => 1000,
        81..=100 => 2000,
        101..=120 => 3000,
        121..=150 => 5000,
        _ => 8000,
    }
}
