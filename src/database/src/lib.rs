mod generators;
mod loaders;

pub use loaders::{
    ClubEntity, ClubLoader, ContinentEntity, ContinentLoader, CountryEntity, CountryLoader,
    ForeignPlayerEntry, LeagueEntity, LeagueLoader, NamesByCountryEntity, NamesByCountryLoader,
    NationalCompetitionEntity, NationalCompetitionLoader,
};

pub use generators::DatabaseGenerator;

pub struct DatabaseEntity {
    pub continents: Vec<ContinentEntity>,
    pub countries: Vec<CountryEntity>,
    pub leagues: Vec<LeagueEntity>,
    pub clubs: Vec<ClubEntity>,
    pub national_competitions: Vec<NationalCompetitionEntity>,

    pub names_by_country: Vec<NamesByCountryEntity>,
}

pub struct DatabaseLoader;

impl DatabaseLoader {
    pub fn load() -> DatabaseEntity {
        DatabaseEntity {
            continents: ContinentLoader::load(),
            countries: CountryLoader::load(),
            leagues: LeagueLoader::load(),
            clubs: ClubLoader::load(),
            national_competitions: NationalCompetitionLoader::load(),
            names_by_country: NamesByCountryLoader::load(),
        }
    }
}
