use crate::shared::RosterRevision;
use rustc_hash::FxHashSet;

/// A roster the index keeps a roll for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum RosterKey {
    TeamPlayers(u32),
    TeamStaff(u32),
    Retired(u32),
    NationalPool(u32),
}

impl RosterKey {
    /// Whether the team or country that holds this roster is still in the world.
    pub(super) fn stands_in(self, teams: &FxHashSet<u32>, countries: &FxHashSet<u32>) -> bool {
        match self {
            RosterKey::TeamPlayers(team) | RosterKey::TeamStaff(team) => teams.contains(&team),
            RosterKey::Retired(country) | RosterKey::NationalPool(country) => {
                countries.contains(&country)
            }
        }
    }
}

/// What the index last saw of one roster: its revision and its members in
/// roster order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RosterRoll {
    revision: RosterRevision,
    members: Vec<u32>,
}

impl RosterRoll {
    pub(super) fn new(revision: RosterRevision, members: Vec<u32>) -> Self {
        RosterRoll { revision, members }
    }

    pub(super) fn is_current(&self, revision: RosterRevision) -> bool {
        self.revision == revision
    }

    pub(super) fn members(&self) -> &[u32] {
        &self.members
    }
}
