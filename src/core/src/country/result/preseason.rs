use chrono::NaiveDate;
use log::debug;
use super::CountryResult;
use crate::Country;
use crate::simulator::SimulatorData;

impl CountryResult {
    pub(super) fn simulate_preseason_activities(data: &mut SimulatorData, country_id: u32, date: NaiveDate) {
        debug!("Running preseason activities...");

        if let Some(country) = data.country_mut(country_id) {
            Self::schedule_friendly_matches(country, date);
            Self::organize_training_camps(country);
            Self::organize_preseason_tournaments(country);
        }
    }

    fn schedule_friendly_matches(_country: &mut Country, _date: NaiveDate) {
        debug!("Scheduling preseason friendlies");
    }

    fn organize_training_camps(_country: &mut Country) {
        debug!("Organizing training camps");
    }

    fn organize_preseason_tournaments(_country: &mut Country) {
        debug!("Organizing preseason tournaments");
    }
}
