//! Kinematic contract of the velocity ramp (`MovementEffort::sprint_ramp`).
//!
//! These pin the physical claims behind the `acceleration` attribute:
//!
//!   1. A higher-`acceleration` player reaches top speed measurably
//!      sooner than a lower one — the attribute has a KINEMATIC channel,
//!      not just its 0.2 weight inside the top-speed blend. Before the
//!      ramp existed the integrator allowed a full stop-to-sprint change
//!      in 1–2 AI ticks, so a 6 and an 18 differed by ~2 cm off a
//!      standing start and every race was settled by top speed alone.
//!   2. The ramp is a BUILD, not a teleport — a standing start takes
//!      tenths of a second to top speed, not the old 20–40 ms teleport
//!      and not ten seconds of treacle. Bounds are deliberately loose;
//!      the exact dose is calibration (see `ACCEL_PEAK_FLOOR_MS2`'s
//!      titration history), not a unit test's business.
//!   3. Braking/turning outruns accelerating (eccentric beats
//!      concentric), which is what keeps arrivals crisp.
//!   4. Fatigue shrinks burst: the same player at broken condition ramps
//!      slower than fresh — the `effective_skill` explosive band reaches
//!      the integrator.
//!   5. Nothing the ramp returns ever exceeds the athletic ceiling.
//!
//! All comparative — no absolute u/tick constants that would lock the
//! calibration (same policy as `fatigue_calibration_tests`).

#![cfg(test)]

use crate::PlayerSkills;
use crate::club::player::builder::PlayerBuilder;
use crate::r#match::MatchPlayer;
use crate::r#match::MovementEffort;
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
};
use chrono::NaiveDate;
use nalgebra::Vector3;

fn build_runner(acceleration: f32, agility: f32, condition: i16) -> MatchPlayer {
    let mut attrs = PlayerAttributes::default();
    attrs.condition = condition;
    attrs.jadedness = 0;
    let mut skills = PlayerSkills::default();
    // Identical top-speed inputs across every runner in a comparison —
    // pace fixed, and the attribute under test varied per call. Stamina /
    // NF at the population mean so the fatigue-mitigation lever stays
    // constant unless a test moves condition itself.
    skills.physical.pace = 14.0;
    skills.physical.acceleration = acceleration;
    skills.physical.agility = agility;
    skills.physical.stamina = 12.0;
    skills.physical.natural_fitness = 12.0;
    let player = PlayerBuilder::new()
        .id(1)
        .full_name(FullName::new("R".to_string(), "P".to_string()))
        .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
        .country_id(1)
        .attributes(PersonAttributes::default())
        .skills(skills)
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position: PlayerPositionType::MidfielderCenter,
                level: 18,
            }],
        })
        .player_attributes(attrs)
        .build()
        .unwrap();
    MatchPlayer::from_player(
        1,
        &player,
        PlayerPositionType::MidfielderCenter,
        false,
        None,
    )
}

/// AI ticks for a standing start to reach `fraction` of top speed,
/// driving the ramp exactly as the integrator does (desired = flat-out
/// toward +x, both ceilings at conditioned max).
fn ticks_to_speed(player: &mut MatchPlayer, fraction: f32) -> u32 {
    let max = player.max_speed_with_condition_cached();
    let desired = Vector3::new(max, 0.0, 0.0);
    player.velocity = Vector3::zeros();
    for tick in 1..=600 {
        player.velocity = MovementEffort::sprint_ramp(player, 10, desired, max, max);
        if player.velocity.norm() >= fraction * max {
            return tick;
        }
    }
    601
}

#[test]
fn higher_acceleration_reaches_top_speed_sooner() {
    // The budget itself must separate the attribute (continuous — no
    // tick quantization): ≥15% more burst for 18 than for 6.
    let quick = build_runner(18.0, 12.0, 9500);
    let slow = build_runner(6.0, 12.0, 9500);
    let b_quick = MovementEffort::accel_budget(&quick, 10, true);
    let b_slow = MovementEffort::accel_budget(&slow, 10, true);
    assert!(
        b_quick > b_slow * 1.15,
        "accel 18 budget {b_quick} should be ≥15% over accel 6 {b_slow}"
    );
    // And the race must show it: same pace, standing start, first
    // second (50 AI ticks) — the quick starter leads by a real margin
    // (≥1.5 u ≈ a stride). Ramp + blend both contribute; the point is
    // that the ATTRIBUTE decides the first metres.
    let mut quick = quick;
    let mut slow = slow;
    quick.velocity = Vector3::zeros();
    slow.velocity = Vector3::zeros();
    let (mut d_quick, mut d_slow) = (0.0f32, 0.0f32);
    for _ in 0..50 {
        let mq = quick.max_speed_with_condition_cached();
        let ms = slow.max_speed_with_condition_cached();
        quick.velocity =
            MovementEffort::sprint_ramp(&quick, 10, Vector3::new(mq, 0.0, 0.0), mq, mq);
        slow.velocity = MovementEffort::sprint_ramp(&slow, 10, Vector3::new(ms, 0.0, 0.0), ms, ms);
        // Positions integrate on BOTH sim ticks of each AI tick.
        d_quick += quick.velocity.x * 2.0;
        d_slow += slow.velocity.x * 2.0;
    }
    assert!(
        d_quick > d_slow + 1.5,
        "accel 18 should lead a same-pace standing start by ≥1.5 u after 1 s: {d_quick} vs {d_slow}"
    );
}

#[test]
fn standing_start_is_a_build_not_a_teleport() {
    // Population-mean burst: 90% of top speed within 0.12–2.4 s (6–120
    // AI ticks at 20 ms). Both edges matter: the low bound is the old
    // instant-ramp defect (1–2 ticks), the high bound is a treacle
    // engine. The dose currently sits at the fast end deliberately —
    // the titration history lives on `ACCEL_PEAK_FLOOR_MS2`.
    let mut avg = build_runner(12.0, 12.0, 9500);
    let t = ticks_to_speed(&mut avg, 0.90);
    assert!(
        (6..=120).contains(&t),
        "mean standing start should take 0.12–2.4 s to 90% top speed, got {t} AI ticks"
    );
}

#[test]
fn braking_budget_exceeds_acceleration_budget() {
    let player = build_runner(12.0, 12.0, 9500);
    let gain = MovementEffort::accel_budget(&player, 10, true);
    let brake = MovementEffort::accel_budget(&player, 10, false);
    assert!(
        brake > gain * 1.5,
        "braking {brake} should comfortably exceed accelerating {gain}"
    );
}

#[test]
fn agility_owns_the_braking_multiplier() {
    let nimble = build_runner(12.0, 18.0, 9500);
    let stiff = build_runner(12.0, 6.0, 9500);
    // Same acceleration → same forward budget…
    let f_nimble = MovementEffort::accel_budget(&nimble, 10, true);
    let f_stiff = MovementEffort::accel_budget(&stiff, 10, true);
    assert!((f_nimble - f_stiff).abs() < 1e-6);
    // …but the nimble player redirects harder.
    let b_nimble = MovementEffort::accel_budget(&nimble, 10, false);
    let b_stiff = MovementEffort::accel_budget(&stiff, 10, false);
    assert!(
        b_nimble > b_stiff,
        "agility 18 braking {b_nimble} should exceed agility 6 {b_stiff}"
    );
}

#[test]
fn broken_condition_shrinks_burst() {
    let fresh = build_runner(14.0, 12.0, 9500);
    let broken = build_runner(14.0, 12.0, 2000);
    let g_fresh = MovementEffort::accel_budget(&fresh, 80, true);
    let g_broken = MovementEffort::accel_budget(&broken, 80, true);
    assert!(
        g_broken < g_fresh * 0.97,
        "broken legs should lose burst: fresh {g_fresh} vs broken {g_broken}"
    );
}

#[test]
fn ramp_never_exceeds_athletic_ceiling() {
    let mut p = build_runner(20.0, 20.0, 9500);
    let max = p.max_speed_with_condition_cached();
    // Ask for something absurd (a raw-attribute-as-speed state bug) —
    // the ramped result must stay under the athletic ceiling every tick.
    let desired = Vector3::new(9.0, 4.0, 0.0);
    p.velocity = Vector3::zeros();
    for _ in 0..300 {
        p.velocity = MovementEffort::sprint_ramp(&p, 10, desired, max, max);
        assert!(
            p.velocity.norm() <= max * 1.0001,
            "ramp exceeded athletic ceiling: {} > {max}",
            p.velocity.norm()
        );
    }
}

#[test]
fn sprint_reversal_transits_brake_then_reaccelerates() {
    let mut p = build_runner(12.0, 12.0, 9500);
    let max = p.max_speed_with_condition_cached();
    p.velocity = Vector3::new(max, 0.0, 0.0);
    let desired = Vector3::new(-max, 0.0, 0.0);
    // The x-velocity must fall monotonically through the reversal —
    // no single-tick flip (the old twitch signature).
    let mut prev_x = p.velocity.x;
    let mut reversed_at = None;
    for tick in 1..=600 {
        p.velocity = MovementEffort::sprint_ramp(&p, 10, desired, max, max);
        assert!(
            p.velocity.x < prev_x + 1e-6,
            "reversal must be monotonic in x, tick {tick}"
        );
        prev_x = p.velocity.x;
        if reversed_at.is_none() && p.velocity.x <= -0.9 * max {
            reversed_at = Some(tick);
            break;
        }
    }
    let t = reversed_at.expect("player never completed the reversal");
    // A full sprint reversal is slower than a standing start (more
    // speed to shed than to gain) but still resolves within ~3 s —
    // and takes more than a couple of ticks (the old twitch flip).
    assert!(
        (8..=150).contains(&t),
        "sprint reversal should take 0.16–3 s, got {t} AI ticks"
    );
}
