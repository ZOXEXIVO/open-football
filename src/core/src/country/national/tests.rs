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

fn comp_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 4).unwrap()
}
fn tournament_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()
}

fn comp_ctx() -> CallUpContext {
    CallUpContext::new(comp_date(), 1, CallUpWindowType::CompetitiveWindow)
}
fn friendly_ctx() -> CallUpContext {
    CallUpContext::new(comp_date(), 1, CallUpWindowType::FriendlyWindow)
}
fn tournament_ctx() -> CallUpContext {
    CallUpContext::new(tournament_date(), 1, CallUpWindowType::TournamentFinals)
}

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

/// Build a healthy candidate pool covering every position group, with
/// real wide-defensive cover available for the role-coverage tests.
fn realistic_pool() -> Vec<CallUpCandidate> {
    let mut v: Vec<CallUpCandidate> = Vec::new();
    for i in 0..5 {
        v.push(make_candidate(
            100 + i,
            140,
            PlayerFieldPositionGroup::Goalkeeper,
        ));
    }
    for i in 0..12 {
        let mut c = make_candidate(200 + i, 145, PlayerFieldPositionGroup::Defender);
        if i == 0 {
            c.position_levels = vec![(PlayerPositionType::DefenderLeft, 17)];
        }
        if i == 1 {
            c.position_levels = vec![(PlayerPositionType::DefenderRight, 17)];
        }
        v.push(c);
    }
    for i in 0..12 {
        v.push(make_candidate(
            300 + i,
            150,
            PlayerFieldPositionGroup::Midfielder,
        ));
    }
    for i in 0..10 {
        v.push(make_candidate(
            400 + i,
            150,
            PlayerFieldPositionGroup::Forward,
        ));
    }
    v
}

#[test]
fn ordinary_break_selects_23_players() {
    let candidates = realistic_pool();
    let tactics = Tactics::new(MatchTacticType::T442);
    let ctx = comp_ctx();
    let selected =
        NationalTeam::select_balanced_squad(&candidates, &tactics, &ctx, &HashSet::new());
    assert_eq!(selected.len(), 23);
}

#[test]
fn tournament_finals_select_26_players() {
    let candidates = realistic_pool();
    let tactics = Tactics::new(MatchTacticType::T442);
    let ctx = tournament_ctx();
    let selected =
        NationalTeam::select_balanced_squad(&candidates, &tactics, &ctx, &HashSet::new());
    assert_eq!(selected.len(), 26);
}

#[test]
fn single_nationality_eligibility_unchanged() {
    let player = make_player_with_history(1, 10, 25, 130);
    assert!(NationalTeam::is_eligible_for_country(&player, 1));
    assert!(!NationalTeam::is_eligible_for_country(&player, 7));
}

#[test]
fn incumbent_beats_marginally_better_uncapped_in_competitive_window() {
    let tactics = Tactics::new(MatchTacticType::T442);
    // country_id=4 → TacticalSpecialist coach: bias is driven by best
    // tactic position level, which is identical for both candidates,
    // so the coach term cancels and continuity / experience decide.
    let ctx = CallUpContext::new(comp_date(), 4, CallUpWindowType::CompetitiveWindow);

    let mut incumbent = make_candidate(999, 138, PlayerFieldPositionGroup::Forward);
    incumbent.international_apps = 35;
    incumbent.average_rating = 6.9;

    let mut newbie = make_candidate(1000, 142, PlayerFieldPositionGroup::Forward);
    newbie.international_apps = 0;
    newbie.age = 25;
    newbie.potential_ability = 152; // suppress any youth coach quirk
    newbie.average_rating = 7.0;

    let mut incumbents = HashSet::new();
    incumbents.insert(999_u32);

    let s_incumbent = NationalTeam::score_candidate(&incumbent, &tactics, &ctx, &incumbents);
    let s_newbie = NationalTeam::score_candidate(&newbie, &tactics, &ctx, &incumbents);

    assert!(
        s_incumbent > s_newbie,
        "incumbent ({:.2}) must edge out marginal uncapped newbie ({:.2})",
        s_incumbent,
        s_newbie
    );
}

#[test]
fn clearly_superior_uncapped_beats_incumbent() {
    let tactics = Tactics::new(MatchTacticType::T442);
    let ctx = comp_ctx();

    let mut weak_incumbent = make_candidate(999, 100, PlayerFieldPositionGroup::Forward);
    weak_incumbent.international_apps = 15;
    weak_incumbent.average_rating = 5.8;
    weak_incumbent.world_reputation = 1_500;

    let mut elite = make_candidate(1000, 180, PlayerFieldPositionGroup::Forward);
    elite.international_apps = 0;
    elite.age = 24;
    elite.average_rating = 8.4;
    elite.world_reputation = 7_500;
    elite.goals = 18;

    let mut incumbents = HashSet::new();
    incumbents.insert(999_u32);

    let s_weak = NationalTeam::score_candidate(&weak_incumbent, &tactics, &ctx, &incumbents);
    let s_elite = NationalTeam::score_candidate(&elite, &tactics, &ctx, &incumbents);

    assert!(
        s_elite > s_weak,
        "elite uncapped ({:.2}) must beat weak incumbent ({:.2})",
        s_elite,
        s_weak
    );
}

#[test]
fn friendly_mode_favors_u24_high_potential_uncapped_player() {
    let mut prospect = make_candidate(1, 130, PlayerFieldPositionGroup::Midfielder);
    prospect.age = 20;
    prospect.potential_ability = 175;
    prospect.international_apps = 0;
    prospect.world_reputation = 1_500;

    let mut veteran = make_candidate(2, 135, PlayerFieldPositionGroup::Midfielder);
    veteran.age = 31;
    veteran.potential_ability = 135;
    veteran.international_apps = 45;
    veteran.world_reputation = 4_000;

    let ctx = friendly_ctx();
    let tactics = Tactics::new(MatchTacticType::T442);

    let s_prospect = NationalTeam::score_candidate(&prospect, &tactics, &ctx, &HashSet::new());
    let s_veteran = NationalTeam::score_candidate(&veteran, &tactics, &ctx, &HashSet::new());
    assert!(
        s_prospect > s_veteran,
        "friendly window should favour the U24 prospect ({:.2}) over the veteran ({:.2})",
        s_prospect,
        s_veteran
    );
}

#[test]
fn tournament_mode_favors_experienced_high_cap_player() {
    let mut prospect = make_candidate(1, 130, PlayerFieldPositionGroup::Midfielder);
    prospect.age = 19;
    prospect.potential_ability = 175;
    prospect.international_apps = 0;
    prospect.world_reputation = 1_500;

    let mut veteran = make_candidate(2, 135, PlayerFieldPositionGroup::Midfielder);
    veteran.age = 30;
    veteran.potential_ability = 135;
    veteran.international_apps = 60;
    veteran.international_goals = 10;
    veteran.world_reputation = 5_500;

    let ctx = tournament_ctx();
    let tactics = Tactics::new(MatchTacticType::T442);
    let s_prospect = NationalTeam::score_candidate(&prospect, &tactics, &ctx, &HashSet::new());
    let s_veteran = NationalTeam::score_candidate(&veteran, &tactics, &ctx, &HashSet::new());
    assert!(
        s_veteran > s_prospect,
        "tournament should favour the experienced veteran ({:.2}) over the prospect ({:.2})",
        s_veteran,
        s_prospect
    );
}

#[test]
fn role_coverage_selects_left_right_defensive_cover_when_available() {
    let mut candidates: Vec<CallUpCandidate> = Vec::new();
    for i in 0..3 {
        candidates.push(make_candidate(
            100 + i,
            150,
            PlayerFieldPositionGroup::Goalkeeper,
        ));
    }
    // 12 elite centre-backs
    for i in 0..12 {
        let mut c = make_candidate(200 + i, 160, PlayerFieldPositionGroup::Defender);
        c.position_levels = vec![(PlayerPositionType::DefenderCenter, 18)];
        candidates.push(c);
    }
    // Real wide cover available, slightly weaker.
    let mut lb = make_candidate(280, 138, PlayerFieldPositionGroup::Defender);
    lb.position_levels = vec![(PlayerPositionType::DefenderLeft, 17)];
    candidates.push(lb);
    let mut rb = make_candidate(281, 138, PlayerFieldPositionGroup::Defender);
    rb.position_levels = vec![(PlayerPositionType::DefenderRight, 17)];
    candidates.push(rb);
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

    let tactics = Tactics::new(MatchTacticType::T442);
    let ctx = comp_ctx();
    let selected =
        NationalTeam::select_balanced_squad(&candidates, &tactics, &ctx, &HashSet::new());

    let picked_ids: Vec<u32> = selected
        .iter()
        .map(|(i, _, _)| candidates[*i].player_id)
        .collect();

    assert!(
        picked_ids.contains(&280),
        "role coverage should include left-back; picks: {:?}",
        picked_ids
    );
    assert!(
        picked_ids.contains(&281),
        "role coverage should include right-back; picks: {:?}",
        picked_ids
    );
}

#[test]
fn role_coverage_does_not_replace_a_much_stronger_selected_player() {
    let mut candidates: Vec<CallUpCandidate> = Vec::new();
    for i in 0..3 {
        candidates.push(make_candidate(
            100 + i,
            180,
            PlayerFieldPositionGroup::Goalkeeper,
        ));
    }
    for i in 0..10 {
        let mut c = make_candidate(200 + i, 180, PlayerFieldPositionGroup::Defender);
        c.position_levels = vec![(PlayerPositionType::DefenderCenter, 20)];
        c.average_rating = 8.5;
        c.world_reputation = 8_000;
        candidates.push(c);
    }
    for i in 0..10 {
        let mut c = make_candidate(300 + i, 180, PlayerFieldPositionGroup::Midfielder);
        c.average_rating = 8.5;
        c.world_reputation = 8_000;
        candidates.push(c);
    }
    for i in 0..8 {
        let mut c = make_candidate(400 + i, 180, PlayerFieldPositionGroup::Forward);
        c.average_rating = 8.5;
        c.world_reputation = 8_000;
        candidates.push(c);
    }

    // Very weak left-back: gap too wide to justify a coverage swap.
    let mut weak_lb = make_candidate(500, 70, PlayerFieldPositionGroup::Defender);
    weak_lb.position_levels = vec![(PlayerPositionType::DefenderLeft, 14)];
    weak_lb.average_rating = 5.5;
    weak_lb.world_reputation = 500;
    weak_lb.international_apps = 0;
    weak_lb.age = 29;
    candidates.push(weak_lb);

    let tactics = Tactics::new(MatchTacticType::T442);
    let ctx = comp_ctx();
    let selected =
        NationalTeam::select_balanced_squad(&candidates, &tactics, &ctx, &HashSet::new());
    let picked_ids: Vec<u32> = selected
        .iter()
        .map(|(i, _, _)| candidates[*i].player_id)
        .collect();
    assert!(
        !picked_ids.contains(&500),
        "role coverage must not swap out a much stronger pick; picks: {:?}",
        picked_ids
    );
}

#[test]
fn every_selected_player_has_a_primary_call_up_reason() {
    let candidates = realistic_pool();
    let tactics = Tactics::new(MatchTacticType::T442);
    let ctx = comp_ctx();
    let selected =
        NationalTeam::select_balanced_squad(&candidates, &tactics, &ctx, &HashSet::new());

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
        CallUpReason::Incumbent,
        CallUpReason::RoleCoverage,
        CallUpReason::FriendlyExperiment,
        CallUpReason::TournamentExperience,
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
fn synthetic_fallback_still_works_for_weak_countries() {
    let mut nt = NationalTeam {
        country_id: 1,
        country_name: "Mini".to_string(),
        level: NationalTeamLevel::Senior,
        staff: Vec::new(),
        squad: Vec::new(),
        generated_squad: Vec::new(),
        tactics: Tactics::new(MatchTacticType::T442),
        reputation: 600,
        elo_rating: 1500,
        schedule: Vec::new(),
    };

    nt.call_up_squad(Vec::new(), comp_date(), 1, &[(2, "Other".to_string())]);
    assert!(!nt.generated_squad.is_empty());
    assert!(nt.squad.len() + nt.generated_squad.len() >= 23);
}

#[test]
fn no_pending_friendly_fixtures_are_created_by_call_up_squad() {
    let mut nt = NationalTeam {
        country_id: 1,
        country_name: "Test".to_string(),
        level: NationalTeamLevel::Senior,
        staff: Vec::new(),
        squad: Vec::new(),
        generated_squad: Vec::new(),
        tactics: Tactics::new(MatchTacticType::T442),
        reputation: 8_000,
        elo_rating: 1500,
        schedule: Vec::new(),
    };

    let candidates = realistic_pool();
    nt.call_up_squad(candidates, comp_date(), 1, &[(2, "Other".to_string())]);

    let pending_friendlies = nt.schedule.iter().filter(|f| f.result.is_none()).count();
    assert_eq!(pending_friendlies, 0);
}

#[test]
fn foreign_based_players_remain_eligible_for_their_birth_country() {
    // The collect-by-country pipeline buckets players by player.country_id,
    // not the club's country. Pin the eligibility helper so the world-wide
    // Int-status pass keeps working for foreign-based call-ups.
    let mut player = make_player_with_history(1, 12, 25, 150);
    player.country_id = 42;
    assert!(NationalTeam::is_eligible_for_country(&player, 42));
    assert!(!NationalTeam::is_eligible_for_country(&player, 1));
}

#[test]
fn derive_reasons_returns_position_need_when_flagged() {
    let c = make_candidate(1, 110, PlayerFieldPositionGroup::Defender);
    let ctx = comp_ctx();
    let (primary, _) = NationalTeam::derive_reasons(&c, true, false, &ctx, &HashSet::new());
    assert_eq!(primary, CallUpReason::PositionNeed);
}

#[test]
fn derive_reasons_returns_role_coverage_when_flagged() {
    let c = make_candidate(1, 110, PlayerFieldPositionGroup::Defender);
    let ctx = comp_ctx();
    let (primary, _) = NationalTeam::derive_reasons(&c, false, true, &ctx, &HashSet::new());
    assert_eq!(primary, CallUpReason::RoleCoverage);
}

#[test]
fn derive_reasons_picks_key_player_for_high_ability_and_world_rep() {
    let mut c = make_candidate(1, 175, PlayerFieldPositionGroup::Midfielder);
    c.world_reputation = 8_000;
    c.average_rating = 6.0;
    c.played = 0;
    c.international_apps = 5;
    let ctx = comp_ctx();
    let (primary, secondaries) =
        NationalTeam::derive_reasons(&c, false, false, &ctx, &HashSet::new());
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
    let ctx = comp_ctx();
    let (primary, _) = NationalTeam::derive_reasons(&c, false, false, &ctx, &HashSet::new());
    assert_eq!(primary, CallUpReason::YouthProspect);
}

#[test]
fn derive_reasons_picks_friendly_experiment_in_friendly_window() {
    let mut c = make_candidate(1, 120, PlayerFieldPositionGroup::Midfielder);
    c.age = 22;
    c.international_apps = 0;
    c.current_ability = 120;
    c.world_reputation = 1_500;
    c.average_rating = 6.5;
    c.played = 4;
    c.last_season_apps = 8;
    c.leadership = 8.0;
    c.league_reputation = 400;
    c.potential_ability = 140;
    c.position_levels = vec![(PlayerPositionType::MidfielderCenter, 14)];

    let ctx = friendly_ctx();
    let (primary, _) = NationalTeam::derive_reasons(&c, false, false, &ctx, &HashSet::new());
    assert_eq!(primary, CallUpReason::FriendlyExperiment);
}

#[test]
fn derive_reasons_picks_tournament_experience_for_capped_veteran() {
    let mut c = make_candidate(1, 140, PlayerFieldPositionGroup::Midfielder);
    c.age = 30;
    c.international_apps = 55;
    c.world_reputation = 4_000;
    c.average_rating = 6.5;
    c.played = 4;
    c.current_ability = 140;
    c.position_levels = vec![(PlayerPositionType::MidfielderCenter, 14)];
    c.leadership = 8.0;
    c.league_reputation = 400;

    let ctx = tournament_ctx();
    let (primary, _) = NationalTeam::derive_reasons(&c, false, false, &ctx, &HashSet::new());
    assert_eq!(primary, CallUpReason::TournamentExperience);
}

#[test]
fn summarise_last_season_aggregates_multiple_items() {
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
    assert!(
        (rating - 7.36).abs() < 0.05,
        "expected regressed rating ~7.36, got {}",
        rating
    );
}

#[test]
fn build_candidate_accepts_player_with_low_current_apps_but_strong_history() {
    let player = make_player_with_history(1, 1, 32, 130);
    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    let c = NationalTeam::build_candidate(&player, 1, 1, 5_000, 700, date);
    assert!(c.is_some());
    let c = c.unwrap();
    assert_eq!(c.last_season_apps, 32);
    assert_eq!(c.played, 1);
}

#[test]
fn build_candidate_rejects_player_with_no_track_record() {
    let player = make_player_with_history(1, 1, 0, 80);
    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    let c = NationalTeam::build_candidate(&player, 1, 1, 3_000, 400, date);
    assert!(c.is_none());
}

#[test]
fn call_up_squad_clears_generated_squad_on_subsequent_call() {
    let mut nt = NationalTeam {
        country_id: 1,
        country_name: "TestLand".to_string(),
        level: NationalTeamLevel::Senior,
        staff: Vec::new(),
        squad: Vec::new(),
        generated_squad: Vec::new(),
        tactics: Tactics::new(MatchTacticType::T442),
        reputation: 5_000,
        elo_rating: 1500,
        schedule: Vec::new(),
    };

    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    nt.call_up_squad(Vec::new(), date, 1, &[(2, "Other".to_string())]);
    assert!(!nt.generated_squad.is_empty());
    let initial_synthetic_count = nt.generated_squad.len();

    let candidates = realistic_pool();
    let next_break = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
    nt.call_up_squad(candidates, next_break, 1, &[(2, "Other".to_string())]);

    assert!(
        nt.generated_squad.is_empty(),
        "generated_squad must be cleared; was {} before, {} after",
        initial_synthetic_count,
        nt.generated_squad.len()
    );
    assert_eq!(nt.squad.len(), SQUAD_SIZE);
}

#[test]
fn call_up_squad_preserves_completed_fixtures_when_reselecting() {
    let mut nt = NationalTeam {
        country_id: 1,
        country_name: "TestLand".to_string(),
        level: NationalTeamLevel::Senior,
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
            .any(|f| f.result.is_some() && f.opponent_country_name == "Old Opp")
    );
}

#[test]
fn league_reputation_is_zero_when_no_league_assigned_in_candidate() {
    let player = make_player_with_history(1, 10, 25, 130);
    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    let c = NationalTeam::build_candidate(&player, 1, 1, 7_000, 250, date)
        .expect("candidate should build");
    assert_eq!(c.club_reputation, 7_000);
    assert_eq!(c.league_reputation, 250);
}

#[test]
fn weak_country_still_gets_squad_but_no_friendlies() {
    let mut nt = NationalTeam {
        country_id: 1,
        country_name: "Tiny".to_string(),
        level: NationalTeamLevel::Senior,
        staff: Vec::new(),
        squad: Vec::new(),
        generated_squad: Vec::new(),
        tactics: Tactics::new(MatchTacticType::T442),
        reputation: 1_500,
        elo_rating: 1500,
        schedule: Vec::new(),
    };

    let candidates = realistic_pool();
    let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
    nt.call_up_squad(candidates, date, 1, &[(2, "Other".to_string())]);

    assert_eq!(nt.squad.len(), SQUAD_SIZE);
    let pending_friendlies = nt
        .schedule
        .iter()
        .filter(|f| f.competition_name == "Friendly" && f.result.is_none())
        .count();
    assert_eq!(pending_friendlies, 0);
}

#[test]
fn stale_pending_friendlies_are_dropped_on_recall() {
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
        level: NationalTeamLevel::Senior,
        staff: Vec::new(),
        squad: Vec::new(),
        generated_squad: Vec::new(),
        tactics: Tactics::new(MatchTacticType::T442),
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
    assert!(names.contains(&"Hist"));
    assert!(!names.contains(&"Stale"));
    assert!(!names.contains(&"OldPending"));
    assert!(names.contains(&"Future"));
}

#[test]
fn squad_picks_returns_real_then_synthetic_with_synthetic_depth_reason() {
    let mut nt = NationalTeam {
        country_id: 1,
        country_name: "TestLand".to_string(),
        level: NationalTeamLevel::Senior,
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
            assert!(player.id >= 900_000);
        }
        _ => panic!("second pick should be Synthetic"),
    }
}

// ============================================================
// U21 selection
// ============================================================

fn u21_ctx() -> CallUpContext {
    CallUpContext::new_with_level(
        comp_date(),
        1,
        CallUpWindowType::CompetitiveWindow,
        NationalTeamLevel::Under21,
        23,
    )
}

/// U21 blend weights potential and the age curve heavily: a high-ceiling
/// 19-year-old should rank above an average 21-year-old even when the
/// 21-year-old has slightly higher current ability.
#[test]
fn u21_scoring_prefers_high_potential_teenager_over_average_21yo() {
    let tactics = Tactics::new(MatchTacticType::T442);
    let ctx = u21_ctx();

    let mut prospect = make_candidate(1, 120, PlayerFieldPositionGroup::Midfielder);
    prospect.age = 19;
    prospect.potential_ability = 170;
    prospect.international_apps = 0;

    let mut average = make_candidate(2, 132, PlayerFieldPositionGroup::Midfielder);
    average.age = 21;
    average.potential_ability = 140;
    average.international_apps = 0;

    let s_prospect = NationalTeam::score_candidate(&prospect, &tactics, &ctx, &HashSet::new());
    let s_average = NationalTeam::score_candidate(&average, &tactics, &ctx, &HashSet::new());
    assert!(
        s_prospect > s_average,
        "U21 blend should rank the high-ceiling 19yo above the average 21yo (prospect={s_prospect}, average={s_average})"
    );
}

/// Senior caps are heavily penalised in the U21 blend so genuine youth
/// prospects are preferred over players already established at senior level.
#[test]
fn u21_scoring_penalises_senior_capped_players() {
    let tactics = Tactics::new(MatchTacticType::T442);
    let ctx = u21_ctx();

    let mut uncapped = make_candidate(1, 130, PlayerFieldPositionGroup::Midfielder);
    uncapped.age = 20;
    uncapped.international_apps = 0;

    let mut senior_regular = make_candidate(2, 130, PlayerFieldPositionGroup::Midfielder);
    senior_regular.age = 20;
    senior_regular.international_apps = 15; // -45 penalty

    let s_uncapped = NationalTeam::score_candidate(&uncapped, &tactics, &ctx, &HashSet::new());
    let s_senior = NationalTeam::score_candidate(&senior_regular, &tactics, &ctx, &HashSet::new());
    assert!(
        s_uncapped > s_senior,
        "a true U21 prospect should outscore an equally-able senior regular"
    );
}

/// A high-ceiling youngster reads as `U21EliteProspect` under the U21
/// reason-derivation path.
#[test]
fn u21_reason_is_elite_prospect_for_high_ceiling() {
    let ctx = u21_ctx();
    let mut c = make_candidate(1, 120, PlayerFieldPositionGroup::Forward);
    c.age = 18;
    c.potential_ability = 165;
    let (primary, _) = NationalTeam::derive_reasons(&c, false, false, &ctx, &HashSet::new());
    assert_eq!(primary, CallUpReason::U21EliteProspect);
}

/// Senior scoring is unchanged: a senior context must not route through
/// the U21 blend (regression guard for the level branch in `score_candidate`).
#[test]
fn senior_scoring_unaffected_by_u21_branch() {
    let tactics = Tactics::new(MatchTacticType::T442);
    let senior_ctx = comp_ctx();
    let c = make_candidate(1, 150, PlayerFieldPositionGroup::Midfielder);
    let senior_score = NationalTeam::score_candidate(&c, &tactics, &senior_ctx, &HashSet::new());
    let u21_score = NationalTeam::score_candidate(&c, &tactics, &u21_ctx(), &HashSet::new());
    // The two blends are different functions; they should not coincide for
    // a mid-tier 27-year-old (sanity that the branch actually diverges).
    assert!((senior_score - u21_score).abs() > f32::EPSILON);
}
