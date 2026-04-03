#[derive(Clone)]
pub struct BoardContext {
    pub balance: i64,
    pub total_annual_wages: u32,
    pub reputation_score: f32,
    pub main_squad_size: usize,
    pub reserve_squad_size: usize,
    /// Country economic multiplier (0.0-1.0). Derived from country reputation squared.
    /// Top countries ≈ 0.9, Colombia ≈ 0.56, Malta ≈ 0.08.
    pub country_economic_factor: f32,
    /// Country-level price multiplier from data. England 1.5, Colombia 0.4, default 1.0.
    /// Used together with economic factor to cap transfer budgets realistically.
    pub country_price_level: f32,
}

impl BoardContext {
    pub fn new() -> Self {
        BoardContext {
            balance: 0,
            total_annual_wages: 0,
            reputation_score: 0.0,
            main_squad_size: 0,
            reserve_squad_size: 0,
            country_economic_factor: 1.0,
            country_price_level: 1.0,
        }
    }
}
