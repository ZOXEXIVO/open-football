use super::CountryResult;
use crate::Country;
use crate::simulator::SimulatorData;
use chrono::NaiveDate;

impl CountryResult {
    pub(super) fn simulate_preseason_activities(
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        if let Some(country) = data.country_mut(country_id) {
            Self::run_training_camps(country, date);
            Self::run_preseason_conditioning(country, date);
        }
    }

    /// Training camps: boost player condition and match readiness during off-season.
    /// Simulates daily training sessions that prepare players for the new season.
    fn run_training_camps(country: &mut Country, _date: NaiveDate) {
        for club in &mut country.clubs {
            let training_quality = club.facilities.training.multiplier();

            for team in &mut club.teams.teams {
                for player in &mut team.players.players {
                    if player.player_attributes.is_injured {
                        continue;
                    }

                    // Match readiness recovers faster with better facilities.
                    // Scale is 0-20, not 0-100 — older code mis-capped at
                    // 100 which let preseason readiness silently overrun.
                    let readiness_gain = 0.3 + training_quality * 0.4;
                    player.skills.physical.match_readiness =
                        (player.skills.physical.match_readiness + readiness_gain).min(20.0);

                    // Stamina recovery during off-season rest
                    let fitness = player.skills.physical.natural_fitness;
                    let stamina_gain = 0.01 + (fitness / 20.0) * 0.02;
                    player.skills.physical.stamina =
                        (player.skills.physical.stamina + stamina_gain).min(20.0);
                }
            }
        }
    }

    /// Pre-season conditioning: mental and technical sharpness recovery.
    /// Players regain focus and technique through structured practice sessions.
    fn run_preseason_conditioning(country: &mut Country, _date: NaiveDate) {
        for club in &mut country.clubs {
            for team in &mut club.teams.teams {
                for player in &mut team.players.players {
                    if player.player_attributes.is_injured {
                        continue;
                    }

                    // Mental freshness restoration
                    player.skills.mental.rest();
                    // Technical sharpness
                    player.skills.technical.rest();
                }
            }
        }
    }
}
