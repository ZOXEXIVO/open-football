use crate::r#match::engine::FootballEngine;
use crate::r#match::{Match, MatchResult, MatchResultRaw, MatchSquad};
use rayon::iter::{IntoParallelIterator, ParallelIterator};

pub struct MatchPlayEnginePool {
    pool: rayon::ThreadPool,
}

impl MatchPlayEnginePool {
    pub fn new(num_threads: usize) -> Self {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .thread_name(|idx| format!("match-worker-{}", idx))
            .build()
            .expect("failed to create match engine thread pool");

        MatchPlayEnginePool { pool }
    }

    /// Play league/cup matches through the pool (produces MatchResult with league metadata)
    pub fn play(&self, matches: Vec<Match>) -> Vec<MatchResult> {
        self.pool
            .install(|| matches.into_par_iter().map(|m| m.play()).collect())
    }

    /// Play raw squad-vs-squad matches through the pool (for national team / international matches).
    /// Each input is (index, home_squad, away_squad). Returns (index, MatchResultRaw).
    /// Convenience wrapper that defaults `is_knockout = false`.
    pub fn play_squads(
        &self,
        matches: Vec<(usize, MatchSquad, MatchSquad)>,
    ) -> Vec<(usize, MatchResultRaw)> {
        let with_flag: Vec<(usize, MatchSquad, MatchSquad, bool)> = matches
            .into_iter()
            .map(|(i, h, a)| (i, h, a, false))
            .collect();
        self.play_squads_with_knockout(with_flag)
    }

    /// Play raw squad-vs-squad matches with explicit knockout flagging.
    /// Knockout fixtures route through the engine's full penalty
    /// shootout when the score is level after extra time — callers can
    /// then read the winner straight from `Score::outcome()` instead of
    /// guessing based on reputation.
    pub fn play_squads_with_knockout(
        &self,
        matches: Vec<(usize, MatchSquad, MatchSquad, bool)>,
    ) -> Vec<(usize, MatchResultRaw)> {
        self.pool.install(|| {
            matches
                .into_par_iter()
                .map(|(idx, home, away, is_knockout)| {
                    let result =
                        FootballEngine::<840, 545>::play(home, away, false, false, is_knockout);
                    (idx, result)
                })
                .collect()
        })
    }
}
