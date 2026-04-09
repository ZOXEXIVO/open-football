#[derive(Clone)]
pub struct BoardContext {
    pub balance: i64,
    pub total_annual_wages: u32,
    pub reputation_score: f32,
    pub main_squad_size: usize,
    pub reserve_squad_size: usize,
    pub country_economic_factor: f32,
    pub country_price_level: f32,

    // Performance tracking
    /// Current league position (1-based, 0 = unknown)
    pub league_position: u8,
    /// Total teams in the league
    pub league_size: u8,
    /// Recent form: wins in last 5 matches
    pub recent_wins: u8,
    /// Recent form: losses in last 5 matches
    pub recent_losses: u8,
    /// Season progress: matches played
    pub matches_played: u8,
    /// Season progress: total matches in a full season
    pub total_matches: u8,
    /// Average squad CA (main team)
    pub avg_squad_ability: u8,
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
            league_position: 0,
            league_size: 0,
            recent_wins: 0,
            recent_losses: 0,
            matches_played: 0,
            total_matches: 0,
            avg_squad_ability: 0,
        }
    }
}
