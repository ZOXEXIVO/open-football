use crate::MatchRuntime;
use crate::r#match::engine::FootballEngine;
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::{Match, MatchDispatcherRegistry, MatchResult, MatchResultRaw, SquadFixture};
use rayon::ThreadPool;
use rayon::ThreadPoolBuilder;
use rayon::iter::{ParallelBridge, ParallelIterator};
use std::cmp::Reverse;

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
        self.play_longest_first(matches, |m| m.fixture.expected_length(), Match::play)
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
        self.play_local(matches)
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
        self.play_longest_first(
            matches,
            |(_, _, _, fixture)| fixture.expected_length(),
            |(idx, home, away, fixture)| {
                let config = MatchEngineConfig::for_fixture(&fixture, recordings);
                (idx, FootballEngine::<840, 545>::play_with_config(home, away, config))
            },
        )
    }

    /// Play a batch with its longest matches started first, one match per
    /// unit of work, and hand the results back in the batch's order. A
    /// worker picks up the next match the moment it frees, so the batch
    /// ends with the short matches rather than with an extra-time tie that
    /// happened to sit last.
    fn play_longest_first<T: Send, R: Send>(
        &self,
        items: Vec<T>,
        expected_length: impl Fn(&T) -> u8,
        play: impl Fn(T) -> R + Sync + Send,
    ) -> Vec<R> {
        let mut ordered: Vec<(usize, T)> = items.into_iter().enumerate().collect();
        ordered.sort_by_key(|(_, item)| Reverse(expected_length(item)));
        let mut played: Vec<(usize, R)> = self.pool.install(|| {
            ordered
                .into_iter()
                .par_bridge()
                .map(|(index, item)| (index, play(item)))
                .collect()
        });
        played.sort_unstable_by_key(|(index, _)| *index);
        played.into_iter().map(|(_, result)| result).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::MatchPlayEnginePool;

    #[test]
    fn results_come_back_in_batch_order_whatever_order_they_were_played_in() {
        let pool = MatchPlayEnginePool::new(4);
        let lengths = [0u8, 2, 1, 2, 0, 1, 1, 2, 0, 0];
        let played = pool.play_longest_first(
            lengths.iter().copied().enumerate().collect(),
            |(_, length)| *length,
            |(index, length)| (index, length),
        );
        let expected: Vec<(usize, u8)> = lengths.iter().copied().enumerate().collect();
        assert_eq!(played, expected);
    }
}
