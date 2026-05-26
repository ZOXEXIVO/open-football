use super::*;
use crate::PlayerSkills;
use crate::club::player::builder::PlayerBuilder;
use crate::r#match::MatchCoach;
use crate::r#match::MatchPlayer;
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
};
use chrono::NaiveDate;

fn build_test_player(skill_fill: f32, position: PlayerPositionType) -> MatchPlayer {
    let mut attrs = PlayerAttributes::default();
    attrs.condition = 9000;
    attrs.jadedness = 0;
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
    MatchPlayer::from_player(1, &player, position, false)
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

fn make_input(
    xg_for: f32,
    xg_against: f32,
    shots: u32,
    pressures: u32,
    succ: u32,
    deep: u32,
    turnovers: u32,
) -> RollingMetricsInput {
    RollingMetricsInput {
        cum_xg_for: xg_for,
        cum_xg_against: xg_against,
        cum_shots_for: shots,
        cum_pressures: pressures,
        cum_successful_pressures: succ,
        cum_deep_entries: deep,
        cum_dangerous_turnovers: turnovers,
    }
}

#[test]
fn rolling_metrics_first_call_diffs_from_zero() {
    // First evaluate_coaches pass: snapshot tick is 0 (default),
    // current_tick is well below the 90 000 window. The window is
    // not rotated, so the snapshot stays at zero and the deltas
    // equal the absolute current totals.
    let mut coach = MatchCoach::new();
    coach.cum_possession_ticks = 600; // 6 sim s
    coach.cum_field_tilt_ticks = 300; // 3 sim s
    let m = FootballEngine::<840, 545>::build_rolling_metrics(
        &mut coach,
        1_000,
        &make_input(0.5, 0.2, 4, 30, 12, 7, 1),
    );
    assert!((m.xg_for_last_15 - 0.5).abs() < 1e-4);
    assert!((m.xg_against_last_15 - 0.2).abs() < 1e-4);
    assert_eq!(m.shots_for_last_15, 4);
    assert_eq!(m.deep_entries_for_last_15, 7);
    assert_eq!(m.dangerous_turnovers_last_10, 1);
    // 30 pressures, 12 successful → 0.4
    assert!((m.press_success_rate_last_10 - 0.40).abs() < 1e-4);
    // possession 600 / window 1000 = 0.6 (window clamped to elapsed)
    assert!((m.possession_last_10 - 0.6).abs() < 1e-4);
    // Snapshot must NOT have rotated yet (elapsed << 90 000).
    assert_eq!(coach.metric_snapshot.tick, 0);
}

#[test]
fn rolling_metrics_window_rotates_at_15_minutes() {
    // After 15 sim minutes (≈ 90 000 ticks) the snapshot rotates
    // forward; subsequent deltas are computed from the new
    // baseline, not the start of the match.
    let mut coach = MatchCoach::new();
    // Pretend we already had 60 sim s of possession before the rotation.
    coach.cum_possession_ticks = 6_000;
    coach.cum_field_tilt_ticks = 0;

    // First pass at exactly the window boundary — rotates.
    let _ = FootballEngine::<840, 545>::build_rolling_metrics(
        &mut coach,
        90_000,
        &make_input(1.5, 0.6, 12, 80, 30, 18, 3),
    );
    assert_eq!(coach.metric_snapshot.tick, 90_000);
    assert!((coach.metric_snapshot.xg_for - 1.5).abs() < 1e-4);
    assert_eq!(coach.metric_snapshot.shots_for, 12);
    assert_eq!(coach.metric_snapshot.deep_entries_for, 18);
    assert_eq!(coach.metric_snapshot.dangerous_turnovers, 3);

    // Second pass shortly after rotation: deltas are vs the new
    // baseline (1.5 xg, 12 shots, 18 deep, 3 turnovers).
    let m = FootballEngine::<840, 545>::build_rolling_metrics(
        &mut coach,
        95_000,
        &make_input(1.7, 0.6, 13, 82, 31, 19, 3),
    );
    assert!((m.xg_for_last_15 - 0.2).abs() < 1e-4);
    assert_eq!(m.shots_for_last_15, 1);
    assert_eq!(m.deep_entries_for_last_15, 1);
    assert_eq!(m.dangerous_turnovers_last_10, 0);
}

#[test]
fn rolling_metrics_zero_pressures_returns_neutral_press_rate() {
    // Press rate is undefined when no pressures occurred; we pin
    // it to 0.5 so the smart coach evaluator's "failing press"
    // branch doesn't fire spuriously.
    let mut coach = MatchCoach::new();
    let m = FootballEngine::<840, 545>::build_rolling_metrics(
        &mut coach,
        500,
        &make_input(0.0, 0.0, 0, 0, 0, 0, 0),
    );
    assert!((m.press_success_rate_last_10 - 0.5).abs() < 1e-4);
}
