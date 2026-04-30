use crate::r#match::{MatchField, MatchPlayer};
use std::cmp::Ordering;

const MAX_DISTANCE: f32 = 999.0;
const MAX_PLAYERS: usize = 32;
const SLOT_TABLE_SIZE: usize = 128;
const SLOT_EMPTY: u8 = 0xFF;
// Max entries per player in per_player flat array
const MAX_NEIGHBORS: usize = MAX_PLAYERS - 1;

#[derive(Debug, Clone)]
pub struct PlayerDistanceClosure {
    // Flat matrix: dist_matrix[slot_a * MAX_PLAYERS + slot_b] = distance
    dist_matrix: [f32; MAX_PLAYERS * MAX_PLAYERS],
    // Open-addressing hash: id_slots[hash(id)] = (player_id, slot)
    id_slots: [(u32, u8); SLOT_TABLE_SIZE],
    // Flat per-player neighbor data: fixed array avoids heap indirection
    per_player_data: [(u32, bool, f32); MAX_PLAYERS * MAX_NEIGHBORS],
    per_player_len: [u8; MAX_PLAYERS],
    num_players: usize,
}

// Keep for external use (e.g. debug tools)
#[derive(Debug, Clone)]
pub struct PlayerDistanceItem {
    pub player_from_id: u32,
    pub player_from_team: u32,
    pub player_to_id: u32,
    pub player_to_team: u32,
    pub distance: f32,
}

impl PlayerDistanceClosure {
    pub fn new() -> Self {
        PlayerDistanceClosure {
            dist_matrix: [MAX_DISTANCE; MAX_PLAYERS * MAX_PLAYERS],
            id_slots: [(0, SLOT_EMPTY); SLOT_TABLE_SIZE],
            per_player_data: [(0, false, 0.0); MAX_PLAYERS * MAX_NEIGHBORS],
            per_player_len: [0; MAX_PLAYERS],
            num_players: 0,
        }
    }

    #[inline(always)]
    fn slot_of(&self, player_id: u32) -> Option<usize> {
        let mask = (SLOT_TABLE_SIZE - 1) as u32;
        let mut idx = player_id.wrapping_mul(2654435761) & mask;
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
    fn insert_slot(&mut self, player_id: u32, slot: u8) {
        let mask = (SLOT_TABLE_SIZE - 1) as u32;
        let mut idx = player_id.wrapping_mul(2654435761) & mask;
        loop {
            let entry = &mut self.id_slots[idx as usize];
            if entry.1 == SLOT_EMPTY {
                *entry = (player_id, slot);
                return;
            }
            idx = (idx + 1) & mask;
        }
    }

    pub fn update_from_field(&mut self, field: &MatchField) {
        self.update_from_players(&field.players);
    }

    pub fn update_from_players(&mut self, players: &[MatchPlayer]) {
        let n = players.len();

        // Only rebuild hash table when player count changes (substitution)
        if n != self.num_players {
            self.num_players = n;
            self.id_slots = [(0, SLOT_EMPTY); SLOT_TABLE_SIZE];
            for (slot, p) in players.iter().enumerate() {
                self.insert_slot(p.id, slot as u8);
            }
        }

        // Reset per-player counts
        for i in 0..n {
            self.per_player_len[i] = 0;
        }

        // Compute distances — only upper triangle, mirror to lower
        for i in 0..n {
            let outer = &players[i];
            for j in (i + 1)..n {
                let inner = &players[j];
                let dx = outer.position.x - inner.position.x;
                let dy = outer.position.y - inner.position.y;
                let distance = (dx * dx + dy * dy).sqrt();

                self.dist_matrix[i * MAX_PLAYERS + j] = distance;
                self.dist_matrix[j * MAX_PLAYERS + i] = distance;

                let same_team = outer.team_id == inner.team_id;

                let count_i = self.per_player_len[i] as usize;
                self.per_player_data[i * MAX_NEIGHBORS + count_i] = (inner.id, same_team, distance);
                self.per_player_len[i] = (count_i + 1) as u8;

                let count_j = self.per_player_len[j] as usize;
                self.per_player_data[j * MAX_NEIGHBORS + count_j] = (outer.id, same_team, distance);
                self.per_player_len[j] = (count_j + 1) as u8;
            }
        }
    }
}

impl From<&MatchField> for PlayerDistanceClosure {
    fn from(field: &MatchField) -> Self {
        let mut closure = PlayerDistanceClosure::new();
        closure.update_from_field(field);
        closure
    }
}

impl PlayerDistanceClosure {
    #[inline]
    pub fn get(&self, player_from_id: u32, player_to_id: u32) -> f32 {
        if player_from_id == player_to_id {
            return 0.0;
        }

        let slot_a = match self.slot_of(player_from_id) {
            Some(s) => s,
            None => return MAX_DISTANCE,
        };
        let slot_b = match self.slot_of(player_to_id) {
            Some(s) => s,
            None => return MAX_DISTANCE,
        };

        unsafe {
            *self
                .dist_matrix
                .get_unchecked(slot_a * MAX_PLAYERS + slot_b)
        }
    }

    pub fn teammates<'t>(
        &'t self,
        player_id: u32,
        min_distance: f32,
        max_distance: f32,
    ) -> impl Iterator<Item = (u32, f32)> + 't {
        let slot = self.slot_of(player_id);
        slot.into_iter()
            .flat_map(move |s| {
                let len = self.per_player_len[s] as usize;
                let base = s * MAX_NEIGHBORS;
                self.per_player_data[base..base + len].iter()
            })
            .filter(move |(_, same_team, dist)| {
                *same_team && *dist >= min_distance && *dist <= max_distance
            })
            .map(|(id, _, dist)| (*id, *dist))
    }

    pub fn opponents<'t>(
        &'t self,
        player_id: u32,
        distance: f32,
    ) -> impl Iterator<Item = (u32, f32)> + 't {
        let slot = self.slot_of(player_id);
        slot.into_iter()
            .flat_map(move |s| {
                let len = self.per_player_len[s] as usize;
                let base = s * MAX_NEIGHBORS;
                self.per_player_data[base..base + len].iter()
            })
            .filter(move |(_, same_team, dist)| !*same_team && *dist <= distance)
            .map(|(id, _, dist)| (*id, *dist))
    }
}

impl Eq for PlayerDistanceItem {}

impl PartialEq<PlayerDistanceItem> for PlayerDistanceItem {
    fn eq(&self, other: &Self) -> bool {
        self.player_from_id == other.player_from_id
            && self.player_from_team == other.player_from_team
            && self.player_to_id == other.player_to_id
            && self.player_to_team == other.player_to_team
            && self.distance == other.distance
    }
}

impl PartialOrd<Self> for PlayerDistanceItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PlayerDistanceItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance
            .partial_cmp(&other.distance)
            .unwrap_or(Ordering::Equal)
    }
}
