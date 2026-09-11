use crate::context::{GlobalContext, HomeLeagueTable, SimulationContext, TournamentClocks};
use crate::continent::national::world::WorldNationalCompetitions;
use crate::simulator::SimulationResult;
use crate::utils::PerformanceProfiler;
use crate::world::SimulatorData;

/// Everything the world must agree on before any continent ticks:
/// the confederation calendars, what every country's top flight is
/// worth, the passports, the international call-ups, and the national
/// fixture programme those call-ups exist to fill.
pub struct Prologue;

impl Prologue {
    pub fn run<'gc>(data: &mut SimulatorData, result: &mut SimulationResult) -> GlobalContext<'gc> {
        let _phase = PerformanceProfiler::phase_scope("0_prologue", 0);
        let today = data.date.date();

        let ctx = Self::global_context(data);

        // Where the world's players are FROM. Seeded once at load, and
        // re-run at each season turn because everything created since —
        // an academy intake, a regen, a synthetic international — starts
        // with an unstamped passport, and an unstamped passport reads as
        // "no home" everywhere the loan-home pathway looks. Cheap: a
        // parallel pass that skips anyone already stamped.
        if SimulatorData::is_nationality_reseed_day(today) {
            data.seed_player_nationality_continents();
        }

        // Call-ups run at the world level so a player's nationality and
        // their club's continent can differ, and BEFORE the national
        // fixtures below — those matches need a populated squad with
        // world visibility.
        data.process_world_national_team_callups();

        // National-team matches simulate at the world level so squads can
        // include foreign-based players and post-match stats updates fan
        // out across every continent. Lifted out of the parallel continent
        // phase because squad construction needs read access to clubs in
        // *every* continent.
        let national_results = WorldNationalCompetitions::simulate(&mut data.continents, today);
        for match_result in &national_results {
            data.match_store.push(match_result.clone(), today);
        }
        result.match_results.extend(national_results);

        ctx
    }

    /// The two world-level tables every continent reads but none can
    /// build: a tournament clock and a home-league value both belong to
    /// a passport, not a postcode — a Brazilian at Arsenal counts down to
    /// the Copa, not the Euros, and "is his HOME league worth going back
    /// to?" is a question only the world level holds every answer to.
    ///
    /// The home-league table is read off `country_info`, which covers
    /// nationalities whose leagues are not in this save at all.
    fn global_context<'gc>(data: &SimulatorData) -> GlobalContext<'gc> {
        let tournament_clocks = TournamentClocks::new(
            data.continents
                .iter()
                .map(|c| {
                    (
                        c.id,
                        c.national_team_competitions
                            .months_to_next_tournament(data.date.date()),
                    )
                })
                .collect(),
        );
        let home_leagues = HomeLeagueTable::new(
            data.country_info
                .values()
                .map(|c| (c.id, c.top_flight_reputation))
                .collect(),
        );
        GlobalContext::new(
            SimulationContext::new(data.date)
                .with_tournament_clocks(tournament_clocks)
                .with_home_leagues(home_leagues),
        )
    }
}
