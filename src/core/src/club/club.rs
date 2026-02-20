use crate::club::academy::ClubAcademy;
use crate::club::board::ClubBoard;
use crate::club::status::ClubStatus;
use crate::club::{ClubFinances, ClubResult};
use crate::context::GlobalContext;
use crate::shared::Location;
use crate::transfers::pipeline::ClubTransferPlan;
use crate::TeamCollection;

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
        let result = ClubResult::new(
            self.finance.simulate(ctx.with_finance()),
            self.teams.simulate(ctx.with_club(self.id, &self.name)),
            self.board.simulate(ctx.with_board()),
            self.academy.simulate(ctx.clone()),
        );

        let date = ctx.simulation.date.date();

        if ctx.simulation.is_week_beginning() {
            // Weekly: comprehensive review (demotions, recalls, youth promotions, salaries)
            // Subsumes daily critical moves to avoid double-processing
            self.process_salaries(ctx);
            self.teams.manage_squad_composition(date);
        } else {
            // Daily: only immediate demotions + ability swaps
            self.teams.manage_critical_squad_moves(date);
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
