use crate::PlayerPositionType;
use crate::r#match::MatchField;
use nalgebra::Vector3;

const CELL_SIZE: f32 = 100.0;
const GRID_COLS: usize = 9; // ceil(840 / 100)
const GRID_ROWS: usize = 6; // ceil(545 / 100)
const MAX_PER_CELL: usize = 8;
const MAX_GRID_PLAYERS: usize = 24;
const SLOT_TABLE_SIZE: usize = 64;
const SLOT_EMPTY: u8 = 0xFF;
const MAX_DISTANCE: f32 = 999.0;

#[derive(Clone, Copy)]
pub struct GridPlayer {
    pub id: u32,
    pub team_id: u32,
    pub position: Vector3<f32>,
    pub tactical_position: PlayerPositionType,
}

impl Default for GridPlayer {
    fn default() -> Self {
        GridPlayer {
            id: 0,
            team_id: 0,
            position: Vector3::zeros(),
            tactical_position: PlayerPositionType::Goalkeeper,
        }
    }
}

const NUM_CELLS: usize = GRID_ROWS * GRID_COLS;

/// Nearest-neighbour index over the 22-ish on-pitch players.
///
/// Layout note: this used to be a real 9×6 bucket grid whose queries
/// walked every cell in the radius window. With only ~22 players on a
/// 54-cell pitch, a 250-unit query visited ~40 mostly-empty cells —
/// `NearbyIter::next` alone was ~27% of match CPU. The buckets are gone:
/// `all_players` now holds everyone SORTED by (row-major cell key, field
/// index), and `key_start` gives each cell key's slice of that array
/// (prefix-sum offsets). A query walks one contiguous segment per window
/// row — only the players actually inside the window, no empty-cell
/// visits. The sort keeps the yield order identical to the old cell walk
/// (cells row-major, insertion order within a cell), and `query_mask`
/// reproduces its MAX_PER_CELL overflow rule (players past the 8th in
/// one cell were invisible to queries), so results are bit-for-bit what
/// the bucket walk produced.
pub struct SpatialGrid {
    /// On-pitch players sorted by (cell key, field index). The full
    /// roster is always present here (overflow players included) so
    /// id-keyed lookups (`get` / `position_of` / `player_at`) never miss.
    all_players: [GridPlayer; MAX_GRID_PLAYERS],
    /// `key_start[k]..key_start[k+1]` = the sorted-array slice holding
    /// cell key `k`'s players. `key_start[NUM_CELLS] == num_players`.
    key_start: [u8; NUM_CELLS + 1],
    /// Bit i set ⇔ `all_players[i]` is visible to proximity queries.
    /// Clear only for players dropped by the MAX_PER_CELL overflow rule.
    query_mask: u32,
    num_players: usize,
    id_slots: [(u32, u8); SLOT_TABLE_SIZE],
    /// Last tick's (id, cell key) per FIELD index, plus the sorted
    /// (key, field index) layout built from it. Players cross a 100-unit
    /// cell boundary rarely relative to the tick rate, so most updates
    /// keep the exact same layout — then only the position/tactical data
    /// needs refreshing and the sort / prefix table / id table / overflow
    /// mask all carry over.
    prev_ids: [u32; MAX_GRID_PLAYERS],
    prev_keys: [u16; MAX_GRID_PLAYERS],
    order: [(u16, u8); MAX_GRID_PLAYERS],
    layout_valid: bool,
}

impl SpatialGrid {
    pub fn new() -> Self {
        SpatialGrid {
            all_players: [GridPlayer {
                id: 0,
                team_id: 0,
                position: Vector3::new(0.0, 0.0, 0.0),
                tactical_position: PlayerPositionType::Goalkeeper,
            }; MAX_GRID_PLAYERS],
            key_start: [0; NUM_CELLS + 1],
            query_mask: 0,
            num_players: 0,
            id_slots: [(0, SLOT_EMPTY); SLOT_TABLE_SIZE],
            prev_ids: [0; MAX_GRID_PLAYERS],
            prev_keys: [0; MAX_GRID_PLAYERS],
            order: [(0, 0); MAX_GRID_PLAYERS],
            layout_valid: false,
        }
    }

    #[inline(always)]
    fn hash_of(id: u32) -> u32 {
        id.wrapping_mul(2654435761) & (SLOT_TABLE_SIZE as u32 - 1)
    }

    #[inline]
    fn lookup_index(&self, player_id: u32) -> Option<usize> {
        let mask = (SLOT_TABLE_SIZE - 1) as u32;
        let mut idx = Self::hash_of(player_id);
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
        let mut idx = Self::hash_of(player_id);
        loop {
            let entry = &mut self.id_slots[idx as usize];
            if entry.1 == SLOT_EMPTY {
                *entry = (player_id, index);
                return;
            }
            idx = (idx + 1) & mask;
        }
    }

    /// Row-major cell key for a position — the sort key that reproduces
    /// the old cell-walk visit order (rows ascending, columns within).
    #[inline]
    fn cell_key_of(position: Vector3<f32>) -> u16 {
        let col = ((position.x / CELL_SIZE).max(0.0) as usize).min(GRID_COLS - 1);
        let row = ((position.y / CELL_SIZE).max(0.0) as usize).min(GRID_ROWS - 1);
        (row * GRID_COLS + col) as u16
    }

    /// Window of cell rows/cols a radius query must examine — identical
    /// to the old bucket grid's window, so the visited-cell set (and with
    /// it the yield order) is unchanged.
    fn cell_range(position: Vector3<f32>, radius: f32) -> (usize, usize, usize, usize) {
        let center_col = ((position.x / CELL_SIZE).max(0.0) as usize).min(GRID_COLS - 1);
        let center_row = ((position.y / CELL_SIZE).max(0.0) as usize).min(GRID_ROWS - 1);
        let cells_r = (radius / CELL_SIZE) as usize + 1;
        (
            center_row.saturating_sub(cells_r),
            (center_row + cells_r).min(GRID_ROWS - 1),
            center_col.saturating_sub(cells_r),
            (center_col + cells_r).min(GRID_COLS - 1),
        )
    }

    /// O(N) update: re-sort the roster into cell order, rebuild the
    /// per-cell offsets and the id→index table. Called every full tick
    /// from GameTickContext::update().
    pub fn update(&mut self, field: &MatchField) {
        let n = field.players.len().min(MAX_GRID_PLAYERS);

        // Detect layout changes: any player crossing a cell boundary, a
        // substitution swapping an id in place, or a roster-length
        // change. When none happened, the sorted order / prefix table /
        // id table / overflow mask from last tick are all still exact.
        let mut layout_changed = !self.layout_valid || n != self.num_players;
        for (i, p) in field.players.iter().take(n).enumerate() {
            let key = Self::cell_key_of(p.position);
            if self.prev_keys[i] != key || self.prev_ids[i] != p.id {
                self.prev_keys[i] = key;
                self.prev_ids[i] = p.id;
                layout_changed = true;
            }
        }
        self.num_players = n;

        if !layout_changed {
            // Fast path (the common case): same players in the same
            // cells — refresh the live data in the existing slots.
            for slot in 0..n {
                let field_idx = self.order[slot].1 as usize;
                let p = &field.players[field_idx];
                let gp = &mut self.all_players[slot];
                gp.position = p.position;
                gp.tactical_position = p.tactical_position.current_position;
            }
            return;
        }

        // (cell key, field index) pairs, insertion-sorted. Stable for
        // equal keys because field index strictly increases — matching
        // the old per-cell insertion order.
        let mut order = [(0u16, 0u8); MAX_GRID_PLAYERS];
        for i in 0..n {
            let key = self.prev_keys[i];
            let item = (key, i as u8);
            let mut j = i;
            while j > 0 && order[j - 1].0 > key {
                order[j] = order[j - 1];
                j -= 1;
            }
            order[j] = item;
        }
        self.order = order;
        self.layout_valid = true;

        // Write the sorted snapshot; drop players past the MAX_PER_CELL
        // overflow rule from query visibility (parity with the old
        // fixed-size buckets — they stay id-addressable). Count per-key
        // occupancy for the prefix table as we go.
        self.id_slots = [(0, SLOT_EMPTY); SLOT_TABLE_SIZE];
        let mut counts = [0u8; NUM_CELLS];
        let mut visible = 0u32;
        let mut run_key = u16::MAX;
        let mut run_len = 0usize;
        for (slot, &(key, field_idx)) in order[..n].iter().enumerate() {
            let p = &field.players[field_idx as usize];
            self.all_players[slot] = GridPlayer {
                id: p.id,
                team_id: p.team_id,
                position: p.position,
                tactical_position: p.tactical_position.current_position,
            };
            counts[key as usize] += 1;
            if key == run_key {
                run_len += 1;
            } else {
                run_key = key;
                run_len = 1;
            }
            if run_len <= MAX_PER_CELL {
                visible |= 1 << slot;
            }
            self.insert_slot(p.id, slot as u8);
        }
        self.query_mask = visible;

        // Prefix-sum the counts into slice offsets.
        let mut acc = 0u8;
        for (k, &c) in counts.iter().enumerate() {
            self.key_start[k] = acc;
            acc += c;
        }
        self.key_start[NUM_CELLS] = acc;
    }

    // ─── Public API (compatible with PlayerDistanceClosure) ───

    /// On-demand distance between two players.
    #[inline]
    pub fn get(&self, player_from_id: u32, player_to_id: u32) -> f32 {
        if player_from_id == player_to_id {
            return 0.0;
        }
        let a = match self.lookup_index(player_from_id) {
            Some(i) => i,
            None => return MAX_DISTANCE,
        };
        let b = match self.lookup_index(player_to_id) {
            Some(i) => i,
            None => return MAX_DISTANCE,
        };
        let pa = self.all_players[a].position;
        let pb = self.all_players[b].position;
        let dx = pa.x - pb.x;
        let dy = pa.y - pb.y;
        (dx * dx + dy * dy).sqrt()
    }

    /// Opponents within max_distance — returns (player_id, distance).
    /// Same signature as PlayerDistanceClosure::opponents().
    pub fn opponents(
        &self,
        player_id: u32,
        max_distance: f32,
    ) -> impl Iterator<Item = (u32, f32)> + '_ {
        self.create_iter(player_id, 0.0, max_distance, false)
            .map(|(gp, dist)| (gp.id, dist))
    }

    /// Teammates within [min_distance, max_distance] — returns (player_id, distance).
    /// Same signature as PlayerDistanceClosure::teammates().
    pub fn teammates(
        &self,
        player_id: u32,
        min_distance: f32,
        max_distance: f32,
    ) -> impl Iterator<Item = (u32, f32)> + '_ {
        self.create_iter(player_id, min_distance, max_distance, true)
            .map(|(gp, dist)| (gp.id, dist))
    }

    /// Rich opponent query — returns GridPlayer directly (avoids HashMap lookup in nearby()).
    pub fn opponents_full(
        &self,
        player_id: u32,
        team_id: u32,
        position: Vector3<f32>,
        max_distance: f32,
    ) -> NearbyIter<'_> {
        let (r_min, r_max, c_min, c_max) = Self::cell_range(position, max_distance);
        NearbyIter {
            grid: self,
            idx: 0,
            seg_end: 0,
            row: r_min,
            r_max,
            c_min,
            c_max,
            same_team: false,
            team_id,
            player_id,
            position,
            radius_sq: max_distance * max_distance,
            min_dist_sq: 0.0,
        }
    }

    /// Rich teammate query — returns GridPlayer directly.
    pub fn teammates_full(
        &self,
        player_id: u32,
        team_id: u32,
        position: Vector3<f32>,
        min_distance: f32,
        max_distance: f32,
    ) -> NearbyIter<'_> {
        let (r_min, r_max, c_min, c_max) = Self::cell_range(position, max_distance);
        NearbyIter {
            grid: self,
            idx: 0,
            seg_end: 0,
            row: r_min,
            r_max,
            c_min,
            c_max,
            same_team: true,
            team_id,
            player_id,
            position,
            radius_sq: max_distance * max_distance,
            min_dist_sq: min_distance * min_distance,
        }
    }

    /// Lookup a player's cached position.
    #[inline]
    pub fn position_of(&self, player_id: u32) -> Vector3<f32> {
        match self.lookup_index(player_id) {
            Some(i) => self.all_players[i].position,
            None => Vector3::new(-1000.0, -1000.0, 0.0),
        }
    }

    /// Smallest squared distance from `player_id` to any query-visible
    /// same/other-team entry — the whole-board reduction behind the
    /// `exists(radius)` fast path: `∃ entry with dist_sq ≤ r²` ⇔
    /// `nearest_dist_sq ≤ r²`, with the same entry set, the same center
    /// (the player's grid-stored position) and the same squared-distance
    /// values a radius query would filter on. `f32::INFINITY` when no
    /// candidate exists (matching the empty iterator).
    pub fn nearest_dist_sq(&self, player_id: u32, same_team: bool) -> f32 {
        let Some(i) = self.lookup_index(player_id) else {
            return f32::INFINITY;
        };
        let center = self.all_players[i].position;
        let team_id = self.all_players[i].team_id;
        let mut best = f32::INFINITY;
        for (slot, gp) in self.all_players[..self.num_players].iter().enumerate() {
            if self.query_mask & (1 << slot) == 0 {
                continue;
            }
            if gp.id == player_id {
                continue;
            }
            if (gp.team_id == team_id) != same_team {
                continue;
            }
            let dx = gp.position.x - center.x;
            let dy = gp.position.y - center.y;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq < best {
                best = dist_sq;
            }
        }
        best
    }

    /// Lookup full GridPlayer data by ID.
    #[inline]
    pub fn player_at(&self, player_id: u32) -> Option<&GridPlayer> {
        self.lookup_index(player_id).map(|i| &self.all_players[i])
    }

    // ─── Internal ───

    fn create_iter(
        &self,
        player_id: u32,
        min_distance: f32,
        max_distance: f32,
        same_team: bool,
    ) -> NearbyIter<'_> {
        match self.lookup_index(player_id) {
            Some(i) => {
                let gp = &self.all_players[i];
                let (r_min, r_max, c_min, c_max) = Self::cell_range(gp.position, max_distance);
                NearbyIter {
                    grid: self,
                    idx: 0,
                    seg_end: 0,
                    row: r_min,
                    r_max,
                    c_min,
                    c_max,
                    same_team,
                    team_id: gp.team_id,
                    player_id,
                    position: gp.position,
                    radius_sq: max_distance * max_distance,
                    min_dist_sq: min_distance * min_distance,
                }
            }
            // Player not found — empty iterator (row cursor past r_max).
            None => NearbyIter {
                grid: self,
                idx: 0,
                seg_end: 0,
                row: 1,
                r_max: 0,
                c_min: 0,
                c_max: 0,
                same_team,
                team_id: 0,
                player_id,
                position: Vector3::zeros(),
                radius_sq: 0.0,
                min_dist_sq: 0.0,
            },
        }
    }
}

// ─── Iterator ───

/// Walks the query window one cell-row at a time. Because the sorted
/// array groups players by row-major cell key, each window row
/// `[row*COLS + c_min, row*COLS + c_max]` is ONE contiguous slice — the
/// iterator touches only players actually inside the window, in exactly
/// the order the old per-cell walk yielded them.
pub struct NearbyIter<'g> {
    grid: &'g SpatialGrid,
    /// Cursor / end of the current row segment in the sorted array.
    idx: usize,
    seg_end: usize,
    /// Next window row to open (`row > r_max` + drained segment = done).
    row: usize,
    r_max: usize,
    c_min: usize,
    c_max: usize,
    same_team: bool,
    team_id: u32,
    player_id: u32,
    position: Vector3<f32>,
    radius_sq: f32,
    min_dist_sq: f32,
}

impl<'g> Iterator for NearbyIter<'g> {
    type Item = (GridPlayer, f32);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            while self.idx < self.seg_end {
                let i = self.idx;
                self.idx += 1;

                if self.grid.query_mask & (1 << i) == 0 {
                    continue;
                }
                let gp = &self.grid.all_players[i];
                if gp.id == self.player_id {
                    continue;
                }
                if (gp.team_id == self.team_id) != self.same_team {
                    continue;
                }

                let dx = gp.position.x - self.position.x;
                let dy = gp.position.y - self.position.y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq > self.radius_sq || dist_sq < self.min_dist_sq {
                    continue;
                }

                return Some((*gp, dist_sq.sqrt()));
            }

            // Open the next window row's slice.
            if self.row > self.r_max {
                return None;
            }
            let base = self.row * GRID_COLS;
            self.idx = self.grid.key_start[base + self.c_min] as usize;
            self.seg_end = self.grid.key_start[base + self.c_max + 1] as usize;
            self.row += 1;
        }
    }
}
