use chrono::{Datelike, NaiveDate};
use log::debug;
use super::CountryResult;
use crate::{ClubResult, Country};
use crate::league::LeagueResult;
use crate::simulator::SimulatorData;

impl CountryResult {
    pub(super) fn simulate_international_competitions(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        if let Some(country) = data.country_mut(country_id) {
            for competition in &mut country.international_competitions {
                competition.simulate_round(date);
            }
        }
    }

    pub(super) fn update_economic_factors(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        if date.day() == 1 {
            if let Some(country) = data.country_mut(country_id) {
                country.economic_factors.monthly_update();
            }
        }
    }

    pub(super) fn simulate_media_coverage(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        league_results: &[LeagueResult],
    ) {
        if let Some(country) = data.country_mut(country_id) {
            country.media_coverage.update_from_results(league_results);
            country.media_coverage.generate_weekly_stories(&country.clubs);
        }
    }

    pub(super) fn update_country_reputation(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        _league_results: &[LeagueResult],
        _club_results: &[ClubResult],
    ) {
        if let Some(country) = data.country_mut(country_id) {
            let mut reputation_change: i16 = 0;

            for league in &country.leagues.leagues {
                let competitiveness = Self::calculate_league_competitiveness(league);
                reputation_change += (competitiveness * 5.0) as i16;
            }

            let international_success = Self::calculate_international_success(country);
            reputation_change += international_success as i16;

            let transfer_reputation = Self::calculate_transfer_market_reputation(country);
            reputation_change += transfer_reputation as i16;

            let new_reputation = (country.reputation as i16 + reputation_change).clamp(0, 1000) as u16;

            if new_reputation != country.reputation {
                debug!(
                    "Country {} reputation changed: {} -> {} ({})",
                    country.name,
                    country.reputation,
                    new_reputation,
                    if reputation_change > 0 {
                        format!("+{}", reputation_change)
                    } else {
                        reputation_change.to_string()
                    }
                );
                country.reputation = new_reputation;
            }
        }
    }

    fn calculate_league_competitiveness(_league: &crate::league::League) -> f32 {
        0.5
    }

    fn calculate_international_success(_country: &Country) -> i16 {
        0
    }

    fn calculate_transfer_market_reputation(_country: &Country) -> i16 {
        0
    }
}
