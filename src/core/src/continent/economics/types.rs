use crate::continent::ContinentalRankings;

#[derive(Debug, Clone)]
pub struct EconomicZone {
    pub tv_rights_pool: f64,
    pub sponsorship_value: f64,
    pub economic_health_indicator: f32,
}

impl EconomicZone {
    pub fn new() -> Self {
        EconomicZone {
            tv_rights_pool: 5_000_000_000.0,
            sponsorship_value: 2_000_000_000.0,
            economic_health_indicator: 0.7,
        }
    }

    pub fn get_overall_health(&self) -> f32 {
        self.economic_health_indicator
    }

    pub fn update_indicators(&mut self, total_revenue: f64, total_expenses: f64) {
        let profit_margin = (total_revenue - total_expenses) / total_revenue;

        // Update health indicator based on profit margin
        self.economic_health_indicator =
            (self.economic_health_indicator * 0.8 + profit_margin as f32 * 0.2).clamp(0.0, 1.0);
    }

    pub fn recalculate_tv_rights(&mut self, rankings: &ContinentalRankings) {
        // Adjust TV rights based on competitive balance
        let competitive_balance = self.calculate_competitive_balance(rankings);
        self.tv_rights_pool *= 1.0 + competitive_balance as f64 * 0.1;
    }

    pub fn update_sponsorship_market(&mut self, _rankings: &ContinentalRankings) {
        // Update based on top clubs' performance
        self.sponsorship_value *= 1.02; // Simplified growth
    }

    fn calculate_competitive_balance(&self, _rankings: &ContinentalRankings) -> f32 {
        // Measure how competitive the continent is
        0.5 // Simplified
    }
}
