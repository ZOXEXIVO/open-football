use crate::shared::CurrencyValue;
use crate::{Player, PlayerValueCalculator};
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

/// Transfer-market-specific player valuation.
/// Wraps `PlayerValueCalculator` with market conditions (selling pressure, squad role).
pub struct PlayerValuationCalculator;

impl PlayerValuationCalculator {
    pub fn calculate_value(player: &Player, date: NaiveDate) -> CurrencyValue {
        Self::calculate_value_with_price_level(player, date, 1.0)
    }

    pub fn calculate_value_with_price_level(player: &Player, date: NaiveDate, price_level: f32) -> CurrencyValue {
        let base_value = PlayerValueCalculator::calculate(player, date, price_level);

        // Transfer-listed players face market discount (buyer leverage)
        let mut market_value = base_value;

        if player.statuses.get().contains(&crate::PlayerStatusType::Lst) {
            market_value *= 0.9;
        }

        // Players wanting to leave lose negotiating power
        if player.statuses.get().contains(&crate::PlayerStatusType::Req) {
            market_value *= 0.85;
        }

        CurrencyValue {
            amount: market_value,
            currency: crate::shared::Currency::Usd,
        }
    }
}