use crate::r#match::MatchField;
use crate::PlayerPositionType;
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

#[derive(Clone, Copy)]
struct GridCell {
    players: [GridPlayer; MAX_PER_CELL],
    count: u8,
    generation: u32,
}

impl GridCell {
    const fn new() -> Self {
        GridCell {
            players: [GridPlayer {
                id: 0,
                team_id: 0,
                position: Vector3::new(0.0, 0.0, 0.0),
                tactical_position: PlayerPositionType::Goalkeeper,
            }; MAX_PER_CELL],
            count: 0,
            generation: 0,
        }
    }
}

pub struct SpatialGrid {
    cells: [[GridCell; GRID_COLS]; GRID_ROWS],
    all_players: [GridPlayer; MAX_GRID_PLAYERS],
    num_players: usize,
    id_slots: [(u32, u8); SLOT_TABLE_SIZE],
    generation: u32,
}

impl SpatialGrid {
    pub fn new() -> Self {
        // Use const array init to avoid Default requirement
        const EMPTY_CELL: GridCell = GridCell::new();
        SpatialGrid {
            cells: [[EMPTY_CELL; GRID_COLS]; GRID_ROWS],
            all_players: [GridPlayer {
                id: 0,
                team_id: 0,
                position: Vector3::new(0.0, 0.0, 0.0),
                tactical_position: PlayerPositionType::Goalkeeper,
            }; MAX_GRID_PLAYERS],
            num_players: 0,
            id_slots: [(0, SLOT_EMPTY); SLOT_TABLE_SIZE],
            generation: 0,
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

    #[inline]
    fn cell_of(position: Vector3<f32>) -> (usize, usize) {
        let col = (position.x / CELL_SIZE).max(0.0) as usize;
        let row = (position.y / CELL_SIZE).max(0.0) as usize;
        (row.min(GRID_ROWS - 1), col.min(GRID_COLS - 1))
    }

    fn cell_range(position: Vector3<f32>, radius: f32) -> (usize, usize, usize, usize) {
        let (center_row, center_col) = Self::cell_of(position);
        let cells_r = (radius / CELL_SIZE) as usize + 1;
        (
            center_row.saturating_sub(cells_r),
            (center_row + cells_r).min(GRID_ROWS - 1),
            center_col.saturating_sub(cells_r),
            (center_col + cells_r).min(GRID_COLS - 1),
        )
    }

    /// O(N) update: refresh all positions and grid cell assignments.
    /// Called every full tick from GameTickContext::update().
    pub fn update(&mut self, field: &MatchField) {
        let n = field.players.len();
        self.generation = self.generation.wrapping_add(1);
        let current_gen = self.generation;

        // Detect roster changes (substitutions, sent-off players). The
        // earlier heuristic only checked count + the first slot's id,
        // which missed substitutions at any index > 0 — `field.players`
        // length doesn't change on a sub (the slot is replaced in
        // place), and the new id never made it into `id_slots`. That
        // produced stale grid lookups (and panics downstream when AI
        // strategies dereferenced the missing id). Compare every slot
        // against the previous tick's snapshot; the loop is bounded by
        // MAX_GRID_PLAYERS so this is still effectively O(1).
        let mut ids_changed = n != self.num_players;
        if !ids_changed {
            for i in 0..n {
                if self.all_players[i].id != field.players[i].id {
                    ids_changed = true;
                    break;
                }
            }
        }

        if ids_changed {
            self.num_players = n;
            self.id_slots = [(0, SLOT_EMPTY); SLOT_TABLE_SIZE];
            for (i, p) in field.players.iter().enumerate() {
                self.insert_slot(p.id, i as u8);
            }
        }

        for (i, p) in field.players.iter().enumerate() {
            let gp = GridPlayer {
                id: p.id,
                team_id: p.team_id,
                position: p.position,
                tactical_position: p.tactical_position.current_position,
            };
            self.all_players[i] = gp;

            let (row, col) = Self::cell_of(p.position);
            let cell = &mut self.cells[row][col];
            if cell.generation != current_gen {
                cell.count = 0;
                cell.generation = current_gen;
            }
            if (cell.count as usize) < MAX_PER_CELL {
                cell.players[cell.count as usize] = gp;
                cell.count += 1;
            }
        }
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
    pub fn opponents(&self, player_id: u32, max_distance: f32) -> impl Iterator<Item = (u32, f32)> + '_ {
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
            row: r_min,
            col: c_min,
            cell_idx: 0,
            _r_min: r_min,
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
            row: r_min,
            col: c_min,
            cell_idx: 0,
            _r_min: r_min,
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

    /// Lookup full GridPlayer data by ID.
    #[inline]
    pub fn player_at(&self, player_id: u32) -> Option<&GridPlayer> {
        self.lookup_index(player_id).map(|i| &self.all_players[i])
    }

    // ─── Internal ───

    fn create_iter(&self, player_id: u32, min_distance: f32, max_distance: f32, same_team: bool) -> NearbyIter<'_> {
        let info = self.lookup_index(player_id).map(|i| {
            let gp = &self.all_players[i];
            (gp.position, gp.team_id)
        });

        match info {
            Some((position, team_id)) => {
                let (r_min, r_max, c_min, c_max) = Self::cell_range(position, max_distance);
                NearbyIter {
                    grid: self,
                    row: r_min,
                    col: c_min,
                    cell_idx: 0,
                    _r_min: r_min,
                    r_max,
                    c_min,
                    c_max,
                    same_team,
                    team_id,
                    player_id,
                    position,
                    radius_sq: max_distance * max_distance,
                    min_dist_sq: min_distance * min_distance,
                }
            }
            None => {
                // Player not found — empty iterator (r_min > r_max)
                NearbyIter {
                    grid: self,
                    row: 1,
                    col: 0,
                    cell_idx: 0,
                    _r_min: 1,
                    r_max: 0,
                    c_min: 0,
                    c_max: 0,
                    same_team,
                    team_id: 0,
                    player_id,
                    position: Vector3::zeros(),
                    radius_sq: 0.0,
                    min_dist_sq: 0.0,
                }
            }
        }
    }
}

// ─── Iterator ───

pub struct NearbyIter<'g> {
    grid: &'g SpatialGrid,
    row: usize,
    col: usize,
    cell_idx: usize,
    _r_min: usize,
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
        let current_gen = self.grid.generation;

        loop {
            if self.row > self.r_max {
                return None;
            }

            let cell = &self.grid.cells[self.row][self.col];
            let count = if cell.generation == current_gen {
                cell.count as usize
            } else {
                0
            };

            while self.cell_idx < count {
                let gp = cell.players[self.cell_idx];
                self.cell_idx += 1;

                if gp.id == self.player_id {
                    continue;
                }

                let is_same_team = gp.team_id == self.team_id;
                if self.same_team != is_same_team {
                    continue;
                }

                let dx = gp.position.x - self.position.x;
                let dy = gp.position.y - self.position.y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq > self.radius_sq || dist_sq < self.min_dist_sq {
                    continue;
                }

                return Some((gp, dist_sq.sqrt()));
            }

            // Advance to next cell
            self.cell_idx = 0;
            self.col += 1;
            if self.col > self.c_max {
                self.col = self.c_min;
                self.row += 1;
            }
        }
    }
}
