use super::*;
use crate::PlayerSkills;
use crate::club::player::builder::PlayerBuilder;
use crate::r#match::MatchPlayer;
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
};
use chrono::NaiveDate;

fn build_test_player(skill_fill: f32, position: PlayerPositionType) -> MatchPlayer {
    let attrs = PlayerAttributes {
        condition: 9000,
        jadedness: 0,
        ..Default::default()
    };
    let mut skills = PlayerSkills::default();
    skills.technical.passing = skill_fill;
    skills.technical.technique = skill_fill;
    skills.technical.first_touch = skill_fill;
    skills.technical.finishing = skill_fill;
    skills.technical.long_shots = skill_fill;
    skills.technical.dribbling = skill_fill;
    skills.technical.tackling = skill_fill;
    skills.technical.marking = skill_fill;
    skills.technical.heading = skill_fill;
    skills.technical.crossing = skill_fill;
    skills.mental.vision = skill_fill;
    skills.mental.decisions = skill_fill;
    skills.mental.composure = skill_fill;
    skills.mental.concentration = skill_fill;
    skills.mental.anticipation = skill_fill;
    skills.mental.flair = skill_fill;
    skills.mental.positioning = skill_fill;
    skills.mental.off_the_ball = skill_fill;
    skills.mental.work_rate = skill_fill;
    skills.mental.aggression = skill_fill;
    skills.mental.bravery = skill_fill;
    skills.mental.teamwork = skill_fill;
    skills.mental.determination = skill_fill;
    skills.physical.balance = skill_fill;
    skills.physical.agility = skill_fill;
    skills.physical.acceleration = skill_fill;
    skills.physical.pace = skill_fill;
    skills.physical.strength = skill_fill;
    skills.physical.jumping = skill_fill;
    skills.physical.stamina = skill_fill;
    skills.physical.natural_fitness = skill_fill;
    skills.physical.match_readiness = skill_fill;
    skills.goalkeeping.reflexes = skill_fill;
    skills.goalkeeping.handling = skill_fill;
    let player = PlayerBuilder::new()
        .id(1)
        .full_name(FullName::new("T".to_string(), "P".to_string()))
        .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
        .country_id(1)
        .attributes(PersonAttributes::default())
        .skills(skills)
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position,
                level: 18,
            }],
        })
        .player_attributes(attrs)
        .build()
        .unwrap();
    MatchPlayer::from_player(1, &player, position, false, None)
}

#[test]
fn weighted_aggregator_handles_midfielder_attacking_correctly() {
    // A team of 1 forward (skill 14) + 1 attacking midfielder (skill 14)
    // should produce an attacking_quality higher than a team of just
    // the forward (since AM contributes positively at half weight),
    // but lower than 1.0 (forwards still drive most of attacking).
    // Critically, a team of 1 forward + 4 holding-mids (low attack)
    // shouldn't have its attacking_quality crushed below the forward's
    // own quality just because the AM slot got mis-counted.
    let mut acc_fwd_only = SkillAccumulator::new();
    acc_fwd_only.add(
        &build_test_player(14.0, PlayerPositionType::ForwardCenter),
        30,
    );
    let fwd_only = acc_fwd_only.finalize().attacking_quality;

    let mut acc_fwd_plus_am = SkillAccumulator::new();
    acc_fwd_plus_am.add(
        &build_test_player(14.0, PlayerPositionType::ForwardCenter),
        30,
    );
    acc_fwd_plus_am.add(
        &build_test_player(14.0, PlayerPositionType::AttackingMidfielderCenter),
        30,
    );
    let fwd_plus_am = acc_fwd_plus_am.finalize().attacking_quality;

    // With equal skill, adding an AM at weight 0.5 alongside a
    // forward at weight 1.0 should land near the forward-only value
    // (averages out the same since both contribute the same skill).
    // Crucially, it should NOT collapse to half the forward value
    // (which the buggy non-weighted denominator produced).
    assert!(
        (fwd_plus_am - fwd_only).abs() < 0.05,
        "fwd_only={fwd_only} fwd_plus_am={fwd_plus_am} — AM with same skill should preserve the level"
    );

    // Defender-only attacking should fall back to the 0.5 default
    // (no eligible attackers contributed).
    let mut acc_def_only = SkillAccumulator::new();
    acc_def_only.add(
        &build_test_player(14.0, PlayerPositionType::DefenderCenter),
        30,
    );
    assert!((acc_def_only.finalize().attacking_quality - 0.5).abs() < 1e-3);
}

#[test]
fn weighted_aggregator_high_skill_lifts_attacking_quality() {
    // Sanity: a team of an elite forward and an elite AM should
    // produce significantly higher attacking_quality than the
    // 0.5 default — confirms the weighted denominator isn't
    // squashing real signal.
    let mut acc = SkillAccumulator::new();
    acc.add(
        &build_test_player(18.0, PlayerPositionType::ForwardCenter),
        30,
    );
    acc.add(
        &build_test_player(18.0, PlayerPositionType::AttackingMidfielderCenter),
        30,
    );
    let aq = acc.finalize().attacking_quality;
    assert!(
        aq > 0.7,
        "elite attackers should produce attacking_quality > 0.7, got {aq}"
    );
    assert!(aq <= 1.0);
}

#[test]
fn weighted_aggregator_defender_dominates_defensive_quality() {
    // A team of 1 defender + 1 forward: defensive_quality should
    // be much closer to the defender's contribution (weight 1.00)
    // than the forward's (weight 0.35).
    let mut acc = SkillAccumulator::new();
    acc.add(
        &build_test_player(18.0, PlayerPositionType::DefenderCenter),
        30,
    );
    acc.add(
        &build_test_player(6.0, PlayerPositionType::ForwardCenter),
        30,
    );
    let dq = acc.finalize().defensive_quality;

    let mut acc_d_only = SkillAccumulator::new();
    acc_d_only.add(
        &build_test_player(18.0, PlayerPositionType::DefenderCenter),
        30,
    );
    let dq_solo = acc_d_only.finalize().defensive_quality;

    // The forward drags the average down a little, but should
    // not exceed a 25% reduction from the defender-only baseline
    // (forward weight is only 0.35, so its influence is bounded).
    let drop = dq_solo - dq;
    assert!(drop >= 0.0, "drop={drop} solo={dq_solo} blend={dq}");
    assert!(
        drop < 0.25 * dq_solo,
        "drop {drop} too large from solo {dq_solo}"
    );
}

#[test]
fn test_initialization() {
    let match_time = MatchTime::new();
    assert_eq!(match_time.time, 0);
}

#[test]
fn test_increment() {
    let mut match_time = MatchTime::new();

    let incremented_time = match_time.increment(10);
    assert_eq!(match_time.time, 10);
    assert_eq!(incremented_time, 10);

    let incremented_time_again = match_time.increment(5);
    assert_eq!(match_time.time, 15);
    assert_eq!(incremented_time_again, 15);
}
