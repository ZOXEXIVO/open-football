mod generators;
mod loaders;

pub use loaders::{
    ClubEntity, ContinentEntity, ContinentLoader, CountryEntity, CountryLoader,
    DataTreeLoader, ForeignPlayerEntry, LeagueEntity, NamesByCountryEntity,
    NationalCompetitionEntity, NationalCompetitionLoader, OdbContract, OdbFile, OdbLoan,
    OdbPlayer, OdbPosition, OdbReputation, PlayersOdb,
};

pub use generators::DatabaseGenerator;

pub struct DatabaseEntity {
    pub continents: Vec<ContinentEntity>,
    pub countries: Vec<CountryEntity>,
    pub leagues: Vec<LeagueEntity>,
    pub clubs: Vec<ClubEntity>,
    pub national_competitions: Vec<NationalCompetitionEntity>,

    pub names_by_country: Vec<NamesByCountryEntity>,

    /// Optional external player database, loaded from `players.odb` next to
    /// the binary. When present, every club referenced by at least one record
    /// is populated from this file instead of via procedural generation.
    pub players_odb: Option<PlayersOdb>,
}

pub struct DatabaseLoader;

impl DatabaseLoader {
    pub fn load() -> DatabaseEntity {
        let continents = ContinentLoader::load();
        let countries = CountryLoader::load();
        let tree = DataTreeLoader::load(&countries);
        let players_odb = PlayersOdb::load();

        DatabaseEntity {
            continents,
            countries,
            leagues: tree.leagues,
            clubs: tree.clubs,
            national_competitions: NationalCompetitionLoader::load(),
            names_by_country: tree.names_by_country,
            players_odb,
        }
    }
}
