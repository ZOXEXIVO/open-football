use crate::Country;
use crate::MatchRuntime;
use crate::context::GlobalContext;
use crate::continent::ContinentResult;
use crate::continent::national::{NationalCompetitionConfig, NationalTeamCompetitions};
use crate::continent::{
    ContinentalCompetitions, ContinentalRankings, ContinentalRegulations, EconomicZone,
};
use crate::country::{CountryPendingState, CountryResult};
use crate::league::result::WorldSnapshot;
use crate::r#match::{Match, MatchResult};
use crate::utils::Logging;
use log::{debug, info};
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use rayon::prelude::{IntoParallelRefIterator, IntoParallelRefMutIterator};
use std::ops::Range;

/// Reserved continent ids used to scope continental club competitions.
/// UEFA competitions only run in Europe; Copa Libertadores only runs in
/// South America. The id is the primary check; the name is a readable
/// fallback for data sets that haven't pinned the canonical id.
pub const CONTINENT_EUROPE_ID: u32 = 1;
pub const CONTINENT_SOUTH_AMERICA_ID: u32 = 3;

#[derive(Clone)]
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
    pub fn is_europe(&self) -> bool {
        self.id == CONTINENT_EUROPE_ID || self.name == "Europe"
    }

    /// True when this continent hosts the CONMEBOL club competition
    /// (Copa Libertadores).
    pub fn is_south_america(&self) -> bool {
        self.id == CONTINENT_SOUTH_AMERICA_ID || self.name == "South America"
    }

    pub fn simulate(
        &mut self,
        ctx: GlobalContext<'_>,
        world: WorldSnapshot<'_>,
    ) -> ContinentResult {
        let continent_name = self.name.clone();

        debug!(
            "Simulating continent: {} ({} countries)",
            continent_name,
            self.countries.len()
        );

        // National-team competition matches and the related call-up /
        // release flow now run at the world level (see
        // `SimulatorData::simulate_with` and
        // `national_pipeline::simulate_world_national_competitions`) so
        // squads can include foreign-based players and stat updates can
        // span continents. The continent's parallel pass is country
        // simulation only — split into three sub-phases so club-match
        // play can fan out as one continent-wide dispatch.

        // Build per-country GlobalContext once and reuse across the
        // build and process passes. `with_country_and_names` clones
        // each country's name generator (a few hundred KB on big
        // catalogues), so building it twice per tick — once per phase
        // — adds up over the world. One alloc per country per tick is
        // unavoidable, two isn't.
        let country_ctxs: Vec<GlobalContext<'_>> = self
            .countries
            .iter()
            .map(|c| {
                ctx.with_country_and_names(
                    c.id,
                    c.code.clone(),
                    c.generator_data.people_names.clone(),
                    c.season_dates(),
                )
            })
            .collect();

        // Phase A: parallel build. Each country prepares its leagues /
        // cup schedules and hands back a Vec<Match> ready to play plus
        // a CountryPendingState to resume with in Phase C.
        let builds: Vec<(Vec<Match>, CountryPendingState)> = self
            .countries
            .par_iter_mut()
            .zip(country_ctxs.par_iter())
            .map(|(country, country_ctx)| country.simulate_build(country_ctx, world))
            .collect();

        // Phase B: drain all per-country matches into one continent
        // batch and dispatch in a single call. With external workers
        // this is the whole point of the split — a 30-match per-league
        // batch becomes a 600-match per-continent batch, and the
        // dispatcher's round-robin spreads it across the whole fleet
        // instead of pinning each league to one worker.
        let mut all_matches: Vec<Match> = Vec::new();
        let mut pending_states: Vec<CountryPendingState> = Vec::with_capacity(builds.len());
        let mut ranges: Vec<Range<usize>> = Vec::with_capacity(builds.len());
        for (matches, pending) in builds {
            let start = all_matches.len();
            all_matches.extend(matches);
            ranges.push(start..all_matches.len());
            pending_states.push(pending);
        }
        let total = all_matches.len();
        let all_results: Vec<MatchResult> = if total == 0 {
            Vec::new()
        } else {
            info!(
                "continent {}: dispatching {} matches in one batch",
                continent_name, total
            );
            MatchRuntime::engine_pool().play(all_matches)
        };
        let per_country_results: Vec<Vec<MatchResult>> = ranges
            .iter()
            .map(|r| all_results[r.clone()].to_vec())
            .collect();

        // Phase C: parallel process. Each country routes its slice of
        // results back to its leagues / cup, then runs the rest of the
        // country tick (clubs, transfers, result fan-out).
        let country_results: Vec<CountryResult> = self
            .countries
            .par_iter_mut()
            .zip(country_ctxs.into_par_iter())
            .zip(pending_states.into_par_iter())
            .zip(per_country_results.into_par_iter())
            .map(|(((country, country_ctx), pending), results)| {
                let message = format!("simulate country: {}", &country.name);
                Logging::estimate_result(
                    || country.simulate_process(country_ctx, world, pending, results),
                    &message,
                )
            })
            .collect();

        debug!("Continent {} simulation complete", continent_name);

        ContinentResult::new(self.id, country_results, Vec::new())
    }
}
