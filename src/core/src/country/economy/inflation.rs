//! Transfer-market inflation: how a country's `price_level` moves.
//!
//! `price_level` already feeds every valuation in the game
//! (`PlayerValuationCalculator::calculate_value_with_price_level`) and the
//! board's own budget ceiling. It was loaded once from the country data and
//! never moved again, so a league could double its television income over a
//! decade and its players' valuations would not notice.
//!
//! What actually inflates a football market is not values — it is DEMAND
//! measured against the money behind it. A league whose clubs spend two
//! thirds of their income on transfers is bidding prices up; one spending a
//! tenth is not. So the driver here is gross spend ÷ league income, and the
//! response is bounded hard: a market that re-priced itself off its own
//! valuations would spiral, which is precisely the failure this is designed
//! against.

use chrono::{Datelike, NaiveDate};

/// Moves one country's price level once a year.
pub struct MarketInflation;

impl MarketInflation {
    /// How strongly a year's spending pressure moves the price level.
    pub const INFLATION_K: f32 = 0.15;
    /// Gross transfer spend ÷ league income at which a market is in
    /// equilibrium — it neither inflates nor deflates. Sits inside the
    /// 0.35–0.55 band real big-league clubs operate in, so an ordinary
    /// league drifts gently and an unusually hot or cold one moves.
    pub const INFLATION_REF: f32 = 0.45;
    /// Hardest bound in the model: a market may not re-price itself by more
    /// than a tenth in a year, whatever the pressure says. Without it, a
    /// rising price level raises values, which raises budgets through the
    /// income share, which raises prices.
    pub const MAX_YEARLY_DRIFT: f32 = 0.10;
    /// Absolute band a price level may wander in. A country cannot inflate
    /// its way into being a different country.
    const FLOOR: f32 = 0.25;
    const CEILING: f32 = 4.0;

    /// The new price level for a country, given a year of trading.
    ///
    /// `gross_spend` is what its clubs paid in fees over the year;
    /// `league_income` is what they earned. A country with no measurable
    /// income does not move — reading a missing denominator as infinite
    /// pressure would deflate every fresh world on its first anniversary.
    pub fn next_level(current: f32, gross_spend: f64, league_income: f64) -> f32 {
        if league_income <= 0.0 {
            return current;
        }
        let pressure = (gross_spend / league_income) as f32;
        let drift = (Self::INFLATION_K * (pressure - Self::INFLATION_REF))
            .clamp(-Self::MAX_YEARLY_DRIFT, Self::MAX_YEARLY_DRIFT);
        (current * (1.0 + drift)).clamp(Self::FLOOR, Self::CEILING)
    }

    /// True on the one day a year the drift is applied. Anchored to the
    /// calendar rather than to a season boundary so every country in the
    /// world re-prices on the same tick and no league can drift twice by
    /// straddling a season change.
    pub fn is_repricing_day(date: NaiveDate) -> bool {
        date.month() == 1 && date.day() == 1
    }
}

#[cfg(test)]
mod inflation_tests {
    use super::*;

    #[test]
    fn a_market_spending_at_the_reference_rate_does_not_move() {
        let next = MarketInflation::next_level(1.0, 45.0, 100.0);
        assert!((next - 1.0).abs() < 1e-6, "{next}");
    }

    #[test]
    fn a_hot_market_inflates_and_a_cold_one_deflates() {
        assert!(MarketInflation::next_level(1.0, 90.0, 100.0) > 1.0);
        assert!(MarketInflation::next_level(1.0, 5.0, 100.0) < 1.0);
    }

    #[test]
    fn drift_is_bounded_at_a_tenth_a_year_however_hot_the_market() {
        let runaway = MarketInflation::next_level(1.0, 10_000.0, 100.0);
        assert!(
            runaway <= 1.0 + MarketInflation::MAX_YEARLY_DRIFT + 1e-6,
            "the spiral guard is the whole point: {runaway}"
        );
        let collapse = MarketInflation::next_level(1.0, 0.0, 100.0);
        assert!(collapse >= 1.0 - MarketInflation::MAX_YEARLY_DRIFT - 1e-6);
    }

    #[test]
    fn a_country_with_no_income_evidence_does_not_move() {
        assert_eq!(MarketInflation::next_level(1.3, 50.0, 0.0), 1.3);
    }

    #[test]
    fn a_price_level_cannot_wander_out_of_its_band() {
        let mut level = 1.0;
        for _ in 0..200 {
            level = MarketInflation::next_level(level, 10_000.0, 100.0);
        }
        assert!(level <= 4.0);
        let mut level = 1.0;
        for _ in 0..200 {
            level = MarketInflation::next_level(level, 0.0, 100.0);
        }
        assert!(level >= 0.25);
    }

    #[test]
    fn every_country_reprices_on_the_same_day() {
        assert!(MarketInflation::is_repricing_day(
            NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        ));
        assert!(!MarketInflation::is_repricing_day(
            NaiveDate::from_ymd_opt(2027, 7, 1).unwrap()
        ));
    }
}
