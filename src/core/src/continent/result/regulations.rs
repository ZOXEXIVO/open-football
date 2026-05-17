use super::ContinentResult;
use crate::continent::Continent;
use chrono::{Datelike, NaiveDate};
use log::debug;

impl ContinentResult {
    /// Continent-local: refresh FFP thresholds, foreign-player rules and
    /// youth requirements once per year. Takes `&mut Continent` so the
    /// orchestrator can run this in parallel across continents.
    pub(crate) fn update_continental_regulations(continent: &mut Continent, date: NaiveDate) {
        debug!("📋 Updating continental regulations");

        // Financial Fair Play adjustments
        continent
            .regulations
            .update_ffp_thresholds(&continent.economic_zone);

        // Foreign player regulations
        continent
            .regulations
            .review_foreign_player_rules(&continent.continental_rankings);

        // Youth development requirements
        continent.regulations.update_youth_requirements();

        debug!("Continental regulations updated for year {}", date.year());
    }
}
