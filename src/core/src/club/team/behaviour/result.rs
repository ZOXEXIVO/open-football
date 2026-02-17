use crate::{ChangeType, SimulatorData};

pub struct TeamBehaviourResult {
    pub players: PlayerBehaviourResult,
}

impl Default for TeamBehaviourResult {
    fn default() -> Self {
        Self::new()
    }
}

impl TeamBehaviourResult {
    pub fn new() -> Self {
        TeamBehaviourResult {
            players: PlayerBehaviourResult::new(),
        }
    }

    pub fn process(&self, data: &mut SimulatorData) {
        self.players.process(data);
    }
}

pub struct PlayerBehaviourResult {
    pub relationship_result: Vec<PlayerRelationshipChangeResult>,
}

impl Default for PlayerBehaviourResult {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerBehaviourResult {
    pub fn new() -> Self {
        PlayerBehaviourResult {
            relationship_result: Vec::new(),
        }
    }

    pub fn process(&self, data: &mut SimulatorData) {
        let sim_date = data.date.date();

        for relationship_result in &self.relationship_result {
            if let Some(player_to_modify) = data.player_mut(relationship_result.from_player_id) {
                player_to_modify.relations.update_with_type(
                    relationship_result.to_player_id,
                    relationship_result.relationship_change,
                    relationship_result.change_type.clone(),
                    sim_date,
                );
            }
        }
    }
}

pub struct PlayerRelationshipChangeResult {
    pub from_player_id: u32,
    pub to_player_id: u32,
    pub relationship_change: f32,
    pub change_type: ChangeType,
}
