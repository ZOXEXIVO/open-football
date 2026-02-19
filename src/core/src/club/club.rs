use crate::club::academy::ClubAcademy;
use crate::club::board::ClubBoard;
use crate::club::status::ClubStatus;
use crate::club::{ClubFinances, ClubResult};
use crate::context::GlobalContext;
use crate::shared::Location;
use crate::TeamCollection;

#[derive(Debug, Clone)]
pub struct ClubColors {
    pub primary: String,
    pub secondary: String,
}

impl Default for ClubColors {
    fn default() -> Self {
        ClubColors {
            primary: "#1e272d".to_string(),
            secondary: "#ffffff".to_string(),
        }
    }
}

#[derive(Debug)]
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
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ClubResult {
        let result = ClubResult::new(
            self.finance.simulate(ctx.with_finance()),
            self.teams.simulate(ctx.with_club(self.id, &self.name)),
            self.board.simulate(ctx.with_board()),
            self.academy.simulate(ctx.clone()),
        );

        if ctx.simulation.is_week_beginning() {
            let date = ctx.simulation.date.date();
            self.process_salaries(ctx);
            self.teams.manage_squad_composition(date);
        }

        result
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
