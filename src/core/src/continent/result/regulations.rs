use super::ContinentResult;
use crate::simulator::SimulatorData;
use chrono::{Datelike, NaiveDate};
use log::{debug, info};

impl ContinentResult {
    pub(crate) fn update_continental_regulations(&self, data: &mut SimulatorData, date: NaiveDate) {
        info!("📋 Updating continental regulations");

        let continent_id = self.get_continent_id(data);

        if let Some(continent) = data.continent_mut(continent_id) {
            // Financial Fair Play adjustments
            continent.regulations.update_ffp_thresholds(&continent.economic_zone);

            // Foreign player regulations
            continent.regulations.review_foreign_player_rules(&continent.continental_rankings);

            // Youth development requirements
            continent.regulations.update_youth_requirements();

            debug!("Continental regulations updated for year {}", date.year());
        }
    }
}
