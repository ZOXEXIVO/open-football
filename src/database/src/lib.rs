mod generators;
mod loaders;

pub use loaders::{
    ClubEntity, ContinentEntity, ContinentLoader, CountryEntity, CountryLoader,
    ForeignPlayerEntry, LeagueEntity, NamesByCountryEntity,
    NationalCompetitionEntity, NationalCompetitionLoader, DataTreeLoader,
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
        let continents = ContinentLoader::load();
        let countries = CountryLoader::load();
        let tree = DataTreeLoader::load(&countries);

        DatabaseEntity {
            continents,
            countries,
            leagues: tree.leagues,
            clubs: tree.clubs,
            national_competitions: NationalCompetitionLoader::load(),
            names_by_country: tree.names_by_country,
        }
    }
}
