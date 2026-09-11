use super::ClubIdentity;
use crate::club::Team;
use crate::league::LeagueTable;
use crate::transfers::scouting::desk::StaffIdSequence;
use crate::world::SimulatorData;
use crate::{Club, Player};
use rayon::prelude::*;

/// The rows a freshly-loaded world is missing: an empty league table, a
/// player with no career history, an id sequence that has not been told
/// how far the shipped data already goes.
///
/// Every pass here is idempotent — [`SimulatorData::seed_missing_player_histories`]
/// runs on every tick and must cost nothing when there is nothing to do.
pub struct WorldSeeder;

impl WorldSeeder {
    /// Every team id that participates in `league_id` across `clubs`.
    fn team_ids_for_league(clubs: &[Club], league_id: u32) -> Vec<u32> {
        clubs
            .iter()
            .flat_map(|c| c.teams.with_league(league_id))
            .collect()
    }

    /// True if any team in the club has at least one player needing a
    /// current-season seed entry. Exits as soon as one is found.
    fn club_needs_seed(club: &Club) -> bool {
        club.teams.iter().any(Self::team_needs_seed)
    }

    fn team_needs_seed(team: &Team) -> bool {
        team.players.iter().any(Self::player_needs_seed)
    }

    fn player_needs_seed(player: &Player) -> bool {
        player.statistics_history.needs_current_season_seed()
    }

    /// Highest player id anywhere in the world — rosters, retirees,
    /// generated national squads, and the free-agent pool.
    fn max_player_id(data: &SimulatorData) -> u32 {
        let in_countries = data
            .continents
            .iter()
            .flat_map(|continent| &continent.countries)
            .flat_map(|country| {
                country
                    .clubs
                    .iter()
                    .flat_map(|club| &club.teams.teams)
                    .flat_map(|team| &team.players.players)
                    .chain(&country.retired_players)
                    .chain(&country.national_team.generated_squad)
                    .chain(&country.u21_national_team.generated_squad)
            });
        in_countries
            .chain(&data.free_agents)
            .map(|player| player.id)
            .max()
            .unwrap_or(0)
    }

    /// Highest staff id on any team, plus the unemployed pool.
    fn max_staff_id(data: &SimulatorData) -> u32 {
        data.continents
            .iter()
            .flat_map(|continent| &continent.countries)
            .flat_map(|country| &country.clubs)
            .flat_map(|club| &club.teams.teams)
            .flat_map(|team| &team.staffs.staffs)
            .chain(&data.free_agent_staff)
            .map(|staff| staff.id)
            .max()
            .unwrap_or(0)
    }
}

impl SimulatorData {
    /// Initial population of league tables at construction time.
    /// Per-season rebuilds happen inside `League::simulate` when a new
    /// schedule is generated. The skip-if-non-empty guard below is
    /// therefore intentional: it only prevents the initial seed from
    /// clobbering an already-populated table.
    pub(in crate::world) fn init_league_tables(&mut self) {
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                let clubs = &country.clubs;
                for league in &mut country.leagues.leagues {
                    if !league.table.rows.is_empty() {
                        continue;
                    }
                    let team_ids = WorldSeeder::team_ids_for_league(clubs, league.id);
                    if !team_ids.is_empty() {
                        league.table = LeagueTable::new(&team_ids);
                    }
                }
            });
    }

    /// Seed statistics history for every player. Called once at
    /// construction time — touches every player unconditionally.
    /// Non-senior squads (Reserve, U18..U23) seed under the main-team
    /// alias from `team_info_for` so a player who never leaves the
    /// youth setup still has a "career home" row pointing at the
    /// parent club's main team.
    pub(in crate::world) fn seed_player_histories(&mut self) {
        let date = self.date.date();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                let league_lookup = ClubIdentity::league_lookup(country);
                for club in &mut country.clubs {
                    let identity = ClubIdentity::resolve(club, &league_lookup);
                    for team in club.teams.iter_mut() {
                        let team_info = identity.team_info_for(team);
                        for player in &mut team.players.players {
                            let is_loan = player.is_on_loan();
                            player
                                .statistics_history
                                .seed_initial_team(&team_info, date, is_loan);
                        }
                    }
                }
            });
    }

    /// Seed any players whose history is still empty — catches youth intake,
    /// regens, and newly-generated clubs within one simulated tick.
    /// Skip-fast at club AND team level so the steady-state cost is close
    /// to zero when nothing needs seeding.
    pub fn seed_missing_player_histories(&mut self) {
        let date = self.date.date();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                let league_lookup = ClubIdentity::league_lookup(country);
                for club in &mut country.clubs {
                    if !WorldSeeder::club_needs_seed(club) {
                        continue;
                    }
                    let identity = ClubIdentity::resolve(club, &league_lookup);
                    for team in club.teams.iter_mut() {
                        if !WorldSeeder::team_needs_seed(team) {
                            continue;
                        }
                        let team_info = identity.team_info_for(team);
                        for player in &mut team.players.players {
                            if !WorldSeeder::player_needs_seed(player) {
                                continue;
                            }
                            let is_loan = player.is_on_loan();
                            player
                                .statistics_history
                                .seed_initial_team(&team_info, date, is_loan);
                        }
                    }
                }
            });
    }

    /// Bump the procedural id sequences past the highest id the shipped
    /// world already uses. The single source of truth for future id
    /// allocation — call after world generation (and after any future
    /// save-load path) so runtime academy intake and the scout market
    /// can never mint an id that already exists.
    pub fn seed_player_id_sequence(&self) {
        crate::seed_core_player_id_sequence(WorldSeeder::max_player_id(self));
        StaffIdSequence::seed(WorldSeeder::max_staff_id(self));
    }
}
