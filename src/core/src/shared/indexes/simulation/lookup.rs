use super::ReconcileStats;
use super::entry::{PlayerHome, RosterEntry};
use crate::continent::Continent;
use crate::country::Country;
use crate::league::League;
use crate::{Club, Player, SimulatorData, Staff, Team};

impl SimulatorData {
    pub fn continent(&self, id: u32) -> Option<&Continent> {
        self.continents.iter().find(|c| c.id == id)
    }

    pub fn continent_mut(&mut self, id: u32) -> Option<&mut Continent> {
        self.continents.iter_mut().find(|c| c.id == id)
    }

    pub fn country(&self, id: u32) -> Option<&Country> {
        if let Some(country) = self
            .indexes
            .as_ref()
            .and_then(|indexes| indexes.countries.get(&id))
            .and_then(|place| place.resolve(&self.continents, id))
        {
            return Some(country);
        }
        self.continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == id)
    }

    pub fn country_mut(&mut self, id: u32) -> Option<&mut Country> {
        if let Some(place) = self
            .indexes
            .as_ref()
            .and_then(|indexes| indexes.countries.get(&id).copied())
            && place.resolve(&self.continents, id).is_some()
        {
            return place.resolve_mut(&mut self.continents, id);
        }
        self.continents
            .iter_mut()
            .flat_map(|c| &mut c.countries)
            .find(|c| c.id == id)
    }

    pub fn league(&self, id: u32) -> Option<&League> {
        let place = self.indexes.as_ref()?.leagues.get(&id)?;
        place.resolve(&self.continents, id).or_else(|| {
            self.continents
                .iter()
                .flat_map(|c| &c.countries)
                .find_map(|country| {
                    country.league(id).or_else(|| {
                        country
                            .playoffs
                            .iter()
                            .map(|playoff| &playoff.league)
                            .find(|league| league.id == id)
                    })
                })
        })
    }

    pub fn league_mut(&mut self, id: u32) -> Option<&mut League> {
        let place = *self.indexes.as_ref()?.leagues.get(&id)?;
        if place.resolve(&self.continents, id).is_some() {
            return place.resolve_mut(&mut self.continents, id);
        }
        let country = self
            .continents
            .iter_mut()
            .flat_map(|c| &mut c.countries)
            .find(|country| {
                country.league(id).is_some()
                    || country
                        .playoffs
                        .iter()
                        .any(|playoff| playoff.league.id == id)
            })?;
        if country.league(id).is_some() {
            return country.league_mut(id);
        }
        country
            .playoffs
            .iter_mut()
            .map(|playoff| &mut playoff.league)
            .find(|league| league.id == id)
    }

    // A club never changes country, so its recorded country answers without
    // checking the club itself.
    pub fn country_by_club(&self, club_id: u32) -> Option<&Country> {
        self.indexes
            .as_ref()?
            .clubs
            .get(&club_id)?
            .country_place()
            .country(&self.continents)
    }

    /// Get the continent a club belongs to
    pub fn continent_by_club(&self, club_id: u32) -> Option<&Continent> {
        self.indexes
            .as_ref()?
            .clubs
            .get(&club_id)?
            .country_place()
            .continent(&self.continents)
    }

    pub fn club(&self, id: u32) -> Option<&Club> {
        let place = self.indexes.as_ref()?.clubs.get(&id)?;
        place.resolve(&self.continents, id).or_else(|| {
            place
                .country_place()
                .country(&self.continents)?
                .clubs
                .iter()
                .find(|club| club.id == id)
        })
    }

    pub fn club_mut(&mut self, id: u32) -> Option<&mut Club> {
        let place = *self.indexes.as_ref()?.clubs.get(&id)?;
        if place.resolve(&self.continents, id).is_some() {
            return place.resolve_mut(&mut self.continents, id);
        }
        place
            .country_place()
            .country_mut(&mut self.continents)?
            .clubs
            .iter_mut()
            .find(|club| club.id == id)
    }

    pub fn team(&self, id: u32) -> Option<&Team> {
        let place = self.indexes.as_ref()?.teams.get(&id)?;
        place.resolve(&self.continents, id).or_else(|| {
            place
                .club_place()
                .country_place()
                .country(&self.continents)?
                .clubs
                .iter()
                .find_map(|club| club.teams.find(id))
        })
    }

    pub fn team_mut(&mut self, id: u32) -> Option<&mut Team> {
        let place = *self.indexes.as_ref()?.teams.get(&id)?;
        if place.resolve(&self.continents, id).is_some() {
            return place.resolve_mut(&mut self.continents, id);
        }
        place
            .club_place()
            .country_place()
            .country_mut(&mut self.continents)?
            .clubs
            .iter_mut()
            .find_map(|club| club.teams.find_mut(id))
    }

    pub fn player(&self, id: u32) -> Option<&Player> {
        if let Some((_, team, slot)) = self.indexed_player(id) {
            return Some(&team.players.players[slot]);
        }
        // A pooled free agent is never on the index; look there before
        // walking the world.
        if let Some(player) = self.free_agents.find(id) {
            return Some(player);
        }
        if let Some(player) = self.indexed_listed_player(id) {
            return Some(player);
        }
        // Fallback: brute-force scan (index may be stale after transfers)
        self.player_brute_force(id)
    }

    /// A retired player or a national team's generated one, from the place
    /// the index recorded for him.
    fn indexed_listed_player(&self, id: u32) -> Option<&Player> {
        match *self.indexes.as_ref()?.players.get(&id)? {
            PlayerHome::Roster(_) => None,
            PlayerHome::Retired(place) => {
                place.find(&self.continents, |country| &country.retired_players, id)
            }
            PlayerHome::NationalPool(place) => place.find(
                &self.continents,
                |country| &country.national_team.generated_squad,
                id,
            ),
        }
    }

    pub fn player_with_team(&self, player_id: u32) -> Option<(&Player, &Team)> {
        if let Some((_, team, slot)) = self.indexed_player(player_id) {
            return Some((&team.players.players[slot], team));
        }
        // A retired or national generated player has no team.
        if self.indexed_listed_player(player_id).is_some() {
            return None;
        }

        // Fallback: brute-force
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if let Some(player) = team.players.find(player_id) {
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
                        if let Some(player) = team.players.find(id) {
                            return Some(player);
                        }
                    }
                }
                // Also check retired players
                if let Some(player) = country.retired_players.iter().find(|p| p.id == id) {
                    return Some(player);
                }
                // Also check national team generated (synthetic) players
                if let Some(player) = country
                    .national_team
                    .generated_squad
                    .iter()
                    .find(|p| p.id == id)
                {
                    return Some(player);
                }
            }
        }
        self.free_agents.find(id)
    }

    /// Find a retired player by ID across all countries.
    pub fn retired_player(&self, id: u32) -> Option<&Player> {
        if let Some(PlayerHome::Retired(place)) = self
            .indexes
            .as_ref()
            .and_then(|indexes| indexes.players.get(&id))
            .copied()
            && let Some(player) =
                place.find(&self.continents, |country| &country.retired_players, id)
        {
            return Some(player);
        }
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
        // Phase 1: immutable search for the player's place, his own slot
        // in the team included, so the team is scanned once.
        if let Some((ci, coi, cli, ti, slot)) = self.indexed_player_slot(id) {
            return Some(
                &mut self.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
                    .players
                    .players[slot],
            );
        }
        if let Some(slot) = self.free_agents.slot_of(id) {
            return Some(&mut self.free_agents[slot]);
        }
        let (ci, coi, cli, ti) = self.find_player_position(id)?;
        self.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
            .players
            .find_mut(id)
    }

    /// Where the index has this player — `(country id, club, team, slot)` —
    /// when he is still there. `None` only says the index cannot answer: a
    /// player moved since the last rebuild has to be looked for.
    pub(crate) fn indexed_player_place(&self, id: u32) -> Option<(u32, usize, usize, usize)> {
        let (ci, coi, cli, ti, slot) = self.indexed_player_slot(id)?;
        Some((self.continents[ci].countries[coi].id, cli, ti, slot))
    }

    /// The index's place for this player and his slot in that team, if he
    /// is still there.
    fn indexed_player_slot(&self, id: u32) -> Option<(usize, usize, usize, usize, usize)> {
        let (entry, _, slot) = self.indexed_player(id)?;
        let (ci, coi, cli, ti) = entry.place.team_place().indices();
        Some((ci, coi, cli, ti, slot))
    }

    fn indexed_player(&self, id: u32) -> Option<(RosterEntry, &Team, usize)> {
        let entry = self.indexes.as_ref()?.players.get(&id)?.roster()?;
        let team = entry.team(&self.continents)?;
        let slot = entry
            .place
            .slot_in(&team.players.players, |player| player.id == id)?;
        Some((entry, team, slot))
    }

    pub fn find_player_position(&self, id: u32) -> Option<(usize, usize, usize, usize)> {
        if let Some((ci, coi, cli, ti, _)) = self.indexed_player_slot(id) {
            return Some((ci, coi, cli, ti));
        }

        // Fallback: brute-force scan (index may be stale after transfers)
        for (ci, continent) in self.continents.iter().enumerate() {
            for (coi, country) in continent.countries.iter().enumerate() {
                for (cli, club) in country.clubs.iter().enumerate() {
                    for (ti, team) in club.teams.iter().enumerate() {
                        if team.players.contains(id) {
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
        if let Some(place) = self
            .indexes
            .as_ref()
            .and_then(|indexes| indexes.clubs.get(&club_id))
            && let Some(club) = place.resolve(&self.continents, club_id)
        {
            let (ci, coi, cli) = place.indices();
            return Some((ci, coi, cli, club.teams.main_index().unwrap_or(0)));
        }
        for (ci, continent) in self.continents.iter().enumerate() {
            for (coi, country) in continent.countries.iter().enumerate() {
                for (cli, club) in country.clubs.iter().enumerate() {
                    if club.id == club_id {
                        let ti = club.teams.main_index().unwrap_or(0);
                        return Some((ci, coi, cli, ti));
                    }
                }
            }
        }
        None
    }

    /// Bring the indexes up to date after entities moved.
    pub fn reconcile_indexes(&mut self) -> ReconcileStats {
        let Some(mut indexes) = self.indexes.take() else {
            return ReconcileStats::default();
        };
        let stats = indexes.reconcile(self);
        self.indexes = Some(indexes);
        stats
    }

    pub fn staff_with_team(&self, staff_id: u32) -> Option<(&Staff, &Team)> {
        if let Some(entry) = self
            .indexes
            .as_ref()
            .and_then(|indexes| indexes.staff.get(&staff_id))
            && let Some(team) = entry.team(&self.continents)
            && let Some(slot) = entry
                .place
                .slot_in(&team.staffs.staffs, |staff| staff.id == staff_id)
        {
            return Some((&team.staffs.staffs[slot], team));
        }

        // Fallback: brute-force
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in club.teams.iter() {
                        if let Some(staff) = team.staffs.find(staff_id) {
                            return Some((staff, team));
                        }
                    }
                }
            }
        }
        None
    }
}
