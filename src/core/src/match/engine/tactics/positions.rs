use crate::PlayerPositionType;

#[derive(Debug, Clone)]
pub struct MatchTacticalPosition {
    pub position: PlayerPositionType,
    pub waypoints: Vec<(f32, f32)>,
}

#[derive(Debug, Clone)]
pub struct TacticalPositions {
    pub current_position: PlayerPositionType,
    pub tactical_positions: Vec<MatchTacticalPosition>
}

impl TacticalPositions {
    pub fn new(current_position: PlayerPositionType) -> Self {
        TacticalPositions {
            current_position,
            tactical_positions: Vec::new()
        }
    }
}
