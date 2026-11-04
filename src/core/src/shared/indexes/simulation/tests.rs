use super::entry::PlayerHome;
use super::place::{ClubPlace, CountryPlace, LeaguePlace, LeagueSeat, RosterPlace, TeamPlace};
use super::{CountryLookupIndex, ReconcileStats, SimulatorDataIndexes};
use crate::academy::ClubAcademy;
use crate::club::player::core::builder::PlayerBuilder;
use crate::competitions::global::GlobalCompetitions;
use crate::continent::Continent;
use crate::country::Country;
use crate::league::{
    DayMonthPeriod, DomesticCup, League, LeagueCollection, LeaguePlayoff, LeagueSettings,
    PlayoffFormat, PlayoffStage,
};
use crate::shared::Location;
use crate::shared::fullname::FullName;
use crate::{
    Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
    PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerSkills, SimulatorData, Staff, StaffCollection, StaffStub, Team, TeamCollection,
    TeamReputation, TeamType, TrainingSchedule,
};
use chrono::{NaiveDate, NaiveTime};

struct World;

impl World {
    fn player(id: u32) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".to_string(), format!("Player{id}")))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 15,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn staff(id: u32) -> Staff {
        let mut staff = StaffStub::build();
        staff.id = id;
        staff
    }

    fn team(id: u32, club_id: u32, team_type: TeamType, players: &[u32], staff: &[u32]) -> Team {
        Team::builder()
            .id(id)
            .league_id(None)
            .club_id(club_id)
            .name(format!("Team {id}"))
            .slug(format!("team-{id}"))
            .team_type(team_type)
            .players(PlayerCollection::new(
                players.iter().map(|&id| Self::player(id)).collect(),
            ))
            .staffs(StaffCollection::new(
                staff.iter().map(|&id| Self::staff(id)).collect(),
            ))
            .reputation(TeamReputation::new(1000, 1000, 1000))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    fn club(id: u32, teams: Vec<Team>) -> Club {
        Club::new(
            id,
            format!("Club {id}"),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(teams),
            ClubFacilities::default(),
        )
    }

    fn league(id: u32, country_id: u32) -> League {
        League::new(
            id,
            format!("League {id}"),
            format!("league-{id}"),
            country_id,
            1000,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier: 1,
                promotion_spots: 0,
                relegation_spots: 0,
                league_group: None,
                split_season: false,
            },
            false,
        )
    }

    /// Country `id` has league `id * 100`; with a cup, cup `id * 100 + 50`;
    /// with a playoff, playoff `id * 100 + 60`.
    fn country(id: u32, clubs: Vec<Club>, with_cup_and_playoff: bool) -> Country {
        let playoffs = if with_cup_and_playoff {
            vec![LeaguePlayoff::new(
                Self::league(id * 100 + 60, id),
                "playoff".to_string(),
                vec![id * 100],
                4,
                PlayoffFormat::SingleElimination,
                PlayoffStage::FullSeason,
            )]
        } else {
            Vec::new()
        };
        Country::builder()
            .id(id)
            .code(format!("c{id}"))
            .slug(format!("country-{id}"))
            .name(format!("Country {id}"))
            .continent_id(1)
            .leagues(LeagueCollection::new(vec![Self::league(id * 100, id)]))
            .domestic_cup(
                with_cup_and_playoff.then(|| DomesticCup::new(Self::league(id * 100 + 50, id))),
            )
            .playoffs(playoffs)
            .clubs(clubs)
            .build()
            .unwrap()
    }

    /// Continent 1 holds country 1 (club 10 with teams 100 and 101, club 11
    /// with team 110, a cup and a playoff) and country 2 (club 20, team 200).
    fn build() -> SimulatorData {
        let first = Self::country(
            1,
            vec![
                Self::club(
                    10,
                    vec![
                        Self::team(100, 10, TeamType::Main, &[1001, 1002, 1003], &[5001, 5002]),
                        Self::team(101, 10, TeamType::Reserve, &[1011], &[]),
                    ],
                ),
                Self::club(
                    11,
                    vec![Self::team(110, 11, TeamType::Main, &[1101], &[5101])],
                ),
            ],
            true,
        );
        let second = Self::country(
            2,
            vec![Self::club(
                20,
                vec![Self::team(200, 20, TeamType::Main, &[2001], &[5201])],
            )],
            false,
        );
        let continent = Continent::new(1, "Europe".to_string(), vec![first, second], Vec::new());
        SimulatorData::new(
            NaiveDate::from_ymd_opt(2026, 8, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        )
    }
}

#[test]
fn every_place_kind_resolves_its_entity() {
    let mut data = World::build();
    let country = CountryPlace::new(0, 0);
    let club = ClubPlace::new(country, 1);
    let team = TeamPlace::new(ClubPlace::new(country, 0), 1);
    let standings = LeaguePlace::new(country, LeagueSeat::standings(0));
    let cup = LeaguePlace::new(country, LeagueSeat::DomesticCup);
    let playoff = LeaguePlace::new(country, LeagueSeat::playoff(0));

    assert_eq!(country.resolve(&data.continents, 1).map(|c| c.id), Some(1));
    assert_eq!(club.resolve(&data.continents, 11).map(|c| c.id), Some(11));
    assert_eq!(team.resolve(&data.continents, 101).map(|t| t.id), Some(101));
    assert_eq!(
        standings.resolve(&data.continents, 100).map(|l| l.id),
        Some(100)
    );
    assert_eq!(cup.resolve(&data.continents, 150).map(|l| l.id), Some(150));
    assert_eq!(
        playoff.resolve(&data.continents, 160).map(|l| l.id),
        Some(160)
    );

    assert_eq!(
        country.resolve_mut(&mut data.continents, 1).map(|c| c.id),
        Some(1)
    );
    assert_eq!(
        club.resolve_mut(&mut data.continents, 11).map(|c| c.id),
        Some(11)
    );
    assert_eq!(
        team.resolve_mut(&mut data.continents, 101).map(|t| t.id),
        Some(101)
    );
    assert_eq!(
        cup.resolve_mut(&mut data.continents, 150).map(|l| l.id),
        Some(150)
    );
    assert_eq!(
        playoff.resolve_mut(&mut data.continents, 160).map(|l| l.id),
        Some(160)
    );
}

#[test]
fn a_place_holding_another_id_resolves_nothing() {
    let data = World::build();
    let country = CountryPlace::new(0, 0);
    assert!(country.resolve(&data.continents, 2).is_none());
    assert!(
        ClubPlace::new(country, 0)
            .resolve(&data.continents, 11)
            .is_none()
    );
    assert!(
        TeamPlace::new(ClubPlace::new(country, 0), 0)
            .resolve(&data.continents, 101)
            .is_none()
    );
    assert!(
        LeaguePlace::new(country, LeagueSeat::DomesticCup)
            .resolve(&data.continents, 100)
            .is_none()
    );
}

#[test]
fn a_position_too_large_for_its_field_resolves_nothing() {
    let data = World::build();
    let country = CountryPlace::new(0, 0);
    assert!(
        CountryPlace::new(0, 70_000)
            .resolve(&data.continents, 1)
            .is_none()
    );
    assert!(
        ClubPlace::new(country, 70_000)
            .resolve(&data.continents, 10)
            .is_none()
    );
    assert!(
        TeamPlace::new(ClubPlace::new(country, 0), 300)
            .resolve(&data.continents, 100)
            .is_none()
    );
    assert!(
        LeaguePlace::new(country, LeagueSeat::standings(70_000))
            .resolve(&data.continents, 100)
            .is_none()
    );
}

#[test]
fn a_roster_slot_too_large_for_its_field_falls_back_to_the_roster() {
    let roster = [7u32, 8, 9];
    let team = TeamPlace::new(ClubPlace::new(CountryPlace::new(0, 0), 0), 0);
    let place = RosterPlace::new(team, 70_000);
    assert_eq!(place.slot_in(&roster, |&id| id == 9), Some(2));
    assert_eq!(place.slot_in(&roster, |&id| id == 4), None);
    assert_eq!(
        RosterPlace::new(team, 1).slot_in(&roster, |&id| id == 8),
        Some(1)
    );
}

#[test]
fn cached_position_still_points_at_the_requested_entity() {
    let data = World::build();
    assert_eq!(data.player(1002).map(|p| p.id), Some(1002));
    assert_eq!(data.indexed_player_place(1002), Some((1, 0, 0, 1)));
    let (player, team) = data.player_with_team(1011).unwrap();
    assert_eq!((player.id, team.id), (1011, 101));
    assert_eq!(
        data.indexes.as_ref().unwrap().get_player_location(1011),
        Some((1, 1, 10, 101))
    );
}

#[test]
fn players_roster_slot_shifted_inside_the_same_team() {
    let mut data = World::build();
    data.team_mut(100).unwrap().players.take_player(&1001);

    // Recorded at slot 1; the index alone still finds him at slot 0.
    assert_eq!(data.indexed_player_place(1002), Some((1, 0, 0, 0)));
    assert_eq!(data.player_mut(1003).map(|p| p.id), Some(1003));
}

#[test]
fn cached_position_has_gone_stale_after_a_roster_change() {
    let mut data = World::build();
    let moved = data
        .team_mut(110)
        .unwrap()
        .players
        .take_player(&1101)
        .unwrap();
    data.team_mut(200).unwrap().players.add(moved);

    assert_eq!(data.indexed_player_place(1101), None);
    assert_eq!(data.player(1101).map(|p| p.id), Some(1101));
    assert_eq!(
        data.player_with_team(1101).map(|(_, team)| team.id),
        Some(200)
    );
    assert_eq!(data.find_player_position(1101), Some((0, 1, 0, 0)));
    assert_eq!(data.player_mut(1101).map(|p| p.id), Some(1101));
}

#[test]
fn league_lookup_covers_competitions_stored_outside_the_standard_league_list() {
    let mut data = World::build();
    assert_eq!(data.league(100).map(|l| l.id), Some(100));
    assert_eq!(data.league(150).map(|l| l.id), Some(150));
    assert_eq!(data.league(160).map(|l| l.id), Some(160));
    assert_eq!(data.league_mut(150).map(|l| l.id), Some(150));
    assert_eq!(data.league_mut(160).map(|l| l.id), Some(160));
    assert!(data.league(999).is_none());
}

#[test]
fn staff_resolve_through_their_place() {
    let data = World::build();
    let (staff, team) = data.staff_with_team(5002).unwrap();
    assert_eq!((staff.id, team.id), (5002, 100));
    assert_eq!(
        data.indexes.as_ref().unwrap().get_staff_location(5201),
        Some((1, 2, 20, 200))
    );
}

#[test]
fn clubs_teams_and_countries_resolve_through_their_place() {
    let mut data = World::build();
    assert_eq!(data.club(11).map(|c| c.id), Some(11));
    assert_eq!(data.club_mut(20).map(|c| c.id), Some(20));
    assert_eq!(data.team(101).map(|t| t.id), Some(101));
    assert_eq!(data.team_mut(200).map(|t| t.id), Some(200));
    assert_eq!(data.country(2).map(|c| c.id), Some(2));
    assert_eq!(data.country_mut(1).map(|c| c.id), Some(1));
    assert_eq!(data.country_by_club(20).map(|c| c.id), Some(2));
    assert_eq!(data.continent_by_club(11).map(|c| c.id), Some(1));
    assert_eq!(data.find_club_main_team(10), Some((0, 0, 0, 0)));
    assert!(data.club(99).is_none());
    assert!(data.team(999).is_none());
}

fn reconciled(data: &mut SimulatorData) -> ReconcileStats {
    let mut indexes = data.indexes.take().unwrap();
    let stats = indexes.reconcile(data);
    data.indexes = Some(indexes);
    stats
}

fn moved_player(data: &mut SimulatorData, id: u32, from: u32, to: u32) {
    let player = data
        .team_mut(from)
        .unwrap()
        .players
        .take_player(&id)
        .unwrap();
    data.team_mut(to).unwrap().players.add(player);
}

fn assert_fresh(data: &SimulatorData) {
    assert!(
        data.indexes.as_ref().unwrap() == &SimulatorDataIndexes::build(data),
        "a reconciled index must equal a fresh build of the same world"
    );
}

#[test]
fn unchanged_rosters_are_not_walked() {
    let mut data = World::build();
    let before = data.indexes.clone().unwrap();
    let stats = reconciled(&mut data);
    assert_eq!((stats.rosters, stats.members, stats.departures), (0, 0, 0));
    assert!(data.indexes.as_ref().unwrap() == &before);
}

#[test]
fn a_transferred_player_follows_his_new_roster() {
    let mut data = World::build();
    moved_player(&mut data, 1101, 110, 200);
    let stats = reconciled(&mut data);

    let indexes = data.indexes.as_ref().unwrap();
    assert_eq!(indexes.get_player_location(1101), Some((1, 2, 20, 200)));
    assert_eq!(data.indexed_player_place(1101), Some((2, 0, 0, 1)));
    assert_eq!((stats.rosters, stats.departures), (2, 0));
    assert_fresh(&data);
}

#[test]
fn a_released_player_loses_his_roster_location() {
    let mut data = World::build();
    let player = data
        .team_mut(100)
        .unwrap()
        .players
        .take_player(&1003)
        .unwrap();
    data.free_agents.push(player);
    let stats = reconciled(&mut data);

    assert_eq!(
        data.indexes.as_ref().unwrap().get_player_location(1003),
        None
    );
    assert_eq!(data.player(1003).map(|p| p.id), Some(1003));
    assert_eq!(stats.departures, 1);
    assert_fresh(&data);
}

#[test]
fn a_move_inside_one_club_is_reconciled_like_any_other() {
    let mut data = World::build();
    moved_player(&mut data, 1001, 100, 101);
    reconciled(&mut data);

    assert_eq!(
        data.indexes.as_ref().unwrap().get_player_location(1001),
        Some((1, 1, 10, 101))
    );
    assert_eq!(data.indexed_player_place(1001), Some((1, 0, 1, 1)));
    assert_eq!(data.indexed_player_place(1002), Some((1, 0, 0, 0)));
    assert_fresh(&data);
}

#[test]
fn an_arrival_survives_whichever_roster_is_applied_first() {
    let mut data = World::build();
    // Country 1 is surveyed before country 2: one move leaves the earlier
    // country, the other arrives in it.
    moved_player(&mut data, 1002, 100, 200);
    moved_player(&mut data, 2001, 200, 110);
    reconciled(&mut data);

    let indexes = data.indexes.as_ref().unwrap();
    assert_eq!(indexes.get_player_location(1002), Some((1, 2, 20, 200)));
    assert_eq!(indexes.get_player_location(2001), Some((1, 1, 11, 110)));
    assert_fresh(&data);
}

#[test]
fn a_team_added_after_load_becomes_resolvable() {
    let mut data = World::build();
    let team = World::team(102, 10, TeamType::U21, &[1021], &[5021]);
    data.club_mut(10).unwrap().teams.teams.push(team);
    reconciled(&mut data);

    assert_eq!(data.team(102).map(|t| t.id), Some(102));
    assert_eq!(
        data.indexes
            .as_ref()
            .unwrap()
            .slug_indexes
            .get_team_by_slug("team-102"),
        Some(102)
    );
    assert_eq!(data.indexed_player_place(1021), Some((1, 0, 2, 0)));
    assert_eq!(data.staff_with_team(5021).map(|(_, t)| t.id), Some(102));
    assert_fresh(&data);
}

#[test]
fn a_team_that_disappears_takes_its_entries_with_it() {
    let mut data = World::build();
    data.club_mut(10).unwrap().teams.teams.remove(1);
    let stats = reconciled(&mut data);

    let indexes = data.indexes.as_ref().unwrap();
    assert!(data.team(101).is_none());
    assert_eq!(indexes.get_player_location(1011), None);
    assert_eq!(indexes.slug_indexes.get_team_by_slug("team-101"), None);
    assert_eq!(stats.departures, 1);
    assert_fresh(&data);
}

#[test]
fn a_coach_who_changes_clubs_is_found_at_his_new_club() {
    let mut data = World::build();
    let coach = data.team_mut(110).unwrap().staffs.take_by_id(5101).unwrap();
    data.team_mut(200).unwrap().staffs.push(coach);
    reconciled(&mut data);

    let (staff, team) = data.staff_with_team(5101).unwrap();
    assert_eq!((staff.id, team.id), (5101, 200));
    assert_eq!(
        data.indexes.as_ref().unwrap().get_staff_location(5101),
        Some((1, 2, 20, 200))
    );
    assert_fresh(&data);
}

#[test]
fn a_scout_hired_after_load_is_indexed() {
    let mut data = World::build();
    data.team_mut(101).unwrap().staffs.push(World::staff(5999));
    reconciled(&mut data);

    assert_eq!(
        data.indexes.as_ref().unwrap().get_staff_location(5999),
        Some((1, 1, 10, 101))
    );
    assert_fresh(&data);
}

#[test]
fn reconciled_indexes_equal_a_fresh_build_after_every_kind_of_move() {
    let mut data = World::build();
    moved_player(&mut data, 1001, 100, 101);
    moved_player(&mut data, 2001, 200, 100);
    let released = data
        .team_mut(110)
        .unwrap()
        .players
        .take_player(&1101)
        .unwrap();
    data.free_agents.push(released);
    data.team_mut(200).unwrap().players.add(World::player(2002));
    let coach = data.team_mut(100).unwrap().staffs.take_by_id(5001).unwrap();
    data.team_mut(110).unwrap().staffs.push(coach);
    reconciled(&mut data);
    assert_fresh(&data);

    let stats = reconciled(&mut data);
    assert_eq!((stats.rosters, stats.members), (0, 0));
}

#[test]
fn a_reconciled_country_lookup_equals_a_fresh_build() {
    let mut data = World::build();
    let mut lookup = CountryLookupIndex::build(data.country(1).unwrap());
    moved_player(&mut data, 1001, 100, 101);
    moved_player(&mut data, 2001, 200, 110);
    data.team_mut(110).unwrap().players.add(World::player(1102));
    let released = data
        .team_mut(100)
        .unwrap()
        .players
        .take_player(&1003)
        .unwrap();
    data.free_agents.push(released);

    let country = data.country(1).unwrap();
    assert_eq!(lookup.reconcile(country), 6);
    assert!(lookup == CountryLookupIndex::build(country));
    assert_eq!(lookup.player(1002), Some((0, 0, 0)));
    assert_eq!(lookup.player(1001), Some((0, 1, 1)));
    assert_eq!(lookup.player(2001), Some((1, 0, 1)));
    assert_eq!(lookup.player(1102), Some((1, 0, 2)));
    assert_eq!(lookup.player(1003), None);
    assert_eq!(lookup.team(101), Some((0, 1)));
    assert_eq!(lookup.club(11), Some(1));
}

#[test]
fn an_unchanged_country_lookup_visits_no_player() {
    let data = World::build();
    let country = data.country(1).unwrap();
    let mut lookup = CountryLookupIndex::build(country);
    assert_eq!(lookup.reconcile(country), 0);
    assert!(lookup == CountryLookupIndex::build(country));
}

#[test]
fn a_retired_or_national_generated_player_is_looked_up() {
    let mut data = World::build();
    let retiring = data
        .team_mut(100)
        .unwrap()
        .players
        .take_player(&1003)
        .unwrap();
    data.country_mut(1).unwrap().retired_players.add(retiring);
    data.country_mut(2)
        .unwrap()
        .national_team
        .generated_squad
        .add(World::player(7002));
    reconciled(&mut data);

    let indexes = data.indexes.as_ref().unwrap();
    assert!(matches!(
        indexes.players.get(&1003),
        Some(PlayerHome::Retired(_))
    ));
    assert!(matches!(
        indexes.players.get(&7002),
        Some(PlayerHome::NationalPool(_))
    ));
    assert_eq!(indexes.get_player_location(1003), None);
    assert_eq!(data.player(1003).map(|p| p.id), Some(1003));
    assert_eq!(data.retired_player(1003).map(|p| p.id), Some(1003));
    assert_eq!(data.player(7002).map(|p| p.id), Some(7002));
    assert!(data.player_with_team(1003).is_none());
    // `player_mut` never reached retired or generated players; it still does not.
    assert!(data.player_mut(1003).is_none());
    assert_fresh(&data);
}
