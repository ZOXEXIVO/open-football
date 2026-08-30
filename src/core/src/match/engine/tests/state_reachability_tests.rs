//! Reachability guard for the player state machine.
//!
//! Every `PlayerState` in the universe is meant to be a state a player can
//! actually occupy during a match. Nothing enforced that, and the drift was
//! severe: an audit of the exported transition graph found **12 of 79
//! states that no match ever entered** — the whole goalkeeper distribution
//! fork (`Throwing` / `Kicking` / `PickingUpBall`), `MidfielderState::
//! Walking` and `Distributing`, `ForwardState::Finishing`, `Injured`, and
//! four goalkeeper states with no reason to exist at all. Each had a
//! complete, maintained handler that never ran, and each was silently
//! costing behaviour: midfielders never used the low-intensity fatigue
//! band because they could not walk, and every keeper in every match
//! released the ball exactly one way.
//!
//! Two layers guard it now:
//!
//! * [`static_reachability`] — always compiled, no match run. Scans the
//!   engine source for transition sites and fails if a state is never
//!   constructed as a transition target.
//! * [`observed_graph_has_no_dead_states`] — `match-logs` only. Runs the
//!   real recorder's audit over an observed edge set, which is precise
//!   where the source scan is merely conservative.

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Source scanner for state-transition sites.
struct TransitionSiteScanner;

impl TransitionSiteScanner {
    /// Root of the engine source tree.
    fn engine_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("match")
    }

    /// Files whose mentions of a state are declaration rather than use:
    /// the four role enums (variant list, dispatch arm, `Display` arm) and
    /// this test itself.
    fn is_declaration_file(path: &Path) -> bool {
        let name = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
        if name == "state_reachability_tests.rs" {
            return true;
        }
        // `<role>/states/state.rs` — the enum definitions.
        name == "state.rs"
            && path
                .to_string_lossy()
                .replace('\\', "/")
                .contains("/states/state.rs")
    }

    /// Collect every `.rs` file that can legitimately drive a transition.
    fn source_files() -> Vec<PathBuf> {
        let mut out = Vec::new();
        Self::walk(&Self::engine_dir(), &mut out);
        out
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
                && !Self::is_declaration_file(&path)
            {
                out.push(path);
            }
        }
    }

    /// Names of every state mentioned outside the enum declarations, as
    /// `"<Enum>::<Variant>"`.
    fn mentioned_states() -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let enums = [
            "GoalkeeperState",
            "DefenderState",
            "MidfielderState",
            "ForwardState",
        ];
        for path in Self::source_files() {
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in src.lines() {
                // Strip line comments so prose about a retired state
                // can't keep it looking alive.
                let code = match line.find("//") {
                    Some(i) => &line[..i],
                    None => line,
                };
                for enum_name in enums {
                    let needle = format!("{enum_name}::");
                    let mut from = 0;
                    while let Some(rel) = code[from..].find(&needle) {
                        let start = from + rel + needle.len();
                        let variant: String = code[start..]
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect();
                        from = start + variant.len().max(1);
                        if variant.is_empty() || variant == "ALL" {
                            continue;
                        }
                        found.insert(format!("{enum_name}::{variant}"));
                    }
                }
            }
        }
        found
    }
}

/// `"<Enum>::<Variant>"` for a `PlayerState`, matching the scanner's key
/// format. `Injured` has no role enum, so it is handled by the caller.
fn scanner_key(state: PlayerState) -> Option<String> {
    let name = match state {
        PlayerState::Injured => return None,
        PlayerState::Goalkeeper(s) => format!("GoalkeeperState::{s:?}"),
        PlayerState::Defender(s) => format!("DefenderState::{s:?}"),
        PlayerState::Midfielder(s) => format!("MidfielderState::{s:?}"),
        PlayerState::Forward(s) => format!("ForwardState::{s:?}"),
    };
    Some(name)
}

#[test]
fn static_reachability() {
    // Conservative by construction: a state counts as reachable if it is
    // named ANYWHERE outside its enum declaration, including inside a
    // `matches!` read. That direction is deliberate — the failure mode
    // this catches is a state with no mention at all, which is exactly
    // what every one of the 12 dead states looked like. A state that is
    // only ever read and never transitioned into would slip through here
    // and be caught by the `match-logs` observed-graph audit below.
    let mentioned = TransitionSiteScanner::mentioned_states();
    let entry: Vec<PlayerState> = PlayerState::entry_states().to_vec();
    let reserved: Vec<PlayerState> = PlayerState::reserved_states().to_vec();

    let mut unreachable = Vec::new();
    for state in PlayerState::all() {
        if reserved.iter().any(|r| *r == state) {
            continue;
        }
        // Entry states are reached via `set_default_state`, not a named
        // transition, but they ARE named there — so they need no
        // exemption. Kept explicit so the intent survives a refactor.
        let is_entry = entry.iter().any(|e| *e == state);
        let Some(key) = scanner_key(state) else {
            // `Injured` — reached through `PlayerState::Injured` in the
            // substitution layer's injury roll, which the enum-keyed scan
            // above doesn't cover. Check it directly.
            let injured_wired = TransitionSiteScanner::source_files().iter().any(|p| {
                std::fs::read_to_string(p)
                    .map(|s| s.contains("PlayerState::Injured"))
                    .unwrap_or(false)
            });
            assert!(
                injured_wired,
                "PlayerState::Injured has no transition site — it is either wired \
                 into the match (via the in-match injury roll) or listed in \
                 `PlayerState::reserved_states()`"
            );
            continue;
        };
        if !mentioned.contains(&key) && !is_entry {
            unreachable.push(key);
        }
    }

    assert!(
        unreachable.is_empty(),
        "state(s) exist in the universe but nothing in the engine transitions into \
         them — either wire them up or delete them (retiring a state means removing \
         its variant, its handler module and its `ALL` entry, leaving the \
         discriminant permanently unused so replays keep decoding): {unreachable:?}"
    );
}

#[test]
fn every_state_has_a_handler_and_a_distinct_id() {
    // Cheap structural companion to the reachability scan: the dispatch
    // `match` is exhaustive by compilation, so this pins the pieces the
    // compiler can't — that `ALL` really lists every variant (via the id
    // count) and that no two states collide in the replay id space.
    let all = PlayerState::all();
    let ids: BTreeSet<u16> = all.iter().map(|s| s.compact_id()).collect();
    assert_eq!(
        ids.len(),
        all.len(),
        "compact_id collision in the state space"
    );

    assert_eq!(
        all.len(),
        1 + GoalkeeperState::ALL.len()
            + DefenderState::ALL.len()
            + MidfielderState::ALL.len()
            + ForwardState::ALL.len(),
        "PlayerState::all() disagrees with the per-role ALL registries"
    );
}

/// Precise version of the guard: run the real audit over an observed edge
/// set. Requires the `match-logs` recorder, so it is feature-gated to keep
/// the default test run fast.
///
/// # What a unit test can and cannot claim here
///
/// The edge set is whatever this PROCESS happened to simulate, and only
/// two files in the suite play a match at all (`goal_clip_recording_tests`
/// and `friendly_recording_tests`). Two matches is nowhere near enough to
/// see every state: `Defender: Shooting`, `Goalkeeper: Punching` and
/// `Injured` are all genuinely rare, and at that sample size "no inbound
/// edge" means "did not come up", not "cannot be reached".
///
/// So this asserts the invariant the sample actually supports — **a state
/// the run entered must have a way in and a way out** — filtering to
/// states the run exercised, exactly as `dev_match stats` already does at
/// its own call site ("an unreached state is 'not observed', not a
/// structural dead-end").
///
/// ⚠ It therefore does NOT answer "does every state occur in a real
/// match". That question needs hundreds of matches and lives in the
/// harness: `dev_match stats 400 14 14` prints `states observed: N/81`
/// and names the ones it never saw. Left unfiltered here, this test was
/// simply red — 16 states on master, before any of the work that added
/// this note.
#[cfg(feature = "match-logs")]
#[test]
fn observed_graph_has_no_dead_states() {
    use crate::r#match::player::transition::{GraphInvariantViolation, TransitionGraph};
    use std::collections::HashSet;

    // The recorder accumulates across every match run in the process, so
    // whatever the rest of the suite has already simulated contributes.
    let edges = TransitionGraph::edges();
    if edges.is_empty() {
        // Nothing simulated in this process — the static scan above is the
        // guard in that case.
        return;
    }

    // ⚠ **Only a COMPLETED match can be audited.** Most of the edges in
    // a test process come from fixtures that drive players directly —
    // the corner-shape, throw-in and substitution-break tests — and
    // never reach the end of `play_with_config`. Those leave every
    // player parked in whatever state the fixture stopped him in, with
    // no outbound edge and no whistle to excuse it, so each one reads as
    // a dead end. `final_states()` is empty until a real match finishes,
    // which is exactly the condition under which this audit means
    // something.
    //
    // Tests run in parallel and in an arbitrary order, so whether that
    // has happened by the time this runs is luck. It is genuinely
    // opportunistic; the guaranteed version of this check is
    // `dev_match stats`, over hundreds of matches, and `static_reachability`
    // above is the one that always runs.
    let whistled = TransitionGraph::final_states();
    if whistled.is_empty() {
        return;
    }

    let universe = PlayerState::all();
    // Reserved states count as entries AND as terminals: `Injured` is
    // reached through the injury roll rather than a handler transition,
    // and nothing leads out of it. Same pair the harness passes.
    let reserved = PlayerState::reserved_states();
    let mut entry = PlayerState::entry_states().to_vec();
    entry.extend(reserved);
    // …plus whatever the twenty-two were doing when each whistle went. A
    // player still in a state has not left it, so it has no outbound
    // edge and reads exactly like a dead end. See
    // `TransitionGraph::note_final_states`.
    let mut terminal = reserved.to_vec();
    terminal.extend(whistled);
    let violations = TransitionGraph::audit(&edges, &universe, &entry, &terminal);

    // Only states this run actually exercised. See the note above.
    let observed: HashSet<u16> = edges
        .iter()
        .flat_map(|e| [e.from.compact_id(), e.to.compact_id()])
        .collect();

    let unreachable: Vec<u16> = violations
        .iter()
        .filter_map(|v| match v {
            GraphInvariantViolation::Unreachable(id) if observed.contains(id) => Some(*id),
            _ => None,
        })
        .collect();
    let dead_ends: Vec<u16> = violations
        .iter()
        .filter_map(|v| match v {
            GraphInvariantViolation::DeadEnd(id) if observed.contains(id) => Some(*id),
            _ => None,
        })
        .collect();

    let name_of = |id: u16| {
        universe
            .iter()
            .find(|s| s.compact_id() == id)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("state_{id}"))
    };

    assert!(
        unreachable.is_empty(),
        "state(s) a player was observed LEAVING with no observed way in — \
         something puts him there without a recorded transition: {:?}",
        unreachable.iter().copied().map(name_of).collect::<Vec<_>>()
    );
    assert!(
        dead_ends.is_empty(),
        "state(s) with no way out once entered: {:?}",
        dead_ends.iter().copied().map(name_of).collect::<Vec<_>>()
    );
}
