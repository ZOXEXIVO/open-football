//! Standalone match types produced/consumed by the engine: events, ball
//! side, team-tactics snapshot, field size, player collection, the match
//! clock, and the per-state result.

use crate::r#match::field::MatchField;
use crate::r#match::{MatchPlayer, MatchSquad};
use crate::{PlayerPositionType, Tactics};
use std::collections::HashMap;

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

pub struct MatchPlayerCollection {
    players: HashMap<u32, MatchPlayer>,
    /// Compact index for fast cache-friendly iteration
    pub entries: Vec<PlayerEntry>,
}

impl MatchPlayerCollection {
    pub fn from_squads(home_squad: &MatchSquad, away_squad: &MatchSquad) -> Self {
        let mut players = HashMap::new();
        let mut entries = Vec::with_capacity(44);

        let add = |p: &MatchPlayer,
                   map: &mut HashMap<u32, MatchPlayer>,
                   entries: &mut Vec<PlayerEntry>| {
            entries.push(PlayerEntry {
                id: p.id,
                team_id: p.team_id,
                position: p.tactical_position.current_position,
            });
            map.insert(p.id, p.clone());
        };

        for p in &home_squad.main_squad {
            add(p, &mut players, &mut entries);
        }
        for p in &away_squad.main_squad {
            add(p, &mut players, &mut entries);
        }

        let add_lookup_only = |p: &MatchPlayer, map: &mut HashMap<u32, MatchPlayer>| {
            map.insert(p.id, p.clone());
        };
        for p in &home_squad.substitutes {
            add_lookup_only(p, &mut players);
        }
        for p in &away_squad.substitutes {
            add_lookup_only(p, &mut players);
        }

        MatchPlayerCollection { players, entries }
    }

    pub fn by_id(&self, player_id: u32) -> Option<&MatchPlayer> {
        self.players.get(&player_id)
    }

    pub fn raw_players(&self) -> impl Iterator<Item = &MatchPlayer> {
        self.players.values()
    }

    pub fn remove_player(&mut self, player_id: u32) {
        self.players.remove(&player_id);
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
        self.players.insert(player_id, player);
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
