use crate::r#match::{MatchField, PlayerSide};
use nalgebra::Vector3;

#[derive(Debug)]
pub struct PlayerFieldData {
    pub items: Vec<PlayerFieldMetadata>,
}

#[derive(Debug)]
pub struct PlayerFieldMetadata {
    pub player_id: u32,
    pub side: PlayerSide,
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
}

impl PlayerFieldData {
    pub fn position(&self, player_id: u32) -> Vector3<f32> {
        self
            .items
            .iter()
            .find(|p| p.player_id == player_id)
            .map(|p| p.position)
            .unwrap_or_else(|| Vector3::new(-1000.0, -1000.0, 0.0))
    }

    pub fn has_player(&self, player_id: u32) -> bool {
        self.items.iter().any(|p| p.player_id == player_id)
    }

    pub fn velocity(&self, player_id: u32) -> Vector3<f32> {
        self.items
            .iter()
            .find(|p| p.player_id == player_id)
            .map(|p| p.velocity)
            .unwrap_or_else(|| Vector3::zeros())
    }
}

impl From<&MatchField> for PlayerFieldData {
    #[inline]
    fn from(field: &MatchField) -> Self {
        PlayerFieldData {
            items: field
                .players
                .iter()
                .chain(field.substitutes.iter())
                .map(|p| PlayerFieldMetadata {
                    player_id: p.id,
                    side: p
                        .side
                        .unwrap_or_else(|| panic!("unknown player side, player_id = {}", p.id)),
                    position: p.position,
                    velocity: p.velocity,
                })
                .collect(),
        }
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum PlayerDistanceFromStartPosition {
    Small,
    Medium,
    Big,
}
