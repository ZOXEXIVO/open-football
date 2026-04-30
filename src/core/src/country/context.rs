use crate::PeopleNameGeneratorData;
use crate::country::SeasonDates;

#[derive(Clone)]
pub struct CountryContext {
    pub id: u32,
    pub code: String,
    pub people_names: Option<PeopleNameGeneratorData>,
    pub season_dates: SeasonDates,
    pub tv_revenue_multiplier: f32,
    pub sponsorship_market_strength: f32,
    pub stadium_attendance_factor: f32,
    /// Country-level price multiplier for transfers and budgets (from country data).
    /// England 1.5, Spain 1.2, Colombia 0.4, etc. Default 1.0.
    pub price_level: f32,
    /// Country football-ecosystem strength (0..10000). Drives, among other
    /// things, how realistic the academy generator is allowed to be when
    /// minting elite prospects: a Brazilian academy of the same physical
    /// quality as a Cambodian one should produce stronger youth on average.
    pub reputation: u16,
}

impl CountryContext {
    pub fn new(id: u32) -> Self {
        CountryContext {
            id,
            code: String::new(),
            people_names: None,
            season_dates: SeasonDates::default(),
            tv_revenue_multiplier: 1.0,
            sponsorship_market_strength: 1.0,
            stadium_attendance_factor: 1.0,
            price_level: 1.0,
            reputation: 0,
        }
    }

    pub fn with_people_names(id: u32, people_names: PeopleNameGeneratorData) -> Self {
        CountryContext {
            id,
            code: String::new(),
            people_names: Some(people_names),
            season_dates: SeasonDates::default(),
            tv_revenue_multiplier: 1.0,
            sponsorship_market_strength: 1.0,
            stadium_attendance_factor: 1.0,
            price_level: 1.0,
            reputation: 0,
        }
    }

    pub fn with_code(mut self, code: String) -> Self {
        self.code = code;
        self
    }

    pub fn with_season_dates(mut self, season_dates: SeasonDates) -> Self {
        self.season_dates = season_dates;
        self
    }
}
