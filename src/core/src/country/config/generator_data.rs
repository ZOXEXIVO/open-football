// Update CountryGeneratorData and PeopleNameGeneratorData as per original
#[derive(Clone)]
pub struct CountryGeneratorData {
    pub people_names: PeopleNameGeneratorData,
}

impl CountryGeneratorData {
    pub fn new(first_names: Vec<String>, last_names: Vec<String>, nicknames: Vec<String>) -> Self {
        CountryGeneratorData {
            people_names: PeopleNameGeneratorData {
                first_names,
                last_names,
                nicknames,
            },
        }
    }

    pub fn empty() -> Self {
        CountryGeneratorData {
            people_names: PeopleNameGeneratorData {
                first_names: Vec::new(),
                last_names: Vec::new(),
                nicknames: Vec::new(),
            },
        }
    }
}

#[derive(Clone)]
pub struct PeopleNameGeneratorData {
    pub first_names: Vec<String>,
    pub last_names: Vec<String>,
    pub nicknames: Vec<String>,
}
