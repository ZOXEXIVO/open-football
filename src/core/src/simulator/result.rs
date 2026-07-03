use crate::r#match::MatchResult;

pub struct SimulationResult {
    pub match_results: Vec<MatchResult>,
    /// Number of continents whose `simulate` call panicked during this
    /// tick. Surfaces silent failures the orchestrator catches and
    /// substitutes empty results for. Sum across ticks via the
    /// process-global `ContinentPanicMetrics::total()`.
    pub panicked_continents: u32,
}

impl Default for SimulationResult {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulationResult {
    pub fn new() -> Self {
        SimulationResult {
            match_results: Vec::new(),
            panicked_continents: 0,
        }
    }

    pub fn has_match_results(&self) -> bool {
        !self.match_results.is_empty()
    }
}
