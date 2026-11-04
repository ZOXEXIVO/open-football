use crate::continent::Continent;
use crate::country::Country;
use crate::league::League;
use crate::{Club, Team};

/// Array positions from `SimulatorData::continents` down. A position too
/// large for its field is stored as the field's `MAX`, which never holds
/// the requested id, so the lookup falls back instead of reading a
/// neighbour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CountryPlace {
    continent: u8,
    country: u16,
}

impl CountryPlace {
    pub(super) fn new(continent: usize, country: usize) -> Self {
        CountryPlace {
            continent: u8::try_from(continent).unwrap_or(u8::MAX),
            country: u16::try_from(country).unwrap_or(u16::MAX),
        }
    }

    pub(super) fn indices(self) -> (usize, usize) {
        (self.continent as usize, self.country as usize)
    }

    pub(super) fn continent(self, continents: &[Continent]) -> Option<&Continent> {
        continents.get(self.continent as usize)
    }

    pub(super) fn country(self, continents: &[Continent]) -> Option<&Country> {
        self.continent(continents)?
            .countries
            .get(self.country as usize)
    }

    pub(super) fn country_mut(self, continents: &mut [Continent]) -> Option<&mut Country> {
        continents
            .get_mut(self.continent as usize)?
            .countries
            .get_mut(self.country as usize)
    }

    pub(super) fn resolve(self, continents: &[Continent], id: u32) -> Option<&Country> {
        self.country(continents).filter(|country| country.id == id)
    }

    pub(super) fn resolve_mut(self, continents: &mut [Continent], id: u32) -> Option<&mut Country> {
        self.country_mut(continents)
            .filter(|country| country.id == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LeagueSeat {
    Standings(u16),
    DomesticCup,
    Playoff(u16),
}

impl LeagueSeat {
    pub(super) fn standings(index: usize) -> Self {
        LeagueSeat::Standings(u16::try_from(index).unwrap_or(u16::MAX))
    }

    pub(super) fn playoff(index: usize) -> Self {
        LeagueSeat::Playoff(u16::try_from(index).unwrap_or(u16::MAX))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LeaguePlace {
    country: CountryPlace,
    seat: LeagueSeat,
}

impl LeaguePlace {
    pub(super) fn new(country: CountryPlace, seat: LeagueSeat) -> Self {
        LeaguePlace { country, seat }
    }

    pub(super) fn resolve(self, continents: &[Continent], id: u32) -> Option<&League> {
        let country = self.country.country(continents)?;
        match self.seat {
            LeagueSeat::Standings(index) => country.leagues.leagues.get(index as usize),
            LeagueSeat::DomesticCup => country.domestic_cup.as_ref().map(|cup| &cup.league),
            LeagueSeat::Playoff(index) => country
                .playoffs
                .get(index as usize)
                .map(|playoff| &playoff.league),
        }
        .filter(|league| league.id == id)
    }

    pub(super) fn resolve_mut(self, continents: &mut [Continent], id: u32) -> Option<&mut League> {
        let country = self.country.country_mut(continents)?;
        match self.seat {
            LeagueSeat::Standings(index) => country.leagues.leagues.get_mut(index as usize),
            LeagueSeat::DomesticCup => country.domestic_cup.as_mut().map(|cup| &mut cup.league),
            LeagueSeat::Playoff(index) => country
                .playoffs
                .get_mut(index as usize)
                .map(|playoff| &mut playoff.league),
        }
        .filter(|league| league.id == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ClubPlace {
    continent: u8,
    country: u16,
    club: u16,
}

impl ClubPlace {
    pub(super) fn new(country: CountryPlace, club: usize) -> Self {
        ClubPlace {
            continent: country.continent,
            country: country.country,
            club: u16::try_from(club).unwrap_or(u16::MAX),
        }
    }

    pub(super) fn country_place(self) -> CountryPlace {
        CountryPlace {
            continent: self.continent,
            country: self.country,
        }
    }

    pub(super) fn indices(self) -> (usize, usize, usize) {
        let (continent, country) = self.country_place().indices();
        (continent, country, self.club as usize)
    }

    fn club(self, continents: &[Continent]) -> Option<&Club> {
        self.country_place()
            .country(continents)?
            .clubs
            .get(self.club as usize)
    }

    fn club_mut(self, continents: &mut [Continent]) -> Option<&mut Club> {
        self.country_place()
            .country_mut(continents)?
            .clubs
            .get_mut(self.club as usize)
    }

    pub(super) fn resolve(self, continents: &[Continent], id: u32) -> Option<&Club> {
        self.club(continents).filter(|club| club.id == id)
    }

    pub(super) fn resolve_mut(self, continents: &mut [Continent], id: u32) -> Option<&mut Club> {
        self.club_mut(continents).filter(|club| club.id == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TeamPlace {
    continent: u8,
    team: u8,
    country: u16,
    club: u16,
}

impl TeamPlace {
    pub(super) fn new(club: ClubPlace, team: usize) -> Self {
        TeamPlace {
            continent: club.continent,
            team: u8::try_from(team).unwrap_or(u8::MAX),
            country: club.country,
            club: club.club,
        }
    }

    pub(super) fn club_place(self) -> ClubPlace {
        ClubPlace {
            continent: self.continent,
            country: self.country,
            club: self.club,
        }
    }

    pub(super) fn indices(self) -> (usize, usize, usize, usize) {
        let (continent, country, club) = self.club_place().indices();
        (continent, country, club, self.team as usize)
    }

    pub(super) fn resolve(self, continents: &[Continent], id: u32) -> Option<&Team> {
        self.club_place()
            .club(continents)?
            .teams
            .teams
            .get(self.team as usize)
            .filter(|team| team.id == id)
    }

    pub(super) fn resolve_mut(self, continents: &mut [Continent], id: u32) -> Option<&mut Team> {
        self.club_place()
            .club_mut(continents)?
            .teams
            .teams
            .get_mut(self.team as usize)
            .filter(|team| team.id == id)
    }
}

/// A person's team plus his slot in that team's roster. The slot is a
/// hint: a roster that lost someone listed earlier shifts everyone after
/// him.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RosterPlace {
    continent: u8,
    team: u8,
    country: u16,
    club: u16,
    slot: u16,
}

impl RosterPlace {
    pub(super) fn new(team: TeamPlace, slot: usize) -> Self {
        RosterPlace {
            continent: team.continent,
            team: team.team,
            country: team.country,
            club: team.club,
            slot: u16::try_from(slot).unwrap_or(u16::MAX),
        }
    }

    pub(super) fn team_place(self) -> TeamPlace {
        TeamPlace {
            continent: self.continent,
            team: self.team,
            country: self.country,
            club: self.club,
        }
    }

    /// The person's slot in `roster`: the recorded one while it still
    /// holds him, otherwise wherever he now stands in the same roster.
    pub(super) fn slot_in<T>(self, roster: &[T], is_person: impl Fn(&T) -> bool) -> Option<usize> {
        let recorded = self.slot as usize;
        if roster.get(recorded).is_some_and(&is_person) {
            return Some(recorded);
        }
        roster.iter().position(is_person)
    }
}
