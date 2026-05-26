use crate::Country;
use crate::context::GlobalContext;
use crate::continent::ContinentResult;
use crate::continent::national::{NationalCompetitionConfig, NationalTeamCompetitions};
use crate::continent::{
    ContinentalCompetitions, ContinentalRankings, ContinentalRegulations, EconomicZone,
};
use crate::country::CountryResult;
use crate::league::result::WorldSnapshot;
use crate::utils::Logging;
use log::debug;
use rayon::iter::ParallelIterator;
use rayon::prelude::IntoParallelRefMutIterator;

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
        // span continents. The continent's parallel pass is now country
        // simulation only.
        let country_results = self.simulate_countries(&ctx, world);

        debug!("Continent {} simulation complete", continent_name);

        ContinentResult::new(self.id, country_results, Vec::new())
    }

    fn simulate_countries(
        &mut self,
        ctx: &GlobalContext<'_>,
        world: WorldSnapshot<'_>,
    ) -> Vec<CountryResult> {
        self.countries
            .par_iter_mut()
            .map(|country| {
                let message = &format!("simulate country: {}", &country.name);
                Logging::estimate_result(
                    || {
                        country.simulate(
                            ctx.with_country_and_names(
                                country.id,
                                country.code.clone(),
                                country.generator_data.people_names.clone(),
                                country.season_dates(),
                            ),
                            world,
                        )
                    },
                    message,
                )
            })
            .collect()
    }
}
