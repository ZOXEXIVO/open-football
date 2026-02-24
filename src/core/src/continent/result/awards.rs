use super::ContinentResult;
use crate::continent::Continent;
use crate::country::CountryResult;
use crate::simulator::SimulatorData;
use log::{debug, info};

impl ContinentResult {
    pub(crate) fn process_continental_awards(&self, data: &mut SimulatorData, _country_results: &[CountryResult]) {
        info!("🏆 Processing continental awards");

        let continent_id = self.get_continent_id(data);

        if let Some(continent) = data.continent(continent_id) {
            // Player of the Year
            let _player_of_year = Self::determine_player_of_year(continent);

            // Team of the Year
            let _team_of_year = Self::determine_team_of_year(continent);

            // Coach of the Year
            let _coach_of_year = Self::determine_coach_of_year(continent);

            // Young Player Award
            let _young_player = Self::determine_young_player_award(continent);

            debug!("Continental awards distributed");
        }
    }

    fn determine_player_of_year(_continent: &Continent) -> Option<u32> {
        None
    }

    fn determine_team_of_year(_continent: &Continent) -> Option<Vec<u32>> {
        None
    }

    fn determine_coach_of_year(_continent: &Continent) -> Option<u32> {
        None
    }

    fn determine_young_player_award(_continent: &Continent) -> Option<u32> {
        None
    }
}
