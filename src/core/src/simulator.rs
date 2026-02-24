use crate::context::{GlobalContext, SimulationContext};
use crate::continent::{Continent, ContinentResult};
use crate::competitions::GlobalCompetitions;
use crate::r#match::MatchResult;
use crate::shared::SimulatorDataIndexes;
use crate::transfers::TransferPool;
use crate::utils::Logging;
use crate::Player;
use chrono::{Duration, NaiveDateTime};

pub struct FootballSimulator;

impl FootballSimulator {
    pub fn simulate(data: &mut SimulatorData) -> SimulationResult {
        let mut result = SimulationResult::new();

        let current_data = data.date;

        Logging::estimate(
            || {
                let ctx = GlobalContext::new(SimulationContext::new(data.date));

                let results: Vec<ContinentResult> = data
                    .continents
                    .iter_mut()
                    .map(|continent| continent.simulate(ctx.with_continent(continent.id)))
                    .collect();

                for continent_result in results {
                    continent_result.process(data, &mut result);
                }

                // Global competitions: assembly + simulation + phase transitions
                let date = data.date.date();
                data.global_competitions.check_tournament_assembly(date, &data.continents);
                Self::simulate_global_competitions(data, date);
                data.global_competitions.check_phase_transitions();

                data.next_date();
            },
            &format!("simulate date {}", current_data),
        );

        result
    }

    fn simulate_global_competitions(data: &mut SimulatorData, date: chrono::NaiveDate) {
        use crate::NationalTeam;
        use log::info;
        use rayon::iter::{IntoParallelIterator, ParallelIterator};
        use std::collections::HashMap;

        let todays_matches = data.global_competitions.get_todays_matches(date);
        if todays_matches.is_empty() {
            return;
        }

        // Build squads - need to search across all continents
        let prepared: Vec<(usize, crate::r#match::MatchSquad, crate::r#match::MatchSquad)> = todays_matches
            .iter()
            .enumerate()
            .filter_map(|(idx, fixture)| {
                let home = Self::build_global_match_squad(&mut data.continents, fixture.home_country_id, date)?;
                let away = Self::build_global_match_squad(&mut data.continents, fixture.away_country_id, date)?;
                Some((idx, home, away))
            })
            .collect();

        // Run match engines in parallel
        let engine_results: Vec<(usize, u8, u8, HashMap<u32, u16>)> = prepared
            .into_par_iter()
            .map(|(idx, home_squad, away_squad)| {
                let (home_score, away_score, player_goals) =
                    NationalTeam::play_competition_match(home_squad, away_squad);
                (idx, home_score, away_score, player_goals)
            })
            .collect();

        // Apply results
        for (fixture_idx, home_score, away_score, _player_goals) in engine_results {
            let fixture = &todays_matches[fixture_idx];
            let home_country_id = fixture.home_country_id;
            let away_country_id = fixture.away_country_id;

            let penalty_winner = if fixture.phase.is_knockout() && home_score == away_score {
                let home_rep = Self::get_global_country_reputation(&data.continents, home_country_id);
                let away_rep = Self::get_global_country_reputation(&data.continents, away_country_id);
                if home_rep >= away_rep {
                    Some(home_country_id)
                } else {
                    Some(away_country_id)
                }
            } else {
                None
            };

            data.global_competitions.record_result(
                fixture,
                home_score,
                away_score,
                penalty_winner,
            );

            // Update Elo ratings across continents
            let away_elo = Self::get_global_country_elo(&data.continents, away_country_id);
            let home_elo = Self::get_global_country_elo(&data.continents, home_country_id);

            for continent in &mut data.continents {
                if let Some(country) = continent.countries.iter_mut().find(|c| c.id == home_country_id) {
                    country.national_team.update_elo(home_score, away_score, away_elo);
                }
                if let Some(country) = continent.countries.iter_mut().find(|c| c.id == away_country_id) {
                    country.national_team.update_elo(away_score, home_score, home_elo);
                }
            }

            let label = data.global_competitions
                .tournaments
                .get(fixture.tournament_idx)
                .map(|t| t.short_name())
                .unwrap_or("INT");

            let home_name = Self::get_global_country_name(&data.continents, home_country_id);
            let away_name = Self::get_global_country_name(&data.continents, away_country_id);

            info!(
                "Global competition ({}): {} vs {} - {}:{}",
                label, home_name, away_name, home_score, away_score
            );
        }
    }

    fn build_global_match_squad(
        continents: &mut [Continent],
        country_id: u32,
        date: chrono::NaiveDate,
    ) -> Option<crate::r#match::MatchSquad> {
        for continent in continents.iter_mut() {
            if continent.countries.iter().any(|c| c.id == country_id) {
                return continent.build_country_match_squad(country_id, date);
            }
        }
        None
    }

    fn get_global_country_reputation(continents: &[Continent], country_id: u32) -> u16 {
        continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == country_id)
            .map(|c| c.reputation)
            .unwrap_or(0)
    }

    fn get_global_country_elo(continents: &[Continent], country_id: u32) -> u16 {
        continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == country_id)
            .map(|c| c.national_team.elo_rating)
            .unwrap_or(1500)
    }

    fn get_global_country_name(continents: &[Continent], country_id: u32) -> String {
        continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == country_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("Country {}", country_id))
    }
}

pub struct SimulatorData {
    pub continents: Vec<Continent>,

    pub date: NaiveDateTime,

    pub transfer_pool: TransferPool<Player>,

    pub indexes: Option<SimulatorDataIndexes>,

    pub match_played: bool,

    pub watchlist: Vec<u32>,

    pub global_competitions: GlobalCompetitions,
}

impl SimulatorData {
    pub fn new(date: NaiveDateTime, continents: Vec<Continent>, global_competitions: GlobalCompetitions) -> Self {
        let mut data = SimulatorData {
            continents,
            date,
            transfer_pool: TransferPool::new(),
            indexes: None,
            match_played: false,
            watchlist: Vec::new(),
            global_competitions,
        };

        let mut indexes = SimulatorDataIndexes::new();

        indexes.refresh(&data);

        data.indexes = Some(indexes);

        data
    }

    pub fn next_date(&mut self) {
        self.date += Duration::days(1);
    }

}

pub struct SimulationResult {
    pub match_results: Vec<MatchResult>,
}

impl SimulationResult {
    pub fn new() -> Self {
        SimulationResult {
            match_results: Vec::new(),
        }
    }

    pub fn has_match_results(&self) -> bool {
        !self.match_results.is_empty()
    }
}
