use crate::r#match::{MatchField, PlayerSide};
use nalgebra::Vector3;

const MAX_FIELD_PLAYERS: usize = 48; // players + substitutes
const SLOT_TABLE_SIZE: usize = 64;
const SLOT_EMPTY: u8 = 0xFF;

#[derive(Debug, Clone)]
pub struct PlayerFieldData {
    items: [PlayerFieldMetadata; MAX_FIELD_PLAYERS],
    len: usize,
    // Open-addressing hash: id_slots[hash(id)] = (player_id, index into items)
    id_slots: [(u32, u8); SLOT_TABLE_SIZE],
}

#[derive(Debug, Clone, Copy)]
pub struct PlayerFieldMetadata {
    pub player_id: u32,
    pub side: PlayerSide,
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
}

impl Default for PlayerFieldMetadata {
    #[inline]
    fn default() -> Self {
        PlayerFieldMetadata {
            player_id: 0,
            side: PlayerSide::Left,
            position: Vector3::zeros(),
            velocity: Vector3::zeros(),
        }
    }
}

impl PlayerFieldData {
    #[inline(always)]
    fn hash_slot(player_id: u32) -> u32 {
        player_id.wrapping_mul(2654435761) & (SLOT_TABLE_SIZE as u32 - 1)
    }

    #[inline]
    fn lookup_index(&self, player_id: u32) -> Option<usize> {
        let mask = (SLOT_TABLE_SIZE - 1) as u32;
        let mut idx = Self::hash_slot(player_id);
        for _ in 0..8 {
            let entry = unsafe { self.id_slots.get_unchecked(idx as usize) };
            if entry.1 == SLOT_EMPTY {
                return None;
            }
            if entry.0 == player_id {
                return Some(entry.1 as usize);
            }
            idx = (idx + 1) & mask;
        }
        None
    }

    #[inline]
    fn insert_slot(&mut self, player_id: u32, index: u8) {
        let mask = (SLOT_TABLE_SIZE - 1) as u32;
        let mut idx = Self::hash_slot(player_id);
        loop {
            let entry = &mut self.id_slots[idx as usize];
            if entry.1 == SLOT_EMPTY {
                *entry = (player_id, index);
                return;
            }
            idx = (idx + 1) & mask;
        }
    }

    #[inline]
    pub fn position(&self, player_id: u32) -> Vector3<f32> {
        if let Some(idx) = self.lookup_index(player_id) {
            unsafe { self.items.get_unchecked(idx) }.position
        } else {
            Vector3::new(-1000.0, -1000.0, 0.0)
        }
    }

    #[inline]
    pub fn has_player(&self, player_id: u32) -> bool {
        self.lookup_index(player_id).is_some()
    }

    #[inline]
    pub fn velocity(&self, player_id: u32) -> Vector3<f32> {
        if let Some(idx) = self.lookup_index(player_id) {
            unsafe { self.items.get_unchecked(idx) }.velocity
        } else {
            Vector3::zeros()
        }
    }

    /// Slice of active player metadata
    #[inline]
    pub fn as_slice(&self) -> &[PlayerFieldMetadata] {
        &self.items[..self.len]
    }
}

impl PlayerFieldData {
    pub fn update(&mut self, field: &MatchField) {
        let new_count = field.players.len() + field.substitutes.len();

        // Full rebuild only when player count changes (substitution)
        if new_count != self.len {
            self.len = 0;
            self.id_slots = [(0, SLOT_EMPTY); SLOT_TABLE_SIZE];

            for p in field.players.iter().chain(field.substitutes.iter()) {
                let idx = self.len;
                self.items[idx] = PlayerFieldMetadata {
                    player_id: p.id,
                    side: p
                        .side
                        .unwrap_or_else(|| panic!("unknown player side, player_id = {}", p.id)),
                    position: p.position,
                    velocity: p.velocity,
                };
                self.insert_slot(p.id, idx as u8);
                self.len += 1;
            }
        } else {
            // Fast path: only update positions and velocities in-place
            for (i, p) in field
                .players
                .iter()
                .chain(field.substitutes.iter())
                .enumerate()
            {
                self.items[i].position = p.position;
                self.items[i].velocity = p.velocity;
            }
        }
    }
}

impl From<&MatchField> for PlayerFieldData {
    #[inline]
    fn from(field: &MatchField) -> Self {
        let mut data = PlayerFieldData {
            items: [PlayerFieldMetadata::default(); MAX_FIELD_PLAYERS],
            len: 0,
            id_slots: [(0, SLOT_EMPTY); SLOT_TABLE_SIZE],
        };
        data.update(field);
        data
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum PlayerDistanceFromStartPosition {
    Small,
    Medium,
    Big,
}
