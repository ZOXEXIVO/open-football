#[derive(Debug, Clone)]
pub struct CountryEconomicFactors {
    pub gdp_growth: f32,
    pub inflation_rate: f32,
    pub tv_revenue_multiplier: f32,
    pub sponsorship_market_strength: f32,
    pub stadium_attendance_factor: f32,
}

impl Default for CountryEconomicFactors {
    fn default() -> Self {
        Self::new()
    }
}

impl CountryEconomicFactors {
    pub fn new() -> Self {
        CountryEconomicFactors {
            gdp_growth: 0.02,
            inflation_rate: 0.03,
            tv_revenue_multiplier: 1.0,
            sponsorship_market_strength: 1.0,
            stadium_attendance_factor: 1.0,
        }
    }

    /// Create economic factors scaled by country reputation.
    /// Top countries (rep ~9500) get multipliers near 1.0,
    /// small countries (rep ~3000) get ~0.09.
    pub fn from_reputation(reputation: u16) -> Self {
        let factor = (reputation as f64 / 10000.0).clamp(0.0, 1.0);
        let market = (factor * factor) as f32;

        CountryEconomicFactors {
            gdp_growth: 0.02,
            inflation_rate: 0.03,
            tv_revenue_multiplier: market,
            sponsorship_market_strength: market,
            stadium_attendance_factor: market.max(0.3), // floor: attendance doesn't scale as harshly
        }
    }

    pub fn get_financial_multiplier(&self) -> f32 {
        1.0 + self.gdp_growth - self.inflation_rate
    }

    pub fn monthly_update(&mut self) {
        // Simulate economic fluctuations (relative to current values)
        use crate::utils::FloatUtils;

        self.gdp_growth += FloatUtils::random(-0.005, 0.005);
        self.gdp_growth = self.gdp_growth.clamp(-0.05, 0.10);

        self.inflation_rate += FloatUtils::random(-0.003, 0.003);
        self.inflation_rate = self.inflation_rate.clamp(0.0, 0.10);

        // ±2% relative fluctuation (preserves country-based scaling)
        let tv_delta = self.tv_revenue_multiplier * FloatUtils::random(-0.02, 0.02);
        self.tv_revenue_multiplier = (self.tv_revenue_multiplier + tv_delta).max(0.005);
    }
}
