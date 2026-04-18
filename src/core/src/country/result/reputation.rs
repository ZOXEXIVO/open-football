use chrono::NaiveDate;
use log::debug;
use super::CountryResult;
use crate::utils::DateUtils;
use crate::{ClubResult, Country};
use crate::league::LeagueResult;
use crate::simulator::SimulatorData;

impl CountryResult {
    pub(super) fn simulate_international_competitions(
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
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        if DateUtils::is_month_beginning(date) {
            if let Some(country) = data.country_mut(country_id) {
                country.economic_factors.monthly_update();
            }
        }
    }

    pub(super) fn simulate_media_coverage(
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

            let new_reputation = (country.reputation as i32 + reputation_change as i32).clamp(0, 10000) as u16;

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

    fn calculate_league_competitiveness(league: &crate::league::League) -> f32 {
        if league.table.rows.is_empty() {
            return 0.0;
        }

        let rows = &league.table.rows;
        let total = rows.len() as f32;
        if total < 2.0 {
            return 0.0;
        }

        // Measure point spread between top and bottom — tighter = more competitive
        let max_points = rows.iter().map(|r| r.points).max().unwrap_or(0) as f32;
        let min_points = rows.iter().map(|r| r.points).min().unwrap_or(0) as f32;

        if max_points <= 0.0 {
            return 0.0;
        }

        let spread = (max_points - min_points) / max_points;
        // spread ~0.3 = very competitive, spread ~0.8 = dominated
        // Map to -1.0 (bad) to 1.0 (good)
        (1.0 - spread * 2.0).clamp(-1.0, 1.0)
    }

    fn calculate_international_success(country: &Country) -> i16 {
        // Count clubs in continental competitions (approximated by having high world reputation)
        let high_rep_clubs = country.clubs.iter()
            .filter(|c| c.teams.teams.first()
                .map(|t| t.reputation.overall_score() >= 0.6)
                .unwrap_or(false))
            .count();

        match high_rep_clubs {
            0 => -2,
            1 => 0,
            2..=3 => 2,
            _ => 5,
        }
    }

    fn calculate_transfer_market_reputation(country: &Country) -> i16 {
        // Active transfer market with incoming signings boosts reputation
        let completed = country.transfer_market.transfer_history.len();
        match completed {
            0..=5 => -1,
            6..=20 => 0,
            21..=50 => 1,
            _ => 3,
        }
    }
}
