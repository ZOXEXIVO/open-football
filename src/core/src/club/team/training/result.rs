use crate::club::player::training::result::PlayerTrainingResult;
use crate::SimulatorData;

pub struct TeamTrainingResult {
    pub player_results: Vec<PlayerTrainingResult>,
}

impl TeamTrainingResult {
    pub fn new() -> Self {
        TeamTrainingResult {
            player_results: Vec::new(),
        }
    }

    pub fn empty() -> Self {
        TeamTrainingResult {
            player_results: Vec::new(),
        }
    }

    pub fn process(&self, data: &mut SimulatorData) {
        for player_result in &self.player_results {
            player_result.process(data);
        }
    }
}
