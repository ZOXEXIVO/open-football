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

    /// Whether `line` says `name` as a whole identifier.
    ///
    /// Substring matching is useless here: `PlayerFieldPositionGroup` is the
    /// model's own vocabulary and contains `Player`, `TeamType` contains
    /// `Team`, `CountryResult` contains `Country`. A name counts only when
    /// neither neighbour is an identifier character.
    fn says(line: &str, name: &str) -> bool {
        let bytes = line.as_bytes();
        let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        line.match_indices(name).any(|(at, _)| {
            let before = at.checked_sub(1).map(|i| bytes[i]);
            let after = bytes.get(at + name.len()).copied();
            !before.is_some_and(is_word) && !after.is_some_and(is_word)
        })
    }

    /// Every non-comment line of one file that says any of `names`.
    fn mentions(relative: &str, names: &[&str]) -> Vec<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("transfers")
            .join(relative);
        let src = fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("{relative} is on the model roster but does not exist"));
        let mut out = Vec::new();
        for (index, line) in src.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if let Some(name) = names.iter().find(|n| Self::says(trimmed, n)) {
                out.push(format!("{relative}:{}  [{name}]  {trimmed}", index + 1));
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

/// `transfers` must never import from `simulator` at all.
///
/// The three things it legitimately reached for were never the tick
/// driver's: `SimulatorData` and `FreeAgentFlowCounters` are the world
/// (`crate::world`) and `PerformanceProfiler` is instrumentation
/// (`crate::utils`). With those in their own modules, `crate::simulator`
/// holds nothing but the phase order — and a market that needs to know
/// the phase order has been written upside down.
#[test]
fn transfers_never_imports_the_tick_driver() {
    let offenders = SourceScan::offenders(|line| line.starts_with("use crate::simulator"));

    assert!(
        offenders.is_empty(),
        "src/transfers may borrow the world (`crate::world`) and the profiler \
         (`crate::utils`), but nothing from the tick driver.\n{}",
        offenders.join("\n")
    );
}

/// The model layer decides from numbers; it never names the world.
///
/// This is rule 4 of the target shape. A model file that can say `Club` will
/// eventually say it — `gate/mod.rs` held both halves for a year, separated
/// by nothing but a banner comment and a mid-file `use` block, and every gate
/// in it could have reached for a club because a club was already in scope.
/// The hydration that reads the world lives beside it — `gate/build.rs`,
/// `gate/stance.rs`, `gate/fit.rs` — and hands the model its numbers.
///
/// The roster is deliberate, not "every file that happens to be clean":
/// bringing a module over is a line added here, so undoing it is a line
/// removed here rather than an edit nobody notices. Step 4 grows it one
/// module at a time.
#[test]
fn the_model_layer_never_names_a_world_object() {
    /// Things you can only get by reading the world. A model is handed the
    /// numbers instead. Position groups, squad statuses and team *types* are
    /// vocabulary, not the world, and are matched out by whole-identifier
    /// comparison rather than by an exception list.
    const WORLD_OBJECTS: [&str; 6] = [
        "Club",
        "Country",
        "Player",
        "PlayerSummary",
        "SimulatorData",
        "Team",
    ];

    const MODEL: [&str; 16] = [
        "deal/auction.rs",
        "deal/negotiation.rs",
        "deal/offer.rs",
        "deal/reason.rs",
        "gate/appraisal.rs",
        "gate/mod.rs",
        "market/affinity.rs",
        "market/map.rs",
        "market/region.rs",
        "market/route.rs",
        "market/window.rs",
        "pool/mod.rs",
        "scouting/exposure.rs",
        "squad/bands.rs",
        "squad/ledger.rs",
        "squad/minutes.rs",
    ];

    let offenders: Vec<String> = MODEL
        .iter()
        .flat_map(|file| SourceScan::mentions(file, &WORLD_OBJECTS))
        .collect();

    assert!(
        offenders.is_empty(),
        "the model layer decides from numbers and must not name the world. \
         Either the code belongs in this folder's hydration file, or the \
         value it wants should be read there and passed in.\n{}",
        offenders.join("\n")
    );
}
