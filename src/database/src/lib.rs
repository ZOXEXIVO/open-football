mod generators;
mod loaders;

use std::collections::HashMap;
use std::sync::OnceLock;

pub use loaders::{
    ClubEntity, ClubTeamEntity, ContinentEntity, ContinentLoader, CountryEntity, CountryLoader,
    DataTreeLoader, DomesticCupEntity, ForeignPlayerEntry, LeagueEntity, NamesByCountryEntity,
    NationalCompetitionEntity, NationalCompetitionLoader, OdbContract, OdbLoan, OdbPlayer,
    OdbPosition, OdbReputation, PlayersOdb,
};

pub use generators::DatabaseGenerator;

/// id → vector-position indexes over the loaded entity lists. Player
/// hydration resolves clubs/teams/leagues per record (and per history row);
/// the linear scans it replaces were O(clubs) each, which multiplied out to
/// hundreds of millions of iterations at world init on a full database.
struct EntityIndex {
    clubs_by_id: HashMap<u32, usize>,
    /// team id → (club position, team position within the club)
    teams_by_id: HashMap<u32, (usize, usize)>,
    leagues_by_id: HashMap<u32, usize>,
}

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

    /// Lazily-built id indexes; safe to share across the parallel
    /// generation passes.
    index: OnceLock<EntityIndex>,
}

impl DatabaseEntity {
    fn index(&self) -> &EntityIndex {
        self.index.get_or_init(|| {
            let mut clubs_by_id = HashMap::with_capacity(self.clubs.len());
            let mut teams_by_id = HashMap::with_capacity(self.clubs.len() * 2);
            for (ci, club) in self.clubs.iter().enumerate() {
                clubs_by_id.insert(club.id, ci);
                for (ti, team) in club.teams.iter().enumerate() {
                    teams_by_id.insert(team.id, (ci, ti));
                }
            }
            let leagues_by_id = self
                .leagues
                .iter()
                .enumerate()
                .map(|(i, l)| (l.id, i))
                .collect();
            EntityIndex {
                clubs_by_id,
                teams_by_id,
                leagues_by_id,
            }
        })
    }

    pub fn club_by_id(&self, id: u32) -> Option<&ClubEntity> {
        self.index().clubs_by_id.get(&id).map(|&i| &self.clubs[i])
    }

    /// Resolve a sub-team id (e.g. a satellite squad folded into its parent)
    /// to the owning club and the team itself.
    pub fn team_by_id(&self, id: u32) -> Option<(&ClubEntity, &ClubTeamEntity)> {
        self.index().teams_by_id.get(&id).map(|&(ci, ti)| {
            let club = &self.clubs[ci];
            (club, &club.teams[ti])
        })
    }

    pub fn league_by_id(&self, id: u32) -> Option<&LeagueEntity> {
        self.index()
            .leagues_by_id
            .get(&id)
            .map(|&i| &self.leagues[i])
    }
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
            index: OnceLock::new(),
        }
    }
}
