//! Seedable RNG for the sim utilities. Call sites go through
//! `IntegerUtils::random` / `FloatUtils::random` and draw from a thread-local
//! `SmallRng` seeded from a global seed. Callers pin reproducibility by
//! calling [`set_seed`] before building `SimulatorData`.
//!
//! Parallelism caveat: Rayon worker scheduling is non-deterministic, so
//! seeding alone does not guarantee bit-identical runs — it guarantees a
//! *per-thread* reproducible stream and eliminates OS-entropy drift. Good
//! enough for regression bisection; not a replay tool.

use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

/// Global sim seed. Generation bumps whenever `set_seed` is called so
/// thread-local RNGs notice and rebuild from the new base.
static SIM_SEED: AtomicU64 = AtomicU64::new(0);
static SEED_GENERATION: AtomicU64 = AtomicU64::new(0);
static THREAD_ID_COUNTER: AtomicU64 = AtomicU64::new(1);
/// Starting point when no explicit seed has been pinned. Picked once per
/// process and mixed with a per-thread id so threads don't share streams.
const UNSEEDED_BASE: u64 = 0x4F5A455845_5F4F46_u64; // "ZOXEXO_OF" — stable
/// Multiplicative mixer for combining seed with thread id.
const GOLDEN_RATIO_ODD: u64 = 0x9E3779B97F4A7C15;

thread_local! {
    static TL_RNG: RefCell<ThreadRng> = RefCell::new(ThreadRng::new());
    static TL_ID: u64 = THREAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
}

struct ThreadRng {
    rng: SmallRng,
    known_generation: u64,
}

impl ThreadRng {
    fn new() -> Self {
        ThreadRng {
            rng: SmallRng::seed_from_u64(UNSEEDED_BASE),
            // Sentinel: not equal to any real generation, forces initial seed.
            known_generation: u64::MAX,
        }
    }

    fn ensure_fresh(&mut self, thread_id: u64) {
        let generation = SEED_GENERATION.load(Ordering::Relaxed);
        if generation == self.known_generation {
            return;
        }
        // `generation == 0` means unseeded — pick a per-thread base so
        // different threads don't share the same stream. Otherwise use
        // the current sim seed.
        let base = if generation == 0 {
            UNSEEDED_BASE
        } else {
            SIM_SEED.load(Ordering::Relaxed)
        };
        let mixed = base.wrapping_mul(GOLDEN_RATIO_ODD).wrapping_add(thread_id);
        self.rng = SmallRng::seed_from_u64(mixed);
        self.known_generation = generation;
    }
}

/// Pin the sim RNG to a specific seed. Takes effect lazily on each thread's
/// next draw. Pass 0 to return to OS-entropy seeding (i.e. disable pinning).
pub fn set_seed(seed: u64) {
    SIM_SEED.store(seed, Ordering::Relaxed);
    SEED_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Current sim seed. Zero means unseeded (process-unique base).
pub fn current_seed() -> u64 {
    SIM_SEED.load(Ordering::Relaxed)
}

pub fn gen_f32() -> f32 {
    with_rng(|rng| rng.random::<f32>())
}

pub fn gen_f64() -> f64 {
    with_rng(|rng| rng.random::<f64>())
}

fn with_rng<R>(f: impl FnOnce(&mut SmallRng) -> R) -> R {
    let id = TL_ID.with(|id| *id);
    TL_RNG.with(|cell| {
        let mut guard = cell.borrow_mut();
        guard.ensure_fresh(id);
        f(&mut guard.rng)
    })
}
