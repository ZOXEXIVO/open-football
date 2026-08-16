//! Match-engine ball model, split by concern. The `Ball` struct lives
//! here together with the per-tick orchestrator (`update` / `update_light`)
//! and the simple state queries the rest of the engine reads. The
//! heavier domain passes are sibling modules:
//!
//! | Submodule       | Concern                                                      |
//! |-----------------|--------------------------------------------------------------|
//! | [`ownership`]   | Pass-target claims, deadlock resolution, stall safety nets, ball-ownership claim flow |
//! | [`interactions`]| Intercept / shot-block / shot-save resolution                |
//! | [`goal`]        | Goal / over-the-bar / wide-of-goal handling                  |
//! | [`net`]         | What the ball does after it crosses the line                 |
//! | [`motion`]      | Velocity integration, owner tracking, boundary inset         |
//! | [`stall`]       | Position-anchor stall detector + snapshot diagnostics        |

mod goal;
pub mod interactions;
// `pub` for `GoalNet` / `BallInNet` — the celebration choreography in the
// flow layer reads the goal geometry to send a keeper in after the ball,
// and the replay viewer needs the same net depth to draw it.
pub mod net;
// `pub` for `SpinModel` — the strike sites (shot / cross) solve the
// rotation they need from the same Magnus coefficient the physics
// integrates, so the two can never drift apart.
pub mod motion;
pub mod ownership;
mod restart;
// `pub` for `dead_ball_diag` — the stall attribution counters are read by
// the dev harness, same as `ownership::reception_diag`.
pub mod stall;

use crate::r#match::engine::ball::ball::net::BallInNet;
use crate::r#match::engine::ball::events::BallEvent;
use crate::r#match::engine::set_pieces::CornerRoutine;
use crate::r#match::events::EventCollection;
use crate::r#match::player::strategies::passing::CrossType;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer, PlayerSide};
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::{CrossDiag, PassWeightCensus};
use nalgebra::Vector3;
use std::collections::VecDeque;

/// Origin of the most recent live pass / restart. Read by the offside
/// resolver: only goal kicks, throw-ins, and corners are exempt from
/// offside; free kicks (direct/indirect) and penalties are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassOriginRestart {
    OpenPlay,
    GoalKick,
    Corner,
    ThrowIn,
    /// Generic free kick (legacy / offside fallback). Treated like a
    /// direct free kick by the offside resolver.
    FreeKick,
    /// Foul outside the penalty area, severity Normal+: ball can be shot
    /// at goal directly.
    DirectFreeKick,
    /// Offside or technical infringement: cannot be shot directly into
    /// goal — needs a touch from a second player first.
    IndirectFreeKick,
    /// Foul inside defending penalty area: ball at penalty spot.
    Penalty,
}

impl Default for PassOriginRestart {
    fn default() -> Self {
        PassOriginRestart::OpenPlay
    }
}

impl PassOriginRestart {
    /// Set-piece restarts that exempt the receiver from offside.
    pub fn is_offside_exempt(self) -> bool {
        matches!(
            self,
            PassOriginRestart::GoalKick | PassOriginRestart::Corner | PassOriginRestart::ThrowIn
        )
    }

    /// True for any free-kick-style restart (direct/indirect/legacy).
    /// Penalties and corners are NOT free kicks for routine selection.
    pub fn is_free_kick(self) -> bool {
        matches!(
            self,
            PassOriginRestart::FreeKick
                | PassOriginRestart::DirectFreeKick
                | PassOriginRestart::IndirectFreeKick
        )
    }
}

/// Snapshot of the offside-relevant geometry at the moment a pass is
/// kicked. Stored on the ball for the duration of an in-flight pass so
/// the offside check can fire on receiver involvement (touch / claim /
/// active challenge) instead of at pass start.
#[derive(Debug, Clone, Copy)]
pub struct OffsideSnapshot {
    pub origin: PassOriginRestart,
    pub passer_id: u32,
    pub passer_side: PlayerSide,
    pub receiver_id: u32,
    pub ball_x_at_kick: f32,
    pub second_last_defender_x: f32,
    pub receiver_x_at_kick: f32,
    pub receiver_y_at_kick: f32,
    pub set_tick: u64,
}

impl OffsideSnapshot {
    /// Decide whether the snapshot represents an offside position.
    /// Tolerance 1.5u absorbs foot-vs-shoulder ambiguity.
    pub fn is_offside(&self) -> bool {
        const TOLERANCE: f32 = 1.5;
        match self.passer_side {
            PlayerSide::Left => {
                if self.receiver_x_at_kick <= self.ball_x_at_kick + TOLERANCE {
                    return false;
                }
                self.receiver_x_at_kick > self.second_last_defender_x + TOLERANCE
            }
            PlayerSide::Right => {
                if self.receiver_x_at_kick >= self.ball_x_at_kick - TOLERANCE {
                    return false;
                }
                self.receiver_x_at_kick < self.second_last_defender_x - TOLERANCE
            }
        }
    }
}

/// Why a goal did or didn't carry an assist. The credited-assist rate is
/// a headline realism number (real football assists ~70% of goals), and
/// the count alone can't say whether the resolver is too strict or the
/// engine simply isn't scoring off passes. These split the outcomes at
/// the one decision point that knows: `assist_for_goal`.
#[cfg(feature = "match-logs")]
pub mod assist_diag {
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Non-own goals that reached the resolver.
    pub static GOALS: AtomicU64 = AtomicU64::new(0);
    /// Pass chain was empty — nothing was recorded, or a clear wiped it.
    pub static EMPTY_CHAIN: AtomicU64 = AtomicU64::new(0);
    /// Newest chain entry belongs to the conceding team: the scoring team
    /// won the ball and finished without completing a pass of its own.
    pub static OPPONENT_CHAIN: AtomicU64 = AtomicU64::new(0);
    /// Of those, how many still had a scoring-team pass deeper in the
    /// ring — i.e. the same-possession rule is what rejected them, not
    /// the absence of a teammate's pass.
    pub static OPPONENT_CHAIN_HAS_TEAMMATE: AtomicU64 = AtomicU64::new(0);
    /// Age in ticks of the blocking opponent entry, summed.
    pub static OPPONENT_CHAIN_AGE: AtomicU64 = AtomicU64::new(0);
    /// Only the scorer appears in the chain (they passed, got it back).
    pub static SCORER_ONLY: AtomicU64 = AtomicU64::new(0);
    /// A teammate's pass was there but older than `ASSIST_WINDOW_TICKS`.
    pub static STALE: AtomicU64 = AtomicU64::new(0);
    pub static CREDITED: AtomicU64 = AtomicU64::new(0);
    /// Sum of (goal tick − assist pass tick) over credited assists, so
    /// the harness can print the mean delay and size the window.
    pub static CREDITED_DELAY_TICKS: AtomicU64 = AtomicU64::new(0);

    pub fn reset() {
        for c in [
            &GOALS,
            &EMPTY_CHAIN,
            &OPPONENT_CHAIN,
            &OPPONENT_CHAIN_HAS_TEAMMATE,
            &OPPONENT_CHAIN_AGE,
            &SCORER_ONLY,
            &STALE,
            &CREDITED,
            &CREDITED_DELAY_TICKS,
        ] {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// `(goals, empty, opponent, scorer_only, stale, credited, delay_sum)`
    pub fn snapshot() -> (u64, u64, u64, u64, u64, u64, u64) {
        (
            GOALS.load(Ordering::Relaxed),
            EMPTY_CHAIN.load(Ordering::Relaxed),
            OPPONENT_CHAIN.load(Ordering::Relaxed),
            SCORER_ONLY.load(Ordering::Relaxed),
            STALE.load(Ordering::Relaxed),
            CREDITED.load(Ordering::Relaxed),
            CREDITED_DELAY_TICKS.load(Ordering::Relaxed),
        )
    }

    /// `(opponent_chain_with_teammate_deeper, opponent_entry_age_sum)`
    pub fn opponent_chain_detail() -> (u64, u64) {
        (
            OPPONENT_CHAIN_HAS_TEAMMATE.load(Ordering::Relaxed),
            OPPONENT_CHAIN_AGE.load(Ordering::Relaxed),
        )
    }
}

/// Where the ball actually goes, and which line of code sent it there.
///
/// Two symptoms motivated this and neither is visible in any existing
/// counter: balls that climb absurdly high, and balls that arrive
/// somewhere far away in a single tick without having travelled. Both
/// are silent — the physics never complains, the stat sheet is
/// unaffected, and only somebody watching the 3D replay sees it.
///
/// # Why a launch census rather than a height histogram
///
/// The vertical axis is in METRES while `x`/`y` are in game units (see
/// [`GRAVITY_PER_TICK`]). A hand-written `z` therefore reads as a
/// perfectly sane number and means something absurd: `4.5` looks like a
/// firm hoof and is a 10 km apex. Sampling `position.z` per tick would
/// mostly measure how long the ball spends on the deck; sampling the
/// APEX IMPLIED AT LAUNCH names the offending kick directly, which is
/// what a fix needs.
///
/// # Why teleports are attributed per stage
///
/// `Ball::update` runs seventeen passes over the ball and six of them
/// can move it without touching the velocity. "The ball jumped" is not
/// actionable; "the ball jumped 91u inside `try_block_shot`" is.
#[cfg(feature = "match-logs")]
pub mod flight_diag {
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
            let apex = super::Ball::apex_for_launch(vz) + z.max(0.0);
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
}

/// Per-tick rolling-friction decay for a ball on the ground: each tick
/// its horizontal speed is multiplied by `1 - GROUND_FRICTION`.
///
/// Derived from the real figure rather than fitted: a football on grass
/// loses roughly **15% of its speed per second**. At 100 ticks to the
/// second that is `k^100 = 0.85`, so `k = 0.85^(1/100) = 0.998375` and
/// the coefficient is 0.001625.
///
/// It was 0.006 — a 45%/s loss, ~3.7× real. That single number is why
/// `calculate_horizontal_velocity` had to aim every pass 79-157% BEYOND
/// its target (the old `overshoot` table): with the ball dying that fast,
/// a pass weighted to arrive at its man arrived at walking pace or not at
/// all, so the code compensated by hitting it 5-12 m too far. Both halves
/// are fixed together; neither works alone.
///
/// Shared so the physics and the pass-weighting can never disagree again
/// — they were separate literals in `motion.rs` and `players.rs`.
pub const GROUND_FRICTION: f32 = 0.0016;

/// Downward acceleration applied to an airborne ball, in **m/tick²**.
///
/// # The ball's vertical axis is in METRES
///
/// `x` and `y` are in game units (1u = 0.125 m); `z` is in metres. The
/// engine has always said so — `GOAL_HEIGHT` is annotated "crossbar height
/// in meters (z-axis is in meters)", and every reach threshold in the
/// engine (`PLAYER_JUMP_REACH` 3.5, `is_aerial` 2.3, the receiver ceiling
/// 2.8, the heading band 1.4-2.5) is a sane figure in metres and a
/// nonsense one in units. What did NOT honour the convention was the
/// motion: gravity and the launch velocities were written in units, so a
/// ball climbing to "4.0" was climbing four metres' worth of threshold at
/// four units' worth of speed.
///
/// This constant is the reconciliation. At 10 ms a tick,
/// `9.81 m/s² × (0.01 s)² = 9.81e-4 m/tick²`. It replaces `9.81 * 0.016`
/// (= 0.157), which was 160× too strong in metres — the ball fell like a
/// stone, so nothing could hang, so the pass solver had to fire lofted
/// balls at 85 m/s to get them anywhere, and clearances and shots were
/// each hand-fitted to that in their own units.
///
/// Consequences, all of them wanted: hang times become real (a 30 m cross
/// hangs ~2.3 s instead of ~0.5 s), lofted passes come back inside normal
/// pass speeds, and every height threshold in the engine starts meaning
/// what it says.
///
/// Every site that integrates or inverts vertical motion MUST read this
/// (or the helpers below) rather than carry its own literal — the physics,
/// the landing projection, the pass solver, the shot arc, the clearance
/// and the cross-chase all used to hold private copies of `9.81`-something
/// in three different unit systems.
pub const GRAVITY_PER_TICK: f32 = 9.81 * 0.01 * 0.01;

impl Ball {
    /// Vertical launch speed (m/tick) that peaks at `apex` metres.
    ///
    /// Apex is the natural way to ask for a trajectory: it is the one
    /// property of a kick a player actually aims at ("clip it over him",
    /// "put it on his head", "row Z"), it reads in metres so it can be
    /// sanity-checked against a human being, and it is unit-clean — the
    /// alternative, a launch angle, cannot be expressed at all when the
    /// horizontal and vertical axes carry different units.
    #[inline]
    pub fn launch_speed_for_apex(apex_metres: f32) -> f32 {
        (2.0 * GRAVITY_PER_TICK * apex_metres.max(0.0)).sqrt()
    }

    /// How long a ball launched at `vertical_speed` (m/tick) stays up, in
    /// ticks, before returning to the height it left from.
    #[inline]
    pub fn hang_ticks(vertical_speed: f32) -> f32 {
        2.0 * vertical_speed.max(0.0) / GRAVITY_PER_TICK
    }

    /// Peak height in metres of a ball launched at `vertical_speed`.
    #[inline]
    pub fn apex_for_launch(vertical_speed: f32) -> f32 {
        vertical_speed * vertical_speed / (2.0 * GRAVITY_PER_TICK)
    }
}

/// How high a footballer can play the ball, and what it costs him to do
/// it. All heights in metres, matching the ball's vertical axis (see
/// [`GRAVITY_PER_TICK`]).
///
/// # Why this is one model rather than a constant per call site
///
/// Every aerial decision in the engine used to carry its own literal —
/// `2.5` in the intercept gate, `3.5` in the claim loop, `2.8` for a pass
/// receiver, `1.5` to enter a header — and none of them agreed with any
/// other or with a human being. Worse, all of them were BINARY: below the
/// number the ball was as easy to play as one rolling along the floor,
/// above it the ball did not exist. A binary gate is what produces the
/// two symptoms that look opposite and share a cause — a defender picking
/// a ball out of the air at shoulder height without moving, and nobody at
/// all going for one a few centimetres higher.
///
/// Height is a difficulty, not a door. [`Self::reach_difficulty`] is the
/// curve; [`Self::ceiling`] is the only genuine door, and it is a
/// property of the player rather than of the engine.
pub struct AerialReach;

impl AerialReach {
    /// Head height of an average player.
    pub const HEAD: f32 = 1.8;

    /// The highest a player can play the ball with both feet on the
    /// floor: a raised boot, a stretched neck, a chest-high volley.
    /// Above this he has to leave the ground, and if he does not, he
    /// should not be getting the ball.
    pub const STANDING: f32 = 2.2;

    /// Ball height a poor leaper reaches at the top of a jump.
    const JUMP_MIN: f32 = 2.5;
    /// Ball height an elite leaper reaches at the top of a jump. Real
    /// aerial specialists head the ball around 2.9-3.0 m.
    const JUMP_MAX: f32 = 3.1;

    /// The highest ball this player can play, given his `jumping`
    /// attribute on the raw 1-20 scale.
    #[inline]
    pub fn ceiling(jumping: f32) -> f32 {
        let spring = ((jumping - 1.0) / 19.0).clamp(0.0, 1.0);
        Self::JUMP_MIN + spring * (Self::JUMP_MAX - Self::JUMP_MIN)
    }

    /// True when the ball is high enough that playing it means leaving
    /// the ground.
    #[inline]
    pub fn needs_leap(ball_z: f32) -> bool {
        ball_z > Self::STANDING
    }

    /// How much of his usual chance a player keeps at this ball height,
    /// 1.0 on the deck falling to 0 at his own ceiling.
    ///
    /// Squared rather than linear because the hard part of an aerial ball
    /// is the last few centimetres: a ball at knee height and one at
    /// chest height are both simply *there*, while one at the very top of
    /// the jump is a fingertip touch that mostly does not come off.
    #[inline]
    pub fn reach_difficulty(ball_z: f32, jumping: f32) -> f32 {
        if ball_z <= Self::HEAD {
            return 1.0;
        }
        let ceiling = Self::ceiling(jumping);
        if ball_z >= ceiling {
            return 0.0;
        }
        let over = (ball_z - Self::HEAD) / (ceiling - Self::HEAD);
        (1.0 - over * over).clamp(0.0, 1.0)
    }

    /// Apex, in metres, of the jump this player must make to meet a ball
    /// at `ball_z` with whatever he plays it with — a boot, a knee, a
    /// shoulder. Zero when he can reach it standing.
    ///
    /// He jumps to bring his own reach up to the ball and no further —
    /// an aerial challenge is timed, not maximal, and a player who
    /// launched himself to his ceiling for every ball above his head
    /// would spend the match in orbit.
    #[inline]
    pub fn leap_for(ball_z: f32, jumping: f32) -> f32 {
        Self::leap_from(ball_z, jumping, Self::STANDING)
    }

    /// The same, for a ball he is going to HEAD.
    ///
    /// A header is played off the forehead, not off a raised boot, so it
    /// is measured from [`Self::HEAD`] — 40 cm lower than
    /// [`Self::STANDING`]. Using the standing reach here is what would
    /// keep a player flat-footed for every header between 1.8 m and
    /// 2.2 m, which is most of them.
    #[inline]
    pub fn header_leap_for(ball_z: f32, jumping: f32) -> f32 {
        Self::leap_from(ball_z, jumping, Self::HEAD)
    }

    #[inline]
    fn leap_from(ball_z: f32, jumping: f32, reach: f32) -> f32 {
        if ball_z <= reach {
            return 0.0;
        }
        let ceiling = Self::ceiling(jumping);
        (ball_z - reach).min((ceiling - reach).max(0.0)).max(0.0)
    }
}

/// How close a player must be to the ball to take control of it, in game
/// units (1u = 0.125 m, so this is 1.5 m — one stride, a real first-touch
/// distance).
///
/// This MUST stay at or below [`MAX_OWNER_TRACK_DISTANCE`]. The two used
/// to be independent numbers that disagreed by a factor of six: the
/// pass-target claim granted ownership at 100u while `Ball::move_to`
/// refused to track the ball to an owner beyond 15u and dropped the
/// ownership again. The effect was that a pass was booked COMPLETED on
/// the first tick of its flight — the receiver is within 100u of the
/// ball the moment it leaves the passer's foot — and then instantly
/// released, so the ball flew its whole course as a loose ball with no
/// owner and no intended receiver (the claim had already consumed
/// `pass_target_player_id`). Measured: 100% of receptions landed beyond
/// the tracking cap, `move_to` dropped ownership 5.4k times a match, and
/// 86% of all shots were struck off loose balls against a real ~15%.
/// Pass accuracy read 87% the whole time — the metric counted claims,
/// not deliveries.
pub const CONTROL_DISTANCE: f32 = 12.0;

/// Hard cap on how far the ball will track to its owner before ownership
/// is treated as impossible and dropped (1.9 m). See [`CONTROL_DISTANCE`].
pub const MAX_OWNER_TRACK_DISTANCE: f32 = 15.0;

/// How close the ball has to be for a player to kick it (1.9 m — within
/// reach at a stretch, which is what makes a first-time pass legal).
///
/// `PlayerEvent::PassTo` had no such check: any player in a passing state
/// rewrote the ball's velocity from anywhere on the pitch, whether or not
/// they had the ball. 59% of all passes were emitted on top of a pass
/// that was still in the air, which is why the engine recorded ~1150
/// passes a team against a real ~500 — the surplus was players kicking a
/// ball that was 40 m away, and each one destroyed the pass already in
/// flight.
pub const KICKABLE_DISTANCE: f32 = MAX_OWNER_TRACK_DISTANCE;

/// How long a pass stays assist-eligible, in ticks (100 ticks ≈ 1 s).
///
/// An assist is the pass that *led to* the goal, so the two have to be
/// close together. 6 s covers the slowest legitimate chain the engine
/// produces — a long ball is ~3 s of flight, plus a touch and a strike —
/// while excluding the case that used to dominate the charts: a goal
/// kick counted as the assist for a solo run that ended half a minute
/// later. The same-possession rule in `assist_for_goal` does most of the
/// work; this is the backstop for a phase that never changes hands.
pub const ASSIST_WINDOW_TICKS: u64 = 600;

/// How the current ball carrier came by the ball.
///
/// Stamped at the event-dispatch choke point (every acquisition emits
/// exactly one ball event), so it stays correct without threading a
/// reason through the ~20 sites that assign `current_owner`. Read at
/// shot time by `shot_supply_diag`: in real football roughly 55-60% of
/// shots are struck by the player who was just passed to, and this is
/// the counter that says whether the engine feeds its shooters or lets
/// them scavenge.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PossessionSource {
    /// No acquisition recorded since the last restart.
    Unknown,
    /// Received a teammate's pass — the one that should dominate.
    PassReception,
    /// Won an uncontrolled ball: rebound, spill, deflection, failed
    /// first touch, or a clearance that dropped to them.
    LooseBall,
    /// Picked off an opponent's pass.
    Interception,
    /// Took it off an opponent in a challenge.
    Tackle,
}

impl PossessionSource {
    pub const COUNT: usize = 5;

    pub fn index(self) -> usize {
        match self {
            PossessionSource::Unknown => 0,
            PossessionSource::PassReception => 1,
            PossessionSource::LooseBall => 2,
            PossessionSource::Interception => 3,
            PossessionSource::Tackle => 4,
        }
    }

    pub const NAMES: [&'static str; Self::COUNT] =
        ["unknown", "pass", "loose", "intercept", "tackle"];
}

/// One kick in the current possession's pass chain.
///
/// The chain used to be a bare `VecDeque<u32>` of player ids, which is
/// enough for the AI heuristics that read it (one-two detection, the
/// "don't pass straight back" recency penalty) but not for crediting an
/// assist. An assist has to answer three questions a lone id cannot:
/// is the passer a TEAMMATE of the scorer, was the pass in the SAME
/// possession phase, and was it RECENT. Carrying the team and the tick
/// on every entry answers all three at the point of use.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PassChainEntry {
    pub player_id: u32,
    pub team_id: u32,
    pub tick: u64,
}

pub struct Ball {
    pub start_position: Vector3<f32>,
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    /// Angular velocity in rad/tick. Set at strike time from where on the
    /// ball the player's foot met it, integrated as a Magnus force while
    /// airborne, and scrubbed off on contact with the turf or a player.
    /// This is the only channel in the engine that can turn a flight
    /// sideways — see [`SpinModel`](super::ball::SpinModel).
    pub spin: Vector3<f32>,
    /// `velocity.z` as this tick's physics left it, so the next tick can
    /// tell a KICK from the rest of the flight. Anything that raises the
    /// vertical speed between two `update` calls came from outside the
    /// physics — a clearance, a punch, a shot — and is the only event the
    /// apex census wants to count. Diagnostic only.
    #[cfg(feature = "match-logs")]
    pub settled_vz: f32,
    pub center_field_position: f32,

    pub field_width: f32,
    pub field_height: f32,

    pub flags: BallFlags,

    pub previous_owner: Option<u32>,
    pub current_owner: Option<u32>,
    pub take_ball_notified_players: Vec<u32>,
    pub notification_cooldown: u32,
    pub notification_timeout: u32,
    pub last_boundary_position: Option<Vector3<f32>>,
    pub unowned_stopped_ticks: u32,
    pub ownership_duration: u32,
    pub claim_cooldown: u32,
    pub pass_target_player_id: Option<u32>,
    /// Passer id of the most-recent live pass. Set on pass emit,
    /// cleared on any opponent touch or when the pass's natural
    /// window (150 ticks ≈ 1.5 s) expires. The pass-completion stat
    /// uses this as the source of truth for "was this claim a pass
    /// reception?" — `pass_target_player_id` gets cleared in too
    /// many unrelated paths to serve that role. None outside an
    /// active pass window.
    pub pending_pass_passer: Option<u32>,
    pub pending_pass_set_tick: u64,
    pub recent_passers: VecDeque<PassChainEntry>,
    /// How `current_owner` came by the ball. See [`PossessionSource`].
    pub possession_source: PossessionSource,
    /// Who `possession_source` describes, so a repeat event for the
    /// player who already has the ball cannot relabel their acquisition.
    pub possession_source_for: Option<u32>,
    /// Whether the current pass has already had its one interception
    /// attempt. Mirrors `ShotTarget::block_rolled`: without a latch the
    /// intercept test fires every tick the ball is in flight, so its
    /// rate is set by how long the flight window happens to be rather
    /// than by the defending. Reset when a pass is struck.
    pub intercept_rolled: bool,
    pub contested_claim_count: u32,
    pub unowned_ticks: u32,
    /// Snapshot captured at the moment the ball became uncontrolled — ball
    /// kinematics plus every player's state/position/velocity. Held until
    /// the stall resolves, then attached to the resolution log (only if
    /// the stall was long enough to log). Provides the "what did the
    /// pitch look like when this got stuck" context in the same line as
    /// the duration. Cleared on ownership resume.
    pub stall_start_snapshot: Option<String>,
    pub goal_scored: bool,
    /// The ball is in the goal — see [`BallInNet`]. Set the instant it
    /// crosses the line and cleared by the restart, so it outlives
    /// `goal_scored` (which the flow layer consumes on the same tick to arm
    /// the celebration). Every resolver that would otherwise see a ball
    /// behind the goal line and award a corner, a goal kick or a boundary
    /// clamp keys off this being `Some`.
    pub in_net: Option<BallInNet>,
    pub kickoff_team_side: Option<PlayerSide>,
    pub cached_landing_position: Vector3<f32>,
    /// When a set-piece (corner, goal kick) rewrites ownership to a
    /// specific player, the ball can only mutate itself here — player
    /// teleport requires &mut field.players which lives one layer up.
    /// Populated inside `check_wide_of_goal` and drained by the engine
    /// after `ball.update` returns, so the owner is on the ball before
    /// the next `move_to` distance check can null their ownership.
    pub pending_set_piece_teleport: Option<(u32, Vector3<f32>)>,
    /// Attacking centre-backs to teleport into the box when a corner is
    /// awarded — the dead-ball set-up (in real football the big men walk
    /// up during the stoppage). Populated in the corner branch of
    /// `check_wide_of_goal`, drained by the engine alongside the taker
    /// teleport. Each entry is (player_id, box_target_position). Without
    /// this the CBs cannot cover the length of the pitch before the cross
    /// is delivered, so defenders never get to attack corners.
    pub pending_corner_teleports: Vec<(u32, Vector3<f32>)>,
    /// Fire-once guard for the discrete corner aerial contest. A played-out
    /// lofted corner can't thread the congested box to a specific runner, so
    /// once the cross is struck the engine resolves a single skill-weighted
    /// aerial contest (attacking headers vs the defending line + GK command)
    /// and, if an attacker wins, drops the ball on their head to be headed
    /// on goal. False = armed (a corner has been awarded, not yet resolved);
    /// true = nothing to resolve.
    pub corner_contest_resolved: bool,
    /// Corner routine picked by `pick_corner_routine` at corner setup.
    /// Lets the corner aerial-contest in `resolve_corner_contest` and
    /// downstream xG accounting know whether the delivery is targeting
    /// the near post, far post, penalty spot, or short. Cleared after
    /// the corner resolves. `None` whenever a corner isn't pending.
    pub pending_corner_routine: Option<CornerRoutine>,
    /// Fire-once guard for the OPEN-PLAY cross aerial contest, the
    /// sibling of `corner_contest_resolved`. A lofted cross is aimed at a
    /// patch of the box, not at a pair of feet, so it cannot be settled by
    /// whichever player's state machine happens to run first — the engine
    /// resolves one skill-weighted contest (best attacking header vs the
    /// nearest defenders vs the keeper's command of his area) and drops
    /// the ball on the winner. `false` = armed (a lofted cross is in the
    /// air, not yet resolved); `true` = nothing to resolve, which is also
    /// the resting state for ground deliveries and every ordinary pass.
    pub cross_contest_resolved: bool,
    /// Which delivery the crossing model chose for the ball currently in
    /// flight. Read by the contest (a whipped near-post ball is harder for
    /// a keeper to claim than a floated one) and cleared with the rest of
    /// the pending-pass metadata.
    pub pending_cross_type: Option<CrossType>,
    /// Player an engine-level aerial contest has already awarded the ball
    /// to. Their heading state must NOT roll a second duel — the contest
    /// is the duel, and re-rolling it is double jeopardy (the bug the
    /// corner path documents and works around with a clean-contact
    /// floor). Cleared on the next touch or when the ball settles.
    pub aerial_contest_winner: Option<u32>,
    /// Counter for "ball is owned but nothing is happening" stalls.
    /// The unowned-stall warning can't see these because ownership is
    /// set, but visually the ball sits with a player who isn't moving,
    /// isn't passing, isn't dribbling — same "ball stuck" symptom, no
    /// warning. Reset whenever owner changes or any meaningful motion
    /// resumes; fires a separate warning once it crosses the threshold.
    pub owned_stuck_ticks: u32,
    /// Diagnostic only: was the ball owned by a player in a TakeBall
    /// state on the previous full tick? Used to count spells rather
    /// than ticks — see `dead_ball_diag::TAKEBALL_OWN_SPELLS`.
    #[cfg(feature = "match-logs")]
    pub takeball_owned_last_tick: bool,
    pub owned_stuck_logged: bool,
    /// Position-based stall detector — catches cases the owned/unowned
    /// counters miss, specifically: rapid ownership flipping keeps
    /// resetting both counters (each "change" looks like progress) but
    /// the ball physically never leaves a small region. We sample the
    /// ball's position every N ticks and if it hasn't moved more than
    /// a threshold distance over a window, it's stuck regardless of
    /// who "owns" it at any given instant.
    pub stall_anchor_pos: Vector3<f32>,
    pub stall_anchor_tick: u32,

    /// Trajectory projection cached at the moment a shot is fired. Lets
    /// the goalkeeper commit to an intercept line instead of re-chasing
    /// the ball's current position every tick (which lost ground vs a
    /// 5.6 u/tick shot). `None` whenever the ball isn't a shot in
    /// flight; cleared on catch, goal, or any ownership event.
    pub cached_shot_target: Option<ShotTarget>,

    /// Per-shot lifecycle marker: when the physics-level `try_save_shot`
    /// resolves a shot mid-flight (catch / parry / dangerous parry), it
    /// stores `(keeper_id, shooter_id)` here so the post-tick stat
    /// credit can fire saves and on-target without relying on the GK
    /// state machine to also re-detect the same shot.
    /// Consumed (cleared to `None`) by the event dispatcher once
    /// stats have been credited. This makes saves-on-target match
    /// physics-resolved saves 1:1 — the previous architecture had two
    /// independent save systems (physics and state-machine) where one
    /// changed ball state without crediting and the other rolled
    /// independent saves that often missed.
    pub pending_save_credit: Option<(u32, u32)>,

    /// How hard the keeper had to work for that save, in reach ratio
    /// (0 = straight at him, 1 = full-stretch). Consumed alongside
    /// `pending_save_credit` to put him into the matching STATE.
    ///
    /// Without it the physics save resolves a shot entirely inside ball
    /// physics and the keeper's own state machine never runs, so he never
    /// visibly dives, catches or gets up — the ball simply stops at a
    /// standing man. Measured: ~86 saves a match, of which only 8.4 put
    /// him in `Diving` and `Goalkeeper: Diving` sat below 0.25% of ticks.
    pub pending_save_reach: f32,

    /// Last meaningful touch on the ball. Drives restart resolution
    /// (throw-ins, corners, goal kicks) and pass-origin metadata. Updated
    /// from any path that hands ownership to a player (claim, intercept,
    /// block, save, pass) and from foot-deflections that don't transfer
    /// ownership but still count as a touch for the dead-ball decision.
    pub last_touch_player_id: Option<u32>,
    /// Where the last touch happened. Diagnostic-only, so it exists only
    /// under `match-logs` — see `EndlineCensus`.
    #[cfg(feature = "match-logs")]
    pub last_touch_position: Vector3<f32>,
    pub last_touch_team_id: Option<u32>,
    pub last_touch_tick: u64,
    pub last_touch_was_controlled: bool,
    /// Latest tick captured at update entry. Lets per-update helpers
    /// (intercept, block, save, claim, throw-in) record_touch without
    /// having to thread the tick through every signature.
    pub current_tick_cached: u64,

    /// Origin of the most recent live pass — set when a PassTo event
    /// fires from a restart (goal kick, throw-in, corner, free kick).
    /// Read by the delayed-offside resolver. Resets to OpenPlay on any
    /// non-restart pass or once the pass-window expires.
    pub pass_origin_restart: PassOriginRestart,
    /// Set at pass-kick. Lives for the pass window (~220 ticks) and the
    /// offside resolver fires the call only when the receiver becomes
    /// active (touches the ball or claims). Cleared on resolution,
    /// opponent touch, or expiry.
    pub offside_snapshot: Option<OffsideSnapshot>,

    /// Origin of the most-recent live pass (passer's position when the
    /// pass was emitted). Read by the pass-completion classifier to
    /// decide if the pass was progressive / cross / box-entry. None
    /// outside an active pass window.
    pub pending_pass_origin: Option<Vector3<f32>>,
    /// Intended target position of the most-recent live pass. Cleared
    /// alongside `pending_pass_passer`.
    pub pending_pass_target: Option<Vector3<f32>>,
    /// Pass was emitted from the wide channel toward the box — flagged
    /// at emit-time so the completion classifier can credit
    /// `crosses_completed` when the same pass is received.
    pub pending_pass_was_cross: bool,

    /// Snapshot of the most recently *completed* pass — populated by
    /// `credit_completed_pass` AFTER it bumps `passes_completed` and
    /// BEFORE it clears `pending_pass_*`. The shot-handler key-pass
    /// linker reads these (rather than `pending_pass_*` which the
    /// completion path nulls out) so a receive-then-shoot sequence
    /// still credits the assister with a key pass. None outside the
    /// shot-after-pass window.
    pub last_completed_pass_passer_id: Option<u32>,
    pub last_completed_pass_receiver_id: Option<u32>,
    pub last_completed_pass_tick: u64,

    /// Opponents that were within the pressing radius of the passer at
    /// pass-emit time. Read by the interception handler to credit a
    /// successful pressure when their close-range presence forced the
    /// turnover. Capped at 4 entries — the count of "real" pressers in
    /// any single moment is small. Cleared at pass-completion or
    /// pass-window expiry.
    pub pressers_at_pass: [u32; 4],
    pub pressers_at_pass_count: u8,

    /// Most-recent shot's **post-shot** expected goal — the probability a
    /// league-average keeper concedes it, from
    /// [`SaveModel::expected_goal_on_target`]. Booked against the
    /// defending keeper by `note_shot_faced` as both the expectation his
    /// goals-prevented is measured against and the sign of his
    /// `xg_prevented` ledger. Cleared on resolution (save / goal / wide /
    /// over) and on any non-shot ownership change.
    ///
    /// Post-shot, not pre-shot, and the distinction is the whole point:
    /// the pre-shot value describes the SITUATION the defence conceded,
    /// so charging the keeper's expectation with it made a keeper behind
    /// a good defence look like one facing league-average chances however
    /// tame the strikes actually were. This value describes the STRIKE.
    pub last_shot_xgot: f32,
    pub last_shot_shooter_id: Option<u32>,
    /// Tick the ball was last STRUCK as a shot, whoever has touched it
    /// since. `check_goal` needs a property of the BALL here, not of
    /// whoever happens to be its `previous_owner` when it crosses the
    /// line: a keeper who gets a hand to a shot becomes the previous
    /// owner, and the shot-provenance test then failed on him and
    /// refused the goal. Measured 2026-08: 2604 balls per 300 matches
    /// crossed the line and were rejected — 34% of all shots, and the
    /// single largest reason the engine scored 1.6 goals a game.
    pub last_shot_struck_tick: u64,

    /// Shot-lifecycle census state (`match-logs` only). Set at the strike
    /// and cleared the moment the shot resolves; see
    /// [`Ball::census_shot_fate`], which is the only reader. `0.0` in
    /// `census_shot_dist` means no shot is being tracked.
    #[cfg(feature = "match-logs")]
    pub census_shot_live: bool,
    #[cfg(feature = "match-logs")]
    pub census_shot_dist: f32,
    #[cfg(feature = "match-logs")]
    pub census_shot_side: Option<PlayerSide>,

    /// Tick of the most recent live rebound — a dangerous GK parry or
    /// a loose shot-block deflection that left the ball contestable in
    /// front of goal. Read by the team shot gate: within the rebound
    /// window (~3 s) the team-level shot SPACING and build-up gates
    /// are suspended so the box scramble / tap-in — one of football's
    /// core goal patterns — can actually fire. The per-possession shot
    /// cap (2) still rules out machine-gun scrambles. 0 = no rebound.
    pub last_rebound_tick: u64,

    /// Last meaningful giveaway: the player who lost possession via a
    /// misplaced pass that was intercepted by an opponent. Read by the
    /// "errors leading to shot/goal" linker — when an opponent shoots
    /// within the response window after this is stamped, the giver is
    /// charged with the error.
    pub last_giveaway_player_id: Option<u32>,
    pub last_giveaway_team_id: Option<u32>,
    pub last_giveaway_tick: u64,
    /// Defensive zone the giveaway happened in (from the giver's
    /// perspective). Lets the goal handler credit
    /// `errors_to_goal_own_box` when an opponent converts a giveaway
    /// from inside the giver's own box.
    pub last_giveaway_was_own_box: bool,
    /// Player charged with `errors_leading_to_shot` for the shot
    /// currently in flight. Held from shoot-time until the shot
    /// resolves; if the shot becomes a goal we also bump
    /// `errors_leading_to_goal` on this player.
    pub pending_error_to_shot_player_id: Option<u32>,
    /// Goalkeeper who has just flapped a claim — dropped a cross, punched
    /// it back into the box, missed the ball entirely. Held until the
    /// possession resolves so a shot that follows can be charged to the
    /// keeper as `gk_failed_claims_to_shot` (and, if it goes in,
    /// `gk_failed_claims_to_goal`).
    ///
    /// Deliberately SEPARATE from `pending_error_to_shot_player_id`: the
    /// rating de-dups nested mistake counters (see `errors_and_cards`),
    /// and a failed claim that also stamped `errors_leading_to_goal`
    /// would bill one incident through two lanes — the triple-counting
    /// bug that once dropped a one-conceded keeper to ~3.9.
    pub pending_failed_claim_gk_id: Option<u32>,
    pub pending_failed_claim_tick: u64,
    /// Set once the flap has been charged as `gk_failed_claims_to_shot`.
    /// The id survives so a goal from the same scramble can still be
    /// promoted, but a second shot in the same possession must not bill
    /// the keeper twice for one mistake.
    pub pending_failed_claim_charged: bool,

    /// Carry tracking. `carry_owner` is the player currently dribbling /
    /// running with the ball; `carry_start_position` is where the carry
    /// began. Evaluated when the carry ends (owner change / shot / pass)
    /// to credit progressive carries and box entries.
    pub carry_owner: Option<u32>,
    pub carry_start_position: Vector3<f32>,

    /// Who last put the ball into play out of their own control — a pass,
    /// a goal kick, a clearance — with where and when they did it.
    ///
    /// Read by [`Ball::blocked_recollect_player`] to stop the releaser
    /// immediately re-collecting a delivery that has barely moved. Real
    /// football has no rule against running onto your own pass, but the
    /// engine had a degenerate cycle that did need one: a goalkeeper
    /// whose kick landed at his feet picked it up, kicked again, and
    /// never got out of his own six-yard box. The ball-travel test (not a
    /// blanket ban) is what keeps a legitimate one-two or chip-over-the-
    /// top intact.
    ///
    /// Cleared the moment any OTHER player touches the ball, and on every
    /// dead-ball restart.
    pub last_release_player_id: Option<u32>,
    pub last_release_position: Vector3<f32>,
    pub last_release_tick: u64,
    /// Whether that release was out of a goalkeeper's HANDS. Drives the
    /// second-touch half of Law 12: once a keeper puts the ball back into
    /// play he may not handle it again until someone else has played it.
    pub last_release_from_hands: bool,

    /// The ball is in a goalkeeper's gloves.
    ///
    /// Distinct from `current_owner` being a keeper, which only says he has
    /// it at his feet. A ball in the hands is out of play in every sense
    /// that matters to the other twenty-one players: it cannot be tackled,
    /// intercepted, or claimed, and pressing it is pointless. Nothing
    /// represented that before — a keeper who had caught a cross could be
    /// dispossessed by a forward standing next to him, because
    /// `check_ball_ownership` just hands the ball to the best tackler
    /// within 5u whoever they are.
    pub held_in_hands: bool,

    /// The last touch was a team-mate deliberately playing the ball with
    /// their feet (a pass or a throw-in), which is what arms the back-pass
    /// prohibition. Set by [`Ball::note_deliberate_kick`] and cleared by
    /// [`Ball::record_touch`] — so ANY subsequent touch by anyone, of any
    /// kind, disarms it automatically. That is exactly the Law: a header
    /// back, a deflection, an opponent's touch, all restore the keeper's
    /// right to use his hands.
    pub last_touch_was_deliberate_kick: bool,
}

/// Whether a goalkeeper may pick this ball up, and if not, why not.
///
/// The engine had no notion of this at all: `Catching` never checked where
/// it was happening, so a keeper would take the ball cleanly in his hands
/// forty metres from his own goal, and a back-pass was gathered exactly
/// like a cross.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlingVerdict {
    /// Hands are legal — gather it.
    Legal,
    /// Outside his own penalty area. Handling here is a direct free kick
    /// (and usually a red card), so a keeper simply does not do it.
    OutsideArea,
    /// Deliberately kicked to him by a team-mate, or thrown in by one.
    /// Indirect free kick if he handles it, so he plays it with his feet.
    BackPass,
    /// He has already released it and nobody else has touched it since.
    SecondTouch,
}

impl HandlingVerdict {
    #[inline]
    pub fn is_legal(self) -> bool {
        matches!(self, HandlingVerdict::Legal)
    }
}

/// Projection of a shot at the moment it's taken. The `PreparingForSave`
/// and `Catching` goalkeeper states read this to know where the ball
/// will actually arrive rather than chasing its current position — a
/// diving keeper commits to a spot on the line, they don't track the
/// ball every frame.
#[derive(Debug, Clone, Copy)]
pub struct ShotTarget {
    /// y-coordinate at which the shot is projected to cross the goal
    /// line, in field units. Falls outside the posts if the shot is
    /// going wide — the keeper should still attempt the save, the
    /// post-vs-net check happens in `check_goal`.
    pub goal_line_y: f32,
    /// z-coordinate (height) at projected crossing. Above `GOAL_HEIGHT`
    /// (2.44) is an over-the-bar ball the keeper shouldn't commit to.
    pub goal_line_z: f32,
    /// Goal the ball is heading for — left (x=0) or right (x=field_w).
    /// Used so the correct keeper reads the cache.
    pub defending_side: PlayerSide,
    /// True once the physics save roll has been resolved for THIS
    /// shot. The roll used to run on every tick the ball sat inside the
    /// keeper's reach window (~2-3 ticks), compounding to ~88% per shot
    /// from a 0.55 per-tick cap — which is why `skill_mult` needed five
    /// successive empirical retunes whenever state-machine timing moved
    /// the window length. One shot, one roll: the probability below is
    /// now a genuine per-shot save chance calibrated straight against
    /// real save% (~67% of shots on target).
    pub save_rolled: bool,
    /// True once the block roll has been resolved for THIS shot — the
    /// same one-shot-one-roll discipline `save_rolled` enforces. Without
    /// it, widening the block window means rolling once per tick the
    /// defender stays in the lane, so the block rate becomes a function
    /// of flight timing rather than of the model.
    pub block_rolled: bool,
    /// Set when the shot took a deflection off a body in the lane.
    /// Catching/Diving states damp the save probability — the keeper
    /// was set for the original trajectory and the redirected ball is
    /// arriving on a new line they haven't committed to.
    pub deflected: bool,
    /// The striker's `shot_threat` composite (0..1) at the moment he hit
    /// it. Carried on the shot rather than looked up at save time
    /// because the save resolves several ticks later, by which point
    /// `previous_owner` may have moved on and the shooter's fatigue
    /// bands have drifted.
    ///
    /// `SaveModel` reads this to score the save as a CONTEST against the
    /// man who struck the ball instead of against an absolute bar — see
    /// `SaveModel::skill_multiplier`. Defaults to
    /// `SaveModel::NEUTRAL_THREAT` on the paths that synthesise a shot
    /// target without a shooter, which reproduces the old
    /// absolute-quality behaviour exactly for those cases.
    pub shooter_threat: f32,
}

#[derive(Default, Clone)]
pub struct BallFlags {
    pub in_flight_state: usize,
    pub running_for_ball: bool,
}

impl BallFlags {
    pub fn reset(&mut self) {
        self.in_flight_state = 0;
        self.running_for_ball = false;
    }
}

impl Ball {
    pub fn with_coord(field_width: f32, field_height: f32) -> Self {
        let x = field_width / 2.0;
        let y = field_height / 2.0;

        Ball {
            position: Vector3::new(x, y, 0.0),
            start_position: Vector3::new(x, y, 0.0),
            field_width,
            field_height,
            velocity: Vector3::zeros(),
            spin: Vector3::zeros(),
            #[cfg(feature = "match-logs")]
            settled_vz: 0.0,
            center_field_position: x, // initial ball position = center field
            flags: BallFlags::default(),
            previous_owner: None,
            current_owner: None,
            take_ball_notified_players: Vec::new(),
            notification_cooldown: 0,
            notification_timeout: 0,
            last_boundary_position: None,
            unowned_stopped_ticks: 0,
            ownership_duration: 0,
            claim_cooldown: 0,
            pass_target_player_id: None,
            pending_pass_passer: None,
            pending_pass_set_tick: 0,
            recent_passers: VecDeque::with_capacity(5),
            possession_source: PossessionSource::Unknown,
            possession_source_for: None,
            intercept_rolled: false,
            contested_claim_count: 0,
            unowned_ticks: 0,
            stall_start_snapshot: None,
            goal_scored: false,
            in_net: None,
            kickoff_team_side: None,
            cached_landing_position: Vector3::new(x, y, 0.0),
            pending_set_piece_teleport: None,
            pending_corner_teleports: Vec::new(),
            corner_contest_resolved: true,
            pending_corner_routine: None,
            cross_contest_resolved: true,
            pending_cross_type: None,
            aerial_contest_winner: None,
            owned_stuck_ticks: 0,
            #[cfg(feature = "match-logs")]
            takeball_owned_last_tick: false,
            owned_stuck_logged: false,
            stall_anchor_pos: Vector3::new(x, y, 0.0),
            stall_anchor_tick: 0,
            cached_shot_target: None,
            pending_save_credit: None,
            pending_save_reach: 0.0,
            last_touch_player_id: None,
            #[cfg(feature = "match-logs")]
            last_touch_position: Vector3::new(x, y, 0.0),
            last_touch_team_id: None,
            last_touch_tick: 0,
            last_touch_was_controlled: false,
            current_tick_cached: 0,
            pass_origin_restart: PassOriginRestart::OpenPlay,
            offside_snapshot: None,
            pending_pass_origin: None,
            pending_pass_target: None,
            pending_pass_was_cross: false,
            last_completed_pass_passer_id: None,
            last_completed_pass_receiver_id: None,
            last_completed_pass_tick: 0,
            pressers_at_pass: [0; 4],
            pressers_at_pass_count: 0,
            last_shot_xgot: 0.0,
            last_shot_shooter_id: None,
            last_shot_struck_tick: 0,
            #[cfg(feature = "match-logs")]
            census_shot_live: false,
            #[cfg(feature = "match-logs")]
            census_shot_dist: 0.0,
            #[cfg(feature = "match-logs")]
            census_shot_side: None,
            last_rebound_tick: 0,
            last_giveaway_player_id: None,
            last_giveaway_team_id: None,
            last_giveaway_tick: 0,
            last_giveaway_was_own_box: false,
            pending_error_to_shot_player_id: None,
            pending_failed_claim_gk_id: None,
            pending_failed_claim_tick: 0,
            pending_failed_claim_charged: false,
            carry_owner: None,
            carry_start_position: Vector3::new(x, y, 0.0),
            last_release_player_id: None,
            last_release_position: Vector3::new(x, y, 0.0),
            last_release_tick: 0,
            last_release_from_hands: false,
            held_in_hands: false,
            last_touch_was_deliberate_kick: false,
        }
    }

    /// Record that `player_id` has just released the ball into open play
    /// from `position`. See [`Ball::last_release_player_id`].
    pub fn note_release(&mut self, player_id: u32, position: Vector3<f32>, tick: u64) {
        self.last_release_player_id = Some(player_id);
        self.last_release_position = position;
        self.last_release_tick = tick;
        // Any release puts the ball back in open play — it is no longer in
        // anyone's gloves. `from_hands` is stamped separately by the
        // goalkeeper release paths.
        self.last_release_from_hands = self.held_in_hands;
        self.held_in_hands = false;
    }

    /// A field player has deliberately played the ball with their feet.
    ///
    /// Routed through `record_touch` so the touch bookkeeping stays in one
    /// place, then raises the deliberate-kick flag that
    /// [`Ball::is_backpass_to`] reads. Because `record_touch` LOWERS the
    /// flag, the very next touch by anybody disarms the back-pass bar with
    /// no explicit clearing anywhere.
    pub fn note_deliberate_kick(&mut self, player_id: u32, team_id: u32, tick: u64) {
        self.record_touch(player_id, team_id, tick, true);
        self.last_touch_was_deliberate_kick = true;
    }

    /// True when handling this ball would breach the back-pass law: the
    /// last touch was a team-mate of `keeper_id` deliberately kicking or
    /// throwing it.
    pub fn is_backpass_to(&self, keeper_id: u32, keeper_team: u32) -> bool {
        self.last_touch_was_deliberate_kick
            && self.last_touch_team_id == Some(keeper_team)
            && self.last_touch_player_id != Some(keeper_id)
    }

    /// True when `keeper_id` put this ball back into play from his hands
    /// and nobody has played it since — the second-touch prohibition.
    pub fn awaiting_touch_after_release_by(&self, keeper_id: u32) -> bool {
        self.last_release_from_hands && self.last_release_player_id == Some(keeper_id)
    }

    /// Height the ball rides at while it is being carried, in metres.
    ///
    /// At a player's feet normally — and at CHEST HEIGHT in a keeper's
    /// gloves. `held_in_hands` was a rules concept only: the ball still
    /// snapped to z = 0, so a keeper who had just caught a cross was drawn
    /// with it lying on the grass by his boots, and the replay showed a
    /// goalkeeper who never uses his hands for anything. Nothing else was
    /// wrong — the viewer draws exactly the height it is given.
    ///
    /// 1.15 m is where a man of the model's 1.79 m holds a ball into his
    /// chest. It stays well under `is_aerial`'s 2.3 m and under the 2.44 m
    /// crossbar, so no height-gated rule changes behaviour because of it.
    pub fn carry_height(&self) -> f32 {
        if self.held_in_hands { 1.15 } else { 0.0 }
    }

    /// Is this player close enough to the ball to be given it?
    ///
    /// # Why every grant has to ask
    ///
    /// [`MAX_OWNER_TRACK_DISTANCE`] is the furthest the ball will follow
    /// the player who owns it. Grant possession beyond that and
    /// [`Ball::move_to`] disowns the ball on the very next tick — but by
    /// then the granting handler has already **zeroed the velocity**, so
    /// what `move_to` releases is a dead ball. It stops in mid-pitch with
    /// nobody near it, everyone converges on it, and somebody eventually
    /// plays it backwards. Reported from the viewer exactly that way, and
    /// counted at 87 times a match by `reception_diag::OWNER_TOO_FAR`.
    ///
    /// So the check is not new — `move_to` has always made it. It was just
    /// made one tick too late to be survivable. Asking here, before
    /// anything is mutated, means the grant simply does not happen and the
    /// ball flies on untouched, which is the same outcome minus the
    /// wreckage.
    ///
    /// Measured in the XY plane, exactly as `move_to` measures it: a ball
    /// directly overhead is within reach whatever its height.
    pub fn within_possession_reach(&self, player_position: Vector3<f32>) -> bool {
        let dx = player_position.x - self.position.x;
        let dy = player_position.y - self.position.y;
        dx * dx + dy * dy <= MAX_OWNER_TRACK_DISTANCE * MAX_OWNER_TRACK_DISTANCE
    }

    /// Take the ball into `keeper_id`'s gloves.
    pub fn gather_in_hands(&mut self, keeper_id: u32, team_id: u32, tick: u64) {
        #[cfg(feature = "match-logs")]
        ownership::reception_diag::GATHERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.record_touch(keeper_id, team_id, tick, true);
        self.held_in_hands = true;
        self.last_release_from_hands = false;
    }

    /// The player currently barred from re-collecting the ball because
    /// they released it themselves and it has not yet gone anywhere, or
    /// `None` when nobody is barred.
    ///
    /// Bounded on BOTH axes so it can never become a deadlock of its own:
    /// the bar lifts as soon as the ball has travelled `MIN_TRAVEL`, and
    /// unconditionally after `MAX_BLOCK_TICKS` whether it moved or not.
    /// Without the time bound, a ball that stops 2 m from a lone player
    /// with no one else nearby would sit there forever.
    pub fn blocked_recollect_player(&self) -> Option<u32> {
        /// 5 m. Short enough that a genuine one-two or a chip over the top
        /// is unaffected; long enough that a delivery which never left the
        /// striker's own feet is caught.
        const MIN_TRAVEL: f32 = 40.0;
        /// 2 s. Deadlock escape — see above.
        const MAX_BLOCK_TICKS: u64 = 200;

        let releaser = self.last_release_player_id?;
        if self
            .current_tick_cached
            .saturating_sub(self.last_release_tick)
            > MAX_BLOCK_TICKS
        {
            return None;
        }
        let dx = self.position.x - self.last_release_position.x;
        let dy = self.position.y - self.last_release_position.y;
        if dx * dx + dy * dy >= MIN_TRAVEL * MIN_TRAVEL {
            return None;
        }
        Some(releaser)
    }

    /// Record a meaningful touch. Drives restart resolution. `controlled`
    /// distinguishes a clean reception from a deflection / failed save.
    pub fn record_touch(&mut self, player_id: u32, team_id: u32, tick: u64, controlled: bool) {
        // Where the touch happened, so a downstream diagnostic can ask how
        // far the ball ran afterwards. Diagnostic-only — see
        // `EndlineCensus`.
        #[cfg(feature = "match-logs")]
        {
            self.last_touch_position = self.position;
            // A lofted delivery that somebody touches before the aerial
            // contest resolves it never gets contested at all — the ball
            // was reserved for one named receiver rather than fought for
            // by the box. Counting these says whether the crossing gap is
            // a DELIVERY problem or a RECEPTION problem.
            if !self.cross_contest_resolved
                && self.pending_cross_type.is_some_and(CrossType::is_lofted)
            {
                CrossDiag::note_touched_first();
            }
            // Pass OVERSHOOT, measured at the chokepoint every touch goes
            // through: a live pass that somebody has just touched tells us
            // how far it was meant to travel and how far it actually did.
            // The whole question "is the ball being struck too hard" is
            // this ratio, and nothing else measures it.
            if let (Some(origin), Some(target)) =
                (self.pending_pass_origin, self.pending_pass_target)
            {
                PassWeightCensus::note(
                    (target - origin).magnitude(),
                    (self.position - origin).magnitude(),
                    self.pass_target_player_id == Some(player_id),
                );
            }
        }
        // Somebody else has been on the ball — whatever the last releaser
        // did is history, and their re-collect bar lifts.
        if self
            .last_release_player_id
            .is_some_and(|id| id != player_id)
        {
            self.last_release_player_id = None;
            // Somebody else has played it, so the keeper who put it into
            // play may use his hands again (Law 12's second-touch bar
            // lifts on any other player's touch).
            self.last_release_from_hands = false;
        }
        // Every touch disarms the back-pass bar. `note_deliberate_kick`
        // re-raises it immediately afterwards for the one touch that
        // should — see its docs.
        self.last_touch_was_deliberate_kick = false;
        // A touch ends whatever aerial contest awarded the ball: the
        // planted header has been struck, or somebody else got there
        // first. Either way the "don't re-roll the duel" grant is spent.
        self.aerial_contest_winner = None;
        // A foot or a chest kills the rotation. Whatever the ball was
        // doing in the air, the next kick decides what it does now.
        self.spin = Vector3::zeros();
        self.last_touch_player_id = Some(player_id);
        self.last_touch_team_id = Some(team_id);
        self.last_touch_tick = tick;
        self.last_touch_was_controlled = controlled;
    }

    /// Clear the offside snapshot. Called on opponent touch, claim, foul,
    /// or pass expiry.
    pub fn clear_offside_snapshot(&mut self) {
        self.offside_snapshot = None;
    }

    /// Force the ball into a clean dead-ball restart state. Centralises
    /// the flag clearing that every set-piece restart (corner / goal
    /// kick / throw-in / kickoff after goal) used to do by hand,
    /// dropping stale open-play metadata so a shot/pass that was in
    /// flight when the ball went dead cannot leak across the restart.
    ///
    /// This is the canonical "ball just went dead — reset everything
    /// open-play touched" helper. New restart paths should call this
    /// rather than zeroing individual fields, so a future field added
    /// to the open-play set is reset automatically.
    pub fn clear_open_play_metadata(&mut self) {
        #[cfg(feature = "match-logs")]
        if self.pending_pass_passer.is_some() {
            use std::sync::atomic::Ordering;
            ownership::reception_diag::DIED_DEAD_BALL.fetch_add(1, Ordering::Relaxed);
        }
        self.cached_shot_target = None;
        self.pass_target_player_id = None;
        self.pending_pass_passer = None;
        self.pending_pass_origin = None;
        self.pending_pass_target = None;
        self.pending_pass_was_cross = false;
        self.offside_snapshot = None;
        self.pending_save_credit = None;
        self.pending_error_to_shot_player_id = None;
        self.pending_failed_claim_gk_id = None;
        self.pending_failed_claim_charged = false;
        self.last_shot_xgot = 0.0;
        self.last_shot_shooter_id = None;
        // A dead ball ends the shot: without this a stale strike would
        // let the next pass that rolls over the line stand as a goal.
        self.last_shot_struck_tick = 0;
        // A restart is a fresh delivery — the taker may legally be the
        // player who last released the ball in open play, and no dead ball
        // is ever in a keeper's gloves.
        self.last_release_player_id = None;
        self.last_release_from_hands = false;
        self.held_in_hands = false;
        self.last_touch_was_deliberate_kick = false;
    }

    /// Soft invariant check on the ball's lifecycle flags. Returns the
    /// first violation as `Err(msg)` so debug builds and tests can
    /// assert the ball never enters a contradictory state. Production
    /// callers ignore the result — the cost is a handful of field
    /// reads.
    ///
    /// Invariants checked:
    ///   * Open-play shot metadata implies a previous owner (someone
    ///     fired the shot).
    ///   * Pending save credit references a real shooter id (so the
    ///     stat dispatch can fold the on-target back to a shot taker).
    ///   * A pass target id implies a passer id was set when the pass
    ///     was launched (else the receive-classifier has nothing to
    ///     pair the completion to).
    ///   * Ball/owner position coordinates are finite — non-finite x/y/z
    ///     leak into distance comparisons and trigger
    ///     `partial_cmp().unwrap()` panics in sort paths.
    ///   * On a dead-ball restart (corner / goal kick / throw-in /
    ///     free kick / penalty), open-play metadata (cached shot,
    ///     pending pass envelope, save credit, offside snapshot) must
    ///     be cleared — otherwise a shot that was in flight when the
    ///     ball went dead can leak across the restart and credit
    ///     phantom stats.
    ///   * Pending shot xG implies a shooter id (paired metadata,
    ///     consumed together).
    ///   * Pending pass envelope is coherent: a passer implies an
    ///     origin and target position.
    ///   * Carry tracking is consistent: a carrying owner means the
    ///     current owner matches the carrier.
    pub fn check_invariants(&self) -> Result<(), &'static str> {
        if self.cached_shot_target.is_some() && self.previous_owner.is_none() {
            return Err("cached_shot_target without previous_owner");
        }
        if let Some((_keeper, shooter)) = self.pending_save_credit {
            if shooter == 0 {
                return Err("pending_save_credit shooter id is sentinel zero");
            }
        }
        if self.pass_target_player_id.is_some() && self.pending_pass_passer.is_none() {
            return Err("pass_target without pending_pass_passer");
        }
        // Non-finite coordinates leak into distance comparisons and
        // trigger `partial_cmp().unwrap()` panics in nearby/sort paths.
        if !self.position.x.is_finite()
            || !self.position.y.is_finite()
            || !self.position.z.is_finite()
        {
            return Err("ball position has non-finite coordinate");
        }
        if !self.velocity.x.is_finite()
            || !self.velocity.y.is_finite()
            || !self.velocity.z.is_finite()
        {
            return Err("ball velocity has non-finite coordinate");
        }
        // Dead-ball restart cleanliness — any restart origin must drop
        // open-play metadata.
        if matches!(
            self.pass_origin_restart,
            PassOriginRestart::Corner
                | PassOriginRestart::GoalKick
                | PassOriginRestart::ThrowIn
                | PassOriginRestart::Penalty
        ) {
            if self.cached_shot_target.is_some() {
                return Err("dead-ball restart with leftover cached_shot_target");
            }
            if self.pending_save_credit.is_some() {
                return Err("dead-ball restart with leftover pending_save_credit");
            }
            if self.offside_snapshot.is_some() {
                return Err("dead-ball restart with leftover offside_snapshot");
            }
        }
        // Pending shot xG and shooter id are kept in lock-step.
        if self.last_shot_xgot > 0.0 && self.last_shot_shooter_id.is_none() {
            return Err("last_shot_xgot without last_shot_shooter_id");
        }
        // Pending pass envelope: any leg must imply the rest.
        if self.pending_pass_passer.is_some()
            && (self.pending_pass_origin.is_none() || self.pending_pass_target.is_none())
        {
            return Err("pending_pass_passer without origin/target metadata");
        }
        // Carry tracking — a current carrier must match the ball owner.
        if let (Some(carrier), Some(owner)) = (self.carry_owner, self.current_owner) {
            if carrier != owner {
                return Err("carry_owner disagrees with current_owner");
            }
        }
        // A ball in the gloves has a keeper holding it. Nothing else in
        // the engine may take ownership away without lowering the flag,
        // or the ball becomes permanently unclaimable.
        if self.held_in_hands && self.current_owner.is_none() {
            return Err("held_in_hands with no owner");
        }
        Ok(())
    }
}

#[cfg(test)]
mod aerial_reach_tests {
    use super::*;

    /// Height must be a difficulty, not a door. The engine's aerial gates
    /// were all binary: below the number the ball was as easy to play as
    /// one on the floor, above it the ball did not exist. That single
    /// shape produced both reported symptoms — defenders picking balls
    /// out of the air without moving, and nobody at all going for one a
    /// few centimetres higher.
    #[test]
    fn reach_difficulty_falls_away_smoothly_instead_of_switching_off() {
        let jumping = 12.0;
        let ceiling = AerialReach::ceiling(jumping);
        assert_eq!(
            AerialReach::reach_difficulty(0.0, jumping),
            1.0,
            "a ball on the deck is no harder than a ball on the deck"
        );
        assert_eq!(
            AerialReach::reach_difficulty(AerialReach::HEAD, jumping),
            1.0,
            "up to head height costs nothing"
        );
        assert_eq!(
            AerialReach::reach_difficulty(ceiling + 0.01, jumping),
            0.0,
            "past his own ceiling he cannot play it at all"
        );

        // Strictly decreasing in between — no plateau a player could sit
        // on, and no cliff.
        let mut previous = 1.0;
        let mut z = AerialReach::HEAD;
        while z < ceiling {
            let d = AerialReach::reach_difficulty(z, jumping);
            assert!(
                d <= previous,
                "difficulty must not rise as the ball climbs (at {z} m)"
            );
            assert!((0.0..=1.0).contains(&d), "difficulty stays a fraction");
            previous = d;
            z += 0.05;
        }
        assert!(
            previous < 0.25,
            "a ball at the very top of the jump must be a fingertip touch, got {previous}"
        );
    }

    /// The ceiling belongs to the PLAYER. The old flat `2.5` gate meant
    /// the best header of the ball in the division and the worst had
    /// exactly the same aerial range.
    #[test]
    fn a_better_leaper_reaches_a_higher_ball() {
        let poor = AerialReach::ceiling(1.0);
        let elite = AerialReach::ceiling(20.0);
        assert!(
            elite > poor + 0.4,
            "jumping must be worth real height: {poor} vs {elite}"
        );
        // A ball an elite leaper can just about reach is out of a poor
        // one's range entirely.
        let z = poor + 0.1;
        assert_eq!(AerialReach::reach_difficulty(z, 1.0), 0.0);
        assert!(AerialReach::reach_difficulty(z, 20.0) > 0.0);
    }

    /// A jump is timed to the ball, not maximal — otherwise a player
    /// would launch himself to his ceiling for every ball above his head.
    #[test]
    fn a_leap_reaches_the_ball_and_no_further() {
        let jumping = 14.0;
        assert_eq!(
            AerialReach::leap_for(AerialReach::STANDING - 0.1, jumping),
            0.0,
            "a ball he can reach standing needs no jump"
        );
        let low = AerialReach::leap_for(AerialReach::STANDING + 0.2, jumping);
        let high = AerialReach::leap_for(AerialReach::STANDING + 0.6, jumping);
        assert!(low > 0.0 && high > low, "higher ball, bigger jump");
        // Never asked to jump past his own ceiling.
        let ceiling = AerialReach::ceiling(jumping);
        let beyond = AerialReach::leap_for(ceiling + 5.0, jumping);
        assert!(
            beyond <= ceiling - AerialReach::STANDING + 1.0e-4,
            "the leap is bounded by what he can actually jump"
        );
    }

    /// A header is played off the forehead, not off a raised boot, so it
    /// starts 40 cm lower. Measuring it from the standing reach is what
    /// would keep a player flat-footed for most real headers.
    #[test]
    fn a_header_leaves_the_ground_earlier_than_a_boot_does() {
        let jumping = 12.0;
        let z = AerialReach::HEAD + 0.15; // 1.95 m — a normal header
        assert_eq!(
            AerialReach::leap_for(z, jumping),
            0.0,
            "a boot can still reach this standing"
        );
        assert!(
            AerialReach::header_leap_for(z, jumping) > 0.0,
            "but heading it means jumping"
        );
    }
}

#[cfg(test)]
mod gk_handling_tests {
    use super::*;

    const KEEPER: u32 = 1;
    const KEEPER_TEAM: u32 = 10;
    const DEFENDER: u32 = 2;
    const OPPONENT: u32 = 3;
    const OPPONENT_TEAM: u32 = 20;

    fn ball() -> Ball {
        Ball::with_coord(840.0, 545.0)
    }

    #[test]
    fn a_teammates_deliberate_kick_bars_the_keepers_hands() {
        let mut b = ball();
        b.note_deliberate_kick(DEFENDER, KEEPER_TEAM, 100);
        assert!(b.is_backpass_to(KEEPER, KEEPER_TEAM));
    }

    #[test]
    fn any_later_touch_disarms_the_backpass_bar() {
        // The Law: a header back, a deflection, an opponent's touch — each
        // restores the keeper's right to use his hands. This falls out of
        // `record_touch` lowering the flag rather than from any explicit
        // clearing, so it holds for touch paths that do not exist yet.
        for (toucher, team, controlled) in [
            (DEFENDER, KEEPER_TEAM, false),   // deflection off a team-mate
            (OPPONENT, OPPONENT_TEAM, true),  // opponent played it
            (OPPONENT, OPPONENT_TEAM, false), // opponent deflected it
        ] {
            let mut b = ball();
            b.note_deliberate_kick(DEFENDER, KEEPER_TEAM, 100);
            b.record_touch(toucher, team, 120, controlled);
            assert!(
                !b.is_backpass_to(KEEPER, KEEPER_TEAM),
                "touch by {toucher} (controlled={controlled}) should have disarmed the bar"
            );
        }
    }

    #[test]
    fn an_opponents_pass_is_not_a_backpass() {
        let mut b = ball();
        b.note_deliberate_kick(OPPONENT, OPPONENT_TEAM, 100);
        assert!(!b.is_backpass_to(KEEPER, KEEPER_TEAM));
    }

    #[test]
    fn a_keeper_does_not_bar_himself_by_kicking() {
        // His own distribution is governed by the second-touch rule, not
        // the back-pass one — and that rule only bites if he released it
        // from his HANDS.
        let mut b = ball();
        b.note_deliberate_kick(KEEPER, KEEPER_TEAM, 100);
        assert!(!b.is_backpass_to(KEEPER, KEEPER_TEAM));
    }

    #[test]
    fn releasing_from_the_hands_bars_a_second_handling() {
        let mut b = ball();
        b.gather_in_hands(KEEPER, KEEPER_TEAM, 100);
        assert!(b.held_in_hands);

        b.note_release(KEEPER, Vector3::new(20.0, 270.0, 0.0), 400);
        assert!(!b.held_in_hands, "releasing empties the gloves");
        assert!(b.awaiting_touch_after_release_by(KEEPER));
    }

    #[test]
    fn the_second_touch_bar_lifts_once_anyone_else_plays_it() {
        let mut b = ball();
        b.gather_in_hands(KEEPER, KEEPER_TEAM, 100);
        b.note_release(KEEPER, Vector3::new(20.0, 270.0, 0.0), 400);
        b.record_touch(DEFENDER, KEEPER_TEAM, 460, true);
        assert!(!b.awaiting_touch_after_release_by(KEEPER));
    }

    #[test]
    fn a_kick_off_the_deck_does_not_arm_the_second_touch_bar() {
        // Only a release FROM THE HANDS does. A keeper who sweeps a ball
        // clear with his feet may pick up the next one.
        let mut b = ball();
        b.note_release(KEEPER, Vector3::new(20.0, 270.0, 0.0), 400);
        assert!(!b.awaiting_touch_after_release_by(KEEPER));
    }

    #[test]
    fn a_dead_ball_clears_every_handling_bar() {
        let mut b = ball();
        b.note_deliberate_kick(DEFENDER, KEEPER_TEAM, 100);
        b.gather_in_hands(KEEPER, KEEPER_TEAM, 110);
        b.note_release(KEEPER, Vector3::new(20.0, 270.0, 0.0), 400);

        b.clear_open_play_metadata();

        assert!(!b.held_in_hands);
        assert!(!b.is_backpass_to(KEEPER, KEEPER_TEAM));
        assert!(!b.awaiting_touch_after_release_by(KEEPER));
    }

    #[test]
    fn a_held_ball_keeps_the_invariants() {
        let mut b = ball();
        b.current_owner = Some(KEEPER);
        b.gather_in_hands(KEEPER, KEEPER_TEAM, 100);
        assert!(b.check_invariants().is_ok());

        // Ownership taken away without lowering the flag would leave the
        // ball permanently unclaimable — the claim path skips it entirely.
        b.current_owner = None;
        assert!(b.check_invariants().is_err());
    }
}

#[allow(dead_code, unused_imports)]
mod offside_snapshot_tests {
    use super::*;

    fn snap_left(receiver_x: f32, ball_x: f32, second_last: f32) -> OffsideSnapshot {
        OffsideSnapshot {
            origin: PassOriginRestart::OpenPlay,
            passer_id: 1,
            passer_side: PlayerSide::Left,
            receiver_id: 2,
            ball_x_at_kick: ball_x,
            second_last_defender_x: second_last,
            receiver_x_at_kick: receiver_x,
            receiver_y_at_kick: 200.0,
            set_tick: 0,
        }
    }

    #[test]
    fn left_attacker_beyond_second_last_is_offside() {
        // Receiver ahead of ball AND past the second-last defender.
        let snap = snap_left(700.0, 600.0, 680.0);
        assert!(snap.is_offside());
    }

    #[test]
    fn left_attacker_behind_ball_not_offside() {
        // Receiver is behind the ball — offside cannot occur.
        let snap = snap_left(500.0, 600.0, 680.0);
        assert!(!snap.is_offside());
    }

    #[test]
    fn left_attacker_level_with_defender_not_offside() {
        // Within tolerance — onside.
        let snap = snap_left(681.0, 600.0, 680.0);
        assert!(!snap.is_offside());
    }

    #[test]
    fn restart_origins_offside_exempt() {
        assert!(PassOriginRestart::GoalKick.is_offside_exempt());
        assert!(PassOriginRestart::Corner.is_offside_exempt());
        assert!(PassOriginRestart::ThrowIn.is_offside_exempt());
        assert!(!PassOriginRestart::OpenPlay.is_offside_exempt());
        assert!(!PassOriginRestart::FreeKick.is_offside_exempt());
    }
}

impl Ball {
    /// Update cached landing position. Call after physics changes position/velocity.
    #[inline]
    pub fn update_landing_cache(&mut self) {
        self.cached_landing_position = self.calculate_landing_position();
    }

    pub fn update(
        &mut self,
        context: &mut MatchContext,
        players: &[MatchPlayer],
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        self.current_tick_cached = context.current_tick();
        #[cfg(feature = "match-logs")]
        let owner_at_entry = self.current_owner;
        #[cfg(feature = "match-logs")]
        let spell_at_entry = self.ownership_duration;
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            ownership::reception_diag::TOTAL_TICKS.fetch_add(1, Ordering::Relaxed);
            if self.held_in_hands {
                ownership::reception_diag::HELD_TICKS.fetch_add(1, Ordering::Relaxed);
            }
        }

        // The ball is in the goal: the netting owns it and play is dead
        // until the restart. Nothing below applies — there is no pass to
        // intercept, no shot to save, no owner to track and no boundary to
        // clamp against — and several of those passes would actively
        // misread it (see the guards in `goal.rs`). The celebration drives
        // the ball from here; this covers the ticks between the goal and
        // the flow layer noticing it, plus any caller driving `update`
        // directly.
        if self.in_net.is_some() {
            self.tick_net(&context.goal_positions);
            return;
        }

        // Decrement claim cooldown
        if self.claim_cooldown > 0 {
            self.claim_cooldown -= 1;
        }

        // A vertical speed that is higher than the one this ball's own
        // physics produced last tick was put there by a kick — see
        // `settled_vz`. Sampled before `update_velocity` so the bounce
        // it applies is not mistaken for one.
        #[cfg(feature = "match-logs")]
        {
            if self.velocity.z > self.settled_vz + 1.0e-5 && self.velocity.z > 0.0 {
                let striker = self
                    .current_owner
                    .or(self.previous_owner)
                    .and_then(|id| players.iter().find(|p| p.id == id))
                    .map(|p| p.state.compact_id() as usize);
                flight_diag::FlightDiag::note_launch(self.velocity.z, self.position.z, striker);
            }
        }
        #[cfg(feature = "match-logs")]
        let mut probe = flight_diag::StageProbe::new(self.position);

        self.update_velocity();

        self.try_intercept(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            flight_diag::STAGE_INTERCEPT,
            self.position,
            self.velocity,
            0.0,
        );
        self.try_block_shot(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(flight_diag::STAGE_BLOCK, self.position, self.velocity, 0.0);
        self.try_save_shot(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(flight_diag::STAGE_SAVE, self.position, self.velocity, 0.0);
        self.try_notify_standing_ball(players, events);

        // NUCLEAR OPTION: Force claiming if ball unowned and stopped for too long
        self.force_claim_if_deadlock(players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            flight_diag::STAGE_DEADLOCK,
            self.position,
            self.velocity,
            0.0,
        );

        // Unconditional unowned safety net - forces nearest players to TakeBall
        self.force_takeball_if_unowned_too_long(players, events);
        // `detect_owned_stuck` was too sensitive — it fired on legitimate
        // possession play (defender holding in back line for 6-12s is
        // normal). `detect_position_stall` is the stricter signal: ball
        // hasn't moved ANYWHERE in 1000 ticks, regardless of who owns
        // it. That's a real stall.
        self.detect_position_stall(players);
        #[cfg(feature = "match-logs")]
        probe.note(flight_diag::STAGE_STALL, self.position, self.velocity, 0.0);

        self.process_ownership(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            flight_diag::STAGE_OWNERSHIP,
            self.position,
            self.velocity,
            0.0,
        );
        self.tick_carry_tracker(events);

        // Move ball FIRST, then check goal/boundary on new position
        // `move_to` is entitled to a tick of its own velocity, plus the
        // owner-tracking step it uses instead when the ball is carried.
        #[cfg(feature = "match-logs")]
        let move_allowance = (self.velocity.x * self.velocity.x
            + self.velocity.y * self.velocity.y)
            .sqrt()
            .max(1.5);
        self.move_to(tick_context);
        #[cfg(feature = "match-logs")]
        probe.note(
            flight_diag::STAGE_MOVE,
            self.position,
            self.velocity,
            move_allowance,
        );
        self.check_goal(context, events);
        #[cfg(feature = "match-logs")]
        probe.note(flight_diag::STAGE_GOAL, self.position, self.velocity, 0.0);
        self.check_over_goal(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            flight_diag::STAGE_OVER_BAR,
            self.position,
            self.velocity,
            0.0,
        );
        self.check_wide_of_goal(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(flight_diag::STAGE_WIDE, self.position, self.velocity, 0.0);
        self.check_throw_in(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            flight_diag::STAGE_THROW_IN,
            self.position,
            self.velocity,
            0.0,
        );
        self.check_boundary_collision(context);
        #[cfg(feature = "match-logs")]
        probe.note(
            flight_diag::STAGE_BOUNDARY,
            self.position,
            self.velocity,
            0.0,
        );
        self.expire_offside_snapshot(context);
        self.update_landing_cache();

        #[cfg(feature = "match-logs")]
        {
            flight_diag::FlightDiag::note_tick(
                self.position,
                self.velocity,
                self.current_owner.is_some(),
            );
            self.settled_vz = self.velocity.z;

            // Possession churn, sampled once per full tick around the
            // whole ball update — so it catches every release site,
            // including the ones inside `move_to` and the boundary
            // checks, without a counter planted at each.
            use crate::r#match::engine::ball::ball::stall::dead_ball_diag as dbd;
            use std::sync::atomic::Ordering;

            // Pressure on the man in possession — the "is anybody coming
            // to him" number. Sampled here rather than in any state
            // because it is a property of the SITUATION, and every state
            // that could measure it has already decided not to engage.
            if let Some(owner) = self
                .current_owner
                .and_then(|id| players.iter().find(|p| p.id == id))
            {
                let mut nearest = f32::MAX;
                let mut engagers = 0u64;
                for opp in players.iter() {
                    if opp.team_id == owner.team_id || opp.is_sent_off {
                        continue;
                    }
                    let d = (opp.position - owner.position).magnitude();
                    nearest = nearest.min(d);
                    if d < 80.0 {
                        engagers += 1;
                    }
                }
                if nearest < f32::MAX {
                    let m = nearest * 0.125;
                    let bucket = if m < 2.0 {
                        0
                    } else if m < 5.0 {
                        1
                    } else if m < 10.0 {
                        2
                    } else if m < 20.0 {
                        3
                    } else {
                        4
                    };
                    dbd::CARRIER_PRESSURE[bucket].fetch_add(1, Ordering::Relaxed);
                    // Thirds from the CARRIER's point of view, so "own
                    // third" means his own regardless of which way he
                    // is playing.
                    let attacking_right = owner.side == Some(crate::r#match::PlayerSide::Left);
                    let progress = if attacking_right {
                        self.position.x / self.field_width
                    } else {
                        1.0 - self.position.x / self.field_width
                    };
                    let third = if progress < 0.333 {
                        0
                    } else if progress < 0.667 {
                        1
                    } else {
                        2
                    };
                    dbd::CARRIER_PRESSURE_BY_THIRD[third * 5 + bucket]
                        .fetch_add(1, Ordering::Relaxed);
                    dbd::CARRIER_NEAREST_X10.fetch_add((nearest * 10.0) as u64, Ordering::Relaxed);
                    dbd::CARRIER_ENGAGERS.fetch_add(engagers, Ordering::Relaxed);
                    dbd::CARRIER_SAMPLES.fetch_add(1, Ordering::Relaxed);
                }
            }

            // Whole-match TakeBall ownership, not just stalls: is this a
            // state that holds the ball, or the state everybody is in on
            // the tick they claim it?
            let tb_now = self
                .current_owner
                .and_then(|id| players.iter().find(|p| p.id == id))
                .is_some_and(|p| p.state.is_take_ball());
            if tb_now {
                dbd::TAKEBALL_OWN_TICKS.fetch_add(1, Ordering::Relaxed);
                if !self.takeball_owned_last_tick || owner_at_entry != self.current_owner {
                    dbd::TAKEBALL_OWN_SPELLS.fetch_add(1, Ordering::Relaxed);
                }
            }
            self.takeball_owned_last_tick = tb_now;

            if owner_at_entry != self.current_owner {
                // Turnovers that happen while the ball is already judged
                // stuck. Cross-team means a real scramble; same-team
                // means it is bouncing around one side, which would be a
                // passing problem rather than a contest.
                if self.stall_anchor_tick >= 250 && self.current_owner.is_some() {
                    dbd::STALL_TURNOVERS.fetch_add(1, Ordering::Relaxed);
                    let team_of = |id: Option<u32>| {
                        id.and_then(|i| players.iter().find(|p| p.id == i))
                            .map(|p| p.team_id)
                    };
                    let before = team_of(owner_at_entry);
                    let after = team_of(self.current_owner);
                    if before.is_some() && after.is_some() && before != after {
                        dbd::STALL_TURNOVERS_CROSS_TEAM.fetch_add(1, Ordering::Relaxed);
                    }
                }
                if owner_at_entry.is_some() {
                    dbd::OWNERSHIP_LOST.fetch_add(1, Ordering::Relaxed);
                    dbd::SPELL_LENGTH[dbd::spell_bucket(spell_at_entry)]
                        .fetch_add(1, Ordering::Relaxed);
                }
                if self.current_owner.is_some() {
                    dbd::OWNERSHIP_GAINED.fetch_add(1, Ordering::Relaxed);
                    if self.current_owner == self.previous_owner {
                        dbd::OWNERSHIP_RECLAIMED_SELF.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
        #[cfg(feature = "match-logs")]
        self.census_shot_fate(context, players);
    }

    /// Classify how the shot in flight ended, exactly once, at the end of
    /// the tick it ended on. Diagnostic only — see the `FATE_*` counters
    /// in `ownership::reception_diag` for why this exists.
    ///
    /// Deliberately central rather than a flag planted at each exit: the
    /// per-site counters that came before it accounted for ~20 of every
    /// 3500 shots struck, because most shots do not leave through any of
    /// the sites that had one.
    #[cfg(feature = "match-logs")]
    fn census_shot_fate(&mut self, context: &MatchContext, players: &[MatchPlayer]) {
        use ownership::reception_diag as d;
        use std::sync::atomic::Ordering;

        if !self.census_shot_live {
            return;
        }
        d::FATE_LIVE_TICKS.fetch_add(1, Ordering::Relaxed);

        let dist_x100 = (self.census_shot_dist * 100.0) as u64;
        let mut resolve = |counter: &'static std::sync::atomic::AtomicU64, reached_goal: bool| {
            counter.fetch_add(1, Ordering::Relaxed);
            if reached_goal {
                d::FATE_REACHED_DIST_X100.fetch_add(dist_x100, Ordering::Relaxed);
            }
        };

        if self.goal_scored {
            resolve(&d::FATE_GOAL, true);
        } else if self.pass_origin_restart != PassOriginRestart::OpenPlay {
            // A restart was staged this tick — corner, goal kick or
            // throw. The shot went out of play.
            resolve(&d::FATE_OUT, false);
        } else if let Some(owner) = self.current_owner {
            let owner_p = players.iter().find(|p| p.id == owner);
            let is_gk = owner_p
                .map(|p| p.tactical_position.current_position.is_goalkeeper())
                .unwrap_or(false);
            let same_side = owner_p.and_then(|p| p.side) == self.census_shot_side;
            if is_gk && !same_side {
                resolve(&d::FATE_GK, true);
            } else if same_side {
                resolve(&d::FATE_CLAIMED_ATT, false);
            } else {
                resolve(&d::FATE_CLAIMED_DEF, false);
            }
        } else if self.is_delivery_spent() {
            resolve(&d::FATE_STOPPED, false);
        } else if context
            .current_tick()
            .saturating_sub(self.last_shot_struck_tick)
            > 400
        {
            resolve(&d::FATE_TIMEOUT, false);
        } else {
            return; // still in the air
        }
        self.census_shot_live = false;
    }

    /// Light update: full ball logic but reads owner position from players slice directly.
    pub fn update_light(
        &mut self,
        context: &mut MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        self.current_tick_cached = context.current_tick();

        // See `Ball::update` — the netting owns a ball that has gone in.
        if self.in_net.is_some() {
            self.tick_net(&context.goal_positions);
            return;
        }

        if self.claim_cooldown > 0 {
            self.claim_cooldown -= 1;
        }

        #[cfg(feature = "match-logs")]
        if self.velocity.z > self.settled_vz + 1.0e-5 && self.velocity.z > 0.0 {
            let striker = self
                .current_owner
                .or(self.previous_owner)
                .and_then(|id| players.iter().find(|p| p.id == id))
                .map(|p| p.state.compact_id() as usize);
            flight_diag::FlightDiag::note_launch(self.velocity.z, self.position.z, striker);
        }

        self.update_velocity();
        self.try_intercept(context, players, events);
        self.try_block_shot(context, players, events);
        self.try_save_shot(context, players, events);
        self.process_ownership(context, players, events);
        self.tick_carry_tracker(events);

        // Move ball: find owner position from players slice directly
        self.move_to_with_players(players);
        self.check_goal(context, events);
        self.check_over_goal(context, players, events);
        self.check_wide_of_goal(context, players, events);
        self.check_throw_in(context, players, events);
        self.check_boundary_collision(context);
        self.expire_offside_snapshot(context);
        self.update_landing_cache();

        #[cfg(feature = "match-logs")]
        {
            flight_diag::FlightDiag::note_tick(
                self.position,
                self.velocity,
                self.current_owner.is_some(),
            );
            self.settled_vz = self.velocity.z;
        }
    }

    /// Calculate where an aerial ball will land (when z reaches 0).
    /// Uses projectile motion: z(t) = h + vz·t − ½g·t² = 0, solving for
    /// the positive root. Ignores air drag — close enough for chase
    /// positioning, and erring long is better than erring short.
    ///
    /// Units are ticks, not seconds: position integration is
    /// `position += velocity` per tick (no dt scaling), while gravity
    /// applies `velocity.z += -GRAVITY * 0.016` per tick. So the
    /// effective per-tick² gravity is `9.81 * 0.016 ≈ 0.157`, and the
    /// resulting `time_to_ground` comes out in ticks — which matches
    /// the horizontal integration `x += vx` per tick.
    pub fn calculate_landing_position(&self) -> Vector3<f32> {
        if self.position.z <= 0.1 || self.current_owner.is_some() {
            return self.position;
        }

        const G_PER_TICK: f32 = GRAVITY_PER_TICK;
        let vz = self.velocity.z;
        let h = self.position.z;

        // Positive root of ½g·t² − vz·t − h = 0
        let discriminant = vz * vz + 2.0 * G_PER_TICK * h;
        let time_to_ground = (vz + discriminant.sqrt()) / G_PER_TICK;

        let landing_x = self.position.x + self.velocity.x * time_to_ground;
        let landing_y = self.position.y + self.velocity.y * time_to_ground;

        let clamped_x = landing_x.clamp(0.0, self.field_width);
        let clamped_y = landing_y.clamp(0.0, self.field_height);

        Vector3::new(clamped_x, clamped_y, 0.0)
    }

    /// Check if the ball is aerial (in the air above player reach)
    pub fn is_aerial(&self) -> bool {
        const PLAYER_REACH_HEIGHT: f32 = 2.3;
        // 0.005 m/tick = 0.5 m/s. The old 0.1 was 10 m/s — a bar set in
        // the units gravity used to be written in, which meant a ball
        // hanging at head height read as "not aerial" the moment it
        // slowed near its apex.
        const MOVING_VERTICALLY: f32 = 0.005;
        self.position.z > PLAYER_REACH_HEIGHT && self.velocity.z.abs() > MOVING_VERTICALLY
    }

    pub fn is_stands_outside(&self) -> bool {
        self.is_ball_outside()
            && self.velocity.norm_squared() < 0.25 // 0.5^2, allow tiny velocities from physics
            && self.current_owner.is_none()
    }

    pub fn is_ball_stopped_on_field(&self) -> bool {
        !self.is_ball_outside()
            && self.velocity.norm_squared() < 6.25 // 2.5^2, catch slow rolling balls that need claiming
            && self.current_owner.is_none()
    }

    pub fn is_ball_outside(&self) -> bool {
        self.position.x <= 0.0
            || self.position.x >= self.field_width
            || self.position.y <= 0.0
            || self.position.y >= self.field_height
    }

    /// Lightweight movement: just apply velocity to position (no ownership logic)
    pub fn apply_movement(&mut self) {
        self.position.x += self.velocity.x;
        self.position.y += self.velocity.y;
        self.position.z += self.velocity.z;
        if self.position.z < 0.0 {
            self.position.z = 0.0;
        }
    }

    pub fn reset(&mut self) {
        self.position.x = self.start_position.x;
        self.position.y = self.start_position.y;
        self.position.z = 0.0;

        self.velocity = Vector3::zeros();
        // The goal is over — whatever is left of it goes with the restart.
        self.in_net = None;

        self.clear_for_dead_ball();
    }

    /// Everything [`Ball::reset`] drops apart from where the ball IS.
    ///
    /// Split out for the goal path: a ball that has just crossed the line is
    /// as dead as one on the centre spot — no owner, no pass in flight, no
    /// shot target, no offside snapshot — but it is emphatically not on the
    /// centre spot, it is in the net travelling at whatever it was hit at.
    /// Sharing the body is what stops the two drifting apart.
    fn clear_for_dead_ball(&mut self) {
        self.current_owner = None;
        self.previous_owner = None;
        self.ownership_duration = 0;
        self.claim_cooldown = 0;

        self.flags.reset();
        self.pass_target_player_id = None;
        self.clear_pass_history();
        self.possession_source = PossessionSource::Unknown;
        self.possession_source_for = None;
        self.intercept_rolled = false;
        self.contested_claim_count = 0;
        self.unowned_ticks = 0;
        self.cached_landing_position = self.position;
        self.pending_set_piece_teleport = None;
        self.pending_corner_teleports.clear();
        self.owned_stuck_ticks = 0;
        self.owned_stuck_logged = false;
        self.stall_anchor_pos = self.position;
        self.stall_anchor_tick = 0;
        self.cached_shot_target = None;
        self.pending_save_credit = None;
        self.last_touch_player_id = None;
        self.last_touch_team_id = None;
        self.last_touch_tick = 0;
        self.last_touch_was_controlled = false;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;
        self.offside_snapshot = None;
        self.last_completed_pass_passer_id = None;
        self.last_completed_pass_receiver_id = None;
        self.last_completed_pass_tick = 0;
        self.last_shot_struck_tick = 0;
        self.last_release_player_id = None;
        self.last_release_from_hands = false;
        self.held_in_hands = false;
        self.last_touch_was_deliberate_kick = false;
    }

    /// Snapshot the most-recent completed pass so the shot-handler
    /// key-pass linker can credit the passer when the receiver
    /// shoots within the key-pass window. Called from
    /// `credit_completed_pass` *before* `clear_pending_pass_metadata`
    /// nulls out the live pass envelope.
    #[inline]
    pub fn record_completed_pass(&mut self, passer_id: u32, receiver_id: u32, tick: u64) {
        self.last_completed_pass_passer_id = Some(passer_id);
        self.last_completed_pass_receiver_id = Some(receiver_id);
        self.last_completed_pass_tick = tick;
    }

    pub fn clear_player_reference(&mut self, player_id: u32) {
        if self.current_owner == Some(player_id) {
            self.current_owner = None;
            self.ownership_duration = 0;
            // A substituted / sent-off keeper cannot still be holding it.
            self.held_in_hands = false;
        }
        if self.previous_owner == Some(player_id) {
            self.previous_owner = None;
        }
        if self.pass_target_player_id == Some(player_id) {
            self.pass_target_player_id = None;
        }
        if self.last_release_player_id == Some(player_id) {
            self.last_release_player_id = None;
        }
        if self.last_completed_pass_passer_id == Some(player_id)
            || self.last_completed_pass_receiver_id == Some(player_id)
        {
            self.last_completed_pass_passer_id = None;
            self.last_completed_pass_receiver_id = None;
        }
        self.take_ball_notified_players
            .retain(|&id| id != player_id);
        self.recent_passers.retain(|e| e.player_id != player_id);
    }

    /// Record a passer in the recent passers ring buffer.
    /// Skips consecutive duplicates and caps at 5 entries.
    pub fn record_passer(&mut self, passer_id: u32, team_id: u32, tick: u64) {
        // Skip consecutive duplicates
        if self.recent_passers.back().map(|e| e.player_id) == Some(passer_id) {
            return;
        }
        if self.recent_passers.len() >= 5 {
            self.recent_passers.pop_front();
        }
        self.recent_passers.push_back(PassChainEntry {
            player_id: passer_id,
            team_id,
            tick,
        });
    }

    /// The teammate whose pass should be credited with an assist for a
    /// goal scored by `scorer_id` of `scorer_team_id` at `tick`, if any.
    ///
    /// Walks the chain newest-first and applies the three rules a real
    /// assist obeys:
    ///
    ///  1. **Same team.** The credited player must be a teammate of the
    ///     scorer. Without this the resolver happily handed the assist to
    ///     the goalkeeper whose goal kick got turned over — measured at
    ///     71% of all assists, 63% of them to keepers.
    ///  2. **Same possession.** Stop at the first opponent entry. A pass
    ///     made before the other team had the ball belongs to an earlier
    ///     phase of play, not to this goal.
    ///  3. **Recent.** The pass has to have led to the goal, so it must
    ///     land inside `ASSIST_WINDOW_TICKS`. This is what stops a goal
    ///     kick from being an "assist" for a solo run half a minute later.
    pub fn assist_for_goal(&self, scorer_id: u32, scorer_team_id: u32, tick: u64) -> Option<u32> {
        #[cfg(feature = "match-logs")]
        use std::sync::atomic::Ordering;
        #[cfg(feature = "match-logs")]
        assist_diag::GOALS.fetch_add(1, Ordering::Relaxed);

        for entry in self.recent_passers.iter().rev() {
            // Rule 2: an opponent touched the chain — earlier entries
            // belong to a possession that is not this one.
            if entry.team_id != scorer_team_id {
                #[cfg(feature = "match-logs")]
                {
                    assist_diag::OPPONENT_CHAIN.fetch_add(1, Ordering::Relaxed);
                    assist_diag::OPPONENT_CHAIN_AGE
                        .fetch_add(tick.saturating_sub(entry.tick), Ordering::Relaxed);
                    if self
                        .recent_passers
                        .iter()
                        .any(|e| e.team_id == scorer_team_id && e.player_id != scorer_id)
                    {
                        assist_diag::OPPONENT_CHAIN_HAS_TEAMMATE.fetch_add(1, Ordering::Relaxed);
                    }
                }
                return None;
            }
            if entry.player_id == scorer_id {
                continue;
            }
            // Rule 3: `tick` is monotonic within a match, but stay
            // defensive about the ordering anyway.
            let delay = tick.saturating_sub(entry.tick);
            if delay > ASSIST_WINDOW_TICKS {
                #[cfg(feature = "match-logs")]
                assist_diag::STALE.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            #[cfg(feature = "match-logs")]
            {
                assist_diag::CREDITED.fetch_add(1, Ordering::Relaxed);
                assist_diag::CREDITED_DELAY_TICKS.fetch_add(delay, Ordering::Relaxed);
            }
            return Some(entry.player_id);
        }
        #[cfg(feature = "match-logs")]
        {
            if self.recent_passers.is_empty() {
                assist_diag::EMPTY_CHAIN.fetch_add(1, Ordering::Relaxed);
            } else {
                assist_diag::SCORER_ONLY.fetch_add(1, Ordering::Relaxed);
            }
        }
        None
    }

    /// Clear the recent passers history (e.g. on tackles, interceptions, clearances).
    pub fn clear_pass_history(&mut self) {
        self.recent_passers.clear();
    }

    /// Label how `player_id` came by the ball.
    ///
    /// Ignores repeat events for a player who already has it: `Claimed`
    /// fires to re-affirm existing ownership as well as to acquire, so
    /// without this guard a receiver's `PassReception` was relabelled
    /// `LooseBall` a second later while the ball was still at his feet —
    /// which read as 97% of shots coming from loose balls.
    /// For the same carrier only a MORE SPECIFIC label may overwrite: a
    /// repeat `Claimed` must not downgrade a reception to a loose ball,
    /// but the pass-completion credit that lands just after a bare
    /// `Claimed` (a teammate other than the intended target collected
    /// it) must be allowed to upgrade it.
    pub fn note_possession_source(&mut self, player_id: u32, source: PossessionSource) {
        if self.possession_source_for == Some(player_id) && source == PossessionSource::LooseBall {
            return;
        }
        self.possession_source_for = Some(player_id);
        self.possession_source = source;
    }

    /// Note that `team_id` now has the ball, dropping the pass chain only
    /// if the ball genuinely changed hands.
    ///
    /// The recovery paths (loose ball gained, ball headed clear, tackle)
    /// all used to wipe the chain unconditionally. But a loose ball won
    /// by a TEAMMATE is the same attacking phase: a cross flicked on at
    /// the near post, a rebound off a block, a knock-down in the box. The
    /// cross that started the move is still the assist if the move ends
    /// in a goal, and wiping it left the resolver with nothing to credit
    /// on roughly a third of all goals (`assist_diag::EMPTY_CHAIN`).
    ///
    /// Only a change of TEAM ends the phase.
    pub fn note_possession(&mut self, team_id: u32) {
        if self.recent_passers.back().map(|e| e.team_id) != Some(team_id) {
            self.recent_passers.clear();
        }
    }

    /// Clear the pass-window metadata used by the pass-completion classifier
    /// and the key-pass linker. Called whenever the live pass is no longer
    /// in flight (claim, interception, expiry, set-piece restart).
    #[inline]
    pub fn clear_pending_pass_metadata(&mut self) {
        // A lofted delivery being disarmed here never reached the aerial
        // contest. Record the height it died at — that says whether it
        // was cut out on the way up, at head height, or after landing,
        // and those are three different bugs.
        #[cfg(feature = "match-logs")]
        if !self.cross_contest_resolved && self.pending_cross_type.is_some_and(CrossType::is_lofted)
        {
            CrossDiag::note_disarmed_at(self.position.z);
        }
        self.pending_pass_passer = None;
        self.pending_pass_origin = None;
        self.pending_pass_target = None;
        self.pending_pass_was_cross = false;
        self.pending_cross_type = None;
        // Disarm the aerial contest with the delivery it belonged to — a
        // cross that has been claimed, cleared or intercepted is over.
        self.cross_contest_resolved = true;
    }

    /// Drop any in-flight shot metadata (xG / shooter id). Called once
    /// the shot resolves (save / goal / wide / over / opponent claim).
    #[inline]
    pub fn clear_shot_metadata(&mut self) {
        self.last_shot_xgot = 0.0;
        self.last_shot_shooter_id = None;
        // A dead ball ends the shot: without this a stale strike would
        // let the next pass that rolls over the line stand as a goal.
        self.last_shot_struck_tick = 0;
    }

    /// Stamp the giveaway tracker for the player who just lost the ball
    /// via a misplaced pass / lost tackle / dispossession. Subsequent
    /// shot / goal events from the opposing team within the response
    /// window will be charged back as an error to this player. The
    /// `was_own_box` flag is read later by the goal handler to layer the
    /// own-box-extra penalty on top of `errors_leading_to_goal`.
    #[inline]
    pub fn stamp_giveaway(&mut self, player_id: u32, team_id: u32, tick: u64, was_own_box: bool) {
        self.last_giveaway_player_id = Some(player_id);
        self.last_giveaway_team_id = Some(team_id);
        self.last_giveaway_tick = tick;
        self.last_giveaway_was_own_box = was_own_box;
    }

    /// Drop the giveaway tracker — the response window has expired or
    /// the giver's team has recovered the ball.
    #[inline]
    pub fn clear_giveaway(&mut self) {
        self.last_giveaway_player_id = None;
        self.last_giveaway_team_id = None;
        self.last_giveaway_was_own_box = false;
    }

    /// Detect and resolve carry transitions. Called once per tick from
    /// `update` / `update_light`, after `process_ownership` has settled
    /// the current owner. When the owner changes (or goes None) we emit
    /// a `BallEvent::CarryEnded` for the previous carrier; the
    /// dispatcher classifies the carry and credits the carrier's stats.
    /// A new carry starts the moment ownership lands on a player.
    pub fn tick_carry_tracker(&mut self, events: &mut EventCollection) {
        match (self.carry_owner, self.current_owner) {
            (Some(prev), Some(curr)) if prev == curr => {
                // Same carrier — nothing to emit.
            }
            (Some(prev), _) => {
                // Carry ended (owner changed or went None).
                events.add_ball_event(BallEvent::CarryEnded(
                    prev,
                    self.carry_start_position,
                    self.position,
                ));
                self.carry_owner = self.current_owner;
                self.carry_start_position = self.position;
            }
            (None, Some(curr)) => {
                // Carry begins.
                self.carry_owner = Some(curr);
                self.carry_start_position = self.position;
            }
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod completed_pass_tests {
    use super::*;

    #[test]
    fn record_completed_pass_populates_snapshot() {
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.record_completed_pass(7, 11, 1234);
        assert_eq!(ball.last_completed_pass_passer_id, Some(7));
        assert_eq!(ball.last_completed_pass_receiver_id, Some(11));
        assert_eq!(ball.last_completed_pass_tick, 1234);
    }

    #[test]
    fn clear_pending_pass_metadata_does_not_clear_completed_snapshot() {
        // Regression: the centralized completion path used to clear
        // pending_pass_passer immediately, leaving the shot-handler
        // key-pass linker without a passer to credit. The completed
        // snapshot survives the pending clear.
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.pending_pass_passer = Some(7);
        ball.pending_pass_set_tick = 100;
        ball.pending_pass_origin = Some(Vector3::new(50.0, 100.0, 0.0));
        ball.pending_pass_target = Some(Vector3::new(150.0, 100.0, 0.0));
        ball.pending_pass_was_cross = true;
        ball.record_completed_pass(7, 11, 200);
        ball.clear_pending_pass_metadata();
        assert!(ball.pending_pass_passer.is_none());
        assert!(ball.pending_pass_origin.is_none());
        assert!(ball.pending_pass_target.is_none());
        assert!(!ball.pending_pass_was_cross);
        // The completed snapshot stays — the key-pass linker reads it.
        assert_eq!(ball.last_completed_pass_passer_id, Some(7));
        assert_eq!(ball.last_completed_pass_receiver_id, Some(11));
        assert_eq!(ball.last_completed_pass_tick, 200);
    }

    #[test]
    fn clear_player_reference_drops_completed_pass_snapshot() {
        // If a player is removed (red card, sub), any completed-pass
        // metadata referencing them must be cleared so the next shot
        // doesn't credit a phantom key pass.
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.record_completed_pass(7, 11, 200);
        ball.clear_player_reference(7);
        assert!(ball.last_completed_pass_passer_id.is_none());
        assert!(ball.last_completed_pass_receiver_id.is_none());

        // Receiver removal also wipes (consistency).
        ball.record_completed_pass(7, 11, 300);
        ball.clear_player_reference(11);
        assert!(ball.last_completed_pass_passer_id.is_none());
        assert!(ball.last_completed_pass_receiver_id.is_none());
    }
}

#[cfg(test)]
mod assist_tests {
    use super::*;

    const HOME: u32 = 1;
    const AWAY: u32 = 2;

    fn ball() -> Ball {
        Ball::with_coord(840.0, 545.0)
    }

    #[test]
    fn credits_the_teammate_who_played_the_last_pass() {
        let mut ball = ball();
        ball.record_passer(7, HOME, 1000);
        ball.record_passer(9, HOME, 1200);
        assert_eq!(ball.assist_for_goal(10, HOME, 1300), Some(9));
    }

    #[test]
    fn never_credits_an_opponent() {
        // The headline bug: an away keeper's goal kick sat in the ring,
        // the home team turned it over and scored, and the resolver
        // handed the keeper an assist for the goal he conceded. Across a
        // season that put goalkeepers at the top of the assist charts.
        let mut ball = ball();
        ball.record_passer(200, AWAY, 1000); // away GK's goal kick
        assert_eq!(ball.assist_for_goal(10, HOME, 1200), None);
    }

    #[test]
    fn stops_at_a_possession_break() {
        // Home passed, the away team had it and passed too, then home
        // won it back and scored without a pass. The earlier home pass
        // belongs to a different phase of play — no assist.
        let mut ball = ball();
        ball.record_passer(7, HOME, 800);
        ball.record_passer(200, AWAY, 1000);
        assert_eq!(ball.assist_for_goal(10, HOME, 1100), None);
    }

    #[test]
    fn skips_the_scorer_but_keeps_walking_back() {
        // Give-and-go: 7 passes, gets it back, scores. The assist is the
        // teammate who returned it, not 7 himself.
        let mut ball = ball();
        ball.record_passer(9, HOME, 1000);
        ball.record_passer(7, HOME, 1100);
        ball.record_passer(9, HOME, 1200);
        assert_eq!(ball.assist_for_goal(7, HOME, 1250), Some(9));
    }

    #[test]
    fn a_chain_holding_only_the_scorer_yields_nothing() {
        let mut ball = ball();
        ball.record_passer(7, HOME, 1000);
        assert_eq!(ball.assist_for_goal(7, HOME, 1100), None);
    }

    #[test]
    fn a_stale_pass_is_not_an_assist() {
        // A goal kick is not the assist for a solo run that ends half a
        // minute later, however unbroken the possession was.
        let mut ball = ball();
        ball.record_passer(1, HOME, 1000);
        let late = 1000 + ASSIST_WINDOW_TICKS + 1;
        assert_eq!(ball.assist_for_goal(10, HOME, late), None);
        // One tick inside the window still counts.
        assert_eq!(
            ball.assist_for_goal(10, HOME, 1000 + ASSIST_WINDOW_TICKS),
            Some(1)
        );
    }

    #[test]
    fn empty_chain_yields_nothing() {
        assert_eq!(ball().assist_for_goal(10, HOME, 500), None);
    }

    #[test]
    fn possession_survives_a_teammate_winning_a_loose_ball() {
        // A cross flicked on, a rebound off a block, a knock-down in the
        // box — same attacking phase, so the cross is still the assist.
        let mut ball = ball();
        ball.record_passer(2, HOME, 1000);
        ball.note_possession(HOME);
        assert_eq!(ball.assist_for_goal(9, HOME, 1150), Some(2));
    }

    #[test]
    fn possession_drops_the_chain_when_the_ball_changes_hands() {
        let mut ball = ball();
        ball.record_passer(2, HOME, 1000);
        ball.note_possession(AWAY);
        assert!(ball.recent_passers.is_empty());
    }

    #[test]
    fn chain_entries_carry_team_and_tick() {
        let mut ball = ball();
        ball.record_passer(7, HOME, 1000);
        // Consecutive duplicates are still collapsed.
        ball.record_passer(7, HOME, 1050);
        assert_eq!(ball.recent_passers.len(), 1);
        let entry = ball.recent_passers.back().unwrap();
        assert_eq!(entry.player_id, 7);
        assert_eq!(entry.team_id, HOME);
        assert_eq!(entry.tick, 1000);
    }

    #[test]
    fn ring_caps_at_five_and_drops_the_oldest() {
        let mut ball = ball();
        for i in 0..7u32 {
            ball.record_passer(i, HOME, 1000 + i as u64);
        }
        assert_eq!(ball.recent_passers.len(), 5);
        assert_eq!(ball.recent_passers.front().unwrap().player_id, 2);
        assert_eq!(ball.recent_passers.back().unwrap().player_id, 6);
    }
}
