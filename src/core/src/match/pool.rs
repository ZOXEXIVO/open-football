use crate::MatchRuntime;
use crate::r#match::engine::FootballEngine;
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::{Match, MatchDispatcherRegistry, MatchResult, MatchResultRaw, SquadFixture};
use rayon::ThreadPool;
use rayon::ThreadPoolBuilder;
use rayon::iter::{IntoParallelIterator, ParallelIterator};

pub struct MatchPlayEnginePool {
    pool: ThreadPool,
    num_threads: usize,
}

impl MatchPlayEnginePool {
    pub fn new(num_threads: usize) -> Self {
        let pool = ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .thread_name(|idx| format!("match-worker-{}", idx))
            .build()
            .expect("failed to create match engine thread pool");

        MatchPlayEnginePool { pool, num_threads }
    }

    /// Worker-thread count this pool was built with. Reported by the
    /// distributed worker handshake so the coordinator can weight the
    /// per-worker share by CPU.
    pub fn num_threads(&self) -> usize {
        self.num_threads
    }

    /// Run a batch of league matches strictly on the local rayon pool,
    /// bypassing any installed `MatchDispatcher`. Used by the
    /// distributed dispatcher itself for its "local share" of a batch
    /// so the coordinator's own CPU isn't idle while remote workers
    /// process the rest. Functionally identical to the local branch of
    /// `play()`; existing call sites should keep calling `play()`.
    pub fn play_local(&self, matches: Vec<Match>) -> Vec<MatchResult> {
        self.pool
            .install(|| matches.into_par_iter().map(|m| m.play()).collect())
    }

    /// Squad-only counterpart to [`play_local`]. Same semantics —
    /// bypasses the dispatcher; runs on the local rayon pool only.
    pub fn play_squads_local(&self, matches: Vec<SquadFixture>) -> Vec<(usize, MatchResultRaw)> {
        self.play_squads_here(matches)
    }

    /// Play league/cup matches through the pool (produces MatchResult with league metadata).
    ///
    /// When a [`MatchDispatcher`](crate::r#match::MatchDispatcher) is
    /// installed via `MatchDispatcherRegistry::set`, the pool first
    /// offers the work to the dispatcher. On `Ok` the dispatcher fully
    /// claims the batch (no local execution); on `Err` it hands the
    /// input back and the pool runs the local rayon path.
    pub fn play(&self, matches: Vec<Match>) -> Vec<MatchResult> {
        let matches = match MatchDispatcherRegistry::try_get() {
            Some(dispatcher) => match dispatcher.dispatch_league(matches) {
                Ok(results) => return results,
                Err(returned) => returned,
            },
            None => matches,
        };
        self.pool
            .install(|| matches.into_par_iter().map(|m| m.play()).collect())
    }

    /// Play raw squad-vs-squad matches — the national-team fixtures, the
    /// one match kind that never becomes a `Match` — each as its fixture
    /// says: format, weather, crowd and referee.
    pub fn play_squads(&self, matches: Vec<SquadFixture>) -> Vec<(usize, MatchResultRaw)> {
        let matches = match MatchDispatcherRegistry::try_get() {
            Some(dispatcher) => match dispatcher.dispatch_squads(matches) {
                Ok(results) => return results,
                Err(returned) => returned,
            },
            None => matches,
        };
        self.play_squads_here(matches)
    }

    /// The local rayon run of a squad batch. Recordings follow the same
    /// process-global flag `Match::play` reads: a cap is not a lesser
    /// fixture than a league game.
    fn play_squads_here(&self, matches: Vec<SquadFixture>) -> Vec<(usize, MatchResultRaw)> {
        let recordings = MatchRuntime::recordings_mode();
        self.pool.install(|| {
            matches
                .into_par_iter()
                .map(|(idx, home, away, fixture)| {
                    let config = MatchEngineConfig::for_fixture(&fixture, recordings);
                    (idx, FootballEngine::<840, 545>::play_with_config(home, away, config))
                })
                .collect()
        })
    }
}
