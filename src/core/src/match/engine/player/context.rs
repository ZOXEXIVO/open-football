use crate::r#match::{MatchField, MatchObjectsPositions, ShotTarget, Space, SpatialGrid};

pub struct GameTickContext {
    pub positions: MatchObjectsPositions,
    pub grid: SpatialGrid,
    pub ball: BallMetadata,
    pub space: Space,
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
        }
    }

    #[inline]
    pub fn update(&mut self, field: &MatchField) {
        self.ball.update(field);
        self.positions.update(field);
        self.grid.update(field);
        self.space.update(field);
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
        };
        meta.update(field);
        meta
    }
}
