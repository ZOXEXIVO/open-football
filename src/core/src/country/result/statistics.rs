use super::CountryResult;
use crate::TeamInfo;
use crate::league::Season;
use crate::simulator::SimulatorData;
use log::info;

impl CountryResult {
    /// Snapshot all player statistics into history when a new season starts.
    pub(super) fn snapshot_player_season_statistics(data: &mut SimulatorData, country_id: u32) {
        let date = data.date.date();

        let current_season = Season::from_date(date);
        let ended_season = Season::new(current_season.start_year.saturating_sub(1));

        info!(
            "📋 New season snapshot: saving player statistics for season {} (country {})",
            ended_season.start_year, country_id
        );

        let country = match data.country_mut(country_id) {
            Some(c) => c,
            None => return,
        };

        // Build league lookup so we can resolve team.league_id -> (name, slug)
        let league_lookup: std::collections::HashMap<u32, (String, String)> = country
            .leagues
            .leagues
            .iter()
            .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
            .collect();

        for club in &mut country.clubs {
            // Get main team info — used as fallback for sub-teams that
            // share the parent's brand (Reserve, U18..U23).
            let main_team_info: Option<(String, String, u16)> = club
                .teams
                .main()
                .map(|t| (t.name.clone(), t.slug.clone(), t.reputation.world));

            let main_team_league = club
                .teams
                .main()
                .and_then(|t| t.league_id)
                .and_then(|lid| league_lookup.get(&lid))
                .cloned()
                .unwrap_or_default();

            for team in &mut club.teams.teams {
                // Senior squads (Main, B, Second) keep their own identity
                // because they each compete in a real league; the player's
                // history must show the actual team and league played for.
                // Youth/Reserve sub-teams collapse into the main brand so
                // synthetic-sub-league stats still aggregate under the club.
                let keeps_own_identity = team.team_type.is_own_team();

                let (team_name, team_slug, team_reputation) =
                    if keeps_own_identity || main_team_info.is_none() {
                        (team.name.clone(), team.slug.clone(), team.reputation.world)
                    } else {
                        let (name, slug, rep) = main_team_info.as_ref().unwrap();
                        (name.clone(), slug.clone(), *rep)
                    };

                let (league_name, league_slug) = if keeps_own_identity {
                    team.league_id
                        .and_then(|lid| league_lookup.get(&lid))
                        .cloned()
                        .unwrap_or_else(|| main_team_league.clone())
                } else {
                    main_team_league.clone()
                };

                let team_info = TeamInfo {
                    name: team_name,
                    slug: team_slug,
                    reputation: team_reputation,
                    league_name,
                    league_slug,
                };

                for player in &mut team.players.players {
                    player.on_season_end(ended_season.clone(), &team_info, date);
                    player.evaluate_favorite_club(club.id, &team_info.slug, date);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::{
        Club, ClubColors, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerCollection, PlayerPositions, PlayerSkills, StaffCollection, TeamBuilder,
        TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::NaiveDate;

    fn make_date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn make_player(id: u32, played: u16, goals: u16) -> crate::Player {
        let mut player = PlayerBuilder::new()
            .id(id)
            .full_name(crate::shared::fullname::FullName::new(
                "Test".to_string(),
                format!("Player{}", id),
            ))
            .birth_date(make_date(2000, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions { positions: vec![] })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap();
        player.statistics.played = played;
        player.statistics.goals = goals;
        player
    }

    fn make_training_schedule() -> TrainingSchedule {
        use chrono::NaiveTime;
        TrainingSchedule::new(
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        )
    }

    fn make_team(
        id: u32,
        club_id: u32,
        name: &str,
        slug: &str,
        team_type: TeamType,
        league_id: Option<u32>,
        players: Vec<crate::Player>,
    ) -> crate::Team {
        TeamBuilder::new()
            .id(id)
            .league_id(league_id)
            .club_id(club_id)
            .name(name.to_string())
            .slug(slug.to_string())
            .team_type(team_type)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(make_training_schedule())
            .build()
            .unwrap()
    }

    fn make_club(id: u32, name: &str, teams: Vec<crate::Team>) -> Club {
        Club::new(
            id,
            name.to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(teams),
            crate::ClubFacilities::default(),
        )
    }

    fn make_league(id: u32, name: &str, slug: &str, friendly: bool) -> League {
        League::new(
            id,
            name.to_string(),
            slug.to_string(),
            1,
            500,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier: 1,
                promotion_spots: 0,
                relegation_spots: 0,
                league_group: None,
            },
            friendly,
        )
    }

    fn make_country(clubs: Vec<Club>, leagues: Vec<League>) -> crate::Country {
        crate::Country::builder()
            .id(1)
            .code("IT".to_string())
            .slug("italy".to_string())
            .name("Italy".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(leagues))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    fn make_simulator_data(date: NaiveDate, country: crate::Country) -> SimulatorData {
        let continent =
            crate::continent::Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
        SimulatorData::new(
            date.and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        )
    }

    #[test]
    fn snapshot_resets_player_stats_and_creates_history() {
        let player = make_player(1, 20, 5);
        let main_team = make_team(
            10,
            100,
            "Inter",
            "inter",
            TeamType::Main,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Inter", vec![main_team]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let player = &country.clubs[0].teams.teams[0].players.players[0];

        assert_eq!(player.statistics.played, 0);
        assert_eq!(player.statistics.goals, 0);
        let entry = player
            .statistics_history
            .items
            .iter()
            .find(|i| i.season.start_year == 2031)
            .expect("Frozen 2031 row missing");
        assert_eq!(entry.team_slug, "inter");
        assert_eq!(entry.league_slug, "serie-a");
        assert_eq!(entry.statistics.played, 20);
    }

    #[test]
    fn reserve_players_use_main_team_info() {
        let player = make_player(1, 10, 2);
        let main_team = make_team(
            10,
            100,
            "Juventus",
            "juventus",
            TeamType::Main,
            Some(1),
            vec![],
        );
        let reserve_team = make_team(
            11,
            100,
            "Juventus B",
            "juventus-b",
            TeamType::Reserve,
            Some(2),
            vec![player],
        );
        let club = make_club(100, "Juventus", vec![main_team, reserve_team]);
        let league_main = make_league(1, "Serie A", "serie-a", false);
        let league_reserve = make_league(2, "Serie B", "serie-b", true);
        let country = make_country(vec![club], vec![league_main, league_reserve]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let entry = &country.clubs[0].teams.teams[1].players.players[0]
            .statistics_history
            .items[0];
        assert_eq!(entry.team_slug, "juventus");
        assert_eq!(entry.team_name, "Juventus");
        assert_eq!(entry.league_slug, "serie-a");
    }

    #[test]
    fn snapshot_processes_all_players_in_all_teams() {
        let p1 = make_player(1, 30, 10);
        let p2 = make_player(2, 15, 3);
        let p3 = make_player(3, 5, 0);

        let main_team = make_team(
            10,
            100,
            "Roma",
            "roma",
            TeamType::Main,
            Some(1),
            vec![p1, p2],
        );
        let reserve_team = make_team(
            11,
            100,
            "Roma B",
            "roma-b",
            TeamType::Reserve,
            None,
            vec![p3],
        );
        let club = make_club(100, "Roma", vec![main_team, reserve_team]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();

        for player in &country.clubs[0].teams.teams[0].players.players {
            assert_eq!(player.statistics.played, 0);
            assert_eq!(player.statistics_history.items.len(), 1);
        }
        assert_eq!(
            country.clubs[0].teams.teams[0].players.players[0]
                .statistics_history
                .items[0]
                .statistics
                .played,
            30
        );
        assert_eq!(
            country.clubs[0].teams.teams[0].players.players[1]
                .statistics_history
                .items[0]
                .statistics
                .played,
            15
        );

        let reserve_player = &country.clubs[0].teams.teams[1].players.players[0];
        assert_eq!(reserve_player.statistics.played, 0);
        assert_eq!(
            reserve_player.statistics_history.items[0].statistics.played,
            5
        );
    }

    #[test]
    fn snapshot_with_invalid_country_id_does_nothing() {
        let club = make_club(100, "Inter", vec![]);
        let country = make_country(vec![club], vec![]);
        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 999);
    }

    #[test]
    fn snapshot_calculates_correct_ended_season() {
        let player = make_player(1, 10, 1);
        let main_team = make_team(
            10,
            100,
            "Milan",
            "milan",
            TeamType::Main,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Milan", vec![main_team]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2033, 9, 1), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let entry = &country.clubs[0].teams.teams[0].players.players[0]
            .statistics_history
            .items[0];
        assert_eq!(entry.season.start_year, 2032);
    }

    #[test]
    fn snapshot_processes_multiple_clubs() {
        let p1 = make_player(1, 20, 5);
        let p2 = make_player(2, 25, 8);

        let team1 = make_team(10, 100, "Inter", "inter", TeamType::Main, Some(1), vec![p1]);
        let team2 = make_team(20, 200, "Milan", "milan", TeamType::Main, Some(1), vec![p2]);
        let club1 = make_club(100, "Inter", vec![team1]);
        let club2 = make_club(200, "Milan", vec![team2]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club1, club2], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        assert_eq!(
            country.clubs[0].teams.teams[0].players.players[0]
                .statistics_history
                .items[0]
                .team_slug,
            "inter"
        );
        assert_eq!(
            country.clubs[1].teams.teams[0].players.players[0]
                .statistics_history
                .items[0]
                .team_slug,
            "milan"
        );
    }

    #[test]
    fn club_without_main_team_uses_own_team_info() {
        let player = make_player(1, 8, 1);
        let reserve = make_team(
            10,
            100,
            "Team B",
            "team-b",
            TeamType::Reserve,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Club", vec![reserve]);
        let league = make_league(1, "Serie B", "serie-b", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let entry = &country.clubs[0].teams.teams[0].players.players[0]
            .statistics_history
            .items[0];
        assert_eq!(entry.team_slug, "team-b");
    }

    #[test]
    fn reserve_player_gets_main_team_reputation() {
        let player = make_player(1, 12, 3);
        let main_team = make_team(10, 100, "Napoli", "napoli", TeamType::Main, Some(1), vec![]);
        let reserve = make_team(
            11,
            100,
            "Napoli B",
            "napoli-b",
            TeamType::Reserve,
            None,
            vec![player],
        );
        let club = make_club(100, "Napoli", vec![main_team, reserve]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let entry = &country.clubs[0].teams.teams[1].players.players[0]
            .statistics_history
            .items[0];
        assert_eq!(entry.team_reputation, 200);
    }
}
