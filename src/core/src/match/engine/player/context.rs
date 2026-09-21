use crate::PlayerPositionType;
use crate::r#match::common_states::ChasePath;
use crate::r#match::engine::ball::ball::contest::interception::InterceptionContest;
use crate::r#match::engine::ball::ball::{
    Ball, CONTROL_DISTANCE, LOOSE_CLAIM_DISTANCE, RunUpPhase,
};
use crate::r#match::player::strategies::players::DefensiveRole;
use crate::r#match::player::strategies::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::player::strategies::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::position_players::PlayerFieldData;
use crate::r#match::{
    MatchField, MatchObjectsPositions, MatchPlayerCollection, MatchPlayerLite, PassOriginRestart,
    PlayerSide, ShotTarget, Space, SpatialGrid,
};
use nalgebra::Vector3;
use std::cell::RefCell;

pub struct GameTickContext {
    pub positions: MatchObjectsPositions,
    pub grid: SpatialGrid,
    pub ball: BallMetadata,
    pub space: Space,
    /// Per-side first-to-the-ball table, recomputed whenever the ball
    /// view refreshes. Replaces the per-player O(N) roster scan in the
    /// dispatcher's loose-ball force/yield overrides (22 players × ~44
    /// entries per un-owned tick) with an O(1) lookup per player.
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
    /// Keyless rows: the guard receiver-threat blend
    /// (`DefenderGuardingState::receiver_threat`) reads only static
    /// in-match skills, so a value computed once holds for the whole
    /// match — no invalidation key needed.
    receiver_threat: Vec<(u32, f32)>,
}

impl ProfileMemos {
    fn new() -> Self {
        ProfileMemos {
            defender: Vec::with_capacity(24),
            midfielder: Vec::with_capacity(24),
            goalkeeper: Vec::with_capacity(4),
            receiver_threat: Vec::with_capacity(24),
        }
    }

    #[inline]
    pub fn receiver_threat_get(&self, player_id: u32) -> Option<f32> {
        self.receiver_threat
            .iter()
            .find(|(id, _)| *id == player_id)
            .map(|(_, v)| *v)
    }

    pub fn receiver_threat_put(&mut self, player_id: u32, value: f32) {
        self.receiver_threat.push((player_id, value));
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
    ///
    /// The defender slot carries the condition/jadedness it was built at
    /// as well: the goal-side rule reads a profile *before* dispatch, and
    /// dispatch is what applies the tick's fatigue, so this one memo can
    /// legitimately be asked for a profile on both sides of a condition
    /// change. See `DefenderSkillProfile::from_ctx`.
    pub defender_profile: Option<(u32, DefenderSkillProfile)>,
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
    /// Guard-target pick (`find_guard_target`) — a scored scan over
    /// nearby opponents that both `process()` and `velocity()` of the
    /// guarding states run within one tick. One slot serves the
    /// defender AND midfielder variants: a player occupies exactly one
    /// role-specific state per tick, so the slot never mixes formulas.
    /// `Some(inner)` = computed this tick (`inner` = the pick).
    pub guard_target: Option<Option<MatchPlayerLite>>,
    /// Passer-side invariants of the pass evaluator, recomputed
    /// identically for EVERY candidate receiver — and some states run
    /// `find_best_pass_option` several times within one tick. Both are
    /// pure over the frozen snapshot + the passer's own frozen
    /// condition, and cached only when the passer IS the slot player
    /// (`evaluate_pass` keeps an explicit passer arg for generality,
    /// but every live call site passes `ctx.player`).
    pub pass_pressure_factor: Option<f32>,
    /// `(passing_execution, long_passing)` composite pair backing
    /// `PassEvaluator::calculate_passer_ability` — only the per-
    /// candidate distance blend varies between calls.
    pub passing_composites: Option<(f32, f32)>,
    /// `ShapeDiscipline::organisation` — the recall multiplier. Read on
    /// the positional hot path (`apply_with_pull` runs for every player
    /// every tick, and `velocity()` and `process()` both reach it), and
    /// it builds a `SkillBands` set for `decision_quality`. Tick-frozen:
    /// skills are static in-match and the team aggregate it reads is
    /// itself only recomputed every ~100 ticks.
    pub shape_organisation: Option<f32>,
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
            guard_target: None,
            pass_pressure_factor: None,
            passing_composites: None,
            shape_organisation: None,
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
            self.guard_target = None;
            self.pass_pressure_factor = None;
            self.passing_composites = None;
            self.shape_organisation = None;
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
        chase.update(&positions, &field.ball);
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
        self.chase.update(&self.positions, &field.ball);
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
        // The ball may have moved (restart, deflection inside play_ball)
        // — the chase table keys off its path, so recompute. Same for
        // the roster's control table (keys off the ball position).
        self.chase.update(&self.positions, &field.ball);
        self.roster.refresh_control(self.positions.ball.position);
    }
}

/// One entry in the roster's ball-distance control table.
#[derive(Debug, Clone, Copy)]
pub struct ChaseEntry {
    pub dist_sq: f32,
    pub id: u32,
}

impl ChaseEntry {
    #[inline]
    fn beats(self, other: ChaseEntry) -> bool {
        self.dist_sq < other.dist_sq || (self.dist_sq == other.dist_sq && self.id < other.id)
    }
}

/// One player in the loose-ball chase table.
#[derive(Debug, Clone, Copy)]
pub struct ChaseRow {
    pub id: u32,
    pub side: PlayerSide,
    pub eligible: bool,
    /// Ticks until he can be on the ball — see [`ChasePath`] — scaled
    /// by his role's `chase_bias`.
    pub cost: f32,
}

impl ChaseRow {
    /// Lexicographic `(cost, id)`: exactly one man per side, the same
    /// one from every asker's point of view.
    #[inline]
    fn beats(self, other: ChaseRow) -> bool {
        self.cost < other.cost || (self.cost == other.cost && self.id < other.id)
    }
}

/// Who gets to a loose ball first, per side, over the SAME entry set the
/// dispatcher's loose-ball overrides used to scan per player
/// (`positions.players.as_slice()`, substitutes included). Players
/// committed to an un-abortable action (`chase_eligible == false`) keep
/// a row but never hold a designation.
///
/// Priced in TIME along the ball's projected path rather than distance
/// to where it is — see [`ChasePath`] for why. The two-smallest slots
/// per side answer every election query in O(1); `should_force_takeball`
/// is "nobody on my side beats my row" and `should_yield_takeball` is
/// "the best other row beats mine by the hysteresis".
///
/// A pass in flight ends where its receiver takes it: the path every
/// row is priced on stops at his collection point ([`ChasePath::end_at`]),
/// so the defending side's man races to where the pass is collected
/// rather than along a roll the ball will never complete.
pub struct LooseBallChase {
    left: [Option<ChaseRow>; 2],
    right: [Option<ChaseRow>; 2],
    rows: [ChaseRow; PlayerFieldData::CAPACITY],
    len: usize,
    end: Option<PathEnd>,
}

/// Where a pass in flight is going to be taken, and by whom.
#[derive(Debug, Clone, Copy)]
pub struct PathEnd {
    pub receiver: u32,
    pub point: Vector3<f32>,
    /// Ticks from now.
    pub tick: f32,
}

impl LooseBallChase {
    pub fn new() -> Self {
        LooseBallChase {
            left: [None; 2],
            right: [None; 2],
            rows: [ChaseRow {
                id: 0,
                side: PlayerSide::Left,
                eligible: false,
                cost: f32::INFINITY,
            }; PlayerFieldData::CAPACITY],
            len: 0,
            end: None,
        }
    }

    pub fn update(&mut self, positions: &MatchObjectsPositions, ball: &Ball) {
        let mut path = ChasePath::project(&positions.ball, ball.field_width, ball.field_height);
        // A man has to READ the ball before he can go for it. The side it
        // was played to knew it was coming; everybody else waits out his
        // own read of the strike (`InterceptionContest::read_delay`) and
        // holds no designation until then. Measured without it, 780
        // passes a match were cut out by men who stood 3.5 m off the lane
        // at the strike and were running for the meeting point on the
        // next tick.
        let since_strike = ball
            .current_tick_cached
            .saturating_sub(ball.last_release_tick) as f32;
        let knowing_side = ball
            .pass_target_player_id
            .and_then(|id| positions.players.side(id));
        self.end = (ball.flags.in_flight_state > 0)
            .then_some(ball.pass_target_player_id)
            .flatten()
            .and_then(|id| {
                positions
                    .players
                    .as_slice()
                    .iter()
                    .find(|meta| meta.player_id == id && meta.chase_eligible)
            })
            .map(|meta| {
                let tick = path.time_to_reach(meta.position, meta.max_speed, CONTROL_DISTANCE);
                PathEnd {
                    receiver: meta.player_id,
                    point: path.end_at(tick),
                    tick,
                }
            });
        self.left = [None; 2];
        self.right = [None; 2];
        self.len = 0;
        // The lane, for the readiness half of the clock below, measured
        // from where the ball was struck.
        let release = ball.last_release_position;
        let ball_vel = positions.ball.velocity;
        let lane = {
            let speed = ball_vel.x.hypot(ball_vel.y);
            (speed > 1e-3).then(|| (ball_vel.x / speed, ball_vel.y / speed))
        };
        for meta in positions.players.as_slice() {
            let read_wait = if Some(meta.side) == knowing_side {
                0.0
            } else {
                // A man who was ALREADY STANDING IN THE LANE when it was
                // struck is not made ready by the clock that prices a
                // reaction and a step — see
                // `InterceptionContest::chase_delay`. Applied to the
                // election as well as to the contest, so eligibility to go
                // and readiness to play it cannot disagree.
                //
                // Where he was, not where he is: asked at his current
                // position the test catches everyone the ball happens to
                // pass close to, which is most of the men it passes at
                // all. Ahead of the ball, too — one it has already gone
                // by is not standing in the way of anything.
                let perp = lane.map_or(f32::MAX, |(dx, dy)| {
                    let was_there = meta.position - meta.velocity * since_strike;
                    let rx = was_there.x - release.x;
                    let ry = was_there.y - release.y;
                    let along = rx * dx + ry * dy;
                    if along <= 0.0 {
                        f32::MAX
                    } else {
                        (rx * dy - ry * dx).abs()
                    }
                });
                (InterceptionContest::chase_delay(perp) - since_strike).max(0.0)
            };
            let row = ChaseRow {
                id: meta.player_id,
                side: meta.side,
                eligible: meta.chase_eligible && read_wait <= 0.0,
                cost: (read_wait
                    + path.time_to_reach(meta.position, meta.max_speed, LOOSE_CLAIM_DISTANCE))
                    * meta.chase_bias,
            };
            self.rows[self.len] = row;
            self.len += 1;
            if !row.eligible {
                continue;
            }
            let slots = match meta.side {
                PlayerSide::Left => &mut self.left,
                PlayerSide::Right => &mut self.right,
            };
            match slots[0] {
                None => slots[0] = Some(row),
                Some(best) if row.beats(best) => {
                    slots[1] = slots[0];
                    slots[0] = Some(row);
                }
                Some(_) => match slots[1] {
                    None => slots[1] = Some(row),
                    Some(second) if row.beats(second) => slots[1] = Some(row),
                    Some(_) => {}
                },
            }
        }
    }

    /// Every player's row, in position-store order.
    #[inline]
    pub fn rows(&self) -> &[ChaseRow] {
        &self.rows[..self.len]
    }

    /// This player's time to the ball. `None` only for an id that is not
    /// in the position store.
    #[inline]
    pub fn cost_of(&self, id: u32) -> Option<f32> {
        self.rows().iter().find(|r| r.id == id).map(|r| r.cost)
    }

    /// Lexicographic-min `(cost, id)` eligible row on `side`, excluding
    /// `exclude_id` (the asking player). `None` only when the side has
    /// no other eligible row.
    #[inline]
    pub fn best_other(&self, side: PlayerSide, exclude_id: u32) -> Option<ChaseRow> {
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

    #[inline]
    pub fn best(&self, side: PlayerSide) -> Option<ChaseRow> {
        match side {
            PlayerSide::Left => self.left[0],
            PlayerSide::Right => self.right[0],
        }
    }

    /// Is this player his side's designated chaser for the loose ball —
    /// no other eligible row on his side beats his?
    ///
    /// Shared so the questions that must agree cannot drift apart —
    /// `PlayerFieldPositionGroup::should_force_takeball`, which sends
    /// him after it, `TeamOperationsImpl::is_best_player_to_chase_ball`,
    /// which the state trees ask, and `DefensiveRecovery::depth_override`,
    /// which must not then turn him round and run him at his own goal.
    #[inline]
    pub fn is_designated(&self, side: PlayerSide, id: u32) -> bool {
        let Some(mine) = self.rows().iter().find(|r| r.id == id) else {
            return true;
        };
        if !mine.eligible {
            return false;
        }
        match self.best_other(side, id) {
            Some(best) => !best.beats(*mine),
            None => true,
        }
    }

    /// May this man go for the ball at all — not mid-way through an
    /// action he cannot abort, and past his read of the strike?
    #[inline]
    pub fn may_go(&self, id: u32) -> bool {
        self.rows()
            .iter()
            .find(|r| r.id == id)
            .is_none_or(|r| r.eligible)
    }

    /// Where the pass in flight, if there is one, is going to be taken.
    #[inline]
    pub fn path_end(&self) -> Option<PathEnd> {
        self.end
    }
}

impl Default for LooseBallChase {
    fn default() -> Self {
        Self::new()
    }
}

/// One on-pitch player in the per-tick roster join: the static
/// `PlayerEntry` fields and the live position/velocity copied from the
/// position store.
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
            let live = RosterEntryLive {
                id: entry.id,
                team_id: entry.team_id,
                position_type: entry.position,
                position,
                velocity,
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
                Some(best) if candidate.beats(best) => {
                    slot[1] = slot[0];
                    slot[0] = Some(candidate);
                }
                Some(_) => match slot[1] {
                    None => slot[1] = Some(candidate),
                    Some(second) if candidate.beats(second) => slot[1] = Some(candidate),
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

    /// The last man the ball came off WITHOUT him controlling it — a parry,
    /// a spill, a block, a fumbled claim. `None` after a controlled touch:
    /// a catch, a pass, a first touch that stuck.
    pub rebounded_off: Option<u32>,

    /// Who the live pass is meant for, if one is in the air.
    ///
    /// The claim rules already give this player sole right to take the
    /// ball while it is in flight, but nothing told him to go and get
    /// it: the loose-ball chase designation went to whichever teammate
    /// happened to be nearest the landing spot. When that was somebody
    /// else — which it often is, because a pass is played AHEAD of its
    /// target — the only man allowed to receive it was not moving to it
    /// and the only man moving to it was not allowed to receive it, so
    /// the pass ran through to nobody. Read by the receiving override in
    /// `PlayerFieldPositionGroup::process`.
    pub pass_target: Option<u32>,

    /// The player currently barred from re-collecting the ball because he
    /// released it himself and it has not travelled yet, if any.
    ///
    /// The ownership layer enforces this on its own claim paths, but the
    /// goalkeeper reaches ownership through his state machine
    /// (`Standing` → `PickingUpBall` → `CaughtBall` → `secure_ball_for`),
    /// which bypasses those paths entirely. Surfacing the bar here lets
    /// the keeper decline to go for it in the first place, instead of
    /// lunging at a ball the engine will not let him have.
    /// See `Ball::blocked_recollect_player`.
    pub recollect_blocked_player: Option<u32>,

    /// The player whose delivery this ball still is — he played it and
    /// nobody has touched it since. See `Ball::own_delivery_player`, and
    /// `KeeperDelivery`, which is what reads it: a keeper who has just
    /// thrown the ball out does not then chase it up the pitch.
    pub delivered_by: Option<u32>,

    /// The ball is in a goalkeeper's hands — uncontestable. Read by the
    /// pressing states (there is nothing to press) and by anything that
    /// would otherwise treat the keeper as a carrier who can be closed
    /// down. See `Ball::held_in_hands`.
    ///
    /// …and, since the throw-in became a throw, in the THROWER's hands
    /// for the second or two he stands on the line looking for a
    /// team-mate. The two are the same situation for everything that
    /// reads this — a man holding the ball with both hands who nobody may
    /// take it off — which is why they share the flag; where they differ
    /// is `throw_taker` below.
    pub held_in_hands: bool,
    /// **The man taking a throw-in right now**, from the tick he picks the
    /// ball up on the touchline until somebody else plays it.
    ///
    /// Read by [`ThrowInDelivery`](crate::r#match::player::strategies::
    /// common::states::ThrowInDelivery), which is the only thing that may
    /// move him while it is set: he is not dribbling, marking or making a
    /// run, he is standing over the line with the ball in his hands.
    /// See `Ball::throw_in_taker`.
    pub throw_taker: Option<u32>,
    /// **The man taking a kick-off right now**, and the team-mate the
    /// set-up put beside him to receive it.
    ///
    /// Read by [`KickoffDelivery`](crate::r#match::common_states::KickoffDelivery),
    /// the only thing that may move him while they are set: he is not
    /// dribbling, pressing or making a run, he is standing over a dead
    /// ball on the centre mark. See `Ball::kickoff_taker`.
    pub kickoff_taker: Option<u32>,
    pub kickoff_partner: Option<u32>,
    /// `(player, team)` of the team-mate whose deliberate kick or throw-in
    /// was the last touch, if the last touch was one. Feeds the back-pass
    /// half of `BallOperationsImpl::handling_verdict`.
    pub deliberate_kick_by: Option<(u32, u32)>,
    /// Goalkeeper who released the ball from his hands and is waiting for
    /// somebody else to play it before he may handle it again.
    pub hands_released_by: Option<u32>,
    /// Player an engine-level aerial contest has already awarded the ball
    /// to (`resolve_corner_contest` / `resolve_cross_contest`). Their
    /// heading state reads this to take a clean-contact roll instead of
    /// re-rolling the duel the contest just decided.
    /// See `Ball::aerial_contest_winner`.
    pub aerial_contest_winner: Option<u32>,

    /// The man a dead ball is waiting for, if one is
    /// (`Ball::awaiting_restart`).
    ///
    /// **The loose-ball election must not run on a ball that is out of
    /// play.** A restart's taker is not racing anybody — the ball is his
    /// by award — but to `should_yield_takeball` he was simply a player in
    /// `TakeBall` with a teammate nearer the ball, which for a goal kick
    /// he almost always is. So he was yielded straight back to `Standing`
    /// on the tick after every nudge and never took a step: measured, of
    /// the goal kicks whose keeper failed to arrive, **100% of them found
    /// him standing still**, a mean 15.7 m short, and the backstop
    /// teleport put the ball under him — which is the artefact
    /// [`AwaitedRestart`] exists to remove.
    ///
    /// It is also what stops the other twenty-one converging on a dead
    /// ball they are not allowed to touch.
    pub restart_taker: Option<u32>,

    /// The taker of a corner while he is CARRYING the ball to the arc, if
    /// one is (`AwaitedRestart::carrying`).
    ///
    /// He is the one player a dead ball moves with, and every rule that
    /// normally puts a man on a ball reads him as already there: the ball
    /// is at his feet, so `run_for_ball` stops him and `CornerHold`'s
    /// release fades to nothing. His set-piece station is what actually
    /// walks him to the flag, and this is what tells `CornerHold` to obey
    /// it instead of standing him down as the chaser.
    pub restart_carrier: Option<u32>,

    /// Where the goal-kick taker stands to take his run from, while the
    /// restart is holding for him to get there and set himself — `None`
    /// once he is running in, and for every other restart. Read by the
    /// keeper's `TakeBall`. See `GoalKickRunUp` and `KeeperGoalKick`.
    pub restart_mark: Option<Vector3<f32>>,
    /// …and whether he is on it, standing still, looking up.
    pub restart_set: bool,
    /// The goal kick in his possession was placed for a LONG kick. See
    /// `KeeperGoalKick::decided_long`.
    pub goal_kick_long: bool,
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
        for (i, entry) in field.ball.recent_passers.iter().take(5).enumerate() {
            self.recent_buf[i] = entry.player_id;
        }

        self.cached_shot_target = field.ball.cached_shot_target;
        self.pass_origin_restart = field.ball.pass_origin_restart;
        self.last_rebound_tick = field.ball.last_rebound_tick;
        self.rebounded_off = field
            .ball
            .last_touch_player_id
            .filter(|_| !field.ball.last_touch_was_controlled);
        self.pass_target = field.ball.pass_target_player_id;
        self.recollect_blocked_player = field.ball.blocked_recollect_player();
        self.delivered_by = field.ball.own_delivery_player();
        self.held_in_hands = field.ball.held_in_hands;
        self.throw_taker = field.ball.throw_in_taker;
        self.kickoff_taker = field.ball.kickoff_taker;
        self.kickoff_partner = field.ball.kickoff_partner;
        self.aerial_contest_winner = field.ball.aerial_contest_winner;
        self.restart_taker = field.ball.awaiting_restart.map(|r| r.taker_id);
        self.restart_carrier = field
            .ball
            .awaiting_restart
            .filter(|r| r.carrying)
            .map(|r| r.taker_id);
        let run_up = field.ball.goal_kick_run_up;
        self.restart_mark = run_up
            .filter(|r| r.phase != RunUpPhase::Running)
            .map(|r| r.mark);
        self.restart_set = run_up.is_some_and(|r| r.phase == RunUpPhase::Set);
        self.goal_kick_long = field.ball.goal_kick_long;
        self.deliberate_kick_by = if field.ball.last_touch_was_deliberate_kick {
            field
                .ball
                .last_touch_player_id
                .zip(field.ball.last_touch_team_id)
        } else {
            None
        };
        self.hands_released_by = field
            .ball
            .last_release_player_id
            .filter(|_| field.ball.last_release_from_hands);
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
            rebounded_off: None,
            pass_target: None,
            recollect_blocked_player: None,
            delivered_by: None,
            held_in_hands: false,
            throw_taker: None,
            kickoff_taker: None,
            kickoff_partner: None,
            deliberate_kick_by: None,
            hands_released_by: None,
            aerial_contest_winner: None,
            restart_taker: None,
            restart_carrier: None,
            restart_mark: None,
            restart_set: false,
            goal_kick_long: false,
        };
        meta.update(field);
        meta
    }
}
