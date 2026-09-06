//! The profiler itself: the recording API and the report.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use super::clock::process_cpu_nanos;
use super::sampler;

static ENABLED: AtomicBool = AtomicBool::new(false);
static INITIALISED: OnceLock<()> = OnceLock::new();

/// One profiled region, accumulated across every tick in the run.
struct MeasuredRegion {
    name: &'static str,
    /// Nesting level, purely for report indentation. A nested row's time is
    /// also counted in its parent — the report is inclusive.
    depth: u8,
    wall_ns: u64,
    cpu_ns: u64,
    /// Longest single call. For a phase this is the worst tick; for a stage
    /// it is the straggler — the one item that alone bounds how short the
    /// enclosing phase can get.
    max_ns: u64,
    /// Who the `max_ns` call was. Only stages that pass a label carry one
    /// — it answers "which country was the straggler", which the number
    /// alone never can.
    max_label: String,
    calls: u32,
    /// Stage rows are recorded from inside the rayon tree, where a
    /// process-CPU delta means nothing. `wall_ns` is then the SUM of every
    /// worker's time in the stage and `max_ns` the single slowest item, so
    /// `wall / max` is the widest that stage could ever run — its
    /// load-balance ceiling. That ratio is printed in the `cores` column.
    is_stage: bool,
}

fn regions() -> &'static Mutex<Vec<MeasuredRegion>> {
    static REGIONS: OnceLock<Mutex<Vec<MeasuredRegion>>> = OnceLock::new();
    REGIONS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Namespace for the world-tick profiler. Unit struct so the whole API hangs
/// off one type rather than loose module functions.
pub struct PerformanceProfiler;

impl PerformanceProfiler {
    /// Read `OF_PERFORMANCE` once and latch the global flag, starting the
    /// width sampler if it is set. Idempotent — called at the top of every
    /// tick, and costs one `OnceLock` load after the first.
    pub fn init_from_env() {
        INITIALISED.get_or_init(|| {
            if std::env::var_os("OF_PERFORMANCE").is_some() {
                ENABLED.store(true, Ordering::Relaxed);
                sampler::spawn();
            }
        });
    }

    #[inline(always)]
    pub fn enabled() -> bool {
        ENABLED.load(Ordering::Relaxed)
    }

    /// Run `f` as a *phase*, charging its wall time and the process CPU
    /// burned while it ran. `depth` only indents the report.
    ///
    /// Must be called from the thread driving the tick — inside a rayon
    /// closure the process-CPU delta belongs to the whole pool, so use
    /// [`stage`][Self::stage] there instead.
    #[inline(always)]
    pub fn phase<R>(name: &'static str, depth: u8, f: impl FnOnce() -> R) -> R {
        if !Self::enabled() {
            return f();
        }
        let scope = Self::phase_scope(name, depth);
        let out = f();
        drop(scope);
        out
    }

    /// Open a phase that ends when the returned guard drops. For regions
    /// that produce bindings the rest of the tick needs, and so can't be
    /// wrapped in a closure — end one early with `drop(guard)`.
    #[inline(always)]
    pub fn phase_scope(name: &'static str, depth: u8) -> PhaseScope {
        let on = Self::enabled();
        PhaseScope {
            name,
            depth,
            start_ns: if on { sampler::now_ns() } else { 0 },
            wall0: on.then(Instant::now),
            cpu0: if on { process_cpu_nanos() } else { 0 },
        }
    }

    /// Run `f` as a *stage* — a region inside the rayon tree, entered once
    /// per item (per country, per club) on whatever worker picked it up.
    ///
    /// A stage records the SUM of every worker's time in it and the single
    /// slowest item. `sum / max` is the widest the stage could ever run even
    /// on an infinite machine: when one country's tail is half the stage's
    /// total work, no amount of fan-out gets past 2×. The report prints that
    /// ratio in the `cores` column.
    #[inline(always)]
    pub fn stage<R>(name: &'static str, depth: u8, f: impl FnOnce() -> R) -> R {
        if !Self::enabled() {
            return f();
        }
        let wall0 = Instant::now();
        let out = f();
        Self::record(
            name,
            depth,
            wall0.elapsed().as_nanos() as u64,
            0,
            true,
            None,
        );
        out
    }

    /// Open a stage that ends when the returned guard drops.
    #[inline(always)]
    pub fn stage_scope(name: &'static str, depth: u8) -> StageScope {
        StageScope {
            name,
            depth,
            wall0: Self::enabled().then(Instant::now),
            label: None,
        }
    }

    fn record(
        name: &'static str,
        depth: u8,
        wall_ns: u64,
        cpu_ns: u64,
        is_stage: bool,
        label: Option<&str>,
    ) {
        let mut regions = regions().lock().expect("performance regions poisoned");
        // Linear scan: the region set is a few dozen entries and insertion
        // order is the report order, which a map would lose.
        if let Some(row) = regions.iter_mut().find(|r| r.name == name) {
            row.wall_ns += wall_ns;
            row.cpu_ns += cpu_ns;
            if wall_ns > row.max_ns {
                row.max_ns = wall_ns;
                row.max_label = label.unwrap_or_default().to_string();
            }
            row.calls += 1;
        } else {
            regions.push(MeasuredRegion {
                name,
                depth,
                wall_ns,
                cpu_ns,
                max_ns: wall_ns,
                max_label: label.unwrap_or_default().to_string(),
                calls: 1,
                is_stage,
            });
        }
    }

    /// Print the accumulated breakdown and clear it. No-op when profiling is
    /// off or nothing was recorded.
    pub fn report(label: &str) {
        if !Self::enabled() {
            return;
        }
        let mut regions = regions().lock().expect("performance regions poisoned");
        if regions.is_empty() {
            return;
        }
        // Depth-0 phases partition the tick, so their sum is the run's
        // profiled wall time; everything else is already inside one of them.
        let is_top = |r: &&MeasuredRegion| r.depth == 0 && !r.is_stage;
        let top_wall: u64 = regions.iter().filter(is_top).map(|r| r.wall_ns).sum();
        let top_cpu: u64 = regions.iter().filter(is_top).map(|r| r.cpu_ns).sum();
        eprintln!(
            "\n[PERFORMANCE {}] profiled wall {:.2} s   cpu {:.2} s   avg {:.1} cores",
            label,
            top_wall as f64 / 1e9,
            top_cpu as f64 / 1e9,
            ratio(top_cpu, top_wall),
        );
        eprintln!(
            "{:<34} {:>10} {:>7} {:>11} {:>7} {:>9} {:>7}  {}",
            "phase", "wall_ms", "%wall", "cpu_ms", "cores", "max_ms", "calls", "straggler"
        );
        for row in regions.iter() {
            let wall_ms = row.wall_ns as f64 / 1e6;
            let cores = if row.is_stage {
                // Load-balance ceiling: total thread-time over the straggler.
                ratio(row.wall_ns, row.max_ns)
            } else {
                ratio(row.cpu_ns, row.wall_ns)
            };
            let indent = "  ".repeat(row.depth as usize);
            eprintln!(
                "{:<34} {:>10.1} {:>6.1}% {:>11.1} {:>7.2} {:>9.1} {:>7}  {}",
                format!("{indent}{}", row.name),
                wall_ms,
                ratio(row.wall_ns, top_wall) * 100.0,
                row.cpu_ns as f64 / 1e6,
                cores,
                row.max_ns as f64 / 1e6,
                row.calls,
                row.max_label,
            );
        }
        // How much wall time would vanish if every phase ran at the box's
        // full width — the number this whole module exists to shrink.
        let width = std::thread::available_parallelism()
            .map(|n| n.get() as f64)
            .unwrap_or(1.0);
        let ideal: f64 = regions
            .iter()
            .filter(is_top)
            .map(|r| r.cpu_ns as f64 / 1e9 / width)
            .sum();
        let profiled = top_wall as f64 / 1e9;
        eprintln!(
            "ideal wall at {width:.0} cores {ideal:.2} s  →  headroom {:.2} s ({:.0}% of profiled wall)",
            profiled - ideal,
            (profiled - ideal) / profiled.max(1e-9) * 100.0,
        );
        regions.clear();
        drop(regions);
        sampler::report();
    }
}

/// `numerator / denominator`, or zero when the denominator is.
#[inline]
fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

/// RAII form of [`PerformanceProfiler::phase`]. Records on drop.
pub struct PhaseScope {
    name: &'static str,
    depth: u8,
    /// Offset from the run origin, so the width sampler's readings can be
    /// joined back to the phase that was running.
    start_ns: u64,
    /// `None` when profiling is off, which is also the "did this guard
    /// record anything" flag — so a disabled guard reads no clock at all,
    /// at either end.
    wall0: Option<Instant>,
    cpu0: u64,
}

impl Drop for PhaseScope {
    fn drop(&mut self) {
        let Some(wall0) = self.wall0 else {
            return;
        };
        let wall = wall0.elapsed().as_nanos() as u64;
        let cpu = process_cpu_nanos().saturating_sub(self.cpu0);
        PerformanceProfiler::record(self.name, self.depth, wall, cpu, false, None);
        sampler::push_window(self.name, self.start_ns, self.start_ns + wall);
    }
}

/// RAII form of [`PerformanceProfiler::stage`]. Records on drop.
pub struct StageScope {
    name: &'static str,
    depth: u8,
    /// `None` when profiling is off — see [`PhaseScope::wall0`]. Stages are
    /// opened a few thousand times a tick, so this is the one that has to
    /// cost nothing.
    wall0: Option<Instant>,
    label: Option<String>,
}

impl StageScope {
    /// Name the item this stage is running for, so the report can say WHICH
    /// one was the straggler. Only built when profiling is on.
    pub fn labelled(mut self, label: impl FnOnce() -> String) -> Self {
        if self.wall0.is_some() {
            self.label = Some(label());
        }
        self
    }
}

impl Drop for StageScope {
    fn drop(&mut self) {
        if let Some(wall0) = self.wall0 {
            PerformanceProfiler::record(
                self.name,
                self.depth,
                wall0.elapsed().as_nanos() as u64,
                0,
                true,
                self.label.as_deref(),
            );
        }
    }
}

impl PerformanceProfiler {
    /// [`Self::stage`] that also names the item it is running for, so the
    /// report can say WHICH country (or club, or league) was the straggler.
    /// The label closure only runs when profiling is on.
    #[inline(always)]
    pub fn stage_labelled<R>(
        name: &'static str,
        depth: u8,
        label: impl FnOnce() -> String,
        f: impl FnOnce() -> R,
    ) -> R {
        if !Self::enabled() {
            return f();
        }
        let scope = Self::stage_scope(name, depth).labelled(label);
        let out = f();
        drop(scope);
        out
    }
}
