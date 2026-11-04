use entry::{PlayerHome, RosterEntry};
use place::{ClubPlace, CountryPlace, LeaguePlace, TeamPlace};
use roll::{RosterKey, RosterRoll};
use rustc_hash::FxHashMap;

mod country;
mod entry;
mod lookup;
mod place;
mod reconcile;
mod roll;
mod slug;
#[cfg(test)]
mod tests;

pub use country::CountryLookupIndex;
pub use reconcile::ReconcileStats;
pub use slug::SlugIndexes;

/// Where every country, league, club, team, player and staff member sits in
/// `SimulatorData::continents`, built once from the world and then kept by
/// [`reconcile`](Self::reconcile).
#[derive(Clone, Default, PartialEq)]
pub struct SimulatorDataIndexes {
    countries: FxHashMap<u32, CountryPlace>,
    leagues: FxHashMap<u32, LeaguePlace>,
    clubs: FxHashMap<u32, ClubPlace>,
    teams: FxHashMap<u32, TeamPlace>,
    players: FxHashMap<u32, PlayerHome>,
    staff: FxHashMap<u32, RosterEntry>,
    rolls: FxHashMap<RosterKey, RosterRoll>,
    pub slug_indexes: SlugIndexes,
}

impl SimulatorDataIndexes {
    pub fn get_player_location(&self, player_id: u32) -> Option<(u32, u32, u32, u32)> {
        self.players
            .get(&player_id)
            .and_then(|home| home.roster())
            .map(|entry| entry.location.ids())
    }

    pub fn get_staff_location(&self, staff_id: u32) -> Option<(u32, u32, u32, u32)> {
        self.staff.get(&staff_id).map(|entry| entry.location.ids())
    }

    /// The parts in which two indexes disagree.
    pub fn differences(&self, other: &Self) -> Vec<&'static str> {
        [
            ("countries", self.countries == other.countries),
            ("leagues", self.leagues == other.leagues),
            ("clubs", self.clubs == other.clubs),
            ("teams", self.teams == other.teams),
            ("players", self.players == other.players),
            ("staff", self.staff == other.staff),
            ("rolls", self.rolls == other.rolls),
            ("slugs", self.slug_indexes == other.slug_indexes),
        ]
        .into_iter()
        .filter(|(_, same)| !same)
        .map(|(part, _)| part)
        .collect()
    }
}
