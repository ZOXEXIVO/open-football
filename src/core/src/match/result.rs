use nalgebra::Vector3;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct ResultPositionDataItem {
    pub timestamp: u64,
    pub position: Vector3<f32>,
}

impl ResultPositionDataItem {
    pub fn new(timestamp: u64, position: Vector3<f32>) -> Self {
        ResultPositionDataItem {
            timestamp,
            position,
        }
    }
}

impl PartialEq<ResultPositionDataItem> for ResultPositionDataItem {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp && self.position == other.position
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ResultMatchPositionData {
    ball: Vec<ResultPositionDataItem>,
    players: HashMap<u32, Vec<ResultPositionDataItem>>,
}

impl ResultMatchPositionData {
    pub fn new() -> Self {
        ResultMatchPositionData {
            ball: Vec::new(),
            players: HashMap::with_capacity(22 * 2 * 9000),
        }
    }

    pub fn compress(&mut self) {}

    pub fn add_player_positions(&mut self, player_id: u32, timestamp: u64, position: Vector3<f32>) {
        if let Some(player_data) = self.players.get_mut(&player_id) {
            let last_data = player_data.last().unwrap();
            if last_data.position.x != position.x
                || last_data.position.y != position.y
                || last_data.position.z != position.z
            {
                let position_data = ResultPositionDataItem::new(timestamp, position);
                player_data.push(position_data);
            }
        } else {
            self.players
                .insert(player_id, vec![ResultPositionDataItem::new(timestamp, position)]);
        }
    }

    pub fn add_ball_positions(&mut self, timestamp: u64, position: Vector3<f32>) {
        let position = ResultPositionDataItem::new(timestamp, position);

        if let Some(last_position) = self.ball.last() {
            if last_position != &position {
                self.ball.push(position);
            }
        } else {
            self.ball.push(position);
        }
    }
}

pub trait VectorExtensions {
    fn length(&self) -> f32;
    fn distance_to(&self, other: &Vector3<f32>) -> f32;
}

impl VectorExtensions for Vector3<f32> {
    #[inline]
    fn length(&self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    #[inline]
    fn distance_to(&self, other: &Vector3<f32>) -> f32 {
        let diff = self - other;
        diff.dot(&diff).sqrt()
    }
}
