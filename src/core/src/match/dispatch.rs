use crate::r#match::{Match, MatchResult, MatchResultRaw, MatchSquad};
use std::sync::OnceLock;

/// Pluggable executor for match work. When installed via
/// [`MatchDispatcherRegistry::set`], `MatchPlayEnginePool` consults the
/// dispatcher first and only falls back to the local rayon thread-pool
/// when the dispatcher declines.
///
/// Distributed worker support (see `web::worker`) installs a
/// `DistributedDispatcher` here at startup, but the trait is plain
/// enough that tests / alternative back-ends can plug in too.
///
/// Ownership: the dispatcher takes the input by value. On `Ok` it
/// commits to producing all results (in the same order as the input)
/// — local failures are the dispatcher's job to backfill. On `Err` it
/// hands the input back unchanged so the pool can run the local rayon
/// path without re-allocating.
pub trait MatchDispatcher: Send + Sync {
    fn dispatch_league(&self, matches: Vec<Match>) -> Result<Vec<MatchResult>, Vec<Match>>;
    fn dispatch_squads(
        &self,
        matches: Vec<(usize, MatchSquad, MatchSquad, bool)>,
    ) -> Result<Vec<(usize, MatchResultRaw)>, Vec<(usize, MatchSquad, MatchSquad, bool)>>;
}

/// Process-wide handle to the active [`MatchDispatcher`]. The binary
/// wires a dispatcher at startup without `core` having to know what
/// `web` does.
pub struct MatchDispatcherRegistry;

static DISPATCHER: OnceLock<Box<dyn MatchDispatcher>> = OnceLock::new();

impl MatchDispatcherRegistry {
    /// Install the process-wide dispatcher. First call wins — subsequent
    /// calls are silently ignored so a duplicated startup path can't
    /// race-replace an already-published dispatcher.
    pub fn set(dispatcher: Box<dyn MatchDispatcher>) {
        let _ = DISPATCHER.set(dispatcher);
    }

    /// Borrow the active dispatcher, if any.
    pub fn try_get() -> Option<&'static dyn MatchDispatcher> {
        DISPATCHER.get().map(|b| b.as_ref())
    }
}
