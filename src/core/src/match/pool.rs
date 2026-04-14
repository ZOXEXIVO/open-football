use crate::r#match::{Match, MatchResult, MatchResultRaw, MatchSquad};
use crate::r#match::engine::FootballEngine;
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
        self.pool.install(|| {
            matches
                .into_par_iter()
                .map(|m| m.play())
                .collect()
        })
    }

    /// Play raw squad-vs-squad matches through the pool (for national team / international matches).
    /// Each input is (index, home_squad, away_squad). Returns (index, MatchResultRaw).
    pub fn play_squads(&self, matches: Vec<(usize, MatchSquad, MatchSquad)>) -> Vec<(usize, MatchResultRaw)> {
        self.pool.install(|| {
            matches
                .into_par_iter()
                .map(|(idx, home, away)| {
                    let result = FootballEngine::<840, 545>::play(home, away, false, false, false);
                    (idx, result)
                })
                .collect()
        })
    }
}
