pub use chrono::prelude::*;

use crate::PeopleNameGeneratorData;
use crate::TeamContext;
use crate::TeamType;
use crate::club::{BoardContext, ClubContext, ClubFinanceContext, PlayerContext, StaffContext};
use crate::continent::ContinentContext;
use crate::country::{CountryContext, SeasonDates};
use crate::league::LeagueContext;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct GlobalContext<'gc> {
    pub simulation: SimulationContext,
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
    pub fn new(simulation_ctx: SimulationContext) -> Self {
        GlobalContext {
            simulation: simulation_ctx,
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

    pub fn with_country_and_names(
        &self,
        country_id: u32,
        country_code: String,
        people_names: Arc<PeopleNameGeneratorData>,
        season_dates: SeasonDates,
    ) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.country = Some(
            CountryContext::with_people_names(country_id, people_names)
                .with_code(country_code)
                .with_season_dates(season_dates),
        );
        ctx
    }

    /// Stamp the confederation's tournament clock onto an existing
    /// country scope. Separate from [`Self::with_country_and_names`]
    /// because the number belongs to the continent, is the same for every
    /// country under it, and is worked out once above the fan-out.
    pub fn with_country_tournament_clock(mut self, months: u8) -> Self {
        if let Some(country) = self.country.take() {
            self.country = Some(country.with_tournament_clock(months));
        }
        self
    }

    pub fn with_league(
        &self,
        league_id: u32,
        league_slug: String,
        team_ids: &'gc [u32],
        reputation: u16,
    ) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.league = Some(LeagueContext::new(
            league_id,
            league_slug,
            team_ids,
            reputation,
        ));
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

    /// `with_team` plus the squad tier (Main / B / Reserve / …) so squad
    /// behaviour passes can reason about life below the first team.
    pub fn with_team_typed(&self, team_id: u32, team_type: TeamType) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.team = Some(TeamContext::new(team_id).with_type(team_type));
        ctx
    }

    /// `with_team_typed` plus the official captaincy pair, so
    /// captain-centric behaviour passes (mediation, morale propagation)
    /// act through the appointed armband holder rather than electing
    /// their own.
    pub fn with_team_behaviour(
        &self,
        team_id: u32,
        team_type: TeamType,
        captain_id: Option<u32>,
        vice_captain_id: Option<u32>,
    ) -> Self {
        let mut ctx = GlobalContext::clone(self);
        ctx.team = Some(
            TeamContext::new(team_id)
                .with_type(team_type)
                .with_captaincy(captain_id, vice_captain_id),
        );
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

    /// Get training facility quality from club context (0.0-1.0)
    pub fn club_facilities_training(&self) -> f32 {
        self.club
            .as_ref()
            .map(|c| c.training_facility_quality)
            .unwrap_or(0.35)
    }

    /// Get youth facility quality from club context (0.0-1.0)
    pub fn club_facilities_youth(&self) -> f32 {
        self.club
            .as_ref()
            .map(|c| c.youth_facility_quality)
            .unwrap_or(0.35)
    }

    /// Get academy quality from club context (0.0-1.0)
    pub fn club_academy_quality(&self) -> f32 {
        self.club
            .as_ref()
            .map(|c| c.academy_quality)
            .unwrap_or(0.35)
    }

    /// Get youth recruitment quality from club context (0.0-1.0)
    pub fn club_recruitment_quality(&self) -> f32 {
        self.club
            .as_ref()
            .map(|c| c.recruitment_quality)
            .unwrap_or(0.35)
    }

    /// Best physiotherapy score on the club's medical staff (0.0-1.0).
    pub fn club_medical_quality(&self) -> f32 {
        self.club
            .as_ref()
            .map(|c| c.medical_quality)
            .unwrap_or(0.35)
    }

    /// Best sports_science score on the club's medical staff (0.0-1.0).
    pub fn club_sports_science_quality(&self) -> f32 {
        self.club
            .as_ref()
            .map(|c| c.sports_science_quality)
            .unwrap_or(0.35)
    }

    /// Best working_with_youngsters score on the club's coaching staff (0.0-1.0).
    pub fn club_youth_coaching_quality(&self) -> f32 {
        self.club
            .as_ref()
            .map(|c| c.youth_coaching_quality)
            .unwrap_or(0.35)
    }

    /// Main team's blended reputation (0..10000) from club context.
    pub fn club_main_reputation(&self) -> u16 {
        self.club
            .as_ref()
            .map(|c| c.main_team_reputation)
            .unwrap_or(0)
    }

    /// Main team's world reputation (0..10000) from club context.
    pub fn club_main_world_reputation(&self) -> u16 {
        self.club
            .as_ref()
            .map(|c| c.main_team_world_reputation)
            .unwrap_or(0)
    }

    /// Main team's league reputation (0..10000).
    pub fn club_league_reputation(&self) -> u16 {
        self.club.as_ref().map(|c| c.league_reputation).unwrap_or(0)
    }

    /// Country football-ecosystem reputation (0..10000).
    pub fn club_country_reputation(&self) -> u16 {
        self.club
            .as_ref()
            .map(|c| c.country_reputation)
            .unwrap_or(0)
    }

    /// Academy pathway reputation (0..100) — internal prestige of the youth
    /// pipeline, separate from the club's outward-facing reputation.
    pub fn club_pathway_reputation(&self) -> u8 {
        self.club
            .as_ref()
            .map(|c| c.pathway_reputation)
            .unwrap_or(50)
    }
}

/// Months to each confederation's next major tournament, keyed by
/// continent id.
///
/// A tournament clock belongs to a PASSPORT, not to a postcode. Every
/// country was stamped with its own confederation's calendar and the mind
/// read that, so a Brazilian at Arsenal counted down to the Euros and a
/// Ghanaian in Spain never felt the AFCON at all — while the whole point
/// of the term is a man measuring a move against the tournament HE might
/// play in. Published once at the world level, read by nationality.
///
/// Empty (the default) means nobody has published a calendar this tick;
/// callers then keep whatever their country says, which is the old
/// behaviour and the correct one for a native.
#[derive(Debug, Clone, Default)]
pub struct TournamentClocks {
    by_continent: Arc<HashMap<u32, u8>>,
}

impl TournamentClocks {
    pub fn new(by_continent: HashMap<u32, u8>) -> Self {
        TournamentClocks {
            by_continent: Arc::new(by_continent),
        }
    }

    /// Months to the tournament this passport plays in. `None` for an
    /// unstamped nationality or an unpublished calendar — the caller then
    /// falls back to where he plays.
    ///
    /// The argument is an `Option` rather than a zero sentinel because
    /// continent 0 is AFRICA: the old `== 0 → None` guard handed every
    /// African abroad his CLUB's calendar, which is the one population the
    /// passport clock exists for.
    pub fn months_for(&self, nationality_continent_id: Option<u32>) -> Option<u8> {
        self.by_continent.get(&nationality_continent_id?).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.by_continent.is_empty()
    }
}

/// The standard of every country's TOP FLIGHT, keyed by country id.
///
/// "Is his own league worth going back to?" is a question about a
/// PASSPORT, and no country can answer it about anyone but itself — the
/// transfer pipeline runs inside one country's borrow. So the world
/// publishes the table once a tick and every country carries an `Arc` of
/// it, exactly the way [`TournamentClocks`] carries the calendars.
///
/// This is a league-reputation lookup, not a list: no country is named
/// anywhere, and a league that rises or falls moves its own countrymen
/// with it (memory `feedback_balance_system_not_cases`).
#[derive(Debug, Clone, Default)]
pub struct HomeLeagueTable {
    by_country: Arc<HashMap<u32, u16>>,
}

impl HomeLeagueTable {
    pub fn new(by_country: HashMap<u32, u16>) -> Self {
        HomeLeagueTable {
            by_country: Arc::new(by_country),
        }
    }

    /// Standard of the strongest league this country runs. `0` for a
    /// country with no leagues in the save and for an unpublished table —
    /// both fail every "worth going home for" bar closed.
    pub fn reputation_of(&self, country_id: u32) -> u16 {
        self.by_country.get(&country_id).copied().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.by_country.is_empty()
    }
}

#[derive(Clone)]
pub struct SimulationContext {
    pub date: NaiveDateTime,
    pub day: u8,
    pub hour: u8,
    /// How far off each confederation's next major tournament is, by
    /// continent id. See [`TournamentClocks`].
    pub tournament_clocks: TournamentClocks,
    /// What each country's best league is worth. See [`HomeLeagueTable`].
    pub home_leagues: HomeLeagueTable,
}

impl SimulationContext {
    pub fn new(date: NaiveDateTime) -> Self {
        SimulationContext {
            date,
            day: date.day() as u8,
            hour: date.hour() as u8,
            tournament_clocks: TournamentClocks::default(),
            home_leagues: HomeLeagueTable::default(),
        }
    }

    /// Stamp the world's tournament calendars onto the tick.
    pub fn with_tournament_clocks(mut self, clocks: TournamentClocks) -> Self {
        self.tournament_clocks = clocks;
        self
    }

    /// Stamp the world's top-flight standings onto the tick.
    pub fn with_home_leagues(mut self, leagues: HomeLeagueTable) -> Self {
        self.home_leagues = leagues;
        self
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
    pub fn is_quarter_beginning(&self) -> bool {
        self.day == 1u8 && self.date.month() % 3 == 0 && self.hour == 0
    }

    #[inline]
    pub fn is_year_beginning(&self) -> bool {
        self.day == 1u8 && self.date.month() == 1 && self.hour == 0
    }

    #[inline]
    pub fn is_season_start(&self, season: &SeasonDates) -> bool {
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
        let global_ctx = GlobalContext::new(sim_ctx.clone());

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
            .with_league(1, "slug".to_owned(), &[1, 2], 5000)
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

/// A5 — the tournament clock belongs to a passport, and continent 0 is
/// AFRICA. The old signature took a `u32` with 0 meaning "unknown", so
/// every African abroad read his CLUB's calendar — the single largest
/// population the passport clock exists for.
#[cfg(test)]
mod tournament_clock_tests {
    use super::*;

    /// `ScoutingRegion::from_country` maps continent 0 to Africa.
    const AFRICA: u32 = 0;
    const EUROPE: u32 = 1;

    fn clocks() -> TournamentClocks {
        TournamentClocks::new(HashMap::from([(AFRICA, 11u8), (EUROPE, 34u8)]))
    }

    #[test]
    fn an_african_abroad_counts_down_to_his_own_tournament() {
        let clocks = clocks();
        assert_eq!(
            clocks.months_for(Some(AFRICA)),
            Some(11),
            "eleven months to the AFCON, wherever he plays"
        );
        assert_eq!(clocks.months_for(Some(EUROPE)), Some(34));
    }

    #[test]
    fn an_unstamped_passport_has_no_clock_of_its_own() {
        assert_eq!(clocks().months_for(None), None);
        // …and so does a nationality nobody published a calendar for.
        assert_eq!(clocks().months_for(Some(99)), None);
    }

    /// What every country's best league is worth, published once for the
    /// whole world and read by passport.
    #[test]
    fn a_home_league_lookup_fails_closed_on_a_country_it_has_never_heard_of() {
        let table = HomeLeagueTable::new(HashMap::from([(7u32, 6_400u16)]));
        assert_eq!(table.reputation_of(7), 6_400);
        assert_eq!(table.reputation_of(8), 0);
        assert_eq!(HomeLeagueTable::default().reputation_of(7), 0);
    }
}
