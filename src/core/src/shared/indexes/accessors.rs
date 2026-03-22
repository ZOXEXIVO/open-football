use crate::continent::Continent;
use crate::country::Country;
use crate::league::League;
use crate::{Club, Player, Staff, Team, SimulatorData};
use super::TeamData;

impl SimulatorData {
    pub fn continent(&self, id: u32) -> Option<&Continent> {
        self.continents.iter().find(|c| c.id == id)
    }

    pub fn continent_mut(&mut self, id: u32) -> Option<&mut Continent> {
        self.continents.iter_mut().find(|c| c.id == id)
    }

    pub fn country(&self, id: u32) -> Option<&Country> {
        self.continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == id)
    }

    pub fn country_mut(&mut self, id: u32) -> Option<&mut Country> {
        self.continents
            .iter_mut()
            .flat_map(|c| &mut c.countries)
            .find(|c| c.id == id)
    }

    pub fn league(&self, id: u32) -> Option<&League> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_league_location(id))
            .and_then(|(league_continent_id, league_country_id)| {
                self.continent(league_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter()
                            .find(|country| country.id == league_country_id)
                    })
                    .and_then(|country| {
                        country
                            .leagues
                            .leagues
                            .iter()
                            .find(|league| league.id == id)
                    })
            })
    }

    pub fn league_mut(&mut self, id: u32) -> Option<&mut League> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_league_location(id))
            .and_then(|(league_continent_id, league_country_id)| {
                self.continent_mut(league_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter_mut()
                            .find(|country| country.id == league_country_id)
                    })
                    .and_then(|country| {
                        country
                            .leagues
                            .leagues
                            .iter_mut()
                            .find(|league| league.id == id)
                    })
            })
    }

    pub fn team_data(&self, id: u32) -> Option<&TeamData> {
        self.indexes.as_ref().unwrap().get_team_data(id)
    }

    pub fn country_by_club(&self, club_id: u32) -> Option<&Country> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_club_location(club_id))
            .and_then(|(club_continent_id, club_country_id)| {
                self.continent(club_continent_id).and_then(|continent| {
                    continent
                        .countries
                        .iter()
                        .find(|country| country.id == club_country_id)
                })
            })
    }

    /// Get the continent a club belongs to
    pub fn continent_by_club(&self, club_id: u32) -> Option<&Continent> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_club_location(club_id))
            .and_then(|(continent_id, _)| self.continent(continent_id))
    }

    /// Get all continental competition matches (CL, EL, Conference) for a club.
    /// Returns (competition_name, home_club_id, away_club_id, date, match_id, result).
    pub fn continental_matches_for_club(&self, club_id: u32) -> Vec<(&str, u32, u32, chrono::NaiveDate, &str, Option<(u8, u8)>)> {
        let Some(continent) = self.continent_by_club(club_id) else {
            return Vec::new();
        };

        let cc = &continent.continental_competitions;
        let mut matches = Vec::new();

        for m in &cc.champions_league.matches {
            if m.home_team == club_id || m.away_team == club_id {
                matches.push(("Champions League", m.home_team, m.away_team, m.date, m.match_id.as_str(), m.result));
            }
        }
        for m in &cc.europa_league.matches {
            if m.home_team == club_id || m.away_team == club_id {
                matches.push(("Europa League", m.home_team, m.away_team, m.date, m.match_id.as_str(), m.result));
            }
        }
        for m in &cc.conference_league.matches {
            if m.home_team == club_id || m.away_team == club_id {
                matches.push(("Conference League", m.home_team, m.away_team, m.date, m.match_id.as_str(), m.result));
            }
        }

        matches
    }

    pub fn club(&self, id: u32) -> Option<&Club> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_club_location(id))
            .and_then(|(club_continent_id, club_country_id)| {
                self.continent(club_continent_id).and_then(|continent| {
                    continent
                        .countries
                        .iter()
                        .find(|country| country.id == club_country_id)
                })
            })
            .and_then(|country| country.clubs.iter().find(|club| club.id == id))
    }

    pub fn club_mut(&mut self, id: u32) -> Option<&mut Club> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_club_location(id))
            .and_then(|(club_continent_id, club_country_id)| {
                self.continent_mut(club_continent_id).and_then(|continent| {
                    continent
                        .countries
                        .iter_mut()
                        .find(|country| country.id == club_country_id)
                })
            })
            .and_then(|country| country.clubs.iter_mut().find(|club| club.id == id))
    }

    pub fn team(&self, id: u32) -> Option<&Team> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_team_location(id))
            .and_then(|(team_continent_id, team_country_id, team_club_id)| {
                self.continent(team_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter()
                            .find(|country| country.id == team_country_id)
                    })
                    .and_then(|country| country.clubs.iter().find(|club| club.id == team_club_id))
                    .and_then(|club| club.teams.teams.iter().find(|team| team.id == id))
            })
    }

    pub fn team_mut(&mut self, id: u32) -> Option<&mut Team> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_team_location(id))
            .and_then(|(team_continent_id, team_country_id, team_club_id)| {
                self.continent_mut(team_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter_mut()
                            .find(|country| country.id == team_country_id)
                    })
                    .and_then(|country| {
                        country
                            .clubs
                            .iter_mut()
                            .find(|club| club.id == team_club_id)
                    })
                    .and_then(|club| club.teams.teams.iter_mut().find(|team| team.id == id))
            })
    }

    pub fn player(&self, id: u32) -> Option<&Player> {
        // Fast path: indexed lookup
        if let Some((player_continent_id, player_country_id, player_club_id, player_team_id)) =
            self.indexes.as_ref().and_then(|indexes| indexes.get_player_location(id))
        {
            let found = self
                .continent(player_continent_id)
                .and_then(|continent| {
                    continent
                        .countries
                        .iter()
                        .find(|country| country.id == player_country_id)
                })
                .and_then(|country| country.clubs.iter().find(|club| club.id == player_club_id))
                .and_then(|club| {
                    club.teams
                        .teams
                        .iter()
                        .find(|team| team.id == player_team_id)
                })
                .and_then(|team| team.players.players.iter().find(|c| c.id == id));

            if found.is_some() {
                return found;
            }
        }

        // Fallback: brute-force scan (index may be stale after transfers)
        self.player_brute_force(id)
    }

    pub fn player_with_team(&self, player_id: u32) -> Option<(&Player, &Team)> {
        // Fast path: indexed lookup
        if let Some((continent_id, country_id, club_id, team_id)) =
            self.indexes.as_ref().and_then(|idx| idx.get_player_location(player_id))
        {
            let result = self
                .continent(continent_id)
                .and_then(|c| c.countries.iter().find(|co| co.id == country_id))
                .and_then(|co| co.clubs.iter().find(|cl| cl.id == club_id))
                .and_then(|cl| cl.teams.teams.iter().find(|t| t.id == team_id))
                .and_then(|team| {
                    team.players.players.iter().find(|p| p.id == player_id).map(|p| (p, team))
                });

            if result.is_some() {
                return result;
            }
        }

        // Fallback: brute-force
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if let Some(player) = team.players.players.iter().find(|p| p.id == player_id) {
                            return Some((player, team));
                        }
                    }
                }
            }
        }
        None
    }

    fn player_brute_force(&self, id: u32) -> Option<&Player> {
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if let Some(player) = team.players.players.iter().find(|p| p.id == id) {
                            return Some(player);
                        }
                    }
                }
                // Also check retired players
                if let Some(player) = country.retired_players.iter().find(|p| p.id == id) {
                    return Some(player);
                }
                // Also check national team generated (synthetic) players
                if let Some(player) = country.national_team.generated_squad.iter().find(|p| p.id == id) {
                    return Some(player);
                }
            }
        }
        None
    }

    /// Find a retired player by ID across all countries.
    pub fn retired_player(&self, id: u32) -> Option<&Player> {
        for continent in &self.continents {
            for country in &continent.countries {
                if let Some(player) = country.retired_players.iter().find(|p| p.id == id) {
                    return Some(player);
                }
            }
        }
        None
    }

    pub fn player_mut(&mut self, id: u32) -> Option<&mut Player> {
        // Phase 1: immutable search for array indices
        let pos = self.find_player_position(id);

        // Phase 2: mutable access at known position
        let (ci, coi, cli, ti) = pos?;
        self.continents[ci].countries[coi].clubs[cli]
            .teams.teams[ti].players.players.iter_mut().find(|p| p.id == id)
    }

    pub fn find_player_position(&self, id: u32) -> Option<(usize, usize, usize, usize)> {
        // Fast path: indexed lookup
        if let Some((pc, pco, pcl, pt)) =
            self.indexes.as_ref().and_then(|indexes| indexes.get_player_location(id))
        {
            for (ci, continent) in self.continents.iter().enumerate() {
                if continent.id != pc { continue; }
                for (coi, country) in continent.countries.iter().enumerate() {
                    if country.id != pco { continue; }
                    for (cli, club) in country.clubs.iter().enumerate() {
                        if club.id != pcl { continue; }
                        for (ti, team) in club.teams.teams.iter().enumerate() {
                            if team.id != pt { continue; }
                            if team.players.players.iter().any(|p| p.id == id) {
                                return Some((ci, coi, cli, ti));
                            }
                        }
                    }
                }
            }
        }

        // Fallback: brute-force scan (index may be stale after transfers)
        for (ci, continent) in self.continents.iter().enumerate() {
            for (coi, country) in continent.countries.iter().enumerate() {
                for (cli, club) in country.clubs.iter().enumerate() {
                    for (ti, team) in club.teams.teams.iter().enumerate() {
                        if team.players.players.iter().any(|p| p.id == id) {
                            return Some((ci, coi, cli, ti));
                        }
                    }
                }
            }
        }
        None
    }

    /// Find the array indices (continent, country, club, main team) for a club by ID.
    pub fn find_club_main_team(&self, club_id: u32) -> Option<(usize, usize, usize, usize)> {
        for (ci, continent) in self.continents.iter().enumerate() {
            for (coi, country) in continent.countries.iter().enumerate() {
                for (cli, club) in country.clubs.iter().enumerate() {
                    if club.id == club_id {
                        // Find the main team (first team, or index 0 as fallback)
                        let ti = club.teams.teams.iter().position(|t| {
                            t.team_type == crate::TeamType::Main
                        }).unwrap_or(0);
                        return Some((ci, coi, cli, ti));
                    }
                }
            }
        }
        None
    }

    /// Rebuild player indexes after a transfer/loan move.
    pub fn rebuild_indexes(&mut self) {
        if let Some(mut indexes) = self.indexes.take() {
            indexes.refresh_player_indexes(self);
            self.indexes = Some(indexes);
        }
    }

    pub fn staff_with_team(&self, staff_id: u32) -> Option<(&Staff, &Team)> {
        // Fast path: indexed lookup
        if let Some((continent_id, country_id, club_id, team_id)) =
            self.indexes.as_ref().and_then(|idx| idx.get_staff_location(staff_id))
        {
            let result = self
                .continent(continent_id)
                .and_then(|c| c.countries.iter().find(|co| co.id == country_id))
                .and_then(|co| co.clubs.iter().find(|cl| cl.id == club_id))
                .and_then(|cl| cl.teams.teams.iter().find(|t| t.id == team_id))
                .and_then(|team| {
                    team.staffs.staffs.iter().find(|s| s.id == staff_id).map(|s| (s, team))
                });

            if result.is_some() {
                return result;
            }
        }

        // Fallback: brute-force
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if let Some(staff) = team.staffs.staffs.iter().find(|s| s.id == staff_id) {
                            return Some((staff, team));
                        }
                    }
                }
            }
        }
        None
    }

    /// Find all clubs that have this player in their scouting assignments,
    /// shortlists, or scouting reports (i.e. clubs interested in signing the player).
    /// Returns Vec of (club_id, club_name, team_slug).
    pub fn clubs_interested_in_player(&self, player_id: u32) -> Vec<(u32, String, String)> {
        let mut interested = Vec::new();

        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    let mut is_interested = false;

                    // Check scouting assignments for observations of this player
                    for assignment in &club.transfer_plan.scouting_assignments {
                        if assignment.observations.iter().any(|o| o.player_id == player_id) {
                            is_interested = true;
                            break;
                        }
                    }

                    // Check scouting reports
                    if !is_interested {
                        if club.transfer_plan.scouting_reports.iter().any(|r| r.player_id == player_id) {
                            is_interested = true;
                        }
                    }

                    // Check shortlists
                    if !is_interested {
                        for shortlist in &club.transfer_plan.shortlists {
                            if shortlist.candidates.iter().any(|c| c.player_id == player_id) {
                                is_interested = true;
                                break;
                            }
                        }
                    }

                    // Check staff recommendations
                    if !is_interested {
                        if club.transfer_plan.staff_recommendations.iter().any(|r| r.player_id == player_id) {
                            is_interested = true;
                        }
                    }

                    if is_interested {
                        let team_slug = club.teams.teams.first()
                            .map(|t| t.slug.clone())
                            .unwrap_or_default();
                        interested.push((club.id, club.name.clone(), team_slug));
                    }
                }
            }
        }

        interested
    }
}
