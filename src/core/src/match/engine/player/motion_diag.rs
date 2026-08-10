//! Runtime motion / state-churn tracer.
//!
//! Two failure modes are invisible to every event-based diagnostic in the
//! engine, because neither produces an event:
//!
//!   1. **Position flicker** — the player burns ground every tick yet ends
//!      the second where they started. On screen it reads as a twitch, a
//!      shiver, a player vibrating on the spot. Mechanically it is a
//!      steering vector that reverses sign between ticks: two targets
//!      fighting, or an `Arrive` that overshoots and is pulled back.
//!
//!   2. **State looping** — `A -> B -> A -> B` at tick cadence. The
//!      transition GRAPH cannot see this: it dedups edges, so an
//!      oscillation and a once-per-match transition look identical. What
//!      separates them is DWELL — how long the player actually stayed.
//!
//! This module measures both at simulation-tick resolution while the match
//! runs. Per-tick accumulation lives on the player itself
//! ([`MotionTrace`]), so the hot path takes no lock; the globals below are
//! touched only when a one-second window closes or a state transition
//! fires — roughly one write per player per second, plus transitions.
//!
//! Compiled only under `--features match-logs`; production builds never
//! see it (see the `#[cfg]` on the `pub mod` in `player/mod.rs`).

use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use nalgebra::Vector3;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Simulation ticks per motion window. The engine ticks at 10 ms, so 100
/// ticks is one second of match time — long enough that a purposeful run
/// covers real ground, short enough that a genuine stop-turn-go does not
/// average out into a false twitch.
pub const WINDOW_TICKS: u32 = 100;

/// Metres per field unit (pitch is 840x545 units ≈ 105x68 m).
pub const M_PER_UNIT: f32 = 0.125;

/// A window counts as a TWITCH when the player covered at least this much
/// ground but finished within `TWITCH_NET_M` of where they started. 1.5 m
/// of path is a deliberate movement; ending 0.30 m away means none of it
/// went anywhere.
pub const TWITCH_PATH_M: f32 = 1.5;
pub const TWITCH_NET_M: f32 = 0.30;

/// Per-tick step below which the player counts as stationary
/// (0.02 u/tick = 0.25 m/s).
pub const STILL_STEP_U: f32 = 0.02;

/// Minimum per-tick speed for a velocity sample to take part in the
/// reversal test — below this the direction is noise, not a decision.
pub const REVERSAL_MIN_SPEED_U: f32 = 0.05;

/// Per-player accumulator carried ON the player. Lock-free by
/// construction: only the owning player writes it, once per tick.
#[derive(Debug, Clone)]
pub struct MotionTrace {
    /// Position at the start of the open window.
    pub win_start: Vector3<f32>,
    /// Position on the previous tick, for the per-tick step length.
    pub prev_pos: Vector3<f32>,
    /// Velocity on the previous tick, for the direction-reversal test.
    pub prev_velocity: Vector3<f32>,
    /// Ground covered inside the open window, in field units.
    pub win_path: f32,
    /// Ticks elapsed in the open window.
    pub win_ticks: u32,
    /// Direction reversals inside the open window.
    pub win_reversals: u32,
    /// Near-stationary ticks inside the open window.
    pub win_still: u32,
    /// Of `win_reversals`, those that happened while the player stayed in
    /// ONE state — the state's own velocity fn reversed under it. The
    /// rest coincided with a state change, i.e. two states pulling in
    /// opposite directions. Separating them says whether the flicker is
    /// a decision problem or a steering problem.
    pub win_reversals_in_state: u32,
    /// State occupied on the previous sampled tick.
    pub last_state: Option<PlayerState>,
    /// State changes seen inside the open window.
    pub win_state_changes: u32,
    /// Whether any tick has been observed yet (first tick seeds the window
    /// rather than measuring a step from a zeroed position).
    pub seeded: bool,
}

impl Default for MotionTrace {
    fn default() -> Self {
        MotionTrace {
            win_start: Vector3::zeros(),
            prev_pos: Vector3::zeros(),
            prev_velocity: Vector3::zeros(),
            win_path: 0.0,
            win_ticks: 0,
            win_reversals: 0,
            win_still: 0,
            win_reversals_in_state: 0,
            last_state: None,
            win_state_changes: 0,
            seeded: false,
        }
    }
}

/// Rolled-up motion behaviour for one player across the run.
#[derive(Debug, Clone, Default)]
pub struct PlayerMotion {
    pub group: u8,
    pub windows: u64,
    pub twitch_windows: u64,
    pub path_u: f64,
    pub net_u: f64,
    pub reversals: u64,
    pub reversals_in_state: u64,
    pub state_changes: u64,
    pub still_ticks: u64,
    pub ticks: u64,
    /// Worst single window seen: (match-time ms, path m, net m, state).
    pub worst: Option<(u64, f32, f32, PlayerState)>,
    /// Transitions taken, and how many of those bounced straight back.
    pub transitions: u64,
    pub ping_pongs: u64,
    pub self_transitions: u64,
    /// Transitions that left a state after 0 or 1 AI ticks.
    pub instant_exits: u64,
}

/// Dwell behaviour of one state, aggregated over every visit.
#[derive(Debug, Clone, Default)]
pub struct StateDwell {
    pub exits: u64,
    pub dwell_sum: u64,
    pub le1: u64,
    pub le3: u64,
    pub max: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub players: BTreeMap<u32, PlayerMotion>,
    /// Dwell per state, keyed by `PlayerState::compact_id`.
    pub dwell: BTreeMap<u16, (StateDwell, PlayerState)>,
    /// `A -> B -> A` bounces, keyed by (A, B, source of the return leg).
    pub ping_pong: BTreeMap<(u16, u16, TransitionSource), (u64, PlayerState, PlayerState)>,
    /// `A -> A` transitions, keyed by (state, source). These reset
    /// `in_state_time` on a state the player never left, so every
    /// `in_state_time > N` timeout in that state is unreachable.
    pub self_edges: BTreeMap<(u16, TransitionSource), (u64, PlayerState)>,
}

static STORE: Mutex<Option<Snapshot>> = Mutex::new(None);

/// In-state velocity reversals per state, indexed by
/// `PlayerState::compact_id` (max 418). Lock-free so the per-tick sampler
/// can attribute every reversal to the exact state whose velocity fn
/// produced it — the "which steering code is flickering" question the
/// per-player table can only hint at.
const REV_SLOTS: usize = 512;
#[allow(clippy::declare_interior_mutable_const)]
const REV_ZERO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static REV_BY_STATE: [std::sync::atomic::AtomicU64; REV_SLOTS] = [REV_ZERO; REV_SLOTS];
/// Ticks spent in each state, so a reversal count can be read as a rate
/// rather than as "this state is simply occupied a lot".
pub static TICKS_BY_STATE: [std::sync::atomic::AtomicU64; REV_SLOTS] = [REV_ZERO; REV_SLOTS];

/// Record one sampled tick in `state`, and whether it reversed direction.
#[inline]
pub fn note_tick(state: PlayerState, reversed_in_state: bool) {
    use std::sync::atomic::Ordering;
    let slot = state.compact_id() as usize;
    if slot >= REV_SLOTS {
        return;
    }
    TICKS_BY_STATE[slot].fetch_add(1, Ordering::Relaxed);
    if reversed_in_state {
        REV_BY_STATE[slot].fetch_add(1, Ordering::Relaxed);
    }
}

/// Drop everything recorded so far. Call between measurement batches.
pub fn reset() {
    use std::sync::atomic::Ordering;
    *STORE.lock().unwrap() = Some(Snapshot::default());
    for i in 0..REV_SLOTS {
        REV_BY_STATE[i].store(0, Ordering::Relaxed);
        TICKS_BY_STATE[i].store(0, Ordering::Relaxed);
    }
}

/// Copy out the accumulated trace.
pub fn snapshot() -> Snapshot {
    STORE.lock().unwrap().clone().unwrap_or_default()
}

/// Close one motion window for a player.
#[allow(clippy::too_many_arguments)]
pub fn record_window(
    player_id: u32,
    group: u8,
    t_ms: u64,
    state: PlayerState,
    path_u: f32,
    net_u: f32,
    reversals: u32,
    reversals_in_state: u32,
    state_changes: u32,
    still: u32,
    ticks: u32,
) {
    let path_m = path_u * M_PER_UNIT;
    let net_m = net_u * M_PER_UNIT;
    let twitch = path_m >= TWITCH_PATH_M && net_m < TWITCH_NET_M;

    let mut guard = STORE.lock().unwrap();
    let store = guard.get_or_insert_with(Snapshot::default);
    let e = store.players.entry(player_id).or_default();
    e.group = group;
    e.windows += 1;
    e.path_u += path_u as f64;
    e.net_u += net_u as f64;
    e.reversals += reversals as u64;
    e.reversals_in_state += reversals_in_state as u64;
    e.state_changes += state_changes as u64;
    e.still_ticks += still as u64;
    e.ticks += ticks as u64;
    if twitch {
        e.twitch_windows += 1;
        // Keep the most extreme episode: most ground burnt for least
        // progress. Gives a timestamp to scrub to in the replay viewer.
        let score = path_m - net_m;
        let better = e
            .worst
            .map(|(_, p, n, _)| score > p - n)
            .unwrap_or(true);
        if better {
            e.worst = Some((t_ms, path_m, net_m, state));
        }
    }
}

/// Record one state transition with the dwell it is leaving behind.
pub fn record_transition(
    player_id: u32,
    from: PlayerState,
    to: PlayerState,
    dwell_ai_ticks: u64,
    source: TransitionSource,
    prev_state: Option<PlayerState>,
) {
    let mut guard = STORE.lock().unwrap();
    let store = guard.get_or_insert_with(Snapshot::default);

    let p = store.players.entry(player_id).or_default();
    p.transitions += 1;
    if dwell_ai_ticks <= 1 {
        p.instant_exits += 1;
    }

    if from.compact_id() == to.compact_id() {
        p.self_transitions += 1;
        let e = store
            .self_edges
            .entry((from.compact_id(), source))
            .or_insert((0, from));
        e.0 += 1;
        return;
    }

    // Dwell only means something for a state actually left.
    let d = store
        .dwell
        .entry(from.compact_id())
        .or_insert_with(|| (StateDwell::default(), from));
    d.0.exits += 1;
    d.0.dwell_sum += dwell_ai_ticks;
    if dwell_ai_ticks <= 1 {
        d.0.le1 += 1;
    }
    if dwell_ai_ticks <= 3 {
        d.0.le3 += 1;
    }
    d.0.max = d.0.max.max(dwell_ai_ticks);

    if prev_state.map(|s| s.compact_id()) == Some(to.compact_id()) {
        store.players.entry(player_id).or_default().ping_pongs += 1;
        let key = (to.compact_id(), from.compact_id(), source);
        let e = store.ping_pong.entry(key).or_insert((0, to, from));
        e.0 += 1;
    }
}
