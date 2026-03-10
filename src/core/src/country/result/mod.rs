mod end_of_period;
mod preseason;
mod reputation;
mod statistics;
mod transfers;

use crate::league::LeagueResult;
use crate::simulator::SimulatorData;
use crate::{ClubResult, SimulationResult};

pub struct CountryResult {
    pub country_id: u32,
    pub leagues: Vec<LeagueResult>,
    pub clubs: Vec<ClubResult>,
}

impl CountryResult {
    pub fn new(country_id: u32, leagues: Vec<LeagueResult>, clubs: Vec<ClubResult>) -> Self {
        CountryResult { country_id, leagues, clubs }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        let current_date = data.date.date();
        let country_id = self.get_country_id(data);

        let season_dates = data.country(country_id)
            .map(|c| c.season_dates())
            .unwrap_or_default();

        // Phases that need &self.leagues / &self.clubs references run BEFORE the consuming loops
        Self::simulate_media_coverage(data, country_id, &self.leagues);
        Self::process_end_of_period(data, country_id, current_date, &self.clubs);
        Self::update_country_reputation(data, country_id, &self.leagues, &self.clubs);

        // Phase 1: Process league results — apply match stats, injuries, condition
        // before transfers or club sim act on player state
        let any_new_season = self.leagues.iter().any(|l| l.new_season_started);

        for league_result in self.leagues {
            league_result.process(data, result);
        }

        // Snapshot player statistics when new season starts (all match stats are now up-to-date)
        if any_new_season {
            Self::snapshot_player_season_statistics(data, self.country_id);
        }

        // Phase 2: Process club results (morale, training use post-match state)
        // Collect academy transfers before consuming club results
        let mut academy_transfers = Vec::new();
        for club in &self.clubs {
            if !club.academy_transfers.is_empty() {
                academy_transfers.extend(club.academy_transfers.iter().cloned());
            }
        }

        for club_result in self.clubs {
            club_result.process(data, result);
        }

        // Push academy graduation transfers to country transfer history
        if !academy_transfers.is_empty() {
            if let Some(country) = data.country_mut(self.country_id) {
                for transfer in academy_transfers {
                    country.transfer_market.transfer_history.push(transfer);
                }
            }
        }

        // Phase 2.5: Process loan returns — runs AFTER club results so that
        // ClubResult player references (contract proposals etc.) are fully processed
        // before players are moved between clubs
        Self::process_loan_returns(data, country_id, current_date);

        // Phase 3: Pre-season activities (if applicable)
        if season_dates.is_off_season(current_date) {
            Self::simulate_preseason_activities(data, country_id, current_date);
        }

        // Phase 4: Transfer Market Activities (runs with up-to-date player state,
        // after match results have been applied)
        let _transfer_activities = Self::simulate_transfer_market(data, country_id, current_date);

        // Phase 5: International Competitions
        Self::simulate_international_competitions(data, country_id, current_date);

        // Phase 6: Economic Updates
        Self::update_economic_factors(data, country_id, current_date);
    }

    fn get_country_id(&self, _data: &SimulatorData) -> u32 {
        self.country_id
    }
}
