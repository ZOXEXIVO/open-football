#[derive(Clone)]
pub struct BoardContext {
    pub balance: i64,
    pub total_annual_wages: u32,
    pub reputation_score: f32,
    pub main_squad_size: usize,
    pub reserve_squad_size: usize,
}

impl BoardContext {
    pub fn new() -> Self {
        BoardContext {
            balance: 0,
            total_annual_wages: 0,
            reputation_score: 0.0,
            main_squad_size: 0,
            reserve_squad_size: 0,
        }
    }
}
