use crate::club::{BoardMood, BoardResult, StaffClubContract};
use crate::context::{GlobalContext, SimulationContext};

#[derive(Debug, Clone)]
pub struct ClubBoard {
    pub mood: BoardMood,
    pub director: Option<StaffClubContract>,
    pub sport_director: Option<StaffClubContract>,
}

impl ClubBoard {
    pub fn new() -> Self {
        ClubBoard {
            mood: BoardMood::default(),
            director: None,
            sport_director: None,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> BoardResult {
        let result = BoardResult::new();

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

        result
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
