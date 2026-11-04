use super::roll::RosterRoll;
use crate::country::Country;
use rustc_hash::FxHashMap;

/// One country's players, teams and clubs by id, as positions inside
/// `Country::clubs`, for the country's own result fan-out. The country keeps
/// it between ticks and reconciles it before each use, so it answers what a
/// lookup built at that moment would.
#[derive(Clone, Default, PartialEq)]
pub struct CountryLookupIndex {
    players: FxHashMap<u32, (u16, u8, u16)>,
    teams: FxHashMap<u32, (u16, u8)>,
    clubs: FxHashMap<u32, u16>,
    rolls: FxHashMap<u32, RosterRoll>,
}

impl CountryLookupIndex {
    pub fn build(country: &Country) -> Self {
        let mut lookup = CountryLookupIndex::default();
        lookup.reconcile(country);
        lookup
    }

    /// Bring the lookup up to date with `country`, walking only the rosters
    /// whose membership changed since it last saw them. Returns how many
    /// players it visited.
    pub fn reconcile(&mut self, country: &Country) -> usize {
        let mut changed = Vec::new();
        let mut clubs_seen = 0;
        let mut teams_seen = 0;
        let mut visited = 0;
        for (club_index, club) in country.clubs.iter().enumerate() {
            let club_place = u16::try_from(club_index).unwrap_or(u16::MAX);
            clubs_seen += 1;
            self.clubs.insert(club.id, club_place);
            for (team_index, team) in club.teams.teams.iter().enumerate() {
                let team_place = (club_place, u8::try_from(team_index).unwrap_or(u8::MAX));
                teams_seen += 1;
                let moved = self.teams.insert(team.id, team_place) != Some(team_place);
                let revision = team.players.revision();
                if !moved
                    && self
                        .rolls
                        .get(&team.id)
                        .is_some_and(|roll| roll.is_current(revision))
                {
                    continue;
                }
                let members: Vec<u32> = team.players.players.iter().map(|p| p.id).collect();
                for (slot, &id) in members.iter().enumerate() {
                    let slot = u16::try_from(slot).unwrap_or(u16::MAX);
                    self.players.insert(id, (team_place.0, team_place.1, slot));
                }
                visited += members.len();
                changed.push((team.id, team_place, revision, members));
            }
        }

        // A club or team left the country: never seen in a simulated
        // world, so it pays for a rebuild.
        if self.clubs.len() != clubs_seen || self.teams.len() != teams_seen {
            *self = CountryLookupIndex::default();
            return self.reconcile(country);
        }

        // Every arrival is in before any departure is removed, so a player
        // who moved between two changed rosters keeps his new place.
        for (team_id, team_place, revision, members) in changed {
            let departed: Vec<u32> = self
                .rolls
                .get(&team_id)
                .map(|roll| {
                    roll.members()
                        .iter()
                        .copied()
                        .filter(|id| !members.contains(id))
                        .collect()
                })
                .unwrap_or_default();
            for id in departed {
                if self
                    .players
                    .get(&id)
                    .is_some_and(|&(club, team, _)| (club, team) == team_place)
                {
                    self.players.remove(&id);
                }
            }
            self.rolls
                .insert(team_id, RosterRoll::new(revision, members));
        }
        visited
    }

    /// `(club, team, slot)` inside `Country::clubs`.
    pub fn player(&self, id: u32) -> Option<(usize, usize, usize)> {
        self.players
            .get(&id)
            .map(|&(club, team, slot)| (club as usize, team as usize, slot as usize))
    }

    /// `(club, team)` inside `Country::clubs`.
    pub fn team(&self, id: u32) -> Option<(usize, usize)> {
        self.teams
            .get(&id)
            .map(|&(club, team)| (club as usize, team as usize))
    }

    pub fn club(&self, id: u32) -> Option<usize> {
        self.clubs.get(&id).map(|&club| club as usize)
    }
}
