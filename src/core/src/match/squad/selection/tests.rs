use super::*;
use super::scoring::ScoringEngine;
use crate::PlayerPositions;
use crate::StaffStub;
use crate::Team;
use crate::club::{ClubPhilosophy, PlayerFieldPositionGroup};
use crate::{
    IntegerUtils, MatchTacticType, PeopleNameGeneratorData, PlayerClubContract, PlayerCollection,
    PlayerGenerator, PlayerPosition, PlayerSquadStatus, PlayerStatusType, SelectionOmissionReason,
    StaffCollection, TeamBuilder, TeamReputation, TeamType, TrainingSchedule,
};
use chrono::NaiveDate;
use chrono::{Datelike, Duration, NaiveTime, Utc};

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

// ========== Cup rotation strength tests ==========
//
// All use `paired_cup_roster` (one established star + one idle fringe per
// slot) and count non-established starters via id parity — odd = star,
// even = fringe.

fn non_established_starter_count(result: &PlayerSelectionResult) -> usize {
    result.main_squad.iter().filter(|p| p.id % 2 == 0).count()
}

#[test]
fn early_cup_vs_weak_opponent_rotates_heavily() {
    // Round 1 of 5, own 3000 vs opp 900 (ratio ~0.3). Strong rotation
    // multiplier + post-assignment swap pass target >=7 non-established
    // starters. With like-for-like fringe at every slot, the rotation
    // should hit that target.
    let staff = generate_test_staff();
    let team = cup_team(paired_cup_roster());
    let ctx = domestic_cup_ctx(1, 5, 3000, 900, 0.30);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);
    assert_eq!(result.main_squad.len(), 11);

    let fringe = non_established_starter_count(&result);
    assert!(
        fringe >= 7,
        "early cup vs weak opponent should start >=7 non-established players, got {fringe}"
    );
}

#[test]
fn early_cup_vs_equal_rotates_significantly() {
    // Equal opponents in round 1 of 5: rotation_multiplier 1.0, target 6
    // non-established starters from the post-assignment pass.
    let staff = generate_test_staff();
    let team = cup_team(paired_cup_roster());
    let ctx = domestic_cup_ctx(1, 5, 3000, 3000, 0.30);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);

    let fringe = non_established_starter_count(&result);
    assert!(
        fringe >= 6,
        "early cup vs equal opponent should start >=6 non-established players, got {fringe}"
    );
}

#[test]
fn early_cup_vs_much_stronger_opponent_still_rotates_some() {
    // Heavy underdog (own 1000 vs opp 2500, ratio 2.5). Multiplier 0.55,
    // target only 4 non-established starters — the manager doesn't field
    // his reserves wholesale against the favourite. But still some
    // rotation, and not a full reserve XI.
    let staff = generate_test_staff();
    let team = cup_team(paired_cup_roster());
    let ctx = domestic_cup_ctx(1, 5, 1000, 2500, 0.35);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);

    let fringe = non_established_starter_count(&result);
    assert!(
        fringe >= 3,
        "even vs stronger opponent, early cup should rotate >=3, got {fringe}"
    );
}

#[test]
fn quarterfinal_weak_opponent_keeps_rotation() {
    // Quarterfinal (round 3 of 5) vs weak opponent: post-assignment target
    // 4 non-established starters. Coefficients are halved vs Early so
    // assignment alone may not hit the target — the swap pass does.
    let staff = generate_test_staff();
    let team = cup_team(paired_cup_roster());
    let ctx = domestic_cup_ctx(3, 5, 3000, 900, 0.55);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);

    let fringe = non_established_starter_count(&result);
    assert!(
        fringe >= 4,
        "quarterfinal vs weak opponent should start >=4 non-established, got {fringe}"
    );
}

#[test]
fn semifinal_returns_toward_strength() {
    // Semi: no swap pass, status base barely tilts. The XI should be
    // notably more established than at the quarter stage on the same
    // roster.
    let staff = generate_test_staff();
    let team = cup_team(paired_cup_roster());
    let quarter_ctx = domestic_cup_ctx(3, 5, 3000, 3000, 0.55);
    let semi_ctx = domestic_cup_ctx(4, 5, 3000, 3000, 0.78);

    let quarter = SquadSelector::select_with_context(&team, &staff, &[], &quarter_ctx);
    let semi = SquadSelector::select_with_context(&team, &staff, &[], &semi_ctx);

    let q_fringe = non_established_starter_count(&quarter);
    let s_fringe = non_established_starter_count(&semi);
    assert!(
        s_fringe <= q_fringe,
        "semifinal should not rotate harder than quarterfinal: semi={s_fringe} quarter={q_fringe}"
    );
}

#[test]
fn final_starts_strongest_outfield_xi() {
    // The final reverts to the established XI: opportunity bias is zero,
    // no swap pass runs. Most outfield slots should belong to the
    // established (odd-id) starter rather than the idle fringe player.
    let staff = generate_test_staff();
    let team = cup_team(paired_cup_roster());
    let ctx = domestic_cup_ctx(5, 5, 3000, 3000, 0.95);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);

    let established = result.main_squad.iter().filter(|p| p.id % 2 == 1).count();
    assert!(
        established >= 8,
        "final should field the established XI, got only {established} established starters"
    );
}

#[test]
fn force_selected_established_not_swapped_in_cup_rotation() {
    // Stronger version of the existing force-selection test: even after
    // the swap pass runs in early/weak conditions, a force-selected
    // established player still starts.
    let staff = generate_test_staff();
    let mut players = paired_cup_roster();
    let forced_id = 3u32; // an established outfielder
    for p in players.iter_mut() {
        if p.id == forced_id {
            p.is_force_match_selection = true;
        }
    }
    let team = cup_team(players);
    // Heavy-rotation context: weak opponent, early round.
    let ctx = domestic_cup_ctx(1, 5, 3000, 900, 0.30);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);
    assert!(
        result.main_squad.iter().any(|p| p.id == forced_id),
        "force-selected player must survive the swap pass and remain in XI"
    );
}

#[test]
fn cup_rotation_preserves_team_shape() {
    // The rotation pass must not corrupt the XI: no duplicate ids across
    // XI and bench, exactly one goalkeeper, all 11 tactical slots filled.
    let staff = generate_test_staff();
    let team = cup_team(paired_cup_roster());
    let ctx = domestic_cup_ctx(1, 5, 3000, 900, 0.30);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);

    assert_eq!(result.main_squad.len(), 11, "XI must always be 11");

    let mut all_ids: Vec<u32> = result.main_squad.iter().map(|p| p.id).collect();
    all_ids.extend(result.substitutes.iter().map(|p| p.id));
    let unique = {
        let mut s = all_ids.clone();
        s.sort_unstable();
        s.dedup();
        s.len()
    };
    assert_eq!(all_ids.len(), unique, "no player may appear twice");

    let gk_count = result
        .main_squad
        .iter()
        .filter(|p| p.tactical_position.current_position == PlayerPositionType::Goalkeeper)
        .count();
    assert_eq!(gk_count, 1, "exactly one starting goalkeeper");
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

// ========== Future-aware squad management ==========
//
// Fixtures for the future-aware pathway layer. Grouped into namespace
// structs (a coach factory, a development-player factory, and a single-slot
// contest builder) so the tests read as squad-management scenarios rather
// than a wall of loose helpers.

/// Coach factory: turns staff attribute dials into a `Staff` so a test can
/// pin exactly the youth-development / judgement traits it cares about.
struct TestCoach;

impl TestCoach {
    #[allow(clippy::too_many_arguments)]
    fn build(
        working_with_youngsters: u8,
        judging_potential: u8,
        judging_ability: u8,
        adaptability: u8,
        man_management: u8,
        motivating: u8,
        determination: u8,
        discipline: u8,
    ) -> Staff {
        let mut staff = StaffStub::default();
        let coaching = &mut staff.staff_attributes.coaching;
        coaching.working_with_youngsters = working_with_youngsters;
        // Give a credible fitness/mental read so match-readiness noise stays
        // small and the scenarios are stable.
        coaching.fitness = 16;
        coaching.mental = 16;
        coaching.tactical = 12;
        let knowledge = &mut staff.staff_attributes.knowledge;
        knowledge.judging_player_potential = judging_potential;
        knowledge.judging_player_ability = judging_ability;
        knowledge.tactical_knowledge = 12;
        let mental = &mut staff.staff_attributes.mental;
        mental.adaptability = adaptability;
        mental.man_management = man_management;
        mental.motivating = motivating;
        mental.determination = determination;
        mental.discipline = discipline;
        staff
    }

    /// Strong with youngsters: reads potential well, adaptable, man-manages,
    /// low conservatism.
    fn good_youth() -> Staff {
        Self::build(18, 17, 15, 16, 16, 15, 8, 6)
    }

    /// Weak with youngsters: distrusts kids, poor potential judge, rigid and
    /// disciplinarian (high conservatism).
    fn poor_youth() -> Staff {
        Self::build(3, 4, 12, 5, 8, 8, 16, 16)
    }
}

/// Development-player factory: a fit player at a chosen slot with explicit
/// current/potential ability and a controlled training/readiness profile, so
/// credibility maths is deterministic.
struct DevPlayer;

impl DevPlayer {
    #[allow(clippy::too_many_arguments)]
    fn build(
        id: u32,
        position: PlayerPositionType,
        level: u8,
        status: PlayerSquadStatus,
        age: i32,
        current_ability: u8,
        potential_ability: u8,
        days_idle: u16,
        played: u16,
    ) -> Player {
        let mut player = make_cup_player(id, position, level, status, age, days_idle, played, 60.0);
        player.player_attributes.current_ability = current_ability;
        player.player_attributes.potential_ability = potential_ability;
        player.player_attributes.condition = 9500;
        player.player_attributes.fitness = 9000;
        // Neutral form for everyone: without it, a player with appearances
        // takes the season-average path (default rating 0 → a −1.5 form
        // penalty) while a player with none is spared it — an asymmetry that
        // would quietly tilt senior-vs-youth contests.
        player.load.form_rating = 6.5;
        player.skills.physical.match_readiness = 15.0;
        player.skills.mental.work_rate = 12.0;
        player.skills.mental.determination = 12.0;
        player.skills.mental.teamwork = 12.0;
        player.training.training_performance = 12.0;
        player
    }

    /// An aging incumbent being phased out: backup role, idle, an expiring
    /// deal and reduced condition so the fragility signal reads. Still skilled
    /// enough to be a real selection — succession only bites once a credible
    /// younger option exists.
    fn aging_incumbent(id: u32, position: PlayerPositionType, age: i32) -> Player {
        let mut player = Self::build(
            id,
            position,
            15,
            PlayerSquadStatus::MainBackupPlayer,
            age,
            150,
            150,
            25,
            3,
        );
        let date = Utc::now().date_naive();
        if let Some(contract) = player.contract.as_mut() {
            contract.expiration = date + Duration::days(120);
        }
        player.player_attributes.condition = 6000;
        player
    }

    /// Flatten every outfield skill to `value` so perceived quality is
    /// deterministic. The generator scatters skills regardless of the
    /// position level, so quality contrasts have to be set explicitly here
    /// rather than inferred from the `level` argument.
    fn with_skill(mut player: Player, value: f32) -> Player {
        let technical = &mut player.skills.technical;
        technical.corners = value;
        technical.crossing = value;
        technical.dribbling = value;
        technical.finishing = value;
        technical.first_touch = value;
        technical.free_kicks = value;
        technical.heading = value;
        technical.long_shots = value;
        technical.long_throws = value;
        technical.marking = value;
        technical.passing = value;
        technical.penalty_taking = value;
        technical.tackling = value;
        technical.technique = value;
        let mental = &mut player.skills.mental;
        mental.aggression = value;
        mental.anticipation = value;
        mental.bravery = value;
        mental.composure = value;
        mental.concentration = value;
        mental.decisions = value;
        mental.determination = value;
        mental.flair = value;
        mental.leadership = value;
        mental.off_the_ball = value;
        mental.positioning = value;
        mental.teamwork = value;
        mental.vision = value;
        mental.work_rate = value;
        let physical = &mut player.skills.physical;
        physical.acceleration = value;
        physical.agility = value;
        physical.balance = value;
        physical.jumping = value;
        physical.natural_fitness = value;
        physical.pace = value;
        physical.stamina = value;
        physical.strength = value;
        player
    }
}

/// Single-slot contest builder: a senior and a young player able to fill the
/// same one formation slot, plus a filler for every other slot, so exactly
/// one of the two starts and the loser drops to the bench.
struct Contest;

impl Contest {
    const SLOT: PlayerPositionType = PlayerPositionType::ForwardLeft;

    /// A senior incumbent (id 1) with a modest *skill* edge and an underused
    /// young prospect (id 2). The edge lives in skills — not status or
    /// reputation — so the senior can't carry it into a cross-fill at the
    /// other forward slot; the contest stays at `SLOT`, and the loser benches.
    /// Both carry neutral form so appearances don't skew perceived quality.
    fn contest_pair() -> (Player, Player) {
        let senior = DevPlayer::with_skill(
            DevPlayer::build(
                1,
                Self::SLOT,
                17,
                PlayerSquadStatus::FirstTeamRegular,
                28,
                165,
                168,
                5,
                20,
            ),
            14.0,
        );
        let youth = DevPlayer::with_skill(
            DevPlayer::build(
                2,
                Self::SLOT,
                17,
                PlayerSquadStatus::HotProspectForTheFuture,
                19,
                140,
                185,
                14,
                0,
            ),
            11.0,
        );
        (senior, youth)
    }

    /// Full T442 roster around the contested pair. Every filler is an
    /// established, neutral-form player skilled to match the senior, so no
    /// filler is displaced by a cross-fill — the loser of `SLOT` has nowhere
    /// to go but the bench.
    fn roster(senior: Player, youth: Player) -> Vec<Player> {
        let slots = [
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
        let mut id = 200u32;
        for &pos in slots.iter() {
            if pos == Self::SLOT {
                continue;
            }
            players.push(DevPlayer::with_skill(
                DevPlayer::build(id, pos, 16, PlayerSquadStatus::FirstTeamRegular, 26, 160, 162, 5, 12),
                14.0,
            ));
            id += 1;
        }
        players.push(senior);
        players.push(youth);
        players
    }

    fn ctx(
        importance: f32,
        philosophy: ClubPhilosophy,
        competition: SelectionCompetition,
    ) -> SelectionContext {
        SelectionContext {
            match_importance: importance,
            philosophy: Some(philosophy),
            competition,
            ..SelectionContext::default()
        }
    }

    fn cup(round: u8, total: u8, own: u16, opp: u16) -> DomesticCupContext {
        SelectionCompetition::DomesticCup {
            round,
            total_rounds: total,
            own_reputation: own,
            opponent_reputation: opp,
        }
        .domestic_cup_context(Utc::now().date_naive())
        .unwrap()
    }

    fn started(result: &PlayerSelectionResult, id: u32) -> bool {
        result.main_squad.iter().any(|p| p.id == id)
    }
}

#[test]
fn important_match_keeps_strongest_xi_over_youth_pathway() {
    let staff = TestCoach::good_youth();
    let engine =
        ScoringEngine::from_staff_for_team(&staff, Some(ClubPhilosophy::DevelopAndSell), true);
    let date = Utc::now().date_naive();
    let slot = Contest::SLOT;
    let youth = DevPlayer::build(
        1,
        slot,
        15,
        PlayerSquadStatus::HotProspectForTheFuture,
        19,
        140,
        190,
        7,
        0,
    );
    // Title decider / final stakes: the pathway layer is fully switched off.
    assert_eq!(
        engine.future_pathway_adjustment(&youth, slot, 0.9, date, None, &[], true),
        0.0,
        "future pathway must be off in a high-importance match"
    );
    assert_eq!(engine.pathway_context_multiplier(0.9, None, true), 0.0);
}

#[test]
fn develop_and_sell_early_cup_starts_credible_young_player() {
    let staff = StaffStub::default();
    let (senior, youth) = Contest::contest_pair();
    let team = cup_team(Contest::roster(senior, youth));
    let ctx = Contest::ctx(
        0.30,
        ClubPhilosophy::DevelopAndSell,
        SelectionCompetition::DomesticCup {
            round: 1,
            total_rounds: 5,
            own_reputation: 1000,
            opponent_reputation: 600,
        },
    );

    let result = SquadSelector::select_with_context(&team, &staff, &[], &ctx);
    assert!(
        Contest::started(&result, 2),
        "a DevelopAndSell side should start the credible young player in an early cup tie"
    );
}

#[test]
fn weak_young_player_does_not_displace_senior_player() {
    let staff = TestCoach::good_youth();
    let engine =
        ScoringEngine::from_staff_for_team(&staff, Some(ClubPhilosophy::DevelopAndSell), true);
    let date = Utc::now().date_naive();
    let tactics = Tactics::new(MatchTacticType::T442);
    let slot = Contest::SLOT;
    let group = slot.position_group();
    // Clearly better senior (strong skills) vs a weak youngster (low CA & PA,
    // weak raw skills) who can still nominally fill the slot.
    let senior = DevPlayer::with_skill(
        DevPlayer::build(1, slot, 16, PlayerSquadStatus::FirstTeamRegular, 27, 168, 170, 5, 20),
        16.0,
    );
    let weak_youth = DevPlayer::with_skill(
        DevPlayer::build(2, slot, 14, PlayerSquadStatus::DecentYoungster, 18, 75, 88, 7, 0),
        6.0,
    );
    let pool: Vec<&Player> = vec![&senior, &weak_youth];
    let total = |p: &Player| {
        engine.score_player_for_slot(p, slot, group, &staff, &tactics, date, false, &[])
            + engine.future_pathway_adjustment(p, slot, 0.2, date, None, &pool, true)
    };
    assert!(
        total(&senior) > total(&weak_youth),
        "a clearly better senior keeps his place over a weak youngster in a low-risk match: \
         senior {} vs youth {}",
        total(&senior),
        total(&weak_youth)
    );
}

#[test]
fn sign_to_compete_reduces_youth_pathway_bonus() {
    let staff = TestCoach::good_youth();
    let date = Utc::now().date_naive();
    let slot = Contest::SLOT;
    let youth = DevPlayer::build(
        1,
        slot,
        15,
        PlayerSquadStatus::HotProspectForTheFuture,
        19,
        140,
        190,
        7,
        0,
    );
    let pathway = |philosophy: ClubPhilosophy| {
        ScoringEngine::from_staff_for_team(&staff, Some(philosophy), true)
            .future_pathway_adjustment(&youth, slot, 0.2, date, None, &[], true)
    };
    let develop = pathway(ClubPhilosophy::DevelopAndSell);
    let balanced = pathway(ClubPhilosophy::Balanced);
    let compete = pathway(ClubPhilosophy::SignToCompete);
    assert!(
        develop > balanced && balanced > compete,
        "DevelopAndSell {develop} > Balanced {balanced} > SignToCompete {compete}"
    );
    assert!(compete >= 0.0);
}

#[test]
fn coach_good_with_youngsters_gives_prospect_low_risk_start() {
    let date = Utc::now().date_naive();
    let slot = Contest::SLOT;
    let youth = DevPlayer::build(
        1,
        slot,
        15,
        PlayerSquadStatus::HotProspectForTheFuture,
        19,
        140,
        190,
        7,
        0,
    );
    let good = TestCoach::good_youth();
    let poor = TestCoach::poor_youth();
    let pathway = |staff: &Staff| {
        ScoringEngine::from_staff_for_team(staff, Some(ClubPhilosophy::Balanced), true)
            .future_pathway_adjustment(&youth, slot, 0.2, date, None, &[], true)
    };
    let good_pull = pathway(&good);
    let poor_pull = pathway(&poor);
    assert!(
        good_pull > poor_pull,
        "a coach good with youngsters gives a bigger pathway pull: good {good_pull} vs poor {poor_pull}"
    );
    assert!(good_pull > 0.0);
}

#[test]
fn coach_poor_with_youngsters_prefers_senior_in_same_context() {
    let ctx = Contest::ctx(0.48, ClubPhilosophy::Balanced, SelectionCompetition::League);
    let good = TestCoach::good_youth();
    let poor = TestCoach::poor_youth();

    let (senior_g, youth_g) = Contest::contest_pair();
    let team_good = cup_team(Contest::roster(senior_g, youth_g));
    let result_good = SquadSelector::select_with_context(&team_good, &good, &[], &ctx);

    let (senior_p, youth_p) = Contest::contest_pair();
    let team_poor = cup_team(Contest::roster(senior_p, youth_p));
    let result_poor = SquadSelector::select_with_context(&team_poor, &poor, &[], &ctx);

    assert!(
        Contest::started(&result_good, 2),
        "a coach good with youngsters starts the prospect in a low-risk league game"
    );
    assert!(
        Contest::started(&result_poor, 1) && !Contest::started(&result_poor, 2),
        "a coach poor with youngsters keeps the senior and benches the prospect"
    );
}

#[test]
fn high_judging_potential_identifies_high_potential_player() {
    let date = Utc::now().date_naive();
    let slot = Contest::SLOT;
    // High potential, only modest current ability — invisible to a poor judge.
    let prospect = DevPlayer::build(
        1,
        slot,
        14,
        PlayerSquadStatus::HotProspectForTheFuture,
        18,
        110,
        200,
        7,
        0,
    );
    let high_potential = TestCoach::build(14, 18, 12, 12, 12, 12, 10, 10);
    let low_potential = TestCoach::build(14, 2, 12, 12, 12, 12, 10, 10);
    let credible_high = ScoringEngine::from_staff_for_team(&high_potential, None, true)
        .player_development_credibility(&prospect, slot, date);
    let credible_low = ScoringEngine::from_staff_for_team(&low_potential, None, true)
        .player_development_credibility(&prospect, slot, date);
    assert!(
        credible_high > credible_low,
        "a better potential judge values the high-PA prospect more: {credible_high} vs {credible_low}"
    );
}

#[test]
fn low_judging_potential_requires_more_current_ability() {
    let date = Utc::now().date_naive();
    let slot = Contest::SLOT;
    let low_potential = TestCoach::build(14, 2, 12, 12, 12, 12, 10, 10);
    let engine = ScoringEngine::from_staff_for_team(&low_potential, None, true);
    // Same role and age — one is high current ability, the other all upside.
    let high_ca = DevPlayer::build(
        1,
        slot,
        15,
        PlayerSquadStatus::HotProspectForTheFuture,
        19,
        185,
        190,
        7,
        0,
    );
    let high_pa = DevPlayer::build(
        2,
        slot,
        15,
        PlayerSquadStatus::HotProspectForTheFuture,
        19,
        80,
        200,
        7,
        0,
    );
    let credible_ca = engine.player_development_credibility(&high_ca, slot, date);
    let credible_pa = engine.player_development_credibility(&high_pa, slot, date);
    assert!(
        credible_ca > credible_pa,
        "a poor potential judge leans on current ability: high-CA {credible_ca} vs high-PA {credible_pa}"
    );
}

#[test]
fn dead_rubber_league_gives_underused_prospect_minutes() {
    let staff = StaffStub::default();

    let (senior_a, youth_a) = Contest::contest_pair();
    let dead_rubber = cup_team(Contest::roster(senior_a, youth_a));
    let dead_result = SquadSelector::select_with_context(
        &dead_rubber,
        &staff,
        &[],
        &Contest::ctx(0.15, ClubPhilosophy::Balanced, SelectionCompetition::League),
    );

    let (senior_b, youth_b) = Contest::contest_pair();
    let must_win = cup_team(Contest::roster(senior_b, youth_b));
    let must_win_result = SquadSelector::select_with_context(
        &must_win,
        &staff,
        &[],
        &Contest::ctx(0.90, ClubPhilosophy::Balanced, SelectionCompetition::League),
    );

    assert!(
        Contest::started(&dead_result, 2),
        "a dead rubber should hand the underused prospect a start"
    );
    assert!(
        !Contest::started(&must_win_result, 2),
        "a must-win match keeps the senior — no pathway minutes for the prospect"
    );
}

#[test]
fn aging_incumbent_managed_only_when_successor_is_credible() {
    let date = Utc::now().date_naive();
    let slot = PlayerPositionType::DefenderCenter;
    let engine = ScoringEngine::from_staff_for_team(
        &TestCoach::good_youth(),
        Some(ClubPhilosophy::DevelopAndSell),
        true,
    );
    let aging = DevPlayer::aging_incumbent(1, slot, 35);

    assert_eq!(
        engine.late_career_succession_pressure(&aging, slot, date, &[]),
        0.0,
        "no successor -> no succession pressure"
    );

    let old_alternative = DevPlayer::build(
        2,
        slot,
        15,
        PlayerSquadStatus::FirstTeamRegular,
        33,
        150,
        150,
        7,
        5,
    );
    assert_eq!(
        engine.late_career_succession_pressure(&aging, slot, date, &[&old_alternative]),
        0.0,
        "an equally-old alternative is not a credible successor"
    );

    let young = DevPlayer::build(
        3,
        slot,
        15,
        PlayerSquadStatus::HotProspectForTheFuture,
        19,
        145,
        190,
        7,
        0,
    );
    assert!(
        engine.late_career_succession_pressure(&aging, slot, date, &[&young]) > 0.0,
        "a credible young successor opens the succession question"
    );
}

#[test]
fn cup_final_ignores_future_pathway_adjustment() {
    let date = Utc::now().date_naive();
    let engine = ScoringEngine::from_staff_for_team(
        &TestCoach::good_youth(),
        Some(ClubPhilosophy::DevelopAndSell),
        true,
    );
    let cup = Contest::cup(5, 5, 1000, 1000);
    let slot = Contest::SLOT;
    let youth = DevPlayer::build(
        1,
        slot,
        15,
        PlayerSquadStatus::HotProspectForTheFuture,
        19,
        140,
        190,
        7,
        0,
    );
    let cb_slot = PlayerPositionType::DefenderCenter;
    let aging = DevPlayer::aging_incumbent(2, cb_slot, 35);
    assert_eq!(
        engine.future_pathway_adjustment(&youth, slot, 0.95, date, Some(&cup), &[], true),
        0.0,
        "the final ignores the youth pathway pull"
    );
    assert_eq!(
        engine.future_pathway_adjustment(&aging, cb_slot, 0.95, date, Some(&cup), &[], true),
        0.0,
        "the final ignores succession pressure too"
    );
}

#[test]
fn future_pathway_bonus_is_position_group_aware() {
    let date = Utc::now().date_naive();
    let engine = ScoringEngine::from_staff_for_team(&TestCoach::good_youth(), None, true);
    let slot = Contest::SLOT;
    let youth_forward = DevPlayer::with_skill(
        DevPlayer::build(1, slot, 14, PlayerSquadStatus::HotProspectForTheFuture, 19, 130, 185, 7, 0),
        10.0,
    );
    let strong_centre_back = DevPlayer::with_skill(
        DevPlayer::build(
            2,
            PlayerPositionType::DefenderCenter,
            15,
            PlayerSquadStatus::KeyPlayer,
            27,
            175,
            175,
            5,
            20,
        ),
        16.0,
    );
    let strong_forward = DevPlayer::with_skill(
        DevPlayer::build(3, slot, 15, PlayerSquadStatus::KeyPlayer, 27, 175, 175, 5, 20),
        16.0,
    );
    // A strong senior in another unit must not gate the forward prospect.
    let gap_other_group = engine.same_role_quality_gap(&youth_forward, slot, date, &[&strong_centre_back]);
    assert!(
        gap_other_group <= 0.0001,
        "a defender must not gate a forward prospect: {gap_other_group}"
    );
    // A strong senior in the SAME unit does.
    let gap_same_group = engine.same_role_quality_gap(&youth_forward, slot, date, &[&strong_forward]);
    assert!(
        gap_same_group > 0.5,
        "a strong same-role senior gates the prospect: {gap_same_group}"
    );
}

#[test]
fn bench_includes_development_player_when_starting_gap_is_too_large() {
    let date = Utc::now().date_naive();
    let engine = ScoringEngine::from_staff_for_team(
        &TestCoach::good_youth(),
        Some(ClubPhilosophy::DevelopAndSell),
        true,
    );
    let slot = Contest::SLOT;
    let youth = DevPlayer::build(
        1,
        slot,
        13,
        PlayerSquadStatus::HotProspectForTheFuture,
        18,
        120,
        195,
        7,
        0,
    );
    let strong_senior = DevPlayer::build(
        2,
        slot,
        19,
        PlayerSquadStatus::KeyPlayer,
        27,
        185,
        185,
        5,
        20,
    );
    let pool: Vec<&Player> = vec![&strong_senior];
    // Starting is gated by the big same-role gap; the bench is not.
    let pull_start = engine.future_pathway_adjustment(&youth, slot, 0.25, date, None, &pool, true);
    let pull_bench = engine.future_pathway_adjustment(&youth, slot, 0.25, date, None, &[], false);
    assert!(
        pull_bench > pull_start,
        "ungated bench pull should exceed the gated starting pull: bench {pull_bench} vs start {pull_start}"
    );
    assert!(pull_bench > 0.0, "the prospect still earns a bench place");
}

#[test]
fn not_needed_player_does_not_gain_pathway_priority() {
    let date = Utc::now().date_naive();
    let slot = Contest::SLOT;
    let engine = ScoringEngine::from_staff_for_team(
        &TestCoach::good_youth(),
        Some(ClubPhilosophy::DevelopAndSell),
        true,
    );
    let not_needed = DevPlayer::build(1, slot, 15, PlayerSquadStatus::NotNeeded, 19, 150, 195, 7, 0);
    let prospect = DevPlayer::build(
        2,
        slot,
        15,
        PlayerSquadStatus::HotProspectForTheFuture,
        19,
        150,
        195,
        7,
        0,
    );
    assert_eq!(
        engine.future_pathway_adjustment(&not_needed, slot, 0.2, date, None, &[], true),
        0.0,
        "a frozen-out player gets no pathway pull"
    );
    assert!(
        engine.future_pathway_adjustment(&prospect, slot, 0.2, date, None, &[], true) > 0.0,
        "a prospect with the same numbers does"
    );
}

// ========== Backup-goalkeeper coverage ==========
//
// Every matchday bench should name a substitute keeper whenever one is
// available anywhere — on the team, in the reserve pool the matchday builder
// supplies (which may itself have borrowed a youth keeper), or, on a full
// bench, by displacing the most expendable outfield substitute. These tests
// drive the selector side of that guarantee through the public API and through
// `ensure_backup_goalkeeper` directly; the matchday-pool side (borrowing a
// youth keeper into the reserve list) is covered in `league::simulation::matchday`.

/// Ten established senior outfielders, one at each T442 outfield slot, plus
/// `extra_bench` rotation-grade midfield options so a realistic bench forms.
/// Keepers are supplied by each test. Outfield ids start at 300, extras at 400.
fn gk_outfield(extra_bench: usize) -> Vec<Player> {
    let slots = [
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
    let mut id = 300u32;
    for &pos in slots.iter() {
        players.push(make_cup_player(
            id,
            pos,
            15,
            PlayerSquadStatus::FirstTeamRegular,
            26,
            5,
            12,
            100.0,
        ));
        id += 1;
    }
    let mut extra_id = 400u32;
    for _ in 0..extra_bench {
        players.push(make_cup_player(
            extra_id,
            PlayerPositionType::MidfielderCenter,
            13,
            PlayerSquadStatus::FirstTeamSquadRotation,
            24,
            8,
            5,
            60.0,
        ));
        extra_id += 1;
    }
    players
}

fn league_ctx(importance: f32) -> SelectionContext {
    SelectionContext {
        match_importance: importance,
        ..SelectionContext::default()
    }
}

fn benched_as_goalkeeper(result: &PlayerSelectionResult, id: u32) -> bool {
    result.substitutes.iter().any(|p| {
        p.id == id && p.tactical_position.current_position == PlayerPositionType::Goalkeeper
    })
}

fn bench_goalkeeper_count(result: &PlayerSelectionResult) -> usize {
    result
        .substitutes
        .iter()
        .filter(|p| p.tactical_position.current_position == PlayerPositionType::Goalkeeper)
        .count()
}

#[test]
fn bench_includes_same_team_backup_goalkeeper_when_available() {
    let staff = generate_test_staff();
    let mut players = gk_outfield(6);
    players.push(make_cup_player(
        1,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::KeyPlayer,
        27,
        3,
        25,
        0.0,
    ));
    players.push(make_cup_player(
        2,
        PlayerPositionType::Goalkeeper,
        15,
        PlayerSquadStatus::MainBackupPlayer,
        24,
        10,
        4,
        0.0,
    ));
    let team = cup_team(players);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &league_ctx(0.7));

    assert_eq!(starting_goalkeeper_id(&result), Some(1), "the #1 keeper starts");
    assert!(
        benched_as_goalkeeper(&result, 2),
        "the second keeper is named on the bench"
    );
}

#[test]
fn bench_borrows_u21_goalkeeper_when_first_team_has_only_one_gk() {
    // The first team carries a single keeper; the matchday builder offers a
    // U21 keeper through the reserve pool. The selector must bench it.
    let staff = generate_test_staff();
    let mut players = gk_outfield(6);
    players.push(make_cup_player(
        1,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::KeyPlayer,
        27,
        3,
        25,
        0.0,
    ));
    let team = cup_team(players);
    let u21_keeper = make_cup_player(
        50,
        PlayerPositionType::Goalkeeper,
        12,
        PlayerSquadStatus::HotProspectForTheFuture,
        20,
        14,
        0,
        0.0,
    );
    let reserves: Vec<&Player> = vec![&u21_keeper];

    let result = SquadSelector::select_with_context(&team, &staff, &reserves, &league_ctx(0.7));

    assert_eq!(
        starting_goalkeeper_id(&result),
        Some(1),
        "the senior keeper still starts ahead of the youngster"
    );
    assert!(
        benched_as_goalkeeper(&result, 50),
        "the borrowed U21 keeper is benched as the backup"
    );
}

#[test]
fn bench_borrows_u19_goalkeeper_when_no_senior_reserve_gk_available() {
    let staff = generate_test_staff();
    let mut players = gk_outfield(6);
    players.push(make_cup_player(
        1,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::KeyPlayer,
        27,
        3,
        25,
        0.0,
    ));
    let team = cup_team(players);
    let u19_keeper = make_cup_player(
        60,
        PlayerPositionType::Goalkeeper,
        10,
        PlayerSquadStatus::DecentYoungster,
        18,
        21,
        0,
        0.0,
    );
    let reserves: Vec<&Player> = vec![&u19_keeper];

    let result = SquadSelector::select_with_context(&team, &staff, &reserves, &league_ctx(0.7));

    assert_eq!(starting_goalkeeper_id(&result), Some(1));
    assert!(
        benched_as_goalkeeper(&result, 60),
        "the borrowed U19 keeper is benched when no senior reserve keeper exists"
    );
}

#[test]
fn bench_does_not_borrow_injured_or_banned_youth_goalkeeper() {
    let staff = generate_test_staff();
    let mut players = gk_outfield(6);
    players.push(make_cup_player(
        1,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::KeyPlayer,
        27,
        3,
        25,
        0.0,
    ));
    let team = cup_team(players);
    let mut injured = make_cup_player(
        70,
        PlayerPositionType::Goalkeeper,
        12,
        PlayerSquadStatus::HotProspectForTheFuture,
        19,
        14,
        0,
        0.0,
    );
    injured.player_attributes.is_injured = true;
    let mut banned = make_cup_player(
        71,
        PlayerPositionType::Goalkeeper,
        12,
        PlayerSquadStatus::HotProspectForTheFuture,
        19,
        14,
        0,
        0.0,
    );
    banned.player_attributes.is_banned = true;
    let reserves: Vec<&Player> = vec![&injured, &banned];

    let result = SquadSelector::select_with_context(&team, &staff, &reserves, &league_ctx(0.7));

    assert_eq!(starting_goalkeeper_id(&result), Some(1));
    assert_eq!(
        bench_goalkeeper_count(&result),
        0,
        "an injured or banned youth keeper must not be benched"
    );
    let all_ids: Vec<u32> = result
        .main_squad
        .iter()
        .chain(result.substitutes.iter())
        .map(|p| p.id)
        .collect();
    assert!(
        !all_ids.contains(&70) && !all_ids.contains(&71),
        "unavailable keepers are not selected anywhere"
    );
}

#[test]
fn bench_backup_gk_does_not_duplicate_starting_gk() {
    let staff = generate_test_staff();
    let mut players = gk_outfield(6);
    players.push(make_cup_player(
        1,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::KeyPlayer,
        27,
        3,
        25,
        0.0,
    ));
    players.push(make_cup_player(
        2,
        PlayerPositionType::Goalkeeper,
        15,
        PlayerSquadStatus::MainBackupPlayer,
        24,
        10,
        4,
        0.0,
    ));
    let team = cup_team(players);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &league_ctx(0.7));

    let starter = starting_goalkeeper_id(&result).expect("a keeper starts");
    assert!(
        !result.substitutes.iter().any(|p| p.id == starter),
        "the starting keeper must not also appear on the bench"
    );
    assert_eq!(
        bench_goalkeeper_count(&result),
        1,
        "exactly one backup keeper is benched"
    );
}

/// Build a competitive scoring context for the unit-level keeper-guarantee
/// tests — a normal league fixture, no cup bias.
fn gk_unit_ctx<'a>(
    staff: &'a Staff,
    tactics: &'a Tactics,
    engine: &'a ScoringEngine,
) -> super::competitive::SelectionScoringContext<'a> {
    super::competitive::SelectionScoringContext {
        staff,
        tactics,
        engine,
        date: Utc::now().date_naive(),
        is_friendly: false,
        match_importance: 0.7,
        policy: SelectionPolicy::StrongWithRotation,
        cup: None,
        coach: None,
        competition: super::SelectionCompetition::League,
        game_model: None,
    }
}

#[test]
fn full_bench_replaces_lowest_outfield_sub_with_backup_gk() {
    let staff = generate_test_staff();
    let tactics = Tactics::new(MatchTacticType::T442);
    let engine = ScoringEngine::from_staff(&staff);
    let scx = gk_unit_ctx(&staff, &tactics, &engine);

    // A full bench of seven outfielders, no keeper, with a keeper available in
    // the remaining pool.
    let outfield: Vec<Player> = (0..helpers::DEFAULT_BENCH_SIZE as u32)
        .map(|i| {
            make_cup_player(
                100 + i,
                PlayerPositionType::MidfielderCenter,
                13,
                PlayerSquadStatus::FirstTeamSquadRotation,
                24,
                6,
                6,
                70.0,
            )
        })
        .collect();
    let keeper = make_cup_player(
        9,
        PlayerPositionType::Goalkeeper,
        14,
        PlayerSquadStatus::MainBackupPlayer,
        23,
        12,
        0,
        0.0,
    );
    let mut remaining: Vec<&Player> = outfield.iter().collect();
    remaining.push(&keeper);

    let mut subs: Vec<MatchPlayer> = outfield
        .iter()
        .map(|p| MatchPlayer::from_player(1, p, PlayerPositionType::MidfielderCenter, false))
        .collect();
    let mut used_ids: Vec<u32> = subs.iter().map(|s| s.id).collect();
    assert_eq!(subs.len(), helpers::DEFAULT_BENCH_SIZE);

    scx.ensure_backup_goalkeeper(1, &mut subs, &mut used_ids, &remaining);

    assert_eq!(
        subs.len(),
        helpers::DEFAULT_BENCH_SIZE,
        "bench size stays at the cap"
    );
    assert!(
        subs.iter().any(|s| s.id == 9
            && s.tactical_position.current_position == PlayerPositionType::Goalkeeper),
        "the keeper replaced an outfield substitute on the full bench"
    );
    let outfield_left = subs.iter().filter(|s| s.id >= 100).count();
    assert_eq!(
        outfield_left,
        helpers::DEFAULT_BENCH_SIZE - 1,
        "exactly one outfield sub was dropped for the keeper"
    );
}

#[test]
fn force_selected_outfield_sub_is_not_dropped_for_backup_gk() {
    let staff = generate_test_staff();
    let tactics = Tactics::new(MatchTacticType::T442);
    let engine = ScoringEngine::from_staff(&staff);
    let scx = gk_unit_ctx(&staff, &tactics, &engine);

    let mut outfield: Vec<Player> = (0..helpers::DEFAULT_BENCH_SIZE as u32)
        .map(|i| {
            make_cup_player(
                100 + i,
                PlayerPositionType::MidfielderCenter,
                13,
                PlayerSquadStatus::FirstTeamSquadRotation,
                24,
                6,
                6,
                70.0,
            )
        })
        .collect();
    // Pin every outfield sub except the last to the matchday squad — the lone
    // non-pinned sub (id 106) is the only one the keeper may displace.
    let last = 100 + helpers::DEFAULT_BENCH_SIZE as u32 - 1;
    for p in outfield.iter_mut() {
        if p.id != last {
            p.is_force_match_selection = true;
        }
    }
    let keeper = make_cup_player(
        9,
        PlayerPositionType::Goalkeeper,
        14,
        PlayerSquadStatus::MainBackupPlayer,
        23,
        12,
        0,
        0.0,
    );
    let mut remaining: Vec<&Player> = outfield.iter().collect();
    remaining.push(&keeper);

    let mut subs: Vec<MatchPlayer> = outfield
        .iter()
        .map(|p| MatchPlayer::from_player(1, p, PlayerPositionType::MidfielderCenter, false))
        .collect();
    let mut used_ids: Vec<u32> = subs.iter().map(|s| s.id).collect();

    scx.ensure_backup_goalkeeper(1, &mut subs, &mut used_ids, &remaining);

    assert!(
        subs.iter().any(|s| s.id == 9
            && s.tactical_position.current_position == PlayerPositionType::Goalkeeper),
        "the keeper is benched on the full bench"
    );
    for id in 100..last {
        assert!(
            subs.iter().any(|s| s.id == id),
            "force-selected sub {id} must not be dropped for the keeper"
        );
    }
    assert!(
        !subs.iter().any(|s| s.id == last),
        "the only non-pinned outfield sub is the one displaced"
    );
}

#[test]
fn no_backup_gk_available_keeps_existing_emergency_behavior() {
    let staff = generate_test_staff();
    let mut players = gk_outfield(8);
    players.push(make_cup_player(
        1,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::KeyPlayer,
        27,
        3,
        25,
        0.0,
    ));
    let team = cup_team(players);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &league_ctx(0.7));

    assert_eq!(starting_goalkeeper_id(&result), Some(1));
    assert_eq!(
        result.substitutes.len(),
        helpers::DEFAULT_BENCH_SIZE,
        "the bench still fills with outfielders"
    );
    assert_eq!(
        bench_goalkeeper_count(&result),
        0,
        "no keeper is invented when none is available"
    );
}

// ========== Keeper-availability fallback (competitive) ==========

#[test]
fn keeper_fallback_skips_injured_and_int_duty_keepers() {
    // Every roster keeper is unavailable (one injured, one on international
    // duty). The fallback must NOT press them back in — an outfielder takes the
    // emergency keeper slot instead, and neither unavailable keeper appears.
    let staff = generate_test_staff();
    let date = Utc::now().date_naive();
    let mut players = gk_outfield(3);
    let mut injured = make_cup_player(
        1,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::KeyPlayer,
        27,
        3,
        25,
        0.0,
    );
    injured.player_attributes.is_injured = true;
    let mut on_duty = make_cup_player(
        2,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::FirstTeamRegular,
        26,
        3,
        20,
        0.0,
    );
    on_duty.statuses.add(date, PlayerStatusType::Int);
    players.push(injured);
    players.push(on_duty);
    let team = cup_team(players);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &league_ctx(0.7));

    assert_eq!(result.main_squad.len(), 11);
    let all_ids: Vec<u32> = result
        .main_squad
        .iter()
        .chain(result.substitutes.iter())
        .map(|p| p.id)
        .collect();
    assert!(
        !all_ids.contains(&1),
        "an injured keeper must not be re-added by the fallback"
    );
    assert!(
        !all_ids.contains(&2),
        "an international-duty keeper must not be re-added by the fallback"
    );
    let gk = starting_goalkeeper_id(&result).expect("the XI still fields a keeper");
    assert!(
        gk >= 300,
        "with no available keeper an outfielder takes the gloves, got id {gk}"
    );
}

#[test]
fn keeper_fallback_admits_low_condition_keeper_over_outfielder() {
    // The only keeper is below the hard condition floor (12%). A real keeper —
    // even a tired one — still belongs in goal over an outfielder, so the
    // fallback re-admits him.
    let staff = generate_test_staff();
    let mut players = gk_outfield(3);
    let mut tired = make_cup_player(
        1,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::KeyPlayer,
        27,
        3,
        25,
        0.0,
    );
    tired.player_attributes.condition = 1200; // 12% — below HARD_CONDITION_FLOOR
    players.push(tired);
    let team = cup_team(players);

    let result = SquadSelector::select_with_context(&team, &staff, &[], &league_ctx(0.7));

    assert_eq!(
        starting_goalkeeper_id(&result),
        Some(1),
        "a low-condition real keeper starts over an outfielder"
    );
}

// ========== Rotation goalkeeper selection ==========

#[test]
fn rotation_starts_low_condition_keeper_over_outfielder() {
    // Rotation match, single keeper at 17% — below the rotation preferred
    // condition (20) but at the hard floor (15). A real keeper still starts
    // ahead of an emergency outfielder.
    let staff = generate_test_staff();
    let mut players = gk_outfield(4);
    let mut keeper = make_cup_player(
        1,
        PlayerPositionType::Goalkeeper,
        15,
        PlayerSquadStatus::FirstTeamRegular,
        24,
        5,
        5,
        0.0,
    );
    keeper.player_attributes.condition = 1700; // 17%
    players.push(keeper);
    let team = cup_team(players);

    let result = SquadSelector::select_for_rotation(&team, &staff);

    assert_eq!(
        starting_goalkeeper_id(&result),
        Some(1),
        "a 17% real keeper starts over an outfielder in rotation"
    );
}

#[test]
fn rotation_skips_unavailable_keepers_for_real_keeper() {
    // Injured / international-duty / banned keepers are never handed a rotation
    // start; the one available keeper plays.
    let staff = generate_test_staff();
    let date = Utc::now().date_naive();
    let mut players = gk_outfield(4);
    let mut injured = make_cup_player(
        1,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::KeyPlayer,
        27,
        3,
        25,
        0.0,
    );
    injured.player_attributes.is_injured = true;
    let mut on_duty = make_cup_player(
        2,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::FirstTeamRegular,
        26,
        3,
        20,
        0.0,
    );
    on_duty.statuses.add(date, PlayerStatusType::Int);
    let mut banned = make_cup_player(
        3,
        PlayerPositionType::Goalkeeper,
        16,
        PlayerSquadStatus::FirstTeamRegular,
        26,
        3,
        20,
        0.0,
    );
    banned.player_attributes.is_banned = true;
    let valid = make_cup_player(
        4,
        PlayerPositionType::Goalkeeper,
        15,
        PlayerSquadStatus::FirstTeamRegular,
        24,
        5,
        10,
        0.0,
    );
    players.push(injured);
    players.push(on_duty);
    players.push(banned);
    players.push(valid);
    let team = cup_team(players);

    let result = SquadSelector::select_for_rotation(&team, &staff);

    assert_eq!(
        starting_goalkeeper_id(&result),
        Some(4),
        "rotation fields the only available keeper, not an unavailable one or an outfielder"
    );
    let all_ids: Vec<u32> = result
        .main_squad
        .iter()
        .chain(result.substitutes.iter())
        .map(|p| p.id)
        .collect();
    assert!(!all_ids.contains(&1), "injured keeper not selected");
    assert!(!all_ids.contains(&2), "international-duty keeper not selected");
}

// ========== Bench role coverage ==========

#[test]
fn bench_role_prefers_fit_specialist_over_high_quality_misfit() {
    let staff = generate_test_staff();
    let tactics = Tactics::new(MatchTacticType::T442);
    let engine = ScoringEngine::from_staff(&staff);
    let scx = gk_unit_ctx(&staff, &tactics, &engine);

    // A high-CA forward wins the raw defensive-cover *score* (quality
    // dominates) but has zero defensive-cover fit; a lower-CA centre-back fits.
    // With a surplus of options the role must go to the fitting defender rather
    // than being skipped entirely.
    let forward = make_cup_player(
        1,
        PlayerPositionType::ForwardCenter,
        18,
        PlayerSquadStatus::FirstTeamRegular,
        25,
        5,
        10,
        100.0,
    );
    let defender = make_cup_player(
        2,
        PlayerPositionType::DefenderCenter,
        14,
        PlayerSquadStatus::FirstTeamRegular,
        25,
        5,
        10,
        100.0,
    );
    let mids: Vec<Player> = (0..6)
        .map(|i| {
            make_cup_player(
                10 + i,
                PlayerPositionType::MidfielderCenter,
                13,
                PlayerSquadStatus::FirstTeamRegular,
                25,
                5,
                10,
                100.0,
            )
        })
        .collect();

    let mut pool: Vec<&Player> = vec![&forward, &defender];
    pool.extend(mids.iter());
    assert!(
        pool.len() > helpers::DEFAULT_BENCH_SIZE,
        "the test needs a surplus to exercise the fit filter"
    );

    let subs = scx.select_substitutes(1, &pool);

    assert!(
        subs.iter().any(|s| s.id == 2),
        "the fitting centre-back must be selected for defensive cover, not skipped for the high-CA misfit"
    );
}

// ========== Prospect bench role uses the simulation date ==========

#[test]
fn prospect_bench_fit_uses_simulation_date() {
    let staff = generate_test_staff();
    let tactics = Tactics::new(MatchTacticType::T442);
    let engine = ScoringEngine::from_staff(&staff);
    // A simulation date far from any plausible wall-clock run date.
    let sim_date = NaiveDate::from_ymd_opt(2045, 1, 1).unwrap();
    let scx = super::competitive::SelectionScoringContext {
        staff: &staff,
        tactics: &tactics,
        engine: &engine,
        date: sim_date,
        is_friendly: false,
        match_importance: 0.3,
        policy: SelectionPolicy::CupRotation,
        cup: None,
        coach: None,
        competition: super::SelectionCompetition::League,
        game_model: None,
    };
    // 22 on the simulation date (born 2023) → the 0.65 prospect tier. Against
    // the wall clock he'd read as a small child (the 1.0 tier), so a wall-clock
    // age check would score differently — proving the fit uses `self.date`.
    let mut player = make_cup_player(
        1,
        PlayerPositionType::MidfielderCenter,
        14,
        PlayerSquadStatus::HotProspectForTheFuture,
        0,
        5,
        3,
        50.0,
    );
    player.birth_date = NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();
    let fit = scx.bench_role_fit(&player, super::competitive::BenchRole::Prospect);
    assert!(
        (fit - 0.65).abs() < 1e-6,
        "prospect fit should use the simulation date (22 → 0.65), got {fit}"
    );
}

// ========== Post-assignment cohesion swap ==========

/// One outfielder at every T442 slot (ids 1..=11, GK is id 1), all identical
/// `level`, so the only contest the DP can't settle on merit is at DCR — where
/// the test adds an equally-rated alternative with real back-line rapport.
fn cohesion_t442_starters() -> Vec<Player> {
    let slots = [
        (1u32, PlayerPositionType::Goalkeeper),
        (2, PlayerPositionType::DefenderLeft),
        (3, PlayerPositionType::DefenderCenterLeft),
        (4, PlayerPositionType::DefenderCenterRight),
        (5, PlayerPositionType::DefenderRight),
        (6, PlayerPositionType::MidfielderLeft),
        (7, PlayerPositionType::MidfielderCenterLeft),
        (8, PlayerPositionType::MidfielderCenterRight),
        (9, PlayerPositionType::MidfielderRight),
        (10, PlayerPositionType::ForwardLeft),
        (11, PlayerPositionType::ForwardRight),
    ];
    slots
        .iter()
        .map(|(id, pos)| {
            make_cup_player(*id, *pos, 15, PlayerSquadStatus::FirstTeamRegular, 27, 5, 15, 100.0)
        })
        .collect()
}

fn dcr_starter_id(xi: &[MatchPlayer]) -> Option<u32> {
    xi.iter()
        .find(|mp| mp.tactical_position.current_position == PlayerPositionType::DefenderCenterRight)
        .map(|mp| mp.id)
}

#[test]
fn cohesion_swap_prefers_close_candidate_with_better_unit_rapport() {
    let staff = generate_test_staff();
    let tactics = Tactics::new(MatchTacticType::T442);
    let engine = ScoringEngine::from_staff(&staff);
    let scx = gk_unit_ctx(&staff, &tactics, &engine);
    let date = Utc::now().date_naive();

    let starters = cohesion_t442_starters();
    // The challenger is an exact clone of the DCR incumbent (id 4) — identical
    // base slot score, so the swap turns purely on cohesion — but he has built
    // real rapport with the rest of the back line and the keeper.
    let mut challenger = starters[3].clone();
    challenger.id = 12;
    for mate in [1u32, 2, 3, 5] {
        challenger.relations.update(mate, 100.0, date);
    }

    let mut available: Vec<&Player> = starters.iter().collect();
    available.push(&challenger);

    let xi = scx.select_starting_eleven(1, &available);

    assert_eq!(
        dcr_starter_id(&xi),
        Some(12),
        "the equally-rated candidate with better back-line cohesion should take the DCR slot"
    );
    assert!(
        !xi.iter().any(|mp| mp.id == 4),
        "the rapport-less incumbent is the one dropped"
    );
}

#[test]
fn cohesion_swap_never_drops_force_selected_starter() {
    let staff = generate_test_staff();
    let tactics = Tactics::new(MatchTacticType::T442);
    let engine = ScoringEngine::from_staff(&staff);
    let scx = gk_unit_ctx(&staff, &tactics, &engine);
    let date = Utc::now().date_naive();

    let mut starters = cohesion_t442_starters();
    // Pin the DCR incumbent (id 4) to the XI.
    starters[3].is_force_match_selection = true;

    // The challenger has the better unit rapport and would otherwise be swapped
    // in, but the incumbent is manager-pinned and must stay.
    let mut challenger = starters[3].clone();
    challenger.id = 12;
    challenger.is_force_match_selection = false;
    for mate in [1u32, 2, 3, 5] {
        challenger.relations.update(mate, 100.0, date);
    }

    let mut available: Vec<&Player> = starters.iter().collect();
    available.push(&challenger);

    let xi = scx.select_starting_eleven(1, &available);

    assert_eq!(
        dcr_starter_id(&xi),
        Some(4),
        "a force-selected starter is never swapped out by the cohesion pass"
    );
    assert!(
        !xi.iter().any(|mp| mp.id == 12),
        "the cohesion candidate does not displace the pinned incumbent"
    );
}

// ========== MatchSelectionGameModel + new layered terms ==========

mod game_model_tests {
    use super::super::balance::LineupBalanceScorer;
    use super::super::bench_scenarios::{BenchScenario, BenchScenarioPlan, BenchScenarioScorer};
    use super::super::model::{
        CompetitionSelectionRules, EligibilityDecision, EligibilityEvaluator,
        MatchSelectionGameModel, MatchTypeClassifier, MatchTypeSignal, OpponentSelectionProfile,
        TacticalObjective,
    };
    use super::super::role_duty::{
        OpponentMatchupScorer, RoleDutyFitScorer, RoleProfileResolver, SelectionRoleProfile,
        TacticalDuty,
    };
    use super::super::{SelectionCompetition, SelectionContext};
    use super::*;
    use chrono::NaiveDate;

    fn date() -> NaiveDate {
        Utc::now().date_naive()
    }

    fn slow_cb(id: u32) -> Player {
        let mut p = make_test_player(
            id,
            &[(PlayerPositionType::DefenderCenter, 18)],
            150,
            date(),
        );
        p.skills.physical.pace = 6.0;
        p.skills.physical.acceleration = 6.0;
        p.skills.mental.positioning = 17.0;
        p.skills.technical.tackling = 17.0;
        p.skills.technical.marking = 17.0;
        p
    }

    fn fast_cb(id: u32) -> Player {
        let mut p = make_test_player(
            id,
            &[(PlayerPositionType::DefenderCenter, 16)],
            130,
            date(),
        );
        p.skills.physical.pace = 18.0;
        p.skills.physical.acceleration = 17.0;
        p.skills.mental.positioning = 14.0;
        p.skills.technical.tackling = 14.0;
        p.skills.technical.marking = 14.0;
        p
    }

    #[test]
    fn opponent_matchup_rewards_pace_against_fast_front_line() {
        let slow = slow_cb(1);
        let fast = fast_cb(2);
        let mut opponent = OpponentSelectionProfile::neutral();
        opponent.pace_threat = 0.95;

        let slow_bonus =
            OpponentMatchupScorer::score(&slow, PlayerPositionType::DefenderCenter, &opponent);
        let fast_bonus =
            OpponentMatchupScorer::score(&fast, PlayerPositionType::DefenderCenter, &opponent);

        assert!(
            fast_bonus > slow_bonus,
            "fast CB must outscore slow CB against high pace threat: fast={} slow={}",
            fast_bonus,
            slow_bonus
        );
    }

    #[test]
    fn role_profile_resolver_picks_stopper_for_defend_duty() {
        let role = RoleProfileResolver::resolve(
            PlayerPositionType::DefenderCenter,
            TacticalDuty::Defend,
        );
        assert_eq!(role, SelectionRoleProfile::StopperCentreBack);
    }

    #[test]
    fn role_profile_resolver_picks_ball_playing_for_attack_duty() {
        let role = RoleProfileResolver::resolve(
            PlayerPositionType::DefenderCenter,
            TacticalDuty::Attack,
        );
        assert_eq!(role, SelectionRoleProfile::BallPlayingCentreBack);
    }

    #[test]
    fn role_duty_fit_separates_playmaker_from_destroyer() {
        let mut playmaker = make_test_player(
            10,
            &[(PlayerPositionType::MidfielderCenter, 18)],
            150,
            date(),
        );
        playmaker.skills.mental.vision = 18.0;
        playmaker.skills.technical.passing = 18.0;
        playmaker.skills.technical.technique = 17.0;
        playmaker.skills.mental.composure = 17.0;

        let mut destroyer = make_test_player(
            11,
            &[(PlayerPositionType::MidfielderCenter, 18)],
            150,
            date(),
        );
        destroyer.skills.technical.tackling = 18.0;
        destroyer.skills.mental.aggression = 18.0;
        destroyer.skills.mental.bravery = 17.0;

        let playmaker_fit = RoleDutyFitScorer::score(
            &playmaker,
            PlayerPositionType::MidfielderCenter,
            TacticalDuty::Attack,
        );
        let destroyer_fit = RoleDutyFitScorer::score(
            &destroyer,
            PlayerPositionType::MidfielderCenter,
            TacticalDuty::Defend,
        );

        assert!(playmaker_fit > 0.45, "playmaker fits playmaker duty: {}", playmaker_fit);
        assert!(destroyer_fit > 0.45, "destroyer fits destroyer duty: {}", destroyer_fit);
    }

    #[test]
    fn eligibility_evaluator_blocks_cup_tied() {
        let player = make_test_player(99, &[(PlayerPositionType::Striker, 16)], 130, date());
        let mut rules = CompetitionSelectionRules::open();
        rules.cup_tied_player_ids.push(99);
        match EligibilityEvaluator::evaluate(&player, &rules) {
            EligibilityDecision::HardBlocked { .. } => {}
            other => panic!("expected HardBlocked for cup-tied player, got {:?}", other),
        }
    }

    #[test]
    fn eligibility_evaluator_allows_registered_player() {
        let player = make_test_player(50, &[(PlayerPositionType::Striker, 16)], 130, date());
        let mut rules = CompetitionSelectionRules::open();
        rules.registered_player_ids = Some(vec![50, 51]);
        match EligibilityEvaluator::evaluate(&player, &rules) {
            EligibilityDecision::Eligible => {}
            other => panic!("expected Eligible for registered player, got {:?}", other),
        }
    }

    #[test]
    fn eligibility_evaluator_blocks_unregistered_player() {
        let player = make_test_player(60, &[(PlayerPositionType::Striker, 16)], 130, date());
        let mut rules = CompetitionSelectionRules::open();
        rules.registered_player_ids = Some(vec![70, 71]);
        match EligibilityEvaluator::evaluate(&player, &rules) {
            EligibilityDecision::HardBlocked { .. } => {}
            other => panic!("expected HardBlocked, got {:?}", other),
        }
    }

    #[test]
    fn match_type_classifier_maps_friendly() {
        let ctx = SelectionContext {
            is_friendly: true,
            ..SelectionContext::default()
        };
        assert_eq!(MatchTypeClassifier::classify(&ctx), MatchTypeSignal::Friendly);
    }

    #[test]
    fn match_type_classifier_maps_cup_final() {
        let ctx = SelectionContext {
            competition: SelectionCompetition::DomesticCup {
                round: 5,
                total_rounds: 5,
                own_reputation: 100,
                opponent_reputation: 100,
            },
            match_importance: 1.0,
            ..SelectionContext::default()
        };
        assert_eq!(MatchTypeClassifier::classify(&ctx), MatchTypeSignal::CupFinal);
    }

    #[test]
    fn bench_scenario_plan_for_cup_final_includes_shootout() {
        let plan = BenchScenarioPlan::build(MatchTypeSignal::CupFinal, TacticalObjective::WinNowBalanced);
        let has_shootout = plan
            .weights
            .iter()
            .any(|(s, w)| matches!(s, BenchScenario::PenaltyShootout) && *w >= 0.10);
        assert!(has_shootout, "knockout final must carry shootout weight");
    }

    #[test]
    fn bench_scenario_plan_for_underdog_away_protects_lead() {
        let plan =
            BenchScenarioPlan::build(MatchTypeSignal::LeagueRoutine, TacticalObjective::UnderdogAway);
        let protect = plan
            .weights
            .iter()
            .find(|(s, _)| matches!(s, BenchScenario::ProtectLead))
            .map(|(_, w)| *w)
            .unwrap_or(0.0);
        assert!(protect >= 0.18, "underdog plan must weight ProtectLead heavily: {}", protect);
    }

    #[test]
    fn bench_scenario_coverage_rewards_aerial_target() {
        let mut p = make_test_player(40, &[(PlayerPositionType::Striker, 16)], 130, date());
        p.skills.technical.heading = 19.0;
        p.skills.physical.jumping = 18.0;
        p.skills.physical.strength = 17.0;
        let cover = BenchScenarioScorer::coverage(&p, BenchScenario::AerialPlanB, date());
        assert!(cover >= 0.8, "aerial target should cover AerialPlanB: {}", cover);
    }

    #[test]
    fn bench_scenario_coverage_recognises_youth_cameo() {
        let young = NaiveDate::from_ymd_opt(date().year() - 18, 1, 1).unwrap();
        let mut p = make_test_player(45, &[(PlayerPositionType::Striker, 14)], 120, date());
        p.birth_date = young;
        assert_eq!(
            BenchScenarioScorer::coverage(&p, BenchScenario::YouthCameo, date()),
            1.0
        );
    }

    #[test]
    fn game_model_builds_neutral_defaults_from_context() {
        let staff = generate_test_staff();
        let ctx = SelectionContext::default();
        let model = MatchSelectionGameModel::build(&ctx, &staff, 22);
        // Neutral opponent profile: strength_ratio 1.0.
        assert!((model.opponent_profile.strength_ratio - 1.0).abs() < 1e-6);
        // Squad depth tracks the available pool size.
        assert!(model.squad_state.depth >= 0.0 && model.squad_state.depth <= 1.0);
        // Coach policy values clamped to 0..1.
        let p = &model.coach_policy;
        assert!((0.0..=1.0).contains(&p.rotation_discipline));
        assert!((0.0..=1.0).contains(&p.medical_caution));
    }

    #[test]
    fn lineup_balance_score_changes_with_objective() {
        let team = generate_test_team();
        let staff = generate_test_staff();
        let result = SquadSelector::select(&team, &staff);
        let player_by_id: std::collections::HashMap<u32, &Player> =
            team.players.players().iter().map(|p| (p.id, *p)).collect();

        let security =
            LineupBalanceScorer::score(&result.main_squad, &player_by_id, TacticalObjective::ProtectLead);
        let creation =
            LineupBalanceScorer::score(&result.main_squad, &player_by_id, TacticalObjective::ChaseGame);

        // The same XI evaluated under two different objectives yields
        // different totals — the per-objective weight tables actually
        // weigh different bands.
        assert!(
            (security - creation).abs() > 0.1,
            "expected non-trivial difference between objectives: security={} creation={}",
            security,
            creation
        );
    }
}

// ========== National-team constraints ==========

mod national_callup_tests {
    use crate::country::{NationalCallupConstraints, NationalEligibilityIssue, NationalSquadStage, NationalTournamentRequirements};
    use crate::CallUpWindowType;

    #[test]
    fn national_constraints_block_cap_tied_player() {
        let mut c = NationalCallupConstraints::empty();
        c.cap_tied_elsewhere.push(42);
        assert_eq!(
            c.is_blocked(42),
            Some(NationalEligibilityIssue::CapTiedToAnotherCountry)
        );
    }

    #[test]
    fn national_constraints_block_refusal() {
        let mut c = NationalCallupConstraints::empty();
        c.refused_callup.push(7);
        assert_eq!(
            c.is_blocked(7),
            Some(NationalEligibilityIssue::DeclinedCallup)
        );
    }

    #[test]
    fn national_squad_stage_preliminary_then_final() {
        let preliminary = NationalSquadStage::target_size(CallUpWindowType::TournamentFinals, false);
        let near_deadline = NationalSquadStage::target_size(CallUpWindowType::TournamentFinals, true);
        assert!(preliminary > near_deadline);
        assert_eq!(near_deadline, NationalSquadStage::FINAL_TOURNAMENT);
    }

    #[test]
    fn tournament_window_requires_three_goalkeepers() {
        let req = NationalTournamentRequirements::for_window(CallUpWindowType::TournamentFinals);
        assert_eq!(req.min_goalkeepers, 3);
        assert!(req.min_left_flank >= 2 && req.min_right_flank >= 2);
    }
}
