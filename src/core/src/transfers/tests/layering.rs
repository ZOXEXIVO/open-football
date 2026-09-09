//! Source-level guards on the transfer system's dependency direction.
//!
//! The market layer is meant to be a leaf: the country pass drives it, and
//! it reads the world through `club`, `player` and `shared`. For a long
//! time it was not — the pipeline reached back into `country::result` for
//! two items, closing a module cycle that kept the whole subsystem from
//! ever being lifted into its own crate and left it with no enforceable
//! direction at all.
//!
//! A cycle takes one `use` line to reintroduce and a full read of 58 000
//! lines to notice, so it is asserted here rather than remembered.

use std::fs;
use std::path::{Path, PathBuf};

/// Walks `src/transfers` and answers questions about what its sources say.
struct SourceScan;

impl SourceScan {
    /// This file names the forbidden paths in prose; it is never an
    /// offender.
    const SELF: &'static str = "layering.rs";

    /// Every `.rs` file under `src/transfers`, recursively.
    fn sources() -> Vec<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("transfers");
        let mut out = Vec::new();
        Self::walk(&root, &mut out);
        assert!(
            !out.is_empty(),
            "no sources found under src/transfers — the scanner's path is wrong, \
             not the codebase"
        );
        out
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    /// Path relative to `src/`, for a readable failure message.
    fn label(path: &Path) -> String {
        path.to_string_lossy()
            .replace('\\', "/")
            .rsplit_once("/src/")
            .map(|(_, tail)| tail.to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string())
    }

    /// Every non-comment line in the tree for which `hit` holds, as
    /// `path:line  text`.
    fn offenders(hit: impl Fn(&str) -> bool) -> Vec<String> {
        let mut out = Vec::new();
        for path in Self::sources() {
            if path.file_name().and_then(|f| f.to_str()) == Some(Self::SELF) {
                continue;
            }
            let Ok(src) = fs::read_to_string(&path) else {
                continue;
            };
            for (index, line) in src.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") || !hit(trimmed) {
                    continue;
                }
                out.push(format!("{}:{}  {}", Self::label(&path), index + 1, trimmed));
            }
        }
        out
    }
}

/// `transfers` must never import from `country::result`.
///
/// The country pass owns the tick and calls into the market; the market
/// answers. Reaching the other way is how the cycle was formed: one
/// predicate (`ClubView::can_accept_player`) and one collector
/// (`FreeAgentBumpBatch`) lived on the country side because that is where
/// their first caller was, and both are market concepts. They now live in
/// [`crate::transfers::view::club`] and [`crate::transfers::pool`].
///
/// If this fails: the item you want is a market concept — move it into
/// `transfers` — or the code you are writing belongs on the country side.
#[test]
fn transfers_never_imports_the_country_result_layer() {
    let offenders = SourceScan::offenders(|line| line.contains("crate::country::result"));

    assert!(
        offenders.is_empty(),
        "src/transfers must not depend on crate::country::result — that closes a \
         module cycle and pins the market to the country pass forever.\n{}",
        offenders.join("\n")
    );
}

/// `transfers` must never import from `simulator` except the profiler.
///
/// A softer guard than the one above, and deliberately so: the pipeline
/// still takes `&mut SimulatorData` on the world-level paths, and that is
/// the reach fork the refactor is collapsing rather than a rule it can
/// assert today. What it CAN hold is that no new dependency on the tick
/// orchestrator's internals creeps in while that work is in flight —
/// `PerformanceProfiler` is instrumentation, `SimulatorData` is the world,
/// and everything else in `simulator` belongs to the driver.
#[test]
fn transfers_only_borrows_the_simulator_world_and_its_profiler() {
    const ALLOWED: [&str; 3] = [
        "SimulatorData",
        "PerformanceProfiler",
        "FreeAgentFlowCounters",
    ];

    let offenders = SourceScan::offenders(|line| {
        line.starts_with("use crate::simulator") && !ALLOWED.iter().any(|n| line.contains(n))
    });

    assert!(
        offenders.is_empty(),
        "src/transfers may borrow the world (`SimulatorData`) and the profiler, \
         but not the tick driver's internals.\n{}",
        offenders.join("\n")
    );
}
