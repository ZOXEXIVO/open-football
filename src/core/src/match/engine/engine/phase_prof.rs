//! Lightweight, env-gated per-phase timing for the match tick loop.
//!
//! Set `OF_PHASE_PROF=1` and run a single-threaded workload (e.g.
//! `dev_match bench`) to get a breakdown of where a match's wall time goes
//! across the major tick phases. Off by default: `PhaseProf::enabled()` is
//! a single relaxed atomic load, so the instrumentation costs nothing
//! measurable in production builds and there is no need to gate it behind a
//! Cargo feature. Kept in-tree as a permanent diagnostic (see the
//! project's "keep match debug data" rule).
//!
//! Accumulators are thread-local, so under the world simulator's rayon
//! match parallelism each worker keeps its own tallies; the figures are
//! summed per worker thread and reported per match.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

// Process-wide profiler state. Statics rather than struct fields because
// the flag is process-global and the accumulators are thread-local; the
// `PhaseProf` API wraps every operation over them.
static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static ACC: RefCell<[u64; PhaseProf::NUM_PHASES]> =
        const { RefCell::new([0; PhaseProf::NUM_PHASES]) };
}

/// Namespace for the match tick-phase profiler. Unit struct so the whole
/// API hangs off one type instead of loose module functions.
pub struct PhaseProf;

impl PhaseProf {
    pub const NUM_PHASES: usize = 8;

    // Phase indices — keep in lockstep with `PHASE_NAMES`. These are the
    // coarse per-tick phases (cheap: ~5 atomic loads per full tick when
    // profiling is off). Finer per-player AI sub-phase timing was used as
    // a one-off to establish the breakdown (velocity≈36% / process≈32% /
    // fatigue≈11% / move≈8% / loose-ball-override≈6% of the AI) but was
    // removed afterwards because a per-player atomic load on the 6M-update
    // hot path costs ~1% even when disabled.
    pub const P_TICKCTX: usize = 0;
    pub const P_BALL: usize = 1;
    pub const P_PLAYERS: usize = 2;
    pub const P_DISPATCH: usize = 3;
    pub const P_TACTICAL: usize = 4;
    pub const P_COACH: usize = 5;
    pub const P_LIGHT: usize = 6;
    pub const P_OTHER: usize = 7;

    const PHASE_NAMES: [&'static str; Self::NUM_PHASES] = [
        "tick_ctx.update",
        "play_ball(+sp)",
        "play_players(AI)",
        "dispatch+reset",
        "refresh_tactical",
        "evaluate_coaches",
        "light_tick(move)",
        "other",
    ];

    /// Read `OF_PHASE_PROF` once and latch the global flag. Cheap to call
    /// on every match start (a single env read + atomic store).
    pub fn init_from_env() {
        if std::env::var_os("OF_PHASE_PROF").is_some() {
            ENABLED.store(true, Ordering::Relaxed);
        }
    }

    #[inline(always)]
    pub fn enabled() -> bool {
        ENABLED.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn add(phase: usize, nanos: u64) {
        ACC.with(|a| a.borrow_mut()[phase] += nanos);
    }

    /// Time `f`, charging the elapsed nanos to `phase` (only when
    /// profiling is on — pass the cached `on` flag to avoid re-loading the
    /// atomic). Lets call sites stay free of `Instant` plumbing.
    #[inline(always)]
    pub fn timed<R>(on: bool, phase: usize, f: impl FnOnce() -> R) -> R {
        if on {
            let t = Instant::now();
            let r = f();
            Self::add(phase, t.elapsed().as_nanos() as u64);
            r
        } else {
            f()
        }
    }

    /// Print the accumulated breakdown for this thread and reset it.
    /// Called once per match from `play_with_config` when profiling is on.
    pub fn report_and_reset(label: &str) {
        ACC.with(|a| {
            let mut v = a.borrow_mut();
            let total: u64 = v.iter().sum();
            if total == 0 {
                return;
            }
            eprintln!(
                "[PHASE_PROF {}] total={:.1}ms (tick-phase sum; excludes setup/build_result)",
                label,
                total as f64 / 1e6
            );
            for (i, &ns) in v.iter().enumerate() {
                if ns > 0 {
                    eprintln!(
                        "  {:<18} {:>9.2}ms  {:>5.1}%",
                        Self::PHASE_NAMES[i],
                        ns as f64 / 1e6,
                        ns as f64 / total as f64 * 100.0
                    );
                }
            }
            *v = [0; Self::NUM_PHASES];
        });
    }
}
