#[derive(Debug, Clone)]
pub struct CountryRegulations {
    pub foreign_player_limit: Option<u8>,
    pub salary_cap: Option<f64>,
    pub homegrown_requirements: Option<u8>,
    pub ffp_enabled: bool, // Financial Fair Play
}

impl CountryRegulations {
    pub fn new() -> Self {
        CountryRegulations {
            foreign_player_limit: None,
            salary_cap: None,
            homegrown_requirements: None,
            ffp_enabled: false,
        }
    }
}
