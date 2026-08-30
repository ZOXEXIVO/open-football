//! **The thermal map** — where the twenty-two actually spend the ninety
//! minutes, drawn the way a broadcaster draws one.
//!
//! Every other spatial counter in the harness is a scalar.
//! [`spacing`](super::spacing) reports a clump, `paths` reports a per-line
//! mean, `fwdpath` draws two forwards off the replay track. None of them
//! can answer the question a heat map answers at a glance — **does this
//! player have a position at all** — and that question is the whole
//! difference between a football match and twenty-two men chasing a ball.
//!
//! Four things make what it prints comparable with a real one:
//!
//! * **Time, not samples.** Sampled on the match clock every
//!   [`SAMPLE_INTERVAL_MS`], so a man standing still weighs exactly as
//!   much as a man sprinting. The replay track cannot do this: it is
//!   deduplicated at 0.3 u with a 750 ms heartbeat, so reading it
//!   directly counts *movement*, which is the opposite of what a heat map
//!   is for.
//! * **One frame.** Both sides are folded to attack RIGHT by a 180°
//!   rotation about the centre spot — never a mirror, which would swap a
//!   right-back onto the left flank and cancel the two sides out instead
//!   of stacking them.
//! * **Only men who are on the pitch.** `field.players` drops a
//!   substituted player the instant he goes off (he moves to `departed`,
//!   which the replay goes on drawing on the touchline), so nothing here
//!   has to unpick a touchline blob from a real position.
//! * **The phase, as the ENGINE means it.** Every cell is booked twice:
//!   over all play, and into whichever of the two possession phases was
//!   running. A real full-back's map in possession sits ten to fifteen
//!   metres further up the pitch than the same man's map out of it, and
//!   the pair is the clearest picture of a shape there is. One map that
//!   never moves between the two is the signature of a side with no
//!   phases at all.
//!
//!   ⚠ The phase is [`TeamTacticalState::in_possession`], **not**
//!   `ball.current_owner`. The first cut used the owner and it was
//!   measuring something else: somebody is actually CARRYING the ball for
//!   about 18% of a match, so 82% of every sample fell through into the
//!   all-play bucket and the two phase maps were built out of settled
//!   carrying alone. The possession flag is sticky across loose balls and
//!   passes in flight, it is what `TeamShape` itself reads when it decides
//!   which end the block hangs from, and it partitions the whole match —
//!   so the question "does the block make its excursion" is asked of the
//!   same phase the block was planned for.
//!
//! The ball gets a map of its own, folded into *both* frames, because the
//! per-slot maps hold both sides and the thing they are compared against
//! has to hold both too. Correlating a player's map with the ball's is
//! the sharpest single number here: it is how much of a man's positioning
//! is nothing but the position of the ball.

use crate::PlayerPositionType;
use crate::r#match::engine::engine::*;
use nalgebra::Vector3;
use std::sync::atomic::{AtomicU64, Ordering};

/// Columns of the map, goal line to goal line. 84 over 105 m is 1.25 m a
/// cell — finer than a broadcast heat map, and it prints one character
/// per column without wrapping a terminal.
pub const COLS: usize = 84;
/// Rows of the map, touchline to touchline: 28 over 68.125 m, 2.43 m a
/// cell. Printing two rows to a line gives a character 1.25 m by 4.87 m,
/// which is very nearly square once a terminal cell's own 1:2 shape is
/// taken into account.
pub const ROWS: usize = 28;
/// Cells in one map.
pub const CELLS: usize = COLS * ROWS;

/// The pitch the engine plays on, in metres. 840 x 545 units at 8 u/m.
pub const PITCH_LENGTH_M: f32 = 105.0;
pub const PITCH_WIDTH_M: f32 = 68.125;
/// Area of one cell, m². Area figures are counted in cells and scaled by
/// this.
pub const CELL_AREA_M2: f32 = (PITCH_LENGTH_M * PITCH_WIDTH_M) / (COLS * ROWS) as f32;

/// Engine units to metres.
const M_PER_UNIT: f32 = 0.125;

/// One sample per this much MATCH time — 20 Hz. Full ticks run every
/// 20 ms, so every window holds at least two chances to fire, and gating
/// on the window INDEX rather than on a tick modulo is what keeps the
/// cadence uniform: `current_tick() % n` drifts against the light/full
/// tick parity every time a goal celebration advances the clock without
/// running a tick body.
const SAMPLE_INTERVAL_MS: u64 = 50;

/// Number of slots the maps are keyed by.
const POSITIONS: usize = PlayerPositionType::ALL.len();

/// All play, then the two phases. Index 0 is always booked; 1 or 2 only
/// when somebody actually has the ball.
pub const PHASES: usize = 3;
pub const PHASE_ALL: usize = 0;
pub const PHASE_IN_POSSESSION: usize = 1;
pub const PHASE_OUT_OF_POSSESSION: usize = 2;

/// Penalty area, from the goal line and from the middle of the pitch.
const BOX_DEPTH_M: f32 = 16.5;
const BOX_HALF_WIDTH_M: f32 = 20.16;
/// Inside this of a touchline is the width a coach means by "stay wide".
const TOUCHLINE_M: f32 = 10.0;
/// The ring around the ball a man is in the play in. 15 m.
const BALL_RING_M: f32 = 15.0;

/// Outfielders of one side, the most the shape block ever folds.
const MAX_PER_SIDE: usize = 11;

static GRIDS: [AtomicU64; POSITIONS * PHASES * CELLS] =
    [const { AtomicU64::new(0) }; POSITIONS * PHASES * CELLS];
static BALL: [AtomicU64; CELLS] = [const { AtomicU64::new(0) }; CELLS];

/// The window index of the last sample taken, compared with `swap` so a
/// new match — whose clock restarts at zero — always takes its first
/// sample instead of waiting for the previous match's clock to be
/// overtaken.
static LAST_WINDOW: AtomicU64 = AtomicU64::new(u64::MAX);
static SAMPLES: AtomicU64 = AtomicU64::new(0);

/// Stop sampling after this much of each match. `u64::MAX` — the default
/// — is the whole ninety. Exists because the interesting comparison with
/// a broadcast heat map is often a WINDOW: the shape a side keeps over
/// fifteen minutes of settled play, before substitutions and a scoreline
/// have had time to move anybody.
static WINDOW_MS: AtomicU64 = AtomicU64::new(u64::MAX);

/// Everything about one slot that is a running total rather than a map.
struct PositionAcc {
    samples: AtomicU64,
    sum_x_x10: AtomicU64,
    sum_x2: AtomicU64,
    sum_y_x10: AtomicU64,
    sum_y2: AtomicU64,
    own_half: AtomicU64,
    final_third: AtomicU64,
    own_box: AtomicU64,
    opp_box: AtomicU64,
    wide: AtomicU64,
    touchline: AtomicU64,
    ball_gap_x10: AtomicU64,
    near_ball: AtomicU64,
    poss_n: AtomicU64,
    poss_x_x10: AtomicU64,
    oop_n: AtomicU64,
    oop_x_x10: AtomicU64,
}

impl PositionAcc {
    const fn new() -> Self {
        PositionAcc {
            samples: AtomicU64::new(0),
            sum_x_x10: AtomicU64::new(0),
            sum_x2: AtomicU64::new(0),
            sum_y_x10: AtomicU64::new(0),
            sum_y2: AtomicU64::new(0),
            own_half: AtomicU64::new(0),
            final_third: AtomicU64::new(0),
            own_box: AtomicU64::new(0),
            opp_box: AtomicU64::new(0),
            wide: AtomicU64::new(0),
            touchline: AtomicU64::new(0),
            ball_gap_x10: AtomicU64::new(0),
            near_ball: AtomicU64::new(0),
            poss_n: AtomicU64::new(0),
            poss_x_x10: AtomicU64::new(0),
            oop_n: AtomicU64::new(0),
            oop_x_x10: AtomicU64::new(0),
        }
    }

    fn reset(&self) {
        for slot in [
            &self.samples,
            &self.sum_x_x10,
            &self.sum_x2,
            &self.sum_y_x10,
            &self.sum_y2,
            &self.own_half,
            &self.final_third,
            &self.own_box,
            &self.opp_box,
            &self.wide,
            &self.touchline,
            &self.ball_gap_x10,
            &self.near_ball,
            &self.poss_n,
            &self.poss_x_x10,
            &self.oop_n,
            &self.oop_x_x10,
        ] {
            slot.store(0, Ordering::Relaxed);
        }
    }
}

static POS: [PositionAcc; POSITIONS] = [const { PositionAcc::new() }; POSITIONS];

/// The ten outfielders of one side read as a body: how long, how wide,
/// how high, and how many of them are stood around the ball.
struct ShapeAcc {
    samples: AtomicU64,
    length_x10: AtomicU64,
    width_x10: AtomicU64,
    centroid_x10: AtomicU64,
    deepest_x10: AtomicU64,
    highest_x10: AtomicU64,
    swarm_x10: AtomicU64,
}

impl ShapeAcc {
    const fn new() -> Self {
        ShapeAcc {
            samples: AtomicU64::new(0),
            length_x10: AtomicU64::new(0),
            width_x10: AtomicU64::new(0),
            centroid_x10: AtomicU64::new(0),
            deepest_x10: AtomicU64::new(0),
            highest_x10: AtomicU64::new(0),
            swarm_x10: AtomicU64::new(0),
        }
    }

    fn reset(&self) {
        for slot in [
            &self.samples,
            &self.length_x10,
            &self.width_x10,
            &self.centroid_x10,
            &self.deepest_x10,
            &self.highest_x10,
            &self.swarm_x10,
        ] {
            slot.store(0, Ordering::Relaxed);
        }
    }
}

static SHAPE: [ShapeAcc; PHASES] = [const { ShapeAcc::new() }; PHASES];

/// How long each side spends in each [`GamePhase`], in sampled instants.
///
/// The phase is not decoration — it gates the overlap, the width plan's
/// commitment and the block's own reference — so "how much of a match is
/// each one" is a first-order question about the shape, and nothing else
/// in the harness answers it. Indexed by `GamePhase as usize`.
static PHASE_TICKS: [AtomicU64; GAME_PHASES] = [const { AtomicU64::new(0) }; GAME_PHASES];

/// Variants of [`GamePhase`]. Asserted against the labels the harness
/// prints, so a new variant cannot silently fall off the end of the
/// table.
pub const GAME_PHASES: usize = 8;

/// One slot's finished maps and totals, in metres and shares.
#[derive(Clone, Default)]
pub struct PositionHeat {
    /// Index into [`PlayerPositionType::ALL`].
    pub position: usize,
    pub samples: u64,
    /// [`PHASES`] maps of [`CELLS`] cells each, row-major, indexed by the
    /// `PHASE_*` constants.
    pub grid: Vec<Vec<u64>>,
    /// Distance from the goal he defends, m.
    pub mean_x: f32,
    pub sd_x: f32,
    /// Across the pitch, m from one touchline.
    pub mean_y: f32,
    pub sd_y: f32,
    pub own_half: f32,
    pub final_third: f32,
    pub own_box: f32,
    pub opp_box: f32,
    pub wide: f32,
    pub touchline: f32,
    pub ball_gap: f32,
    pub near_ball: f32,
    /// Mean `x` in each phase, so the two can be subtracted.
    pub poss_x: f32,
    pub oop_x: f32,
}

/// One phase's team shape.
#[derive(Clone, Copy, Default)]
pub struct ShapeHeat {
    pub samples: u64,
    pub length: f32,
    pub width: f32,
    pub centroid_x: f32,
    pub deepest: f32,
    pub highest: f32,
    pub swarm: f32,
}

/// Everything the census holds, read once at full time.
pub struct HeatReport {
    pub samples: u64,
    pub positions: Vec<PositionHeat>,
    pub ball: Vec<u64>,
    pub shape: [ShapeHeat; PHASES],
    /// Side-instants spent in each [`GamePhase`], by `as usize` order.
    pub game_phase: [u64; GAME_PHASES],
}

pub struct HeatMapCensus;

impl HeatMapCensus {
    /// Fold a point into the frame of a side attacking RIGHT, in metres.
    ///
    /// A 180° rotation about the centre spot — `(x, y) -> (W-x, H-y)` —
    /// and deliberately not a mirror. A mirror has determinant −1 and
    /// would put one side's right-back on the other side's left flank,
    /// which is the one transform that makes a two-team heat map say
    /// nothing at all.
    pub fn fold(p: Vector3<f32>, side: PlayerSide, w: f32, h: f32) -> (f32, f32) {
        let (x, y) = if side.forward_dir_x() > 0.0 {
            (p.x, p.y)
        } else {
            (w - p.x, h - p.y)
        };
        (x * M_PER_UNIT, y * M_PER_UNIT)
    }

    /// Cell index for a folded point. Clamped rather than dropped: a
    /// player standing a stride behind his own goal line is a real
    /// sample, and dropping it would put the maps and the totals beside
    /// them on different denominators.
    pub fn cell(x: f32, y: f32) -> usize {
        if !x.is_finite() || !y.is_finite() {
            return 0;
        }
        let col = ((x / PITCH_LENGTH_M) * COLS as f32).clamp(0.0, (COLS - 1) as f32) as usize;
        let row = ((y / PITCH_WIDTH_M) * ROWS as f32).clamp(0.0, (ROWS - 1) as f32) as usize;
        row * COLS + col
    }

    fn grid_slot(position: usize, phase: usize, cell: usize) -> &'static AtomicU64 {
        &GRIDS[(position * PHASES + phase) * CELLS + cell]
    }

    /// Sample only the first `ms` of each match. Zero restores the whole
    /// ninety.
    pub fn set_window(ms: u64) {
        WINDOW_MS.store(if ms == 0 { u64::MAX } else { ms }, Ordering::Relaxed);
    }

    /// Wipe every counter. Called by the harness between runs, never by
    /// the engine.
    pub fn reset() {
        for slot in GRIDS.iter() {
            slot.store(0, Ordering::Relaxed);
        }
        for slot in BALL.iter() {
            slot.store(0, Ordering::Relaxed);
        }
        for acc in POS.iter() {
            acc.reset();
        }
        for acc in SHAPE.iter() {
            acc.reset();
        }
        for slot in PHASE_TICKS.iter() {
            slot.store(0, Ordering::Relaxed);
        }
        LAST_WINDOW.store(u64::MAX, Ordering::Relaxed);
        SAMPLES.store(0, Ordering::Relaxed);
    }

    pub fn snapshot() -> HeatReport {
        let positions = (0..POSITIONS)
            .map(|i| {
                let acc = &POS[i];
                let n = acc.samples.load(Ordering::Relaxed);
                let d = n.max(1) as f32;
                let share = |v: &AtomicU64| v.load(Ordering::Relaxed) as f32 / d;
                let mean_x = acc.sum_x_x10.load(Ordering::Relaxed) as f32 / 10.0 / d;
                let mean_y = acc.sum_y_x10.load(Ordering::Relaxed) as f32 / 10.0 / d;
                let var_x = acc.sum_x2.load(Ordering::Relaxed) as f32 / d - mean_x * mean_x;
                let var_y = acc.sum_y2.load(Ordering::Relaxed) as f32 / d - mean_y * mean_y;
                let poss_n = acc.poss_n.load(Ordering::Relaxed).max(1) as f32;
                let oop_n = acc.oop_n.load(Ordering::Relaxed).max(1) as f32;
                PositionHeat {
                    position: i,
                    samples: n,
                    grid: (0..PHASES)
                        .map(|phase| {
                            (0..CELLS)
                                .map(|c| Self::grid_slot(i, phase, c).load(Ordering::Relaxed))
                                .collect()
                        })
                        .collect(),
                    mean_x,
                    sd_x: var_x.max(0.0).sqrt(),
                    mean_y,
                    sd_y: var_y.max(0.0).sqrt(),
                    own_half: share(&acc.own_half),
                    final_third: share(&acc.final_third),
                    own_box: share(&acc.own_box),
                    opp_box: share(&acc.opp_box),
                    wide: share(&acc.wide),
                    touchline: share(&acc.touchline),
                    ball_gap: acc.ball_gap_x10.load(Ordering::Relaxed) as f32 / 10.0 / d,
                    near_ball: share(&acc.near_ball),
                    poss_x: acc.poss_x_x10.load(Ordering::Relaxed) as f32 / 10.0 / poss_n,
                    oop_x: acc.oop_x_x10.load(Ordering::Relaxed) as f32 / 10.0 / oop_n,
                }
            })
            .collect();

        let mut shape = [ShapeHeat::default(); PHASES];
        for (out, acc) in shape.iter_mut().zip(SHAPE.iter()) {
            let n = acc.samples.load(Ordering::Relaxed);
            let d = n.max(1) as f32;
            let per = |v: &AtomicU64| v.load(Ordering::Relaxed) as f32 / 10.0 / d;
            out.samples = n;
            out.length = per(&acc.length_x10);
            out.width = per(&acc.width_x10);
            out.centroid_x = per(&acc.centroid_x10);
            out.deepest = per(&acc.deepest_x10);
            out.highest = per(&acc.highest_x10);
            out.swarm = per(&acc.swarm_x10);
        }

        HeatReport {
            samples: SAMPLES.load(Ordering::Relaxed),
            positions,
            ball: BALL.iter().map(|c| c.load(Ordering::Relaxed)).collect(),
            shape,
            game_phase: std::array::from_fn(|i| PHASE_TICKS[i].load(Ordering::Relaxed)),
        }
    }
}

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// Book one instant of the match into the thermal map.
    ///
    /// Runs from `game_tick_inner` beside the other censuses, but gates
    /// on a window of the MATCH CLOCK rather than on a tick modulo: full
    /// ticks alternate with light ones and a goal celebration advances
    /// the clock without running either, so `current_tick() % n` samples
    /// at a rate that changes every time somebody scores.
    #[cfg(feature = "match-logs")]
    pub(in crate::r#match::engine::engine) fn sample_heatmap(
        field: &MatchField,
        context: &MatchContext,
    ) {
        if context.total_match_time > WINDOW_MS.load(Ordering::Relaxed) {
            return;
        }
        let window = context.total_match_time / SAMPLE_INTERVAL_MS;
        if LAST_WINDOW.swap(window, Ordering::Relaxed) == window {
            return;
        }
        SAMPLES.fetch_add(1, Ordering::Relaxed);

        let w = field.size.width as f32;
        let h = field.size.height as f32;
        let ball = field.ball.position;

        // The ball, folded into BOTH frames. The slot maps below stack the
        // two sides on top of each other, so the map they are compared
        // against has to be the same shape.
        for side in [PlayerSide::Left, PlayerSide::Right] {
            let (bx, by) = HeatMapCensus::fold(ball, side, w, h);
            BALL[HeatMapCensus::cell(bx, by)].fetch_add(1, Ordering::Relaxed);
        }

        // Per-side shape, gathered on the same walk so the outfielders are
        // only read once. Frame 0 is the side attacking right this half.
        let mut lo_x = [f32::MAX; 2];
        let mut hi_x = [f32::MIN; 2];
        let mut lo_y = [f32::MAX; 2];
        let mut hi_y = [f32::MIN; 2];
        let mut sum_x = [0.0f32; 2];
        let mut count = [0u32; 2];
        let mut swarm = [0u32; 2];
        let mut team_of_frame = [None::<u32>; 2];

        for p in field.players.iter() {
            if p.is_sent_off {
                continue;
            }
            let Some(side) = p.side else { continue };
            let (x, y) = HeatMapCensus::fold(p.position, side, w, h);
            let position = p.tactical_position.current_position;
            let index = position as usize;
            if index >= POSITIONS {
                continue;
            }
            let acc = &POS[index];
            let phase = if context.tactical_for_team(p.team_id).in_possession {
                PHASE_IN_POSSESSION
            } else {
                PHASE_OUT_OF_POSSESSION
            };
            let cell = HeatMapCensus::cell(x, y);
            HeatMapCensus::grid_slot(index, PHASE_ALL, cell).fetch_add(1, Ordering::Relaxed);
            HeatMapCensus::grid_slot(index, phase, cell).fetch_add(1, Ordering::Relaxed);

            let off_centre = (y - PITCH_WIDTH_M * 0.5).abs();
            let gap = (p.position - ball).magnitude() * M_PER_UNIT;
            acc.samples.fetch_add(1, Ordering::Relaxed);
            acc.sum_x_x10
                .fetch_add((x.max(0.0) * 10.0) as u64, Ordering::Relaxed);
            acc.sum_x2
                .fetch_add((x * x).max(0.0) as u64, Ordering::Relaxed);
            acc.sum_y_x10
                .fetch_add((y.max(0.0) * 10.0) as u64, Ordering::Relaxed);
            acc.sum_y2
                .fetch_add((y * y).max(0.0) as u64, Ordering::Relaxed);
            if x < PITCH_LENGTH_M * 0.5 {
                acc.own_half.fetch_add(1, Ordering::Relaxed);
            }
            if x > PITCH_LENGTH_M * 2.0 / 3.0 {
                acc.final_third.fetch_add(1, Ordering::Relaxed);
            }
            if x < BOX_DEPTH_M && off_centre < BOX_HALF_WIDTH_M {
                acc.own_box.fetch_add(1, Ordering::Relaxed);
            }
            if x > PITCH_LENGTH_M - BOX_DEPTH_M && off_centre < BOX_HALF_WIDTH_M {
                acc.opp_box.fetch_add(1, Ordering::Relaxed);
            }
            if off_centre > BOX_HALF_WIDTH_M {
                acc.wide.fetch_add(1, Ordering::Relaxed);
            }
            if y < TOUCHLINE_M || y > PITCH_WIDTH_M - TOUCHLINE_M {
                acc.touchline.fetch_add(1, Ordering::Relaxed);
            }
            acc.ball_gap_x10
                .fetch_add((gap.max(0.0) * 10.0) as u64, Ordering::Relaxed);
            if gap < BALL_RING_M {
                acc.near_ball.fetch_add(1, Ordering::Relaxed);
            }
            if phase == PHASE_IN_POSSESSION {
                acc.poss_n.fetch_add(1, Ordering::Relaxed);
                acc.poss_x_x10
                    .fetch_add((x.max(0.0) * 10.0) as u64, Ordering::Relaxed);
            } else {
                acc.oop_n.fetch_add(1, Ordering::Relaxed);
                acc.oop_x_x10
                    .fetch_add((x.max(0.0) * 10.0) as u64, Ordering::Relaxed);
            }

            // The shape is the ten outfielders. The keeper is 40 m behind
            // everybody and would own every length in the table.
            if position.is_goalkeeper() {
                continue;
            }
            let frame = if side.forward_dir_x() > 0.0 { 0 } else { 1 };
            if count[frame] as usize >= MAX_PER_SIDE {
                continue;
            }
            team_of_frame[frame] = Some(p.team_id);
            lo_x[frame] = lo_x[frame].min(x);
            hi_x[frame] = hi_x[frame].max(x);
            lo_y[frame] = lo_y[frame].min(y);
            hi_y[frame] = hi_y[frame].max(y);
            sum_x[frame] += x;
            count[frame] += 1;
            if gap < BALL_RING_M {
                swarm[frame] += 1;
            }
        }

        for frame in 0..2 {
            if count[frame] < 6 {
                continue;
            }
            let Some(team) = team_of_frame[frame] else {
                continue;
            };
            let tactical = context.tactical_for_team(team);
            let game_phase = tactical.phase as usize;
            if game_phase < GAME_PHASES {
                PHASE_TICKS[game_phase].fetch_add(1, Ordering::Relaxed);
            }
            let phase = if tactical.in_possession {
                PHASE_IN_POSSESSION
            } else {
                PHASE_OUT_OF_POSSESSION
            };
            let n = count[frame] as f32;
            for slot in [PHASE_ALL, phase] {
                let acc = &SHAPE[slot];
                acc.samples.fetch_add(1, Ordering::Relaxed);
                acc.length_x10.fetch_add(
                    ((hi_x[frame] - lo_x[frame]) * 10.0).max(0.0) as u64,
                    Ordering::Relaxed,
                );
                acc.width_x10.fetch_add(
                    ((hi_y[frame] - lo_y[frame]) * 10.0).max(0.0) as u64,
                    Ordering::Relaxed,
                );
                acc.centroid_x10
                    .fetch_add((sum_x[frame] / n * 10.0).max(0.0) as u64, Ordering::Relaxed);
                acc.deepest_x10
                    .fetch_add((lo_x[frame] * 10.0).max(0.0) as u64, Ordering::Relaxed);
                acc.highest_x10
                    .fetch_add((hi_x[frame] * 10.0).max(0.0) as u64, Ordering::Relaxed);
                acc.swarm_x10
                    .fetch_add(swarm[frame] as u64 * 10, Ordering::Relaxed);
            }
        }
    }
}
