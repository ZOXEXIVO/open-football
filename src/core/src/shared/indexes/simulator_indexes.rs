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

    // Positional indices — array indices into nested vectors so
    // `player_mut`/`team_mut`/etc. avoid the per-call ID walk.
    // Keyed by entity id, value is the path of `usize` indices from
    // `data.continents` down. Refreshed by the same refresh pass that
    // populates the ID-based maps; fall back to a brute-force walk if
    // the position is stale (post-transfer before next refresh).
    pub player_positions: HashMap<u32, (u32, u32, u32, u32)>,
    pub team_positions: HashMap<u32, (u32, u32, u32, u32)>,
    pub club_positions: HashMap<u32, (u32, u32, u32)>,
    pub league_positions: HashMap<u32, (u32, u32, u32)>,
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
            player_positions: HashMap::new(),
            team_positions: HashMap::new(),
            club_positions: HashMap::new(),
            league_positions: HashMap::new(),
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
            .enumerate()
            .map(|(idx, continent)| {
                let mut shard = SimulatorDataIndexes::new();
                shard.fill_continent(continent, idx as u32);
                shard
            })
            .collect();

        // Clear positional maps before merging — array indices change
        // when a Vec shrinks (retirement, dissolved club), and a stale
        // tuple would silently point to the wrong entity. ID-based maps
        // are left as-is to preserve historical behaviour for callers
        // that rely on cross-refresh persistence.
        self.player_positions.clear();
        self.team_positions.clear();
        self.club_positions.clear();
        self.league_positions.clear();

        for shard in shards {
            self.merge_shard(shard);
        }
    }

    /// Populate this shard with every entity inside `continent`. Mirrors
    /// the layout the serial walk used to produce — kept here on the
    /// indexes type so the parallel and incremental paths can share a
    /// single canonical traversal.
    ///
    /// `continent_idx` is the position of `continent` inside
    /// `data.continents`; needed so the positional indices can record
    /// array offsets all the way from the root, not just from the
    /// continent down.
    fn fill_continent(&mut self, continent: &Continent, continent_idx: u32) {
        for (country_idx, country) in continent.countries.iter().enumerate() {
            let country_idx = country_idx as u32;
            self.slug_indexes
                .add_country_slug(&country.slug, country.id);

            for (league_idx, league) in country.leagues.leagues.iter().enumerate() {
                let league_idx = league_idx as u32;
                self.add_league_location(league.id, continent.id, country.id);
                self.add_league_position(league.id, continent_idx, country_idx, league_idx);
                self.slug_indexes.add_league_slug(&league.slug, league.id);
            }

            for (club_idx, club) in country.clubs.iter().enumerate() {
                let club_idx = club_idx as u32;
                self.add_club_location(club.id, continent.id, country.id);
                self.add_club_position(club.id, continent_idx, country_idx, club_idx);

                for (team_idx, team) in club.teams.teams.iter().enumerate() {
                    let team_idx = team_idx as u32;
                    self.add_team_data(
                        team.id,
                        TeamData {
                            name: team.name.clone(),
                            slug: team.slug.clone(),
                        },
                    );
                    self.slug_indexes.add_team_slug(&team.slug, team.id);
                    self.add_team_location(team.id, continent.id, country.id, club.id);
                    self.add_team_position(team.id, continent_idx, country_idx, club_idx, team_idx);

                    for player in &team.players.players {
                        self.add_player_location(
                            player.id,
                            continent.id,
                            country.id,
                            club.id,
                            team.id,
                        );
                        self.add_player_position(
                            player.id,
                            continent_idx,
                            country_idx,
                            club_idx,
                            team_idx,
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
            player_positions,
            team_positions,
            club_positions,
            league_positions,
        } = shard;
        self.league_indexes.extend(league_indexes);
        self.club_indexes.extend(club_indexes);
        self.team_indexes.extend(team_indexes);
        self.player_indexes.extend(player_indexes);
        self.staff_indexes.extend(staff_indexes);
        self.team_data_index.extend(team_data_index);
        self.slug_indexes.merge(slug_indexes);
        self.player_positions.extend(player_positions);
        self.team_positions.extend(team_positions);
        self.club_positions.extend(club_positions);
        self.league_positions.extend(league_positions);
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
        // are globally unique so the merge is a disjoint `extend`. Both
        // the ID-based map and the positional map are rebuilt here so a
        // post-transfer caller sees consistent state on either path.
        type IdShard = HashMap<u32, (u32, u32, u32, u32)>;
        type PosShard = HashMap<u32, (u32, u32, u32, u32)>;

        let shards: Vec<(IdShard, PosShard)> = data
            .continents
            .par_iter()
            .enumerate()
            .map(|(ci, continent)| {
                let ci = ci as u32;
                let mut id_shard: IdShard = HashMap::new();
                let mut pos_shard: PosShard = HashMap::new();
                for (coi, country) in continent.countries.iter().enumerate() {
                    let coi = coi as u32;
                    for (cli, club) in country.clubs.iter().enumerate() {
                        let cli = cli as u32;
                        for (ti, team) in club.teams.teams.iter().enumerate() {
                            let ti = ti as u32;
                            for player in &team.players.players {
                                id_shard.insert(
                                    player.id,
                                    (continent.id, country.id, club.id, team.id),
                                );
                                pos_shard.insert(player.id, (ci, coi, cli, ti));
                            }
                        }
                    }
                }
                (id_shard, pos_shard)
            })
            .collect();

        self.player_indexes.clear();
        self.player_positions.clear();
        for (id_shard, pos_shard) in shards {
            self.player_indexes.extend(id_shard);
            self.player_positions.extend(pos_shard);
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

    // Positional setters/getters. Tuple values are array indices
    // (continent_idx, country_idx, club_idx[, team_idx]) into the
    // nested `data.continents[..].countries[..].clubs[..].teams[..]`
    // structure. Used by `accessors.rs` to skip the per-call ID walk.

    pub fn add_player_position(
        &mut self,
        player_id: u32,
        continent_idx: u32,
        country_idx: u32,
        club_idx: u32,
        team_idx: u32,
    ) {
        self.player_positions
            .insert(player_id, (continent_idx, country_idx, club_idx, team_idx));
    }

    pub fn get_player_position(&self, player_id: u32) -> Option<(u32, u32, u32, u32)> {
        self.player_positions.get(&player_id).copied()
    }

    pub fn add_team_position(
        &mut self,
        team_id: u32,
        continent_idx: u32,
        country_idx: u32,
        club_idx: u32,
        team_idx: u32,
    ) {
        self.team_positions
            .insert(team_id, (continent_idx, country_idx, club_idx, team_idx));
    }

    pub fn get_team_position(&self, team_id: u32) -> Option<(u32, u32, u32, u32)> {
        self.team_positions.get(&team_id).copied()
    }

    pub fn add_club_position(
        &mut self,
        club_id: u32,
        continent_idx: u32,
        country_idx: u32,
        club_idx: u32,
    ) {
        self.club_positions
            .insert(club_id, (continent_idx, country_idx, club_idx));
    }

    pub fn get_club_position(&self, club_id: u32) -> Option<(u32, u32, u32)> {
        self.club_positions.get(&club_id).copied()
    }

    pub fn add_league_position(
        &mut self,
        league_id: u32,
        continent_idx: u32,
        country_idx: u32,
        league_idx: u32,
    ) {
        self.league_positions
            .insert(league_id, (continent_idx, country_idx, league_idx));
    }

    pub fn get_league_position(&self, league_id: u32) -> Option<(u32, u32, u32)> {
        self.league_positions.get(&league_id).copied()
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
