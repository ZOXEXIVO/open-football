//! Standalone match types produced/consumed by the engine: events, ball
//! side, team-tactics snapshot, field size, player collection, the match
//! clock, and the per-state result.

use crate::r#match::field::MatchField;
use crate::r#match::{MatchPlayer, MatchSquad};
use crate::{PlayerPositionType, Tactics};

pub enum MatchEvent {
    MatchPlayed(u32, bool, u8),
    Goal(u32),
    Assist(u32),
    Injury(u32),
}

// ───────────────────────────────────────────────────────────────────────────────
// Types: BallSide, TeamsTactics, MatchFieldSize
// ───────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BallSide {
    Left,
    Right,
}

impl From<BallSide> for u8 {
    fn from(side: BallSide) -> Self {
        match side {
            BallSide::Left => 0,
            BallSide::Right => 1,
        }
    }
}

#[derive(Clone)]
pub struct TeamsTactics {
    pub left: Tactics,
    pub right: Tactics,
}

impl TeamsTactics {
    pub fn from_field(field: &MatchField) -> Self {
        TeamsTactics {
            left: field.left_team_tactics.clone(),
            right: field.right_team_tactics.clone(),
        }
    }
}

#[derive(Clone)]
pub struct MatchFieldSize {
    pub width: usize,
    pub height: usize,

    pub half_width: usize,
}

impl MatchFieldSize {
    pub fn new(width: usize, height: usize) -> Self {
        MatchFieldSize {
            width,
            height,
            half_width: width / 2,
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// Types: PlayerEntry, MatchPlayerCollection
// ───────────────────────────────────────────────────────────────────────────────

/// Compact player entry for fast iteration in hot loops
#[derive(Clone, Copy)]
pub struct PlayerEntry {
    pub id: u32,
    pub team_id: u32,
    pub position: PlayerPositionType,
}

const PLAYER_SLOT_TABLE_SIZE: usize = 128;
/// Sentinel `index` meaning "empty slot" in `id_slots`.
const PLAYER_SLOT_EMPTY: u32 = u32::MAX;

/// Player lookup store for the match. Holds every player (both teams'
/// starters + substitutes) in a contiguous `Vec` and resolves `by_id`
/// through a small open-addressing `id → index` table — the same pattern
/// the `SpatialGrid` / `PlayerFieldData` use. Replaces a `HashMap<u32,
/// MatchPlayer>`: keeping the large `MatchPlayer` values OUT of the probe
/// table makes lookups cache-friendly (the table is ~1 KB and stays hot
/// instead of the keys being scattered across big inline buckets), the
/// multiply-hash never collides for distinct ids, and `raw_players` now
/// iterates in deterministic insertion order (one fewer source of
/// run-to-run non-determinism).
pub struct MatchPlayerCollection {
    players: Vec<MatchPlayer>,
    id_slots: [(u32, u32); PLAYER_SLOT_TABLE_SIZE],
    /// Compact index for fast cache-friendly iteration
    pub entries: Vec<PlayerEntry>,
}

impl MatchPlayerCollection {
    #[inline]
    fn slot_of(id: u32) -> usize {
        (id.wrapping_mul(2654435761) as usize) & (PLAYER_SLOT_TABLE_SIZE - 1)
    }

    fn lookup_index(&self, id: u32) -> Option<usize> {
        let mut idx = Self::slot_of(id);
        for _ in 0..PLAYER_SLOT_TABLE_SIZE {
            let (slot_id, slot_index) = self.id_slots[idx];
            if slot_index == PLAYER_SLOT_EMPTY {
                return None;
            }
            if slot_id == id {
                return Some(slot_index as usize);
            }
            idx = (idx + 1) & (PLAYER_SLOT_TABLE_SIZE - 1);
        }
        None
    }

    /// Insert one `id → index` mapping into the open-addressing table.
    fn insert_slot(&mut self, id: u32, index: u32) {
        let mut idx = Self::slot_of(id);
        loop {
            if self.id_slots[idx].1 == PLAYER_SLOT_EMPTY {
                self.id_slots[idx] = (id, index);
                return;
            }
            idx = (idx + 1) & (PLAYER_SLOT_TABLE_SIZE - 1);
        }
    }

    /// Rebuild the table from the current `players` order. Cheap (≤128
    /// inserts) and only runs at construction or on the rare roster
    /// mutation (substitution) that shifts indices.
    fn rebuild_slots(&mut self) {
        self.id_slots = [(0, PLAYER_SLOT_EMPTY); PLAYER_SLOT_TABLE_SIZE];
        for i in 0..self.players.len() {
            let id = self.players[i].id;
            self.insert_slot(id, i as u32);
        }
    }

    pub fn from_squads(home_squad: &MatchSquad, away_squad: &MatchSquad) -> Self {
        let mut players = Vec::with_capacity(48);
        let mut entries = Vec::with_capacity(44);

        // Starters of both teams carry a PlayerEntry (the compact iteration
        // index used by teammates()/opponents()); substitutes are
        // lookup-only until they come on (added to `entries` via
        // `update_player` at the swap).
        for p in &home_squad.main_squad {
            entries.push(PlayerEntry {
                id: p.id,
                team_id: p.team_id,
                position: p.tactical_position.current_position,
            });
            players.push(p.clone());
        }
        for p in &away_squad.main_squad {
            entries.push(PlayerEntry {
                id: p.id,
                team_id: p.team_id,
                position: p.tactical_position.current_position,
            });
            players.push(p.clone());
        }
        for p in &home_squad.substitutes {
            players.push(p.clone());
        }
        for p in &away_squad.substitutes {
            players.push(p.clone());
        }

        let mut collection = MatchPlayerCollection {
            players,
            id_slots: [(0, PLAYER_SLOT_EMPTY); PLAYER_SLOT_TABLE_SIZE],
            entries,
        };
        collection.rebuild_slots();
        collection
    }

    pub fn by_id(&self, player_id: u32) -> Option<&MatchPlayer> {
        self.lookup_index(player_id).map(|i| &self.players[i])
    }

    pub fn raw_players(&self) -> impl Iterator<Item = &MatchPlayer> {
        self.players.iter()
    }

    pub fn remove_player(&mut self, player_id: u32) {
        if let Some(i) = self.lookup_index(player_id) {
            // swap_remove is O(1) but moves the last player into slot `i`,
            // so the index table must be rebuilt afterwards.
            self.players.swap_remove(i);
            self.rebuild_slots();
        }
        self.entries.retain(|e| e.id != player_id);
    }

    pub fn update_player(&mut self, player_id: u32, player: MatchPlayer) {
        let pos = player.tactical_position.current_position;
        let team_id = player.team_id;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == player_id) {
            entry.position = pos;
            entry.team_id = team_id;
        } else {
            self.entries.push(PlayerEntry {
                id: player_id,
                team_id,
                position: pos,
            });
        }
        if let Some(i) = self.lookup_index(player_id) {
            // In-place replace — index unchanged, table stays valid.
            self.players[i] = player;
        } else {
            let index = self.players.len() as u32;
            self.players.push(player);
            self.insert_slot(player_id, index);
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// Types: MatchTime, PlayMatchStateResult
// ───────────────────────────────────────────────────────────────────────────────

#[cfg(debug_assertions)]
pub const MATCH_HALF_TIME_MS: u64 = 5 * 60 * 1000;
#[cfg(not(debug_assertions))]
pub const MATCH_HALF_TIME_MS: u64 = 45 * 60 * 1000;

pub const MATCH_TIME_MS: u64 = MATCH_HALF_TIME_MS * 2;

/// Extra time is a single continuous 30-minute period in this simulation.
/// Real football splits it into 2×15 with an interval; we skip the break
/// since there's no tactical depth to add between the two halves here.
#[cfg(debug_assertions)]
pub const MATCH_EXTRA_TIME_MS: u64 = 3 * 60 * 1000;
#[cfg(not(debug_assertions))]
pub const MATCH_EXTRA_TIME_MS: u64 = 30 * 60 * 1000;

pub struct MatchTime {
    pub time: u64,
}

impl MatchTime {
    pub fn new() -> Self {
        MatchTime { time: 0 }
    }

    #[inline]
    pub fn increment(&mut self, val: u64) -> u64 {
        self.time += val;
        self.time
    }

    pub fn is_running_out(&self) -> bool {
        self.time > (2 * MATCH_TIME_MS / 3)
    }
}

#[derive(Default, Clone)]
pub struct PlayMatchStateResult {
    pub additional_time: u64,
}
