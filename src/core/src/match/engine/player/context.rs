use crate::r#match::player::strategies::players::DefensiveRole;
use crate::r#match::{
    MatchField, MatchObjectsPositions, PassOriginRestart, ShotTarget, Space, SpatialGrid,
};
use std::cell::RefCell;

pub struct GameTickContext {
    pub positions: MatchObjectsPositions,
    pub grid: SpatialGrid,
    pub ball: BallMetadata,
    pub space: Space,
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
    pub defensive_role: Option<DefensiveRole>,
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
            defensive_role: None,
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
            self.defensive_role = None;
        }
        self
    }
}

impl GameTickContext {
    pub fn new(field: &MatchField) -> Self {
        let mut grid = SpatialGrid::new();
        grid.update(field);
        GameTickContext {
            ball: BallMetadata::from(field),
            positions: MatchObjectsPositions::from(field),
            grid,
            space: Space::from(field),
            player_agg_cache: RefCell::new(PlayerTickCache::new()),
        }
    }

    #[inline]
    pub fn update(&mut self, field: &MatchField) {
        self.ball.update(field);
        self.positions.update(field);
        self.grid.update(field);
        self.space.update(field);
    }

    /// Cheaper refresh used during shot-flight light ticks where only
    /// the two goalkeepers run AI. Skips `Space` (raycast / pass-line
    /// scratchpad) because GK strategies don't read it — they react off
    /// `BallMetadata::cached_shot_target` and live positions. Keeps
    /// ball + player positions + spatial grid in sync so the keeper's
    /// distance-to-ball and chase decisions stay correct.
    #[inline]
    pub fn update_for_goalkeeper_shot(&mut self, field: &MatchField) {
        self.ball.update(field);
        self.positions.update(field);
        self.grid.update(field);
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
