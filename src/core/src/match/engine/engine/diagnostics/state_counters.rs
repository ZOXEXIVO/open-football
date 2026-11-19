//! **Per-role state counters** the `dev_match` harness prints as the
//! tackle-flow, shot-gate and stuck-exit blocks. Counted from the state
//! handlers of every role; nothing here changes what the match does.

// ───────────────────────────────────────────────────────────────────────────
// Shot-gate rejection counters — `match-logs` feature only. A forward tick
// with the ball in shooting range increments a waterfall of passes: each
// counter represents the subset of ticks that got past the prior gate.
// Drop-off between adjacent counters = how often that specific gate fires.
// `fired` is the ultimate output; gap between `passed_willingness` and
// `fired` is the rng drop inside the willingness roll.
// ───────────────────────────────────────────────────────────────────────────
#[cfg(feature = "match-logs")]
/// Which `return None` a stuck forward is taking.
///
/// `Forward: Running` is the last row in the ball-stuck table that is a
/// genuine freeze rather than the claim-tick label (96-98% of its stuck
/// ticks have the owner 250-plus AI ticks into the state). Only a
/// `return None` keeps him in Running — every other exit changes state
/// and resets the clock — so counting them under exactly that condition
/// names the branch without a replay dump.
#[cfg(feature = "match-logs")]
pub mod stuck_exit_stats {
    use std::sync::atomic::{AtomicU64, Ordering};

    pub const EXITS: usize = 5;
    const ZERO: AtomicU64 = AtomicU64::new(0);
    /// 0 tempo-hold, 1 point-blank Hold, 2 box Hold, 3 drive-at-goal,
    /// 4 fell off the end of the tree.
    pub static EXIT: [AtomicU64; EXITS] = [ZERO; EXITS];
    pub const NAMES: [&str; EXITS] = [
        "tempo-hold",
        "point-blank Hold",
        "box Hold",
        "drive-at-goal",
        "end of tree",
    ];
    /// Speed at the moment, in hundredths of a unit per tick — a frozen
    /// carrier reads ~0, one oscillating in place reads high.
    pub static SPEED_X100: AtomicU64 = AtomicU64::new(0);
    pub static SAMPLES: AtomicU64 = AtomicU64::new(0);

    /// Only counts while the carrier is deep enough into the state to be
    /// what the stuck table is measuring.
    pub fn note(slot: usize, in_state_time: u64, speed: f32) {
        if in_state_time <= 250 {
            return;
        }
        EXIT[slot].fetch_add(1, Ordering::Relaxed);
        SPEED_X100.fetch_add((speed * 100.0) as u64, Ordering::Relaxed);
        SAMPLES.fetch_add(1, Ordering::Relaxed);
    }

    pub fn reset() {
        for c in EXIT.iter() {
            c.store(0, Ordering::Relaxed);
        }
        SPEED_X100.store(0, Ordering::Relaxed);
        SAMPLES.store(0, Ordering::Relaxed);
    }

    /// `(per-exit counts, mean speed in units/tick)`.
    pub fn snapshot() -> ([u64; EXITS], f32) {
        let mut out = [0u64; EXITS];
        for (i, c) in EXIT.iter().enumerate() {
            out[i] = c.load(Ordering::Relaxed);
        }
        let n = SAMPLES.load(Ordering::Relaxed).max(1);
        (
            out,
            SPEED_X100.load(Ordering::Relaxed) as f32 / n as f32 / 100.0,
        )
    }
}

#[cfg(not(feature = "match-logs"))]
pub mod stuck_exit_stats {
    #[inline(always)]
    pub fn note(_slot: usize, _in_state_time: u64, _speed: f32) {}
}

pub mod shot_gate_stats {
    use std::sync::atomic::{AtomicU64, Ordering};

    pub static HAS_BALL_IN_RANGE: AtomicU64 = AtomicU64::new(0);
    pub static PASSED_CAN_SHOOT: AtomicU64 = AtomicU64::new(0);
    pub static PASSED_SETTLED: AtomicU64 = AtomicU64::new(0);
    pub static PASSED_NOT_POSSESSION: AtomicU64 = AtomicU64::new(0);
    pub static PASSED_NOT_DEFER: AtomicU64 = AtomicU64::new(0);
    pub static PASSED_MAX_DIST: AtomicU64 = AtomicU64::new(0);
    pub static PASSED_CLEAR_SHOT: AtomicU64 = AtomicU64::new(0);
    pub static PASSED_WILLINGNESS: AtomicU64 = AtomicU64::new(0);
    pub static FIRED: AtomicU64 = AtomicU64::new(0);
    pub static HELPER_SHOOT: AtomicU64 = AtomicU64::new(0);
    pub static HELPER_PASS: AtomicU64 = AtomicU64::new(0);
    pub static HELPER_HOLD: AtomicU64 = AtomicU64::new(0);

    pub fn reset() {
        for c in [
            &HAS_BALL_IN_RANGE,
            &PASSED_CAN_SHOOT,
            &PASSED_SETTLED,
            &PASSED_NOT_POSSESSION,
            &PASSED_NOT_DEFER,
            &PASSED_MAX_DIST,
            &PASSED_CLEAR_SHOT,
            &PASSED_WILLINGNESS,
            &FIRED,
            &HELPER_SHOOT,
            &HELPER_PASS,
            &HELPER_HOLD,
        ] {
            c.store(0, Ordering::Relaxed);
        }
    }

    pub fn snapshot() -> [u64; 12] {
        [
            HAS_BALL_IN_RANGE.load(Ordering::Relaxed),
            PASSED_CAN_SHOOT.load(Ordering::Relaxed),
            PASSED_SETTLED.load(Ordering::Relaxed),
            PASSED_NOT_POSSESSION.load(Ordering::Relaxed),
            PASSED_NOT_DEFER.load(Ordering::Relaxed),
            PASSED_MAX_DIST.load(Ordering::Relaxed),
            PASSED_CLEAR_SHOT.load(Ordering::Relaxed),
            PASSED_WILLINGNESS.load(Ordering::Relaxed),
            FIRED.load(Ordering::Relaxed),
            HELPER_SHOOT.load(Ordering::Relaxed),
            HELPER_PASS.load(Ordering::Relaxed),
            HELPER_HOLD.load(Ordering::Relaxed),
        ]
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Tackle instrumentation — `match-logs` feature only. Counts:
//   * state_entries: how often a Tackling state is entered (any role)
//   * attempts:      how often attempt_*_tackle actually rolls the dice
//   * successes:     how often TacklingBall event fires (stat bumped)
//   * fouls:         CommitFoul events emitted from tackle paths
// Drop-off reveals whether we over-enter tackling states, over-attempt,
// or have an inflated success rate — each suggests a different fix.
// ───────────────────────────────────────────────────────────────────────────
#[cfg(feature = "match-logs")]
pub mod tackle_stats {
    use std::sync::atomic::{AtomicU64, Ordering};

    pub static DEF_TICKS_IN_STATE: AtomicU64 = AtomicU64::new(0);
    pub static MID_TICKS_IN_STATE: AtomicU64 = AtomicU64::new(0);
    pub static FWD_TICKS_IN_STATE: AtomicU64 = AtomicU64::new(0);
    pub static GK_TICKS_IN_STATE: AtomicU64 = AtomicU64::new(0);

    pub static DEF_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
    pub static MID_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
    pub static FWD_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
    pub static GK_ATTEMPTS: AtomicU64 = AtomicU64::new(0);

    pub static DEF_SUCCESSES: AtomicU64 = AtomicU64::new(0);
    pub static MID_SUCCESSES: AtomicU64 = AtomicU64::new(0);
    pub static FWD_SUCCESSES: AtomicU64 = AtomicU64::new(0);
    pub static GK_SUCCESSES: AtomicU64 = AtomicU64::new(0);

    pub fn reset() {
        for c in [
            &DEF_TICKS_IN_STATE,
            &MID_TICKS_IN_STATE,
            &FWD_TICKS_IN_STATE,
            &GK_TICKS_IN_STATE,
            &DEF_ATTEMPTS,
            &MID_ATTEMPTS,
            &FWD_ATTEMPTS,
            &GK_ATTEMPTS,
            &DEF_SUCCESSES,
            &MID_SUCCESSES,
            &FWD_SUCCESSES,
            &GK_SUCCESSES,
        ] {
            c.store(0, Ordering::Relaxed);
        }
    }

    pub fn snapshot() -> [u64; 12] {
        [
            DEF_TICKS_IN_STATE.load(Ordering::Relaxed),
            MID_TICKS_IN_STATE.load(Ordering::Relaxed),
            FWD_TICKS_IN_STATE.load(Ordering::Relaxed),
            GK_TICKS_IN_STATE.load(Ordering::Relaxed),
            DEF_ATTEMPTS.load(Ordering::Relaxed),
            MID_ATTEMPTS.load(Ordering::Relaxed),
            FWD_ATTEMPTS.load(Ordering::Relaxed),
            GK_ATTEMPTS.load(Ordering::Relaxed),
            DEF_SUCCESSES.load(Ordering::Relaxed),
            MID_SUCCESSES.load(Ordering::Relaxed),
            FWD_SUCCESSES.load(Ordering::Relaxed),
            GK_SUCCESSES.load(Ordering::Relaxed),
        ]
    }
}
