use super::SimulatorDataIndexes;
use super::entry::{PersonLocation, PlayerHome, PoolPlace, RosterEntry};
use super::place::{ClubPlace, CountryPlace, LeaguePlace, LeagueSeat, RosterPlace, TeamPlace};
use super::roll::{RosterKey, RosterRoll};
use crate::country::Country;
use crate::league::League;
use crate::shared::RosterRevision;
use crate::{PlayerCollection, SimulatorData, Team};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

/// How much one reconcile had to look at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReconcileStats {
    pub containers: usize,
    pub rosters: usize,
    pub members: usize,
    pub departures: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ContainerCount {
    countries: usize,
    leagues: usize,
    clubs: usize,
    teams: usize,
}

impl ContainerCount {
    fn add(&mut self, other: ContainerCount) {
        self.countries += other.countries;
        self.leagues += other.leagues;
        self.clubs += other.clubs;
        self.teams += other.teams;
    }

    fn total(self) -> usize {
        self.countries + self.leagues + self.clubs + self.teams
    }
}

/// A roster whose membership changed since its roll, with the entry each
/// member now gets.
struct RosterSurvey<T> {
    key: RosterKey,
    revision: RosterRevision,
    members: Vec<(u32, T)>,
}

impl<T: Copy> RosterSurvey<T> {
    /// Record every member of the changed rosters, then drop each member who
    /// left one of them and arrived in none. Arrivals go first, so a person
    /// who moved between two changed rosters keeps the entry of the one he
    /// joined, whichever roster comes first.
    fn settle(
        rosters: Vec<RosterSurvey<T>>,
        entries: &mut FxHashMap<u32, T>,
        rolls: &mut FxHashMap<RosterKey, RosterRoll>,
        stats: &mut ReconcileStats,
    ) {
        let mut arrived = FxHashSet::default();
        for roster in &rosters {
            stats.rosters += 1;
            stats.members += roster.members.len();
            for &(id, entry) in &roster.members {
                entries.insert(id, entry);
                arrived.insert(id);
            }
        }
        for roster in rosters {
            if let Some(roll) = rolls.get(&roster.key) {
                for id in roll.members() {
                    if !arrived.contains(id) && entries.remove(id).is_some() {
                        stats.departures += 1;
                    }
                }
            }
            let members = roster.members.into_iter().map(|(id, _)| id).collect();
            rolls.insert(roster.key, RosterRoll::new(roster.revision, members));
        }
    }
}

/// One country compared with the index: the containers whose place is new
/// or moved, and the rosters whose membership changed since their roll.
#[derive(Default)]
struct CountrySurvey<'a> {
    countries: Vec<(u32, CountryPlace)>,
    leagues: Vec<(u32, LeaguePlace)>,
    clubs: Vec<(u32, ClubPlace)>,
    teams: Vec<(u32, TeamPlace)>,
    country_slugs: Vec<(&'a str, u32)>,
    league_slugs: Vec<(&'a str, u32)>,
    team_slugs: Vec<(&'a str, u32)>,
    players: Vec<RosterSurvey<PlayerHome>>,
    staff: Vec<RosterSurvey<RosterEntry>>,
    seen: ContainerCount,
}

impl<'a> CountrySurvey<'a> {
    fn take(
        index: &SimulatorDataIndexes,
        continent_id: u32,
        place: CountryPlace,
        country: &'a Country,
    ) -> Self {
        let mut survey = CountrySurvey::default();
        survey.seen.countries = 1;
        let recorded = index.countries.get(&country.id);
        let moved = recorded != Some(&place);
        if moved {
            if recorded.is_none() {
                survey.country_slugs.push((&country.slug, country.id));
            }
            survey.countries.push((country.id, place));
        }
        survey.pool(
            index,
            RosterKey::Retired(country.id),
            &country.retired_players,
            moved,
            place,
            PlayerHome::Retired,
        );
        survey.pool(
            index,
            RosterKey::NationalPool(country.id),
            &country.national_team.generated_squad,
            moved,
            place,
            PlayerHome::NationalPool,
        );

        // Cups and playoffs are leagues stored beside the standings; a
        // standings league of the same id keeps the seat.
        let mut league_ids = Vec::new();
        for (standing, league) in country.leagues.leagues.iter().enumerate() {
            let seat = LeaguePlace::new(place, LeagueSeat::standings(standing));
            survey.league(index, league, seat, &mut league_ids);
        }
        if let Some(cup) = &country.domestic_cup {
            let seat = LeaguePlace::new(place, LeagueSeat::DomesticCup);
            survey.league(index, &cup.league, seat, &mut league_ids);
        }
        for (playoff_index, playoff) in country.playoffs.iter().enumerate() {
            let seat = LeaguePlace::new(place, LeagueSeat::playoff(playoff_index));
            survey.league(index, &playoff.league, seat, &mut league_ids);
        }

        for (club_index, club) in country.clubs.iter().enumerate() {
            let club_place = ClubPlace::new(place, club_index);
            survey.seen.clubs += 1;
            if index.clubs.get(&club.id) != Some(&club_place) {
                survey.clubs.push((club.id, club_place));
            }
            for (team_index, team) in club.teams.teams.iter().enumerate() {
                let location = PersonLocation::new(continent_id, country.id, club.id, team.id);
                survey.team(
                    index,
                    team,
                    TeamPlace::new(club_place, team_index),
                    location,
                );
            }
        }
        survey
    }

    fn league(
        &mut self,
        index: &SimulatorDataIndexes,
        league: &'a League,
        place: LeaguePlace,
        seen_ids: &mut Vec<u32>,
    ) {
        if seen_ids.contains(&league.id) {
            return;
        }
        seen_ids.push(league.id);
        self.seen.leagues += 1;
        let recorded = index.leagues.get(&league.id);
        if recorded != Some(&place) {
            if recorded.is_none() {
                self.league_slugs.push((&league.slug, league.id));
            }
            self.leagues.push((league.id, place));
        }
    }

    fn team(
        &mut self,
        index: &SimulatorDataIndexes,
        team: &'a Team,
        place: TeamPlace,
        location: PersonLocation,
    ) {
        self.seen.teams += 1;
        let recorded = index.teams.get(&team.id);
        let moved = recorded != Some(&place);
        if moved {
            if recorded.is_none() {
                self.team_slugs.push((&team.slug, team.id));
            }
            self.teams.push((team.id, place));
        }
        let entry = |slot| RosterEntry::new(RosterPlace::new(place, slot), location);
        let players = RosterKey::TeamPlayers(team.id);
        if moved || !index.holds(players, team.players.revision()) {
            self.players.push(RosterSurvey {
                key: players,
                revision: team.players.revision(),
                members: Self::members(team.players.players.iter().map(|p| p.id), |slot| {
                    PlayerHome::Roster(entry(slot))
                }),
            });
        }
        let staff = RosterKey::TeamStaff(team.id);
        if moved || !index.holds(staff, team.staffs.revision()) {
            self.staff.push(RosterSurvey {
                key: staff,
                revision: team.staffs.revision(),
                members: Self::members(team.staffs.staffs.iter().map(|s| s.id), entry),
            });
        }
    }

    fn pool(
        &mut self,
        index: &SimulatorDataIndexes,
        key: RosterKey,
        roster: &PlayerCollection,
        moved: bool,
        place: CountryPlace,
        home: fn(PoolPlace) -> PlayerHome,
    ) {
        if moved || !index.holds(key, roster.revision()) {
            self.players.push(RosterSurvey {
                key,
                revision: roster.revision(),
                members: Self::members(roster.players.iter().map(|p| p.id), |slot| {
                    home(PoolPlace::new(place, slot))
                }),
            });
        }
    }

    fn members<T>(ids: impl Iterator<Item = u32>, entry: impl Fn(usize) -> T) -> Vec<(u32, T)> {
        ids.enumerate()
            .map(|(slot, id)| (id, entry(slot)))
            .collect()
    }
}

impl SimulatorDataIndexes {
    pub fn build(data: &SimulatorData) -> Self {
        let mut indexes = SimulatorDataIndexes::default();
        indexes.reconcile(data);
        indexes
    }

    /// Bring the index up to date with `data`, walking only the rosters whose
    /// membership changed since they were last indexed.
    pub fn reconcile(&mut self, data: &SimulatorData) -> ReconcileStats {
        let countries: Vec<(u32, CountryPlace, &Country)> = data
            .continents
            .iter()
            .enumerate()
            .flat_map(|(continent_index, continent)| {
                continent
                    .countries
                    .iter()
                    .enumerate()
                    .map(move |(country_index, country)| {
                        (
                            continent.id,
                            CountryPlace::new(continent_index, country_index),
                            country,
                        )
                    })
            })
            .collect();
        let index: &SimulatorDataIndexes = self;
        let surveys: Vec<CountrySurvey<'_>> = countries
            .par_iter()
            .map(|&(continent_id, place, country)| {
                CountrySurvey::take(index, continent_id, place, country)
            })
            .collect();
        self.apply(surveys, data)
    }

    fn holds(&self, key: RosterKey, revision: RosterRevision) -> bool {
        self.rolls
            .get(&key)
            .is_some_and(|roll| roll.is_current(revision))
    }

    fn apply(&mut self, surveys: Vec<CountrySurvey<'_>>, data: &SimulatorData) -> ReconcileStats {
        let mut stats = ReconcileStats::default();
        let mut seen = ContainerCount::default();
        let mut players = Vec::new();
        let mut staff = Vec::new();
        for survey in surveys {
            seen.add(survey.seen);
            self.countries.extend(survey.countries);
            self.leagues.extend(survey.leagues);
            self.clubs.extend(survey.clubs);
            self.teams.extend(survey.teams);
            for (slug, id) in survey.country_slugs {
                self.slug_indexes.add_country_slug(slug, id);
            }
            for (slug, id) in survey.league_slugs {
                self.slug_indexes.add_league_slug(slug, id);
            }
            for (slug, id) in survey.team_slugs {
                self.slug_indexes.add_team_slug(slug, id);
            }
            players.extend(survey.players);
            staff.extend(survey.staff);
        }
        stats.containers = seen.total();

        let vanished = if self.container_count() != seen {
            self.drop_vanished(data)
        } else {
            Vec::new()
        };
        // A roster that left the world settles as an empty one: its members
        // lose their entries unless they arrived somewhere else.
        for &key in &vanished {
            match key {
                RosterKey::TeamStaff(_) => staff.push(RosterSurvey {
                    key,
                    revision: RosterRevision::next(),
                    members: Vec::new(),
                }),
                _ => players.push(RosterSurvey {
                    key,
                    revision: RosterRevision::next(),
                    members: Vec::new(),
                }),
            }
        }

        RosterSurvey::settle(players, &mut self.players, &mut self.rolls, &mut stats);
        RosterSurvey::settle(staff, &mut self.staff, &mut self.rolls, &mut stats);
        for key in vanished {
            self.rolls.remove(&key);
        }
        stats
    }

    fn container_count(&self) -> ContainerCount {
        ContainerCount {
            countries: self.countries.len(),
            leagues: self.leagues.len(),
            clubs: self.clubs.len(),
            teams: self.teams.len(),
        }
    }

    /// The world lost a container the index still holds. Never seen in a
    /// simulated world, so it pays for one full walk of the containers.
    /// Returns the rosters that went with them.
    fn drop_vanished(&mut self, data: &SimulatorData) -> Vec<RosterKey> {
        let mut countries = FxHashSet::default();
        let mut leagues = FxHashSet::default();
        let mut clubs = FxHashSet::default();
        let mut teams = FxHashSet::default();
        for country in data.continents.iter().flat_map(|c| &c.countries) {
            countries.insert(country.id);
            leagues.extend(country.leagues.leagues.iter().map(|league| league.id));
            leagues.extend(country.domestic_cup.iter().map(|cup| cup.league.id));
            leagues.extend(country.playoffs.iter().map(|playoff| playoff.league.id));
            for club in &country.clubs {
                clubs.insert(club.id);
                teams.extend(club.teams.teams.iter().map(|team| team.id));
            }
        }
        self.countries.retain(|id, _| countries.contains(id));
        self.leagues.retain(|id, _| leagues.contains(id));
        self.clubs.retain(|id, _| clubs.contains(id));
        self.teams.retain(|id, _| teams.contains(id));
        self.slug_indexes.retain(&countries, &leagues, &teams);
        self.rolls
            .keys()
            .copied()
            .filter(|key| !key.stands_in(&teams, &countries))
            .collect()
    }
}
