use crate::r#match::player_positions::{BallFieldData, PlayerFieldData};
use crate::r#match::{MatchField};

pub struct MatchObjectsPositions {
    pub ball: BallFieldData,
    pub players: PlayerFieldData,
}

impl MatchObjectsPositions {
    pub fn from(field: &MatchField) -> Self {
        MatchObjectsPositions {
            ball: BallFieldData::from(&field.ball),
            players: PlayerFieldData::from(field)
        }
    }

    pub fn update(&mut self, field: &MatchField) {
        self.ball = BallFieldData::from(&field.ball);
        self.players.update(field);
    }
}
