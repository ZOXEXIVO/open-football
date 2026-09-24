use crate::PlayerFieldPositionGroup;
use crate::PlayerSkills;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchField, MatchPlayer, PlayerSide};
use nalgebra::Vector3;

const MAX_FIELD_PLAYERS: usize = 48; // players + substitutes
const SLOT_TABLE_SIZE: usize = 64;
const SLOT_EMPTY: u8 = 0xFF;

#[derive(Debug, Clone)]
pub struct PlayerFieldData {
    items: [PlayerFieldMetadata; MAX_FIELD_PLAYERS],
    len: usize,
    /// `items[..on_pitch]` are the men on the pitch, the rest the bench —
    /// the store chains `field.players` before `field.substitutes`.
    on_pitch: usize,
    // Open-addressing hash: id_slots[hash(id)] = (player_id, index into items)
    id_slots: [(u32, u8); SLOT_TABLE_SIZE],
}

#[derive(Debug, Clone, Copy)]
pub struct PlayerFieldMetadata {
    pub player_id: u32,
    pub side: PlayerSide,
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    /// False while the player is mid-way through a physical action they
    /// cannot abort — see [`PlayerState::is_committed_action`]. The
    /// loose-ball chase table skips these entries, so the "closest
    /// teammate" designation passes to someone who can actually go for
    /// the ball instead of yanking a diving keeper or a player already
    /// in the air out of their action.
    pub chase_eligible: bool,
    /// Multiplier on this player's time to a loose ball in the chase
    /// election — see [`Self::chase_bias_for`].
    pub chase_bias: f32,
    /// Top speed this tick, in u/tick — what a chase is priced in.
    pub max_speed: f32,
    /// How well he reads a pass — the interception composite, which sets
    /// how long after a strike he can set off for the ball.
    pub read: f32,
    /// What `read` was last priced on — see [`Self::refresh_read`].
    read_key: ReadKey,
}

/// The only inputs of the interception composite that move during a match:
/// condition, the minute, and a substitute's entry clock. Skills, crowd,
/// settledness and form are stamped before kick-off.
type ReadKey = (i16, u32, u64);

impl Default for PlayerFieldMetadata {
    #[inline]
    fn default() -> Self {
        PlayerFieldMetadata {
            player_id: 0,
            side: PlayerSide::Left,
            position: Vector3::zeros(),
            velocity: Vector3::zeros(),
            chase_eligible: true,
            chase_bias: 1.0,
            max_speed: 0.0,
            read: 0.5,
            read_key: (i16::MIN, u32::MAX, u64::MAX),
        }
    }
}

impl PlayerFieldMetadata {
    /// Re-price `read` only when one of its moving inputs has moved. The
    /// composite was ~90% of this store's per-tick refresh, recomputed for
    /// every man on every tick to reproduce last tick's number.
    #[inline]
    fn refresh_read(&mut self, player: &MatchPlayer, minute: u32) {
        let key = (
            player.player_attributes.condition,
            minute,
            player.entry_match_time_ms,
        );
        if key != self.read_key {
            self.read_key = key;
            self.read = sc::interception(player, minute);
        }
        debug_assert_eq!(
            self.read.to_bits(),
            sc::interception(player, minute).to_bits(),
            "interception-read memo mismatch: player={}",
            player.id
        );
    }

    /// Strikers gamble on loose balls and rebounds — it is a defining
    /// part of the role — so they read ~10% quicker to one than they
    /// are. Without it the election is pure geometry, and at youth level
    /// (where a forward's pace and finishing edge is smallest)
    /// midfielders were winning the six-yard-box scraps: measured 28.2%
    /// of youth MID shots came from <6m vs forwards' 17.5%, exactly
    /// inverted from senior (8.0% vs 25.0%). Keepers read slower: a ball
    /// an outfielder can reach is his to reach, and the keeper's own
    /// territory gates decide the rest.
    pub fn chase_bias_for(group: PlayerFieldPositionGroup) -> f32 {
        match group {
            PlayerFieldPositionGroup::Forward => 0.9,
            PlayerFieldPositionGroup::Goalkeeper => 1.25,
            _ => 1.0,
        }
    }
}

impl PlayerFieldData {
    pub const CAPACITY: usize = MAX_FIELD_PLAYERS;

    #[inline(always)]
    fn hash_slot(player_id: u32) -> u32 {
        player_id.wrapping_mul(2654435761) & (SLOT_TABLE_SIZE as u32 - 1)
    }

    #[inline]
    fn lookup_index(&self, player_id: u32) -> Option<usize> {
        let mask = (SLOT_TABLE_SIZE - 1) as u32;
        let mut idx = Self::hash_slot(player_id);
        // Walk to the first empty slot, as `insert_slot` did: the hash only
        // reads an id's low bits, and a lookup that gave up after eight
        // probes lost whoever the insert had displaced further — one man
        // in roughly one 36-player store in six.
        for _ in 0..SLOT_TABLE_SIZE {
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

    /// Which side this player is on, or `None` if he is not in the store.
    ///
    /// Same O(1) probe as [`Self::position`]. Exists so a caller that has
    /// only an id — the ball's `current_owner`, say — can ask "is he one
    /// of mine?" without a linear scan of the whole store in a per-player
    /// per-tick path.
    #[inline]
    pub fn side(&self, player_id: u32) -> Option<PlayerSide> {
        self.lookup_index(player_id)
            .map(|idx| unsafe { self.items.get_unchecked(idx) }.side)
    }

    /// `position(player_id)` without the hash probe for callers that
    /// already know the player's slot (`items` order == `field.players`
    /// then `field.substitutes` order, so a `field.players` index maps
    /// 1:1). The id check keeps it exact: on any mismatch (roster drift)
    /// it falls back to the id-keyed lookup, so the returned value is
    /// always what `position()` would produce.
    #[inline]
    pub fn position_by_index(&self, index: usize, player_id: u32) -> Vector3<f32> {
        if let Some(item) = self.items.get(index)
            && item.player_id == player_id
        {
            return item.position;
        }
        self.position(player_id)
    }

    /// Everything the store holds for this player, by the same O(1)
    /// probe as [`Self::position`].
    #[inline]
    pub fn get(&self, player_id: u32) -> Option<&PlayerFieldMetadata> {
        self.lookup_index(player_id)
            .map(|idx| unsafe { self.items.get_unchecked(idx) })
    }

    #[inline]
    pub fn has_player(&self, player_id: u32) -> bool {
        self.lookup_index(player_id).is_some()
    }

    /// Top speed in **units per physics tick**, condition-adjusted — the
    /// same number the movement layer and the chase table race on.
    ///
    /// `skills.physical.pace` is a 1-20 ATTRIBUTE and is not a speed;
    /// dividing a distance by it produces a tick count that is wrong by
    /// more than an order of magnitude and, because the real relation is
    /// affine (`0.36 + pace01 * 0.27`), the error does not cancel in a
    /// ratio either: pace 5 against pace 20 is 4.00:1 as an attribute and
    /// 1.51:1 as a speed.
    #[inline]
    pub fn max_speed(&self, player_id: u32) -> f32 {
        self.lookup_index(player_id)
            .map(|idx| unsafe { self.items.get_unchecked(idx) }.max_speed)
            .unwrap_or(PlayerSkills::MIN_MAX_SPEED)
    }

    #[inline]
    pub fn velocity(&self, player_id: u32) -> Vector3<f32> {
        if let Some(idx) = self.lookup_index(player_id) {
            unsafe { self.items.get_unchecked(idx) }.velocity
        } else {
            Vector3::zeros()
        }
    }

    /// Position and velocity in one probe. Missing-id fallbacks match
    /// `position` / `velocity` exactly (off-field sentinel, zero vector).
    #[inline]
    pub fn pos_vel(&self, player_id: u32) -> (Vector3<f32>, Vector3<f32>) {
        if let Some(idx) = self.lookup_index(player_id) {
            let item = unsafe { self.items.get_unchecked(idx) };
            (item.position, item.velocity)
        } else {
            (Vector3::new(-1000.0, -1000.0, 0.0), Vector3::zeros())
        }
    }

    /// Slice of active player metadata
    #[inline]
    pub fn as_slice(&self) -> &[PlayerFieldMetadata] {
        &self.items[..self.len]
    }

    /// The men on the pitch, sent-off players included — [`Self::as_slice`]
    /// without the bench.
    #[inline]
    pub fn on_pitch(&self) -> &[PlayerFieldMetadata] {
        &self.items[..self.on_pitch]
    }
}

impl PlayerFieldData {
    pub fn update(&mut self, field: &MatchField) {
        let new_count = field.players.len() + field.substitutes.len();
        let minute = sc::minute_from_ticks(field.ball.current_tick_cached);
        self.on_pitch = field.players.len();

        // Full rebuild only when player count changes (substitution)
        if new_count != self.len {
            self.len = 0;
            self.id_slots = [(0, SLOT_EMPTY); SLOT_TABLE_SIZE];

            for p in field.players.iter().chain(field.substitutes.iter()) {
                let idx = self.len;
                let mut meta = PlayerFieldMetadata {
                    player_id: p.id,
                    side: p
                        .side
                        .unwrap_or_else(|| panic!("unknown player side, player_id = {}", p.id)),
                    position: p.position,
                    velocity: p.velocity,
                    chase_eligible: !p.state.is_committed_action(),
                    chase_bias: PlayerFieldMetadata::chase_bias_for(
                        p.tactical_position.current_position.position_group(),
                    ),
                    max_speed: p.max_speed_with_condition_cached(),
                    ..PlayerFieldMetadata::default()
                };
                meta.refresh_read(p, minute);
                self.items[idx] = meta;
                self.insert_slot(p.id, idx as u8);
                self.len += 1;
            }
        } else {
            // Fast path: only the per-tick mutable fields. `chase_eligible`
            // rides along with position/velocity because state changes
            // every tick — a stale value would leave a diving keeper in
            // the chase table for a full tick after they committed.
            for (i, p) in field
                .players
                .iter()
                .chain(field.substitutes.iter())
                .enumerate()
            {
                self.items[i].position = p.position;
                self.items[i].velocity = p.velocity;
                self.items[i].chase_eligible = !p.state.is_committed_action();
                self.items[i].max_speed = p.max_speed_with_condition_cached();
                self.items[i].refresh_read(p, minute);
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
            on_pitch: 0,
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
