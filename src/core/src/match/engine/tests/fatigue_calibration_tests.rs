//! Calibration tests for the quality-vs-condition relationship.
//!
//! These do not spin up a full match — that lives in `dev_match` and
//! needs the squad/data pipeline. Instead they exercise the pure
//! composite + profile helpers that resolve every event in the engine.
//! If these relationships hold, the match-level statistics will follow.
//!
//! Scenarios covered (from the match-engine fatigue prompt):
//!   1. Equal quality, equal condition → composites match within noise.
//!   2. Strong fresh vs weak fresh → strong dominates every composite.
//!   3. Strong tired vs weak fresh → freshness closes the gap (esp.
//!      explosive) but does not erase a large skill gap on technical
//!      composites or one-on-one finishing.
//!   4. Elite high-stamina tired vs average tired → elite degrades less.
//!   5. Low-quality fresh vs high-quality tired → gap narrows in
//!      explosive, big gap holds in technical / mental.
//!
//! Each test uses relative assertions (composite A > composite B by
//! ≥ delta) — never absolute output values that would lock the
//! engine into a brittle calibration.

#![cfg(test)]

use crate::PlayerSkills;
use crate::club::player::builder::PlayerBuilder;
use crate::r#match::MatchPlayer;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext, effective_skill,
};
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::{
    GoalkeeperSkillInputs, GoalkeeperSkillProfile,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
};
use chrono::NaiveDate;

/// Build a uniform-skill player. Same shape used by the composite
/// tests so each composite reads a single skill band — keeps the
/// calibration tests focused on the band → composite relationship.
fn build_player(fill: f32, condition: i16, position: PlayerPositionType) -> MatchPlayer {
    let mut attrs = PlayerAttributes::default();
    attrs.condition = condition;
    attrs.jadedness = 0;
    let mut skills = PlayerSkills::default();
    skills.technical.passing = fill;
    skills.technical.technique = fill;
    skills.technical.first_touch = fill;
    skills.technical.finishing = fill;
    skills.technical.long_shots = fill;
    skills.technical.dribbling = fill;
    skills.technical.tackling = fill;
    skills.technical.marking = fill;
    skills.technical.heading = fill;
    skills.technical.crossing = fill;
    skills.technical.corners = fill;
    skills.technical.free_kicks = fill;
    skills.technical.long_throws = fill;
    skills.technical.penalty_taking = fill;
    skills.mental.vision = fill;
    skills.mental.decisions = fill;
    skills.mental.composure = fill;
    skills.mental.concentration = fill;
    skills.mental.anticipation = fill;
    skills.mental.flair = fill;
    skills.mental.positioning = fill;
    skills.mental.off_the_ball = fill;
    skills.mental.work_rate = fill;
    skills.mental.aggression = fill;
    skills.mental.bravery = fill;
    skills.mental.teamwork = fill;
    skills.mental.determination = fill;
    skills.mental.leadership = fill;
    skills.physical.balance = fill;
    skills.physical.agility = fill;
    skills.physical.acceleration = fill;
    skills.physical.pace = fill;
    skills.physical.strength = fill;
    skills.physical.jumping = fill;
    skills.physical.stamina = fill;
    skills.physical.natural_fitness = fill;
    skills.physical.match_readiness = fill;
    skills.goalkeeping.reflexes = fill;
    skills.goalkeeping.handling = fill;
    skills.goalkeeping.one_on_ones = fill;
    skills.goalkeeping.aerial_reach = fill;
    skills.goalkeeping.command_of_area = fill;
    skills.goalkeeping.communication = fill;
    skills.goalkeeping.kicking = fill;
    skills.goalkeeping.throwing = fill;
    skills.goalkeeping.passing = fill;
    skills.goalkeeping.first_touch = fill;
    skills.goalkeeping.rushing_out = fill;
    skills.goalkeeping.punching = fill;
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

/// Build an elite-stamina variant: pump stamina/NF up while keeping
/// every other skill at `fill`. Used to isolate the mitigation lever.
fn build_elite_stamina(fill: f32, condition: i16) -> MatchPlayer {
    let mut p = build_player(fill, condition, PlayerPositionType::MidfielderCenter);
    p.skills.physical.stamina = 19.0;
    p.skills.physical.natural_fitness = 18.0;
    p
}

/// All-composite snapshot for a given player at a given minute. Drops
/// the engine's per-tick `MatchPlayer` into a stable shape for relative
/// comparisons across the five scenario rungs.
struct CompositeSnapshot {
    passing: f32,
    long_passing: f32,
    first_touch: f32,
    shooting_close: f32,
    shooting_medium: f32,
    long_shot: f32,
    dribble_attack: f32,
    defensive_duel: f32,
    interception: f32,
    pressing: f32,
    aerial_def: f32,
    off_ball: f32,
    shot_selection: f32,
    pass_selection: f32,
    defensive_positioning: f32,
    mobility: f32,
    decision_quality: f32,
    loose_ball: f32,
    tackle_timing: f32,
}

impl CompositeSnapshot {
    fn capture(p: &MatchPlayer, minute: u32) -> Self {
        Self {
            passing: sc::passing_execution(p, minute),
            long_passing: sc::long_passing(p, minute),
            first_touch: sc::receiving_first_touch(p, minute),
            shooting_close: sc::shooting_close(p, minute),
            shooting_medium: sc::shooting_medium(p, minute),
            long_shot: sc::long_shot(p, minute),
            dribble_attack: sc::dribble_attack(p, minute),
            defensive_duel: sc::defensive_duel(p, minute),
            interception: sc::interception(p, minute),
            pressing: sc::pressing(p, minute),
            aerial_def: sc::aerial_outfield_defender(p, minute),
            off_ball: sc::off_ball_attack(p, minute),
            shot_selection: sc::shot_selection(p, minute),
            pass_selection: sc::pass_selection(p, minute),
            defensive_positioning: sc::defensive_positioning(p, minute),
            mobility: sc::mobility(p, minute),
            decision_quality: sc::decision_quality(p, minute),
            loose_ball: sc::loose_ball_claim(p, minute),
            tackle_timing: sc::tackle_timing(p, minute),
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario 1 — equal quality, equal condition. Composites must agree.
// ---------------------------------------------------------------------------

#[test]
fn scenario_equal_quality_equal_condition_composites_match() {
    let a = build_player(14.0, 9500, PlayerPositionType::MidfielderCenter);
    let b = build_player(14.0, 9500, PlayerPositionType::MidfielderCenter);
    let s_a = CompositeSnapshot::capture(&a, 30);
    let s_b = CompositeSnapshot::capture(&b, 30);
    // Identical builds must produce identical composites — no random
    // drift hiding in `effective_skill` or composite math.
    for (label, va, vb) in [
        ("passing", s_a.passing, s_b.passing),
        ("shooting_close", s_a.shooting_close, s_b.shooting_close),
        ("defensive_duel", s_a.defensive_duel, s_b.defensive_duel),
        ("pressing", s_a.pressing, s_b.pressing),
        ("mobility", s_a.mobility, s_b.mobility),
    ] {
        assert!((va - vb).abs() < 1e-6, "{label} drifted {va} vs {vb}");
    }
}

// ---------------------------------------------------------------------------
// Scenario 2 — strong fresh vs weak fresh. Strong wins everywhere.
// ---------------------------------------------------------------------------

#[test]
fn scenario_strong_fresh_dominates_weak_fresh_every_composite() {
    let strong = build_player(17.0, 9500, PlayerPositionType::MidfielderCenter);
    let weak = build_player(8.0, 9500, PlayerPositionType::MidfielderCenter);
    let s = CompositeSnapshot::capture(&strong, 30);
    let w = CompositeSnapshot::capture(&weak, 30);
    let pairs = [
        ("passing", s.passing, w.passing),
        ("long_passing", s.long_passing, w.long_passing),
        ("first_touch", s.first_touch, w.first_touch),
        ("shooting_close", s.shooting_close, w.shooting_close),
        ("shooting_medium", s.shooting_medium, w.shooting_medium),
        ("long_shot", s.long_shot, w.long_shot),
        ("dribble_attack", s.dribble_attack, w.dribble_attack),
        ("defensive_duel", s.defensive_duel, w.defensive_duel),
        ("interception", s.interception, w.interception),
        ("pressing", s.pressing, w.pressing),
        ("aerial_def", s.aerial_def, w.aerial_def),
        ("off_ball", s.off_ball, w.off_ball),
        ("shot_selection", s.shot_selection, w.shot_selection),
        ("pass_selection", s.pass_selection, w.pass_selection),
        (
            "defensive_positioning",
            s.defensive_positioning,
            w.defensive_positioning,
        ),
        ("mobility", s.mobility, w.mobility),
        ("decision_quality", s.decision_quality, w.decision_quality),
        ("loose_ball", s.loose_ball, w.loose_ball),
        ("tackle_timing", s.tackle_timing, w.tackle_timing),
    ];
    for (label, sv, wv) in pairs {
        // Strong must beat weak by at least 0.05 on every composite.
        // Loose bound — the goal is "strong always wins on paper", not
        // a brittle margin lock.
        assert!(
            sv >= wv + 0.05,
            "strong should dominate {label}: strong={sv} weak={wv}"
        );
    }
}

// ---------------------------------------------------------------------------
// Scenario 3 — strong tired vs weak fresh. Gap narrows on explosive /
// pressing / mobility but holds on technical execution.
// ---------------------------------------------------------------------------

#[test]
fn scenario_tired_strong_vs_fresh_weak_freshness_closes_explosive_gap() {
    // 17/20 skill, condition 25% — broken legs late-game.
    let tired_strong = build_player(17.0, 2500, PlayerPositionType::MidfielderCenter);
    let fresh_weak = build_player(8.0, 9500, PlayerPositionType::MidfielderCenter);

    let ts = CompositeSnapshot::capture(&tired_strong, 80);
    let fw = CompositeSnapshot::capture(&fresh_weak, 80);

    // Strong-but-tired still leads on technical composites — skill gap
    // 17 vs 8 is larger than any fatigue penalty.
    assert!(
        ts.passing > fw.passing,
        "tired strong passing {} should exceed fresh weak {}",
        ts.passing,
        fw.passing
    );
    assert!(
        ts.shooting_close > fw.shooting_close,
        "tired strong shooting_close {} should beat fresh weak {}",
        ts.shooting_close,
        fw.shooting_close
    );
    assert!(
        ts.dribble_attack > fw.dribble_attack,
        "tired strong dribble {} should beat fresh weak {}",
        ts.dribble_attack,
        fw.dribble_attack
    );

    // Reference: same strong player fresh — measure how much fatigue
    // narrowed each composite gap.
    let fresh_strong = build_player(17.0, 9500, PlayerPositionType::MidfielderCenter);
    let fs = CompositeSnapshot::capture(&fresh_strong, 80);

    // Pressing must lose more ground to fatigue than passing — confirms
    // the explosive-band penalty still dominates the technical one,
    // which is what closes the gap for the fresh weak side.
    let press_loss = fs.pressing - ts.pressing;
    let pass_loss = fs.passing - ts.passing;
    assert!(press_loss > 0.0);
    assert!(pass_loss > 0.0);
    assert!(
        press_loss > pass_loss,
        "press_loss {press_loss} should exceed pass_loss {pass_loss}"
    );

    // Mobility loss should also exceed pass loss — same band reasoning.
    let mob_loss = fs.mobility - ts.mobility;
    assert!(
        mob_loss > pass_loss,
        "mob_loss {mob_loss} should exceed pass_loss {pass_loss}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 4 — elite high-stamina tired vs average tired. Elite
// degrades meaningfully less than average.
// ---------------------------------------------------------------------------

#[test]
fn scenario_elite_stamina_degrades_less_than_average_under_fatigue() {
    // Both at the same shattered condition; elite has stamina 19 + NF 18.
    let elite_tired = build_elite_stamina(14.0, 2500);
    let avg_tired = build_player(14.0, 2500, PlayerPositionType::MidfielderCenter);
    let et = CompositeSnapshot::capture(&elite_tired, 80);
    let at = CompositeSnapshot::capture(&avg_tired, 80);
    // Mobility, pressing, shooting_close — explosive-heavy composites
    // — must favour the elite stamina player.
    assert!(
        et.mobility > at.mobility,
        "elite mobility {} vs avg {}",
        et.mobility,
        at.mobility
    );
    assert!(
        et.pressing > at.pressing,
        "elite pressing {} vs avg {}",
        et.pressing,
        at.pressing
    );
    assert!(
        et.passing > at.passing,
        "elite passing {} vs avg {}",
        et.passing,
        at.passing
    );
}

#[test]
fn scenario_elite_stamina_does_not_match_fresh_self_at_extreme_fatigue() {
    // The mitigation cap taper means even elite stamina cannot make a
    // 15%-condition player look fresh. Composites must drop visibly.
    let fresh = build_elite_stamina(14.0, 9500);
    let broken = build_elite_stamina(14.0, 1500);
    let f = CompositeSnapshot::capture(&fresh, 80);
    let b = CompositeSnapshot::capture(&broken, 80);
    assert!(
        f.mobility - b.mobility > 0.03,
        "broken elite mobility {} too close to fresh {}",
        b.mobility,
        f.mobility
    );
    assert!(
        f.pressing - b.pressing > 0.03,
        "broken elite pressing {} too close to fresh {}",
        b.pressing,
        f.pressing
    );
}

// ---------------------------------------------------------------------------
// Scenario 5 — low-quality fresh vs high-quality tired. Freshness
// closes the gap (explosive) but does not erase technical skill gap.
// ---------------------------------------------------------------------------

#[test]
fn scenario_fresh_weak_narrows_but_does_not_erase_skill_gap() {
    let fresh_weak = build_player(8.0, 9500, PlayerPositionType::MidfielderCenter);
    let tired_strong = build_player(17.0, 2500, PlayerPositionType::MidfielderCenter);
    let fw = CompositeSnapshot::capture(&fresh_weak, 80);
    let ts = CompositeSnapshot::capture(&tired_strong, 80);

    // Technical execution gap survives the fatigue.
    assert!(ts.passing - fw.passing > 0.03);
    assert!(ts.shooting_close - fw.shooting_close > 0.03);
    assert!(ts.long_passing - fw.long_passing > 0.03);

    // The pressing / mobility gap should be tighter than the passing
    // gap — explosive band penalty bites the tired strong player.
    let pressing_gap = ts.pressing - fw.pressing;
    let mobility_gap = ts.mobility - fw.mobility;
    let passing_gap = ts.passing - fw.passing;
    assert!(
        pressing_gap < passing_gap,
        "pressing_gap {pressing_gap} should be tighter than passing_gap {passing_gap}"
    );
    assert!(
        mobility_gap < passing_gap,
        "mobility_gap {mobility_gap} should be tighter than passing_gap {passing_gap}"
    );
}

#[test]
fn scenario_fresh_weak_can_outwork_severely_depleted_strong_in_pressing() {
    // The most depleted strong player (15% floor, no stamina) gets out
    // -pressed by a fresh weak player — confirms freshness alone can
    // flip an explosive-heavy phase when the strong side is shattered.
    let mut depleted_strong =
        build_player(15.0, 1500, PlayerPositionType::MidfielderCenter);
    depleted_strong.skills.physical.stamina = 8.0;
    depleted_strong.skills.physical.natural_fitness = 8.0;
    let mut fresh_weak = build_player(10.0, 9500, PlayerPositionType::MidfielderCenter);
    fresh_weak.skills.physical.stamina = 17.0;
    fresh_weak.skills.physical.natural_fitness = 16.0;
    fresh_weak.skills.mental.work_rate = 17.0;

    let ds = sc::pressing(&depleted_strong, 85);
    let fw = sc::pressing(&fresh_weak, 85);
    assert!(
        fw > ds,
        "fresh hard-working weak {fw} should out-press shattered strong {ds}"
    );
}

// ---------------------------------------------------------------------------
// Goalkeeper-specific: fatigue meaningfully drops save reach but does
// not collapse the keeper into an open-net scenario.
// ---------------------------------------------------------------------------

#[test]
fn scenario_tired_keeper_drops_reach_without_collapsing() {
    let fresh_inputs = GoalkeeperSkillInputs {
        minute: 80,
        condition_pct: 0.95,
    };
    let tired_inputs = GoalkeeperSkillInputs {
        minute: 80,
        condition_pct: 0.25,
    };
    let fresh_keeper = build_player(15.0, 9500, PlayerPositionType::Goalkeeper);
    let tired_keeper = build_player(15.0, 2500, PlayerPositionType::Goalkeeper);
    let fresh = GoalkeeperSkillProfile::from_player(&fresh_keeper, &fresh_inputs);
    let tired = GoalkeeperSkillProfile::from_player(&tired_keeper, &tired_inputs);

    // Reach should drop visibly — but not collapse to zero.
    assert!(fresh.dive_reach > tired.dive_reach);
    assert!(tired.dive_reach > 0.15);

    // Save probability vs a high-quality shot drops but stays usable.
    let fresh_save = fresh.save_probability(0.80);
    let tired_save = tired.save_probability(0.80);
    assert!(fresh_save > tired_save);
    assert!(tired_save > 0.05);
    assert!(fresh_save - tired_save < 0.40);
}

// ---------------------------------------------------------------------------
// Spec line: low condition increases miscontrols / errors and reduces
// pressure / sprint / reach. We don't directly count miscontrols here
// (engine event), but we verify the upstream first-touch / receiving
// composite drops, which feeds the miscontrol probability.
// ---------------------------------------------------------------------------

#[test]
fn fatigue_lowers_receiving_first_touch_composite() {
    let fresh = build_player(13.0, 9500, PlayerPositionType::MidfielderCenter);
    let tired = build_player(13.0, 2500, PlayerPositionType::MidfielderCenter);
    let f = sc::receiving_first_touch(&fresh, 80);
    let t = sc::receiving_first_touch(&tired, 80);
    assert!(f > t, "fresh first_touch {f} should exceed tired {t}");
}

#[test]
fn fatigue_lowers_defensive_duel_and_tackle_timing() {
    let fresh = build_player(13.0, 9500, PlayerPositionType::DefenderCenter);
    let tired = build_player(13.0, 2500, PlayerPositionType::DefenderCenter);
    assert!(sc::defensive_duel(&fresh, 80) > sc::defensive_duel(&tired, 80));
    assert!(sc::tackle_timing(&fresh, 80) > sc::tackle_timing(&tired, 80));
    assert!(sc::interception(&fresh, 80) > sc::interception(&tired, 80));
}

// ---------------------------------------------------------------------------
// Cross-cutting: `effective_skill` is the single source of truth. If
// fatigue stopped flowing through it, every test above would still
// pass on a single player, but the cross-player invariants below would
// break — guard against future raw-skill-read reintroductions.
// ---------------------------------------------------------------------------

#[test]
fn effective_skill_applies_to_all_three_categories() {
    let fresh = build_player(15.0, 9500, PlayerPositionType::MidfielderCenter);
    let tired = build_player(15.0, 2500, PlayerPositionType::MidfielderCenter);
    for cat in [
        ActionContext::technical(80),
        ActionContext::mental(80),
        ActionContext::explosive(80),
    ] {
        let f = effective_skill(&fresh, 15.0, cat);
        let t = effective_skill(&tired, 15.0, cat);
        assert!(f > t, "category {:?}: fresh {f} should exceed tired {t}", cat);
    }
}
