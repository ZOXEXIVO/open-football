use crate::shared::{Currency, CurrencyValue};
use crate::{Club, Country, Player, PlayerStatusType, PlayerValueCalculator};
use chrono::{Datelike, NaiveDate};
use std::collections::HashMap;

#[derive(Debug, Clone)]
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
        self.current_window_dates(country_id, date).is_some()
    }

    /// Returns the (start, end) dates of the currently open transfer window,
    /// or None if no window is currently open for this country.
    pub fn current_window_dates(
        &self,
        country_id: u32,
        date: NaiveDate,
    ) -> Option<(NaiveDate, NaiveDate)> {
        let (summer, winter) = if let Some(window) = self.windows.get(&country_id) {
            (window.summer_window, window.winter_window)
        } else {
            Self::default_european_windows(date)
        };

        if self.is_date_in_window(date, &summer) {
            Some(summer)
        } else if self.is_date_in_window(date, &winter) {
            Some(winter)
        } else {
            None
        }
    }

    fn is_date_in_window(&self, date: NaiveDate, window: &(NaiveDate, NaiveDate)) -> bool {
        date >= window.0 && date <= window.1
    }

    fn default_european_windows(
        date: NaiveDate,
    ) -> ((NaiveDate, NaiveDate), (NaiveDate, NaiveDate)) {
        let year = date.year();
        let summer_start = NaiveDate::from_ymd_opt(year, 6, 1).unwrap_or(date);
        let summer_end = NaiveDate::from_ymd_opt(year, 8, 31).unwrap_or(date);
        let winter_start = NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or(date);
        let winter_end = NaiveDate::from_ymd_opt(year, 1, 31).unwrap_or(date);
        ((summer_start, summer_end), (winter_start, winter_end))
    }
}

impl Default for TransferWindowManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod transfer_window_tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn custom_country_window_overrides_default_dates() {
        let mut manager = TransferWindowManager::new();
        manager.add_window(
            99,
            TransferWindow {
                summer_window: (d(2026, 2, 1), d(2026, 3, 15)),
                winter_window: (d(2026, 7, 1), d(2026, 7, 31)),
                country_id: 99,
            },
        );

        assert!(manager.is_window_open(99, d(2026, 2, 20)));
        assert!(!manager.is_window_open(99, d(2026, 6, 20)));
    }
}

/// Transfer-market-specific player valuation.
/// Wraps `PlayerValueCalculator` with market conditions (selling pressure, squad role).
pub struct PlayerValuationCalculator;

impl PlayerValuationCalculator {
    pub fn calculate_value(
        player: &Player,
        date: NaiveDate,
        league_reputation: u16,
        club_reputation: u16,
    ) -> CurrencyValue {
        Self::calculate_value_with_price_level(
            player,
            date,
            1.0,
            league_reputation,
            club_reputation,
        )
    }

    pub fn calculate_value_with_price_level(
        player: &Player,
        date: NaiveDate,
        price_level: f32,
        league_reputation: u16,
        club_reputation: u16,
    ) -> CurrencyValue {
        let base_value = PlayerValueCalculator::calculate(
            player,
            date,
            price_level,
            league_reputation,
            club_reputation,
        );

        // Transfer-listed players face market discount (buyer leverage)
        let mut market_value = base_value;

        if player.statuses.get().contains(&PlayerStatusType::Lst) {
            market_value *= 0.9;
        }

        // Players wanting to leave lose negotiating power
        if player.statuses.get().contains(&PlayerStatusType::Req) {
            market_value *= 0.85;
        }

        CurrencyValue {
            amount: market_value,
            currency: Currency::Usd,
        }
    }

    /// Resolve (league_reputation, club_market_score) for a club within
    /// its country. Single source of truth for seller-side market
    /// context — avoids each call site re-implementing the same league
    /// lookup or, worse, passing 0/0 and flattening price levels across
    /// every league. Returns (0, 0) only when the club has no main team
    /// or its league isn't registered.
    pub fn seller_context(country: &Country, club: &Club) -> (u16, u16) {
        let main = club.teams.main();
        let club_rep = main
            .map(|t| t.reputation.market_value_score())
            .unwrap_or(0);
        let league_rep = main
            .and_then(|t| t.league_id)
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);
        (league_rep, club_rep)
    }

    /// Variant for callers that don't carry a `Country` reference (board
    /// audits, AI transfer-listing AI). League reputation is approximated
    /// from the club's blended score since the two correlate strongly
    /// (top-rep clubs play in top-rep leagues), keeping market values
    /// roughly correct without forcing every caller to plumb the country
    /// down.
    pub fn seller_context_from_club(club: &Club) -> (u16, u16) {
        let club_rep = club
            .teams
            .main()
            .map(|t| t.reputation.market_value_score())
            .unwrap_or(0);
        (club_rep, club_rep)
    }
}
