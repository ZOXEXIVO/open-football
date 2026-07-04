use crate::r#match::player::strategies::players::DefensiveRole;
use crate::r#match::player::strategies::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::player::strategies::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::{
    MatchField, MatchObjectsPositions, MatchPlayerCollection, PassOriginRestart, PlayerSide,
    ShotTarget, Space, SpatialGrid,
};
use crate::{PlayerFieldPositionGroup, PlayerPositionType};
use nalgebra::Vector3;
use std::cell::RefCell;

pub struct GameTickContext {
    pub positions: MatchObjectsPositions,
    pub grid: SpatialGrid,
    pub ball: BallMetadata,
    pub space: Space,
    /// Per-side closest-to-loose-ball table, recomputed whenever the
    /// ball view refreshes. Replaces the per-player O(N) roster scan in
    /// the dispatcher's loose-ball force/yield overrides (22 players ×
    /// ~44 entries per un-owned tick) with an O(1) lookup per player.
    pub chase: LooseBallChase,
    /// Once-per-tick join of the on-pitch roster (`context.players.
    /// entries`) with the live position store. The `teammates()/
    /// opponents()` iterators and the per-player team aggregates
    /// (`is_best_player_to_chase_ball`, `is_teammate_chasing_ball`)
    /// previously re-did an id→position hash probe (and a `by_id` skill
    /// lookup) PER ELEMENT PER CALL — with several calls per player per
    /// tick that was the engine's dominant hidden cost. The join pays
    /// ~22 probes once per full tick; every consumer then walks a
    /// contiguous array.
    pub roster: RosterJoin,
    /// Per-(player, tick) memo for aggregate queries a single player's
    /// state machine recomputes several times within one tick (e.g.
    /// `is_control_ball` is reached via `should_transition_to_walking`
    /// AND `should_push_up` in one `DefenderStanding::process`). Each
    /// value is a deterministic function of the asking player + the
    /// FROZEN tick snapshot, so memoizing per (player, tick) is
    /// bit-identical. NB: these queries are NOT team-level — several
    /// compare a teammate's distance-to-ball against the ASKING player's
    /// own position — so the key MUST be the player, not the team.
    /// Players are processed sequentially, so one slot suffices; it
    /// resets whenever the (player, tick) changes. The accessors carry a
    /// `debug_assert` recompute-and-compare on every hit so a memo bug
    /// panics in debug/test runs (it already caught a wrong team-level
    /// assumption here).
    pub player_agg_cache: RefCell<PlayerTickCache>,
    /// Cross-tick per-player memos for the role skill profiles — see
    /// `DefenderSkillProfile::from_player_memo`. Lives on the tick
    /// context (single match thread) rather than `MatchPlayer` so the
    /// player stays `Sync` for the parallel league/world harnesses.
    pub profile_memos: RefCell<ProfileMemos>,
}

/// Cross-tick `(player_id, packed key, profile)` memo rows for the role
/// skill profiles. The profiles are pure functions of static skills plus
/// a handful of slowly-moving integers (condition, jadedness, minute,
/// pressure counts — all packed into the key by the profile's
/// `memo_key`), yet cost ~40 `powf` curve evaluations to build. Rows are
/// keyed by player id (≤ 22 on-pitch entries — a linear scan is cheaper
/// than any table at this size) and overwritten in place when the key
/// moves.
#[derive(Default)]
pub struct ProfileMemos {
    defender: Vec<(u32, u64, DefenderSkillProfile)>,
    midfielder: Vec<(u32, u64, MidfielderSkillProfile)>,
    goalkeeper: Vec<(u32, u64, GoalkeeperSkillProfile)>,
}

impl ProfileMemos {
    fn new() -> Self {
        ProfileMemos {
            defender: Vec::with_capacity(24),
            midfielder: Vec::with_capacity(24),
            goalkeeper: Vec::with_capacity(4),
        }
    }

    #[inline]
    pub fn goalkeeper_get(&self, player_id: u32, key: u64) -> Option<GoalkeeperSkillProfile> {
        self.goalkeeper
            .iter()
            .find(|(id, k, _)| *id == player_id && *k == key)
            .map(|(_, _, p)| *p)
    }

    pub fn goalkeeper_put(&mut self, player_id: u32, key: u64, profile: GoalkeeperSkillProfile) {
        if let Some(row) = self
            .goalkeeper
            .iter_mut()
            .find(|(id, _, _)| *id == player_id)
        {
            *row = (player_id, key, profile);
        } else {
            self.goalkeeper.push((player_id, key, profile));
        }
    }

    #[inline]
    pub fn defender_get(&self, player_id: u32, key: u64) -> Option<DefenderSkillProfile> {
        self.defender
            .iter()
            .find(|(id, k, _)| *id == player_id && *k == key)
            .map(|(_, _, p)| *p)
    }

    pub fn defender_put(&mut self, player_id: u32, key: u64, profile: DefenderSkillProfile) {
        if let Some(row) = self.defender.iter_mut().find(|(id, _, _)| *id == player_id) {
            *row = (player_id, key, profile);
        } else {
            self.defender.push((player_id, key, profile));
        }
    }

    #[inline]
    pub fn midfielder_get(&self, player_id: u32, key: u64) -> Option<MidfielderSkillProfile> {
        self.midfielder
            .iter()
            .find(|(id, k, _)| *id == player_id && *k == key)
            .map(|(_, _, p)| *p)
    }

    pub fn midfielder_put(&mut self, player_id: u32, key: u64, profile: MidfielderSkillProfile) {
        if let Some(row) = self
            .midfielder
            .iter_mut()
            .find(|(id, _, _)| *id == player_id)
        {
            *row = (player_id, key, profile);
        } else {
            self.midfielder.push((player_id, key, profile));
        }
    }
}

/// One player's cached per-tick aggregates. `None` = not computed yet for
/// the current (player, tick). Extend in lockstep with the
/// `TeamOperationsImpl` accessors that fill them.
pub struct PlayerTickCache {
    player_id: u32,
    tick: u64,
    pub is_control_ball: Option<bool>,
    pub is_teammate_chasing_ball: Option<bool>,
    pub counter_window: Option<bool>,
    pub is_attack_ready: Option<bool>,
    pub is_best_to_chase_ball: Option<bool>,
    pub defensive_role: Option<DefensiveRole>,
    /// Role skill profiles — ~26 banded skill reads + ~40 `powf` curve
    /// evaluations each, and a state machine reaches `from_ctx` several
    /// times within one tick (velocity() and process() both consult
    /// them). Every input is tick-frozen (skills static, condition
    /// updated once before the state runs, grid/ball snapshots), so the
    /// memo is bit-identical.
    pub defender_profile: Option<DefenderSkillProfile>,
    pub midfielder_profile: Option<MidfielderSkillProfile>,
    /// Deepest outfield opponent's x (the offside line) as computed by
    /// `MidfielderAttackSupportingState::is_offside_risk` — a roster
    /// min-scan that does not depend on the candidate position being
    /// tested, yet ran once per candidate. `Some(inner)` = computed this
    /// tick (`inner` = the scan's `Option<f32>`).
    pub offside_last_defender_x: Option<Option<f32>>,
    /// Squared distance to the nearest query-visible opponent / teammate
    /// (grid entry set; `f32::INFINITY` = none). Backs the `exists(r)`
    /// fast path — states probe several radii per tick and each probe
    /// used to walk the grid window; one whole-board min per (player,
    /// tick) answers them all exactly (`nearest ≤ r²` ⇔ the query
    /// iterator is non-empty).
    pub nearest_opponent_sq: Option<f32>,
    pub nearest_teammate_sq: Option<f32>,
}

impl Default for PlayerTickCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerTickCache {
    pub fn new() -> Self {
        PlayerTickCache {
            player_id: 0,
            // u64::MAX is never a real tick, so the first access always
            // resets.
            tick: u64::MAX,
            is_control_ball: None,
            is_teammate_chasing_ball: None,
            counter_window: None,
            is_attack_ready: None,
            is_best_to_chase_ball: None,
            defensive_role: None,
            defender_profile: None,
            midfielder_profile: None,
            offside_last_defender_x: None,
            nearest_opponent_sq: None,
            nearest_teammate_sq: None,
        }
    }

    /// Mutable view keyed to `(player_id, tick)`. Clears all cached fields
    /// whenever the player or tick changes.
    pub fn slot_mut(&mut self, player_id: u32, tick: u64) -> &mut Self {
        if self.player_id != player_id || self.tick != tick {
            self.player_id = player_id;
            self.tick = tick;
            self.is_control_ball = None;
            self.is_teammate_chasing_ball = None;
            self.counter_window = None;
            self.is_attack_ready = None;
            self.is_best_to_chase_ball = None;
            self.defensive_role = None;
            self.defender_profile = None;
            self.midfielder_profile = None;
            self.offside_last_defender_x = None;
            self.nearest_opponent_sq = None;
            self.nearest_teammate_sq = None;
        }
        self
    }
}

impl GameTickContext {
    pub fn new(field: &MatchField, players: &MatchPlayerCollection) -> Self {
        let mut grid = SpatialGrid::new();
        grid.update(field);
        let positions = MatchObjectsPositions::from(field);
        let mut chase = LooseBallChase::new();
        chase.update(&positions);
        let mut roster = RosterJoin::new();
        roster.update(players, &positions);
        GameTickContext {
            ball: BallMetadata::from(field),
            positions,
            grid,
            space: Space::from(field),
            chase,
            roster,
            player_agg_cache: RefCell::new(PlayerTickCache::new()),
            profile_memos: RefCell::new(ProfileMemos::new()),
        }
    }

    #[inline]
    pub fn update(&mut self, field: &MatchField, players: &MatchPlayerCollection) {
        self.ball.update(field);
        self.positions.update(field);
        self.grid.update(field);
        self.space.update(field);
        // `chase` and the roster's per-team ball-distance `control` table
        // are NOT refreshed here: their only readers run in play_players,
        // and `refresh_ball` (between play_ball and play_players) rebuilds
        // both against the post-physics ball anyway — computing them here
        // was pure dead work (verified: nothing in the ball module touches
        // `tick_context.chase` / `roster.control_*`).
        self.roster.update_entries(players, &self.positions);
    }

    /// Cheaper refresh used during shot-flight light ticks where only
    /// the two goalkeepers run AI. Skips `Space` (raycast / pass-line
    /// scratchpad) because GK strategies don't read it — they react off
    /// `BallMetadata::cached_shot_target` and live positions. Keeps
    /// ball + player positions + spatial grid in sync so the keeper's
    /// distance-to-ball and chase decisions stay correct.
    #[inline]
    pub fn update_for_goalkeeper_shot(
        &mut self,
        field: &MatchField,
        players: &MatchPlayerCollection,
    ) {
        self.ball.update(field);
        self.positions.update(field);
        self.grid.update(field);
        self.chase.update(&self.positions);
        self.roster.update(players, &self.positions);
    }

    /// Refresh just the ball view. Used between `play_ball` and
    /// `play_players` so the dispatcher's TakeBall assignment sees the
    /// latest ownership — otherwise a player who just claimed mid-tick
    /// gets force-assigned to TakeBall because `is_owned` is still the
    /// stale tick-start value of `false`.
    #[inline]
    pub fn refresh_ball(&mut self, field: &MatchField) {
        self.ball.update(field);
        self.positions.ball.update_from(&field.ball);
        // Landing position may have moved (restart, deflection inside
        // play_ball) — the chase table keys off it, so recompute. Same
        // for the roster's control table (keys off the ball position).
        self.chase.update(&self.positions);
        self.roster.refresh_control(self.positions.ball.position);
    }
}

/// One roster entry in the loose-ball chase table.
#[derive(Debug, Clone, Copy)]
pub struct ChaseEntry {
    pub dist_sq: f32,
    pub id: u32,
}

/// Per-side two-smallest `(dist_sq, id)` table against the ball's
/// landing position, over the SAME entry set the dispatcher's loose-ball
/// overrides used to scan per player (`positions.players.as_slice()`,
/// substitutes included). Lexicographic ordering (dist_sq, then id)
/// makes the O(1) queries reproduce the original scans exactly:
///
///   * `should_force_takeball`: "no other same-side entry strictly
///     closer, id tie-break" ⇔ best-other's (dist_sq, id) doesn't beat
///     mine.
///   * `should_yield_takeball`: "any other same-side entry closer than
///     threshold" ⇔ best-other's dist_sq < threshold.
///
/// Two slots per side suffice because queries exclude at most one entry
/// (the asking player) and every id appears once in the store.
pub struct LooseBallChase {
    left: [Option<ChaseEntry>; 2],
    right: [Option<ChaseEntry>; 2],
}

impl LooseBallChase {
    pub fn new() -> Self {
        LooseBallChase {
            left: [None; 2],
            right: [None; 2],
        }
    }

    #[inline]
    fn beats(a: ChaseEntry, b: ChaseEntry) -> bool {
        a.dist_sq < b.dist_sq || (a.dist_sq == b.dist_sq && a.id < b.id)
    }

    pub fn update(&mut self, positions: &MatchObjectsPositions) {
        let ball_pos = positions.ball.landing_position;
        self.left = [None; 2];
        self.right = [None; 2];
        for meta in positions.players.as_slice() {
            let entry = ChaseEntry {
                dist_sq: (ball_pos - meta.position).norm_squared(),
                id: meta.player_id,
            };
            let slots = match meta.side {
                PlayerSide::Left => &mut self.left,
                PlayerSide::Right => &mut self.right,
            };
            match slots[0] {
                None => slots[0] = Some(entry),
                Some(best) if Self::beats(entry, best) => {
                    slots[1] = slots[0];
                    slots[0] = Some(entry);
                }
                Some(_) => match slots[1] {
                    None => slots[1] = Some(entry),
                    Some(second) if Self::beats(entry, second) => slots[1] = Some(entry),
                    Some(_) => {}
                },
            }
        }
    }

    /// Lexicographic-min `(dist_sq, id)` entry on `side`, excluding
    /// `exclude_id` (the asking player). `None` only when the side has
    /// no other entries.
    #[inline]
    pub fn best_other(&self, side: PlayerSide, exclude_id: u32) -> Option<ChaseEntry> {
        let slots = match side {
            PlayerSide::Left => &self.left,
            PlayerSide::Right => &self.right,
        };
        match slots[0] {
            Some(best) if best.id != exclude_id => Some(best),
            Some(_) => slots[1],
            None => None,
        }
    }
}

impl Default for LooseBallChase {
    fn default() -> Self {
        Self::new()
    }
}

/// One on-pitch player in the per-tick roster join: the static
/// `PlayerEntry` fields, the live position/velocity copied from the
/// position store, and the precomputed chase-ability denominator used
/// by `is_best_player_to_chase_ball`.
#[derive(Clone, Copy)]
pub struct RosterEntryLive {
    pub id: u32,
    pub team_id: u32,
    /// The entry's tactical position — same snapshot semantics as
    /// `PlayerEntry::position` (match-start, refreshed on substitution),
    /// which is what `MatchPlayerLite::tactical_positions` carried.
    pub position_type: PlayerPositionType,
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    /// `(pace/20 · accel/20 · position_factor · 0.5 + 0.5)²` — the exact
    /// per-teammate denominator `is_best_player_to_chase_ball` derived
    /// via a `by_id` skill lookup per candidate per call. Skills are
    /// static in-match and the position factor keys off the entry's
    /// tactical position, so once per tick is exact.
    pub chase_ability_sq: f32,
}

/// Once-per-tick join of `MatchPlayerCollection::entries` (the on-pitch
/// roster, entries order) with the live position store. Iteration order,
/// element set, and joined values are exactly what the per-call
/// `entries.iter() + positions.players.position(id)` path produced —
/// consumers are bit-identical, they just stop hashing per element.
pub struct RosterJoin {
    entries: Vec<RosterEntryLive>,
    /// Per-team two-smallest `(dist_sq, id)` against the CURRENT ball
    /// position (not the landing position — that's `LooseBallChase`).
    /// Backs `is_control_ball`'s "closest teammate vs closest opponent"
    /// fallback, which previously re-scanned both teams per asking
    /// player per tick. Lexicographic (dist_sq, id) with a second slot
    /// so a query can exclude the asking player exactly.
    control: [(u32, [Option<ChaseEntry>; 2]); 2],
    /// Per-team entry indices, ascending — the team-filtered iterators
    /// (`teammates().all()`, `opponents().all()`, by-position variants)
    /// walk only their ~11 relevant entries instead of filtering all 22
    /// per call. Ascending indices reproduce the full-walk yield order
    /// exactly.
    team_rows: [(u32, Vec<u8>); 2],
}

impl RosterJoin {
    pub fn new() -> Self {
        RosterJoin {
            entries: Vec::with_capacity(22),
            control: [(0, [None; 2]), (0, [None; 2])],
            team_rows: [(0, Vec::with_capacity(12)), (0, Vec::with_capacity(12))],
        }
    }

    pub fn update(&mut self, players: &MatchPlayerCollection, positions: &MatchObjectsPositions) {
        self.update_entries(players, positions);
        self.refresh_control(positions.ball.position);
    }

    /// Refresh the joined entries WITHOUT rebuilding the per-team ball
    /// `control` table. Used by the per-tick context update, whose
    /// control table would be dead work — `refresh_ball` rebuilds it
    /// against the post-physics ball before any consumer runs.
    pub fn update_entries(
        &mut self,
        players: &MatchPlayerCollection,
        positions: &MatchObjectsPositions,
    ) {
        let n = players.entries.len();
        self.entries.truncate(n);
        for (i, entry) in players.entries.iter().enumerate() {
            let (position, velocity) = positions.players.pos_vel(entry.id);
            // Skills are static in-match and the position factor keys off
            // the entry's tactical position, so the denominator computed
            // last tick is still exact while the same (id, position)
            // occupies the same slot — the steady state between roster /
            // shape changes. Only recompute (a `by_id` probe + a few
            // flops) when that pairing breaks.
            let cached = self.entries.get(i).and_then(|prev| {
                (prev.id == entry.id && prev.position_type == entry.position)
                    .then_some(prev.chase_ability_sq)
            });
            let chase_ability_sq = match cached {
                Some(a) => a,
                None => match players.by_id(entry.id) {
                    Some(p) => {
                        let pace_factor = p.skills.physical.pace / 20.0;
                        let acceleration_factor = p.skills.physical.acceleration / 20.0;
                        let position_factor = match entry.position.position_group() {
                            PlayerFieldPositionGroup::Forward => 1.2,
                            PlayerFieldPositionGroup::Midfielder => 1.1,
                            PlayerFieldPositionGroup::Defender => 0.9,
                            PlayerFieldPositionGroup::Goalkeeper => 0.5,
                        };
                        let ability =
                            pace_factor * acceleration_factor * position_factor * 0.5 + 0.5;
                        ability * ability
                    }
                    // Mirrors the old scan's `by_id → None => return false`
                    // arm ("candidate can't disqualify me"). A NaN denominator
                    // makes `dist_sq / chase_ability_sq < threshold` always
                    // false — the candidate is skipped, same as before.
                    None => f32::NAN,
                },
            };
            let live = RosterEntryLive {
                id: entry.id,
                team_id: entry.team_id,
                position_type: entry.position,
                position,
                velocity,
                chase_ability_sq,
            };
            if let Some(slot) = self.entries.get_mut(i) {
                *slot = live;
            } else {
                self.entries.push(live);
            }
        }

        // Per-team index rows (ascending = entries order). Rebuilt every
        // update — 22 pushes, trivial next to the join above.
        self.team_rows[0].1.clear();
        self.team_rows[1].1.clear();
        for (i, e) in self.entries.iter().enumerate() {
            let row = if self.team_rows[0].1.is_empty() || self.team_rows[0].0 == e.team_id {
                self.team_rows[0].0 = e.team_id;
                &mut self.team_rows[0].1
            } else {
                self.team_rows[1].0 = e.team_id;
                &mut self.team_rows[1].1
            };
            row.push(i as u8);
        }
    }

    /// Entry indices for `team_id` (`same == true`) or for the other
    /// team (`same == false`), ascending. Empty when no such team.
    #[inline]
    fn row(&self, team_id: u32, same: bool) -> &[u8] {
        for (tid, rows) in &self.team_rows {
            if !rows.is_empty() && ((*tid == team_id) == same) {
                return rows;
            }
        }
        &[]
    }

    /// Iterate `team_id`'s entries in entries order — the exact
    /// subsequence a full `iter().filter(team_id ==)` walk yields.
    #[inline]
    pub fn iter_team(&self, team_id: u32) -> impl Iterator<Item = &RosterEntryLive> + '_ {
        self.row(team_id, true)
            .iter()
            .map(move |&i| &self.entries[i as usize])
    }

    /// Iterate the OTHER team's entries in entries order.
    #[inline]
    pub fn iter_other_team(&self, team_id: u32) -> impl Iterator<Item = &RosterEntryLive> + '_ {
        self.row(team_id, false)
            .iter()
            .map(move |&i| &self.entries[i as usize])
    }

    /// Rebuild the per-team ball-distance table from the joined entries.
    /// Split out of `update` because the ball may move within a tick
    /// (`refresh_ball` after `play_ball`) while player positions stay
    /// frozen — only this table needs recomputing then. Operand order
    /// mirrors the scan it replaced (`entry.position - ball_pos`).
    pub fn refresh_control(&mut self, ball_pos: Vector3<f32>) {
        self.control = [(0, [None; 2]), (0, [None; 2])];
        for entry in &self.entries {
            let d = entry.position - ball_pos;
            let candidate = ChaseEntry {
                dist_sq: d.norm_squared(),
                id: entry.id,
            };
            let slot = if self.control[0].1[0].is_none() || self.control[0].0 == entry.team_id {
                self.control[0].0 = entry.team_id;
                &mut self.control[0].1
            } else {
                self.control[1].0 = entry.team_id;
                &mut self.control[1].1
            };
            match slot[0] {
                None => slot[0] = Some(candidate),
                Some(best) if LooseBallChase::beats(candidate, best) => {
                    slot[1] = slot[0];
                    slot[0] = Some(candidate);
                }
                Some(_) => match slot[1] {
                    None => slot[1] = Some(candidate),
                    Some(second) if LooseBallChase::beats(candidate, second) => {
                        slot[1] = Some(candidate)
                    }
                    Some(_) => {}
                },
            }
        }
    }

    /// Minimum ball-distance (squared) among `team_id`'s entries,
    /// excluding `exclude_id`. `None` when the team has no other entry —
    /// matching the empty-iterator `min_by` of the scan it replaces.
    #[inline]
    pub fn control_min_excluding(&self, team_id: u32, exclude_id: u32) -> Option<f32> {
        let slots = if self.control[0].0 == team_id {
            &self.control[0].1
        } else if self.control[1].0 == team_id {
            &self.control[1].1
        } else {
            return None;
        };
        Self::min_from_slots(slots, exclude_id)
    }

    /// Same as [`control_min_excluding`](Self::control_min_excluding)
    /// but for the team that is NOT `my_team_id` — saves callers a scan
    /// to discover the opposing team id.
    #[inline]
    pub fn control_min_other_team(&self, my_team_id: u32, exclude_id: u32) -> Option<f32> {
        let slots = if self.control[0].0 != my_team_id && self.control[0].1[0].is_some() {
            &self.control[0].1
        } else if self.control[1].0 != my_team_id && self.control[1].1[0].is_some() {
            &self.control[1].1
        } else {
            return None;
        };
        Self::min_from_slots(slots, exclude_id)
    }

    #[inline]
    fn min_from_slots(slots: &[Option<ChaseEntry>; 2], exclude_id: u32) -> Option<f32> {
        match slots[0] {
            Some(best) if best.id != exclude_id => Some(best.dist_sq),
            Some(_) => slots[1].map(|e| e.dist_sq),
            None => None,
        }
    }

    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, RosterEntryLive> {
        self.entries.iter()
    }
}

impl Default for RosterJoin {
    fn default() -> Self {
        Self::new()
    }
}

pub struct BallMetadata {
    pub is_owned: bool,
    pub is_in_flight_state: usize,

    pub current_owner: Option<u32>,
    pub last_owner: Option<u32>,

    notified_buf: [u32; 4],
    notified_len: u8,

    pub ownership_duration: u32,

    recent_buf: [u32; 5],
    recent_len: u8,

    /// Projected goal-line crossing for the current shot, if a shot is
    /// in flight. Read by the keeper's `PreparingForSave` /
    /// `Catching` states to commit to an intercept line.
    pub cached_shot_target: Option<ShotTarget>,

    /// How the current possession started. Persists from a restart
    /// (corner / goal-kick / throw-in / free-kick) until the ball is
    /// next brought under open-play control. Read by the corner set-up
    /// logic (taker waits for the box to load; centre-backs push up to
    /// attack the delivery).
    pub pass_origin_restart: PassOriginRestart,

    /// Tick of the most recent live rebound (dangerous parry / loose
    /// block deflection). Read by the team shot gate to suspend the
    /// shot-spacing cooldown during box scrambles. 0 = none yet.
    pub last_rebound_tick: u64,
}

impl BallMetadata {
    #[inline]
    pub fn notified_players(&self) -> &[u32] {
        &self.notified_buf[..self.notified_len as usize]
    }

    #[inline]
    pub fn recent_passers(&self) -> &[u32] {
        &self.recent_buf[..self.recent_len as usize]
    }

    fn update(&mut self, field: &MatchField) {
        self.is_owned = field.ball.current_owner.is_some();
        self.is_in_flight_state = field.ball.flags.in_flight_state;
        self.current_owner = field.ball.current_owner;
        self.last_owner = field.ball.previous_owner;
        self.ownership_duration = field.ball.ownership_duration;

        self.notified_len = field.ball.take_ball_notified_players.len().min(4) as u8;
        for (i, &id) in field
            .ball
            .take_ball_notified_players
            .iter()
            .take(4)
            .enumerate()
        {
            self.notified_buf[i] = id;
        }

        self.recent_len = field.ball.recent_passers.len().min(5) as u8;
        for (i, &id) in field.ball.recent_passers.iter().take(5).enumerate() {
            self.recent_buf[i] = id;
        }

        self.cached_shot_target = field.ball.cached_shot_target;
        self.pass_origin_restart = field.ball.pass_origin_restart;
        self.last_rebound_tick = field.ball.last_rebound_tick;
    }
}

impl From<&MatchField> for BallMetadata {
    fn from(field: &MatchField) -> Self {
        let mut meta = BallMetadata {
            is_owned: false,
            is_in_flight_state: 0,
            current_owner: None,
            last_owner: None,
            notified_buf: [0; 4],
            notified_len: 0,
            ownership_duration: 0,
            recent_buf: [0; 5],
            recent_len: 0,
            cached_shot_target: None,
            pass_origin_restart: PassOriginRestart::OpenPlay,
            last_rebound_tick: 0,
        };
        meta.update(field);
        meta
    }
}
