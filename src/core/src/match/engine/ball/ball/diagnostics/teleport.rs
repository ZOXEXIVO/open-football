//! Whole-tick relocation census: every pass over the ball, not just its own.
//!
//! # Why this exists when `flight_diag` already counts jumps
//!
//! [`flight_diag`](crate::r#match::engine::ball::ball::flight_diag)'s `StageProbe` is opened inside
//! [`Ball::update`](crate::r#match::engine::ball::ball::Ball::update) and closed when it returns. That is
//! about a third of a tick. The rest of a full tick —
//!
//! ```text
//!   play_ball            → Ball::update            ← the ONLY part flight_diag sees
//!   apply_pending_set_piece_teleport
//!   apply_pending_save_credit
//!   resolve_corner_contest                          ← writes the ball onto a head
//!   resolve_cross_contest                           ← writes the ball onto a head
//!   FoulResolver::tick_advantage
//!   play_players         → the state machines
//!   EventDispatcher::dispatch → PlayerEventDispatcher ← secure_ball_for, clearances…
//!   handle_goal_reset
//!   apply_pending_set_piece_teleport
//! ```
//!
//! — is invisible to it, and that is exactly where the set pieces and the
//! aerial contests live. Every teleport report this repo has chased since
//! 2026-08 was found by reading recordings rather than a counter, for that
//! one reason. This closes the gap.
//!
//! # Why the measurement is exact, not an estimate
//!
//! **Nothing integrates the ball after `Ball::update` returns.** The ball's
//! position is advanced in exactly one place (`move_to`, inside `update`),
//! so between any two checkpoints below it, a change in `position` is a
//! change somebody *wrote*. There is no velocity term to subtract and no
//! tolerance to argue about: the delta IS the relocation.
//!
//! Only the first checkpoint needs arithmetic, and there the bound is the
//! ball's own speed over one tick — taken as the larger of the entry and
//! exit speeds, because a stage may have changed the velocity as well.
//!
//! # The two axes are one number
//!
//! x/y are in game units (1u = 12.5 cm) and z is in metres, which makes a
//! 2.4 m vertical snap look like a smaller number than a 3-unit sideways
//! nudge while being eight times more visible. Every magnitude here is
//! reported in game units with the height folded in at `* 8.0`, so one
//! figure ranks a drop onto a head against a drag along the floor.
//!
//! Diagnostic infrastructure. Compiled only under `match-logs`; the call
//! sites are `#[cfg]`-gated so a release build has no checkpoints at all.

use nalgebra::Vector3;
use std::sync::atomic::{AtomicU64, Ordering};

/// Units per metre on the horizontal axes. `position.z` is in metres, so
/// height has to be scaled onto the same ruler before the two are combined.
pub const UNITS_PER_METRE: f32 = 8.0;

/// Below this a relocation is not a teleport, it is arithmetic. 1.5u is
/// 19 cm — the same floor `frame_trace` uses, and under anything a replay
/// at 30 ms a frame could show.
pub const TOLERANCE: f32 = 1.5;

/// A relocation at or above this is the artefact a viewer reports: half a
/// metre of ball moving with nothing touching it.
pub const VISIBLE: f32 = 4.0;

/// The passes over the ball in a full tick, in the order the engine runs
/// them, followed by the light tick's shorter list.
///
/// Named for the function that runs between the previous checkpoint and
/// this one, so a row in the census is a place in `tick.rs` and not a
/// category anybody has to interpret.
pub const STAGES: [&str; 21] = [
    // ── full tick ──────────────────────────────────────────────────────
    "ball_update",
    "set_piece_teleport",
    "save_credit",
    "corner_contest",
    "cross_contest",
    "foul_advantage",
    "play_players",
    "dispatch",
    "goal_reset",
    "set_piece_teleport:post_dispatch",
    // ── light tick ─────────────────────────────────────────────────────
    "light:ball_update",
    "light:set_piece_teleport",
    "light:save_credit",
    "light:goalkeepers",
    "light:player_move",
    "light:dispatch",
    "light:goal_reset",
    "light:set_piece_teleport2",
    // ── a breakdown of the two `ball_update` rows, not a total ──────────
    //
    // `Ball::update` has two early returns above the point where
    // `flight_diag`'s probe starts booking, and `update_light` carries no
    // probe at all — so between them the netting and the whole restart
    // machinery have never appeared in a relocation census. These three
    // partition the ball's own pass and are excluded from the headline
    // sum by [`SUBROWS`].
    "  ∟ ball:net_tick",
    "  ∟ ball:awaited_restart",
    "  ∟ ball:live",
];

pub const STAGE_BALL_UPDATE: usize = 0;
pub const STAGE_SET_PIECE: usize = 1;
pub const STAGE_SAVE_CREDIT: usize = 2;
pub const STAGE_CORNER_CONTEST: usize = 3;
pub const STAGE_CROSS_CONTEST: usize = 4;
pub const STAGE_FOUL_ADVANTAGE: usize = 5;
pub const STAGE_PLAY_PLAYERS: usize = 6;
pub const STAGE_DISPATCH: usize = 7;
pub const STAGE_GOAL_RESET: usize = 8;
pub const STAGE_SET_PIECE_POST: usize = 9;
pub const STAGE_L_BALL_UPDATE: usize = 10;
pub const STAGE_L_SET_PIECE: usize = 11;
pub const STAGE_L_SAVE_CREDIT: usize = 12;
pub const STAGE_L_GOALKEEPERS: usize = 13;
pub const STAGE_L_PLAYER_MOVE: usize = 14;
pub const STAGE_L_DISPATCH: usize = 15;
pub const STAGE_L_GOAL_RESET: usize = 16;
pub const STAGE_L_SET_PIECE2: usize = 17;
pub const STAGE_BALL_NET: usize = 18;
pub const STAGE_BALL_RESTART: usize = 19;
pub const STAGE_BALL_LIVE: usize = 20;

/// The rows that break down `ball_update` rather than adding to it.
/// Summing the table without excluding these double-counts the ball's
/// own pass.
pub const SUBROWS: std::ops::Range<usize> = STAGE_BALL_NET..(STAGE_BALL_LIVE + 1);

const N: usize = STAGES.len();

/// Relocations booked against each stage, their summed and worst
/// magnitude in game units, and how many were large enough to see.
pub static JUMPS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
pub static SUM_X100: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
pub static MAX_X100: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
pub static VISIBLE_JUMPS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
/// Of the relocations booked, how many happened while the ball was DEAD
/// (a restart pending). Those are a different bug from the same row: a
/// dead ball being dragged is [`DeadBall`](crate::r#match::engine::ball::ball::DeadBall)
/// leaking, a live one being dragged is a resolver taking a short cut.
pub static DEAD_JUMPS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
/// Of the relocations booked, how many were purely VERTICAL — the ball
/// dropped onto or lifted off the floor with no horizontal movement.
/// A separate row because the fix is different: a height snap is almost
/// always a missing `carry_toward`, not a missing flight.
///
/// ⚠ This axis is the one `flight_diag` cannot see at all: its `StageProbe`
/// measures `sqrt(dx² + dy²)` and drops `z` on the floor, so a ball
/// snapping two metres downward has always read as zero there.
pub static VERTICAL_JUMPS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
/// Vertical relocations big enough to see — the intersection of the two
/// columns above, which is the one a viewer actually reports.
pub static VISIBLE_VERTICAL: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

/// Ticks the census watched, so every row can be read per match.
pub static TICKS: AtomicU64 = AtomicU64::new(0);

/// Per-`PlayerEvent` attribution inside the `dispatch` stage.
///
/// `dispatch` is one row in the table above and a dozen different writers
/// underneath it, so the stage census can only ever say "the player layer
/// did it". This splits it by the event being handled at the time, which
/// names the handler.
const EVENTS: usize = 32;
pub static EVENT_JUMPS: [AtomicU64; EVENTS] = [const { AtomicU64::new(0) }; EVENTS];
pub static EVENT_SUM_X100: [AtomicU64; EVENTS] = [const { AtomicU64::new(0) }; EVENTS];
pub static EVENT_MAX_X100: [AtomicU64; EVENTS] = [const { AtomicU64::new(0) }; EVENTS];

/// Accessors. Grouped on a unit struct so the module exposes no free
/// functions; the statics stay module-level because Rust has no
/// associated statics.
pub struct TeleportCensus;

impl TeleportCensus {
    /// One ruler for both axes: horizontal units with the height folded
    /// in. See the module header.
    pub fn magnitude(from: Vector3<f32>, to: Vector3<f32>) -> f32 {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let dz = (to.z - from.z) * UNITS_PER_METRE;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    /// Book whatever moved the ball between the last checkpoint and this
    /// one against `stage`, and return the ball's position so the caller
    /// can carry it to the next checkpoint.
    ///
    /// `explained` is how far the ball was entitled to travel on its own —
    /// zero everywhere except [`STAGE_BALL_UPDATE`], because nothing
    /// outside `Ball::update` integrates.
    pub fn note(
        stage: usize,
        from: Vector3<f32>,
        to: Vector3<f32>,
        explained: f32,
        dead: bool,
    ) -> Vector3<f32> {
        let Some(slot) = JUMPS.get(stage) else {
            return to;
        };
        let moved = Self::magnitude(from, to);
        let jump = moved - explained.max(0.0);
        if jump <= TOLERANCE {
            return to;
        }
        slot.fetch_add(1, Ordering::Relaxed);
        let x100 = (jump * 100.0) as u64;
        SUM_X100[stage].fetch_add(x100, Ordering::Relaxed);
        MAX_X100[stage].fetch_max(x100, Ordering::Relaxed);
        if jump >= VISIBLE {
            VISIBLE_JUMPS[stage].fetch_add(1, Ordering::Relaxed);
        }
        if dead {
            DEAD_JUMPS[stage].fetch_add(1, Ordering::Relaxed);
        }
        // "Purely vertical" is generous on purpose — a drop onto a head
        // that drifts a unit sideways is still a height snap.
        let horizontal = (to.x - from.x).hypot(to.y - from.y);
        if horizontal <= TOLERANCE {
            VERTICAL_JUMPS[stage].fetch_add(1, Ordering::Relaxed);
            if jump >= VISIBLE {
                VISIBLE_VERTICAL[stage].fetch_add(1, Ordering::Relaxed);
            }
        }
        to
    }

    /// A checkpoint that needs no arithmetic: everything after
    /// `Ball::update` in a tick.
    pub fn checkpoint(
        stage: usize,
        from: Vector3<f32>,
        to: Vector3<f32>,
        dead: bool,
    ) -> Vector3<f32> {
        Self::note(stage, from, to, 0.0, dead)
    }

    /// A carried ball is dragged to its owner's feet rather than
    /// integrated, so `move_to` is entitled to one tracking step on top of
    /// the velocity. Same floor `flight_diag` gives its own `move_to`
    /// stage, so the two tables can be read against each other.
    pub const OWNER_TRACK_STEP: f32 = 1.5;

    /// The ball's own pass — the one place a position change can be
    /// legitimate movement rather than a write.
    ///
    /// `entry` and `exit` are the velocities either side of it; the bound
    /// is the larger of the two because a stage inside the update may have
    /// changed the velocity as well as the position.
    pub fn note_ball_update(
        stage: usize,
        from: Vector3<f32>,
        to: Vector3<f32>,
        entry: Vector3<f32>,
        exit: Vector3<f32>,
        dead: bool,
    ) -> Vector3<f32> {
        let bound = |v: Vector3<f32>| {
            let dz = v.z * UNITS_PER_METRE;
            (v.x * v.x + v.y * v.y + dz * dz).sqrt()
        };
        let allowance = bound(entry).max(bound(exit)).max(Self::OWNER_TRACK_STEP);
        Self::note(stage, from, to, allowance, dead)
    }

    /// Count a tick, whichever kind it was. The denominator for every row.
    pub fn note_tick() {
        TICKS.fetch_add(1, Ordering::Relaxed);
    }

    /// Book a `dispatch`-stage relocation against the event that was being
    /// handled when it happened. `event` is `PlayerEvent::census_slot`.
    pub fn note_event(event: usize, jump: f32) {
        let Some(slot) = EVENT_JUMPS.get(event) else {
            return;
        };
        if jump <= TOLERANCE {
            return;
        }
        slot.fetch_add(1, Ordering::Relaxed);
        let x100 = (jump * 100.0) as u64;
        EVENT_SUM_X100[event].fetch_add(x100, Ordering::Relaxed);
        EVENT_MAX_X100[event].fetch_max(x100, Ordering::Relaxed);
    }

    /// Per-stage `(count, mean_units, max_units, visible, dead,
    /// visible_and_vertical)`.
    pub fn snapshot() -> [(u64, f32, f32, u64, u64, u64); N] {
        std::array::from_fn(|i| {
            let n = JUMPS[i].load(Ordering::Relaxed);
            let mean = if n == 0 {
                0.0
            } else {
                SUM_X100[i].load(Ordering::Relaxed) as f32 / 100.0 / n as f32
            };
            (
                n,
                mean,
                MAX_X100[i].load(Ordering::Relaxed) as f32 / 100.0,
                VISIBLE_JUMPS[i].load(Ordering::Relaxed),
                DEAD_JUMPS[i].load(Ordering::Relaxed),
                VISIBLE_VERTICAL[i].load(Ordering::Relaxed),
            )
        })
    }

    /// Per-event `(count, mean_units, max_units)` inside `dispatch`.
    pub fn event_snapshot() -> [(u64, f32, f32); EVENTS] {
        std::array::from_fn(|i| {
            let n = EVENT_JUMPS[i].load(Ordering::Relaxed);
            let mean = if n == 0 {
                0.0
            } else {
                EVENT_SUM_X100[i].load(Ordering::Relaxed) as f32 / 100.0 / n as f32
            };
            (
                n,
                mean,
                EVENT_MAX_X100[i].load(Ordering::Relaxed) as f32 / 100.0,
            )
        })
    }

    pub fn ticks() -> u64 {
        TICKS.load(Ordering::Relaxed)
    }

    /// One aerial contest whose ball was given an arc to fly.
    pub fn note_delivery_armed(flight_ticks: u32) {
        DELIVERIES[0].fetch_add(1, Ordering::Relaxed);
        DELIVERIES[3].fetch_add(flight_ticks as u64, Ordering::Relaxed);
    }

    /// It reached the winner, `gap` units from him.
    pub fn note_delivery_arrived(gap: f32) {
        DELIVERIES[1].fetch_add(1, Ordering::Relaxed);
        DELIVERIES[4].fetch_add((gap.max(0.0) * 100.0) as u64, Ordering::Relaxed);
    }

    /// It did not — the deadline passed, or the winner left the field.
    pub fn note_delivery_lost() {
        DELIVERIES[2].fetch_add(1, Ordering::Relaxed);
    }

    /// `(armed, arrived, lost, mean flight ticks, mean arrival gap in units)`.
    pub fn delivery_snapshot() -> (u64, u64, u64, f32, f32) {
        let armed = DELIVERIES[0].load(Ordering::Relaxed);
        let arrived = DELIVERIES[1].load(Ordering::Relaxed);
        (
            armed,
            arrived,
            DELIVERIES[2].load(Ordering::Relaxed),
            DELIVERIES[3].load(Ordering::Relaxed) as f32 / armed.max(1) as f32,
            DELIVERIES[4].load(Ordering::Relaxed) as f32 / 100.0 / arrived.max(1) as f32,
        )
    }

    pub fn reset() {
        for i in 0..N {
            JUMPS[i].store(0, Ordering::Relaxed);
            SUM_X100[i].store(0, Ordering::Relaxed);
            MAX_X100[i].store(0, Ordering::Relaxed);
            VISIBLE_JUMPS[i].store(0, Ordering::Relaxed);
            DEAD_JUMPS[i].store(0, Ordering::Relaxed);
            VERTICAL_JUMPS[i].store(0, Ordering::Relaxed);
            VISIBLE_VERTICAL[i].store(0, Ordering::Relaxed);
        }
        for c in DELIVERIES.iter() {
            c.store(0, Ordering::Relaxed);
        }
        for i in 0..EVENTS {
            EVENT_JUMPS[i].store(0, Ordering::Relaxed);
            EVENT_SUM_X100[i].store(0, Ordering::Relaxed);
            EVENT_MAX_X100[i].store(0, Ordering::Relaxed);
        }
        TICKS.store(0, Ordering::Relaxed);
    }
}

/// What became of the aerial deliveries the contests solved arcs for:
/// `(armed, arrived, timed out, Σ flight ticks, Σ gap at arrival ×100)`.
///
/// The contest's win rate is measured elsewhere and is unchanged by the
/// flight; this is the question the flight introduces and nothing else
/// asks — **does the ball actually get to the man who won it?** A delivery
/// that times out has moved the artefact rather than removed it: the ball
/// no longer teleports onto his head, and he never heads it either.
pub static DELIVERIES: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

/// Splits the ball's own pass into its three exits.
///
/// [`Ball::update`](crate::r#match::engine::ball::ball::Ball::update) returns early twice — for a ball
/// in the netting and for a ball waiting on a restart — and both returns
/// are above the point `flight_diag`'s probe starts booking.
/// `update_light` has no probe at all. So this opens at the top of both
/// and closes on whichever exit is taken, which is what puts the netting
/// and the restart machinery into a census for the first time.
pub struct BallPass {
    pos: Vector3<f32>,
    entry_velocity: Vector3<f32>,
    dead: bool,
}

impl BallPass {
    pub fn open(ball: &crate::r#match::engine::ball::ball::Ball) -> Self {
        Self {
            pos: ball.position,
            entry_velocity: ball.velocity,
            dead: ball.awaiting_restart.is_some(),
        }
    }

    /// Book this pass against the exit it took. Consumes nothing — the
    /// three exits are mutually exclusive by construction.
    pub fn close(&self, ball: &crate::r#match::engine::ball::ball::Ball, stage: usize) {
        TeleportCensus::note_ball_update(
            stage,
            self.pos,
            ball.position,
            self.entry_velocity,
            ball.velocity,
            self.dead,
        );
    }
}

/// Labels for [`TeleportCensus::event_snapshot`], indexed by
/// `PlayerEvent`'s own discriminant order.
pub const EVENT_LABELS: [&str; EVENTS] = [
    "Goal",
    "Assist",
    "BallCollision",
    "TacklingBall",
    "BallOwnerChange",
    "PassTo",
    "ClearBall",
    "RushOut",
    "Shoot",
    "MovePlayer",
    "Leap",
    "StayInGoal",
    "MoveBall",
    "CommunicateMessage",
    "OfferSupport",
    "ClaimBall",
    "GainBall",
    "CaughtBall",
    "ParriedBall",
    "CommitFoul",
    "Offside",
    "RequestHeading",
    "RequestShot",
    "RequestBallReceive",
    "TakeBall",
    "?25",
    "?26",
    "?27",
    "?28",
    "?29",
    "?30",
    "?31",
];

// ─────────────────────────────────────────────────────────────────────────
// PLAYERS
// ─────────────────────────────────────────────────────────────────────────

/// The same census, for the twenty-two.
///
/// # Why this is the bigger number
///
/// The ball census above was built to answer a report about the BALL
/// teleporting on corners. It did — and then a probe at the two set-piece
/// write sites said that on the same corner the ball moved 16 m while
/// **seventeen players moved a mean of 30 m each**, some of them 84 m. On
/// that one restart the ball was **2.5%** of what a viewer was watching
/// move. A relocation census that only watches the ball is measuring the
/// small half.
///
/// # The measurement is exact for the same reason
///
/// A player's position is advanced in exactly one place —
/// [`MatchPlayer::move_to`](crate::r#match::MatchPlayer::move_to), which
/// integrates his velocity — plus the clamp in `check_boundary_collision`,
/// which cannot move him further than the step that took him out. So every
/// other assignment to `position` is a write, and the delta IS the
/// relocation.
///
/// Top speed is about 0.6 u/tick, so [`TOLERANCE`] is generous here rather
/// than tight: anything a census books is already several times a stride.
pub const PLAYER_SITES: [&str; 12] = [
    "restart_reset:whole formation",
    "period:2nd-half/ET shape (SEEN)",
    "period:1st-half end (never sampled)",
    "kickoff:taker onto the ball",
    "set_piece:taker onto the ball",
    "corner:written into the shape",
    "substitution:entering",
    "sent_off:stashed off-pitch",
    "MovePlayer event",
    "boundary_clamp",
    "emergency keeper into goal",
    "NaN salvage (must be 0)",
];

pub const PSITE_GOAL_RESET: usize = 0;
pub const PSITE_PERIOD_RESET: usize = 1;
pub const PSITE_PERIOD_DEAD: usize = 2;
pub const PSITE_KICKOFF_TAKER: usize = 3;
pub const PSITE_SET_PIECE: usize = 4;
pub const PSITE_CORNER_STATION: usize = 5;
pub const PSITE_SUBSTITUTION: usize = 6;
pub const PSITE_SENT_OFF: usize = 7;
pub const PSITE_MOVE_PLAYER: usize = 8;
pub const PSITE_BOUNDARY: usize = 9;
pub const PSITE_EMERGENCY_KEEPER: usize = 10;
pub const PSITE_NAN_SALVAGE: usize = 11;

/// The sites that legitimately re-form the teams rather than being a bug:
/// a substitute coming on, a sent-off player leaving the field of play,
/// and the reset at a PERIOD boundary — half-time, the start of extra
/// time. Reported apart from the rest, exactly as
/// `flight_diag::RESTART_STAGES` separates a throw-in from a stall.
///
/// ⚠ A period boundary is on this list and a goal is NOT, and the
/// difference is the whole point of splitting them. At half-time the
/// teams walk off, change ends and come back out; nobody scrubbing a
/// replay expects position continuity across that, and there is no
/// in-match window to walk them through it. After a GOAL there is such a
/// window — `GoalCelebration` runs for 45-75 s and already walks the cast
/// back to their formation spots — so anybody still being written there
/// when it ends is somebody the celebration failed to bring home.
///
/// # ⚠ The period boundary is TWO writes and only ONE is ever seen
///
/// `StateManager::handle_state_finish` resets the formation twice, ten
/// milliseconds of match clock apart, with `swap_squads` in between:
///
/// * `manager.rs`'s `FirstHalf` arm writes everybody to his own,
///   **un-mirrored** slot — and **nothing samples it**. `increment_time`
///   returns `false` for `HalfTime`, so the half-time `play_inner` runs
///   its loop body zero times and `write_match_positions` is never
///   reached. This write's result is overwritten before any recording
///   exists. That is [`PSITE_PERIOD_DEAD`], and it is booked separately
///   precisely so it stops inflating the row that matters: measured, it
///   is ~519 m/match of movement no viewer can ever see.
/// * the `HalfTime` arm then writes everybody to the **mirrored** slot,
///   which is the one cut a viewer does see — a mean of about 61 m for
///   twenty-two men on one frame, ~1336 m/match and about 89% of all
///   visible player relocation in the engine.
///
/// The second one stays on this list, and the argument for leaving it is
/// football rather than convenience: at half-time the players walk off,
/// go down the tunnel and come back out at the other end. There is no
/// continuous motion between the last frame of the first half and the
/// first of the second, so a walk would be inventing movement that does
/// not happen. What the REPLAY needs is a period-boundary marker, so a
/// viewer scrubbing across the interval is told it is a break instead of
/// watching twenty-two men blink across the halfway line. That is a
/// `src/match` change, not an engine one.
pub const PLAYER_EXPECTED: [usize; 5] = [
    PSITE_PERIOD_RESET,
    PSITE_PERIOD_DEAD,
    PSITE_SUBSTITUTION,
    PSITE_SENT_OFF,
    PSITE_EMERGENCY_KEEPER,
];

const PN: usize = PLAYER_SITES.len();

pub static PJUMPS: [AtomicU64; PN] = [const { AtomicU64::new(0) }; PN];
pub static PSUM_X100: [AtomicU64; PN] = [const { AtomicU64::new(0) }; PN];
pub static PMAX_X100: [AtomicU64; PN] = [const { AtomicU64::new(0) }; PN];
/// Firings, as opposed to players moved — one goal reset is a single
/// event that relocates twenty-two men, and the two numbers answer
/// different questions.
pub static PFIRINGS: [AtomicU64; PN] = [const { AtomicU64::new(0) }; PN];

pub struct PlayerTeleportCensus;

impl PlayerTeleportCensus {
    /// Book one player moved by `site`, from `from` to `to`.
    pub fn note(site: usize, from: Vector3<f32>, to: Vector3<f32>) {
        let Some(slot) = PJUMPS.get(site) else {
            return;
        };
        let moved = (to.x - from.x).hypot(to.y - from.y);
        if moved <= TOLERANCE {
            return;
        }
        slot.fetch_add(1, Ordering::Relaxed);
        let x100 = (moved * 100.0) as u64;
        PSUM_X100[site].fetch_add(x100, Ordering::Relaxed);
        PMAX_X100[site].fetch_max(x100, Ordering::Relaxed);
    }

    /// Book one occurrence of `site`, however many players it moved.
    pub fn note_firing(site: usize) {
        if let Some(slot) = PFIRINGS.get(site) {
            slot.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Per-site `(players moved, mean units, worst units, firings)`.
    pub fn snapshot() -> [(u64, f32, f32, u64); PN] {
        std::array::from_fn(|i| {
            let n = PJUMPS[i].load(Ordering::Relaxed);
            let mean = if n == 0 {
                0.0
            } else {
                PSUM_X100[i].load(Ordering::Relaxed) as f32 / 100.0 / n as f32
            };
            (
                n,
                mean,
                PMAX_X100[i].load(Ordering::Relaxed) as f32 / 100.0,
                PFIRINGS[i].load(Ordering::Relaxed),
            )
        })
    }

    pub fn reset() {
        for i in 0..PN {
            PJUMPS[i].store(0, Ordering::Relaxed);
            PSUM_X100[i].store(0, Ordering::Relaxed);
            PMAX_X100[i].store(0, Ordering::Relaxed);
            PFIRINGS[i].store(0, Ordering::Relaxed);
        }
    }
}
