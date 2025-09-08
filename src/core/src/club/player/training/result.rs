use crate::{SimulatorData, TrainingEffects};

pub struct PlayerTrainingResult {
    pub player_id: u32,
    pub effects: TrainingEffects,
}

impl PlayerTrainingResult {
    pub fn new(player_id: u32, effects: TrainingEffects) -> Self {
        PlayerTrainingResult {
            player_id,
            effects,
        }
    }

    pub fn process(&self, data: &mut SimulatorData) {}
}
