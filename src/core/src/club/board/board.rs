use crate::club::{BoardContext, BoardMood, BoardMoodState, BoardResult, StaffClubContract};
use crate::context::{GlobalContext, SimulationContext};

#[derive(Debug, Clone)]
pub struct SeasonTargets {
    pub transfer_budget: i32,
    pub wage_budget: i32,
    pub max_squad_size: u8,
    pub min_squad_size: u8,
}

#[derive(Debug, Clone)]
pub struct ClubBoard {
    pub mood: BoardMood,
    pub director: Option<StaffClubContract>,
    pub sport_director: Option<StaffClubContract>,
    pub season_targets: Option<SeasonTargets>,
}

impl ClubBoard {
    pub fn new() -> Self {
        ClubBoard {
            mood: BoardMood::default(),
            director: None,
            sport_director: None,
            season_targets: None,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> BoardResult {
        let mut result = BoardResult::new();

        if self.director.is_none() {
            self.run_director_election(&ctx.simulation);
        }

        if self.sport_director.is_none() {
            self.run_sport_director_election(&ctx.simulation);
        }

        if ctx.simulation.check_contract_expiration() {
            if self.is_director_contract_expiring(&ctx.simulation) {}

            if self.is_sport_director_contract_expiring(&ctx.simulation) {}
        }

        // Season start: calculate season targets
        if ctx.simulation.is_season_start() {
            if let Some(board_ctx) = &ctx.board {
                self.calculate_season_targets(board_ctx);
            }
        }

        // Monthly: evaluate mood based on targets
        if ctx.simulation.is_month_beginning() {
            if let Some(board_ctx) = &ctx.board {
                self.evaluate_mood(board_ctx, &mut result);
            }
        }

        result
    }

    fn calculate_season_targets(&mut self, board_ctx: &BoardContext) {
        let rep = board_ctx.reputation_score;

        // Transfer budget: % of balance based on reputation tier
        let budget_pct = if rep >= 0.8 {
            0.40 // Elite
        } else if rep >= 0.6 {
            0.35 // Continental
        } else if rep >= 0.4 {
            0.30 // National
        } else if rep >= 0.2 {
            0.25 // Regional
        } else {
            0.20 // Amateur
        };

        let transfer_budget = if board_ctx.balance > 0 {
            (board_ctx.balance as f64 * budget_pct) as i32
        } else {
            0
        };

        // Wage budget: current annual wages * growth factor
        let wage_growth = if rep >= 0.7 {
            1.10
        } else if rep >= 0.4 {
            1.05
        } else {
            1.00
        };
        let annual_wages = board_ctx.total_annual_wages as f64;
        let wage_budget = (annual_wages * wage_growth) as i32;

        // Squad size limits based on reputation (main team)
        let (min_squad, max_squad) = if rep >= 0.8 {
            (25u8, 50u8) // Elite
        } else if rep >= 0.6 {
            (23, 45) // Continental
        } else if rep >= 0.4 {
            (20, 38) // National
        } else if rep >= 0.2 {
            (18, 30) // Regional
        } else {
            (16, 25) // Amateur
        };

        self.season_targets = Some(SeasonTargets {
            transfer_budget,
            wage_budget,
            max_squad_size: max_squad,
            min_squad_size: min_squad,
        });
    }

    fn evaluate_mood(&mut self, board_ctx: &BoardContext, _result: &mut BoardResult) {
        let targets = match &self.season_targets {
            Some(t) => t,
            None => return,
        };

        let total_squad = board_ctx.main_squad_size + board_ctx.reserve_squad_size;
        let squad_bloated = total_squad > (targets.max_squad_size as usize + 5);
        let financial_stress = board_ctx.balance < 0;

        if squad_bloated && financial_stress {
            self.mood.state = BoardMoodState::Poor;
        } else if squad_bloated || financial_stress {
            self.mood.state = BoardMoodState::Normal;
        } else {
            self.mood.state = BoardMoodState::Good;
        }
    }

    fn is_director_contract_expiring(&self, simulation_ctx: &SimulationContext) -> bool {
        match &self.director {
            Some(d) => d.is_expired(simulation_ctx),
            None => false,
        }
    }

    fn run_director_election(&mut self, _: &SimulationContext) {}

    fn is_sport_director_contract_expiring(&self, simulation_ctx: &SimulationContext) -> bool {
        match &self.director {
            Some(d) => d.is_expired(simulation_ctx),
            None => false,
        }
    }

    fn run_sport_director_election(&mut self, _: &SimulationContext) {}
}
