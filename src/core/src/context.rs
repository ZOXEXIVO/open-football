pub use chrono::prelude::*;

use crate::ai::Ai;
use crate::club::{BoardContext, ClubContext, ClubFinanceContext, PlayerContext, StaffContext};
use crate::continent::ContinentContext;
use crate::country::CountryContext;
use crate::league::LeagueContext;
use crate::TeamContext;

#[derive(Clone)]
pub struct GlobalContext<'gc> {
    pub simulation: SimulationContext,
    pub ai: Ai,
    pub continent: Option<ContinentContext>,
    pub country: Option<CountryContext>,
    pub league: Option<LeagueContext<'gc>>,
    pub club: Option<ClubContext<'gc>>,
    pub team: Option<TeamContext>,
    pub finance: Option<ClubFinanceContext>,
    pub board: Option<BoardContext>,
    pub player: Option<PlayerContext>,
    pub staff: Option<StaffContext>,
}

impl<'gc> GlobalContext<'gc> {
    pub fn new(simulation_ctx: SimulationContext, ai: Ai) -> Self {
        GlobalContext {
            simulation: simulation_ctx,
            ai,
            continent: None,
            country: None,
            league: None,
            club: None,
            team: None,
            finance: None,
            board: None,
            player: None,
            staff: None,
        }
    }

    pub fn with_continent(&self, continent_id: u32) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.continent = Some(ContinentContext::new(continent_id));
        ctx
    }

    pub fn with_country(&self, country_id: u32) -> Self {
        let mut ctx = GlobalContext::clone(self);

        ctx.country = Some(CountryContext::new(country_id));
        ctx
    }

    pub fn with_country_and_names(&self, country_id: u32, country_code: String, people_names: crate::PeopleNameGeneratorData, season_dates: crate::country::SeasonDates) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.country = Some(CountryContext::with_people_names(country_id, people_names).with_code(country_code).with_season_dates(season_dates));
        ctx
    }

    pub fn with_league(&self, league_id: u32, league_slug: String, team_ids: &'gc [u32]) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.league = Some(LeagueContext::new(league_id, league_slug, team_ids));
        ctx
    }

    pub fn with_club(&self, club_id: u32, club_name: &'gc str) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.club = Some(ClubContext::new(club_id, club_name));
        ctx
    }

    pub fn with_team(&self, team_id: u32) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.team = Some(TeamContext::new(team_id));
        ctx
    }

    pub fn with_team_reputation(&self, team_id: u32, reputation: f32) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.team = Some(TeamContext::with_reputation(team_id, reputation));
        ctx
    }

    pub fn with_board(&self) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.board = Some(BoardContext::new());
        ctx
    }

    pub fn with_board_data(&self, board_ctx: BoardContext) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.board = Some(board_ctx);
        ctx
    }

    pub fn with_player(&self, player_id: Option<u32>) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.player = Some(PlayerContext::new(player_id));
        ctx
    }

    pub fn with_staff(&self, staff_id: Option<u32>) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.staff = Some(StaffContext::new(staff_id));
        ctx
    }

    pub fn with_finance(&self) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.finance = Some(ClubFinanceContext::new());
        ctx
    }

}

#[derive(Clone)]
pub struct SimulationContext {
    pub date: NaiveDateTime,
    pub day: u8,
    pub hour: u8,
}

impl SimulationContext {
    pub fn new(date: NaiveDateTime) -> Self {
        SimulationContext {
            date,
            day: date.day() as u8,
            hour: date.hour() as u8,
        }
    }

    #[inline]
    pub fn is_week_beginning(&self) -> bool {
        self.date.weekday() == Weekday::Mon && self.date.hour() == 0
    }

    #[inline]
    pub fn is_month_beginning(&self) -> bool {
        self.day == 1u8 && self.hour == 0
    }

    #[inline]
    pub fn is_year_beginning(&self) -> bool {
        self.day == 1u8 && self.date.month() == 1 && self.hour == 0
    }

    #[inline]
    pub fn is_season_start(&self, season: &crate::country::SeasonDates) -> bool {
        self.hour == 0
            && self.day == season.start_day
            && self.date.month() as u8 == season.start_month
    }

    #[inline]
    pub fn check_contract_expiration(&self) -> bool {
        self.hour == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_simulation_context() {
        // Create a new simulation context
        let date = NaiveDate::from_ymd_opt(2024, 3, 16)
            .unwrap()
            .and_hms_opt(12, 30, 0)
            .unwrap();

        let sim_ctx = SimulationContext::new(date);

        // Test if the date and time are set correctly
        assert_eq!(sim_ctx.date, date);
        assert_eq!(sim_ctx.day, 16);
        assert_eq!(sim_ctx.hour, 12);

        // Test the helper functions
        assert!(!sim_ctx.is_week_beginning()); // Not Monday
        assert!(!sim_ctx.is_month_beginning()); // Not the first day of the month
        assert!(!sim_ctx.is_year_beginning()); // Not the first day of the year
        assert!(!sim_ctx.check_contract_expiration()); // Not midnight

        // Create a new simulation context at the beginning of the week
        let monday_date = NaiveDate::from_ymd_opt(2024, 3, 18)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();

        let monday_sim_ctx = SimulationContext::new(monday_date);

        // Test if the week beginning is detected correctly
        assert!(monday_sim_ctx.is_week_beginning());

        // Create a new simulation context at the beginning of the month (midnight)
        let first_of_month_date = NaiveDate::from_ymd_opt(2024, 3, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();

        let first_of_month_sim_ctx = SimulationContext::new(first_of_month_date);

        // Test if the month beginning is detected correctly
        assert!(first_of_month_sim_ctx.is_month_beginning());

        // Non-midnight on 1st should NOT be month beginning
        let first_noon = NaiveDate::from_ymd_opt(2024, 3, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        assert!(!SimulationContext::new(first_noon).is_month_beginning());

        // Create a new simulation context at the beginning of the year (midnight)
        let first_of_year_date = NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();

        let first_of_year_sim_ctx = SimulationContext::new(first_of_year_date);

        // Test if the year beginning is detected correctly
        assert!(first_of_year_sim_ctx.is_year_beginning());

        // Create a new simulation context at midnight
        let midnight_date = NaiveDate::from_ymd_opt(2024, 3, 16)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();

        let midnight_sim_ctx = SimulationContext::new(midnight_date);

        // Test if contract expiration is checked correctly at midnight
        assert!(midnight_sim_ctx.check_contract_expiration());
    }

    #[test]
    fn test_global_context() {
        // Create a new simulation context
        let date = NaiveDate::from_ymd_opt(2024, 3, 16)
            .unwrap()
            .and_hms_opt(12, 30, 0)
            .unwrap();
        let sim_ctx = SimulationContext::new(date);

        // Create a global context with the simulation context
        let global_ctx = GlobalContext::new(sim_ctx.clone(), Ai::new());

        // Test if the simulation context is set correctly
        assert_eq!(global_ctx.simulation.date, sim_ctx.date);
        assert_eq!(global_ctx.simulation.day, sim_ctx.day);
        assert_eq!(global_ctx.simulation.hour, sim_ctx.hour);

        // Test if other contexts are initially set to None
        assert!(global_ctx.continent.is_none());
        assert!(global_ctx.country.is_none());
        assert!(global_ctx.league.is_none());
        assert!(global_ctx.club.is_none());
        assert!(global_ctx.team.is_none());
        assert!(global_ctx.finance.is_none());
        assert!(global_ctx.board.is_none());
        assert!(global_ctx.player.is_none());
        assert!(global_ctx.staff.is_none());

        // Test if contexts can be added
        let updated_global_ctx = global_ctx
            .with_continent(1)
            .with_country(1)
            .with_league(1, "slug".to_owned(), &[1, 2])
            .with_club(1, "Test Club")
            .with_team(1)
            .with_finance()
            .with_board()
            .with_player(Some(1))
            .with_staff(Some(1));

        // Test if the added contexts are set correctly
        assert!(updated_global_ctx.continent.is_some());
        assert!(updated_global_ctx.country.is_some());
        assert!(updated_global_ctx.league.is_some());
        assert!(updated_global_ctx.club.is_some());
        assert!(updated_global_ctx.team.is_some());
        assert!(updated_global_ctx.finance.is_some());
        assert!(updated_global_ctx.board.is_some());
        assert!(updated_global_ctx.player.is_some());
        assert!(updated_global_ctx.staff.is_some());
    }
}
