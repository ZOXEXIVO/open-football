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

/// Walks the crate's sources and answers questions about what they say.
struct SourceScan;

impl SourceScan {
    /// This file names the forbidden paths in prose; it is never an
    /// offender.
    const SELF: &'static str = "layering.rs";

    /// Every `.rs` file under `src/transfers`, recursively.
    fn sources() -> Vec<PathBuf> {
        Self::sources_under(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("transfers"),
        )
    }

    /// Every `.rs` file under `src/`, recursively.
    fn crate_sources() -> Vec<PathBuf> {
        Self::sources_under(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
    }

    fn sources_under(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        Self::walk(root, &mut out);
        assert!(
            !out.is_empty(),
            "no sources found under {} — the scanner's path is wrong, not the codebase",
            root.display()
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

    /// A file that only builds tests: one under a `tests` folder, or a
    /// `tests.rs` beside its module.
    fn is_test_file(path: &Path) -> bool {
        path.components().any(|c| c.as_os_str() == "tests")
            || path.file_stem().and_then(|f| f.to_str()) == Some("tests")
    }

    /// The source before its test tail, comments dropped, with a method
    /// chain continued on the next line folded into the line it continues,
    /// so `plan\n.scout_monitoring\n.push(..)` reads as one statement. The
    /// tail starts at the first `#[cfg(test)]` that opens an inline module.
    fn production_statements(src: &str) -> Vec<(usize, String)> {
        let lines: Vec<&str> = src.lines().collect();
        let mut out: Vec<(usize, String)> = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed == "#[cfg(test)]"
                && lines[index + 1..]
                    .iter()
                    .map(|l| l.trim())
                    .find(|l| !l.is_empty())
                    .is_some_and(|next| next.starts_with("mod ") && next.ends_with('{'))
            {
                break;
            }
            if trimmed.starts_with("//") || trimmed.is_empty() {
                continue;
            }
            match out.last_mut() {
                Some((_, statement)) if trimmed.starts_with('.') => statement.push_str(trimmed),
                _ => out.push((index + 1, trimmed.to_string())),
            }
        }
        out
    }

    /// Whether `statement` opens or closes a place in one of a club's
    /// attention collections — `scout_monitoring`, `watchlist`,
    /// `staff_recommendations`, `shortlists` or a shortlist's `candidates`:
    /// pushes, inserts, retains, removes, clears, truncates, extends,
    /// drains, dedups or replaces it. Moving a row through its statuses is
    /// the negotiation's lifecycle, not a file opened or closed.
    fn writes_attention(statement: &str) -> bool {
        const FIELDS: [&str; 5] = [
            "scout_monitoring",
            "watchlist",
            "staff_recommendations",
            "shortlists",
            "candidates",
        ];
        const WRITES: [&str; 11] = [
            "push(",
            "insert(",
            "retain(",
            "remove(",
            "clear(",
            "truncate(",
            "extend(",
            "drain(",
            "dedup",
            "swap_remove(",
            "append(",
        ];
        let assigns = |rest: &str| {
            let rest = rest.trim_start();
            rest.starts_with('=') && !rest.starts_with("==")
        };
        let bytes = statement.as_bytes();
        let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        FIELDS.iter().any(|field| {
            let needle = format!(".{field}");
            statement.match_indices(&needle).any(|(at, _)| {
                let end = at + needle.len();
                if bytes.get(end).copied().is_some_and(is_word) {
                    return false;
                }
                let rest = &statement[end..];
                if let Some(call) = rest.trim_start().strip_prefix('.') {
                    return WRITES.iter().any(|w| call.starts_with(w));
                }
                assigns(rest)
            })
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

/// A club's attention on another club's player is opened, kept and closed
/// in one place: the door on `ClubTransferPlan`, which takes the gate's
/// reading and remembers it. A route that pushes a monitoring row or a
/// shortlist place itself skips the reading, and the weekly review can
/// then only close what it never saw opened.
///
/// If this fails: hand the reading to `keep_file`, `board`, `recommend`,
/// `shortlist` or `open_shortlist`, or add the lifecycle step you need as a
/// method beside them.
#[test]
fn attention_state_is_written_only_through_the_plan() {
    const OWNER: &str = "transfers/pipeline/mod.rs";

    let mut offenders = Vec::new();
    for path in SourceScan::crate_sources() {
        let label = SourceScan::label(&path);
        if label == OWNER || SourceScan::is_test_file(&path) {
            continue;
        }
        let Ok(src) = fs::read_to_string(&path) else {
            continue;
        };
        for (line, statement) in SourceScan::production_statements(&src) {
            if SourceScan::writes_attention(&statement) {
                offenders.push(format!("{label}:{line}  {statement}"));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "attention state is written only by the plan's door in {OWNER}.\n{}",
        offenders.join("\n")
    );
}

/// The guard above reads statements, not files: these are the shapes it
/// must catch and the reads it must let through.
#[test]
fn the_attention_guard_knows_a_write_from_a_read() {
    for write in [
        "club.transfer_plan.scout_monitoring.push(row);",
        "plan.watchlist = vec![entry];",
        "plan.staff_recommendations.retain(|r| r.player_id != id);",
        "shortlist.candidates = candidates;",
        "list.candidates.insert(at, candidate),",
        "plan.shortlists.push(TransferShortlist::new(request_id, 0.0));",
    ] {
        assert!(SourceScan::writes_attention(write), "missed: {write}");
    }
    for read in [
        "plan.scout_monitoring.iter().filter(|m| m.is_active_interest())",
        "if plan.watchlist.len() == 0 {",
        "s.candidates.iter().any(|c| c.player_id == id)",
        "plan.loan_out_candidates.retain(|c| c.player_id != id);",
        "let n = plan.shortlists[0].candidates.len();",
        "plan.staff_recommendations == other",
        "for m in plan.scout_monitoring.iter_mut() {",
        "shortlist.candidates[pos].status = ShortlistCandidateStatus::CurrentlyPursuing;",
    ] {
        assert!(
            !SourceScan::writes_attention(read),
            "flagged a read: {read}"
        );
    }
    let folded = SourceScan::production_statements(
        "club.transfer_plan\n    .scout_monitoring\n    .push(row);\n",
    );
    assert!(SourceScan::writes_attention(&folded[0].1));
}

/// The gates this crate retired stay retired. Standing fit in
/// `transfers/gate` answers every route; a second, private realism check
/// elsewhere is how a route drifted from the rest before.
#[test]
fn no_source_names_a_retired_gate() {
    const RETIRED: [&str; 16] = [
        "BuyerPlausibilityContext",
        "MarketReachCache",
        "RealismBuyer",
        "RealismConfig",
        "RealismTarget",
        "TransferPlausibilityVerdict",
        "assess_summary",
        "buyer_outranks_parent",
        "evaluate_summary",
        "is_openly_available",
        "is_target_realistic",
        "is_target_realistic_fields",
        "min_source_rep",
        "selections_read_fringe",
        "target_affordable_for_buyer",
        "waives_wage_floor",
    ];

    let mut offenders = Vec::new();
    for path in SourceScan::crate_sources() {
        if path.file_name().and_then(|f| f.to_str()) == Some(SourceScan::SELF) {
            continue;
        }
        let Ok(src) = fs::read_to_string(&path) else {
            continue;
        };
        for (index, line) in src.lines().enumerate() {
            if let Some(name) = RETIRED.iter().find(|n| SourceScan::says(line, n)) {
                offenders.push(format!(
                    "{}:{}  [{name}]  {}",
                    SourceScan::label(&path),
                    index + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a retired gate is named again — read standing fit through \
         `TransferMovePlausibility::read` instead.\n{}",
        offenders.join("\n")
    );
}
