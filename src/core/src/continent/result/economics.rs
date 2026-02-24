use super::ContinentResult;
use crate::country::CountryResult;
use crate::simulator::SimulatorData;
use log::info;

impl ContinentResult {
    pub(crate) fn update_economic_zone(&self, data: &mut SimulatorData, _country_results: &[CountryResult]) {
        info!("💰 Updating continental economic zone");

        let continent_id = self.get_continent_id(data);

        if let Some(continent) = data.continent_mut(continent_id) {
            // Calculate overall economic health
            let mut total_revenue = 0.0;
            let mut total_expenses = 0.0;

            for country in &continent.countries {
                for club in &country.clubs {
                    total_revenue += club.finance.balance.income as f64;
                    total_expenses += club.finance.balance.outcome as f64;
                }
            }

            continent.economic_zone.update_indicators(total_revenue, total_expenses);

            // Update TV rights distribution
            continent.economic_zone.recalculate_tv_rights(&continent.continental_rankings);

            // Update sponsorship market
            continent.economic_zone.update_sponsorship_market(&continent.continental_rankings);
        }
    }
}
