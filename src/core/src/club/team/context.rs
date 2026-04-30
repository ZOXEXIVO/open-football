use crate::club::player::position::PlayerPositionType;

#[derive(Clone)]
pub struct TeamContext {
    pub id: u32,
    pub reputation: f32,
    /// Snapshot of the team's current formation so per-player processing
    /// (role fit, position coverage checks) can reason about its fit
    /// without reaching back into the team object.
    pub formation: Option<[PlayerPositionType; 11]>,
}

impl TeamContext {
    pub fn new(id: u32) -> Self {
        TeamContext {
            id,
            reputation: 0.0,
            formation: None,
        }
    }

    pub fn with_reputation(id: u32, reputation: f32) -> Self {
        TeamContext {
            id,
            reputation,
            formation: None,
        }
    }

    pub fn with_formation(mut self, formation: [PlayerPositionType; 11]) -> Self {
        self.formation = Some(formation);
        self
    }
}
