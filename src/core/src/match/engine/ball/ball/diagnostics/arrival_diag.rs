//! **The arrival census.** What a grant does to the ball's own motion: the
//! pace it is taken at and the pace it leaves at, how far from the man it
//! is taken, whether the first step after the grant turns it round, and
//! whether that step is the ball's own velocity or a write.
//!
//! A grant opens an [`Arrival`] on the ball; the ball's next move closes
//! it. Between the two sits whatever the grant set in motion — the owner
//! tracking for a grant inside `Ball::update`, the rest of the tick and
//! the next tick's ball pass for a grant made by a player event — so a
//! write anywhere in that span shows up as relocation.
//!
//! Split by [`GrantPath`], the same four rows the mid-air census uses.

use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::engine::ball::ball::strike_diag::GrantPath;
use crate::r#match::engine::ball::ball::teleport::{TOLERANCE, VISIBLE};
use nalgebra::Vector3;
use std::sync::atomic::{AtomicU64, Ordering};

/// A grant waiting for the ball's next move.
#[derive(Clone, Copy, Debug)]
pub struct Arrival {
    path: usize,
    at: Vector3<f32>,
    pace_before: f32,
    step_before: Vector3<f32>,
    gap: f32,
}

/// One row of the census, summed over every grant on its path.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArrivalRow {
    pub grants: u64,
    /// Planar pace at the grant and over the first step after it, u/tick.
    pub pace_before: f64,
    pub pace_after: f64,
    /// Ball–man planar gap at the grant, in game units.
    pub gap: f64,
    pub reversed: u64,
    /// Steps after the grant that moved the ball other than by its own
    /// velocity by at least [`TOLERANCE`], and at least [`VISIBLE`].
    pub relocated: u64,
    pub visible: u64,
    /// Summed relocation, in game units.
    pub relocation: f64,
}

const ZERO: AtomicU64 = AtomicU64::new(0);
const N: usize = GrantPath::COUNT;
static GRANTS: [AtomicU64; N] = [ZERO; N];
static PACE_BEFORE_X1000: [AtomicU64; N] = [ZERO; N];
static PACE_AFTER_X1000: [AtomicU64; N] = [ZERO; N];
static GAP_X1000: [AtomicU64; N] = [ZERO; N];
static REVERSED: [AtomicU64; N] = [ZERO; N];
static RELOCATED: [AtomicU64; N] = [ZERO; N];
static VISIBLE_RELOCATED: [AtomicU64; N] = [ZERO; N];
static RELOCATION_X1000: [AtomicU64; N] = [ZERO; N];

pub struct ArrivalCensus;

impl ArrivalCensus {
    /// A step shorter than this has no direction worth comparing: 1 cm.
    const STILL: f32 = 0.08;
    /// More than 120° between the step before and the step after — the
    /// recording census's definition of a reversal.
    const REVERSAL_COS: f32 = -0.5;

    pub fn open(
        path: usize,
        position: Vector3<f32>,
        velocity: Vector3<f32>,
        last_step: Vector3<f32>,
        man: Vector3<f32>,
    ) -> Arrival {
        Arrival {
            path,
            at: position,
            pace_before: velocity.x.hypot(velocity.y),
            step_before: Vector3::new(last_step.x, last_step.y, 0.0),
            gap: (man.x - position.x).hypot(man.y - position.y),
        }
    }

    /// Book `arrival` against where the ball is after its next move and
    /// the velocity that move integrated.
    pub fn close(arrival: Arrival, position: Vector3<f32>, velocity: Vector3<f32>) {
        let p = arrival.path;
        let step = Vector3::new(position.x - arrival.at.x, position.y - arrival.at.y, 0.0);
        let pace_after = step.norm();
        let relocation = (step.x - velocity.x).hypot(step.y - velocity.y);
        let milli = |v: f32| (v.max(0.0) * 1000.0) as u64;
        GRANTS[p].fetch_add(1, Ordering::Relaxed);
        PACE_BEFORE_X1000[p].fetch_add(milli(arrival.pace_before), Ordering::Relaxed);
        PACE_AFTER_X1000[p].fetch_add(milli(pace_after), Ordering::Relaxed);
        GAP_X1000[p].fetch_add(milli(arrival.gap), Ordering::Relaxed);
        let before = arrival.step_before.norm();
        if before > Self::STILL
            && pace_after > Self::STILL
            && arrival.step_before.dot(&step) < Self::REVERSAL_COS * before * pace_after
        {
            REVERSED[p].fetch_add(1, Ordering::Relaxed);
        }
        if relocation >= TOLERANCE {
            RELOCATED[p].fetch_add(1, Ordering::Relaxed);
            RELOCATION_X1000[p].fetch_add(milli(relocation), Ordering::Relaxed);
            if relocation >= VISIBLE {
                VISIBLE_RELOCATED[p].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn snapshot() -> [ArrivalRow; N] {
        let read = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let units = |c: &AtomicU64| read(c) as f64 / 1000.0;
        std::array::from_fn(|p| ArrivalRow {
            grants: read(&GRANTS[p]),
            pace_before: units(&PACE_BEFORE_X1000[p]),
            pace_after: units(&PACE_AFTER_X1000[p]),
            gap: units(&GAP_X1000[p]),
            reversed: read(&REVERSED[p]),
            relocated: read(&RELOCATED[p]),
            visible: read(&VISIBLE_RELOCATED[p]),
            relocation: units(&RELOCATION_X1000[p]),
        })
    }

    pub fn reset() {
        for c in GRANTS
            .iter()
            .chain(PACE_BEFORE_X1000.iter())
            .chain(PACE_AFTER_X1000.iter())
            .chain(GAP_X1000.iter())
            .chain(REVERSED.iter())
            .chain(RELOCATED.iter())
            .chain(VISIBLE_RELOCATED.iter())
            .chain(RELOCATION_X1000.iter())
        {
            c.store(0, Ordering::Relaxed);
        }
    }
}

/// **How soon a man shoots after he is given the ball**, and how often
/// those shots go in — split by the ticks since his grant and by the ticks
/// since the pass that reached him was struck. A keeper's time to set is
/// the gap.
pub struct ShotTiming;

const TIMING: usize = 6;
static SHOTS_SINCE_GRANT: [AtomicU64; TIMING] = [ZERO; TIMING];
static GOALS_SINCE_GRANT: [AtomicU64; TIMING] = [ZERO; TIMING];
static SHOTS_SINCE_PASS: [AtomicU64; TIMING] = [ZERO; TIMING];
static GOALS_SINCE_PASS: [AtomicU64; TIMING] = [ZERO; TIMING];

impl ShotTiming {
    pub const GRANT_NAMES: [&'static str; TIMING] =
        ["<5", "5-15", "15-30", "30-60", "60+", "not his"];
    const GRANT_EDGES: [u64; TIMING - 1] = [5, 15, 30, 60, u64::MAX];
    pub const PASS_NAMES: [&'static str; TIMING] =
        ["<40", "40-60", "60-90", "90-150", "150-300", "300+"];
    const PASS_EDGES: [u64; TIMING] = [40, 60, 90, 150, 300, u64::MAX];

    /// The bucket pair a shot falls in: ticks since his grant (`None`
    /// when he never had the ball), and since the last pass was struck.
    pub fn buckets(since_grant: Option<u64>, since_pass: u64) -> (usize, usize) {
        let grant = since_grant.map_or(TIMING - 1, |t| {
            Self::GRANT_EDGES
                .iter()
                .position(|&e| t < e)
                .unwrap_or(TIMING - 2)
        });
        let pass = Self::PASS_EDGES
            .iter()
            .position(|&e| since_pass < e)
            .unwrap_or(TIMING - 1);
        (grant, pass)
    }

    pub fn note_shot(buckets: (usize, usize)) {
        SHOTS_SINCE_GRANT[buckets.0].fetch_add(1, Ordering::Relaxed);
        SHOTS_SINCE_PASS[buckets.1].fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_goal(buckets: (usize, usize)) {
        GOALS_SINCE_GRANT[buckets.0].fetch_add(1, Ordering::Relaxed);
        GOALS_SINCE_PASS[buckets.1].fetch_add(1, Ordering::Relaxed);
    }

    /// `(shots, goals)` by grant bucket, then by pass bucket.
    #[allow(clippy::type_complexity)]
    pub fn snapshot() -> ([u64; TIMING], [u64; TIMING], [u64; TIMING], [u64; TIMING]) {
        let read = |c: &[AtomicU64; TIMING]| std::array::from_fn(|i| c[i].load(Ordering::Relaxed));
        (
            read(&SHOTS_SINCE_GRANT),
            read(&GOALS_SINCE_GRANT),
            read(&SHOTS_SINCE_PASS),
            read(&GOALS_SINCE_PASS),
        )
    }

    pub fn reset() {
        for c in SHOTS_SINCE_GRANT
            .iter()
            .chain(GOALS_SINCE_GRANT.iter())
            .chain(SHOTS_SINCE_PASS.iter())
            .chain(GOALS_SINCE_PASS.iter())
        {
            c.store(0, Ordering::Relaxed);
        }
    }
}

impl Ball {
    /// A grant on `path` to a man standing at `man`, booked before the
    /// grant writes anything onto the ball.
    pub(crate) fn open_arrival(&mut self, path: usize, man: Vector3<f32>) {
        self.granted_at = self.current_tick_cached;
        self.arrival = Some(ArrivalCensus::open(
            path,
            self.position,
            self.velocity,
            self.last_step,
            man,
        ));
    }

    /// The ball has just moved from `before`: close any open arrival
    /// against it and remember the step for the next one.
    pub(crate) fn settle_arrival(&mut self, before: Vector3<f32>) {
        if let Some(arrival) = self.arrival.take() {
            ArrivalCensus::close(arrival, self.position, self.velocity);
        }
        self.last_step = Vector3::new(self.position.x - before.x, self.position.y - before.y, 0.0);
    }
}
