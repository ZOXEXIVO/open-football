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
        positions: vec![PlayerPosition {
            position: p,
            level: 20,
        }],
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
            i,
            tagged[i],
            round_tripped[i]
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
    assert!(
        fresh > drained,
        "fresh {} should exceed drained {}",
        fresh,
        drained
    );
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

fn high_roll() -> FixedRolls {
    FixedRolls(1.0)
}

#[test]
fn young_professional_grows_more_than_low_professionalism_peer() {
    let birth = d(2008, 1, 1); // age ~17 on 2025-06-01
    let now = d(2025, 6, 1);
    let pa = 170u8;

    let mut pro = make_player(
        birth,
        PlayerPositionType::Striker,
        baseline_skills(),
        pa,
        person_pro(18.0, 16.0),
    );
    let mut sloth = make_player(
        birth,
        PlayerPositionType::Striker,
        baseline_skills(),
        pa,
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
        pro_gain,
        sloth_gain
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
        pre_pace,
        p.skills.physical.pace
    );
    // Leadership has a +3 peak offset so a 36yo is effectively 33 for it.
    assert!(
        p.skills.mental.leadership >= pre_leadership,
        "leadership should hold or grow: pre={}, post={}",
        pre_leadership,
        p.skills.mental.leadership
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
            i,
            snapshot[i],
            after[i]
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

    assert_eq!(
        p.skills.technical.finishing, pre_finishing,
        "recovering player should not gain technical"
    );
    assert_eq!(
        p.skills.physical.pace, pre_pace,
        "recovering player should not gain physical"
    );
    assert!(
        p.skills.mental.decisions >= pre_decisions,
        "recovering player can still gain mental"
    );
}

#[test]
fn fatigued_jaded_player_grows_less_than_fresh_peer() {
    let birth = d(2006, 1, 1); // ~19yo
    let now = d(2025, 6, 1);
    let pa = 170u8;

    let mut fresh = make_player(
        birth,
        PlayerPositionType::Striker,
        baseline_skills(),
        pa,
        person_pro(15.0, 14.0),
    );
    let mut drained = make_player(
        birth,
        PlayerPositionType::Striker,
        baseline_skills(),
        pa,
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
        fresh_gain,
        drained_gain
    );
}

#[test]
fn deterministic_seeded_rolls_produce_stable_output() {
    let now = d(2025, 6, 1);
    let coach = CoachingEffect::neutral();
    let mut p1 = make_player(
        d(2007, 1, 1),
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        160,
        person_pro(15.0, 12.0),
    );
    let mut p2 = make_player(
        d(2007, 1, 1),
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        160,
        person_pro(15.0, 12.0),
    );

    p1.process_development_with(now, 6000, &coach, 0.4, &mut FixedRolls(0.5));
    p2.process_development_with(now, 6000, &coach, 0.4, &mut FixedRolls(0.5));

    let a = skills_to_array(&p1);
    let b = skills_to_array(&p2);
    for i in 0..SKILL_COUNT {
        assert!(
            (a[i] - b[i]).abs() < 1e-6,
            "deterministic skill {} differed: {} vs {}",
            i,
            a[i],
            b[i]
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
        pre_handling,
        gk.skills.goalkeeping.handling
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
        pre_pace,
        out.skills.physical.pace
    );
}

// ── Maturity / overload regression suite ──────────────────────────────
//
// These tests exist because the previous tick gave the manager an
// implicit shortcut: pin a 14-year-old high-PA prospect to the senior
// XI, win the league reputation lottery, hire an elite coach, and the
// kid would touch world-class CA inside two seasons. The new tick uses
// load + maturity + soft per-week caps so that
//   * the curve under 16 is restrained on its own,
//   * exposure that overshoots the optimal monthly minute band turns
//     negative,
//   * a youth at the wrong level (overmatched in a top league) learns
//     less, not more,
//   * stacked positive multipliers can't compound past a soft cap, and
//   * the step-up bonus does not fire for under-16s and is tiny for 17s.
//
// Each test below pins one of these guarantees so a future tweak that
// re-opens the shortcut fails loudly.

fn make_player_with_load(
    birth: NaiveDate,
    pos: PlayerPositionType,
    skills: PlayerSkills,
    pa: u8,
    person: PersonAttributes,
    minutes_30: f32,
    load_30: f32,
    load_7: f32,
    condition: i16,
    jadedness: i16,
) -> Player {
    let mut p = make_player(birth, pos, skills, pa, person);
    p.player_attributes.condition = condition;
    p.player_attributes.jadedness = jadedness;
    p.load.minutes_last_30 = minutes_30;
    p.load.physical_load_30 = load_30;
    p.load.physical_load_7 = load_7;
    p
}

fn run_weekly_ticks(
    p: &mut Player,
    start: NaiveDate,
    weeks: u32,
    league_rep: u16,
    coach: &CoachingEffect,
    club_rep: f32,
) {
    for w in 0..weeks {
        let now = start + chrono::Duration::weeks(w as i64);
        p.process_development_with(now, league_rep, coach, club_rep, &mut FixedRolls(1.0));
    }
}

#[test]
fn forced_14yo_in_top_first_team_only_drifts_up_over_a_season() {
    // Pinned 14-year-old playing senior matches every week at an elite
    // club, top-five league reputation, with the best coach the system
    // can produce. Even with every multiplier on his side he must not
    // turn into a senior star inside one season.
    let start = d(2025, 7, 1);
    let birth = d(2011, 1, 1); // 14
    let pa = 190u8;

    let mut p = make_player_with_load(
        birth,
        PlayerPositionType::Striker,
        baseline_skills(),
        pa,
        person_pro(18.0, 18.0),
        // Senior schedule load: ~360 mins/30d (pinned starter), heavy
        // 1300-unit chronic load and a recent 350-unit week.
        360.0,
        1300.0,
        350.0,
        7000,
        5000,
    );

    let initial_ca = p.player_attributes.current_ability;
    let initial_finishing = p.skills.technical.finishing;
    let initial_pa = p.player_attributes.potential_ability;

    let coach = CoachingEffect::from_scores(20, 20, 20, 20, 1.0);

    // 40 weeks ≈ a full domestic season.
    run_weekly_ticks(&mut p, start, 40, 9000, &coach, 0.95);

    let ca_gain = p.player_attributes.current_ability as i32 - initial_ca as i32;
    let finishing_gain = p.skills.technical.finishing - initial_finishing;

    // Tight bounds — a forced 14yo in the senior first team must not
    // jump CA into the senior-star band over a season.
    assert!(
        ca_gain <= 15,
        "forced 14yo gained {} CA in a season — should be tightly bounded",
        ca_gain
    );
    assert!(
        finishing_gain < 1.5,
        "forced 14yo gained {} finishing in a season — should drip, not flow",
        finishing_gain
    );
    // PA must NOT be raised by routine development. The biological
    // ceiling is set at intake; a manager's selection choice can't
    // shift it.
    assert_eq!(
        p.player_attributes.potential_ability, initial_pa,
        "PA was raised during weekly development — must stay at its biological ceiling"
    );
}

#[test]
fn forced_14yo_with_extreme_overload_grows_less_than_managed_peer() {
    // Two 14-year-old prospects, identical PA, identical setup. The
    // overloaded one is being pushed past the burn-out band; the managed
    // one is on a youth-appropriate schedule. Over a season the managed
    // peer must end up with more skill growth.
    let start = d(2025, 7, 1);
    let birth = d(2011, 1, 1);
    let pa = 190u8;
    let coach = CoachingEffect::from_scores(20, 20, 20, 20, 1.0);

    // Overloaded: 1800 mins/30d, drained, jaded, deep load.
    let mut overloaded = make_player_with_load(
        birth,
        PlayerPositionType::Striker,
        baseline_skills(),
        pa,
        person_pro(18.0, 18.0),
        1800.0,
        2000.0,
        650.0,
        4500,
        8500,
    );
    overloaded.load.recovery_debt = 500.0;
    let pre_over = overloaded.skills.technical.finishing;

    // Managed: 200 mins/30d, fresh, low jadedness.
    let mut managed = make_player_with_load(
        birth,
        PlayerPositionType::Striker,
        baseline_skills(),
        pa,
        person_pro(18.0, 18.0),
        200.0,
        300.0,
        100.0,
        9500,
        1500,
    );
    let pre_man = managed.skills.technical.finishing;

    run_weekly_ticks(&mut overloaded, start, 40, 9000, &coach, 0.95);
    run_weekly_ticks(&mut managed, start, 40, 9000, &coach, 0.95);

    let overloaded_gain = overloaded.skills.technical.finishing - pre_over;
    let managed_gain = managed.skills.technical.finishing - pre_man;

    assert!(
        managed_gain > overloaded_gain,
        "managed 14yo finishing gain {} should beat overloaded peer {}",
        managed_gain,
        overloaded_gain
    );
}

#[test]
fn controlled_minutes_help_17yo_but_overload_penalises_him() {
    // 17yo benefits from controlled senior minutes; the same 17yo on
    // an extreme schedule grows less. This is the band-shape test for
    // the 16-17 age tier — `senior_exposure_multiplier` peaks inside
    // 300..900 mins/30d and falls past it.
    let start = d(2025, 7, 1);
    let birth = d(2008, 1, 1); // 17
    let pa = 175u8;
    let coach = CoachingEffect::from_scores(15, 15, 15, 15, 0.6);

    let mut controlled = make_player_with_load(
        birth,
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        pa,
        person_pro(15.0, 14.0),
        600.0, // sweet spot for 16-17
        700.0,
        180.0,
        9200,
        2000,
    );
    let pre_controlled = controlled.skills.technical.passing;

    let mut overloaded = make_player_with_load(
        birth,
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        pa,
        person_pro(15.0, 14.0),
        1700.0, // way past the 16-17 hard cap
        2400.0,
        650.0,
        4500,
        8500,
    );
    overloaded.load.recovery_debt = 600.0;
    let pre_overloaded = overloaded.skills.technical.passing;

    run_weekly_ticks(&mut controlled, start, 30, 6000, &coach, 0.6);
    run_weekly_ticks(&mut overloaded, start, 30, 6000, &coach, 0.6);

    let controlled_gain = controlled.skills.technical.passing - pre_controlled;
    let overloaded_gain = overloaded.skills.technical.passing - pre_overloaded;

    assert!(
        controlled_gain > overloaded_gain,
        "controlled 17yo passing gain {} should beat overloaded peer {}",
        controlled_gain,
        overloaded_gain
    );
}

#[test]
fn well_managed_19yo_still_develops_well() {
    // The 18-21 age tier is the main acceleration window. A 19yo with
    // appropriate minutes and a strong coach must actually move the
    // needle — the new restraint on under-16s should not silently drag
    // adult development too.
    let start = d(2025, 7, 1);
    let birth = d(2006, 1, 1); // 19
    let pa = 175u8;
    let coach = CoachingEffect::from_scores(18, 18, 18, 18, 0.6);

    let mut p = make_player_with_load(
        birth,
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        pa,
        person_pro(15.0, 14.0),
        1200.0, // sweet spot for 18-21
        1400.0,
        320.0,
        9200,
        2500,
    );
    let pre = p.skills.technical.passing;

    run_weekly_ticks(&mut p, start, 30, 7000, &coach, 0.7);

    let gain = p.skills.technical.passing - pre;
    assert!(
        gain > 0.4,
        "well-managed 19yo passing gained {} in 30 weeks — should be a real bump",
        gain
    );
}

#[test]
fn step_up_bonus_does_not_fire_for_under_16s_and_is_tiny_for_17s() {
    // Two pairs (15yo and 17yo). Each pair is identical except that the
    // transferee is inside the settlement window after a move to a much
    // bigger club. For under-16s the step_up_age_factor is 0 — the
    // bonus must NOT leak into growth. For 17yo the factor is 0.20 and
    // the raw multiplier ceilings at 1.25, so the maximum amplification
    // a 17yo can absorb is ~1.05 — small but non-zero.
    let start = d(2025, 7, 1);
    let coach = CoachingEffect::neutral();

    fn pair(birth: NaiveDate, pa: u8, start: NaiveDate, coach: &CoachingEffect) -> (f32, f32) {
        let mut control = make_player_with_load(
            birth,
            PlayerPositionType::MidfielderCenter,
            baseline_skills(),
            pa,
            person_pro(15.0, 14.0),
            200.0,
            250.0,
            80.0,
            9500,
            1500,
        );
        let mut transferee = make_player_with_load(
            birth,
            PlayerPositionType::MidfielderCenter,
            baseline_skills(),
            pa,
            person_pro(15.0, 14.0),
            200.0,
            250.0,
            80.0,
            9500,
            1500,
        );
        transferee.player_attributes.world_reputation = 1000;
        transferee.last_transfer_date = Some(start);

        let pre_control = control.skills.technical.passing;
        let pre_transferee = transferee.skills.technical.passing;

        // High-rep destination club so the step-up math wants to fire.
        run_weekly_ticks(&mut control, start, 8, 9000, coach, 0.95);
        run_weekly_ticks(&mut transferee, start, 8, 9000, coach, 0.95);

        (
            control.skills.technical.passing - pre_control,
            transferee.skills.technical.passing - pre_transferee,
        )
    }

    // 15yo: bonus completely stripped — gains must be equal within float
    // noise.
    let (c15, t15) = pair(d(2010, 1, 1), 180, start, &coach);
    assert!(
        (c15 - t15).abs() < 0.01,
        "step-up bonus leaked to a 15yo: control={} transferee={}",
        c15,
        t15
    );

    // 17yo: bonus is dampened to 20% of raw. Raw multiplier ceilings at
    // 1.25, so the most a 17yo can ever pick up is ~1.05× — a tiny
    // bump, never the senior 1.25× shortcut. Verify the transferee
    // gains *no more than* 1.05 of the control's gain.
    let (c17, t17) = pair(d(2008, 1, 1), 180, start, &coach);
    assert!(
        t17 >= c17 - 0.001,
        "17yo transferee should not lose ground: control={} transferee={}",
        c17,
        t17
    );
    let max_ratio = 1.05;
    assert!(
        t17 <= c17 * max_ratio + 0.005,
        "17yo step-up bonus too big: control={} transferee={} (cap {}×)",
        c17,
        t17,
        max_ratio
    );
}

#[test]
fn pa_does_not_increase_during_normal_development() {
    // Run a high-PA young player through a season's worth of ticks at
    // every plausible setup. The biological ceiling (PA) must not move.
    let start = d(2025, 7, 1);
    let birth = d(2008, 1, 1); // 17
    let initial_pa = 180u8;
    let coach = CoachingEffect::from_scores(20, 20, 20, 20, 1.0);

    let mut p = make_player_with_load(
        birth,
        PlayerPositionType::Striker,
        baseline_skills(),
        initial_pa,
        person_pro(18.0, 18.0),
        700.0,
        900.0,
        220.0,
        9500,
        2000,
    );

    run_weekly_ticks(&mut p, start, 50, 8000, &coach, 0.85);

    assert_eq!(
        p.player_attributes.potential_ability, initial_pa,
        "PA drifted from {} to {} — weekly development must not raise the ceiling",
        initial_pa, p.player_attributes.potential_ability
    );
    // CA must remain ≤ PA after the tick clamps it.
    assert!(
        p.player_attributes.current_ability <= p.player_attributes.potential_ability,
        "CA {} exceeded PA {} after development",
        p.player_attributes.current_ability,
        p.player_attributes.potential_ability
    );
}

#[test]
fn under_16_physical_growth_capped_far_below_adult_peer() {
    // A 14yo and a 19yo, both starting at the same physical baseline,
    // both elite-PA, both well-managed. The 14yo's pace must end up
    // gaining a small fraction of what the 19yo gains.
    let start = d(2025, 7, 1);
    let pa = 185u8;
    let coach = CoachingEffect::from_scores(20, 20, 20, 20, 1.0);

    let mut young = make_player_with_load(
        d(2011, 1, 1), // 14
        PlayerPositionType::ForwardLeft,
        baseline_skills(),
        pa,
        person_pro(15.0, 14.0),
        200.0,
        250.0,
        80.0,
        9500,
        1500,
    );
    let pre_young = young.skills.physical.pace;

    let mut prime = make_player_with_load(
        d(2006, 1, 1), // 19
        PlayerPositionType::ForwardLeft,
        baseline_skills(),
        pa,
        person_pro(15.0, 14.0),
        1200.0,
        1400.0,
        300.0,
        9500,
        2000,
    );
    let pre_prime = prime.skills.physical.pace;

    run_weekly_ticks(&mut young, start, 30, 6000, &coach, 0.6);
    run_weekly_ticks(&mut prime, start, 30, 6000, &coach, 0.6);

    let young_gain = young.skills.physical.pace - pre_young;
    let prime_gain = prime.skills.physical.pace - pre_prime;

    assert!(
        young_gain * 3.0 < prime_gain,
        "14yo pace gain {} too close to 19yo {} — physical maturity gate not biting",
        young_gain,
        prime_gain
    );
}

// ── Full-pipeline regression ──────────────────────────────────────────
//
// The handcrafted-load tests above pin the development tick in
// isolation: they set `load.minutes_last_30`, `physical_load_30`, and
// `physical_load_7` directly. That keeps the unit tests fast and
// surgical, but it doesn't exercise the path where `on_match_exertion`
// feeds those exact fields. The tests below run a 14yo through a
// season of 90-minute starts via `on_match_exertion`, with daily decay
// in between, and call `process_development_with` weekly. That's the
// real pipeline a force-selected kid would travel through, and it
// catches integration-level mistuning that unit tests can't see.

fn simulate_weekly(
    p: &mut Player,
    start: NaiveDate,
    weeks: u32,
    minutes_per_week: f32,
    league_rep: u16,
    coach: &CoachingEffect,
    club_rep: f32,
) {
    for w in 0..weeks {
        let match_day = start + chrono::Duration::weeks(w as i64);
        // Age the rolling windows day-by-day across the week leading
        // into the match. The first iteration seeds the decay clock.
        for offset in 0..7 {
            let day = match_day - chrono::Duration::days(6 - offset);
            p.load.daily_decay(day);
        }
        if minutes_per_week > 0.0 {
            p.on_match_exertion(minutes_per_week, match_day, false);
        }
        p.process_development_with(match_day, league_rep, coach, club_rep, &mut FixedRolls(1.0));
    }
}

#[test]
fn forced_14yo_full_pipeline_load_climbs_and_skill_gain_stays_bounded() {
    // Drive the 14yo through 16 weeks of 90-min senior starts via
    // `on_match_exertion`. The rolling windows must climb (load,
    // jadedness, recovery debt) and the per-skill gain must stay
    // bounded — the senior-exposure penalty + youth maturity gate +
    // weekly cap together must prevent a manager-pinned kid from
    // becoming a senior star inside a season.
    let start = d(2025, 7, 1);
    let birth = d(2011, 1, 1); // 14
    let pa = 190u8;
    let coach = CoachingEffect::from_scores(20, 20, 20, 20, 1.0);

    let mut p = make_player(
        birth,
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        pa,
        person_pro(18.0, 18.0),
    );
    let initial_passing = p.skills.technical.passing;
    let initial_jad = p.player_attributes.jadedness;

    simulate_weekly(&mut p, start, 16, 90.0, 9000, &coach, 0.95);

    // Load and fatigue accumulated through the live pipeline.
    assert!(
        p.load.minutes_last_30 > 200.0,
        "minutes window did not accumulate: {}",
        p.load.minutes_last_30
    );
    assert!(
        p.load.physical_load_30 > 200.0,
        "physical load did not accumulate: {}",
        p.load.physical_load_30
    );
    assert!(
        p.player_attributes.jadedness as i32 > initial_jad as i32,
        "jadedness did not rise: pre={} post={}",
        initial_jad,
        p.player_attributes.jadedness
    );

    // Skill gains stay bounded — the youth-pipeline guards bite even
    // with full senior minutes flowing through `on_match_exertion`.
    let passing_gain = p.skills.technical.passing - initial_passing;
    assert!(
        passing_gain < 1.0,
        "forced 14yo gained {} passing through the live pipeline — should drip, not flow",
        passing_gain
    );
}

#[test]
fn managed_cameo_pipeline_outgrows_overloaded_full_start_pipeline() {
    // Same starting 14yo prospect, two pipelines: one gets a controlled
    // 30-minute cameo every other week, the other gets pinned to 90
    // minutes every week. Run both through the live `on_match_exertion`
    // + decay + development tick stack — the managed peer must end up
    // with more skill growth despite playing fewer minutes.
    let start = d(2025, 7, 1);
    let birth = d(2011, 1, 1); // 14
    let pa = 190u8;
    let coach = CoachingEffect::from_scores(20, 20, 20, 20, 1.0);

    let mut managed = make_player(
        birth,
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        pa,
        person_pro(18.0, 18.0),
    );
    let mut overloaded = make_player(
        birth,
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        pa,
        person_pro(18.0, 18.0),
    );

    let pre_managed = managed.skills.technical.passing;
    let pre_over = overloaded.skills.technical.passing;

    // Managed: alternating cameo / rest weeks. The pattern lives in a
    // closure so simulate_weekly's straight-line cadence isn't bent
    // out of shape just for this test.
    let weeks = 24u32;
    for w in 0..weeks {
        let match_day = start + chrono::Duration::weeks(w as i64);
        for offset in 0..7 {
            let day = match_day - chrono::Duration::days(6 - offset);
            managed.load.daily_decay(day);
            overloaded.load.daily_decay(day);
        }
        if w % 2 == 0 {
            managed.on_match_exertion(30.0, match_day, false);
        }
        overloaded.on_match_exertion(90.0, match_day, false);
        managed.process_development_with(match_day, 9000, &coach, 0.95, &mut FixedRolls(1.0));
        overloaded.process_development_with(match_day, 9000, &coach, 0.95, &mut FixedRolls(1.0));
    }

    let managed_gain = managed.skills.technical.passing - pre_managed;
    let over_gain = overloaded.skills.technical.passing - pre_over;
    assert!(
        managed_gain > over_gain,
        "managed cameo gain {} should beat overloaded pinned gain {}",
        managed_gain,
        over_gain
    );
}

// ── Initial CA > PA invariant ─────────────────────────────────────────
//
// PA is the biological ceiling: a single development tick (or any
// number of them) must never raise it. In particular, a player who
// arrives with CA above PA — the kind of state a legacy save or a
// hand-rolled fixture might produce — gets CA clamped down, not PA
// pulled up to match. The generators in
// `core::club::player::generators::generator` and
// `database::generators::player` already normalise via
// `pa.max(ca)` at intake; the test below pins the *runtime* clamp on
// the development tick so the invariant survives even when the input
// is broken.

#[test]
fn initial_ca_above_pa_clamps_ca_down_and_does_not_raise_pa() {
    let now = d(2025, 6, 1);
    let mut p = make_player(
        d(2007, 1, 1), // 18
        PlayerPositionType::Striker,
        baseline_skills(),
        140,
        person_pro(15.0, 12.0),
    );
    // Force the bad input. Generators prevent this at intake — we are
    // exercising the development tick's runtime clamp, not the
    // generation path.
    p.player_attributes.potential_ability = 140;
    p.player_attributes.current_ability = 160;

    let coach = CoachingEffect::neutral();
    p.process_development_with(now, 6000, &coach, 0.5, &mut FixedRolls(0.5));

    assert_eq!(
        p.player_attributes.potential_ability, 140,
        "PA was raised to swallow an initial CA > PA — must remain the biological ceiling",
    );
    assert!(
        p.player_attributes.current_ability <= p.player_attributes.potential_ability,
        "CA {} must be clamped down to PA {} after the tick",
        p.player_attributes.current_ability,
        p.player_attributes.potential_ability,
    );
}

#[test]
fn coaching_effect_amplifies_growth() {
    let now = d(2025, 6, 1);
    let mut weak = make_player(
        d(2007, 1, 1),
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        160,
        person_pro(15.0, 12.0),
    );
    let mut strong = make_player(
        d(2007, 1, 1),
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        160,
        person_pro(15.0, 12.0),
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
        strong_gain,
        weak_gain
    );
}
