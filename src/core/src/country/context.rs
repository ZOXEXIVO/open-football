use crate::PeopleNameGeneratorData;
use crate::country::SeasonDates;

#[derive(Clone)]
pub struct CountryContext {
    pub id: u32,
    pub people_names: Option<PeopleNameGeneratorData>,
    pub season_dates: SeasonDates,
}

impl CountryContext {
    pub fn new(id: u32) -> Self {
        CountryContext { id, people_names: None, season_dates: SeasonDates::default() }
    }

    pub fn with_people_names(id: u32, people_names: PeopleNameGeneratorData) -> Self {
        CountryContext { id, people_names: Some(people_names), season_dates: SeasonDates::default() }
    }

    pub fn with_season_dates(mut self, season_dates: SeasonDates) -> Self {
        self.season_dates = season_dates;
        self
    }
}
