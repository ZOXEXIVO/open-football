use super::CountryResult;
use crate::Country;

impl CountryResult {
    /// Off-season training camps, one day for every club player.
    pub(crate) fn simulate_preseason_activities(country: &mut Country) {
        for club in &mut country.clubs {
            let training_quality = club.facilities.training.multiplier();
            for team in &mut club.teams.teams {
                for player in &mut team.players.players {
                    player.on_offseason_camp_day(training_quality);
                }
            }
        }
    }
}
