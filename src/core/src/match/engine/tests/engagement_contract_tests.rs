//! **The tackle contract**, end to end.
//!
//! Twenty-one defender states, ten midfielder ones and four forward ones
//! each decided for themselves when a player could go and challenge the
//! man on the ball, on their own distance — 2u, 8u, 10u, 15u, 20u, 25u,
//! 30u, 40u, 80u, 100u — while the `Tackling` states they handed him to
//! broke off at `DISENGAGE` (24u) and only ever rolled an attempt inside
//! `CONTACT` (10u). Several of those doors therefore requested a state
//! whose exit condition was already true on the tick it was entered.
//!
//! These pin the contract rather than any one door:
//!
//!   * the distances are ordered, so entry and break-off cannot overlap;
//!   * every transition site in the engine asks the shared predicate;
//!   * the decision clock is a rate, so it survives a state change and
//!     does not depend on the tick size;
//!   * a man who has lost the ball does not kick it anyway.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::common_states::{EngagementClock, TackleEngagement, TackleOutcome};
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::result::Score;
use crate::r#match::player::events::FoulSeverity;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::defenders::states::clearing::DefenderClearingState;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, PlayerSide,
    StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use std::path::{Path, PathBuf};

fn pitch() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    (field, context)
}

// ── The geometry ──────────────────────────────────────────────────────

/// `CONTACT < COMMIT < DISENGAGE`, strictly. The ordering is the whole
/// mechanism: a state whose give-up condition overlaps its own entry
/// condition is a two-cycle by construction.
#[test]
fn the_engagement_distances_are_ordered() {
    assert!(TackleEngagement::CONTACT < TackleEngagement::COMMIT);
    assert!(TackleEngagement::COMMIT < TackleEngagement::DISENGAGE);
}

// ── The doors ─────────────────────────────────────────────────────────

/// Source scan: every site that constructs a `Tackling` transition target
/// carries the shared predicate within a few lines above it.
///
/// A behavioural test cannot reach twenty-one doors, and the defect this
/// replaces was structural — each state deciding the same thing on its
/// own number. Conservative on purpose: it asks only that the contract is
/// *mentioned* near the site, which is exactly what drifted.
#[test]
fn every_tackling_door_asks_the_shared_contract() {
    struct Scan;
    impl Scan {
        fn strategies() -> PathBuf {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("match")
                .join("engine")
                .join("player")
                .join("strategies")
        }

        fn files(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::files(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    out.push(path);
                }
            }
        }
    }

    // `states/state.rs` is the enum declaration and its dispatch table;
    // `tackling/mod.rs` is the state itself, not a door into it.
    let is_declaration = |path: &Path| {
        let p = path.to_string_lossy().replace('\\', "/");
        p.ends_with("/states/state.rs") || p.contains("/tackling/")
    };

    let mut files = Vec::new();
    Scan::files(&Scan::strategies(), &mut files);
    let mut offenders = Vec::new();
    for path in files {
        if is_declaration(&path) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if !code.contains("State::Tackling") {
                continue;
            }
            // The guard sits above the transition; a dozen lines covers
            // the widest of them (the box-emergency helper included).
            let from = i.saturating_sub(12);
            let guarded = lines[from..=i].iter().any(|l| {
                l.contains("TackleEngagement::should_commit")
                    || l.contains("BoxEmergency::response")
            });
            if !guarded {
                offenders.push(format!("{}:{}", path.display(), i + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these hand a player into Tackling without asking TackleEngagement::should_commit:\n{}",
        offenders.join("\n")
    );
}

// ── The clock ─────────────────────────────────────────────────────────

/// A rate drawn per tick reproduces itself over its own window. This is
/// what makes the commitment model independent of the tick size, and it
/// is the property the modulo cadence did not have.
#[test]
fn a_rate_drawn_per_tick_is_the_same_rate_over_its_window() {
    for rate in [0.02f32, 0.098, 0.25, 0.5] {
        let per_tick = EngagementClock::per_tick(rate, 1.0);
        let ticks = (1.0f32 / EngagementClock::TICK_SECONDS).round() as i32;
        let survives = (1.0f32 - per_tick).powi(ticks);
        assert!(
            ((1.0 - survives) - rate).abs() < 1e-4,
            "rate {rate} compounded to {} over one second",
            1.0 - survives
        );
    }
}

/// …and at any tick size. The engine already runs goalkeepers on 10 ms
/// light ticks while a shot is in flight, so this is live, not
/// hypothetical.
#[test]
fn the_same_elapsed_time_gives_the_same_chance_at_any_step() {
    let rate = 0.15f32;
    let over = |window: f32, step_seconds: f32, seconds: f32| {
        let per_step = 1.0 - (1.0 - rate).powf(step_seconds / window);
        let steps = (seconds / step_seconds).round() as i32;
        1.0 - (1.0 - per_step).powi(steps)
    };
    let a = over(1.0, 0.010, 2.0);
    let b = over(1.0, 0.020, 2.0);
    let c = over(1.0, 0.040, 2.0);
    assert!((a - b).abs() < 1e-4, "{a} vs {b}");
    assert!((b - c).abs() < 1e-4, "{b} vs {c}");
}

/// A two-second rate is quoted over two seconds. `ContactFoul` is fitted
/// against that window and must not be silently doubled by moving to a
/// per-tick draw.
#[test]
fn a_two_second_rate_keeps_its_window() {
    let rate = 0.021f32;
    let per_tick = EngagementClock::per_tick(rate, 2.0);
    let ticks = (2.0f32 / EngagementClock::TICK_SECONDS).round() as i32;
    let compounded = 1.0 - (1.0f32 - per_tick).powi(ticks);
    assert!((compounded - rate).abs() < 1e-4, "got {compounded}");
}

// ── The outcome ───────────────────────────────────────────────────────

/// Winning the ball cleanly is not a foul, and the precedence is the
/// model's rather than the call site's — it used to differ by role.
#[test]
fn a_challenge_has_exactly_one_outcome() {
    assert_eq!(
        TackleOutcome::of(true, true, FoulSeverity::Reckless),
        TackleOutcome::Won
    );
    assert_eq!(
        TackleOutcome::of(false, true, FoulSeverity::Reckless),
        TackleOutcome::Foul(FoulSeverity::Reckless)
    );
    assert_eq!(
        TackleOutcome::of(false, false, FoulSeverity::Normal),
        TackleOutcome::Missed
    );
}

// ── Containment is interruptible ──────────────────────────────────────

/// A man standing his opponent up is not mid-action. The challenge itself
/// resolves inside one `process()`; what the committed flag froze was up
/// to 600 ms of jockeying, during which he was dropped from the chase
/// table and ignored every loose-ball redirect.
#[test]
fn jockeying_is_not_a_committed_action() {
    assert!(!PlayerState::Defender(DefenderState::Tackling).is_committed_action());
    // …while the actions that really are un-abortable still are.
    assert!(PlayerState::Defender(DefenderState::Clearing).is_committed_action());
    assert!(PlayerState::Defender(DefenderState::Heading).is_committed_action());
}

// ── No stale kick ─────────────────────────────────────────────────────

/// A defender who loses the ball during the clearance wind-up does not
/// hoof it anyway. The dispatcher's own guard is a REACH test, so a man
/// still within `KICKABLE_DISTANCE` of a ball that has become somebody
/// else's would have had his kick executed.
#[test]
fn losing_the_ball_during_the_wind_up_cancels_the_kick() {
    let (mut field, context) = pitch();
    let defender = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .expect("the home side has an outfielder");
    let thief = field
        .players
        .iter()
        .find(|p| p.side == Some(PlayerSide::Right))
        .map(|p| p.id)
        .expect("the away side has a player");

    let at = Vector3::new(60.0, 272.0, 0.0);
    for p in field.players.iter_mut() {
        if p.id == defender || p.id == thief {
            p.position = at;
        }
    }
    field.ball.position = at;
    field.ball.current_owner = Some(defender);

    let state = DefenderClearingState::default();

    // His ball, well past the wind-up: the kick goes.
    let tick_context = GameTickContext::new(&field, &context.players);
    let player = field.players.iter().find(|p| p.id == defender).unwrap();
    let his = StateProcessingContext {
        in_state_time: 10,
        player,
        context: &context,
        tick_context: &tick_context,
    };
    let kicked = state
        .process(&his)
        .expect("a defender on the ball clears it");
    assert!(
        kicked.events.has_events(),
        "the clearance produced no kick at all — the test is vacuous"
    );

    // Same man, same place, same tick count — but it is not his any more.
    field.ball.current_owner = Some(thief);
    let tick_context = GameTickContext::new(&field, &context.players);
    let player = field.players.iter().find(|p| p.id == defender).unwrap();
    let stolen = StateProcessingContext {
        in_state_time: 10,
        player,
        context: &context,
        tick_context: &tick_context,
    };
    let aborted = state
        .process(&stolen)
        .expect("a defender who has lost it leaves the state");
    assert!(
        !aborted.events.has_events(),
        "he kicked a ball that was no longer his"
    );
    assert_eq!(
        aborted.state,
        Some(PlayerState::Defender(DefenderState::Standing)),
        "an aborted clearance hands him back to an off-ball decision"
    );
}
