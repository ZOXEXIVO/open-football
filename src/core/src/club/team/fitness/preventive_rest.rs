//! Weekly preventive-rest pass.
//!
//! An elite sports-science department flags players whose fatigue/jadedness
//! profile predicts an imminent injury and sets the `Rst` status — a hint
//! that the squad selector treats as "don't pick this week unless
//! emergency". A bare-bones medical team can't do this.
//!
//! Thresholds are tuned so that with neutral (0.35-ish) sports science the
//! pass flags no one, and with elite (0.85+) it flags the worst offenders
//! before they hit the danger zone.

use crate::{Player, PlayerStatusType};
use chrono::NaiveDate;

/// Below this sports-science rating the medical team has no predictive
/// power and finds out about injuries when they actually happen.
const MIN_SPORTS_SCIENCE_FOR_PREDICTION: u8 = 12;

pub struct PreventiveRestPass;

impl PreventiveRestPass {
    /// Run the preventive-rest pass against the senior squad.
    pub fn apply(players: &mut [Player], best_sports_sci: u8, date: NaiveDate) {
        if best_sports_sci < MIN_SPORTS_SCIENCE_FOR_PREDICTION {
            return;
        }

        let (jaded_gate, condition_gate) = Self::gates_for(best_sports_sci);

        for player in players.iter_mut() {
            if player.player_attributes.is_injured {
                continue;
            }
            let statuses = player.statuses.get();
            let already_resting = statuses.contains(&PlayerStatusType::Rst);

            let needs_rest = player.player_attributes.jadedness >= jaded_gate
                || player.player_attributes.condition_percentage() < condition_gate;

            if needs_rest && !already_resting {
                player.statuses.add(date, PlayerStatusType::Rst);
            } else if !needs_rest && already_resting {
                player.statuses.remove(PlayerStatusType::Rst);
            }
        }
    }

    /// (jaded_gate, condition_gate) tuned to sports-science quality.
    /// SS 12 → only the most extreme cases flagged; SS 20 → anyone with
    /// moderately elevated load gets rested.
    fn gates_for(best_sports_sci: u8) -> (i16, u32) {
        let jaded_gate: i16 = match best_sports_sci {
            12..=13 => 8500,
            14..=15 => 7800,
            16..=17 => 7000,
            _ => 6200,
        };
        let condition_gate: u32 = match best_sports_sci {
            12..=13 => 55,
            14..=15 => 60,
            16..=17 => 65,
            _ => 70,
        };
        (jaded_gate, condition_gate)
    }
}
