use super::CountryResult;
use crate::TeamInfo;
use crate::league::Season;
use crate::simulator::SimulatorData;
use chrono::NaiveDate;
use log::info;
use rayon::prelude::*;
use std::collections::HashMap;

impl CountryResult {
    /// Snapshot every player's statistics into career history for one or
    /// more just-ended seasons.
    ///
    /// Catches up from the per-country watermark
    /// (`Country::last_snapshotted_season_year`): if today's
    /// `ended_season` is N years past the watermark, the snapshot fires
    /// N times, oldest first, so a year that slipped through the
    /// `new_season_started` league gate (regen failure, all leagues
    /// briefly inactive, fixture-skip year, etc.) still yields a row
    /// per season the player existed at the club. After each season is
    /// processed the watermark advances so future ticks don't redo
    /// already-frozen years.
    pub(super) fn snapshot_player_season_statistics(data: &mut SimulatorData, country_id: u32) {
        let date = data.date.date();
        let current_season = Season::from_date(date);
        let target_ended_year = current_season.start_year.saturating_sub(1);

        // Decide the inclusive range of season-years to catch up on.
        // The very first call (no watermark) snapshots only
        // `target_ended_year`, matching the long-standing behavior the
        // existing tests rely on. Subsequent calls advance from
        // `watermark + 1` so any year whose `new_season_started` gate
        // dropped is recovered in chronological order when the next
        // gate event eventually fires.
        let watermark = data
            .country(country_id)
            .and_then(|c| c.last_snapshotted_season_year);
        let first_year = match watermark {
            Some(w) => w.saturating_add(1),
            None => target_ended_year,
        };

        if first_year > target_ended_year {
            return;
        }

        for year in first_year..=target_ended_year {
            Self::snapshot_one_season(data, country_id, Season::new(year), date);
            if let Some(country) = data.country_mut(country_id) {
                country.last_snapshotted_season_year = Some(year);
            }
        }
    }

    /// Process every team in the country for a single ended season.
    /// Used by the catch-up loop above so one tick can advance the
    /// watermark across multiple years when the league gate failed for
    /// a previous year.
    fn snapshot_one_season(
        data: &mut SimulatorData,
        country_id: u32,
        ended_season: Season,
        date: NaiveDate,
    ) {
        info!(
            "📋 Season snapshot: saving player statistics for season {} (country {})",
            ended_season.start_year, country_id
        );

        let country = match data.country_mut(country_id) {
            Some(c) => c,
            None => return,
        };

        // Build league lookup so we can resolve team.league_id -> (name, slug)
        let league_lookup: HashMap<u32, (String, String)> = country
            .leagues
            .leagues
            .iter()
            .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
            .collect();

        country.clubs.par_iter_mut().for_each(|club| {
            // Resolve the main-team identity once per club so non-senior
            // squads (Reserve, U18..U23) can alias under it. The player
            // always carries a Main-team row even when they only ever
            // played for a youth squad — the user's rule is that
            // non-owning teams never appear under their own slug, but
            // the player still belongs to the parent club's main team.
            let main_team_info: Option<TeamInfo> = club.teams.main().map(|t| {
                let (league_name, league_slug) = t
                    .league_id
                    .and_then(|lid| league_lookup.get(&lid))
                    .cloned()
                    .unwrap_or_default();
                TeamInfo {
                    name: t.name.clone(),
                    slug: t.slug.clone(),
                    reputation: t.reputation.world,
                    league_name,
                    league_slug,
                }
            });

            for team in &mut club.teams.teams {
                let keeps_own_identity = team.team_type.is_own_team();

                if !keeps_own_identity {
                    // Non-senior squad: alias to the parent club's main
                    // team and call the youth-specific season-end path.
                    // It discards the accumulated youth-team match stats
                    // (U21 games don't count toward career statistics)
                    // but still drains any departed senior spells from
                    // `current` and ensures a Main-team row exists for
                    // the season.
                    let alias = match main_team_info.as_ref() {
                        Some(info) => info.clone(),
                        None => continue,
                    };
                    for player in &mut team.players.players {
                        player.on_non_senior_season_end(ended_season.clone(), &alias, date);
                        player.evaluate_favorite_club(club.id, &alias.slug, date);
                    }
                    continue;
                }

                let (league_name, league_slug) = team
                    .league_id
                    .and_then(|lid| league_lookup.get(&lid))
                    .cloned()
                    .unwrap_or_default();

                let team_info = TeamInfo {
                    name: team.name.clone(),
                    slug: team.slug.clone(),
                    reputation: team.reputation.world,
                    league_name,
                    league_slug,
                };

                for player in &mut team.players.players {
                    player.on_season_end(ended_season.clone(), &team_info, date);
                    player.evaluate_favorite_club(club.id, &team_info.slug, date);
                }
            }
        });
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
        PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills,
        StaffCollection, TeamBuilder, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
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
            // The post-snapshot processing now reads `position()`, so
            // the test fixture must declare at least one role. A
            // central midfielder is the most neutral choice for the
            // squad-aggregation tests in this module.
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
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
    fn reserve_players_snapshot_under_main_team_alias() {
        // Reserve / U18..U23 players never appear under their own slug
        // in career history — the user's rule is that non-owning teams
        // are not tracked. Instead the snapshot writes a row under the
        // parent club's main team so the player always has a "career
        // home" entry for the season. Reserve-league appearances live
        // in `friendly_statistics` (youth/reserve leagues are friendly)
        // and are discarded — the Main row shows 0 senior games.
        let mut player = make_player(1, 0, 0);
        player.friendly_statistics.played = 10;
        player.friendly_statistics.goals = 2;
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
        let reserve_player = &country.clubs[0].teams.teams[1].players.players[0];
        assert_eq!(reserve_player.statistics.played, 0);
        let entry = reserve_player
            .statistics_history
            .items
            .iter()
            .find(|i| i.season.start_year == 2031)
            .expect("reserve player should have a Main alias row");
        assert_eq!(entry.team_slug, "juventus");
        // Reserve games are discarded — Main row carries 0 senior apps.
        assert_eq!(entry.statistics.played, 0);
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
        // Reserve squads alias to Main: the player gets a Roma row
        // under the parent club's main slug. The 5 games on
        // `player.statistics` represent senior callup appearances
        // (reserve-league games would land in friendly_statistics) so
        // they survive into the Main row.
        let reserve_entry = &reserve_player.statistics_history.items[0];
        assert_eq!(reserve_entry.team_slug, "roma");
        assert_eq!(reserve_entry.statistics.played, 5);
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
    fn b_team_keeps_own_history_identity() {
        // B is one of the senior types ("Main, B, Second") that *do*
        // contribute to player history under their own slug.
        let player = make_player(1, 8, 1);
        let b_team = make_team(
            10,
            100,
            "Club B",
            "club-b",
            TeamType::B,
            Some(1),
            vec![player],
        );
        let club = make_club(100, "Club", vec![b_team]);
        let league = make_league(1, "Serie B", "serie-b", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let entry = &country.clubs[0].teams.teams[0].players.players[0]
            .statistics_history
            .items[0];
        assert_eq!(entry.team_slug, "club-b");
        assert_eq!(entry.statistics.played, 8);
    }

    #[test]
    fn youth_squad_player_snapshots_under_main_team_alias() {
        // U21 (and other youth squads) never appear under their own
        // slug. A player who spent the season entirely at U21 still
        // gets a Main-team row in career history (the parent club's
        // main team) — the user's rule is that non-owning team players
        // always show a Main row each season, even with 0 games.
        // U21 league games go to `friendly_statistics` (youth leagues
        // are friendly leagues) so we set them there to model the U21
        // appearances that must be discarded.
        let mut player = make_player(1, 0, 0);
        player.friendly_statistics.played = 12;
        player.friendly_statistics.goals = 3;
        let main_team = make_team(10, 100, "Napoli", "napoli", TeamType::Main, Some(1), vec![]);
        let u21 = make_team(
            11,
            100,
            "Napoli U21",
            "napoli-u21",
            TeamType::U21,
            None,
            vec![player],
        );
        let club = make_club(100, "Napoli", vec![main_team, u21]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let u21_player = &country.clubs[0].teams.teams[1].players.players[0];
        assert_eq!(u21_player.statistics.played, 0);
        // Friendly bucket reset too so U21 games don't bleed forward.
        assert_eq!(u21_player.friendly_statistics.played, 0);
        let entry = u21_player
            .statistics_history
            .items
            .iter()
            .find(|i| i.season.start_year == 2031)
            .expect("U21-only player must still have a Main alias row");
        assert_eq!(entry.team_slug, "napoli");
        // U21 (friendly-bucket) games are discarded — Main row 0 apps.
        assert_eq!(entry.statistics.played, 0);
    }

    #[test]
    fn youth_squad_player_main_team_callups_count_toward_main_row() {
        // Bug repro: a player rostered on U21 who got called up to
        // Main for some matches saw their senior-callup games erased
        // at season end. Match stats from senior matches land in
        // `player.statistics` (not `friendly_statistics`), so the
        // non-senior season-end path must preserve them and feed them
        // into the Main-team row.
        let mut player = make_player(1, 0, 0);
        // Senior callups: 5 apps for the Main team during the season.
        player.statistics.played = 5;
        player.statistics.goals = 1;
        // Plus the regular U21 league output, which must be discarded.
        player.friendly_statistics.played = 18;
        player.friendly_statistics.goals = 4;

        let main_team = make_team(10, 100, "Napoli", "napoli", TeamType::Main, Some(1), vec![]);
        let u21 = make_team(
            11,
            100,
            "Napoli U21",
            "napoli-u21",
            TeamType::U21,
            None,
            vec![player],
        );
        let club = make_club(100, "Napoli", vec![main_team, u21]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let u21_player = &country.clubs[0].teams.teams[1].players.players[0];
        assert_eq!(
            u21_player.statistics.played, 0,
            "stats reset for new season"
        );
        let entry = u21_player
            .statistics_history
            .items
            .iter()
            .find(|i| i.season.start_year == 2031)
            .expect("U21 player with senior callups must have a Main row");
        assert_eq!(entry.team_slug, "napoli");
        assert_eq!(
            entry.statistics.played, 5,
            "senior callup apps must survive the youth-squad season-end"
        );
        assert_eq!(entry.statistics.goals, 1, "callup goals must survive too");
    }

    #[test]
    fn youth_player_promoted_mid_season_keeps_main_row_only() {
        // The bug the user reported: a player who started the season at
        // Main, was demoted to U21 mid-season, and ended the season on
        // U21 should NOT see a duplicate or aliased Main row written by
        // the snapshot. Their pre-demotion Main spell is already frozen
        // into history by the intra-club move; the snapshot must not
        // double-write under the U21 → Main alias.
        use crate::PlayerStatistics;
        use crate::club::player::statistics::CurrentSeasonEntry;

        let mut player = make_player(1, 0, 0);
        // Simulate the state after a mid-season Main → U21 demotion: a
        // departed Main entry sits in `current` carrying the player's
        // pre-demotion stats.
        let mut main_stats = PlayerStatistics::default();
        main_stats.played = 12;
        main_stats.goals = 3;
        player.statistics_history.current.push(CurrentSeasonEntry {
            team_name: "Napoli".to_string(),
            team_slug: "napoli".to_string(),
            team_reputation: 100,
            league_name: "Serie A".to_string(),
            league_slug: "serie-a".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: main_stats,
            joined_date: make_date(2031, 8, 1),
            departed_date: Some(make_date(2032, 1, 15)),
            seq_id: 0,
        });
        // Player is now on U21 and accumulated 5 youth-team apps that
        // must NOT bleed into history. Youth-league apps live in
        // `friendly_statistics`, not `statistics`.
        player.friendly_statistics.played = 5;
        player.friendly_statistics.goals = 1;

        let main_team = make_team(10, 100, "Napoli", "napoli", TeamType::Main, Some(1), vec![]);
        let u21 = make_team(
            11,
            100,
            "Napoli U21",
            "napoli-u21",
            TeamType::U21,
            None,
            vec![player],
        );
        let club = make_club(100, "Napoli", vec![main_team, u21]);
        let league = make_league(1, "Serie A", "serie-a", false);
        let country = make_country(vec![club], vec![league]);

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country_mut(1).unwrap();
        let player = &country.clubs[0].teams.teams[1].players.players[0];
        assert_eq!(player.statistics.played, 0, "match stats must be reset");

        // Exactly one career row: the Main spell with 12 apps. No U21
        // alias, no phantom row from the youth-team snapshot.
        assert_eq!(
            player.statistics_history.items.len(),
            1,
            "expected exactly one career row from the Main spell"
        );
        let entry = &player.statistics_history.items[0];
        assert_eq!(entry.team_slug, "napoli");
        assert_eq!(entry.statistics.played, 12);
        assert_eq!(entry.statistics.goals, 3);
    }

    // Snapshot watermark catches up missed seasons. Simulates the
    // user-reported "missing 2026/27" pattern: the first snapshot
    // fires for 2031/32 (date Aug 15, 2032), then the next snapshot
    // call happens after a one-year gap (date Aug 15, 2034 — the
    // 2032/33 snapshot was dropped because the league gate failed for
    // that year). The catch-up loop must process both 2032/33 and
    // 2033/34 in order so the player has one row per season.
    #[test]
    fn snapshot_catches_up_missed_year_via_watermark() {
        let player = make_player(1, 5, 1);
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

        let mut data = make_simulator_data(make_date(2032, 8, 15), country);
        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        // After the first call: watermark is 2031, one frozen row exists.
        let country = data.country(1).unwrap();
        assert_eq!(country.last_snapshotted_season_year, Some(2031));
        let items_after_first = country.clubs[0].teams.teams[0].players.players[0]
            .statistics_history
            .items
            .len();
        assert_eq!(items_after_first, 1);

        // Advance the date by two years and play another partial season.
        // The 2032/33 snapshot was never fired (gate dropped). The next
        // call's catch-up loop should produce TWO rows: 2032 and 2033.
        let player = &mut data.continents[0].countries[0].clubs[0].teams.teams[0]
            .players
            .players[0];
        player.statistics.played = 8;
        player.statistics.goals = 2;
        data.date = make_date(2034, 8, 15).and_hms_opt(12, 0, 0).unwrap();

        CountryResult::snapshot_player_season_statistics(&mut data, 1);

        let country = data.country(1).unwrap();
        assert_eq!(country.last_snapshotted_season_year, Some(2033));

        let years: Vec<u16> = country.clubs[0].teams.teams[0].players.players[0]
            .statistics_history
            .items
            .iter()
            .map(|i| i.season.start_year)
            .collect();
        assert!(
            years.contains(&2031),
            "first snapshot row missing: {:?}",
            years
        );
        assert!(
            years.contains(&2032),
            "missed-year catch-up row missing: {:?}",
            years
        );
        assert!(
            years.contains(&2033),
            "current ended-season row missing: {:?}",
            years
        );
    }
}
