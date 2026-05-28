//! Solid-engine integration tests.
//!
//! Exercises the wirings landed in `docs/match-engine-solid-engine-prompt.md`:
//!   1. Seeded `MatchRng` produces reproducible streams.
//!   2. Referee `card_modifier` shifts foul card probabilities the way
//!      live foul handling now consumes it.
//!   3. Environment `pass_accuracy` lowers `PassEvaluator` success.
//!   4. Environment `goalkeeper_handling` lowers physics save probability.
//!   5. Ball lifecycle invariants hold across the dead-ball helper.
//!
//! Tests in this file stay at the helper-fn / public-API level so a
//! regression in one wiring (env modifier, referee scoring, ball
//! invariants, RNG seeding) shows up as a focused failure rather than
//! a diffuse full-match drift. Full live-path coverage lives in the
//! sibling test files:
//!   * Same-seed full-match replay: `calibration_harness.rs`
//!     `same_seed_produces_results_within_a_narrow_band` exercises a
//!     full match seeded through `play_with_config`.
//!   * Environment wiring: `match_realism_tests.rs` covers per-modifier
//!     deltas; the rainy/muddy calibration test exercises full live
//!     paths.
//!   * Referee/foul wiring: `calibration_harness.rs`
//!     `strict_referee_awards_more_fouls_than_lenient_referee` runs the
//!     marginal-call gate end-to-end.

#![cfg(test)]

use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::engine::environment::{MatchEnvironment, Pitch, Weather};
use crate::r#match::engine::referee::RefereeProfile;
use crate::r#match::engine::rng::MatchRng;
use nalgebra::Vector3;

// ──────────────────────────────────────────────────────────────────────
// 1. MatchRng — seeded reproducibility
// ──────────────────────────────────────────────────────────────────────

#[test]
fn match_rng_same_seed_replays_identically() {
    let a = MatchRng::from_seed(0xABCDEF);
    let b = MatchRng::from_seed(0xABCDEF);
    let mut diff_count = 0;
    for _ in 0..10_000 {
        if a.unit_f32().to_bits() != b.unit_f32().to_bits() {
            diff_count += 1;
        }
    }
    assert_eq!(
        diff_count, 0,
        "two RNGs seeded with the same value must emit identical streams"
    );
}

#[test]
fn match_rng_substitution_window_draws_are_seedable() {
    // The substitution loop in `engine::play_inner` now draws its 5-15
    // minute and 10-20 minute windows from `context.rng`. Simulating
    // the exact sequence with the same seed must yield identical
    // windows — the foundation for replayable substitution timing.
    fn windows(seed: u64) -> Vec<u64> {
        let rng = MatchRng::from_seed(seed);
        let mut out = Vec::with_capacity(8);
        // First range: initial offset, 10..20 min.
        out.push(rng.range_u64(10, 20));
        // Subsequent: 5..15 each, repeated.
        for _ in 0..7 {
            out.push(rng.range_u64(5, 15));
        }
        out
    }
    assert_eq!(windows(7), windows(7));
    // Different seeds should disagree on at least one draw within 8 picks.
    assert_ne!(windows(7), windows(8));
}

#[test]
fn match_rng_bernoulli_bounds_make_sense() {
    let r = MatchRng::from_seed(1234);
    // p = 0.0 must never fire; p = 1.0 must always fire.
    for _ in 0..256 {
        assert!(!r.bernoulli(0.0));
        assert!(r.bernoulli(1.0));
    }
    // p outside [0, 1] is clamped (no panic).
    for _ in 0..16 {
        let _ = r.bernoulli(-0.5);
        let _ = r.bernoulli(1.5);
    }
}

// ──────────────────────────────────────────────────────────────────────
// 2. Referee card modifier wiring
// ──────────────────────────────────────────────────────────────────────

#[test]
fn card_modifier_grows_with_card_happiness_and_derby_intensity() {
    let calm_env = MatchEnvironment::default();
    let derby_env = MatchEnvironment {
        derby_intensity: 1.0,
        match_importance: 1.0,
        ..Default::default()
    };
    let lenient = RefereeProfile {
        card_happiness: 0.05,
        ..Default::default()
    };
    let strict = RefereeProfile {
        card_happiness: 0.95,
        ..Default::default()
    };

    assert!(
        strict.card_modifier(&calm_env) > lenient.card_modifier(&calm_env),
        "card-happy ref must produce a larger card modifier than a lenient ref"
    );
    assert!(
        lenient.card_modifier(&derby_env) > lenient.card_modifier(&calm_env),
        "even a lenient ref must scale up cards in a derby"
    );

    // The live foul handler in `players.rs::handle_commit_foul_event`
    // multiplies its base card probability by this modifier and
    // re-clamps to the calibrated band. Verify the modifier itself
    // stays in a sane multiplier band so re-clamping after is the only
    // safeguard needed (not a hard ceiling on the modifier).
    let m = strict.card_modifier(&derby_env);
    assert!((0.5..=2.5).contains(&m), "card_modifier out of band: {m}");
}

// ──────────────────────────────────────────────────────────────────────
// 3. Environment wiring — pass evaluator
//
// We can't easily build a `StateProcessingContext` in a small unit
// test, but we can verify the EnvModifiers feeding the evaluator
// behave as the wiring expects: short passes use `pass_accuracy`, long
// passes layer `long_pass_accuracy` on top. The evaluator's call site
// does `pass_accuracy + (if long { long_pass_accuracy } else { 0.0 })`
// — the math here mirrors that.
// ──────────────────────────────────────────────────────────────────────

#[test]
fn env_modifier_for_pass_combines_short_and_long_deltas() {
    let env = MatchEnvironment {
        weather: Weather::Wind,
        pitch: Pitch::Wet,
        ..Default::default()
    };
    let m = env.modifiers();

    // Wind only kicks long-pass accuracy; wet pitch leaves pass_accuracy alone.
    assert_eq!(m.pass_accuracy, 0.0, "wet+wind shouldn't touch short pass accuracy");
    assert!(m.long_pass_accuracy < 0.0, "wind must reduce long pass accuracy");

    // A 50% raw success on a short pass should remain 50% in wind/wet conditions.
    let raw = 0.50_f32;
    let short_delta = m.pass_accuracy;
    let short_adjusted = (raw + short_delta).clamp(0.1, 0.99);
    assert!((short_adjusted - 0.50).abs() < 1e-3);

    // The same 50% raw success at long-pass distance should drop.
    let long_delta = m.pass_accuracy + m.long_pass_accuracy;
    let long_adjusted = (raw + long_delta).clamp(0.1, 0.99);
    assert!(
        long_adjusted < raw,
        "long pass in wind must drop below raw: {long_adjusted} vs {raw}"
    );
}

#[test]
fn heavy_rain_reduces_short_pass_accuracy_too() {
    let env = MatchEnvironment {
        weather: Weather::HeavyRain,
        ..Default::default()
    };
    let m = env.modifiers();
    assert!(m.pass_accuracy < 0.0);
    // Mirrors the evaluator's combine for a short pass.
    let raw = 0.65_f32;
    let adjusted = (raw + m.pass_accuracy).clamp(0.1, 0.99);
    assert!(adjusted < raw - 0.04, "heavy rain hit too small: {adjusted} vs {raw}");
}

// ──────────────────────────────────────────────────────────────────────
// 4. Environment wiring — goalkeeper save probability
//
// `try_save_shot` now folds `env.modifiers().goalkeeper_handling` into
// the save probability via `((base - speed_penalty) * skill_mult +
// env_delta).clamp(0.05, 0.68)`. The math is reproducible without
// running the physics step.
// ──────────────────────────────────────────────────────────────────────

#[test]
fn heavy_rain_reduces_keeper_save_probability() {
    let dry = MatchEnvironment::default();
    let rain = MatchEnvironment {
        weather: Weather::HeavyRain,
        ..Default::default()
    };

    // Reproduce the live formula with placeholder numbers in the
    // calibrated middle of the bands. The point isn't the exact value
    // — it's that the env delta is non-zero and lands in the right
    // direction.
    let base_save = 0.55_f32;
    let dry_save = (base_save + dry.modifiers().goalkeeper_handling).clamp(0.05, 0.68);
    let rain_save = (base_save + rain.modifiers().goalkeeper_handling).clamp(0.05, 0.68);
    assert!(
        rain_save < dry_save,
        "heavy rain must reduce save probability ({rain_save} vs {dry_save})"
    );
    assert!(dry_save - rain_save > 0.05, "rain effect too small: {dry_save} -> {rain_save}");
}

// ──────────────────────────────────────────────────────────────────────
// 5. Ball lifecycle invariants
// ──────────────────────────────────────────────────────────────────────

#[test]
fn fresh_ball_satisfies_lifecycle_invariants() {
    let ball = Ball::with_coord(840.0, 545.0);
    assert!(ball.check_invariants().is_ok());
}

#[test]
fn clear_open_play_metadata_makes_invariants_hold_after_set_piece_restart() {
    use crate::r#match::PlayerSide;
    use crate::r#match::engine::ball::ball::ShotTarget;

    let mut ball = Ball::with_coord(840.0, 545.0);
    // Stage an in-flight shot — open-play metadata populated, owner set.
    ball.previous_owner = Some(9);
    ball.cached_shot_target = Some(ShotTarget {
        goal_line_y: 270.0,
        goal_line_z: 1.5,
        defending_side: PlayerSide::Right,
    });
    ball.pass_target_player_id = Some(10);
    ball.pending_pass_passer = Some(9);
    // Real engine paths populate origin/target alongside the passer id
    // on every emitted pass — the invariant requires the trio.
    ball.pending_pass_origin = Some(Vector3::new(100.0, 200.0, 0.0));
    ball.pending_pass_target = Some(Vector3::new(400.0, 200.0, 0.0));
    ball.last_shot_xg = 0.22;
    ball.last_shot_shooter_id = Some(9);
    // The invariants should hold here: shot + previous_owner, pass +
    // pending_passer are both internally consistent.
    assert!(
        ball.check_invariants().is_ok(),
        "in-flight shot/pass with proper provenance should be invariant-clean: {:?}",
        ball.check_invariants()
    );

    // Now simulate a set-piece restart wiping open-play metadata.
    ball.clear_open_play_metadata();
    assert!(ball.cached_shot_target.is_none());
    assert!(ball.pass_target_player_id.is_none());
    assert!(ball.pending_pass_passer.is_none());
    assert!(ball.offside_snapshot.is_none());
    assert!(ball.pending_save_credit.is_none());
    assert_eq!(ball.last_shot_xg, 0.0);
    assert!(ball.last_shot_shooter_id.is_none());
    assert!(ball.check_invariants().is_ok());
}

#[test]
fn shot_without_previous_owner_fails_invariant() {
    use crate::r#match::PlayerSide;
    use crate::r#match::engine::ball::ball::ShotTarget;

    let mut ball = Ball::with_coord(840.0, 545.0);
    ball.cached_shot_target = Some(ShotTarget {
        goal_line_y: 270.0,
        goal_line_z: 1.5,
        defending_side: PlayerSide::Right,
    });
    // No previous_owner → "who fired this?" can't be answered — debug
    // builds and tests should flag this.
    assert_eq!(
        ball.check_invariants(),
        Err("cached_shot_target without previous_owner")
    );
}

#[test]
fn pass_target_without_passer_fails_invariant() {
    let mut ball = Ball::with_coord(840.0, 545.0);
    ball.pass_target_player_id = Some(10);
    // No pending_pass_passer → completion classifier has nothing to
    // pair the receive event to.
    assert_eq!(
        ball.check_invariants(),
        Err("pass_target without pending_pass_passer")
    );
}

#[test]
fn non_finite_ball_position_fails_invariant() {
    let mut ball = Ball::with_coord(840.0, 545.0);
    ball.position.x = f32::NAN;
    assert!(matches!(
        ball.check_invariants(),
        Err("ball position has non-finite coordinate")
    ));
}

#[test]
fn dead_ball_corner_with_leftover_shot_metadata_fails_invariant() {
    use crate::r#match::PlayerSide;
    use crate::r#match::engine::ball::ball::{PassOriginRestart, ShotTarget};

    let mut ball = Ball::with_coord(840.0, 545.0);
    ball.previous_owner = Some(9);
    ball.pass_origin_restart = PassOriginRestart::Corner;
    // Leftover shot metadata after a dead-ball restart leaks open-play
    // state across the whistle — the invariant must reject this.
    ball.cached_shot_target = Some(ShotTarget {
        goal_line_y: 270.0,
        goal_line_z: 1.5,
        defending_side: PlayerSide::Right,
    });
    assert!(matches!(
        ball.check_invariants(),
        Err("dead-ball restart with leftover cached_shot_target")
    ));
}

#[test]
fn carry_owner_disagreeing_with_current_owner_fails_invariant() {
    let mut ball = Ball::with_coord(840.0, 545.0);
    ball.current_owner = Some(7);
    ball.carry_owner = Some(8);
    assert_eq!(
        ball.check_invariants(),
        Err("carry_owner disagrees with current_owner")
    );
}

#[test]
fn pending_pass_passer_without_origin_fails_invariant() {
    let mut ball = Ball::with_coord(840.0, 545.0);
    ball.pending_pass_passer = Some(7);
    // origin/target unset → pass envelope is incomplete; completion
    // classifier can't decide cross/progressive/box-entry.
    assert_eq!(
        ball.check_invariants(),
        Err("pending_pass_passer without origin/target metadata")
    );
}
