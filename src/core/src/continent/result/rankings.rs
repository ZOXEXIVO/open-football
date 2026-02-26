use super::ContinentResult;
use crate::continent::{ContinentalCompetitions, ContinentalRankings};
use crate::simulator::SimulatorData;
use crate::{Club, Country, SimulationResult};
use log::{debug, info};

impl ContinentResult {
    pub(crate) fn update_continental_rankings(&self, data: &mut SimulatorData, _result: &mut SimulationResult) {
        debug!("📊 Updating continental rankings");

        // Get continent from data
        let continent_id = self.get_continent_id();

        if let Some(continent) = data.continent_mut(continent_id) {
            // Update country coefficients based on club performances
            for country in &mut continent.countries {
                let coefficient = Self::calculate_country_coefficient(country, &continent.continental_competitions);
                continent.continental_rankings.update_country_ranking(country.id, coefficient);
            }

            // Update club rankings
            let all_clubs = Self::get_all_clubs(&continent.countries);
            for club in all_clubs {
                let club_points = Self::calculate_club_continental_points(club, &continent.continental_competitions);
                continent.continental_rankings.update_club_ranking(club.id, club_points);
            }

            // Determine continental competition qualifications
            Self::determine_competition_qualifications(&mut continent.continental_rankings);

            debug!(
                "Continental rankings updated - Top country: {:?}",
                continent.continental_rankings.get_top_country()
            );
        }
    }

    fn calculate_country_coefficient(country: &Country, competitions: &ContinentalCompetitions) -> f32 {
        let mut coefficient = 0.0;

        for club in &country.clubs {
            coefficient += competitions.get_club_points(club.id);
        }

        if !country.clubs.is_empty() {
            coefficient /= country.clubs.len() as f32;
        }

        coefficient
    }

    fn calculate_club_continental_points(club: &Club, competitions: &ContinentalCompetitions) -> f32 {
        let competition_points = competitions.get_club_points(club.id);
        let domestic_bonus = 0.0; // Would need league standings
        competition_points + domestic_bonus
    }

    fn determine_competition_qualifications(rankings: &mut ContinentalRankings) {
        // Collect country rankings data first to avoid borrow conflicts
        let country_rankings: Vec<(u32, f32)> = rankings.get_country_rankings().to_vec();

        // Now we can mutably borrow rankings without conflicts
        for (rank, (country_id, _coefficient)) in country_rankings.iter().enumerate() {
            let cl_spots = match rank {
                0..=3 => 4,
                4..=5 => 3,
                6..=14 => 2,
                _ => 1,
            };

            let el_spots = match rank {
                0..=5 => 2,
                _ => 1,
            };

            rankings.set_qualification_spots(*country_id, cl_spots, el_spots);
        }
    }

    fn get_all_clubs(countries: &[Country]) -> Vec<&Club> {
        countries.iter().flat_map(|c| &c.clubs).collect()
    }
}
