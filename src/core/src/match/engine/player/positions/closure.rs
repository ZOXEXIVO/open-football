use crate::r#match::{MatchField, MatchPlayer};
use crate::utils::cpu::avx2_available;
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

        // SoA scratch so the AVX2 kernel can do contiguous 8-wide loads.
        // Padding past n is zeroed; matrix cells touching those lanes are
        // never read (`slot_of` only resolves ids in 0..n).
        let mut xs = [0.0f32; MAX_PLAYERS];
        let mut ys = [0.0f32; MAX_PLAYERS];
        for i in 0..n {
            xs[i] = players[i].position.x;
            ys[i] = players[i].position.y;
        }

        // Distances — upper triangle then mirror. Runtime-dispatched:
        // AVX2 when the host CPU advertises it (cached by std::detect),
        // scalar otherwise. The two paths produce bit-identical results
        // — both use `(dx*dx + dy*dy).sqrt()` lane-wise; no FMA so the
        // rounding sequence matches the scalar loop.
        if avx2_available() {
            // Safety: `avx2_available` confirms AVX2 is supported on this
            // host. `xs`/`ys` are [f32; MAX_PLAYERS] so 8-wide loads up
            // to MAX_PLAYERS are in bounds; the kernel never reads/writes
            // past index MAX_PLAYERS-1.
            unsafe {
                compute_dist_matrix_avx2(&xs, &ys, n, &mut self.dist_matrix);
            }
        } else {
            compute_dist_matrix_scalar(&xs, &ys, n, &mut self.dist_matrix);
        }

        // Per-player neighbor lists — scalar fan-out reading distances
        // from the matrix the kernel just filled.
        for i in 0..n {
            let outer = &players[i];
            for j in (i + 1)..n {
                let inner = &players[j];
                let distance = self.dist_matrix[i * MAX_PLAYERS + j];

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

// ─── Distance-matrix kernels ─────────────────────────────────────────
//
// Hot path: rebuilt every tick (~5400 ticks/match), N² over up to
// `MAX_PLAYERS` (typically 22). The AVX2 kernel processes 8 j-lanes per
// iteration; the scalar fallback runs on hosts without AVX2 and on
// non-x86 targets.
//
// Both kernels write to the same dist_matrix layout and produce
// bit-identical values lane-by-lane (`(dx*dx + dy*dy).sqrt()` — no
// FMA). Lanes past `n` may be written with garbage; consumers never
// query slot indices ≥ n.

#[inline]
fn compute_dist_matrix_scalar(
    xs: &[f32; MAX_PLAYERS],
    ys: &[f32; MAX_PLAYERS],
    n: usize,
    matrix: &mut [f32; MAX_PLAYERS * MAX_PLAYERS],
) {
    for i in 0..n {
        let xi = xs[i];
        let yi = ys[i];
        let row_base = i * MAX_PLAYERS;
        for j in (i + 1)..n {
            let dx = xi - xs[j];
            let dy = yi - ys[j];
            let distance = (dx * dx + dy * dy).sqrt();
            matrix[row_base + j] = distance;
            matrix[j * MAX_PLAYERS + i] = distance;
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn compute_dist_matrix_avx2(
    xs: &[f32; MAX_PLAYERS],
    ys: &[f32; MAX_PLAYERS],
    n: usize,
    matrix: &mut [f32; MAX_PLAYERS * MAX_PLAYERS],
) {
    use std::arch::x86_64::*;
    unsafe {
        for i in 0..n {
            let xi = _mm256_set1_ps(xs[i]);
            let yi = _mm256_set1_ps(ys[i]);
            let row_base = i * MAX_PLAYERS;

            let mut j = i + 1;
            // 8-wide chunks. Bound on n (not MAX_PLAYERS) so we don't
            // waste lanes computing distances we'll never use.
            while j + 8 <= n {
                let xj = _mm256_loadu_ps(xs.as_ptr().add(j));
                let yj = _mm256_loadu_ps(ys.as_ptr().add(j));
                let dx = _mm256_sub_ps(xi, xj);
                let dy = _mm256_sub_ps(yi, yj);
                let dx2 = _mm256_mul_ps(dx, dx);
                let dy2 = _mm256_mul_ps(dy, dy);
                let d2 = _mm256_add_ps(dx2, dy2);
                let d = _mm256_sqrt_ps(d2);

                // Row store: contiguous.
                _mm256_storeu_ps(matrix.as_mut_ptr().add(row_base + j), d);

                // Column mirror: AVX2 has no f32 scatter — write 8 lanes
                // back via a stack buffer, scalar scatter to column i.
                let mut tmp = [0.0f32; 8];
                _mm256_storeu_ps(tmp.as_mut_ptr(), d);
                for k in 0..8 {
                    *matrix.get_unchecked_mut((j + k) * MAX_PLAYERS + i) = tmp[k];
                }

                j += 8;
            }

            // Scalar tail (<8 elements). For n=22 this runs at most 7
            // times per i, decreasing as i grows.
            while j < n {
                let dx = xs[i] - xs[j];
                let dy = ys[i] - ys[j];
                let distance = (dx * dx + dy * dy).sqrt();
                *matrix.get_unchecked_mut(row_base + j) = distance;
                *matrix.get_unchecked_mut(j * MAX_PLAYERS + i) = distance;
                j += 1;
            }
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
unsafe fn compute_dist_matrix_avx2(
    _xs: &[f32; MAX_PLAYERS],
    _ys: &[f32; MAX_PLAYERS],
    _n: usize,
    _matrix: &mut [f32; MAX_PLAYERS * MAX_PLAYERS],
) {
    // Never called on non-x86_64; `avx2_available` returns false. Stub
    // exists so the call site doesn't need its own cfg gate.
    unreachable!("compute_dist_matrix_avx2 called on non-x86_64 target");
}

#[cfg(test)]
mod avx_tests {
    use super::*;

    fn fill_positions(n: usize, seed: u32) -> ([f32; MAX_PLAYERS], [f32; MAX_PLAYERS]) {
        let mut xs = [0.0f32; MAX_PLAYERS];
        let mut ys = [0.0f32; MAX_PLAYERS];
        // Deterministic, spread across a realistic 105×68 pitch.
        let mut s = seed;
        for i in 0..n {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            xs[i] = (s % 10500) as f32 / 100.0;
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            ys[i] = (s % 6800) as f32 / 100.0;
        }
        (xs, ys)
    }

    #[test]
    fn avx2_matches_scalar_bitwise() {
        if !avx2_available() {
            return;
        }
        for &n in &[0usize, 1, 2, 7, 8, 9, 15, 16, 17, 22, 23, MAX_PLAYERS] {
            let (xs, ys) = fill_positions(n, n as u32 + 1);
            let mut m_scalar = [0.0f32; MAX_PLAYERS * MAX_PLAYERS];
            let mut m_avx = [0.0f32; MAX_PLAYERS * MAX_PLAYERS];
            compute_dist_matrix_scalar(&xs, &ys, n, &mut m_scalar);
            unsafe { compute_dist_matrix_avx2(&xs, &ys, n, &mut m_avx) };

            // Only the in-range (i,j) cells are part of the contract.
            for i in 0..n {
                for j in 0..n {
                    if i == j {
                        continue;
                    }
                    let a = m_scalar[i * MAX_PLAYERS + j].to_bits();
                    let b = m_avx[i * MAX_PLAYERS + j].to_bits();
                    assert_eq!(
                        a, b,
                        "n={} i={} j={} scalar={} avx={}",
                        n,
                        i,
                        j,
                        f32::from_bits(a),
                        f32::from_bits(b)
                    );
                }
            }
        }
    }
}
