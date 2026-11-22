use crate::Country;
use crate::country::economy::inflation::MarketInflation;
use crate::transfers::TransferType;
use crate::utils::DateUtils;
use chrono::{Duration, NaiveDate};
use log::debug;

impl Country {
    pub(crate) fn on_international_competitions(&mut self, date: NaiveDate) {
        for competition in &mut self.international_competitions {
            competition.simulate_round(date);
        }
    }

    pub(crate) fn on_economic_factors(&mut self, date: NaiveDate) {
        if DateUtils::is_month_beginning(date) {
            self.economic_factors.monthly_update();
        }
        self.update_market_price_level(date);
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
    fn update_market_price_level(&mut self, date: NaiveDate) {
        if !MarketInflation::is_repricing_day(date) {
            return;
        }
        let year_ago = date - Duration::days(365);
        let gross_spend: f64 = self
            .transfer_market
            .transfer_history
            .iter()
            .filter(|t| t.transfer_date >= year_ago)
            .filter(|t| matches!(t.transfer_type, TransferType::Permanent))
            .map(|t| t.fee.amount.max(0.0))
            .sum();
        let league_income: f64 = self
            .clubs
            .iter()
            .map(|c| c.finance.estimated_annual_income(date).max(0) as f64)
            .sum();

        let current = self.settings.pricing.price_level;
        let next = MarketInflation::next_level(current, gross_spend, league_income);
        if (next - current).abs() > f32::EPSILON {
            debug!(
                "Country {} price level {:.3} -> {:.3} (spend {:.0} / income {:.0})",
                self.name, current, next, gross_spend, league_income
            );
            self.settings.pricing.price_level = next;
        }
    }

    pub(crate) fn on_country_reputation(&mut self) {
        let mut reputation_change: i16 = 0;

        for league in &self.leagues.leagues {
            let competitiveness = league.competitiveness();
            reputation_change += (competitiveness * 5.0) as i16;
        }

        let international_success = self.calculate_international_success();
        reputation_change += international_success;

        let transfer_reputation = self.calculate_transfer_market_reputation();
        reputation_change += transfer_reputation;

        let new_reputation =
            (self.reputation as i32 + reputation_change as i32).clamp(0, 10000) as u16;

        if new_reputation != self.reputation {
            debug!(
                "Country {} reputation changed: {} -> {} ({})",
                self.name,
                self.reputation,
                new_reputation,
                if reputation_change > 0 {
                    format!("+{}", reputation_change)
                } else {
                    reputation_change.to_string()
                }
            );
            self.reputation = new_reputation;
        }
    }

    fn calculate_international_success(&self) -> i16 {
        // Count clubs in continental competitions (approximated by having high world reputation)
        let high_rep_clubs = self
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

    fn calculate_transfer_market_reputation(&self) -> i16 {
        // Active transfer market with incoming signings boosts reputation
        let completed = self.transfer_market.transfer_history.len();
        match completed {
            0..=5 => -1,
            6..=20 => 0,
            21..=50 => 1,
            _ => 3,
        }
    }
}
