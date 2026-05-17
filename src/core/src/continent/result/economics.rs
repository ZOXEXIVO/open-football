use super::ContinentResult;
use crate::continent::Continent;
use log::debug;

impl ContinentResult {
    /// Continent-local: aggregate finance across every club, refresh
    /// economic indicators, TV-rights and sponsorship distribution.
    /// Takes `&mut Continent` so the orchestrator can run this in
    /// parallel across continents.
    pub(crate) fn update_economic_zone(continent: &mut Continent) {
        debug!("💰 Updating continental economic zone");

        // Calculate overall economic health
        let mut total_revenue = 0.0;
        let mut total_expenses = 0.0;

        for country in &continent.countries {
            for club in &country.clubs {
                total_revenue += club.finance.balance.income as f64;
                total_expenses += club.finance.balance.outcome as f64;
            }
        }

        continent
            .economic_zone
            .update_indicators(total_revenue, total_expenses);

        // Update TV rights distribution
        continent
            .economic_zone
            .recalculate_tv_rights(&continent.continental_rankings);

        // Update sponsorship market
        continent
            .economic_zone
            .update_sponsorship_market(&continent.continental_rankings);
    }
}
