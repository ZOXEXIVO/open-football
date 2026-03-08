use crate::r#match::{Match, MatchResult};
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

    pub fn play(&self, matches: Vec<Match>) -> Vec<MatchResult> {
        self.pool.install(|| {
            matches
                .into_par_iter()
                .map(|m| m.play())
                .collect()
        })
    }
}
