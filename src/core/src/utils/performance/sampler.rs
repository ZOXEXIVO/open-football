//! Instantaneous-parallelism sampler.
//!
//! The `cores` column in the phase table is an average, and an average hides
//! the shape that matters: a phase at 16 cores might be 32 cores throughout,
//! or 32 cores for half its wall and 2 for the other half — a straggler
//! tail. Only the second is worth attacking, and only the second is what
//! "it sometimes only loads 2-3 cores" describes.
//!
//! So when profiling is on, one background thread samples the process cycle
//! counter every [`SAMPLE_INTERVAL`]; each sample's cycles over its wall
//! slice is the number of cores that were busy across it. Phases record
//! their `[start, end)` windows and the report joins the two, giving — per
//! phase — how much wall time was spent at each width.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::clock::{process_cpu_nanos, process_cycles};

/// Sampling period. Fine enough to resolve a tail inside a ~300 ms phase,
/// coarse enough that the sampler itself is invisible in the profile.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(2);

/// Width buckets, as the upper bound of each band in busy cores. The last
/// bucket catches everything above.
const CORE_BUCKETS: [f64; 6] = [2.0, 4.0, 8.0, 16.0, 24.0, f64::INFINITY];
const CORE_BUCKET_LABELS: [&str; 6] = ["<2", "2-4", "4-8", "8-16", "16-24", "24+"];

/// One sampler reading: wall offset from the run origin, the cycles the
/// process burned over the slice that ended there, and the slice's length.
struct CoreSample {
    at_ns: u64,
    cycles: u64,
    dt_ns: u64,
}

/// One profiled window on the tick's driving thread.
struct PhaseWindow {
    name: &'static str,
    start_ns: u64,
    end_ns: u64,
}

fn samples() -> &'static Mutex<Vec<CoreSample>> {
    static SAMPLES: OnceLock<Mutex<Vec<CoreSample>>> = OnceLock::new();
    SAMPLES.get_or_init(|| Mutex::new(Vec::new()))
}

fn windows() -> &'static Mutex<Vec<PhaseWindow>> {
    static WINDOWS: OnceLock<Mutex<Vec<PhaseWindow>>> = OnceLock::new();
    WINDOWS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Run start, so samples and windows share one time origin.
pub fn origin() -> Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

/// Nanoseconds since the run origin.
#[inline]
pub fn now_ns() -> u64 {
    origin().elapsed().as_nanos() as u64
}

/// Start the sampling thread. Called once, from the profiler's env latch.
pub fn spawn() {
    origin();
    std::thread::Builder::new()
        .name("performance_sampler".into())
        .spawn(|| {
            let start = origin();
            let mut last_wall = start.elapsed().as_nanos() as u64;
            let mut last_cycles = process_cycles();
            loop {
                std::thread::sleep(SAMPLE_INTERVAL);
                let wall = start.elapsed().as_nanos() as u64;
                let cycles = process_cycles();
                let dt = wall.saturating_sub(last_wall);
                if dt > 0 {
                    samples()
                        .lock()
                        .expect("performance samples poisoned")
                        .push(CoreSample {
                            at_ns: wall,
                            cycles: cycles.saturating_sub(last_cycles),
                            dt_ns: dt,
                        });
                }
                last_wall = wall;
                last_cycles = cycles;
            }
        })
        .expect("performance sampler thread");
}

/// Record the window a phase occupied, for the report's join.
pub fn push_window(name: &'static str, start_ns: u64, end_ns: u64) {
    windows()
        .lock()
        .expect("performance windows poisoned")
        .push(PhaseWindow {
            name,
            start_ns,
            end_ns,
        });
}

/// Per-phase width histogram: for each profiled phase, how much of its wall
/// time ran at each core count. Printed after the phase table.
pub fn report() {
    let samples = samples().lock().expect("performance samples poisoned");
    let mut windows = windows().lock().expect("performance windows poisoned");
    if samples.is_empty() || windows.is_empty() {
        return;
    }

    // Cycles → busy cores. Both counters are cumulative from process start,
    // so their ratio is this box's effective cycles per CPU second under
    // this workload — no nominal clock rate needed, and frequency scaling is
    // already baked in.
    let cpu_seconds = process_cpu_nanos() as f64 / 1e9;
    let cycles_per_cpu_nano = if cpu_seconds > 0.0 {
        process_cycles() as f64 / (cpu_seconds * 1e9)
    } else {
        0.0
    };
    if cycles_per_cpu_nano <= 0.0 {
        return;
    }

    // One row per distinct phase name, in first-seen order.
    let mut names: Vec<&'static str> = Vec::new();
    for window in windows.iter() {
        if !names.contains(&window.name) {
            names.push(window.name);
        }
    }
    // Windows are pushed in completion order; sort by start so the per-phase
    // scan can seek straight to its first sample.
    windows.sort_by_key(|w| w.start_ns);

    eprintln!(
        "\nwall time by busy-core count (ms)\n{:<34} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "phase",
        CORE_BUCKET_LABELS[0],
        CORE_BUCKET_LABELS[1],
        CORE_BUCKET_LABELS[2],
        CORE_BUCKET_LABELS[3],
        CORE_BUCKET_LABELS[4],
        CORE_BUCKET_LABELS[5],
    );
    for name in names {
        let mut buckets = [0f64; CORE_BUCKETS.len()];
        for window in windows.iter().filter(|w| w.name == name) {
            // Samples are in time order; a sample lands in the window it
            // ended inside.
            let first = samples.partition_point(|s| s.at_ns < window.start_ns);
            for sample in samples[first..]
                .iter()
                .take_while(|s| s.at_ns <= window.end_ns)
            {
                let cores = sample.cycles as f64 / cycles_per_cpu_nano / sample.dt_ns as f64;
                let bucket = CORE_BUCKETS
                    .iter()
                    .position(|&hi| cores < hi)
                    .unwrap_or(CORE_BUCKETS.len() - 1);
                buckets[bucket] += sample.dt_ns as f64 / 1e6;
            }
        }
        if buckets.iter().sum::<f64>() < 1.0 {
            continue;
        }
        eprintln!(
            "{:<34} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1}",
            name, buckets[0], buckets[1], buckets[2], buckets[3], buckets[4], buckets[5],
        );
    }
}
