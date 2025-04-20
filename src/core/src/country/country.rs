use crate::context::GlobalContext;
use crate::country::CountryResult;
use crate::league::LeagueCollection;
use crate::utils::Logging;
use crate::{Club, ClubResult};
use rayon::iter::IntoParallelRefMutIterator;
use rayon::iter::ParallelIterator;

// In module crate::country::country
pub struct Country {
    pub id: u32,
    pub code: String,
    pub slug: String,
    pub name: String,
    pub continent_id: u32,
    pub leagues: LeagueCollection,
    pub clubs: Vec<Club>,
    pub reputation: u16,
    pub generator_data: CountryGeneratorData,
}

impl Country {
    // Create a builder instead of directly constructing the object
    pub fn builder() -> CountryBuilder {
        CountryBuilder::default()
    }

    // Keep a simplified constructor for convenience if all fields are available
    pub fn new(
        id: u32,
        code: String,
        name: String,
        continent_id: u32,
        reputation: u16,
        generator_data: CountryGeneratorData,
    ) -> Self {
        Self::builder()
            .id(id)
            .code(code)
            .name(&name)
            .continent_id(continent_id)
            .reputation(reputation)
            .generator_data(generator_data)
            .build()
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> CountryResult {
        let league_results = self.leagues.simulate(&self.clubs, &ctx);

        let clubs_results: Vec<ClubResult> = self
            .clubs
            .par_iter_mut()
            .map(|club| {
                let message = &format!("simulate club: {}", &club.name);
                Logging::estimate_result(
                    || club.simulate(ctx.with_club(club.id, &club.name.clone())),
                    message,
                )
            })
            .collect();

        CountryResult::new(league_results, clubs_results)
    }
}

#[derive(Default)]
pub struct CountryBuilder {
    id: Option<u32>,
    code: Option<String>,
    slug: Option<String>,
    name: Option<String>,
    continent_id: Option<u32>,
    leagues: Option<LeagueCollection>,
    clubs: Vec<Club>,
    reputation: Option<u16>,
    generator_data: Option<CountryGeneratorData>,
}

impl CountryBuilder {
    pub fn id(mut self, id: u32) -> Self {
        self.id = Some(id);
        self
    }

    pub fn code(mut self, code: String) -> Self {
        self.slug = Some(code.to_lowercase());
        self
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        if self.slug.is_none() {
            self.slug = Some(name.to_lowercase().replace(' ', "-"));
        }
        self
    }

    pub fn continent_id(mut self, continent_id: u32) -> Self {
        self.continent_id = Some(continent_id);
        self
    }

    pub fn leagues(mut self, leagues: LeagueCollection) -> Self {
        self.leagues = Some(leagues);
        self
    }

    pub fn add_club(mut self, club: Club) -> Self {
        self.clubs.push(club);
        self
    }

    pub fn clubs(mut self, clubs: Vec<Club>) -> Self {
        self.clubs = clubs;
        self
    }

    pub fn reputation(mut self, reputation: u16) -> Self {
        self.reputation = Some(reputation);
        self
    }

    pub fn generator_data(mut self, generator_data: CountryGeneratorData) -> Self {
        self.generator_data = Some(generator_data);
        self
    }

    pub fn build(self) -> Country {
        Country {
            id: self.id.expect("Country id is required"),
            code: self.code.expect("Country code is required"),
            slug: self.slug.expect("Country slug is required"),
            name: self.name.expect("Country name is required"),
            continent_id: self.continent_id.expect("Continent id is required"),
            leagues: self.leagues.unwrap_or_else(|| LeagueCollection { leagues: Vec::new() }),
            clubs: self.clubs,
            reputation: self.reputation.unwrap_or(0),
            generator_data: self.generator_data.expect("Generator data is required"),
        }
    }
}

pub struct CountryGeneratorData {
    pub people_names: PeopleNameGeneratorData,
}

impl CountryGeneratorData {
    pub fn new(first_names: Vec<String>, last_names: Vec<String>) -> Self {
        CountryGeneratorData {
            people_names: PeopleNameGeneratorData {
                first_names,
                last_names,
            },
        }
    }

    pub fn empty() -> Self {
        CountryGeneratorData {
            people_names: PeopleNameGeneratorData {
                first_names: Vec::new(),
                last_names: Vec::new(),
            },
        }
    }
}

pub struct PeopleNameGeneratorData {
    pub first_names: Vec<String>,
    pub last_names: Vec<String>,
}