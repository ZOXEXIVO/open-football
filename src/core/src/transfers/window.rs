use crate::shared::CurrencyValue;
use crate::{Person, Player, PlayerValueCalculator};
use chrono::{Datelike, NaiveDate};
use std::collections::HashMap;

#[derive(Debug)]
pub struct TransferWindowManager {
    pub windows: HashMap<u32, TransferWindow>, // Keyed by country_id
}

#[derive(Debug, Clone)]
pub struct TransferWindow {
    pub summer_window: (NaiveDate, NaiveDate),
    pub winter_window: (NaiveDate, NaiveDate),
    pub country_id: u32,
}

impl TransferWindowManager {
    pub fn new() -> Self {
        TransferWindowManager {
            windows: HashMap::new(),
        }
    }

    pub fn add_window(&mut self, country_id: u32, window: TransferWindow) {
        self.windows.insert(country_id, window);
    }

    pub fn is_window_open(&self, country_id: u32, date: NaiveDate) -> bool {
        if let Some(window) = self.windows.get(&country_id) {
            self.is_date_in_window(date, &window.summer_window) ||
                self.is_date_in_window(date, &window.winter_window)
        } else {
            // Default to European standard windows if country-specific not defined
            let year = date.year();
            let summer_start = NaiveDate::from_ymd_opt(year, 6, 1).unwrap_or(date);
            let summer_end = NaiveDate::from_ymd_opt(year, 8, 31).unwrap_or(date);
            let winter_start = NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or(date);
            let winter_end = NaiveDate::from_ymd_opt(year, 1, 31).unwrap_or(date);

            self.is_date_in_window(date, &(summer_start, summer_end)) ||
                self.is_date_in_window(date, &(winter_start, winter_end))
        }
    }

    fn is_date_in_window(&self, date: NaiveDate, window: &(NaiveDate, NaiveDate)) -> bool {
        date >= window.0 && date <= window.1
    }
}

/// Generates appropriate values for players based on multiple factors
pub struct PlayerValuationCalculator;

impl PlayerValuationCalculator {
    pub fn calculate_value(player: &Player, date: NaiveDate) -> CurrencyValue {
        // Use the existing calculator as a base
        let base_value = PlayerValueCalculator::calculate(player, date);

        // Apply market modifiers
        let market_adjusted_value = Self::apply_market_factors(base_value, player, date);

        CurrencyValue {
            amount: market_adjusted_value,
            currency: crate::shared::Currency::Usd,
        }
    }

    fn apply_market_factors(base_value: f64, player: &Player, date: NaiveDate) -> f64 {
        let mut adjusted_value = base_value;

        // Contract length factor - extremely important
        if let Some(contract) = &player.contract {
            let days_remaining = contract.days_to_expiration(date.and_hms_opt(0, 0, 0).unwrap());
            let years_remaining = days_remaining as f64 / 365.0;

            if years_remaining < 0.5 {
                // Less than 6 months - massive devaluation
                adjusted_value *= 0.3;
            } else if years_remaining < 1.0 {
                // Less than a year - significant devaluation
                adjusted_value *= 0.6;
            } else if years_remaining < 2.0 {
                // 1-2 years - moderate devaluation
                adjusted_value *= 0.8;
            } else if years_remaining > 4.0 {
                // Long contract - slight value increase
                adjusted_value *= 1.1;
            }
        }

        // Player age factor (already included in base calculator but we can fine-tune)
        let age = player.age(date);
        if age < 23 {
            // Young players with potential
            adjusted_value *= 1.2;
        } else if age > 32 {
            // Older players
            adjusted_value *= 0.7;
        }

        // Recent performance factor
        let goals = player.statistics.goals;
        let assists = player.statistics.assists;
        let played = player.statistics.played;

        if played > 10 {
            // Had significant playing time
            let goals_per_game = goals as f64 / played as f64;
            let assists_per_game = assists as f64 / played as f64;

            // Attackers valued by goals
            if player.position().is_forward() && goals_per_game > 0.5 {
                adjusted_value *= 1.0 + (goals_per_game - 0.5) * 2.0;
            }

            // Midfielders valued by combined contribution
            if player.position().is_midfielder() && (goals_per_game + assists_per_game) > 0.4 {
                adjusted_value *= 1.0 + ((goals_per_game + assists_per_game) - 0.4) * 1.5;
            }
        }

        // International status
        if player.player_attributes.international_apps > 10 {
            adjusted_value *= 1.2;
        }

        // Apply position factor (goalkeepers and defenders typically valued less)
        if player.position().is_goalkeeper() {
            adjusted_value *= 0.8;
        }

        adjusted_value
    }
}