use super::CountryResult;
use crate::Country;
use crate::country::economy::inflation::MarketInflation;
use crate::league::League;
use crate::league::LeagueResult;
use crate::transfers::TransferType;
use crate::utils::DateUtils;
use chrono::{Duration, NaiveDate};
use log::debug;

impl CountryResult {
    pub(crate) fn simulate_international_competitions(country: &mut Country, date: NaiveDate) {
        for competition in &mut country.international_competitions {
            competition.simulate_round(date);
        }
    }

    pub(crate) fn update_economic_factors(country: &mut Country, date: NaiveDate) {
        if DateUtils::is_month_beginning(date) {
            country.economic_factors.monthly_update();
        }
        Self::update_market_price_level(country, date);
    }

    /// Move the country's transfer-market price level once a year.
    ///
    /// `price_level` feeds every player valuation and the board's own budget
    /// ceiling, and it was loaded once from the country data and never moved
    /// again — so a league could treble its television income over a decade
    /// and its players' valuations would not notice.
    ///
    /// The driver is DEMAND against the money behind it (gross fees paid ÷
    /// what the country's clubs earn), never the valuations themselves: a
    /// market that re-priced off its own prices would spiral. See
    /// [`MarketInflation`].
    fn update_market_price_level(country: &mut Country, date: NaiveDate) {
        if !MarketInflation::is_repricing_day(date) {
            return;
        }
        let year_ago = date - Duration::days(365);
        let gross_spend: f64 = country
            .transfer_market
            .transfer_history
            .iter()
            .filter(|t| t.transfer_date >= year_ago)
            .filter(|t| matches!(t.transfer_type, TransferType::Permanent))
            .map(|t| t.fee.amount.max(0.0))
            .sum();
        let league_income: f64 = country
            .clubs
            .iter()
            .map(|c| c.finance.estimated_annual_income(date).max(0) as f64)
            .sum();

        let current = country.settings.pricing.price_level;
        let next = MarketInflation::next_level(current, gross_spend, league_income);
        if (next - current).abs() > f32::EPSILON {
            debug!(
                "Country {} price level {:.3} -> {:.3} (spend {:.0} / income {:.0})",
                country.name, current, next, gross_spend, league_income
            );
            country.settings.pricing.price_level = next;
        }
    }

    pub(crate) fn simulate_media_coverage(country: &mut Country, league_results: &[LeagueResult]) {
        country.media_coverage.update_from_results(league_results);
        country
            .media_coverage
            .generate_weekly_stories(&country.clubs);
    }

    pub(crate) fn update_country_reputation(country: &mut Country) {
        let mut reputation_change: i16 = 0;

        for league in &country.leagues.leagues {
            let competitiveness = Self::calculate_league_competitiveness(league);
            reputation_change += (competitiveness * 5.0) as i16;
        }

        let international_success = Self::calculate_international_success(country);
        reputation_change += international_success as i16;

        let transfer_reputation = Self::calculate_transfer_market_reputation(country);
        reputation_change += transfer_reputation as i16;

        let new_reputation =
            (country.reputation as i32 + reputation_change as i32).clamp(0, 10000) as u16;

        if new_reputation != country.reputation {
            debug!(
                "Country {} reputation changed: {} -> {} ({})",
                country.name,
                country.reputation,
                new_reputation,
                if reputation_change > 0 {
                    format!("+{}", reputation_change)
                } else {
                    reputation_change.to_string()
                }
            );
            country.reputation = new_reputation;
        }
    }

    fn calculate_league_competitiveness(league: &League) -> f32 {
        if league.table.rows.is_empty() {
            return 0.0;
        }

        let rows = &league.table.rows;
        let total = rows.len() as f32;
        if total < 2.0 {
            return 0.0;
        }

        // Measure point spread between top and bottom — tighter = more competitive
        let max_points = rows.iter().map(|r| r.points).max().unwrap_or(0) as f32;
        let min_points = rows.iter().map(|r| r.points).min().unwrap_or(0) as f32;

        if max_points <= 0.0 {
            return 0.0;
        }

        let spread = (max_points - min_points) / max_points;
        // spread ~0.3 = very competitive, spread ~0.8 = dominated
        // Map to -1.0 (bad) to 1.0 (good)
        (1.0 - spread * 2.0).clamp(-1.0, 1.0)
    }

    fn calculate_international_success(country: &Country) -> i16 {
        // Count clubs in continental competitions (approximated by having high world reputation)
        let high_rep_clubs = country
            .clubs
            .iter()
            .filter(|c| {
                c.teams
                    .teams
                    .first()
                    .map(|t| t.reputation.overall_score() >= 0.6)
                    .unwrap_or(false)
            })
            .count();

        match high_rep_clubs {
            0 => -2,
            1 => 0,
            2..=3 => 2,
            _ => 5,
        }
    }

    fn calculate_transfer_market_reputation(country: &Country) -> i16 {
        // Active transfer market with incoming signings boosts reputation
        let completed = country.transfer_market.transfer_history.len();
        match completed {
            0..=5 => -1,
            6..=20 => 0,
            21..=50 => 1,
            _ => 3,
        }
    }
}
