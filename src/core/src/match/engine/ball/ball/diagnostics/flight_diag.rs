//! Where the ball actually goes, and which line of code sent it there.
//!
//! Two symptoms motivated this and neither is visible in any existing
//! counter: balls that climb absurdly high, and balls that arrive
//! somewhere far away in a single tick without having travelled. Both
//! are silent — the physics never complains, the stat sheet is
//! unaffected, and only somebody watching the 3D replay sees it.
//!
//! # Why a launch census rather than a height histogram
//!
//! The vertical axis is in METRES while `x`/`y` are in game units (see
//! [`GRAVITY_PER_TICK`]). A hand-written `z` therefore reads as a
//! perfectly sane number and means something absurd: `4.5` looks like a
//! firm hoof and is a 10 km apex. Sampling `position.z` per tick would
//! mostly measure how long the ball spends on the deck; sampling the
//! APEX IMPLIED AT LAUNCH names the offending kick directly, which is
//! what a fix needs.
//!
//! # Why teleports are attributed per stage
//!
//! `Ball::update` runs seventeen passes over the ball and six of them
//! can move it without touching the velocity. "The ball jumped" is not
//! actionable; "the ball jumped 91u inside `try_block_shot`" is.

use nalgebra::Vector3;
use std::sync::atomic::{AtomicU64, Ordering};

/// Apex bands in metres. A real football tops out around 30 m from
/// the most violent hoof, so everything from `Absurd` up is a bug by
/// construction — the bands exist to say HOW absurd, because the
/// error is a unit confusion and the magnitude identifies which one.
pub const APEX_BANDS: [f32; 8] = [1.0, 3.0, 6.0, 12.0, 30.0, 100.0, 1000.0, f32::INFINITY];
pub const APEX_LABELS: [&str; 8] = [
    "<1m", "1-3m", "3-6m", "6-12m", "12-30m", "30-100m", "0.1-1km", ">1km",
];

/// One counter per band, over every launch the ball takes.
pub static APEX_HIST: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
/// Highest apex any single launch implied, x1000 (millimetres).
pub static APEX_MAX_MM: AtomicU64 = AtomicU64::new(0);
/// Launches seen.
pub static LAUNCHES: AtomicU64 = AtomicU64::new(0);
/// Launches above 30 m — impossible for a human — split by the state
/// the striker was in. Sized past the state machine's 78 variants so
/// a newly added state cannot silently fall off the end.
pub static ABSURD_BY_STATE: [AtomicU64; 96] = [const { AtomicU64::new(0) }; 96];

/// Highest the ball's own `position.z` ever actually reached, in mm.
/// Read against `APEX_MAX_MM`: a launch apex that never materialises
/// means something cut the flight short (a boundary clamp, a claim),
/// which is its own bug.
pub static PEAK_Z_MM: AtomicU64 = AtomicU64::new(0);

/// Fastest horizontal speed observed on an unowned ball, x1000.
pub static PEAK_SPEED_X1000: AtomicU64 = AtomicU64::new(0);

/// The passes of `Ball::update` that can relocate the ball, in the
/// order they run.
///
/// The four endline / touchline resolvers are listed separately from
/// `boundary` on purpose. They were folded into it at first and the
/// bucket read 64 unexplained relocations a match with a worst case
/// of 53 m, which looks exactly like a bug and is not one: a throw-in
/// puts the ball on the touchline and a goal kick puts it in the six-
/// yard box, and both are supposed to move it a long way. Only the
/// LAST entry is a genuine "nothing decided this" clamp.
pub const STAGES: [&str; 12] = [
    "intercept",
    "block_shot",
    "save_shot",
    "deadlock_claim",
    "position_stall",
    "ownership",
    "move_to",
    "restart:goal",
    "restart:over_bar",
    "restart:wide",
    "restart:throw_in",
    "boundary_clamp",
];
pub const STAGE_INTERCEPT: usize = 0;
pub const STAGE_BLOCK: usize = 1;
pub const STAGE_SAVE: usize = 2;
pub const STAGE_DEADLOCK: usize = 3;
pub const STAGE_STALL: usize = 4;
pub const STAGE_OWNERSHIP: usize = 5;
pub const STAGE_MOVE: usize = 6;
pub const STAGE_GOAL: usize = 7;
pub const STAGE_OVER_BAR: usize = 8;
pub const STAGE_WIDE: usize = 9;
pub const STAGE_THROW_IN: usize = 10;
pub const STAGE_BOUNDARY: usize = 11;

/// The stages above that are RESTARTS — moving the ball is their job,
/// so their relocations are reported apart from the unexplained ones.
pub const RESTART_STAGES: std::ops::Range<usize> = STAGE_GOAL..STAGE_BOUNDARY;

/// Horizontal jumps a stage produced that its own velocity cannot
/// explain, and their summed / worst magnitude in game units.
pub static JUMPS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
pub static JUMP_SUM_X100: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
pub static JUMP_MAX_X100: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
/// Fastest the ball was travelling as each stage left it, x1000.
/// `PEAK_SPEED_X1000` says a runaway speed exists; this says which
/// pass over the ball put it there.
pub static STAGE_PEAK_SPEED_X1000: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];

/// Height (mm) at which an interception fired, summed, and how many
/// fired above a standing player's reach — the ones that need a leap
/// and never get one.
pub static INTERCEPTS: AtomicU64 = AtomicU64::new(0);
pub static INTERCEPT_Z_SUM_MM: AtomicU64 = AtomicU64::new(0);
pub static INTERCEPTS_ABOVE_REACH: AtomicU64 = AtomicU64::new(0);
/// Interceptions taken with both feet on the floor at a height that
/// needs a jump. `INTERCEPTS_ABOVE_REACH` minus this is the number
/// that were properly won in the air.
pub static INTERCEPTS_NO_LEAP: AtomicU64 = AtomicU64::new(0);

/// Headers contested, and how many of those were won by a player who
/// actually left the ground.
pub static HEADERS: AtomicU64 = AtomicU64::new(0);
pub static HEADERS_AIRBORNE: AtomicU64 = AtomicU64::new(0);

/// Accessors. Grouped on a struct so the module exposes no free
/// functions; the statics stay module-level because Rust has no
/// associated statics.
pub struct FlightDiag;

impl FlightDiag {
    /// Record a kick: `vz` is the launch speed in m/tick, `z` the
    /// height it was struck from. `striker_state` is the compact id
    /// of the state the player who last had the ball was in, which is
    /// what names the offending site when an apex comes back absurd —
    /// a bare count says a bug exists but not which kick wrote it.
    pub fn note_launch(vz: f32, z: f32, striker_state: Option<usize>) {
        let apex = crate::r#match::engine::ball::ball::Ball::apex_for_launch(vz) + z.max(0.0);
        LAUNCHES.fetch_add(1, Ordering::Relaxed);
        let band = APEX_BANDS.iter().position(|&b| apex < b).unwrap_or(7);
        APEX_HIST[band].fetch_add(1, Ordering::Relaxed);
        APEX_MAX_MM.fetch_max((apex.max(0.0) * 1000.0) as u64, Ordering::Relaxed);
        // Only the absurd ones are worth attributing; a histogram over
        // every launch in the match would be all noise.
        if apex > 30.0 {
            if let Some(id) = striker_state {
                if let Some(c) = ABSURD_BY_STATE.get(id) {
                    c.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Per-state count of launches above 30 m, indexed by
    /// `PlayerState::compact_id`.
    pub fn absurd_by_state() -> [u64; 96] {
        std::array::from_fn(|i| ABSURD_BY_STATE[i].load(Ordering::Relaxed))
    }

    /// Sample the ball's realised flight once per tick.
    pub fn note_tick(position: Vector3<f32>, velocity: Vector3<f32>, owned: bool) {
        PEAK_Z_MM.fetch_max((position.z.max(0.0) * 1000.0) as u64, Ordering::Relaxed);
        if !owned {
            let speed = (velocity.x * velocity.x + velocity.y * velocity.y).sqrt();
            PEAK_SPEED_X1000.fetch_max((speed.max(0.0) * 1000.0) as u64, Ordering::Relaxed);
        }
    }

    /// Book a relocation `stage` produced beyond what its velocity
    /// accounts for.
    pub fn note_jump(stage: usize, distance: f32) {
        if stage >= STAGES.len() {
            return;
        }
        JUMPS[stage].fetch_add(1, Ordering::Relaxed);
        let x100 = (distance.max(0.0) * 100.0) as u64;
        JUMP_SUM_X100[stage].fetch_add(x100, Ordering::Relaxed);
        JUMP_MAX_X100[stage].fetch_max(x100, Ordering::Relaxed);
    }

    /// Book an interception at `z` metres, by a player `airborne` or
    /// not. `reach` is a standing player's ceiling.
    pub fn note_intercept(z: f32, reach: f32, airborne: bool) {
        INTERCEPTS.fetch_add(1, Ordering::Relaxed);
        INTERCEPT_Z_SUM_MM.fetch_add((z.max(0.0) * 1000.0) as u64, Ordering::Relaxed);
        if z > reach {
            INTERCEPTS_ABOVE_REACH.fetch_add(1, Ordering::Relaxed);
            if !airborne {
                INTERCEPTS_NO_LEAP.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn note_header(airborne: bool) {
        HEADERS.fetch_add(1, Ordering::Relaxed);
        if airborne {
            HEADERS_AIRBORNE.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `(launches, apex_hist, apex_max_m, peak_z_m, peak_speed)`
    pub fn launch_snapshot() -> (u64, [u64; 8], f32, f32, f32) {
        (
            LAUNCHES.load(Ordering::Relaxed),
            std::array::from_fn(|i| APEX_HIST[i].load(Ordering::Relaxed)),
            APEX_MAX_MM.load(Ordering::Relaxed) as f32 / 1000.0,
            PEAK_Z_MM.load(Ordering::Relaxed) as f32 / 1000.0,
            PEAK_SPEED_X1000.load(Ordering::Relaxed) as f32 / 1000.0,
        )
    }

    /// Per-stage `(count, mean_units, max_units, peak_speed)`.
    pub fn jump_snapshot() -> [(u64, f32, f32, f32); 12] {
        std::array::from_fn(|i| {
            let n = JUMPS[i].load(Ordering::Relaxed);
            let mean = if n == 0 {
                0.0
            } else {
                JUMP_SUM_X100[i].load(Ordering::Relaxed) as f32 / 100.0 / n as f32
            };
            (
                n,
                mean,
                JUMP_MAX_X100[i].load(Ordering::Relaxed) as f32 / 100.0,
                STAGE_PEAK_SPEED_X1000[i].load(Ordering::Relaxed) as f32 / 1000.0,
            )
        })
    }

    /// `(intercepts, mean_z_m, above_reach, above_reach_no_leap,
    ///   headers, headers_airborne)`
    pub fn aerial_snapshot() -> (u64, f32, u64, u64, u64, u64) {
        let n = INTERCEPTS.load(Ordering::Relaxed);
        let mean = if n == 0 {
            0.0
        } else {
            INTERCEPT_Z_SUM_MM.load(Ordering::Relaxed) as f32 / 1000.0 / n as f32
        };
        (
            n,
            mean,
            INTERCEPTS_ABOVE_REACH.load(Ordering::Relaxed),
            INTERCEPTS_NO_LEAP.load(Ordering::Relaxed),
            HEADERS.load(Ordering::Relaxed),
            HEADERS_AIRBORNE.load(Ordering::Relaxed),
        )
    }

    pub fn reset() {
        for c in APEX_HIST.iter() {
            c.store(0, Ordering::Relaxed);
        }
        for c in ABSURD_BY_STATE.iter() {
            c.store(0, Ordering::Relaxed);
        }
        for i in 0..STAGES.len() {
            JUMPS[i].store(0, Ordering::Relaxed);
            JUMP_SUM_X100[i].store(0, Ordering::Relaxed);
            JUMP_MAX_X100[i].store(0, Ordering::Relaxed);
            STAGE_PEAK_SPEED_X1000[i].store(0, Ordering::Relaxed);
        }
        for c in [
            &APEX_MAX_MM,
            &LAUNCHES,
            &PEAK_Z_MM,
            &PEAK_SPEED_X1000,
            &INTERCEPTS,
            &INTERCEPT_Z_SUM_MM,
            &INTERCEPTS_ABOVE_REACH,
            &INTERCEPTS_NO_LEAP,
            &HEADERS,
            &HEADERS_AIRBORNE,
        ] {
            c.store(0, Ordering::Relaxed);
        }
    }
}

/// Walks `Ball::update` alongside the ball, comparing where each pass
/// left it against where its velocity said it should be.
pub struct StageProbe {
    position: Vector3<f32>,
}

impl StageProbe {
    pub fn new(position: Vector3<f32>) -> Self {
        Self { position }
    }

    /// Book whatever `stage` did. `allowance` is the horizontal
    /// distance the stage was entitled to move the ball — its own
    /// velocity for `move_to`, nothing for a pass that is only
    /// supposed to change ownership.
    pub fn note(
        &mut self,
        stage: usize,
        position: Vector3<f32>,
        velocity: Vector3<f32>,
        allowance: f32,
    ) {
        let dx = position.x - self.position.x;
        let dy = position.y - self.position.y;
        let moved = (dx * dx + dy * dy).sqrt();
        // 1u (12.5 cm) of slack absorbs the sub-unit nudges several
        // passes legitimately apply.
        if moved > allowance + 1.0 {
            FlightDiag::note_jump(stage, moved - allowance);
        }
        if stage < STAGES.len() {
            let speed = (velocity.x * velocity.x + velocity.y * velocity.y).sqrt();
            STAGE_PEAK_SPEED_X1000[stage]
                .fetch_max((speed.max(0.0) * 1000.0) as u64, Ordering::Relaxed);
        }
        self.position = position;
    }
}
