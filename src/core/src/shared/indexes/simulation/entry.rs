use super::place::{CountryPlace, RosterPlace};
use crate::continent::Continent;
use crate::country::Country;
use crate::{Player, PlayerCollection, Team};

/// The ids around a person. Parallel country passes read them without
/// access to the world, so they are stored, not derived from the place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PersonLocation {
    continent_id: u32,
    country_id: u32,
    club_id: u32,
    team_id: u32,
}

impl PersonLocation {
    pub(super) fn new(continent_id: u32, country_id: u32, club_id: u32, team_id: u32) -> Self {
        PersonLocation {
            continent_id,
            country_id,
            club_id,
            team_id,
        }
    }

    pub(super) fn ids(self) -> (u32, u32, u32, u32) {
        (
            self.continent_id,
            self.country_id,
            self.club_id,
            self.team_id,
        )
    }
}

/// A player or staff member on a team: where he sits and whose he is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RosterEntry {
    pub(super) place: RosterPlace,
    pub(super) location: PersonLocation,
}

impl RosterEntry {
    pub(super) fn new(place: RosterPlace, location: PersonLocation) -> Self {
        RosterEntry { place, location }
    }

    /// The team at the recorded place, while it is still the team the
    /// location names.
    pub(super) fn team<'a>(&self, continents: &'a [Continent]) -> Option<&'a Team> {
        self.place
            .team_place()
            .resolve(continents, self.location.team_id)
    }
}

/// Where a player lives: on a team, or in one of the lists a country keeps
/// beside its clubs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlayerHome {
    Roster(RosterEntry),
    Retired(PoolPlace),
    NationalPool(PoolPlace),
}

impl PlayerHome {
    pub(super) fn roster(self) -> Option<RosterEntry> {
        match self {
            PlayerHome::Roster(entry) => Some(entry),
            PlayerHome::Retired(_) | PlayerHome::NationalPool(_) => None,
        }
    }
}

/// A player's slot in one of his country's lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PoolPlace {
    country: CountryPlace,
    slot: u32,
}

impl PoolPlace {
    pub(super) fn new(country: CountryPlace, slot: usize) -> Self {
        PoolPlace {
            country,
            slot: u32::try_from(slot).unwrap_or(u32::MAX),
        }
    }

    /// The player in the recorded country's `list`: at the recorded slot
    /// while it still holds him, otherwise wherever he now stands in it.
    pub(super) fn find<'a>(
        self,
        continents: &'a [Continent],
        list: impl Fn(&'a Country) -> &'a PlayerCollection,
        id: u32,
    ) -> Option<&'a Player> {
        let players = &list(self.country.country(continents)?).players;
        if let Some(player) = players.get(self.slot as usize).filter(|p| p.id == id) {
            return Some(player);
        }
        players.iter().find(|p| p.id == id)
    }
}
