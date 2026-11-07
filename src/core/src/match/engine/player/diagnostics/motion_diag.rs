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
//! see it (see the `#[cfg]` on the `pub mod` in `player/diagnostics/mod.rs`).

use crate::r#match::ActivityIntensity;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use nalgebra::Vector3;
use std::collections::BTreeMap;
use std::panic::Location;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

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
    /// Rolling window of the last few sampled ticks, `(position,
    /// velocity)`, oldest-to-newest via `ring_idx`.
    ///
    /// Exists so a reversal can be shown in CONTEXT rather than as a
    /// count. Five separate hypotheses about `Defender: Take Ball` were
    /// derived by reading the steering code and every one measured within
    /// noise; the only way left to see what the velocity is actually
    /// doing is to print it.
    pub ring: [(Vector3<f32>, Vector3<f32>); RING],
    pub ring_idx: usize,
    /// The last two states left, for the three-cycle census.
    pub chain: TransitionChain,
    /// AI ticks left of the half second after a missed challenge.
    pub after_miss_ticks: u32,
    /// The state whose handler made this tick's decision. A foul moves its
    /// man on in the same result, so `state` reads the destination by the
    /// time the event is dispatched.
    pub decided_in: Option<PlayerState>,
    /// Whole-match totals, folded into the match tally at the whistle.
    pub total_path_u: f64,
    pub total_ticks: u64,
    pub total_state_changes: u64,
}

/// Ticks of context kept either side of a captured reversal.
pub const RING: usize = 6;

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
            ring: [(Vector3::zeros(), Vector3::zeros()); RING],
            ring_idx: 0,
            chain: TransitionChain::default(),
            after_miss_ticks: 0,
            decided_in: None,
            total_path_u: 0.0,
            total_ticks: 0,
            total_state_changes: 0,
        }
    }
}

/// One captured reversal, with the ticks either side of it.
#[derive(Debug, Clone)]
pub struct ReversalEpisode {
    pub t_ms: u64,
    pub player_id: u32,
    /// `(position, velocity)` oldest-to-newest; the reversal is at the end.
    pub samples: Vec<(Vector3<f32>, Vector3<f32>)>,
    pub ball_pos: Vector3<f32>,
    pub ball_vel: Vector3<f32>,
}

/// Captured episodes per state, capped so the hot path stays cheap.
static EPISODES: Mutex<Option<BTreeMap<u16, Vec<ReversalEpisode>>>> = Mutex::new(None);
const MAX_EPISODES_PER_STATE: usize = 6;

/// Record a fast in-state reversal with its surrounding ticks.
pub fn capture_reversal(
    state: PlayerState,
    t_ms: u64,
    player_id: u32,
    samples: Vec<(Vector3<f32>, Vector3<f32>)>,
    ball_pos: Vector3<f32>,
    ball_vel: Vector3<f32>,
) {
    let mut guard = EPISODES.lock().unwrap();
    let map = guard.get_or_insert_with(BTreeMap::new);
    let slot = map.entry(state.compact_id()).or_default();
    if slot.len() >= MAX_EPISODES_PER_STATE {
        return;
    }
    slot.push(ReversalEpisode {
        t_ms,
        player_id,
        samples,
        ball_pos,
        ball_vel,
    });
}

/// Copy out the captured episodes.
pub fn episodes() -> BTreeMap<u16, Vec<ReversalEpisode>> {
    EPISODES.lock().unwrap().clone().unwrap_or_default()
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
    /// Every observed transition, keyed by (from, to, source).
    pub edges: BTreeMap<(u16, u16, TransitionSource), EdgeStat>,
    /// `A -> B -> C -> A` cycles, keyed by (A, B, C). Each rotation of a
    /// cycle is its own key, so one loop can count up to three times.
    pub three_cycles: BTreeMap<(u16, u16, u16), u64>,
    /// An override moved the player and his handler moved him again in the
    /// same tick, keyed by (before, after override, after handler).
    pub double_hops: BTreeMap<(u16, u16, u16), u64>,
}

impl Snapshot {
    /// Exits of each state, summed over every edge leaving it.
    pub fn exits_by_state(&self) -> BTreeMap<u16, EdgeStat> {
        let mut out: BTreeMap<u16, EdgeStat> = BTreeMap::new();
        for ((from, _, _), edge) in &self.edges {
            out.entry(*from).or_default().merge(edge);
        }
        out
    }
}

static STORE: Mutex<Option<Snapshot>> = Mutex::new(None);

/// Upper bound of each dwell bin, in AI ticks.
pub const DWELL_BINS: [u64; 8] = [1, 5, 10, 25, 50, 100, 250, u64::MAX];
pub const DWELL_BIN_LABELS: [&str; 8] = [
    "<=1", "2-5", "6-10", "11-25", "26-50", "51-100", "101-250", ">250",
];

/// How one observed transition behaves: how often it fires, how long the
/// player had been in the state he left, and how often it undid the
/// transition before it. A guard that only rate-limits an oscillation shows
/// here as a spike in the bin holding its limit.
#[derive(Debug, Clone, Copy, Default)]
pub struct EdgeStat {
    pub n: u64,
    pub dwell_sum: u64,
    pub bins: [u64; DWELL_BINS.len()],
    pub bounces: u64,
}

impl EdgeStat {
    fn note(&mut self, dwell_ai_ticks: u64, bounced: bool) {
        self.n += 1;
        self.dwell_sum += dwell_ai_ticks;
        self.bins[Self::bin(dwell_ai_ticks)] += 1;
        if bounced {
            self.bounces += 1;
        }
    }

    fn merge(&mut self, other: &EdgeStat) {
        self.n += other.n;
        self.dwell_sum += other.dwell_sum;
        self.bounces += other.bounces;
        for (mine, theirs) in self.bins.iter_mut().zip(other.bins.iter()) {
            *mine += theirs;
        }
    }

    pub fn bin(dwell_ai_ticks: u64) -> usize {
        DWELL_BINS
            .iter()
            .position(|upper| dwell_ai_ticks <= *upper)
            .unwrap_or(DWELL_BINS.len() - 1)
    }

    pub fn mean_dwell(&self) -> f64 {
        self.dwell_sum as f64 / self.n.max(1) as f64
    }
}

/// One player's last two exits, enough to see a three-state loop that no
/// two-state reversal test can.
#[derive(Debug, Clone, Copy, Default)]
pub struct TransitionChain {
    recent: [(u16, u64); 2],
    len: u8,
}

impl TransitionChain {
    /// A loop slower than this, in AI ticks through its middle two states,
    /// is the player going somewhere rather than the machine cycling.
    const CYCLE_WINDOW_TICKS: u64 = 50;

    /// Record leaving `from` for `to`. Returns the `(A, B, C)` of the
    /// `A -> B -> C -> A` loop this closes, if it closes one.
    pub fn push(&mut self, from: u16, to: u16, dwell_ai_ticks: u64) -> Option<(u16, u16, u16)> {
        let mut closed = None;
        if self.len == 2 {
            let (a, _) = self.recent[0];
            let (b, b_dwell) = self.recent[1];
            if a == to
                && b != to
                && b != from
                && a != from
                && b_dwell + dwell_ai_ticks <= Self::CYCLE_WINDOW_TICKS
            {
                closed = Some((a, b, from));
            }
        }
        self.recent[0] = self.recent[1];
        self.recent[1] = (from, dwell_ai_ticks);
        self.len = (self.len + 1).min(2);
        closed
    }
}

/// `compact_id` and pseudo-state slots of the per-state tables.
const STATE_SLOTS: usize = 512;
const PAIR_SLOTS: usize = STATE_SLOTS * STATE_SLOTS;
const TIERS: usize = 5;
/// `Take Ball` occupied as the taker of a dead ball, one row per role, so a
/// keeper standing over a goal kick does not read as a keeper who will not
/// chase. Above the 400 band's highest id.
const RESTART_TAKE_BALL_SLOT: usize = 480;

#[allow(clippy::declare_interior_mutable_const)]
const COUNTER_ZERO: AtomicU64 = AtomicU64::new(0);

static STATE_TICKS: [AtomicU64; STATE_SLOTS] = [COUNTER_ZERO; STATE_SLOTS];
static STATE_ON_BALL: [AtomicU64; STATE_SLOTS] = [COUNTER_ZERO; STATE_SLOTS];
static STATE_STILL: [AtomicU64; STATE_SLOTS] = [COUNTER_ZERO; STATE_SLOTS];
static STATE_SPEED_MILLI: [AtomicU64; STATE_SLOTS] = [COUNTER_ZERO; STATE_SLOTS];
static STATE_TIER: [[AtomicU64; TIERS]; STATE_SLOTS] =
    [const { [COUNTER_ZERO; TIERS] }; STATE_SLOTS];
static HOLDS: [AtomicU64; PAIR_SLOTS] = [COUNTER_ZERO; PAIR_SLOTS];
static INJURED_OWNER_TICKS: AtomicU64 = AtomicU64::new(0);
static INJURED_TAKER_TICKS: AtomicU64 = AtomicU64::new(0);
static OUTFIELD_SPEED: [AtomicU64; 2] = [COUNTER_ZERO; 2];
static AFTER_MISS_SPEED: [AtomicU64; 2] = [COUNTER_ZERO; 2];
static REFUSALS: [AtomicU64; PAIR_SLOTS] = [COUNTER_ZERO; PAIR_SLOTS];
static SHORTCUTS: [AtomicU64; PAIR_SLOTS] = [COUNTER_ZERO; PAIR_SLOTS];

/// What one state looks like from the outside, per AI tick spent in it.
#[derive(Debug, Clone)]
pub struct StateTickRow {
    pub label: String,
    /// The `compact_id` whose exits this row shares; `None` on a split row.
    pub compact_id: Option<u16>,
    pub ticks: u64,
    pub on_ball: u64,
    pub still: u64,
    pub speed_milli_sum: u64,
    pub tiers: [u64; TIERS],
}

impl StateTickRow {
    pub fn share(&self, n: u64) -> f64 {
        n as f64 / self.ticks.max(1) as f64 * 100.0
    }

    pub fn mean_speed_ms(&self) -> f64 {
        let u_per_tick = self.speed_milli_sum as f64 / 1000.0 / self.ticks.max(1) as f64;
        u_per_tick * M_PER_UNIT as f64 * 100.0
    }
}

/// The state machine's own census: what no transition graph or two-state
/// reversal test can see. Hot-path counters are atomics so a batch played
/// across threads does not serialise on them.
pub struct StateCensus;

impl StateCensus {
    #[inline]
    fn pair_slot(from: PlayerState, to: PlayerState) -> usize {
        from.compact_id() as usize * STATE_SLOTS + to.compact_id() as usize
    }

    fn pair_rows(table: &[AtomicU64]) -> Vec<(u16, u16, u64)> {
        let mut rows: Vec<(u16, u16, u64)> = table
            .iter()
            .enumerate()
            .filter_map(|(slot, counter)| {
                let n = counter.load(Ordering::Relaxed);
                (n > 0).then_some(((slot / STATE_SLOTS) as u16, (slot % STATE_SLOTS) as u16, n))
            })
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.2));
        rows
    }

    /// A man down who still has the ball or a restart. Both should read zero.
    pub fn note_injured_tick(owner: bool, taker: bool) {
        if owner {
            INJURED_OWNER_TICKS.fetch_add(1, Ordering::Relaxed);
        }
        if taker {
            INJURED_TAKER_TICKS.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `(ball owner, restart taker)` AI ticks spent injured.
    pub fn injured_ticks() -> (u64, u64) {
        (
            INJURED_OWNER_TICKS.load(Ordering::Relaxed),
            INJURED_TAKER_TICKS.load(Ordering::Relaxed),
        )
    }

    /// The half second after a missed challenge, in AI ticks.
    pub const AFTER_MISS_TICKS: u32 = 25;

    /// One outfielder's AI tick, for the speed he runs at after a miss
    /// against the speed the outfield runs at.
    #[inline]
    pub fn note_outfield_speed(step: f32, after_a_miss: bool) {
        let milli = (step * 1000.0) as u64;
        OUTFIELD_SPEED[0].fetch_add(milli, Ordering::Relaxed);
        OUTFIELD_SPEED[1].fetch_add(1, Ordering::Relaxed);
        if after_a_miss {
            AFTER_MISS_SPEED[0].fetch_add(milli, Ordering::Relaxed);
            AFTER_MISS_SPEED[1].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `(outfield mean, after a miss, ticks after a miss)`, m/s.
    pub fn speed_after_a_miss() -> (f64, f64, u64) {
        let mean = |counters: &[AtomicU64; 2]| {
            let n = counters[1].load(Ordering::Relaxed);
            counters[0].load(Ordering::Relaxed) as f64 / 1000.0 / n.max(1) as f64
                * M_PER_UNIT as f64
                * 100.0
        };
        (
            mean(&OUTFIELD_SPEED),
            mean(&AFTER_MISS_SPEED),
            AFTER_MISS_SPEED[1].load(Ordering::Relaxed),
        )
    }

    /// `DecisionCommitment` held this transition back.
    #[inline]
    pub fn note_hold(from: PlayerState, to: PlayerState) {
        HOLDS[Self::pair_slot(from, to)].fetch_add(1, Ordering::Relaxed);
    }

    pub fn holds() -> Vec<(u16, u16, u64)> {
        Self::pair_rows(&HOLDS)
    }

    /// The resolver found `from -> to` would come straight back to where it
    /// had been, and left him in `from`.
    #[inline]
    pub fn note_refusal(from: PlayerState, to: PlayerState) {
        REFUSALS[Self::pair_slot(from, to)].fetch_add(1, Ordering::Relaxed);
    }

    pub fn refusals() -> Vec<(u16, u16, u64)> {
        Self::pair_rows(&REFUSALS)
    }

    /// The resolver took him from `from` straight to `landing`, past states
    /// that would only have handed him on.
    #[inline]
    pub fn note_shortcut(from: PlayerState, landing: PlayerState) {
        SHORTCUTS[Self::pair_slot(from, landing)].fetch_add(1, Ordering::Relaxed);
    }

    pub fn shortcuts() -> Vec<(u16, u16, u64)> {
        Self::pair_rows(&SHORTCUTS)
    }

    pub fn note_double_hop(
        before: PlayerState,
        after_override: PlayerState,
        after_handler: PlayerState,
    ) {
        let mut guard = STORE.lock().unwrap();
        let store = guard.get_or_insert_with(Snapshot::default);
        *store
            .double_hops
            .entry((
                before.compact_id(),
                after_override.compact_id(),
                after_handler.compact_id(),
            ))
            .or_insert(0) += 1;
    }

    #[inline]
    fn tick_slot(state: PlayerState, is_restart_taker: bool) -> usize {
        if is_restart_taker && state.is_take_ball() {
            RESTART_TAKE_BALL_SLOT
                + match state {
                    PlayerState::Goalkeeper(_) => 0,
                    PlayerState::Defender(_) => 1,
                    PlayerState::Midfielder(_) => 2,
                    _ => 3,
                }
        } else {
            state.compact_id() as usize
        }
    }

    /// One AI tick spent in `state`. `step` is the player's speed in units
    /// per simulation tick.
    #[inline]
    pub fn note_tick(
        state: PlayerState,
        is_restart_taker: bool,
        on_ball: bool,
        step: f32,
        tier: ActivityIntensity,
    ) {
        let slot = Self::tick_slot(state, is_restart_taker);
        STATE_TICKS[slot].fetch_add(1, Ordering::Relaxed);
        if on_ball {
            STATE_ON_BALL[slot].fetch_add(1, Ordering::Relaxed);
        }
        if step < STILL_STEP_U {
            STATE_STILL[slot].fetch_add(1, Ordering::Relaxed);
        }
        STATE_SPEED_MILLI[slot].fetch_add((step * 1000.0) as u64, Ordering::Relaxed);
        STATE_TIER[slot][tier as usize].fetch_add(1, Ordering::Relaxed);
    }

    /// One row per state ever occupied, heaviest first. `Take Ball` splits
    /// into a live row and a restart row.
    pub fn state_rows() -> Vec<StateTickRow> {
        let row = |slot: usize, label: String, compact_id: Option<u16>| {
            let mut tiers = [0u64; TIERS];
            for (tier, counter) in tiers.iter_mut().zip(STATE_TIER[slot].iter()) {
                *tier = counter.load(Ordering::Relaxed);
            }
            StateTickRow {
                label,
                compact_id,
                ticks: STATE_TICKS[slot].load(Ordering::Relaxed),
                on_ball: STATE_ON_BALL[slot].load(Ordering::Relaxed),
                still: STATE_STILL[slot].load(Ordering::Relaxed),
                speed_milli_sum: STATE_SPEED_MILLI[slot].load(Ordering::Relaxed),
                tiers,
            }
        };
        let mut rows: Vec<StateTickRow> = PlayerState::all()
            .iter()
            .map(|state| {
                let id = state.compact_id();
                let label = if state.is_take_ball() {
                    format!("{state} (live)")
                } else {
                    state.to_string()
                };
                row(id as usize, label, Some(id))
            })
            .collect();
        for (offset, role) in ["Goalkeeper", "Defender", "Midfielder", "Forward"]
            .iter()
            .enumerate()
        {
            rows.push(row(
                RESTART_TAKE_BALL_SLOT + offset,
                format!("{role}: Take Ball (restart taker)"),
                None,
            ));
        }
        rows.retain(|r| r.ticks > 0);
        rows.sort_by_key(|r| std::cmp::Reverse(r.ticks));
        rows
    }

    fn reset() {
        for table in [
            &STATE_TICKS,
            &STATE_ON_BALL,
            &STATE_STILL,
            &STATE_SPEED_MILLI,
        ] {
            for counter in table.iter() {
                counter.store(0, Ordering::Relaxed);
            }
        }
        for counter in STATE_TIER.iter().flatten() {
            counter.store(0, Ordering::Relaxed);
        }
        INJURED_OWNER_TICKS.store(0, Ordering::Relaxed);
        INJURED_TAKER_TICKS.store(0, Ordering::Relaxed);
        for counter in OUTFIELD_SPEED.iter().chain(AFTER_MISS_SPEED.iter()) {
            counter.store(0, Ordering::Relaxed);
        }
        for table in [&HOLDS, &REFUSALS, &SHORTCUTS] {
            for counter in table.iter() {
                counter.store(0, Ordering::Relaxed);
            }
        }
        DoorCensus::reset();
        POSSESSION_CENSUS.reset();
    }
}

/// Proposals per transition site, whether or not they took effect. Open
/// addressing over `Location::caller()`: every proposal is one relaxed add,
/// and nothing locks on a path that fires a million times a match.
pub struct DoorCensus;

const DOOR_SLOTS: usize = 2048;
static DOOR_SITES: [OnceLock<&'static Location<'static>>; DOOR_SLOTS] =
    [const { OnceLock::new() }; DOOR_SLOTS];
static DOOR_COUNTS: [AtomicU64; DOOR_SLOTS] = [COUNTER_ZERO; DOOR_SLOTS];

impl DoorCensus {
    #[inline]
    pub fn note(site: &'static Location<'static>) {
        let mut slot = Self::home_slot(site);
        loop {
            let held = DOOR_SITES[slot].get_or_init(|| site);
            if std::ptr::eq(*held, site) {
                DOOR_COUNTS[slot].fetch_add(1, Ordering::Relaxed);
                return;
            }
            slot = (slot + 1) % DOOR_SLOTS;
        }
    }

    #[inline]
    fn home_slot(site: &'static Location<'static>) -> usize {
        let address = site as *const Location<'static> as usize;
        (address >> 3).wrapping_mul(0x9E37_79B9) % DOOR_SLOTS
    }

    /// `("path:line", proposals)`, heaviest first. One source line can be
    /// emitted as several `Location`s, so rows merge by name.
    pub fn snapshot() -> Vec<(String, u64)> {
        let mut merged: BTreeMap<String, u64> = BTreeMap::new();
        for (site, count) in DOOR_SITES.iter().zip(DOOR_COUNTS.iter()) {
            let n = count.load(Ordering::Relaxed);
            if let Some(site) = site.get()
                && n > 0
            {
                *merged
                    .entry(format!("{}:{}", site.file(), site.line()))
                    .or_insert(0) += n;
            }
        }
        let mut rows: Vec<(String, u64)> = merged.into_iter().collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.1));
        rows
    }

    fn reset() {
        for count in DOOR_COUNTS.iter() {
            count.store(0, Ordering::Relaxed);
        }
    }
}

/// How players read "does my side have the ball?", sampled at each player's
/// first read per tick. Slots are `id & 4095` and teams `id & 1`: both ids
/// are small and stable in a harness match, and matches must run one at a
/// time for the per-tick comparison to mean anything.
pub struct PossessionCensus {
    last_tick: [AtomicU64; Self::PLAYER_SLOTS],
    last_reading: [AtomicU8; Self::PLAYER_SLOTS],
    team_tick: [AtomicU64; 2],
    team_reading: [AtomicU8; 2],
    flips_owned: AtomicU64,
    flips_unowned: AtomicU64,
    flips_own_pass_in_flight: AtomicU64,
    reads: AtomicU64,
    reads_unowned: AtomicU64,
    disagreeing_team_ticks: AtomicU64,
    unowned_team_ticks: AtomicU64,
    both_claim_ticks: AtomicU64,
    neither_claim_ticks: AtomicU64,
}

/// One copy-out of [`PossessionCensus`], in raw counts.
#[derive(Debug, Clone, Copy, Default)]
pub struct PossessionReadings {
    pub reads: u64,
    pub reads_unowned: u64,
    pub flips_owned: u64,
    pub flips_unowned: u64,
    pub flips_own_pass_in_flight: u64,
    pub disagreeing_team_ticks: u64,
    pub unowned_team_ticks: u64,
    pub both_claim_ticks: u64,
    pub neither_claim_ticks: u64,
}

pub static POSSESSION_CENSUS: PossessionCensus = PossessionCensus::new();

impl PossessionCensus {
    const PLAYER_SLOTS: usize = 4096;
    const NO_READING: u8 = 2;

    pub const fn new() -> Self {
        #[allow(clippy::declare_interior_mutable_const)]
        const UNSEEN: AtomicU8 = AtomicU8::new(PossessionCensus::NO_READING);
        #[allow(clippy::declare_interior_mutable_const)]
        const NEVER: AtomicU64 = AtomicU64::new(u64::MAX);
        PossessionCensus {
            last_tick: [NEVER; Self::PLAYER_SLOTS],
            last_reading: [UNSEEN; Self::PLAYER_SLOTS],
            team_tick: [AtomicU64::new(u64::MAX), AtomicU64::new(u64::MAX)],
            team_reading: [UNSEEN; 2],
            flips_owned: AtomicU64::new(0),
            flips_unowned: AtomicU64::new(0),
            flips_own_pass_in_flight: AtomicU64::new(0),
            reads: AtomicU64::new(0),
            reads_unowned: AtomicU64::new(0),
            disagreeing_team_ticks: AtomicU64::new(0),
            unowned_team_ticks: AtomicU64::new(0),
            both_claim_ticks: AtomicU64::new(0),
            neither_claim_ticks: AtomicU64::new(0),
        }
    }

    pub fn note(
        &self,
        player_id: u32,
        team_id: u32,
        reading: bool,
        ball_owned: bool,
        own_pass_in_flight: bool,
        tick: u64,
    ) {
        let slot = player_id as usize & (Self::PLAYER_SLOTS - 1);
        if self.last_tick[slot].swap(tick, Ordering::Relaxed) == tick {
            return;
        }
        let value = reading as u8;
        let previous = self.last_reading[slot].swap(value, Ordering::Relaxed);
        self.reads.fetch_add(1, Ordering::Relaxed);
        if !ball_owned {
            self.reads_unowned.fetch_add(1, Ordering::Relaxed);
        }
        if previous != Self::NO_READING && previous != value {
            if ball_owned {
                self.flips_owned.fetch_add(1, Ordering::Relaxed);
            } else {
                self.flips_unowned.fetch_add(1, Ordering::Relaxed);
                if own_pass_in_flight {
                    self.flips_own_pass_in_flight
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        let team = team_id as usize & 1;
        let other = 1 - team;
        let team_last_tick = self.team_tick[team].swap(tick, Ordering::Relaxed);
        let team_last_reading = self.team_reading[team].swap(value, Ordering::Relaxed);
        if team_last_tick == tick {
            if team_last_reading != value {
                self.disagreeing_team_ticks.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
        if !ball_owned {
            self.unowned_team_ticks.fetch_add(1, Ordering::Relaxed);
        }
        if self.team_tick[other].load(Ordering::Relaxed) == tick
            && self.team_reading[other].load(Ordering::Relaxed) == value
        {
            let tally = if reading {
                &self.both_claim_ticks
            } else {
                &self.neither_claim_ticks
            };
            tally.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> PossessionReadings {
        let load = |c: &AtomicU64| c.load(Ordering::Relaxed);
        PossessionReadings {
            reads: load(&self.reads),
            reads_unowned: load(&self.reads_unowned),
            flips_owned: load(&self.flips_owned),
            flips_unowned: load(&self.flips_unowned),
            flips_own_pass_in_flight: load(&self.flips_own_pass_in_flight),
            disagreeing_team_ticks: load(&self.disagreeing_team_ticks),
            unowned_team_ticks: load(&self.unowned_team_ticks),
            both_claim_ticks: load(&self.both_claim_ticks),
            neither_claim_ticks: load(&self.neither_claim_ticks),
        }
    }

    fn reset(&self) {
        for tick in self.last_tick.iter() {
            tick.store(u64::MAX, Ordering::Relaxed);
        }
        for reading in self.last_reading.iter() {
            reading.store(Self::NO_READING, Ordering::Relaxed);
        }
        for tick in self.team_tick.iter() {
            tick.store(u64::MAX, Ordering::Relaxed);
        }
        for reading in self.team_reading.iter() {
            reading.store(Self::NO_READING, Ordering::Relaxed);
        }
        for counter in [
            &self.flips_owned,
            &self.flips_unowned,
            &self.flips_own_pass_in_flight,
            &self.reads,
            &self.reads_unowned,
            &self.disagreeing_team_ticks,
            &self.unowned_team_ticks,
            &self.both_claim_ticks,
            &self.neither_claim_ticks,
        ] {
            counter.store(0, Ordering::Relaxed);
        }
    }
}

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

/// Of the in-state reversals, those taken at real running speed.
///
/// `REVERSAL_MIN_SPEED_U` (0.05 u/tick = 0.6 m/s) is deliberately low so
/// the metric catches everything, but that also counts a player settling
/// onto a target — brake, drift back a hair, brake again — as a reversal.
/// That is invisible on screen. A player who inverts direction while
/// actually running is the thing that reads as a twitch, so it gets its
/// own counter: without the split, a state that merely dithers at
/// walking pace and one that genuinely whips at a sprint are the same
/// number, and there is no way to tell which is worth chasing.
pub static REV_FAST_BY_STATE: [std::sync::atomic::AtomicU64; REV_SLOTS] = [REV_ZERO; REV_SLOTS];

/// Speed above which a reversal is a visible change of direction rather
/// than settling jitter. Half of a slow player's top speed (0.36 u/tick).
pub const REVERSAL_FAST_SPEED_U: f32 = 0.18;

/// Direction reversals of the BALL itself, and ticks sampled.
///
/// Every chase state aims at a point derived from the ball's position and
/// velocity, so a ball that keeps inverting its own direction produces
/// chasers that invert theirs — faithfully, not spuriously. Without this
/// number there is no way to tell a steering bug in a chase state from a
/// chaser correctly tracking a jittery ball, and both look identical in
/// the per-state reversal table.
pub static BALL_REVERSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static BALL_TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Sample the ball's own direction stability for one tick.
#[inline]
pub fn note_ball(velocity: Vector3<f32>, prev: Vector3<f32>) {
    BALL_TICKS.fetch_add(1, Ordering::Relaxed);
    // Same test the player sampler uses, on the ball's own 2D travel.
    let flat = Vector3::new(velocity.x, velocity.y, 0.0);
    let flat_prev = Vector3::new(prev.x, prev.y, 0.0);
    if flat.norm() > REVERSAL_MIN_SPEED_U
        && flat_prev.norm() > REVERSAL_MIN_SPEED_U
        && flat.dot(&flat_prev) < 0.0
    {
        BALL_REVERSALS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Record one sampled tick in `state`, and whether it reversed direction.
#[inline]
pub fn note_tick(state: PlayerState, reversed_in_state: bool, fast: bool) {
    let slot = state.compact_id() as usize;
    if slot >= REV_SLOTS {
        return;
    }
    TICKS_BY_STATE[slot].fetch_add(1, Ordering::Relaxed);
    if reversed_in_state {
        REV_BY_STATE[slot].fetch_add(1, Ordering::Relaxed);
        if fast {
            REV_FAST_BY_STATE[slot].fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Drop everything recorded so far. Call between measurement batches.
pub fn reset() {
    *STORE.lock().unwrap() = Some(Snapshot::default());
    *EPISODES.lock().unwrap() = Some(BTreeMap::new());
    BALL_REVERSALS.store(0, Ordering::Relaxed);
    BALL_TICKS.store(0, Ordering::Relaxed);
    for i in 0..REV_SLOTS {
        REV_BY_STATE[i].store(0, Ordering::Relaxed);
        REV_FAST_BY_STATE[i].store(0, Ordering::Relaxed);
        TICKS_BY_STATE[i].store(0, Ordering::Relaxed);
    }
    StateCensus::reset();
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
        let better = e.worst.map(|(_, p, n, _)| score > p - n).unwrap_or(true);
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
    chain: &mut TransitionChain,
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

    let bounced = prev_state.map(|s| s.compact_id()) == Some(to.compact_id());
    store
        .edges
        .entry((from.compact_id(), to.compact_id(), source))
        .or_default()
        .note(dwell_ai_ticks, bounced);
    if let Some(cycle) = chain.push(from.compact_id(), to.compact_id(), dwell_ai_ticks) {
        *store.three_cycles.entry(cycle).or_insert(0) += 1;
    }

    if bounced {
        store.players.entry(player_id).or_default().ping_pongs += 1;
        let key = (to.compact_id(), from.compact_id(), source);
        let e = store.ping_pong.entry(key).or_insert((0, to, from));
        e.0 += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u16 = 1;
    const B: u16 = 2;
    const C: u16 = 3;

    #[test]
    fn a_cycle_through_three_states_is_named_though_nothing_reversed() {
        let mut chain = TransitionChain::default();
        assert_eq!(chain.push(A, B, 1), None);
        assert_eq!(chain.push(B, C, 1), None);
        assert_eq!(chain.push(C, A, 1), Some((A, B, C)));
        // The next leg closes the same loop from its next rotation.
        assert_eq!(chain.push(A, B, 1), Some((B, C, A)));
    }

    #[test]
    fn a_two_state_reversal_is_not_a_three_cycle() {
        let mut chain = TransitionChain::default();
        for (from, to) in [(A, B), (B, A), (A, B), (B, A)] {
            assert_eq!(chain.push(from, to, 1), None);
        }
    }

    #[test]
    fn a_slow_loop_is_not_a_cycle() {
        let mut chain = TransitionChain::default();
        chain.push(A, B, 500);
        chain.push(B, C, 40);
        assert_eq!(chain.push(C, A, 40), None);
    }

    #[test]
    fn a_rate_limited_oscillation_spikes_the_bin_holding_its_limit() {
        let mut edge = EdgeStat::default();
        for _ in 0..50 {
            edge.note(8, true);
        }
        edge.note(300, false);
        assert_eq!(edge.n, 51);
        assert_eq!(edge.bounces, 50);
        assert_eq!(edge.bins[EdgeStat::bin(8)], 50);
        assert_eq!(DWELL_BIN_LABELS[EdgeStat::bin(8)], "6-10");
        assert_eq!(DWELL_BIN_LABELS[EdgeStat::bin(300)], ">250");
        assert_eq!(DWELL_BIN_LABELS[EdgeStat::bin(1)], "<=1");
    }

    #[test]
    fn a_disagreement_between_team_mates_is_counted() {
        let census = PossessionCensus::new();
        census.note(10, 1, true, false, false, 100);
        census.note(11, 1, false, false, false, 100);
        assert_eq!(census.snapshot().disagreeing_team_ticks, 1);
    }

    #[test]
    fn both_and_neither_claims_are_counted_once_per_tick() {
        let census = PossessionCensus::new();
        census.note(10, 1, true, false, false, 100);
        census.note(20, 2, true, false, false, 100);
        census.note(11, 1, false, false, false, 102);
        census.note(21, 2, false, false, false, 102);
        let readings = census.snapshot();
        assert_eq!(readings.both_claim_ticks, 1);
        assert_eq!(readings.neither_claim_ticks, 1);
    }

    #[test]
    fn a_flip_is_a_player_changing_his_reading() {
        let census = PossessionCensus::new();
        census.note(10, 1, true, true, false, 100);
        census.note(10, 1, true, true, false, 102);
        census.note(10, 1, false, false, true, 104);
        census.note(10, 1, true, true, false, 106);
        let readings = census.snapshot();
        assert_eq!(readings.flips_unowned, 1);
        assert_eq!(readings.flips_own_pass_in_flight, 1);
        assert_eq!(readings.flips_owned, 1);
    }

    #[test]
    fn door_census_counts_each_site_by_its_source_line() {
        #[track_caller]
        fn proposal() -> &'static Location<'static> {
            Location::caller()
        }
        let site = proposal();
        for _ in 0..3 {
            DoorCensus::note(site);
        }
        let name = format!("{}:{}", site.file(), site.line());
        let row = DoorCensus::snapshot()
            .into_iter()
            .find(|(door, _)| *door == name)
            .expect("door recorded");
        assert_eq!(row.1, 3);
    }
}
