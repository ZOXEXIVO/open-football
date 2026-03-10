use crate::context::GlobalContext;
use crate::continent::national_competitions::{NationalCompetitionConfig, NationalTeamCompetitions};
use crate::continent::ContinentResult;
use crate::continent::{
    ContinentalCompetitions, ContinentalRankings, ContinentalRegulations, EconomicZone,
};
use crate::country::{CallUpCandidate, CountryResult};
use crate::utils::Logging;
use crate::{Country, NationalTeam};
use log::{debug};
use rayon::prelude::IntoParallelRefMutIterator;
use rayon::iter::ParallelIterator;
use std::collections::HashMap;

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
        let date = ctx.simulation.date.date();

        debug!(
            "Simulating continent: {} ({} countries)",
            continent_name,
            self.countries.len()
        );
        
        // Phase 0: National team competition matches (parallel engine runs)
        //self.simulate_national_competitions(date);

        // Phase 0.5: International friendly matches (parallel engine runs)
        //self.simulate_international_friendlies(date);

        // Phase 1+: Simulate all child entities and accumulate results
        let country_results = self.simulate_countries(&ctx);

        debug!("Continent {} simulation complete", continent_name);

        ContinentResult::new(self.id, country_results)
    }

    fn simulate_countries(&mut self, ctx: &GlobalContext<'_>) -> Vec<CountryResult> {
        use rayon::iter::{IntoParallelIterator, IndexedParallelIterator};

        let country_ids: Vec<(u32, String)> = self.countries.iter().map(|c| (c.id, c.name.clone())).collect();
        let date = ctx.simulation.date.date();

        // Pre-collect national team candidates from ALL clubs across ALL countries
        let need_callups = NationalTeam::is_break_start(date) || NationalTeam::is_tournament_start(date);

        let mut candidates_by_country = if need_callups {
            NationalTeam::collect_all_candidates_by_country(&self.countries, date)
        } else {
            HashMap::new()
        };

        // Pre-extract candidates per country for parallel-safe access
        let candidates_vec: Vec<Option<Vec<CallUpCandidate>>> =
            self.countries.iter().map(|c| candidates_by_country.remove(&c.id)).collect();

        self.countries
            .par_iter_mut()
            .zip(candidates_vec.into_par_iter())
            .map(|(country, candidates)| {
                let message = &format!("simulate country: {} (Continental)", &country.name);
                Logging::estimate_result(
                    || country.simulate(ctx.with_country_and_names(country.id, country.generator_data.people_names.clone(), country.season_dates()), &country_ids, candidates),
                    message,
                )
            })
            .collect()
    }
}
