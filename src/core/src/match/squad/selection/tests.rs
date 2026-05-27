use super::*;
use crate::PlayerPositions;
use crate::StaffStub;
use crate::Team;
use crate::club::PlayerFieldPositionGroup;
use crate::{
    IntegerUtils, MatchTacticType, PeopleNameGeneratorData, PlayerClubContract, PlayerCollection,
    PlayerGenerator, PlayerPosition, PlayerSquadStatus, PlayerStatusType, SelectionOmissionReason,
    StaffCollection, TeamBuilder, TeamReputation, TeamType, TrainingSchedule,
};
use chrono::NaiveDate;
use chrono::{Datelike, NaiveTime, Utc};

fn test_names() -> PeopleNameGeneratorData {
    PeopleNameGeneratorData {
        first_names: vec!["Test".to_string()],
        last_names: vec!["Player".to_string()],
        nicknames: Vec::new(),
    }
}

#[test]
fn test_squad_selection_always_produces_11() {
    let team = generate_test_team();
    let staff = generate_test_staff();

    let result = SquadSelector::select(&team, &staff);

    assert_eq!(result.main_squad.len(), 11);
    assert!(!result.substitutes.is_empty());
    assert!(result.substitutes.len() <= helpers::DEFAULT_BENCH_SIZE);
}

#[test]
fn test_squad_always_has_goalkeeper() {
    let team = generate_test_team();
    let staff = generate_test_staff();

    let result = SquadSelector::select(&team, &staff);

    let has_gk = result
        .main_squad
        .iter()
        .any(|p| p.tactical_position.current_position == PlayerPositionType::Goalkeeper);
    assert!(has_gk, "Starting 11 must always have a goalkeeper");
}

#[test]
fn test_squad_no_duplicate_players() {
    let team = generate_test_team();
    let staff = generate_test_staff();

    let result = SquadSelector::select(&team, &staff);

    let mut all_ids: Vec<u32> = result.main_squad.iter().map(|p| p.id).collect();
    all_ids.extend(result.substitutes.iter().map(|p| p.id));

    let unique_count = {
        let mut sorted = all_ids.clone();
        sorted.sort();
        sorted.dedup();
        sorted.len()
    };
    assert_eq!(all_ids.len(), unique_count, "No player should appear twice");
}

#[test]
fn test_position_group_matching() {
    let score = helpers::position_fit_score(
        &generate_defender_center(),
        PlayerPositionType::DefenderCenterLeft,
        PlayerFieldPositionGroup::Defender,
    );
    assert!(
        score > 5.0,
        "Same-group player should score well: {}",
        score
    );
}

#[test]
fn test_global_assignment_preserves_specialists() {
    let mut team = generate_test_team();
    let staff = generate_test_staff();
    let date = Utc::now().date_naive();

    let mut players = vec![
        make_test_player(1, &[(PlayerPositionType::Goalkeeper, 20)], 120, date),
        make_test_player(
            2,
            &[
                (PlayerPositionType::DefenderLeft, 20),
                (PlayerPositionType::DefenderCenterLeft, 20),
            ],
            180,
            date,
        ),
        make_test_player(3, &[(PlayerPositionType::DefenderLeft, 18)], 135, date),
        make_test_player(4, &[(PlayerPositionType::DefenderCenterLeft, 5)], 80, date),
    ];

    let filler_positions = [
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ];
    for (idx, pos) in filler_positions.iter().enumerate() {
        players.push(make_test_player(10 + idx as u32, &[(*pos, 16)], 120, date));
    }

    team.players = PlayerCollection::new(players);
    team.tactics = Some(Tactics::new(MatchTacticType::T442));

    let result = SquadSelector::select(&team, &staff);
    let elite_pos = result
        .main_squad
        .iter()
        .find(|p| p.id == 2)
        .map(|p| p.tactical_position.current_position);
    let specialist_pos = result
        .main_squad
        .iter()
        .find(|p| p.id == 3)
        .map(|p| p.tactical_position.current_position);

    assert_eq!(elite_pos, Some(PlayerPositionType::DefenderCenterLeft));
    assert_eq!(specialist_pos, Some(PlayerPositionType::DefenderLeft));
}

#[test]
fn test_selection_policy_from_context() {
    assert_eq!(
        SelectionPolicy::from_context(&SelectionContext {
            match_importance: 0.9,
            ..SelectionContext::default()
        }),
        SelectionPolicy::BestEleven
    );
    assert_eq!(
        SelectionPolicy::from_context(&SelectionContext {
            match_importance: 0.3,
            ..SelectionContext::default()
        }),
        SelectionPolicy::CupRotation
    );
    assert_eq!(
        SelectionPolicy::from_context(&SelectionContext {
            is_friendly: true,
            ..SelectionContext::default()
        }),
        SelectionPolicy::YouthDevelopment
    );
}

#[test]
fn score_player_for_slot_with_breakdown_total_matches_legacy() {
    let team = generate_test_team();
    let staff = generate_test_staff();
    let tactics = Tactics::new(MatchTacticType::T442);
    let engine = scoring::ScoringEngine::from_staff(&staff);
    let date = Utc::now().date_naive();
    for player in team.players.players() {
        for slot in [
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::Striker,
        ] {
            let group = slot.position_group();
            let total_legacy = engine.score_player_for_slot(
                player,
                slot,
                group,
                &staff,
                &tactics,
                date,
                false,
                &[],
            );
            let (total_new, breakdown) = engine.score_player_for_slot_with_breakdown(
                player,
                slot,
                group,
                &staff,
                &tactics,
                date,
                false,
                &[],
            );
            assert!(
                (total_legacy - total_new).abs() < 1e-3,
                "breakdown total {} must match legacy total {}",
                total_new,
                total_legacy
            );
            assert!(
                (breakdown.total() - total_new).abs() < 1e-3,
                "summed breakdown {} must match returned total {}",
                breakdown.total(),
                total_new
            );
        }
    }
}

// ========== Test helpers ==========

fn generate_test_team() -> Team {
    let mut team = TeamBuilder::new()
        .id(1)
        .league_id(Some(1))
        .club_id(1)
        .name("Test Team".to_string())
        .slug("test-team".to_string())
        .team_type(TeamType::Main)
        .training_schedule(TrainingSchedule::new(
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        ))
        .reputation(TeamReputation::new(100, 100, 100))
        .players(PlayerCollection::new(generate_test_players()))
        .staffs(StaffCollection::new(Vec::new()))
        .tactics(Some(Tactics::new(MatchTacticType::T442)))
        .build()
        .expect("Failed to build test team");

    team.tactics = Some(Tactics::new(MatchTacticType::T442));
    team
}

fn generate_test_staff() -> Staff {
    StaffStub::default()
}

fn generate_test_players() -> Vec<Player> {
    let mut players = Vec::new();
    let names = test_names();

    for &position in &[
        PlayerPositionType::Goalkeeper,
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenter,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenter,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::Striker,
    ] {
        for _ in 0..3 {
            let level = IntegerUtils::random(15, 20) as u8;
            let player =
                PlayerGenerator::generate(1, Utc::now().date_naive(), position, level, &names);
            players.push(player);
        }
    }

    players
}

fn generate_defender_center() -> Player {
    let names = test_names();
    PlayerGenerator::generate(
        1,
        Utc::now().date_naive(),
        PlayerPositionType::DefenderCenter,
        18,
        &names,
    )
}

fn make_test_player(
    id: u32,
    positions: &[(PlayerPositionType, u8)],
    current_ability: u8,
    date: NaiveDate,
) -> Player {
    let mut player =
        PlayerGenerator::generate(1, date, positions[0].0, positions[0].1, &test_names());
    player.id = id;
    player.positions = PlayerPositions {
        positions: positions
            .iter()
            .map(|(position, level)| PlayerPosition {
                position: *position,
                level: *level,
            })
            .collect(),
    };
    player.player_attributes.current_ability = current_ability;
    player.player_attributes.condition = 9500;
    player.player_attributes.fitness = 9000;
    player.player_attributes.days_since_last_match = 7;
    player
}

// ========== Domestic cup selection tests ==========

#[allow(clippy::too_many_arguments)]
fn make_cup_player(
    id: u32,
    position: PlayerPositionType,
    level: u8,
    status: PlayerSquadStatus,
    age_years: i32,
    days_idle: u16,
    played: u16,
    physical_load_7: f32,
) -> Player {
    let date = Utc::now().date_naive();
    let mut player = PlayerGenerator::generate(1, date, position, level, &test_names());
    player.id = id;
    player.positions = PlayerPositions {
        positions: vec![PlayerPosition { position, level }],
    };
    player.player_attributes.current_ability = (level as u16 * 10).min(255) as u8;
    player.player_attributes.condition = 9500;
    player.player_attributes.fitness = 9000;
    player.player_attributes.days_since_last_match = days_idle;
    player.statistics.played = played;
    player.load.physical_load_7 = physical_load_7;
    player.birth_date = NaiveDate::from_ymd_opt(date.year() - age_years, 1, 1).unwrap();

    let mut contract = PlayerClubContract::new(
        10_000,
        NaiveDate::from_ymd_opt(date.year() + 2, 6, 1).unwrap(),
    );
    contract.squad_status = status;
    player.contract = Some(contract);
    player
}

fn cup_team(players: Vec<Player>) -> Team {
    let mut team = TeamBuilder::new()
        .id(1)
        .league_id(Some(1))
        .club_id(1)
        .name("Cup Team".to_string())
        .slug("cup-team".to_string())
        .team_type(TeamType::Main)
        .training_schedule(TrainingSchedule::new(
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        ))
        .reputation(TeamReputation::new(100, 100, 100))
        .players(PlayerCollection::new(players))
        .staffs(StaffCollection::new(Vec::new()))
        .tactics(Some(Tactics::new(MatchTacticType::T442)))
        .build()
        .expect("Failed to build cup team");
    team.tactics = Some(Tactics::new(MatchTacticType::T442));
    team
}

fn domestic_cup_ctx(round: u8, total: u8, own: u16, opp: u16, importance: f32) -> SelectionContext {
    SelectionContext {
        match_importance: importance,
        competition: SelectionCompetition::DomesticCup {
            round,
            total_rounds: total,
            own_reputation: own,
            opponent_reputation: opp,
        },
        ..SelectionContext::default()
    }
}

/// A starter (high status, fresh, heavily loaded) plus a fringe alternative
/// (rotation/backup/prospect, idle, no minutes) for each formation slot.
/// Starters get odd ids, fringe players even ids — so the tests can count
/// which group started without re-deriving squad status.
fn paired_cup_roster() -> Vec<Player> {
    let positions = [
        PlayerPositionType::Goalkeeper,
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenterLeft,
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ];
    let mut players = Vec::new();
    let mut id = 1u32;
    for (i, pos) in positions.iter().enumerate() {
        let star_status = if i % 2 == 0 {
            PlayerSquadStatus::KeyPlayer
        } else {
            PlayerSquadStatus::FirstTeamRegular
        };
        // Established starter: high status, fresh, recently overloaded.
        players.push(make_cup_player(id, *pos, 16, star_status, 27, 2, 25, 320.0));
        id += 1;

        let (fringe_status, fringe_age) = match i % 4 {
            0 => (PlayerSquadStatus::MainBackupPlayer, 24),
            1 => (PlayerSquadStatus::FirstTeamSquadRotation, 22),
            2 => (PlayerSquadStatus::HotProspectForTheFuture, 18),
            _ => (PlayerSquadStatus::DecentYoungster, 20),
        };
        // Fringe alternative: idle, no minutes this season.
        players.push(make_cup_player(
            id,
            *pos,
            15,
            fringe_status,
            fringe_age,
            21,
            0,
            0.0,
        ));
        id += 1;
    }
    players
}

#[test]
fn early_cup_rotation_starts_fringe_players() {
    // Round 1 of 5 against a weaker opponent. With a like-for-like fringe
    // option at every slot, the opportunity bias should hand at least five
    // starts to rotation/backup/prospect players (the even ids).
    let staff = generate_test_staff();
    let team = cup_team(paired_cup_roster());
    let importance = 0.30;
    let ctx = domestic_cup_ctx(1, 5, 1000, 600, importance);

    assert_eq!(
        SelectionPolicy::from_context(&ctx),
        SelectionPolicy::CupRotation
    );

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);
    assert_eq!(result.main_squad.len(), 11);

    let fringe_starters = result.main_squad.iter().filter(|p| p.id % 2 == 0).count();
    let core_starters = result.main_squad.iter().filter(|p| p.id % 2 == 1).count();
    assert!(
        fringe_starters >= 5,
        "early cup should start >=5 fringe players, got {fringe_starters}"
    );
    assert!(
        core_starters <= 6,
        "early cup should not lean on the established XI, started {core_starters}"
    );
}

#[test]
fn force_selection_overrides_cup_rotation() {
    // A force-selected established player starts even when early-round cup
    // rotation would otherwise bench him.
    let staff = generate_test_staff();
    let mut players = paired_cup_roster();
    // id 3 is an outfield starter (odd id). Pin him to the senior XI.
    let forced_id = 3;
    for p in players.iter_mut() {
        if p.id == forced_id {
            p.is_force_match_selection = true;
        }
    }
    let team = cup_team(players);
    let ctx = domestic_cup_ctx(1, 5, 1000, 600, 0.30);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);
    assert!(
        result.main_squad.iter().any(|p| p.id == forced_id),
        "force-selected player must start regardless of cup rotation"
    );
}

#[test]
fn cup_selection_keeps_safety_guarantees() {
    // Injured / banned / international-duty players stay unavailable, the XI
    // still fields a goalkeeper, and no id appears twice across XI and bench.
    let staff = generate_test_staff();
    let mut players = paired_cup_roster();
    let date = Utc::now().date_naive();
    for p in players.iter_mut() {
        match p.id {
            5 => p.player_attributes.is_injured = true,
            7 => p.player_attributes.is_banned = true,
            9 => p.statuses.add(date, PlayerStatusType::Int),
            _ => {}
        }
    }
    let team = cup_team(players);
    let ctx = domestic_cup_ctx(1, 5, 1000, 600, 0.30);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);

    let mut all_ids: Vec<u32> = result.main_squad.iter().map(|p| p.id).collect();
    all_ids.extend(result.substitutes.iter().map(|p| p.id));
    for unavailable in [5u32, 7, 9] {
        assert!(
            !all_ids.contains(&unavailable),
            "unavailable player {unavailable} must not be selected"
        );
    }

    let unique = {
        let mut s = all_ids.clone();
        s.sort_unstable();
        s.dedup();
        s.len()
    };
    assert_eq!(all_ids.len(), unique, "no player may appear twice");

    assert!(
        result
            .main_squad
            .iter()
            .any(|p| p.tactical_position.current_position == PlayerPositionType::Goalkeeper),
        "starting XI must contain a goalkeeper"
    );
}

/// Roster where a low-quality established KeyPlayer (id 999) is squeezed out
/// of the matchday squad entirely — used to check the omission reason.
fn squeezed_keyplayer_roster() -> Vec<Player> {
    let positions = [
        PlayerPositionType::Goalkeeper,
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenterLeft,
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ];
    let mut players = Vec::new();
    let mut id = 1u32;
    for pos in positions.iter() {
        for _ in 0..2 {
            players.push(make_cup_player(
                id,
                *pos,
                15,
                PlayerSquadStatus::FirstTeamRegular,
                24,
                5,
                10,
                100.0,
            ));
            id += 1;
        }
    }
    // The squeezed star: a low-quality, overloaded 29-year-old KeyPlayer who
    // can't beat the 22 fitter squad players for a place.
    players.push(make_cup_player(
        999,
        PlayerPositionType::ForwardLeft,
        6,
        PlayerSquadStatus::KeyPlayer,
        29,
        2,
        30,
        400.0,
    ));
    players
}

#[test]
fn cup_rotation_omits_keyplayer_as_cup_rotation() {
    let staff = generate_test_staff();
    let team = cup_team(squeezed_keyplayer_roster());
    let ctx = domestic_cup_ctx(1, 5, 1000, 1000, 0.30);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);
    assert!(
        !result.main_squad.iter().any(|p| p.id == 999),
        "the squeezed KeyPlayer should not start"
    );
    let omission = result
        .omissions
        .iter()
        .find(|o| o.player_id == 999)
        .expect("KeyPlayer omission should be recorded");
    assert_eq!(
        omission.context.reason,
        SelectionOmissionReason::CupRotation,
        "a rotated KeyPlayer in a domestic cup is a CupRotation omission"
    );
}

#[test]
fn league_dead_rubber_omits_keyplayer_as_low_importance() {
    // Same squeeze, but a low-importance LEAGUE fixture (no cup context):
    // the reason must be LowMatchImportanceRotation, never CupRotation.
    let staff = generate_test_staff();
    let team = cup_team(squeezed_keyplayer_roster());
    let ctx = SelectionContext {
        match_importance: 0.15,
        competition: SelectionCompetition::League,
        ..SelectionContext::default()
    };

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);
    let omission = result
        .omissions
        .iter()
        .find(|o| o.player_id == 999)
        .expect("KeyPlayer omission should be recorded");
    assert_eq!(
        omission.context.reason,
        SelectionOmissionReason::LowMatchImportanceRotation,
        "a league dead-rubber rotation is not CupRotation"
    );
}

/// Two keepers — an established #1 and a rested backup a notch below — plus
/// one outfielder per outfield slot, so a full XI forms and the only real
/// contest is in goal.
fn two_keeper_roster() -> Vec<Player> {
    let mut players = vec![
        make_cup_player(
            1,
            PlayerPositionType::Goalkeeper,
            16,
            PlayerSquadStatus::KeyPlayer,
            27,
            3,
            25,
            0.0,
        ),
        make_cup_player(
            2,
            PlayerPositionType::Goalkeeper,
            15,
            PlayerSquadStatus::MainBackupPlayer,
            24,
            21,
            0,
            0.0,
        ),
    ];
    let outfield = [
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenterLeft,
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ];
    for (i, pos) in outfield.iter().enumerate() {
        players.push(make_cup_player(
            10 + i as u32,
            *pos,
            15,
            PlayerSquadStatus::FirstTeamRegular,
            24,
            5,
            10,
            100.0,
        ));
    }
    players
}

fn starting_goalkeeper_id(result: &PlayerSelectionResult) -> Option<u32> {
    result
        .main_squad
        .iter()
        .find(|p| p.tactical_position.current_position == PlayerPositionType::Goalkeeper)
        .map(|p| p.id)
}

#[test]
fn early_cup_starts_rested_backup_goalkeeper() {
    // Early round vs an equal opponent, modest ability gap: the rested backup
    // (id 2) gets the run.
    let staff = generate_test_staff();
    let team = cup_team(two_keeper_roster());
    let ctx = domestic_cup_ctx(1, 5, 1000, 1000, 0.30);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);
    assert_eq!(
        starting_goalkeeper_id(&result),
        Some(2),
        "early cup should give the rested backup keeper a start"
    );
}

#[test]
fn cup_final_starts_first_choice_goalkeeper() {
    // The final reverts to the #1 keeper (id 1) — no rotation bias.
    let staff = generate_test_staff();
    let team = cup_team(two_keeper_roster());
    let ctx = domestic_cup_ctx(5, 5, 1000, 1000, 0.95);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);
    assert_eq!(
        starting_goalkeeper_id(&result),
        Some(1),
        "the final demands the first-choice keeper"
    );
}

#[test]
fn domestic_cup_context_carries_bracket_position() {
    // The per-side context keeps the raw bracket position and derives the
    // stage from it; non-cup competitions yield no context at all.
    let date = Utc::now().date_naive();
    let comp = SelectionCompetition::DomesticCup {
        round: 3,
        total_rounds: 5,
        own_reputation: 1000,
        opponent_reputation: 2000,
    };
    let cup = comp
        .domestic_cup_context(date)
        .expect("a domestic cup tie yields a context");
    assert_eq!(cup.round, 3);
    assert_eq!(cup.total_rounds, 5);
    assert_eq!(cup.stage(), CupStage::Quarter);
    assert!((cup.opponent_ratio - 2.0).abs() < 1e-6);

    assert!(
        SelectionCompetition::League
            .domestic_cup_context(date)
            .is_none()
    );
    assert!(
        SelectionCompetition::ContinentalCup
            .domestic_cup_context(date)
            .is_none()
    );
}
