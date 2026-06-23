use crate::r#match::player::state::PlayerState;

/// What kind of code path drove a state transition.
///
/// Every transition flows through [`MatchPlayer::transition_to`], which
/// tags it with one of these. The tag lets the transition-graph audit
/// colour and group edges by origin, and lets a reviewer tell a normal AI
/// hand-off (the bulk of the graph) from the out-of-band overrides that
/// deliberately bypass the per-state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransitionSource {
    /// Normal state-machine hand-off: a state handler returned a
    /// `StateChangeResult { state: Some(..) }` which `change_state`
    /// applied.
    Handler,
    /// The universal loose-ball override in `PlayerFieldPositionGroup`:
    /// force the closest chaser into TakeBall, or yield a stale chaser
    /// back out when a teammate is closer.
    LooseBallOverride,
    /// An event handler reacted to a dispatched event — e.g.
    /// `run_for_ball` fired from a loose-ball signal.
    EventHandler,
    /// A restart that rebuilds the formation: kickoff, goal reset, half
    /// or extra-time restart — anything routed through `set_default_state`
    /// by `reset_players_positions` / `assign_kickoff`.
    Reset,
    /// A substitution swapped a fresh player onto the pitch.
    Substitution,
    /// A set-piece teleport forced a state (the corner centre-back
    /// push-up that lands directly in `AttackingCorner`).
    SetPiece,
}

impl TransitionSource {
    /// Stable lowercase tag used in the DOT export and edge classification.
    pub fn as_tag(self) -> &'static str {
        match self {
            TransitionSource::Handler => "handler",
            TransitionSource::LooseBallOverride => "loose_ball_override",
            TransitionSource::EventHandler => "event_handler",
            TransitionSource::Reset => "reset",
            TransitionSource::Substitution => "substitution",
            TransitionSource::SetPiece => "set_piece",
        }
    }

    /// DOT edge colour per source, so the rendered graph separates the
    /// normal machine (black) from the overrides at a glance.
    pub fn dot_color(self) -> &'static str {
        match self {
            TransitionSource::Handler => "black",
            TransitionSource::LooseBallOverride => "red",
            TransitionSource::EventHandler => "blue",
            TransitionSource::Reset => "gray60",
            TransitionSource::Substitution => "green4",
            TransitionSource::SetPiece => "orange3",
        }
    }
}

/// One observed transition edge: `from -> to` with the source that drove
/// it. Carries the full [`PlayerState`] endpoints so the DOT export can
/// label nodes; identity for dedup is the pair of stable `compact_id()`s
/// plus the source (see [`GraphEdge::key`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphEdge {
    pub from: PlayerState,
    pub to: PlayerState,
    pub source: TransitionSource,
}

impl GraphEdge {
    /// Layout-independent identity used to dedup edges: the two stable
    /// compact ids and the source.
    pub fn key(&self) -> (u16, u16, TransitionSource) {
        (self.from.compact_id(), self.to.compact_id(), self.source)
    }
}

/// An invariant the observed graph can violate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphInvariantViolation {
    /// A state with no inbound edge that is not a documented entry state —
    /// the design says the player can reach it, but nothing transitions
    /// into it.
    Unreachable(u16),
    /// A non-terminal state with no outbound edge — once entered the
    /// player can never leave it (an unintended sink).
    DeadEnd(u16),
}

/// Audit + diagnostics namespace for the player state-transition graph.
///
/// In production (`match-logs` off) the recorder entry points compile
/// out, so the engine pays nothing. Under `--features match-logs` the
/// recorder accumulates the global set of observed `from -> to` edges so
/// the dev harness can export a Graphviz view (`export_dot`) and the
/// invariant checks can run against a real match's graph.
///
/// The pure helpers ([`render_dot`](TransitionGraph::render_dot),
/// [`audit`](TransitionGraph::audit)) are always compiled and unit
/// testable without the recorder.
pub struct TransitionGraph;

impl TransitionGraph {
    /// Render an edge set as a Graphviz DOT digraph. Pure — the node and
    /// edge formatting is exercised by unit tests without enabling the
    /// recorder. Nodes are keyed by stable `compact_id` and labelled with
    /// the state's `Display` text; edges are coloured by source.
    pub fn render_dot(edges: &[GraphEdge]) -> String {
        let mut out = String::from("digraph player_state_transitions {\n");
        out.push_str("  rankdir=LR;\n");
        out.push_str("  node [shape=box, fontsize=10];\n");

        // Emit each distinct node once, in ascending compact-id order so
        // the output is deterministic regardless of edge insertion order.
        let mut node_ids: Vec<u16> = Vec::new();
        for st in edges.iter().flat_map(|e| [e.from, e.to]) {
            let id = st.compact_id();
            if let Err(pos) = node_ids.binary_search(&id) {
                node_ids.insert(pos, id);
            }
        }
        for id in &node_ids {
            // ids are few, so resolving the label by scanning the edges
            // once per id is cheap and keeps the node set the single
            // source of truth.
            let label = edges
                .iter()
                .flat_map(|e| [e.from, e.to])
                .find(|st| st.compact_id() == *id)
                .map(|st| st.to_string())
                .unwrap_or_else(|| format!("state_{id}"));
            out.push_str(&format!("  s{id} [label=\"{label}\"];\n"));
        }

        for e in edges {
            out.push_str(&format!(
                "  s{} -> s{} [color={}, tooltip=\"{}\"];\n",
                e.from.compact_id(),
                e.to.compact_id(),
                e.source.dot_color(),
                e.source.as_tag(),
            ));
        }

        out.push_str("}\n");
        out
    }

    /// Check the structural invariants over an observed edge set.
    ///
    /// * every state in `universe` has at least one inbound edge, unless
    ///   it is listed in `entry_states`;
    /// * every state in `universe` has at least one outbound edge, unless
    ///   it is listed in `terminal_states`.
    ///
    /// Returns the violations (empty == healthy). Pure: the caller decides
    /// the universe / entry / terminal sets, so the same checker serves
    /// both the synthetic unit tests and the real match-logs audit.
    pub fn audit(
        edges: &[GraphEdge],
        universe: &[PlayerState],
        entry_states: &[PlayerState],
        terminal_states: &[PlayerState],
    ) -> Vec<GraphInvariantViolation> {
        let mut violations = Vec::new();
        for &st in universe {
            let id = st.compact_id();
            let is_entry = entry_states.iter().any(|e| e.compact_id() == id);
            let is_terminal = terminal_states.iter().any(|t| t.compact_id() == id);
            let has_inbound = edges.iter().any(|e| e.to.compact_id() == id);
            let has_outbound = edges.iter().any(|e| e.from.compact_id() == id);
            if !has_inbound && !is_entry {
                violations.push(GraphInvariantViolation::Unreachable(id));
            }
            if !has_outbound && !is_terminal {
                violations.push(GraphInvariantViolation::DeadEnd(id));
            }
        }
        violations
    }
}

// ───────────────────────────────────────────────────────────────────────
// Global recorder — `match-logs` only. Accumulates the set of distinct
// transition edges observed across every match run in the process, so the
// dev harness can dump the graph after a batch of simulations.
// ───────────────────────────────────────────────────────────────────────
#[cfg(feature = "match-logs")]
mod recorder {
    use super::{GraphEdge, TransitionGraph, TransitionSource};
    use crate::r#match::player::state::PlayerState;
    use std::collections::HashSet;
    use std::sync::Mutex;

    struct Store {
        edges: Vec<GraphEdge>,
        seen: HashSet<(u16, u16, TransitionSource)>,
    }

    static STORE: Mutex<Option<Store>> = Mutex::new(None);

    impl TransitionGraph {
        /// Record a `from -> to` edge tagged with its source. Deduped by
        /// the layout-independent edge key, so repeated runs of the same
        /// transition collapse to one edge. No-op in production builds
        /// (this whole module is `match-logs` only).
        pub fn record(from: PlayerState, to: PlayerState, source: TransitionSource) {
            let edge = GraphEdge { from, to, source };
            let key = edge.key();
            let mut guard = STORE.lock().unwrap();
            let store = guard.get_or_insert_with(|| Store {
                edges: Vec::new(),
                seen: HashSet::new(),
            });
            if store.seen.insert(key) {
                store.edges.push(edge);
            }
        }

        /// Clear every recorded edge — call before a measured run.
        pub fn reset() {
            let mut guard = STORE.lock().unwrap();
            *guard = None;
        }

        /// Snapshot the distinct edges observed so far.
        pub fn edges() -> Vec<GraphEdge> {
            let guard = STORE.lock().unwrap();
            guard.as_ref().map(|s| s.edges.clone()).unwrap_or_default()
        }

        /// Render the recorded graph as Graphviz DOT.
        pub fn export_dot() -> String {
            TransitionGraph::render_dot(&TransitionGraph::edges())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GraphEdge, GraphInvariantViolation, TransitionGraph, TransitionSource};
    use crate::r#match::forwarders::states::ForwardState;
    use crate::r#match::player::state::PlayerState;
    use std::path::{Path, PathBuf};

    #[test]
    fn audit_flags_unreachable_and_dead_end() {
        let a = PlayerState::Forward(ForwardState::Standing);
        let b = PlayerState::Forward(ForwardState::Running);
        let c = PlayerState::Forward(ForwardState::Dribbling);
        let edges = [GraphEdge {
            from: a,
            to: b,
            source: TransitionSource::Handler,
        }];
        let universe = [a, b, c];
        let v = TransitionGraph::audit(&edges, &universe, &[], &[]);
        // a: outbound only -> unreachable; b: inbound only -> dead end;
        // c: isolated -> both.
        assert!(v.contains(&GraphInvariantViolation::Unreachable(a.compact_id())));
        assert!(v.contains(&GraphInvariantViolation::DeadEnd(b.compact_id())));
        assert!(v.contains(&GraphInvariantViolation::Unreachable(c.compact_id())));
        assert!(v.contains(&GraphInvariantViolation::DeadEnd(c.compact_id())));
    }

    #[test]
    fn audit_respects_entry_and_terminal_exemptions() {
        let a = PlayerState::Forward(ForwardState::Standing);
        let b = PlayerState::Forward(ForwardState::Running);
        let edges = [GraphEdge {
            from: a,
            to: b,
            source: TransitionSource::Handler,
        }];
        let universe = [a, b];
        // a documented entry (no inbound ok), b terminal (no outbound ok).
        let v = TransitionGraph::audit(&edges, &universe, &[a], &[b]);
        assert!(v.is_empty(), "exemptions should clear violations: {v:?}");
    }

    #[test]
    fn render_dot_emits_nodes_edges_and_source_colour() {
        let a = PlayerState::Forward(ForwardState::Standing);
        let b = PlayerState::Forward(ForwardState::Running);
        let edges = [
            GraphEdge {
                from: a,
                to: b,
                source: TransitionSource::Handler,
            },
            GraphEdge {
                from: a,
                to: b,
                source: TransitionSource::LooseBallOverride,
            },
        ];
        let dot = TransitionGraph::render_dot(&edges);
        assert!(dot.starts_with("digraph player_state_transitions {"));
        assert!(dot.contains(&format!("s{} ->", a.compact_id())));
        assert!(dot.contains(&format!("s{} [label=", a.compact_id())));
        assert!(dot.contains("color=black")); // Handler
        assert!(dot.contains("color=red")); // LooseBallOverride
        assert!(dot.trim_end().ends_with('}'));
        // Two edges share endpoints -> exactly two distinct node lines.
        assert_eq!(dot.matches(" [label=").count(), 2);
    }

    /// Source scanner: walks the match engine tree and finds raw `.state`
    /// assignments. Wrapped in a struct so the recursion + line predicate
    /// stay together (and out of the free-function namespace).
    struct StateAssignmentScanner;

    impl StateAssignmentScanner {
        fn scan(dir: &Path) -> Vec<(PathBuf, usize)> {
            let mut hits = Vec::new();
            Self::walk(dir, &mut hits);
            hits
        }

        fn walk(dir: &Path, hits: &mut Vec<(PathBuf, usize)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::walk(&path, hits);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                // Skip this scanner's own file — it necessarily contains
                // the search needle as a string literal.
                if path.file_name().and_then(|f| f.to_str()) == Some("transition.rs") {
                    continue;
                }
                let Ok(src) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let count = src.lines().filter(|l| Self::is_state_assignment(l)).count();
                if count > 0 {
                    hits.push((path, count));
                }
            }
        }

        /// True when the line assigns to a `.state` field (`x.state = ..`),
        /// excluding equality (`==`) and longer identifiers (`.state_time`).
        fn is_state_assignment(line: &str) -> bool {
            // Strip any line comment first, so a comment that mentions
            // `.state =` (e.g. this invariant's own doc) doesn't read as
            // code. A `//` inside a string literal would also truncate, but
            // that only ever loses a match, never invents one.
            let code = match line.find("//") {
                Some(i) => &line[..i],
                None => line,
            };
            let mut from = 0;
            while let Some(rel) = code[from..].find(".state") {
                let idx = from + rel + ".state".len();
                let after = code[idx..].chars().next();
                let boundary = !matches!(after, Some(c) if c.is_alphanumeric() || c == '_');
                let rest = code[idx..].trim_start();
                if boundary && rest.starts_with('=') && !rest.starts_with("==") {
                    return true;
                }
                from = idx;
            }
            false
        }
    }

    #[test]
    fn no_raw_player_state_assignment_outside_transition_api() {
        let match_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("match");
        let hits = StateAssignmentScanner::scan(&match_dir);

        // The ONLY approved `.state =` writes are the two transition
        // primitives: `MatchPlayer::transition_to` (player.rs) and
        // `StateProcessingResult::merge_state_change` (processor.rs).
        // Everything else must route through `transition_to`.
        let mut by_file: Vec<(String, usize)> = hits
            .iter()
            .map(|(p, c)| (p.file_name().unwrap().to_string_lossy().into_owned(), *c))
            .collect();
        by_file.sort();

        assert_eq!(
            by_file,
            vec![
                ("player.rs".to_string(), 1),
                ("processor.rs".to_string(), 1),
            ],
            "unexpected raw `.state =` assignment(s) found: {hits:?}"
        );
    }

    #[cfg(feature = "match-logs")]
    #[test]
    fn recorder_dedups_and_exports() {
        let a = PlayerState::Forward(ForwardState::Standing);
        let b = PlayerState::Forward(ForwardState::Running);
        TransitionGraph::record(a, b, TransitionSource::Handler);
        TransitionGraph::record(a, b, TransitionSource::Handler); // dup -> collapses
        TransitionGraph::record(a, b, TransitionSource::LooseBallOverride);

        let edges = TransitionGraph::edges();
        let handler_edges = edges
            .iter()
            .filter(|e| e.key() == (a.compact_id(), b.compact_id(), TransitionSource::Handler))
            .count();
        assert_eq!(handler_edges, 1, "duplicate edge must collapse to one");
        assert!(
            edges
                .iter()
                .any(|e| e.source == TransitionSource::LooseBallOverride),
            "distinct-source edge must be recorded separately"
        );
        assert!(TransitionGraph::export_dot().contains("digraph player_state_transitions"));
    }
}
