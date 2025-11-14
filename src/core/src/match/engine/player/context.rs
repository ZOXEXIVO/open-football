use crate::r#match::{
    MatchField, MatchObjectsPositions, PlayerDistanceClosure, Space
};

pub struct GameTickContext {
    pub positions: MatchObjectsPositions,
    pub distances: PlayerDistanceClosure,
    pub ball: BallMetadata,
    pub space: Space,
}

impl GameTickContext {
    pub fn new(field: &MatchField) -> Self {
        GameTickContext {
            ball: BallMetadata::from(field),
            positions: MatchObjectsPositions::from(field),
            distances: PlayerDistanceClosure::from(field),
            space: Space::from(field),
        }
    }
}

pub struct BallMetadata {
    pub is_owned: bool,
    pub is_in_flight_state: usize,

    pub current_owner: Option<u32>,
    pub last_owner: Option<u32>,
    pub notified_players: Vec<u32>,

    pub ownership_duration: u32,
}

impl From<&MatchField> for BallMetadata {
    fn from(field: &MatchField) -> Self {
        BallMetadata {
            is_owned: field.ball.current_owner.is_some(),
            is_in_flight_state: field.ball.flags.in_flight_state,
            current_owner: field.ball.current_owner,
            last_owner: field.ball.previous_owner,
            notified_players: field.ball.take_ball_notified_players.clone(),
            ownership_duration: field.ball.ownership_duration,
        }
    }
}

