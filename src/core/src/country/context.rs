use crate::PeopleNameGeneratorData;

#[derive(Clone)]
pub struct CountryContext {
    pub id: u32,
    pub people_names: Option<PeopleNameGeneratorData>,
}

impl CountryContext {
    pub fn new(id: u32) -> Self {
        CountryContext { id, people_names: None }
    }

    pub fn with_people_names(id: u32, people_names: PeopleNameGeneratorData) -> Self {
        CountryContext { id, people_names: Some(people_names) }
    }
}
