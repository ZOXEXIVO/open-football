use crate::club::player::traits::PlayerTrait;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::tactics::TacticalPositions;
use crate::r#match::events::EventCollection;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::memory::PlayerMemory;
use crate::r#match::player::state::{PlayerMatchState, PlayerState};
use crate::r#match::player::statistics::MatchPlayerStatistics;
use crate::r#match::player::waypoints::WaypointManager;
use crate::r#match::{GameTickContext, MatchContext, StateProcessingContext};
use crate::utils::DateUtils;
use crate::{
    PersonAttributes, Player, PlayerAttributes, PlayerFieldPositionGroup, PlayerPositionType,
    PlayerSkills,
};
use chrono::NaiveDate;
use nalgebra::Vector3;
use std::fmt::*;

#[cfg(debug_assertions)]
use log::debug;

#[derive(Debug, Clone)]
pub struct MatchPlayer {
    pub id: u32,
    pub position: Vector3<f32>,
    pub start_position: Vector3<f32>,
    pub attributes: PersonAttributes,
    pub team_id: u32,
    pub player_attributes: PlayerAttributes,
    pub skills: PlayerSkills,
    pub tactical_position: TacticalPositions,
    pub velocity: Vector3<f32>,
    pub side: Option<PlayerSide>,
    pub state: PlayerState,
    pub in_state_time: u64,
    pub statistics: MatchPlayerStatistics,
    pub use_extended_state_logging: bool,

    pub waypoint_manager: WaypointManager,

    pub memory: PlayerMemory,

    /// Accumulates fractional condition changes across ticks
    pub fatigue_accumulator: f32,

    /// Cached waypoint vectors (only changes on substitution/half-time swap)
    cached_waypoints: Vec<Vector3<f32>>,

    /// Signature moves (PPMs) — read by decision helpers to bias behaviour.
    pub traits: Vec<PlayerTrait>,

    /// Yellow cards accumulated in this match. 2 → red.
    pub yellow_cards: u8,
    /// Fouls committed in this match. Feeds end-of-match stats.
    pub fouls_committed: u8,
    /// Player has been sent off — skip state processing, treat as off field.
    pub is_sent_off: bool,
    /// Ticks remaining before this player may attempt another tackle.
    /// Decremented each tick in `update()`. Blocks Tackling-state entry
    /// via `can_attempt_tackle()`. Prevents the Tackling-state machine
    /// from re-firing attempts via Standing/Running/Covering/etc. paths
    /// within the same second, which would otherwise produce 40+ fouls
    /// per team in the first 5 minutes of every match.
    pub tackle_cooldown: u16,
    /// Tagged reason for the next Shoot event. Set by each transition
    /// point that routes into the Shooting state (e.g. "FWD_RUN_PRIO05",
    /// "FWD_POINT_BLANK", "MID_POINT_BLANK_RUN"). The Shooting state
    /// reads this and attaches it to the emitted Shoot event so the
    /// per-match shot-reason log shows which code path fired the shot.
    /// Cleared after consumption.
    pub pending_shot_reason: Option<&'static str>,

    /// Manager flag protecting this player from fatigue / development subs.
    /// Mirrored from `Player::is_force_match_selection` at squad-build time.
    pub is_force_match_selection: bool,

    /// Player's birth date, mirrored from the source `Player`. Read by the
    /// in-match substitution logic to apply age-appropriate protection
    /// thresholds for under-18 players (lower fatigue ceiling, condition
    /// floor that overrides the manager's force-selection flag).
    pub birth_date: NaiveDate,
}

impl MatchPlayer {
    /// Age in whole years on `today`. Defined here (not on `Player`) so
    /// match-side code never has to reach back through the simulator
    /// data graph.
    #[inline]
    pub fn age_at(&self, today: NaiveDate) -> u8 {
        DateUtils::age(self.birth_date, today)
    }
}

impl MatchPlayer {
    /// Fast trait lookup used inside hot decision paths.
    #[inline]
    pub fn has_trait(&self, t: PlayerTrait) -> bool {
        self.traits.iter().any(|x| *x == t)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerSide {
    Left,
    Right,
}

impl PlayerSide {
    /// Forward direction along the pitch's x-axis: +1 toward larger x
    /// for Left teams, -1 for Right. The math everywhere else in the
    /// engine uses this — never write `match side { Left => 1.0, ... }`
    /// inline or you'll inevitably miss a sign and produce the
    /// "right-side never reaches attacking third" bug we hit before.
    #[inline]
    pub fn forward_dir_x(self) -> f32 {
        match self {
            PlayerSide::Left => 1.0,
            PlayerSide::Right => -1.0,
        }
    }

    /// Attacking progress for an x-coordinate, normalised to [0.0, 1.0]:
    ///   0.0 = own goal line, 1.0 = opponent goal line.
    /// Use this everywhere a "what fraction of the pitch have we
    /// covered" check is needed (attacking third, defensive third,
    /// halfway thresholds). Replaces the legacy
    ///   `(x * forward_dir) / field_width`
    /// formula, which produced NEGATIVE values for right-side teams —
    /// that broke `> 0.66` (attacking third) tests by construction
    /// while leaving `< 0.33` (defensive third) accidentally correct.
    #[inline]
    pub fn attacking_progress_x(self, x: f32, field_width: f32) -> f32 {
        if field_width <= 0.0 {
            return 0.0;
        }
        let raw = match self {
            PlayerSide::Left => x / field_width,
            PlayerSide::Right => (field_width - x) / field_width,
        };
        raw.clamp(0.0, 1.0)
    }

    /// Forward delta along x: positive = toward opponent goal, negative
    /// = toward own goal. The right shape for "is this pass forward?",
    /// "did this player advance?", and any signed forward-progress
    /// arithmetic. Side-direction is baked in.
    #[inline]
    pub fn forward_delta(self, from_x: f32, to_x: f32) -> f32 {
        (to_x - from_x) * self.forward_dir_x()
    }

    /// Same as `forward_delta` but normalised by field width — gives a
    /// signed [-1.0, 1.0]-ish ratio for the cases that previously used
    /// `((to.x - from.x) * dir) / field_width`.
    #[inline]
    pub fn forward_delta_norm(self, from_x: f32, to_x: f32, field_width: f32) -> f32 {
        if field_width <= 0.0 {
            return 0.0;
        }
        self.forward_delta(from_x, to_x) / field_width
    }
}

impl MatchPlayer {
    pub fn from_player(
        team_id: u32,
        player: &Player,
        position: PlayerPositionType,
        use_extended_state_logging: bool,
    ) -> Self {
        MatchPlayer {
            id: player.id,
            position: Vector3::zeros(),
            start_position: Vector3::zeros(),
            attributes: player.attributes,
            team_id,
            player_attributes: player.player_attributes,
            skills: player.skills,
            velocity: Vector3::zeros(),
            tactical_position: TacticalPositions::new(position, None),
            side: None,
            state: Self::default_state(position),
            in_state_time: 0,
            statistics: MatchPlayerStatistics::new(),
            waypoint_manager: WaypointManager::new(),
            use_extended_state_logging,
            memory: PlayerMemory::new(),
            fatigue_accumulator: 0.0,
            cached_waypoints: Vec::new(),
            traits: player.traits.clone(),
            yellow_cards: 0,
            fouls_committed: 0,
            is_sent_off: false,
            tackle_cooldown: 0,
            pending_shot_reason: None,
            is_force_match_selection: player.is_force_match_selection,
            birth_date: player.birth_date,
        }
    }

    /// Consumes the tackle cooldown (ticks it down by 1). Called once per
    /// simulation tick from `update()`.
    #[inline]
    pub fn tick_tackle_cooldown(&mut self) {
        self.tackle_cooldown = self.tackle_cooldown.saturating_sub(1);
    }

    /// Can this player currently attempt a sliding tackle? False while the
    /// post-attempt cooldown is still counting down — regardless of which
    /// state routed them into Tackling.
    #[inline]
    pub fn can_attempt_tackle(&self) -> bool {
        self.tackle_cooldown == 0
    }

    /// Start the post-tackle cooldown. Called right after any attempt
    /// resolves (success, miss, or foul).
    #[inline]
    pub fn start_tackle_cooldown(&mut self) {
        // 3000 ticks ≈ 30 seconds. Real football: a player contests 2-4
        // tackles per 90 minutes — one every ~25 minutes. The previous
        // 15-second cooldown still let attempts run at 205/team/match
        // (5x real) because 10 outfield players × 15s allowed up to one
        // attempt per second team-wide. 30s halves the team-wide ceiling
        // and matches the realistic "commit, resolve, regroup,
        // reposition" cadence — a defender who lunges and either wins,
        // misses, or fouls is realistically out of the next play for
        // half a minute, not 15 seconds.
        self.tackle_cooldown = 3000;
    }

    pub fn rebuild_waypoint_cache(&mut self) {
        self.cached_waypoints = self
            .tactical_position
            .tactical_positions
            .iter()
            .filter(|tp| tp.position == self.tactical_position.current_position)
            .flat_map(|tp| &tp.waypoints)
            .map(|(x, y)| Vector3::new(*x, *y, 0.0))
            .collect();
    }

    pub fn update(
        &mut self,
        context: &MatchContext,
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        self.tick_tackle_cooldown();

        let player_events = PlayerMatchState::process(self, context, tick_context);

        events.add_from_collection(player_events);

        self.update_waypoint_index(tick_context);

        self.check_boundary_collision(context);
        self.move_to();
    }

    /// Movement-only update: apply existing velocity without re-evaluating AI state.
    #[inline]
    pub fn update_movement_only(&mut self, context: &MatchContext) {
        self.in_state_time += 1;
        self.check_boundary_collision(context);
        self.move_to();
    }

    #[inline]
    pub fn check_boundary_collision(&mut self, context: &MatchContext) {
        let field_width = context.field_size.width as f32 + 1.0;
        let field_height = context.field_size.height as f32 + 1.0;

        // Clamp position to field boundaries
        self.position.x = self.position.x.clamp(0.0, field_width);
        self.position.y = self.position.y.clamp(0.0, field_height);

        // Only stop velocity if player is trying to move OUT of bounds
        // Allow velocity that moves them back into the field
        if self.position.x <= 0.0 && self.velocity.x < 0.0 {
            // At left boundary, trying to move further left
            self.velocity.x = 0.0;
        } else if self.position.x >= field_width && self.velocity.x > 0.0 {
            // At right boundary, trying to move further right
            self.velocity.x = 0.0;
        }

        if self.position.y <= 0.0 && self.velocity.y < 0.0 {
            // At bottom boundary, trying to move further down
            self.velocity.y = 0.0;
        } else if self.position.y >= field_height && self.velocity.y > 0.0 {
            // At top boundary, trying to move further up
            self.velocity.y = 0.0;
        }
    }

    pub fn set_default_state(&mut self) {
        self.state = Self::default_state(self.tactical_position.current_position);
        self.rebuild_waypoint_cache();
    }

    fn default_state(position: PlayerPositionType) -> PlayerState {
        match position.position_group() {
            PlayerFieldPositionGroup::Goalkeeper => {
                PlayerState::Goalkeeper(GoalkeeperState::Standing)
            }
            PlayerFieldPositionGroup::Defender => PlayerState::Defender(DefenderState::Standing),
            PlayerFieldPositionGroup::Midfielder => {
                PlayerState::Midfielder(MidfielderState::Standing)
            }
            PlayerFieldPositionGroup::Forward => PlayerState::Forward(ForwardState::Standing),
        }
    }

    pub fn run_for_ball(&mut self) {
        self.state = match self.tactical_position.current_position.position_group() {
            PlayerFieldPositionGroup::Goalkeeper => {
                PlayerState::Goalkeeper(GoalkeeperState::TakeBall)
            }
            PlayerFieldPositionGroup::Defender => PlayerState::Defender(DefenderState::TakeBall),
            PlayerFieldPositionGroup::Midfielder => {
                PlayerState::Midfielder(MidfielderState::TakeBall)
            }
            PlayerFieldPositionGroup::Forward => PlayerState::Forward(ForwardState::TakeBall),
        }
    }

    #[inline]
    pub fn move_to(&mut self) {
        #[cfg(debug_assertions)]
        let old_position = self.position;

        // Apply velocity only if finite. `is_finite` rules out both NaN
        // and ±Infinity — either poisons the position, and a corrupt
        // position is excluded from the viewer recording so the player
        // literally disappears mid-match.
        if self.velocity.x.is_finite() {
            self.position.x += self.velocity.x;
        }

        if self.velocity.y.is_finite() {
            self.position.y += self.velocity.y;
        }

        // Last-resort salvage: if position is already corrupt from an
        // earlier tick (before the velocity guard was in place, or from
        // external code paths), reset to the player's tactical start
        // position. The player briefly teleports rather than vanishing.
        if !self.position.x.is_finite() || !self.position.y.is_finite() {
            self.position = self.start_position;
            self.velocity = nalgebra::Vector3::zeros();
        }

        #[cfg(debug_assertions)]
        {
            // Check for abnormally large position changes
            let position_delta = self.position - old_position;
            let position_change = position_delta.norm();

            const MAX_REASONABLE_POSITION_CHANGE: f32 = 20.0;

            if position_change > MAX_REASONABLE_POSITION_CHANGE {
                debug!(
                    "Player {:?} position jumped abnormally! {} from: ({:.2}, {:.2}) to: ({:.2}, {:.2}), delta: ({:.2}, {:.2}), distance: {:.2}, velocity: ({:.2}, {:.2})",
                    self.state,
                    self.id,
                    old_position.x,
                    old_position.y,
                    self.position.x,
                    self.position.y,
                    position_delta.x,
                    position_delta.y,
                    position_change,
                    self.velocity.x,
                    self.velocity.y
                );
            }
        }
    }

    pub fn heading(&self) -> f32 {
        self.velocity.y.atan2(self.velocity.x)
    }

    pub fn has_ball(&self, ctx: &StateProcessingContext<'_>) -> bool {
        ctx.ball().owner_id() == Some(self.id)
    }

    pub fn update_waypoint_index(&mut self, tick_context: &GameTickContext) {
        if self.cached_waypoints.is_empty() {
            self.rebuild_waypoint_cache();
        }
        self.waypoint_manager.update(
            &tick_context.positions.players.position(self.id),
            &self.cached_waypoints,
        );
    }

    pub fn get_waypoints_as_vectors(&self) -> &[Vector3<f32>] {
        &self.cached_waypoints
    }

    pub fn should_follow_waypoints(&self, ctx: &StateProcessingContext) -> bool {
        // Ball carrier doesn't follow waypoints — they move freely
        if self.has_ball(ctx) {
            return false;
        }

        // Best chaser pursues the ball, not waypoints
        if !ctx.ball().is_owned() && ctx.team().is_best_player_to_chase_ball() {
            return false;
        }

        // If any teammate is too close (< 12u, the natural "shoulder-
        // to-shoulder" bunching distance), follow waypoints back to
        // formation. This is the anti-grouping reinforcement: the
        // moment two of our players are crammed into one yard, one
        // of them peels off to their assigned tactical position. The
        // shorter of them (by id) peels, to avoid both trying to move
        // simultaneously.
        let me_id = self.id;
        let me_pos = self.position;
        let teammate_crowding = ctx.players().teammates().all().any(|t| {
            if t.id == me_id {
                return false;
            }
            let d_sq = (t.position - me_pos).norm_squared();
            if d_sq >= 144.0 {
                return false;
            } // 12² = 144
            // Only one of the pair peels (the lower id). Keeps the
            // behaviour deterministic per-tick and avoids both
            // leaving their post simultaneously.
            t.id > me_id
        });
        if teammate_crowding {
            return true;
        }

        // Everyone else follows waypoints to maintain tactical shape
        // Waypoints represent position-specific movement patterns that keep
        // formation spread and prevent clustering
        true
    }
}

#[derive(Copy, Clone)]
pub struct MatchPlayerLite {
    pub id: u32,
    pub position: Vector3<f32>,
    pub tactical_positions: PlayerPositionType,
}

impl MatchPlayerLite {
    pub fn has_ball(&self, ctx: &StateProcessingContext<'_>) -> bool {
        ctx.ball().owner_id() == Some(self.id)
    }

    pub fn velocity(&self, ctx: &StateProcessingContext<'_>) -> Vector3<f32> {
        ctx.tick_context.positions.players.velocity(self.id)
    }

    pub fn distance(&self, ctx: &StateProcessingContext<'_>) -> f32 {
        ctx.tick_context.grid.get(self.id, ctx.player.id)
    }
}

impl From<&MatchPlayer> for MatchPlayerLite {
    fn from(player: &MatchPlayer) -> Self {
        MatchPlayerLite {
            id: player.id,
            position: player.position,
            tactical_positions: player.tactical_position.current_position,
        }
    }
}
