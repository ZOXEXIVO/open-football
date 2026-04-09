mod finances;
mod graduation;
mod squad;
mod utilization;

use graduation::graduation_salary;

use crate::club::academy::ClubAcademy;
use crate::club::board::{BoardContext, ClubBoard};
use crate::club::facilities::ClubFacilities;
use crate::club::status::ClubStatus;
use crate::club::{ClubFinances, ClubResult};
use crate::context::GlobalContext;
use crate::shared::Location;
use crate::transfers::pipeline::ClubTransferPlan;
use crate::{ReputationLevel, TeamCollection, TeamType};

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

        let country_economic_factor = ctx.country.as_ref()
            .map(|c| c.tv_revenue_multiplier)
            .unwrap_or(1.0);
        let country_price_level = ctx.country.as_ref()
            .map(|c| c.price_level)
            .unwrap_or(1.0);
        // League position from country-level context
        let (league_pos, league_sz, total_matches) = ctx.club.as_ref()
            .map(|c| (c.league_position, c.league_size, c.total_league_matches))
            .unwrap_or((0, 0, 0));

        let mut board_ctx = self.build_board_context(country_economic_factor, country_price_level);
        board_ctx.league_position = league_pos;
        board_ctx.league_size = league_sz;
        board_ctx.total_matches = total_matches;

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

            // Weekly: rebalance players across all teams
            self.rebalance_squads(date);
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
            let country_code = ctx.country.as_ref().map(|c| c.code.as_str()).unwrap_or("");
            result.academy_transfers = self.process_academy_graduations(date, country_code);
            self.trim_positional_surplus(date);
        }

        result
    }

    fn build_board_context(&self, country_economic_factor: f32, country_price_level: f32) -> BoardContext {
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

        // Recent form from match history (last 5 matches)
        let (recent_wins, recent_losses) = main_team
            .map(|t| {
                let recent: Vec<_> = t.match_history.items().iter().rev().take(5).collect();
                let wins = recent.iter().filter(|m| m.score.0.get() > m.score.1.get()).count() as u8;
                let losses = recent.iter().filter(|m| m.score.0.get() < m.score.1.get()).count() as u8;
                (wins, losses)
            })
            .unwrap_or((0, 0));

        let matches_played = main_team
            .map(|t| t.match_history.items().len().min(255) as u8)
            .unwrap_or(0);

        // Average squad ability
        let avg_squad_ability = main_team
            .map(|t| {
                if t.players.players.is_empty() { return 0u8; }
                let sum: u32 = t.players.players.iter()
                    .map(|p| p.player_attributes.current_ability as u32)
                    .sum();
                (sum / t.players.players.len() as u32) as u8
            })
            .unwrap_or(0);

        BoardContext {
            balance: self.finance.balance.balance,
            total_annual_wages,
            reputation_score,
            main_squad_size,
            reserve_squad_size,
            country_economic_factor,
            country_price_level,
            league_position: 0,
            league_size: 0,
            recent_wins,
            recent_losses,
            matches_played,
            total_matches: 0,
            avg_squad_ability,
        }
    }
}
