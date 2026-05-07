use crate::SimulatorData;
use crate::continent::Continent;
use rayon::prelude::*;
use std::collections::HashMap;

#[derive(Clone)]
pub struct SimulatorDataIndexes {
    pub league_indexes: HashMap<u32, (u32, u32)>,
    pub club_indexes: HashMap<u32, (u32, u32)>,
    pub team_indexes: HashMap<u32, (u32, u32, u32)>,
    pub player_indexes: HashMap<u32, (u32, u32, u32, u32)>,
    pub staff_indexes: HashMap<u32, (u32, u32, u32, u32)>,
    pub team_data_index: HashMap<u32, TeamData>,
    pub slug_indexes: SlugIndexes,
}

impl SimulatorDataIndexes {
    pub fn new() -> Self {
        SimulatorDataIndexes {
            league_indexes: HashMap::new(),
            club_indexes: HashMap::new(),
            team_indexes: HashMap::new(),
            player_indexes: HashMap::new(),
            staff_indexes: HashMap::new(),
            team_data_index: HashMap::new(),
            slug_indexes: SlugIndexes::new(),
        }
    }

    pub fn refresh(&mut self, data: &SimulatorData) {
        // Build per-continent shards in parallel — every id in the world is
        // globally unique, so the shards have disjoint keys and merge with
        // a plain `extend`. Each rayon worker writes only into its own
        // shard; no shared collector, no lock.
        let shards: Vec<SimulatorDataIndexes> = data
            .continents
            .par_iter()
            .map(|continent| {
                let mut shard = SimulatorDataIndexes::new();
                shard.fill_continent(continent);
                shard
            })
            .collect();

        for shard in shards {
            self.merge_shard(shard);
        }
    }

    /// Populate this shard with every entity inside `continent`. Mirrors
    /// the layout the serial walk used to produce — kept here on the
    /// indexes type so the parallel and incremental paths can share a
    /// single canonical traversal.
    fn fill_continent(&mut self, continent: &Continent) {
        for country in &continent.countries {
            self.slug_indexes
                .add_country_slug(&country.slug, country.id);

            for league in &country.leagues.leagues {
                self.add_league_location(league.id, continent.id, country.id);
                self.slug_indexes.add_league_slug(&league.slug, league.id);
            }

            for club in &country.clubs {
                self.add_club_location(club.id, continent.id, country.id);

                for team in &club.teams.teams {
                    self.add_team_data(
                        team.id,
                        TeamData {
                            name: team.name.clone(),
                            slug: team.slug.clone(),
                        },
                    );
                    self.slug_indexes.add_team_slug(&team.slug, team.id);
                    self.add_team_location(team.id, continent.id, country.id, club.id);

                    for player in &team.players.players {
                        self.add_player_location(
                            player.id,
                            continent.id,
                            country.id,
                            club.id,
                            team.id,
                        );
                    }

                    for staff in team.staffs.iter() {
                        self.add_staff_location(
                            staff.id,
                            continent.id,
                            country.id,
                            club.id,
                            team.id,
                        );
                    }
                }
            }
        }
    }

    fn merge_shard(&mut self, shard: SimulatorDataIndexes) {
        let SimulatorDataIndexes {
            league_indexes,
            club_indexes,
            team_indexes,
            player_indexes,
            staff_indexes,
            team_data_index,
            slug_indexes,
        } = shard;
        self.league_indexes.extend(league_indexes);
        self.club_indexes.extend(club_indexes);
        self.team_indexes.extend(team_indexes);
        self.player_indexes.extend(player_indexes);
        self.staff_indexes.extend(staff_indexes);
        self.team_data_index.extend(team_data_index);
        self.slug_indexes.merge(slug_indexes);
    }

    //league indexes
    pub fn add_league_location(&mut self, league_id: u32, continent_id: u32, country_id: u32) {
        self.league_indexes
            .insert(league_id, (continent_id, country_id));
    }

    pub fn get_league_location(&self, league_id: u32) -> Option<(u32, u32)> {
        match self.league_indexes.get(&league_id) {
            Some((league_continent_id, league_country_id)) => {
                Some((*league_continent_id, *league_country_id))
            }
            None => None,
        }
    }

    //club indexes

    pub fn add_club_location(&mut self, club_id: u32, continent_id: u32, country_id: u32) {
        self.club_indexes
            .insert(club_id, (continent_id, country_id));
    }

    pub fn get_club_location(&self, club_id: u32) -> Option<(u32, u32)> {
        match self.club_indexes.get(&club_id) {
            Some((club_continent_id, club_country_id)) => {
                Some((*club_continent_id, *club_country_id))
            }
            None => None,
        }
    }

    //team data indexes
    pub fn add_team_data(&mut self, team_id: u32, team_data: TeamData) {
        self.team_data_index.insert(team_id, team_data);
    }
    pub fn get_team_data(&self, team_id: u32) -> Option<&TeamData> {
        match self.team_data_index.get(&team_id) {
            Some(team_data) => Some(team_data),
            None => None,
        }
    }

    pub fn add_team_location(
        &mut self,
        team_id: u32,
        continent_id: u32,
        country_id: u32,
        club_id: u32,
    ) {
        self.team_indexes
            .insert(team_id, (continent_id, country_id, club_id));
    }

    pub fn get_team_location(&self, team_id: u32) -> Option<(u32, u32, u32)> {
        match self.team_indexes.get(&team_id) {
            Some((team_continent_id, team_country_id, team_club_id)) => {
                Some((*team_continent_id, *team_country_id, *team_club_id))
            }
            None => None,
        }
    }

    /// Rebuild only the player indexes (after transfers move players between clubs)
    pub fn refresh_player_indexes(&mut self, data: &SimulatorData) {
        // Build per-continent player-index shards in parallel; player ids
        // are globally unique so the merge is a disjoint `extend`.
        let shards: Vec<HashMap<u32, (u32, u32, u32, u32)>> = data
            .continents
            .par_iter()
            .map(|continent| {
                let mut shard: HashMap<u32, (u32, u32, u32, u32)> = HashMap::new();
                for country in &continent.countries {
                    for club in &country.clubs {
                        for team in &club.teams.teams {
                            for player in &team.players.players {
                                shard.insert(
                                    player.id,
                                    (continent.id, country.id, club.id, team.id),
                                );
                            }
                        }
                    }
                }
                shard
            })
            .collect();

        self.player_indexes.clear();
        for shard in shards {
            self.player_indexes.extend(shard);
        }
    }

    //player indexes

    pub fn add_player_location(
        &mut self,
        player_id: u32,
        continent_id: u32,
        country_id: u32,
        club_id: u32,
        team_id: u32,
    ) {
        self.player_indexes
            .insert(player_id, (continent_id, country_id, club_id, team_id));
    }

    pub fn get_player_location(&self, player_id: u32) -> Option<(u32, u32, u32, u32)> {
        match self.player_indexes.get(&player_id) {
            Some((player_continent_id, player_country_id, player_club_id, player_team_id)) => {
                Some((
                    *player_continent_id,
                    *player_country_id,
                    *player_club_id,
                    *player_team_id,
                ))
            }
            None => None,
        }
    }

    //staff indexes

    pub fn add_staff_location(
        &mut self,
        staff_id: u32,
        continent_id: u32,
        country_id: u32,
        club_id: u32,
        team_id: u32,
    ) {
        self.staff_indexes
            .insert(staff_id, (continent_id, country_id, club_id, team_id));
    }

    pub fn get_staff_location(&self, staff_id: u32) -> Option<(u32, u32, u32, u32)> {
        match self.staff_indexes.get(&staff_id) {
            Some((continent_id, country_id, club_id, team_id)) => {
                Some((*continent_id, *country_id, *club_id, *team_id))
            }
            None => None,
        }
    }
}

#[derive(Clone)]
pub struct SlugIndexes {
    country_slug_index: HashMap<String, u32>,
    league_slug_index: HashMap<String, u32>,
    team_slug_index: HashMap<String, u32>,
}

impl SlugIndexes {
    pub fn new() -> Self {
        SlugIndexes {
            country_slug_index: HashMap::new(),
            league_slug_index: HashMap::new(),
            team_slug_index: HashMap::new(),
        }
    }

    // team id slug index
    pub fn add_country_slug(&mut self, slug: &str, country_id: u32) {
        self.country_slug_index.insert(slug.into(), country_id);
    }
    pub fn get_country_by_slug(&self, slug: &str) -> Option<u32> {
        match self.country_slug_index.get(slug) {
            Some(country_id) => Some(*country_id),
            None => None,
        }
    }

    // team id slug index
    pub fn add_league_slug(&mut self, slug: &str, league_id: u32) {
        self.league_slug_index.insert(slug.into(), league_id);
    }
    pub fn get_league_by_slug(&self, slug: &str) -> Option<u32> {
        match self.league_slug_index.get(slug) {
            Some(league_id) => Some(*league_id),
            None => None,
        }
    }

    // team id slug index
    pub fn add_team_slug(&mut self, slug: &str, team_id: u32) {
        self.team_slug_index.insert(slug.into(), team_id);
    }
    pub fn get_team_by_slug(&self, slug: &str) -> Option<u32> {
        match self.team_slug_index.get(slug) {
            Some(team_id) => Some(*team_id),
            None => None,
        }
    }

    /// Absorb another shard's slug entries. Used during the parallel
    /// `SimulatorDataIndexes::refresh` merge — shards are populated by
    /// disjoint continents so collisions can't happen, but `extend` is
    /// still the right primitive in case the same id ever resurfaces in
    /// a later refresh after a transfer.
    pub fn merge(&mut self, other: SlugIndexes) {
        let SlugIndexes {
            country_slug_index,
            league_slug_index,
            team_slug_index,
        } = other;
        self.country_slug_index.extend(country_slug_index);
        self.league_slug_index.extend(league_slug_index);
        self.team_slug_index.extend(team_slug_index);
    }
}

#[derive(Clone)]
pub struct TeamData {
    pub name: String,
    pub slug: String,
}
