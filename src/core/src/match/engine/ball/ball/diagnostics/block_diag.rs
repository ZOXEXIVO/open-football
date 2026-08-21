//! Why shot blocks don't happen. `blocks` reads ~0.01 per defender per
//! match against a real ~0.9, and the counter alone cannot say whether
//! the shot never reaches the check, no defender is ever in the lane, or
//! the roll simply fails. `match-logs` only.

use std::sync::atomic::{AtomicU64, Ordering};

/// `try_block_shot` reached with a live shot in flight.
pub static SHOTS_SEEN: AtomicU64 = AtomicU64::new(0);
/// Rejected because the ball was above blocking height.
pub static TOO_HIGH: AtomicU64 = AtomicU64::new(0);
/// A defender was found inside the lane.
pub static CANDIDATES: AtomicU64 = AtomicU64::new(0);
/// The roll succeeded.
pub static FIRED: AtomicU64 = AtomicU64::new(0);

// ── Per-opponent rejection lanes ────────────────────────────────
//
// `CANDIDATES` alone says "no defender in the lane" without saying
// WHY, and the three possible causes want opposite fixes: defenders
// standing behind the ball is a positioning problem, defenders past
// the lookahead is a window problem, defenders goal-side but wide is
// a corridor-width problem. These split the rejection so the next
// reader doesn't have to re-derive it.
/// Opposition outfielders examined across all shot-ticks.
pub static OPP_SEEN: AtomicU64 = AtomicU64::new(0);
/// Rejected: level with or behind the ball along the shot line.
pub static BEHIND_BALL: AtomicU64 = AtomicU64::new(0);
/// Rejected: goal-side but further than `BLOCK_LOOKAHEAD` ahead.
pub static BEYOND_LOOKAHEAD: AtomicU64 = AtomicU64::new(0);
/// Rejected: inside the lookahead window but wider than the corridor.
pub static OUTSIDE_CORRIDOR: AtomicU64 = AtomicU64::new(0);
/// Sum of perpendicular distances for opponents inside the lookahead
/// window, x100 — divided by `IN_WINDOW` it gives the mean miss
/// distance, which is what says whether the corridor is merely too
/// narrow or the defenders are nowhere near the line.
pub static PERP_SUM_X100: AtomicU64 = AtomicU64::new(0);
/// Opponents inside the lookahead window (the `PERP_SUM_X100` denom).
pub static IN_WINDOW: AtomicU64 = AtomicU64::new(0);

// ── At the moment of the strike ─────────────────────────────────
//
// The per-tick counters above sample the whole flight, which biases
// "behind the ball" upward: a defender the ball has already passed
// counts as behind on every remaining tick. These sample ONCE, when
// the shot is struck, and answer the football question directly —
// was anybody between the shooter and the goal at all?
/// Shots struck with a projected target (one sample each).
pub static SHOTS_STRUCK: AtomicU64 = AtomicU64::new(0);
/// Opposition outfielders goal-side of the ball at the strike,
/// summed over `SHOTS_STRUCK`.
pub static GOALSIDE_AT_STRIKE: AtomicU64 = AtomicU64::new(0);
/// Of those, the ones also within 30u of the ball's line to goal —
/// i.e. actually in a position to get a body in the way.
pub static GOALSIDE_NEAR_LINE: AtomicU64 = AtomicU64::new(0);

/// Distance from the ball to the goal it is aimed at, x100, summed
/// over `SHOTS_STRUCK`. Says where shots are actually taken from.
pub static SHOT_RANGE_X100: AtomicU64 = AtomicU64::new(0);
/// Mean distance of the DEFENDING outfielders from their own goal
/// line at the strike, x100, summed over `SHOTS_STRUCK`. Read against
/// `SHOT_RANGE_X100`: if the defenders sit further out than the ball,
/// the line never dropped; if they sit closer but nobody is in the
/// lane, the line dropped and scattered.
pub static DEF_DEPTH_X100: AtomicU64 = AtomicU64::new(0);

/// Histogram of which `DefenderState` the defending back line is in
/// at the moment a shot is struck, indexed by the enum's discriminant
/// (21 variants). Without this the depth number says the line did not
/// drop but not WHY — and the answer decides whether the fix belongs
/// in a state's steering target or in the state selection above it.
pub static DEF_STATE_AT_STRIKE: [AtomicU64; 21] = [const { AtomicU64::new(0) }; 21];

/// How close the nearest defending outfielder is to the SHOOTER when
/// he strikes it, banded in metres: `<1 | 1-2 | 2-3 | 3-5 | 5-8 | 8+`.
///
/// # Why this band list and not another
///
/// The shot models read pressure through
/// `ShotInputs::pressure_count_5u` / `_10u`, and 1u is 0.125 m — so
/// those are radii of **0.62 m and 1.25 m**, i.e. a defender who is
/// physically touching the shooter. Everything from a stride away
/// outward is priced identically to an empty stadium, in both the
/// accuracy model (`ShotOutcome::pressure_penalty`) and the reported
/// xG (`XgModel::pressure_factor`). Real pressure runs out at 5-8 m,
/// which is most of a defended box.
///
/// The bands therefore straddle the model's own cut-offs: everything
/// in band 0 and part of band 1 is seen by the models, and bands 2-4
/// are the shots taken under real pressure that the engine currently
/// treats as free.
pub static NEAREST_DEF_AT_STRIKE: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

/// Diagnostic accessors. Grouped on a struct so the module exposes
/// no free functions — the statics stay module-level because Rust
/// has no associated statics.
pub struct BlockDiag;

impl BlockDiag {
    /// Book one back-line defender's state at a strike.
    pub fn note_defender_state(state_id: usize) {
        if let Some(c) = DEF_STATE_AT_STRIKE.get(state_id) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Per-state counts, in discriminant order.
    pub fn defender_state_snapshot() -> [u64; 21] {
        std::array::from_fn(|i| DEF_STATE_AT_STRIKE[i].load(Ordering::Relaxed))
    }

    /// Book the nearest defending outfielder's distance to the
    /// shooter, in UNITS. Banding lives here so the caller cannot
    /// disagree with [`NEAREST_DEF_AT_STRIKE`]'s documented edges.
    pub fn note_shot_pressure(gap_units: f32) {
        let metres = gap_units / 8.0;
        let band = if metres < 1.0 {
            0
        } else if metres < 2.0 {
            1
        } else if metres < 3.0 {
            2
        } else if metres < 5.0 {
            3
        } else if metres < 8.0 {
            4
        } else {
            5
        };
        NEAREST_DEF_AT_STRIKE[band].fetch_add(1, Ordering::Relaxed);
    }

    /// Per-band counts, closest first.
    pub fn shot_pressure_snapshot() -> [u64; 6] {
        std::array::from_fn(|i| NEAREST_DEF_AT_STRIKE[i].load(Ordering::Relaxed))
    }

    /// Sample the defensive picture at the moment a shot is struck.
    /// `goalside` / `near_line` are counts for this one strike;
    /// `shot_range` / `def_depth` are distances to the defended goal.
    pub fn note_strike(goalside: u64, near_line: u64, shot_range: f32, def_depth: f32) {
        SHOTS_STRUCK.fetch_add(1, Ordering::Relaxed);
        GOALSIDE_AT_STRIKE.fetch_add(goalside, Ordering::Relaxed);
        GOALSIDE_NEAR_LINE.fetch_add(near_line, Ordering::Relaxed);
        SHOT_RANGE_X100.fetch_add((shot_range.max(0.0) * 100.0) as u64, Ordering::Relaxed);
        DEF_DEPTH_X100.fetch_add((def_depth.max(0.0) * 100.0) as u64, Ordering::Relaxed);
    }

    /// `(shots_struck, goalside_per_shot, near_line_per_shot,
    ///   mean_shot_range, mean_defender_depth)`
    pub fn strike_snapshot() -> (u64, f32, f32, f32, f32) {
        let n = SHOTS_STRUCK.load(Ordering::Relaxed);
        if n == 0 {
            return (0, 0.0, 0.0, 0.0, 0.0);
        }
        let per = |c: &AtomicU64| c.load(Ordering::Relaxed) as f32 / 100.0 / n as f32;
        (
            n,
            GOALSIDE_AT_STRIKE.load(Ordering::Relaxed) as f32 / n as f32,
            GOALSIDE_NEAR_LINE.load(Ordering::Relaxed) as f32 / n as f32,
            per(&SHOT_RANGE_X100),
            per(&DEF_DEPTH_X100),
        )
    }

    pub fn reset() {
        for c in [
            &SHOTS_SEEN,
            &TOO_HIGH,
            &CANDIDATES,
            &FIRED,
            &OPP_SEEN,
            &BEHIND_BALL,
            &BEYOND_LOOKAHEAD,
            &OUTSIDE_CORRIDOR,
            &PERP_SUM_X100,
            &IN_WINDOW,
            &SHOTS_STRUCK,
            &GOALSIDE_AT_STRIKE,
            &GOALSIDE_NEAR_LINE,
            &SHOT_RANGE_X100,
            &DEF_DEPTH_X100,
        ] {
            c.store(0, Ordering::Relaxed);
        }
        for c in &NEAREST_DEF_AT_STRIKE {
            c.store(0, Ordering::Relaxed);
        }
        for c in &DEF_STATE_AT_STRIKE {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// `(shots_seen, too_high, candidates, fired)`
    pub fn snapshot() -> (u64, u64, u64, u64) {
        (
            SHOTS_SEEN.load(Ordering::Relaxed),
            TOO_HIGH.load(Ordering::Relaxed),
            CANDIDATES.load(Ordering::Relaxed),
            FIRED.load(Ordering::Relaxed),
        )
    }

    /// `(opp_seen, behind_ball, beyond_lookahead, outside_corridor,
    ///   in_window, mean_perp)`
    pub fn lane_snapshot() -> (u64, u64, u64, u64, u64, f32) {
        let in_window = IN_WINDOW.load(Ordering::Relaxed);
        let mean_perp = if in_window == 0 {
            0.0
        } else {
            PERP_SUM_X100.load(Ordering::Relaxed) as f32 / 100.0 / in_window as f32
        };
        (
            OPP_SEEN.load(Ordering::Relaxed),
            BEHIND_BALL.load(Ordering::Relaxed),
            BEYOND_LOOKAHEAD.load(Ordering::Relaxed),
            OUTSIDE_CORRIDOR.load(Ordering::Relaxed),
            in_window,
            mean_perp,
        )
    }
}
