use super::coaching::CoachingEffect;
use super::modifiers::*;
use super::position_weights::*;
use super::rolls::FixedRolls;
use super::skills_array::*;

use crate::club::player::builder::PlayerBuilder;
use crate::club::player::player::Player;
use crate::club::player::position::{PlayerPosition, PlayerPositions};
use crate::shared::fullname::FullName;
use crate::{PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills};
use chrono::NaiveDate;

// ── Test helpers ──────────────────────────────────────────────────────

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn person_pro(prof: f32, ambition: f32) -> PersonAttributes {
    PersonAttributes {
        professionalism: prof,
        ambition,
        ..PersonAttributes::default()
    }
}

fn baseline_skills() -> PlayerSkills {
    // Mid-range outfield baseline — meaningfully below the per-skill
    // ceiling of a high-PA player so growth has room to happen.
    let mut s = PlayerSkills::default();
    s.technical.passing = 10.0;
    s.technical.first_touch = 10.0;
    s.technical.dribbling = 10.0;
    s.technical.finishing = 10.0;
    s.technical.tackling = 10.0;
    s.technical.marking = 10.0;
    s.technical.heading = 10.0;
    s.technical.technique = 10.0;
    s.technical.crossing = 10.0;
    s.technical.long_shots = 10.0;
    s.technical.long_throws = 10.0;
    s.technical.corners = 10.0;
    s.technical.free_kicks = 10.0;
    s.technical.penalty_taking = 10.0;

    s.mental.work_rate = 14.0;
    s.mental.determination = 14.0;
    s.mental.composure = 10.0;
    s.mental.decisions = 10.0;
    s.mental.positioning = 10.0;
    s.mental.anticipation = 10.0;
    s.mental.vision = 10.0;
    s.mental.teamwork = 10.0;
    s.mental.concentration = 10.0;
    s.mental.bravery = 10.0;
    s.mental.aggression = 10.0;
    s.mental.flair = 10.0;
    s.mental.leadership = 10.0;
    s.mental.off_the_ball = 10.0;

    s.physical.pace = 12.0;
    s.physical.acceleration = 12.0;
    s.physical.agility = 12.0;
    s.physical.balance = 12.0;
    s.physical.stamina = 12.0;
    s.physical.strength = 12.0;
    s.physical.jumping = 12.0;
    s.physical.natural_fitness = 14.0;
    s.physical.match_readiness = 15.0;

    s
}

fn gk_skills() -> PlayerSkills {
    let mut s = baseline_skills();
    // Give GK a meaningful goalkeeping baseline.
    s.goalkeeping.handling = 10.0;
    s.goalkeeping.reflexes = 10.0;
    s.goalkeeping.aerial_reach = 10.0;
    s.goalkeeping.one_on_ones = 10.0;
    s.goalkeeping.command_of_area = 10.0;
    s.goalkeeping.communication = 10.0;
    s.goalkeeping.kicking = 10.0;
    s.goalkeeping.first_touch = 10.0;
    s.goalkeeping.passing = 10.0;
    s.goalkeeping.throwing = 10.0;
    s.goalkeeping.punching = 10.0;
    s.goalkeeping.rushing_out = 10.0;
    s.goalkeeping.eccentricity = 10.0;
    s
}

fn positions(p: PlayerPositionType) -> PlayerPositions {
    PlayerPositions {
        positions: vec![PlayerPosition { position: p, level: 20 }],
    }
}

fn make_player(
    birth: NaiveDate,
    pos: PlayerPositionType,
    skills: PlayerSkills,
    pa: u8,
    person: PersonAttributes,
) -> Player {
    let mut attrs = PlayerAttributes::default();
    attrs.potential_ability = pa;
    // Start CA below PA so per-skill growth has room.
    attrs.current_ability = (pa as f32 * 0.5) as u8;
    attrs.condition = 9500;
    attrs.jadedness = 1000;
    attrs.injury_proneness = 5;

    PlayerBuilder::new()
        .id(1)
        .full_name(FullName::new("Test".to_string(), "Player".to_string()))
        .birth_date(birth)
        .country_id(1)
        .attributes(person)
        .skills(skills)
        .positions(positions(pos))
        .player_attributes(attrs)
        .build()
        .unwrap()
}

// ── Round-trip ────────────────────────────────────────────────────

#[test]
fn skill_array_round_trip_preserves_all_fields() {
    let mut p = make_player(
        d(2000, 1, 1),
        PlayerPositionType::Striker,
        baseline_skills(),
        150,
        PersonAttributes::default(),
    );
    // Stamp every skill with a unique value so a missing field shows up
    // as a stale default rather than a coincidence.
    let mut tagged = [0.0f32; SKILL_COUNT];
    for i in 0..SKILL_COUNT {
        tagged[i] = 1.0 + (i as f32) * 0.1; // 1.0, 1.1, 1.2, ... 5.9
    }
    write_skills_back(&mut p, &tagged);

    let round_tripped = skills_to_array(&p);
    for i in 0..SKILL_COUNT {
        assert!(
            (round_tripped[i] - tagged[i]).abs() < 1e-6,
            "skill index {} did not round-trip: wrote {}, read {}",
            i, tagged[i], round_tripped[i]
        );
    }
}

#[test]
fn skill_count_matches_enum() {
    assert_eq!(SkillKey::GkThrowing as usize + 1, SKILL_COUNT);
}

// ── Pure helper checks ────────────────────────────────────────────

#[test]
fn workload_growth_modifier_drops_with_fatigue() {
    let fresh = workload_growth_modifier(100, 0);
    let drained = workload_growth_modifier(35, 8000);
    assert!(fresh > drained, "fresh {} should exceed drained {}", fresh, drained);
    assert!(drained <= 0.7);
    assert!(fresh >= 0.99);
}

#[test]
fn match_readiness_multiplier_scales_inside_band() {
    assert!(match_readiness_multiplier(0.0) < match_readiness_multiplier(20.0));
    assert!((match_readiness_multiplier(20.0) - 1.10).abs() < 1e-4);
    assert!((match_readiness_multiplier(0.0) - 0.90).abs() < 1e-4);
}

#[test]
fn skill_gap_factor_is_zero_at_or_above_ceiling() {
    assert_eq!(skill_gap_factor(20.0, 15.0), 0.05);
    assert_eq!(skill_gap_factor(15.0, 15.0), 0.05);
    assert!(skill_gap_factor(5.0, 15.0) > 0.5);
}

#[test]
fn defensive_midfielder_uses_midfielder_dev_weights() {
    // This pins the deliberate divergence from
    // PlayerPositionType::position_group(): for development the DM is
    // a midfielder, not a defender.
    assert_eq!(
        pos_group_from(PlayerPositionType::DefensiveMidfielder),
        PosGroup::Midfielder
    );
}

// ── Position-specific ceilings ────────────────────────────────────

#[test]
fn striker_finishing_ceiling_exceeds_tackling_ceiling() {
    // Verify the position weights produce the expected ceiling shape.
    let w = position_dev_weights(PosGroup::Forward);
    assert!(w[SK_FINISHING] > w[SK_TACKLING]);
    assert!(w[SK_FINISHING] >= 1.4);
    assert!(w[SK_TACKLING] <= 0.5);
}

#[test]
fn defender_marking_grows_faster_than_finishing() {
    let w = position_dev_weights(PosGroup::Defender);
    assert!(w[SK_MARKING] > w[SK_FINISHING]);
    assert!(w[SK_TACKLING] > w[SK_FINISHING]);
}

// ── Behavioral tests using the deterministic roll seam ────────────

fn high_roll() -> FixedRolls { FixedRolls(1.0) }

#[test]
fn young_professional_grows_more_than_low_professionalism_peer() {
    let birth = d(2008, 1, 1); // age ~17 on 2025-06-01
    let now = d(2025, 6, 1);
    let pa = 170u8;

    let mut pro = make_player(
        birth, PlayerPositionType::Striker, baseline_skills(), pa,
        person_pro(18.0, 16.0),
    );
    let mut sloth = make_player(
        birth, PlayerPositionType::Striker, baseline_skills(), pa,
        person_pro(4.0, 6.0),
    );
    // Same starting CA so the gap factor is identical.
    sloth.skills.mental.work_rate = 6.0;
    sloth.skills.mental.determination = 6.0;

    let pre_pro_finishing = pro.skills.technical.finishing;
    let pre_sloth_finishing = sloth.skills.technical.finishing;

    let coach = CoachingEffect::neutral();
    pro.process_development_with(now, 5000, &coach, 0.5, &mut high_roll());
    sloth.process_development_with(now, 5000, &coach, 0.5, &mut high_roll());

    let pro_gain = pro.skills.technical.finishing - pre_pro_finishing;
    let sloth_gain = sloth.skills.technical.finishing - pre_sloth_finishing;
    assert!(
        pro_gain > sloth_gain,
        "pro_gain={}, sloth_gain={}",
        pro_gain, sloth_gain
    );
}

#[test]
fn old_player_declines_physically_but_can_still_grow_mentally() {
    // 36-year-old with neutral coaching, same fixed roll for both
    // categories. Mental should still nudge up, physical should fall.
    let now = d(2025, 6, 1);
    let birth = d(1989, 1, 1); // 36
    let mut p = make_player(
        birth,
        PlayerPositionType::DefenderCenter,
        baseline_skills(),
        150,
        person_pro(15.0, 12.0),
    );
    let pre_pace = p.skills.physical.pace;
    let pre_leadership = p.skills.mental.leadership;

    let coach = CoachingEffect::neutral();
    // Use the midpoint roll so the band is interpreted at its center —
    // physical at 36 is unambiguously negative; mental at 36 is around 0.
    p.process_development_with(now, 5000, &coach, 0.5, &mut FixedRolls(0.5));

    assert!(
        p.skills.physical.pace <= pre_pace,
        "old pace should not grow: pre={}, post={}",
        pre_pace, p.skills.physical.pace
    );
    // Leadership has a +3 peak offset so a 36yo is effectively 33 for it.
    assert!(
        p.skills.mental.leadership >= pre_leadership,
        "leadership should hold or grow: pre={}, post={}",
        pre_leadership, p.skills.mental.leadership
    );
}

#[test]
fn injured_player_skips_development_entirely() {
    let mut p = make_player(
        d(2008, 1, 1),
        PlayerPositionType::Striker,
        baseline_skills(),
        170,
        person_pro(18.0, 16.0),
    );
    p.player_attributes.is_injured = true;
    p.player_attributes.injury_days_remaining = 30;

    let snapshot = skills_to_array(&p);
    let coach = CoachingEffect::neutral();
    p.process_development_with(d(2025, 6, 1), 8000, &coach, 0.6, &mut high_roll());

    let after = skills_to_array(&p);
    for i in 0..SKILL_COUNT {
        assert!(
            (snapshot[i] - after[i]).abs() < 1e-6,
            "injured player skill {} changed: {} -> {}",
            i, snapshot[i], after[i]
        );
    }
}

#[test]
fn recovering_player_only_gains_mental() {
    let mut p = make_player(
        d(2008, 1, 1),
        PlayerPositionType::Striker,
        baseline_skills(),
        170,
        person_pro(18.0, 16.0),
    );
    p.player_attributes.recovery_days_remaining = 14;
    // is_injured is already false (Default), so this puts the player in
    // the recovery phase.

    let pre_finishing = p.skills.technical.finishing;
    let pre_pace = p.skills.physical.pace;
    let pre_decisions = p.skills.mental.decisions;

    let coach = CoachingEffect::neutral();
    p.process_development_with(d(2025, 6, 1), 8000, &coach, 0.6, &mut high_roll());

    assert_eq!(p.skills.technical.finishing, pre_finishing,
        "recovering player should not gain technical");
    assert_eq!(p.skills.physical.pace, pre_pace,
        "recovering player should not gain physical");
    assert!(p.skills.mental.decisions >= pre_decisions,
        "recovering player can still gain mental");
}

#[test]
fn fatigued_jaded_player_grows_less_than_fresh_peer() {
    let birth = d(2006, 1, 1); // ~19yo
    let now = d(2025, 6, 1);
    let pa = 170u8;

    let mut fresh = make_player(
        birth, PlayerPositionType::Striker, baseline_skills(), pa,
        person_pro(15.0, 14.0),
    );
    let mut drained = make_player(
        birth, PlayerPositionType::Striker, baseline_skills(), pa,
        person_pro(15.0, 14.0),
    );
    drained.player_attributes.condition = 3500;
    drained.player_attributes.jadedness = 8000;
    drained.skills.physical.match_readiness = 5.0;

    let pre_fresh = fresh.skills.technical.finishing;
    let pre_drained = drained.skills.technical.finishing;

    let coach = CoachingEffect::neutral();
    fresh.process_development_with(now, 5000, &coach, 0.5, &mut high_roll());
    drained.process_development_with(now, 5000, &coach, 0.5, &mut high_roll());

    let fresh_gain = fresh.skills.technical.finishing - pre_fresh;
    let drained_gain = drained.skills.technical.finishing - pre_drained;
    assert!(
        fresh_gain > drained_gain,
        "fresh {} should grow more than drained {}",
        fresh_gain, drained_gain
    );
}

#[test]
fn deterministic_seeded_rolls_produce_stable_output() {
    let now = d(2025, 6, 1);
    let coach = CoachingEffect::neutral();
    let mut p1 = make_player(
        d(2007, 1, 1), PlayerPositionType::MidfielderCenter,
        baseline_skills(), 160, person_pro(15.0, 12.0),
    );
    let mut p2 = make_player(
        d(2007, 1, 1), PlayerPositionType::MidfielderCenter,
        baseline_skills(), 160, person_pro(15.0, 12.0),
    );

    p1.process_development_with(now, 6000, &coach, 0.4, &mut FixedRolls(0.5));
    p2.process_development_with(now, 6000, &coach, 0.4, &mut FixedRolls(0.5));

    let a = skills_to_array(&p1);
    let b = skills_to_array(&p2);
    for i in 0..SKILL_COUNT {
        assert!(
            (a[i] - b[i]).abs() < 1e-6,
            "deterministic skill {} differed: {} vs {}",
            i, a[i], b[i]
        );
    }
}

#[test]
fn goalkeeping_skills_use_later_peak_curve() {
    // At age 30, a GK should still gain on goalkeeping skills, while
    // an outfield 30yo's physical skills are flat or declining.
    let coach = CoachingEffect::neutral();
    let now = d(2025, 6, 1);

    let mut gk = make_player(
        d(1995, 1, 1), // 30
        PlayerPositionType::Goalkeeper,
        gk_skills(),
        160,
        person_pro(15.0, 12.0),
    );
    let pre_handling = gk.skills.goalkeeping.handling;
    gk.process_development_with(now, 6000, &coach, 0.4, &mut FixedRolls(0.7));
    assert!(
        gk.skills.goalkeeping.handling > pre_handling,
        "30yo GK handling should still grow: pre={} post={}",
        pre_handling, gk.skills.goalkeeping.handling
    );

    let mut out = make_player(
        d(1995, 1, 1), // 30
        PlayerPositionType::Striker,
        baseline_skills(),
        160,
        person_pro(15.0, 12.0),
    );
    let pre_pace = out.skills.physical.pace;
    out.process_development_with(now, 6000, &coach, 0.4, &mut FixedRolls(0.5));
    assert!(
        out.skills.physical.pace <= pre_pace,
        "30yo outfield pace should not grow: pre={} post={}",
        pre_pace, out.skills.physical.pace
    );
}

#[test]
fn coaching_effect_amplifies_growth() {
    let now = d(2025, 6, 1);
    let mut weak = make_player(
        d(2007, 1, 1), PlayerPositionType::MidfielderCenter,
        baseline_skills(), 160, person_pro(15.0, 12.0),
    );
    let mut strong = make_player(
        d(2007, 1, 1), PlayerPositionType::MidfielderCenter,
        baseline_skills(), 160, person_pro(15.0, 12.0),
    );

    let no_coach = CoachingEffect::neutral();
    let elite = CoachingEffect::from_scores(20, 20, 20, 20, 1.0);

    let pre_weak = weak.skills.technical.passing;
    let pre_strong = strong.skills.technical.passing;

    weak.process_development_with(now, 6000, &no_coach, 0.4, &mut high_roll());
    strong.process_development_with(now, 6000, &elite, 0.4, &mut high_roll());

    let weak_gain = weak.skills.technical.passing - pre_weak;
    let strong_gain = strong.skills.technical.passing - pre_strong;
    assert!(
        strong_gain > weak_gain,
        "elite coach gain {} should exceed neutral coach gain {}",
        strong_gain, weak_gain
    );
}
