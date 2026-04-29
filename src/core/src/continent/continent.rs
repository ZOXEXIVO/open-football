use crate::context::GlobalContext;
use crate::continent::national::{NationalCompetitionConfig, NationalTeamCompetitions};
use crate::continent::ContinentResult;
use crate::continent::{
    ContinentalCompetitions, ContinentalRankings, ContinentalRegulations, EconomicZone,
};
use crate::country::CountryResult;
use crate::utils::Logging;
use crate::Country;
use log::debug;
use rayon::prelude::IntoParallelRefMutIterator;
use rayon::iter::ParallelIterator;

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
    pub fn new(id: u32, name: String, countries: Vec<Country>, competition_configs: Vec<NationalCompetitionConfig>) -> Self {
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

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ContinentResult {
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
        let country_results = self.simulate_countries(&ctx);

        debug!("Continent {} simulation complete", continent_name);

        ContinentResult::new(self.id, country_results, Vec::new())
    }

    fn simulate_countries(&mut self, ctx: &GlobalContext<'_>) -> Vec<CountryResult> {
        self.countries
            .par_iter_mut()
            .map(|country| {
                let message = &format!("simulate country: {}", &country.name);
                Logging::estimate_result(
                    || country.simulate(ctx.with_country_and_names(country.id, country.code.clone(), country.generator_data.people_names.clone(), country.season_dates())),
                    message,
                )
            })
            .collect()
    }
}
