use crate::SimulatorData;
use std::collections::HashMap;

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
        for continent in &data.continents {
            for country in &continent.countries {
                self.slug_indexes
                    .add_country_slug(&country.slug, country.id);

                //fill leagues
                for league in &country.leagues.leagues {
                    self.add_league_location(league.id, continent.id, country.id);

                    self.slug_indexes.add_league_slug(&league.slug, league.id);
                }

                //fill teams
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

                        for staff in &team.staffs.staffs {
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
}

pub struct TeamData {
    pub name: String,
    pub slug: String,
}
