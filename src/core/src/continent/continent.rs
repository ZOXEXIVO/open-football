use crate::context::GlobalContext;
use crate::continent::national_competitions::{NationalCompetitionConfig, NationalTeamCompetitions};
use crate::continent::ContinentResult;
use crate::continent::{
    ContinentalCompetitions, ContinentalRankings, ContinentalRegulations, EconomicZone,
};
use crate::country::CountryResult;
use crate::utils::Logging;
use crate::country::national_team::MIN_REPUTATION_FOR_FRIENDLIES;
use crate::{Country, NationalTeam};
use chrono::NaiveDate;
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
        let date = ctx.simulation.date.date();

        debug!(
            "Simulating continent: {} ({} countries)",
            continent_name,
            self.countries.len()
        );

        // Phase 0: National team competition matches (full engine via pool)
        let national_match_results = self.simulate_national_competitions(date);

        // Phase 1: National team call-ups (needs cross-country visibility)
        self.process_national_team_callups(date);

        // Phase 2: Country simulation (parallel — no cross-country dependencies)
        let country_results = self.simulate_countries(&ctx);

        debug!("Continent {} simulation complete", continent_name);

        ContinentResult::new(self.id, country_results, national_match_results)
    }

    /// National team call-ups run at the continent level because players of
    /// nationality X may play at clubs in country Y. The continent has
    /// visibility across all countries so it can scan all clubs at once.
    fn process_national_team_callups(&mut self, date: NaiveDate) {
        let need_callups = NationalTeam::is_break_start(date) || NationalTeam::is_tournament_start(date);
        let need_release = NationalTeam::is_break_end(date) || NationalTeam::is_tournament_end(date);

        if !need_callups && !need_release {
            return;
        }

        let country_ids: Vec<(u32, String)> = self.countries.iter()
            .map(|c| (c.id, c.name.clone()))
            .collect();

        if need_callups {
            // Scan all clubs across all countries for eligible players
            let mut candidates_by_country =
                NationalTeam::collect_all_candidates_by_country(&self.countries, date);

            for country in &mut self.countries {
                if country.reputation < MIN_REPUTATION_FOR_FRIENDLIES {
                    continue;
                }

                country.national_team.country_name = country.name.clone();
                country.national_team.reputation = country.reputation;

                let candidates = candidates_by_country.remove(&country.id)
                    .unwrap_or_default();
                country.national_team.call_up_squad(
                    &mut country.clubs, candidates, date, country.id, &country_ids,
                );
            }
        }

        if need_release {
            for country in &mut self.countries {
                if !country.national_team.squad.is_empty() {
                    country.national_team.release_player_status(&mut country.clubs);
                }
            }
        }
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
