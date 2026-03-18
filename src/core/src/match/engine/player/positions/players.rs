use crate::r#match::{MatchField, PlayerSide};
use nalgebra::Vector3;

const MAX_PLAYERS: usize = 48; // players + substitutes

#[derive(Debug, Clone)]
pub struct PlayerFieldData {
    pub items: Vec<PlayerFieldMetadata>,
}

#[derive(Debug, Clone)]
pub struct PlayerFieldMetadata {
    pub player_id: u32,
    pub side: PlayerSide,
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
}

impl PlayerFieldData {
    #[inline]
    pub fn position(&self, player_id: u32) -> Vector3<f32> {
        for item in &self.items {
            if item.player_id == player_id {
                return item.position;
            }
        }
        Vector3::new(-1000.0, -1000.0, 0.0)
    }

    #[inline]
    pub fn has_player(&self, player_id: u32) -> bool {
        self.items.iter().any(|p| p.player_id == player_id)
    }

    #[inline]
    pub fn velocity(&self, player_id: u32) -> Vector3<f32> {
        for item in &self.items {
            if item.player_id == player_id {
                return item.velocity;
            }
        }
        Vector3::zeros()
    }
}

impl PlayerFieldData {
    pub fn update(&mut self, field: &MatchField) {
        self.items.clear();
        for p in field.players.iter().chain(field.substitutes.iter()) {
            self.items.push(PlayerFieldMetadata {
                player_id: p.id,
                side: p.side.unwrap_or_else(|| panic!("unknown player side, player_id = {}", p.id)),
                position: p.position,
                velocity: p.velocity,
            });
        }
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
