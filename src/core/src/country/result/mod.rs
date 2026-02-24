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

        // Phase 3: Pre-season activities (if applicable)
        if Self::is_preseason(current_date) {
            self.simulate_preseason_activities(data, country_id, current_date);
        }

        // Phase 4: Transfer Market Activities
        let _transfer_activities = self.simulate_transfer_market(data, country_id, current_date);

        // Phase 5: International Competitions
        self.simulate_international_competitions(data, country_id, current_date);

        // Phase 6: Economic Updates
        self.update_economic_factors(data, country_id, current_date);

        // Phase 7: Media and Public Interest
        self.simulate_media_coverage(data, country_id, &self.leagues);

        // Phase 8: End of Period Processing
        self.process_end_of_period(data, country_id, current_date, &self.clubs);

        // Phase 9: Country Reputation Update
        self.update_country_reputation(data, country_id, &self.leagues, &self.clubs);

        // Phase 1: Process league results
        let any_new_season = self.leagues.iter().any(|l| l.new_season_started);

        for league_result in self.leagues {
            league_result.process(data, result);
        }

        // Snapshot player statistics when new season starts (all match stats are now up-to-date)
        if any_new_season {
            Self::snapshot_player_season_statistics(data, self.country_id);
        }

        // Phase 2: Process club results
        for club_result in self.clubs {
            club_result.process(data, result);
        }
    }

    fn get_country_id(&self, _data: &SimulatorData) -> u32 {
        self.country_id
    }
}
