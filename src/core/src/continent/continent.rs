use crate::Country;
use crate::context::GlobalContext;
use crate::continent::national::{NationalCompetitionConfig, NationalTeamCompetitions};
use crate::continent::{
    ContinentalCompetitions, ContinentalRankings, ContinentalRegulations, EconomicZone,
};
use crate::country::{CountryPendingState, CountryResult};
use crate::league::FixtureRest;
use crate::league::result::WorldSnapshot;
use crate::r#match::{Match, MatchResult};
use crate::utils::PerformanceProfiler;
use crate::utils::SlowStep;
use chrono::{Duration, NaiveDate};
use log::debug;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use rayon::prelude::{IntoParallelRefIterator, IntoParallelRefMutIterator};
use std::ops::Range;

/// Reserved continent ids used to scope continental club competitions.
/// UEFA competitions only run in Europe; Copa Libertadores only runs in
/// South America. The id is the primary check; the name is a readable
/// fallback for data sets that haven't pinned the canonical id.
pub const CONTINENT_EUROPE_ID: u32 = 1;
pub const CONTINENT_SOUTH_AMERICA_ID: u32 = 3;

pub struct Continent {
    pub id: u32,
    pub name: String,
    pub countries: Vec<Country>,

    pub continental_competitions: ContinentalCompetitions,
    pub continental_rankings: ContinentalRankings,
    pub regulations: ContinentalRegulations,
    pub economic_zone: EconomicZone,
    pub national_team_competitions: NationalTeamCompetitions,
}

/// Countries are the bulk of a world copy and independent of one another,
/// so they are copied across the pool rather than one after another.
impl Clone for Continent {
    fn clone(&self) -> Self {
        Continent {
            id: self.id,
            name: self.name.clone(),
            countries: self.countries.par_iter().cloned().collect(),
            continental_competitions: self.continental_competitions.clone(),
            continental_rankings: self.continental_rankings.clone(),
            regulations: self.regulations.clone(),
            economic_zone: self.economic_zone.clone(),
            national_team_competitions: self.national_team_competitions.clone(),
        }
    }
}

impl Continent {
    /// Drop the countries across the pool, the counterpart of the copy.
    pub(crate) fn release(self) {
        self.countries.into_par_iter().for_each(drop);
    }
}

/// Resume state stashed between [`Continent::simulate`] and
/// [`Continent::country_work`]. Holds the per-country
/// `GlobalContext`s (cloned with their name generators) and the
/// pending states so the process pass doesn't rebuild them, plus the
/// index ranges that map each country's slice inside the continent's
/// flattened match batch.
pub struct ContinentBuildState<'gc> {
    pub country_ctxs: Vec<GlobalContext<'gc>>,
    pub country_pending: Vec<CountryPendingState>,
    /// Per-country range over the continent's flattened match batch.
    /// `country_ranges[i]` is the slice of the build's `matches` that
    /// belongs to `countries[i]`.
    pub country_ranges: Vec<Range<usize>>,
}

/// One country's post-dispatch tick: its league and cup results, its
/// clubs and its transfer market.
pub struct CountryWork<'a, 'gc> {
    country: &'a mut Country,
    ctx: GlobalContext<'gc>,
    pending: CountryPendingState,
    results: Vec<MatchResult>,
}

impl CountryWork<'_, '_> {
    /// Players on the country's rosters, which is what its tick costs.
    pub fn weight(&self) -> usize {
        self.country
            .clubs
            .iter()
            .flat_map(|club| club.teams.teams.iter())
            .map(|team| team.players.players.len())
            .sum()
    }

    pub fn run(self, world: WorldSnapshot<'_>) -> CountryResult {
        let country = self.country;
        let stage = PerformanceProfiler::stage_scope("country_process", 1)
            .labelled(|| country.name.clone());
        let step = SlowStep::start();
        let output = country.simulate_process(self.ctx, world, self.pending, self.results);
        step.finish(|| format!("simulate country: {}", country.name));
        drop(stage);
        output
    }
}

/// Per-continent output of [`Continent::simulate`]. Only the build
/// has happened — every `Match` is `Match::make`-d but unplayed. The
/// simulator collects every continent's `ContinentBuildOutput` into a
/// single `WorldMatchdayResult`, whose `process` then flattens every
/// continent's matches into ONE global batch and dispatches via the
/// engine pool exactly once per tick.
pub struct ContinentBuildOutput<'gc> {
    pub continent_id: u32,
    pub continent_name: String,
    pub matches: Vec<Match>,
    pub state: ContinentBuildState<'gc>,
}

impl Continent {
    pub fn new(
        id: u32,
        name: String,
        countries: Vec<Country>,
        competition_configs: Vec<NationalCompetitionConfig>,
    ) -> Self {
        Continent {
            id,
            name,
            countries,
            continental_competitions: ContinentalCompetitions::new(),
            continental_rankings: ContinentalRankings::new(),
            regulations: ContinentalRegulations::new(),
            economic_zone: EconomicZone::new(),
            national_team_competitions: NationalTeamCompetitions::new(competition_configs),
        }
    }

    /// True when this continent hosts the UEFA club competitions
    /// (Champions / Europa / Conference League).
    /// How far ahead a league looks when it rearranges a club's domestic
    /// fixtures around its continental nights.
    const CONTINENTAL_HORIZON_DAYS: i64 = 21;

    /// Every country rearranges its clubs' domestic fixtures for rest —
    /// off their continental nights, and knockout ties off each side's other
    /// fixtures — before anybody names a team for them. Ties just played still count: a
    /// Sunday game two days after one is no rest.
    fn rearrange_for_rest(&mut self, today: NaiveDate) {
        let commitments = self.continental_competitions.commitments(
            today - Duration::days(FixtureRest::MIN_REST_DAYS),
            today + Duration::days(Self::CONTINENTAL_HORIZON_DAYS),
        );
        self.countries
            .par_iter_mut()
            .for_each(|country| country.rearrange_for_rest(&commitments, today));
    }

    pub fn is_europe(&self) -> bool {
        self.id == CONTINENT_EUROPE_ID || self.name == "Europe"
    }

    /// True when this continent hosts the CONMEBOL club competition
    /// (Copa Libertadores).
    pub fn is_south_america(&self) -> bool {
        self.id == CONTINENT_SOUTH_AMERICA_ID || self.name == "South America"
    }

    /// Build-only matchday simulation. Walks every country in
    /// parallel, has each one call [`Country::simulate_build`] to
    /// emit today's `Match::make` objects (league fixtures + cup
    /// ties) and a resume token, and packages the lot into a single
    /// [`ContinentBuildOutput`].
    ///
    /// NO `engine_pool().play(..)` runs here. Match dispatch is the
    /// responsibility of [`WorldMatchdayResult::process`][crate::WorldMatchdayResult::process]:
    /// the simulator collects every continent's `ContinentBuildOutput`,
    /// flattens every continent's matches into one global Vec, and
    /// dispatches as a single collection. The distributed worker
    /// fleet then sees one fan-out per tick instead of one per
    /// continent — small continents stop dispatching half-empty
    /// batches, big continents stop pinning slow workers.
    pub fn simulate<'gc>(
        &mut self,
        ctx: GlobalContext<'gc>,
        world: WorldSnapshot<'_>,
    ) -> ContinentBuildOutput<'gc> {
        let continent_name = self.name.clone();
        debug!(
            "Building matchday for continent: {} ({} countries)",
            continent_name,
            self.countries.len()
        );

        self.rearrange_for_rest(ctx.simulation.date.date());

        // National-team competition matches and the related call-up /
        // release flow run at the world level (see
        // `SimulatorData::simulate_with` and
        // `national_pipeline::simulate_world_national_competitions`) so
        // squads can include foreign-based players and stat updates can
        // span continents.

        // Build per-country GlobalContext once and reuse across the
        // build and process passes. `with_country_and_names` clones
        // each country's name generator (a few hundred KB on big
        // catalogues), so building it twice per tick — once per phase
        // — adds up over the world. One alloc per country per tick is
        // unavoidable, two isn't. The clones are independent per country,
        // so fan them out: on a big continent (many countries) this
        // serial prologue used to be the ramp that stalled the continent's
        // worker before its inner `countries.par_iter_mut()` below could
        // widen. `par_iter` collect preserves country order.
        // How far off the next major tournament is, worked out once for
        // the whole confederation. Every country on it is heading for the
        // same calendar, and the cycle arithmetic is the same walk for
        // each — so it belongs here rather than inside the fan-out.
        let tournament_clock = self
            .national_team_competitions
            .months_to_next_tournament(ctx.simulation.date.date());

        // …and stamped on the country itself as well as on its context.
        // The transfer market runs outside `GlobalContext`, and the
        // player-side appraisal needs the same clock the mind reads.
        let world_clocks = ctx.simulation.tournament_clocks.clone();
        let home_leagues = ctx.simulation.home_leagues.clone();
        self.countries.par_iter_mut().for_each(|c| {
            c.months_to_tournament = tournament_clock;
            // …and every OTHER confederation's alongside it, so a
            // foreigner in this league is priced against the tournament HE
            // might play in (C7). An `Arc` clone, not a map copy.
            c.tournament_clocks = world_clocks.clone();
            // …and what every country's best league is worth, so the
            // loan-out sweep can ask "is HIS league worth going home to?"
            // about a passport. Same `Arc` clone.
            c.home_leagues = home_leagues.clone();
        });

        let country_ctxs: Vec<GlobalContext<'gc>> = self
            .countries
            .par_iter()
            .map(|c| {
                ctx.with_country_and_names(
                    c.id,
                    c.code.clone(),
                    c.generator_data.people_names.clone(),
                    c.season_dates(),
                )
                .with_country_tournament_clock(tournament_clock)
            })
            .collect();

        // Parallel build across this continent's countries. Each call
        // prepares its leagues / cup schedules and hands back a
        // Vec<Match> ready to play plus a CountryPendingState to
        // resume with in the process pass.
        let builds: Vec<(Vec<Match>, CountryPendingState)> = self
            .countries
            .par_iter_mut()
            .zip(country_ctxs.par_iter())
            .map(|(country, country_ctx)| {
                let stage = PerformanceProfiler::stage_scope("country_build", 1)
                    .labelled(|| country.name.clone());
                let output = country.simulate_build(country_ctx, world);
                drop(stage);
                output
            })
            .collect();

        // Flatten per-country matches into one continent-local batch
        // with parallel range bookkeeping so the simulator's global
        // dispatch can slice results back per country without
        // re-grouping.
        let mut matches: Vec<Match> = Vec::new();
        let mut country_pending: Vec<CountryPendingState> = Vec::with_capacity(builds.len());
        let mut country_ranges: Vec<Range<usize>> = Vec::with_capacity(builds.len());
        for (m, p) in builds {
            let start = matches.len();
            matches.extend(m);
            country_ranges.push(start..matches.len());
            country_pending.push(p);
        }

        ContinentBuildOutput {
            continent_id: self.id,
            continent_name,
            matches,
            state: ContinentBuildState {
                country_ctxs,
                country_pending,
                country_ranges,
            },
        }
    }

    /// This continent's countries, each with its share of the day's
    /// results — the post-dispatch half of the tick, ready to run in any
    /// order. `results` is this continent's slice of the global batch, in
    /// build order; consecutive ranges move every result exactly once.
    pub fn country_work<'a, 'gc>(
        &'a mut self,
        state: ContinentBuildState<'gc>,
        results: Vec<MatchResult>,
    ) -> Vec<CountryWork<'a, 'gc>> {
        let mut played = results.into_iter();
        self.countries
            .iter_mut()
            .zip(state.country_ctxs)
            .zip(state.country_pending)
            .zip(state.country_ranges)
            .map(|(((country, ctx), pending), range)| CountryWork {
                country,
                ctx,
                pending,
                results: played.by_ref().take(range.len()).collect(),
            })
            .collect()
    }
}

#[cfg(test)]
mod continental_night_tests {
    use super::*;
    use crate::context::SimulationContext;
    use crate::country::core::country::fixture_calendar_tests::september_country;
    use crate::transfers::market::PlacementReachIndex;
    use crate::transfers::market::map::MarketMap;
    use crate::transfers::view::world::WorldPool;
    use rustc_hash::FxHashMap;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn a_europa_league_club_plays_its_league_game_on_the_sunday_after_its_thursday_tie() {
        let mut continent =
            Continent::new(1, "Europe".into(), vec![september_country()], Vec::new());

        // Club 100 (first team 10) is drawn into the Europa League with 31
        // clubs from elsewhere. Matchday one's nominal date is Saturday the
        // 19th — the same day as its league round.
        let field: Vec<u32> = std::iter::once(100).chain(9_001..=9_031).collect();
        continent
            .continental_competitions
            .europa_league
            .conduct_draw(&field, &ContinentalRankings::new(), d(8, 20));
        let nights: Vec<NaiveDate> = continent
            .continental_competitions
            .commitments(d(9, 12), d(9, 27))
            .dates(100)
            .to_vec();
        assert_eq!(nights, vec![d(9, 17)], "matchday one is Thursday the 17th");

        let country_info = FxHashMap::default();
        let market_map = MarketMap::default();
        let placement_reach = PlacementReachIndex::default();
        let mut first_team_days = Vec::new();
        for day in 12..=27 {
            let date = d(9, day).and_hms_opt(0, 0, 0).unwrap();
            let world = WorldSnapshot {
                date,
                country_info: &country_info,
                indexes: None,
                world_pool: WorldPool::empty(),
                global_free_agents: &[],
                market_map: &market_map,
                placement_reach: &placement_reach,
            };
            let ctx = GlobalContext::new(SimulationContext::new(date)).with_continent(1);
            let output = continent.simulate(ctx, world);
            if output
                .matches
                .iter()
                .any(|m| m.home_squad.team_id == 10 || m.away_squad.team_id == 10)
            {
                first_team_days.push(d(9, day));
            }
        }

        assert_eq!(
            first_team_days,
            vec![d(9, 12), d(9, 20), d(9, 26)],
            "the league game after the Thursday tie is played on Sunday"
        );

        let mut calendar: Vec<NaiveDate> = first_team_days.iter().chain(&nights).copied().collect();
        calendar.sort_unstable();
        for pair in calendar.windows(2) {
            assert!(
                (pair[1] - pair[0]).num_days() >= FixtureRest::MIN_REST_DAYS,
                "{} and {} are too close",
                pair[0],
                pair[1]
            );
        }
    }
}
