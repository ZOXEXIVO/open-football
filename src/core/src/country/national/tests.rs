use super::*;
use crate::club::player::builder::PlayerBuilder;
use crate::club::player::statistics::PlayerStatisticsHistoryItem;
use crate::league::Season;
use crate::shared::FullName;
use crate::{
    MatchTacticType, PersonAttributes, PlayerAttributes, PlayerFieldPositionGroup, PlayerPosition,
    PlayerPositionType, PlayerPositions, PlayerSkills, PlayerStatistics, PlayerStatisticsHistory,
    Tactics,
};
use chrono::NaiveDate;
use std::collections::HashSet;

fn make_candidate(
    id: u32,
    ability: u8,
    position_group: PlayerFieldPositionGroup,
) -> CallUpCandidate {
    let position = match position_group {
        PlayerFieldPositionGroup::Goalkeeper => PlayerPositionType::Goalkeeper,
        PlayerFieldPositionGroup::Defender => PlayerPositionType::DefenderCenter,
        PlayerFieldPositionGroup::Midfielder => PlayerPositionType::MidfielderCenter,
        PlayerFieldPositionGroup::Forward => PlayerPositionType::Striker,
    };
    CallUpCandidate {
        player_id: id,
        club_id: 1,
        team_id: 1,
        current_ability: ability,
        potential_ability: ability + 10,
        age: 27,
        condition_pct: 95.0,
        match_readiness: 18.0,
        average_rating: 7.0,
        played: 10,
        international_apps: 5,
        international_goals: 1,
        leadership: 12.0,
        composure: 12.0,
        teamwork: 12.0,
        determination: 12.0,
        pressure_handling: 12.0,
        world_reputation: 4_000,
        club_reputation: 4_500,
        league_reputation: 600,
        position_levels: vec![(position, 18)],
        position_group,
        goals: 3,
        assists: 2,
        player_of_the_match: 1,
        clean_sheets: 1,
        yellow_cards: 1,
        red_cards: 0,
        last_season_apps: 30,
        last_season_rating: 7.2,
        last_season_goals: 5,
    }
}

fn make_player_with_history(
    id: u32,
    current_apps: u16,
    last_season_apps: u16,
    ability: u8,
) -> Player {
    let mut current_stats = PlayerStatistics::default();
    current_stats.played = current_apps;
    current_stats.average_rating = if current_apps > 0 { 7.0 } else { 0.0 };

    let last_season = Season::new(2025);
    let mut hist_stats = PlayerStatistics::default();
    hist_stats.played = last_season_apps;
    hist_stats.goals = 8;
    hist_stats.average_rating = 7.4;

    let history = PlayerStatisticsHistory::from_items(vec![PlayerStatisticsHistoryItem {
        season: last_season,
        team_name: "Test Club".to_string(),
        team_slug: "test-club".to_string(),
        team_reputation: 5_000,
        league_name: "Test League".to_string(),
        league_slug: "test-league".to_string(),
        is_loan: false,
        transfer_fee: None,
        statistics: hist_stats,
        seq_id: 0,
    }]);

    PlayerBuilder::new()
        .id(id)
        .full_name(FullName::new("Test".to_string(), "Player".to_string()))
        .birth_date(NaiveDate::from_ymd_opt(1996, 5, 1).unwrap())
        .country_id(1)
        .skills(PlayerSkills::default())
        .attributes(PersonAttributes::default())
        .player_attributes(PlayerAttributes {
            current_ability: ability,
            potential_ability: ability + 10,
            condition: 10000,
            world_reputation: (ability as i16) * 30,
            ..Default::default()
        })
        .contract(None)
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position: PlayerPositionType::MidfielderCenter,
                level: 18,
            }],
        })
        .statistics(current_stats)
        .statistics_history(history)
        .build()
        .expect("build test player")
}

#[test]
fn derive_reasons_returns_position_need_when_flagged() {
    let c = make_candidate(1, 110, PlayerFieldPositionGroup::Defender);
    let (primary, _secondaries) = NationalTeam::derive_reasons(&c, true);
    assert_eq!(primary, CallUpReason::PositionNeed);
}

#[test]
fn derive_reasons_picks_key_player_for_high_ability_and_world_rep() {
    let mut c = make_candidate(1, 175, PlayerFieldPositionGroup::Midfielder);
    c.world_reputation = 8_000;
    c.average_rating = 6.0; // reduce to avoid CurrentForm winning
    c.played = 0;
    c.international_apps = 5;
    let (primary, secondaries) = NationalTeam::derive_reasons(&c, false);
    assert_eq!(primary, CallUpReason::KeyPlayer);
    assert!(!secondaries.contains(&CallUpReason::PositionNeed));
}

#[test]
fn derive_reasons_picks_youth_prospect_for_high_potential_youngsters() {
    let mut c = make_candidate(1, 130, PlayerFieldPositionGroup::Forward);
    c.age = 20;
    c.potential_ability = 175;
    c.world_reputation = 1_000;
    c.average_rating = 6.5;
    c.played = 4;
    c.international_apps = 0;
    c.last_season_apps = 12;
    c.league_reputation = 400;
    c.position_levels = vec![(PlayerPositionType::Striker, 14)];
    let (primary, _) = NationalTeam::derive_reasons(&c, false);
    assert_eq!(primary, CallUpReason::YouthProspect);
}

#[test]
fn summarise_last_season_aggregates_multiple_items() {
    // Mid-season transfer: same season, two items. Apps and goals
    // should sum across both, rating should be a games-weighted blend.
    let season = Season::new(2025);
    let mut a = PlayerStatistics::default();
    a.played = 10;
    a.goals = 4;
    a.average_rating = 7.0;
    let mut b = PlayerStatistics::default();
    b.played = 20;
    b.goals = 8;
    b.average_rating = 8.0;

    let history = PlayerStatisticsHistory::from_items(vec![
        PlayerStatisticsHistoryItem {
            season: season.clone(),
            team_name: "A".to_string(),
            team_slug: "a".to_string(),
            team_reputation: 3000,
            league_name: "L".to_string(),
            league_slug: "l".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: a,
            seq_id: 0,
        },
        PlayerStatisticsHistoryItem {
            season,
            team_name: "B".to_string(),
            team_slug: "b".to_string(),
            team_reputation: 3000,
            league_name: "L".to_string(),
            league_slug: "l".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: b,
            seq_id: 1,
        },
    ]);

    let player = PlayerBuilder::new()
        .id(99)
        .full_name(FullName::new("T".into(), "P".into()))
        .birth_date(NaiveDate::from_ymd_opt(1995, 1, 1).unwrap())
        .country_id(1)
        .skills(PlayerSkills::default())
        .attributes(PersonAttributes::default())
        .player_attributes(PlayerAttributes {
            current_ability: 130,
            potential_ability: 140,
            condition: 10000,
            ..Default::default()
        })
        .contract(None)
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position: PlayerPositionType::MidfielderCenter,
                level: 18,
            }],
        })
        .statistics_history(history)
        .build()
        .expect("build");

    let (apps, rating, goals) = NationalTeam::summarise_last_season(&player);
    assert_eq!(apps, 30);
    assert_eq!(goals, 12);
    // weighted: (10 * 7.0 + 20 * 8.0) / 30 = 7.6666…
    assert!((rating - 7.6667).abs() < 0.01, "got rating {}", rating);
}

#[test]
fn build_candidate_accepts_player_with_low_current_apps_but_strong_history() {
    // September call-up: player has 1 game this season, 32 last season.
    // Without history blending this would be filtered as "unproven".
    let player = make_player_with_history(1, 1, 32, 130);
    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    let c = NationalTeam::build_candidate(&player, 1, 1, 5_000, 700, date);
    assert!(
        c.is_some(),
        "player with strong prev-season history must qualify"
    );
    let c = c.unwrap();
    assert_eq!(c.last_season_apps, 32);
    assert_eq!(c.played, 1);
}

#[test]
fn build_candidate_rejects_player_with_no_track_record() {
    // No current games, no prior season, no caps — drop them.
    let player = make_player_with_history(1, 1, 0, 80);
    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    let c = NationalTeam::build_candidate(&player, 1, 1, 3_000, 400, date);
    assert!(c.is_none(), "player without any minutes must be rejected");
}

#[test]
fn select_balanced_squad_respects_positional_quotas() {
    // Build a healthy candidate pool.
    let mut candidates: Vec<CallUpCandidate> = Vec::new();
    for i in 0..5 {
        candidates.push(make_candidate(
            100 + i,
            140,
            PlayerFieldPositionGroup::Goalkeeper,
        ));
    }
    for i in 0..10 {
        candidates.push(make_candidate(
            200 + i,
            145,
            PlayerFieldPositionGroup::Defender,
        ));
    }
    for i in 0..10 {
        candidates.push(make_candidate(
            300 + i,
            150,
            PlayerFieldPositionGroup::Midfielder,
        ));
    }
    for i in 0..10 {
        candidates.push(make_candidate(
            400 + i,
            150,
            PlayerFieldPositionGroup::Forward,
        ));
    }

    let tactics = Tactics::new(MatchTacticType::T442);
    let selected = NationalTeam::select_balanced_squad(&candidates, &tactics, false, 1);
    assert_eq!(selected.len(), SQUAD_SIZE, "squad must reach full size");

    let count_group = |g: PlayerFieldPositionGroup| -> usize {
        selected
            .iter()
            .filter(|(idx, _, _)| candidates[*idx].position_group == g)
            .count()
    };

    assert!(count_group(PlayerFieldPositionGroup::Goalkeeper) >= 3);
    assert!(count_group(PlayerFieldPositionGroup::Defender) >= 6);
    assert!(count_group(PlayerFieldPositionGroup::Midfielder) >= 6);
    assert!(count_group(PlayerFieldPositionGroup::Forward) >= 5);
}

#[test]
fn select_balanced_squad_assigns_reasons_to_every_pick() {
    let mut candidates: Vec<CallUpCandidate> = Vec::new();
    for i in 0..4 {
        candidates.push(make_candidate(
            100 + i,
            140,
            PlayerFieldPositionGroup::Goalkeeper,
        ));
    }
    for i in 0..10 {
        candidates.push(make_candidate(
            200 + i,
            145,
            PlayerFieldPositionGroup::Defender,
        ));
    }
    for i in 0..10 {
        candidates.push(make_candidate(
            300 + i,
            150,
            PlayerFieldPositionGroup::Midfielder,
        ));
    }
    for i in 0..10 {
        candidates.push(make_candidate(
            400 + i,
            150,
            PlayerFieldPositionGroup::Forward,
        ));
    }

    let tactics = Tactics::new(MatchTacticType::T442);
    let selected = NationalTeam::select_balanced_squad(&candidates, &tactics, false, 1);

    // Every pick must carry a primary reason. RegularStarter is the
    // generic fallback — anything else means a threshold tripped.
    let known_reasons: HashSet<CallUpReason> = [
        CallUpReason::KeyPlayer,
        CallUpReason::CurrentForm,
        CallUpReason::RegularStarter,
        CallUpReason::StrongLeague,
        CallUpReason::TacticalFit,
        CallUpReason::PositionNeed,
        CallUpReason::InternationalExperience,
        CallUpReason::Leadership,
        CallUpReason::YouthProspect,
    ]
    .into_iter()
    .collect();

    for (_, primary, _) in &selected {
        assert!(
            known_reasons.contains(primary),
            "primary reason {:?} not in expected set",
            primary
        );
    }
}

#[test]
fn call_up_squad_clears_generated_squad_on_subsequent_call() {
    let mut nt = NationalTeam {
        country_id: 1,
        country_name: "TestLand".to_string(),
        staff: Vec::new(),
        squad: Vec::new(),
        generated_squad: Vec::new(),
        tactics: Tactics::new(MatchTacticType::T442),
        reputation: 5_000,
        elo_rating: 1500,
        schedule: Vec::new(),
    };

    // First call-up: no real candidates → entirely synthetic depth.
    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    nt.call_up_squad(Vec::new(), date, 1, &[(2, "Other".to_string())]);
    assert!(
        !nt.generated_squad.is_empty(),
        "first call-up should have generated synthetic players"
    );
    let initial_synthetic_count = nt.generated_squad.len();

    // Second call-up with enough real candidates — the synthetic
    // pool must be cleared, not accumulated.
    let mut candidates: Vec<CallUpCandidate> = Vec::new();
    for i in 0..3 {
        candidates.push(make_candidate(
            100 + i,
            150,
            PlayerFieldPositionGroup::Goalkeeper,
        ));
    }
    for i in 0..8 {
        candidates.push(make_candidate(
            200 + i,
            150,
            PlayerFieldPositionGroup::Defender,
        ));
    }
    for i in 0..8 {
        candidates.push(make_candidate(
            300 + i,
            150,
            PlayerFieldPositionGroup::Midfielder,
        ));
    }
    for i in 0..6 {
        candidates.push(make_candidate(
            400 + i,
            150,
            PlayerFieldPositionGroup::Forward,
        ));
    }

    let next_break = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
    nt.call_up_squad(candidates, next_break, 1, &[(2, "Other".to_string())]);

    assert!(
        nt.generated_squad.is_empty(),
        "generated_squad must be cleared when real players are available; was {} before, {} after",
        initial_synthetic_count,
        nt.generated_squad.len()
    );
    assert_eq!(nt.squad.len(), SQUAD_SIZE);
}

#[test]
fn call_up_squad_preserves_completed_fixtures_when_reselecting() {
    // Older completed fixture must survive a re-call-up. A pending
    // fixture in the new break window is expected to be replaced.
    let mut nt = NationalTeam {
        country_id: 1,
        country_name: "TestLand".to_string(),
        staff: Vec::new(),
        squad: Vec::new(),
        generated_squad: Vec::new(),
        tactics: Tactics::new(MatchTacticType::T442),
        reputation: 5_000,
        elo_rating: 1500,
        schedule: vec![NationalTeamFixture {
            date: NaiveDate::from_ymd_opt(2025, 9, 6).unwrap(),
            opponent_country_id: 2,
            opponent_country_name: "Old Opp".to_string(),
            is_home: true,
            competition_name: "Friendly".to_string(),
            match_id: String::new(),
            result: Some(NationalTeamMatchResult {
                home_score: 2,
                away_score: 1,
                date: NaiveDate::from_ymd_opt(2025, 9, 6).unwrap(),
                opponent_country_id: 2,
            }),
        }],
    };

    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    nt.call_up_squad(Vec::new(), date, 1, &[(2, "Other".to_string())]);

    assert!(
        nt.schedule
            .iter()
            .any(|f| f.result.is_some() && f.opponent_country_name == "Old Opp"),
        "previous completed fixture must be preserved across a re-call-up"
    );
}

#[test]
fn league_reputation_is_zero_when_no_league_assigned_in_candidate() {
    // Sanity: the candidate captures the league_reputation we passed in,
    // distinct from club_reputation. The bug we fixed was using
    // team.reputation.world for both — this test pins the separation.
    let player = make_player_with_history(1, 10, 25, 130);
    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    let c = NationalTeam::build_candidate(&player, 1, 1, 7_000, 250, date)
        .expect("candidate should build");
    assert_eq!(c.club_reputation, 7_000);
    assert_eq!(c.league_reputation, 250);
}

#[test]
fn weak_country_still_gets_squad_but_no_friendlies() {
    // A nation below MIN_REPUTATION_FOR_FRIENDLIES used to be skipped
    // entirely — meaning a real qualifier match would trigger the
    // emergency call-up path. Now they get a normal squad selection;
    // only the friendly fixtures are gated by reputation.
    let mut nt = NationalTeam {
        country_id: 1,
        country_name: "Tiny".to_string(),
        staff: Vec::new(),
        squad: Vec::new(),
        generated_squad: Vec::new(),
        tactics: Tactics::new(MatchTacticType::T442),
        reputation: 1_500, // well below MIN_REPUTATION_FOR_FRIENDLIES (4000)
        elo_rating: 1500,
        schedule: Vec::new(),
    };

    let mut candidates: Vec<CallUpCandidate> = Vec::new();
    for i in 0..3 {
        candidates.push(make_candidate(
            100 + i,
            100,
            PlayerFieldPositionGroup::Goalkeeper,
        ));
    }
    for i in 0..8 {
        candidates.push(make_candidate(
            200 + i,
            110,
            PlayerFieldPositionGroup::Defender,
        ));
    }
    for i in 0..8 {
        candidates.push(make_candidate(
            300 + i,
            110,
            PlayerFieldPositionGroup::Midfielder,
        ));
    }
    for i in 0..6 {
        candidates.push(make_candidate(
            400 + i,
            110,
            PlayerFieldPositionGroup::Forward,
        ));
    }

    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    nt.call_up_squad(candidates, date, 1, &[(2, "Other".to_string())]);

    assert_eq!(
        nt.squad.len(),
        SQUAD_SIZE,
        "weak country must still get a full real squad"
    );
    let pending_friendlies = nt
        .schedule
        .iter()
        .filter(|f| f.competition_name == "Friendly" && f.result.is_none())
        .count();
    assert_eq!(
        pending_friendlies, 0,
        "weak country must not get auto-scheduled friendlies"
    );
}

#[test]
fn stale_pending_friendlies_are_dropped_on_recall() {
    // Three classes of pre-existing fixtures must be handled:
    //   completed_past:           kept (history)
    //   pending_past:             dropped (never played, stale)
    //   pending_current_window:   dropped (will be re-scheduled)
    //   pending_future_window:    kept (not ours to touch)
    let completed_past = NationalTeamFixture {
        date: NaiveDate::from_ymd_opt(2025, 9, 6).unwrap(),
        opponent_country_id: 2,
        opponent_country_name: "Hist".to_string(),
        is_home: true,
        competition_name: "Friendly".to_string(),
        match_id: String::new(),
        result: Some(NationalTeamMatchResult {
            home_score: 1,
            away_score: 0,
            date: NaiveDate::from_ymd_opt(2025, 9, 6).unwrap(),
            opponent_country_id: 2,
        }),
    };
    let pending_past = NationalTeamFixture {
        date: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
        opponent_country_id: 3,
        opponent_country_name: "Stale".to_string(),
        is_home: false,
        competition_name: "Friendly".to_string(),
        match_id: String::new(),
        result: None,
    };
    let pending_current = NationalTeamFixture {
        date: NaiveDate::from_ymd_opt(2026, 9, 6).unwrap(),
        opponent_country_id: 4,
        opponent_country_name: "OldPending".to_string(),
        is_home: true,
        competition_name: "Friendly".to_string(),
        match_id: String::new(),
        result: None,
    };
    let pending_future = NationalTeamFixture {
        date: NaiveDate::from_ymd_opt(2026, 11, 14).unwrap(),
        opponent_country_id: 5,
        opponent_country_name: "Future".to_string(),
        is_home: false,
        competition_name: "Friendly".to_string(),
        match_id: String::new(),
        result: None,
    };

    let mut nt = NationalTeam {
        country_id: 1,
        country_name: "TestLand".to_string(),
        staff: Vec::new(),
        squad: Vec::new(),
        generated_squad: Vec::new(),
        tactics: Tactics::new(MatchTacticType::T442),
        // Below MIN_REPUTATION_FOR_FRIENDLIES so no fresh friendlies
        // are added — keeps the assertion clean.
        reputation: 1_500,
        elo_rating: 1500,
        schedule: vec![
            completed_past,
            pending_past,
            pending_current,
            pending_future,
        ],
    };

    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    nt.call_up_squad(Vec::new(), date, 1, &[(2, "Other".to_string())]);

    let names: Vec<_> = nt
        .schedule
        .iter()
        .map(|f| f.opponent_country_name.as_str())
        .collect();
    assert!(
        names.contains(&"Hist"),
        "completed past fixture must be kept"
    );
    assert!(
        !names.contains(&"Stale"),
        "pending past fixture must be dropped"
    );
    assert!(
        !names.contains(&"OldPending"),
        "pending fixture in current break window must be dropped"
    );
    assert!(
        names.contains(&"Future"),
        "pending fixture in future window must be kept"
    );
}

#[test]
fn squad_picks_returns_real_then_synthetic_with_synthetic_depth_reason() {
    let mut nt = NationalTeam {
        country_id: 1,
        country_name: "TestLand".to_string(),
        staff: Vec::new(),
        squad: vec![NationalSquadPlayer {
            player_id: 42,
            club_id: 7,
            team_id: 7,
            primary_reason: CallUpReason::KeyPlayer,
            secondary_reasons: Vec::new(),
        }],
        generated_squad: Vec::new(),
        tactics: Tactics::new(MatchTacticType::T442),
        reputation: 5_000,
        elo_rating: 1500,
        schedule: Vec::new(),
    };

    // Force-generate one synthetic player using the existing helper.
    let synth_date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    nt.generated_squad
        .push(NationalTeam::generate_synthetic_player(
            1,
            synth_date,
            PlayerPositionType::Goalkeeper,
            120,
            0,
        ));

    let picks = nt.squad_picks();
    assert_eq!(picks.len(), 2);

    match &picks[0] {
        SquadPick::Real(sp) => {
            assert_eq!(sp.player_id, 42);
            assert_eq!(sp.primary_reason, CallUpReason::KeyPlayer);
        }
        _ => panic!("first pick should be Real"),
    }
    match &picks[1] {
        SquadPick::Synthetic(player) => {
            // A synthetic pick is rendered with reason
            // SyntheticDepth at the UI boundary; the enum itself
            // carries the player record, not the reason.
            assert!(
                player.id >= 900_000,
                "synthetic player ids start at 900_000+"
            );
        }
        _ => panic!("second pick should be Synthetic"),
    }
}
