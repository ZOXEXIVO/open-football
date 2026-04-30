use crate::r#match::MatchField;
use crate::r#match::player_positions::{BallFieldData, PlayerFieldData};

pub struct MatchObjectsPositions {
    pub ball: BallFieldData,
    pub players: PlayerFieldData,
}

impl MatchObjectsPositions {
    pub fn from(field: &MatchField) -> Self {
        MatchObjectsPositions {
            ball: BallFieldData::from(&field.ball),
            players: PlayerFieldData::from(field),
        }
    }

    #[inline]
    pub fn update(&mut self, field: &MatchField) {
        self.ball.update_from(&field.ball);
        self.players.update(field);
    }
}
