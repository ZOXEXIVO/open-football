//! Match-engine ball model, split by concern. The `Ball` struct lives
//! here together with the per-tick orchestrator (`update` / `update_light`)
//! and the simple state queries the rest of the engine reads. The
//! heavier domain passes are sibling modules:
//!
//! | Submodule       | Concern                                                      |
//! |-----------------|--------------------------------------------------------------|
//! | [`ownership`]   | Pass-target claims, deadlock resolution, stall safety nets, ball-ownership claim flow |
//! | [`interactions`]| Intercept / shot-block / shot-save resolution                |
//! | [`goal`]        | Goal / over-the-bar / wide-of-goal handling                  |
//! | [`motion`]      | Velocity integration, owner tracking, boundary inset         |
//! | [`stall`]       | Position-anchor stall detector + snapshot diagnostics        |

mod goal;
mod interactions;
mod motion;
mod ownership;
mod stall;

use crate::r#match::events::EventCollection;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer, PlayerSide};
use nalgebra::Vector3;
use std::collections::VecDeque;

pub struct Ball {
    pub start_position: Vector3<f32>,
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    pub center_field_position: f32,

    pub field_width: f32,
    pub field_height: f32,

    pub flags: BallFlags,

    pub previous_owner: Option<u32>,
    pub current_owner: Option<u32>,
    pub take_ball_notified_players: Vec<u32>,
    pub notification_cooldown: u32,
    pub notification_timeout: u32,
    pub last_boundary_position: Option<Vector3<f32>>,
    pub unowned_stopped_ticks: u32,
    pub ownership_duration: u32,
    pub claim_cooldown: u32,
    pub pass_target_player_id: Option<u32>,
    /// Passer id of the most-recent live pass. Set on pass emit,
    /// cleared on any opponent touch or when the pass's natural
    /// window (150 ticks ≈ 1.5 s) expires. The pass-completion stat
    /// uses this as the source of truth for "was this claim a pass
    /// reception?" — `pass_target_player_id` gets cleared in too
    /// many unrelated paths to serve that role. None outside an
    /// active pass window.
    pub pending_pass_passer: Option<u32>,
    pub pending_pass_set_tick: u64,
    pub recent_passers: VecDeque<u32>,
    pub contested_claim_count: u32,
    pub unowned_ticks: u32,
    /// Snapshot captured at the moment the ball became uncontrolled — ball
    /// kinematics plus every player's state/position/velocity. Held until
    /// the stall resolves, then attached to the resolution log (only if
    /// the stall was long enough to log). Provides the "what did the
    /// pitch look like when this got stuck" context in the same line as
    /// the duration. Cleared on ownership resume.
    pub stall_start_snapshot: Option<String>,
    pub goal_scored: bool,
    pub kickoff_team_side: Option<PlayerSide>,
    pub cached_landing_position: Vector3<f32>,
    /// When a set-piece (corner, goal kick) rewrites ownership to a
    /// specific player, the ball can only mutate itself here — player
    /// teleport requires &mut field.players which lives one layer up.
    /// Populated inside `check_wide_of_goal` and drained by the engine
    /// after `ball.update` returns, so the owner is on the ball before
    /// the next `move_to` distance check can null their ownership.
    pub pending_set_piece_teleport: Option<(u32, Vector3<f32>)>,
    /// Counter for "ball is owned but nothing is happening" stalls.
    /// The unowned-stall warning can't see these because ownership is
    /// set, but visually the ball sits with a player who isn't moving,
    /// isn't passing, isn't dribbling — same "ball stuck" symptom, no
    /// warning. Reset whenever owner changes or any meaningful motion
    /// resumes; fires a separate warning once it crosses the threshold.
    pub owned_stuck_ticks: u32,
    pub owned_stuck_logged: bool,
    /// Position-based stall detector — catches cases the owned/unowned
    /// counters miss, specifically: rapid ownership flipping keeps
    /// resetting both counters (each "change" looks like progress) but
    /// the ball physically never leaves a small region. We sample the
    /// ball's position every N ticks and if it hasn't moved more than
    /// a threshold distance over a window, it's stuck regardless of
    /// who "owns" it at any given instant.
    pub stall_anchor_pos: Vector3<f32>,
    pub stall_anchor_tick: u32,

    /// Trajectory projection cached at the moment a shot is fired. Lets
    /// the goalkeeper commit to an intercept line instead of re-chasing
    /// the ball's current position every tick (which lost ground vs a
    /// 5.6 u/tick shot). `None` whenever the ball isn't a shot in
    /// flight; cleared on catch, goal, or any ownership event.
    pub cached_shot_target: Option<ShotTarget>,
}

/// Projection of a shot at the moment it's taken. The `PreparingForSave`
/// and `Catching` goalkeeper states read this to know where the ball
/// will actually arrive rather than chasing its current position — a
/// diving keeper commits to a spot on the line, they don't track the
/// ball every frame.
#[derive(Debug, Clone, Copy)]
pub struct ShotTarget {
    /// y-coordinate at which the shot is projected to cross the goal
    /// line, in field units. Falls outside the posts if the shot is
    /// going wide — the keeper should still attempt the save, the
    /// post-vs-net check happens in `check_goal`.
    pub goal_line_y: f32,
    /// z-coordinate (height) at projected crossing. Above `GOAL_HEIGHT`
    /// (2.44) is an over-the-bar ball the keeper shouldn't commit to.
    pub goal_line_z: f32,
    /// Goal the ball is heading for — left (x=0) or right (x=field_w).
    /// Used so the correct keeper reads the cache.
    pub defending_side: PlayerSide,
}

#[derive(Default, Clone)]
pub struct BallFlags {
    pub in_flight_state: usize,
    pub running_for_ball: bool,
}

impl BallFlags {
    pub fn reset(&mut self) {
        self.in_flight_state = 0;
        self.running_for_ball = false;
    }
}

impl Ball {
    pub fn with_coord(field_width: f32, field_height: f32) -> Self {
        let x = field_width / 2.0;
        let y = field_height / 2.0;

        Ball {
            position: Vector3::new(x, y, 0.0),
            start_position: Vector3::new(x, y, 0.0),
            field_width,
            field_height,
            velocity: Vector3::zeros(),
            center_field_position: x, // initial ball position = center field
            flags: BallFlags::default(),
            previous_owner: None,
            current_owner: None,
            take_ball_notified_players: Vec::new(),
            notification_cooldown: 0,
            notification_timeout: 0,
            last_boundary_position: None,
            unowned_stopped_ticks: 0,
            ownership_duration: 0,
            claim_cooldown: 0,
            pass_target_player_id: None,
            pending_pass_passer: None,
            pending_pass_set_tick: 0,
            recent_passers: VecDeque::with_capacity(5),
            contested_claim_count: 0,
            unowned_ticks: 0,
            stall_start_snapshot: None,
            goal_scored: false,
            kickoff_team_side: None,
            cached_landing_position: Vector3::new(x, y, 0.0),
            pending_set_piece_teleport: None,
            owned_stuck_ticks: 0,
            owned_stuck_logged: false,
            stall_anchor_pos: Vector3::new(x, y, 0.0),
            stall_anchor_tick: 0,
            cached_shot_target: None,
        }
    }

    /// Update cached landing position. Call after physics changes position/velocity.
    #[inline]
    pub fn update_landing_cache(&mut self) {
        self.cached_landing_position = self.calculate_landing_position();
    }

    pub fn update(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        // Decrement claim cooldown
        if self.claim_cooldown > 0 {
            self.claim_cooldown -= 1;
        }

        self.update_velocity();

        self.try_intercept(players, events);
        self.try_block_shot(players, events);
        self.try_save_shot(context, players, events);
        self.try_notify_standing_ball(players, events);

        // NUCLEAR OPTION: Force claiming if ball unowned and stopped for too long
        self.force_claim_if_deadlock(players, events);

        // Unconditional unowned safety net - forces nearest players to TakeBall
        self.force_takeball_if_unowned_too_long(players, events);
        // `detect_owned_stuck` was too sensitive — it fired on legitimate
        // possession play (defender holding in back line for 6-12s is
        // normal). `detect_position_stall` is the stricter signal: ball
        // hasn't moved ANYWHERE in 1000 ticks, regardless of who owns
        // it. That's a real stall.
        self.detect_position_stall(players);

        self.process_ownership(context, players, events);

        // Move ball FIRST, then check goal/boundary on new position
        self.move_to(tick_context);
        self.check_goal(context, events);
        self.check_over_goal(context, players, events);
        self.check_wide_of_goal(context, players, events);
        self.check_boundary_collision(context);
        self.update_landing_cache();
    }

    /// Light update: full ball logic but reads owner position from players slice directly.
    pub fn update_light(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        if self.claim_cooldown > 0 {
            self.claim_cooldown -= 1;
        }

        self.update_velocity();
        self.try_intercept(players, events);
        self.try_block_shot(players, events);
        self.try_save_shot(context, players, events);
        self.process_ownership(context, players, events);

        // Move ball: find owner position from players slice directly
        self.move_to_with_players(players);
        self.check_goal(context, events);
        self.check_over_goal(context, players, events);
        self.check_wide_of_goal(context, players, events);
        self.check_boundary_collision(context);
        self.update_landing_cache();
    }

    /// Calculate where an aerial ball will land (when z reaches 0).
    /// Uses projectile motion: z(t) = h + vz·t − ½g·t² = 0, solving for
    /// the positive root. Ignores air drag — close enough for chase
    /// positioning, and erring long is better than erring short.
    ///
    /// Units are ticks, not seconds: position integration is
    /// `position += velocity` per tick (no dt scaling), while gravity
    /// applies `velocity.z += -GRAVITY * 0.016` per tick. So the
    /// effective per-tick² gravity is `9.81 * 0.016 ≈ 0.157`, and the
    /// resulting `time_to_ground` comes out in ticks — which matches
    /// the horizontal integration `x += vx` per tick.
    pub fn calculate_landing_position(&self) -> Vector3<f32> {
        if self.position.z <= 0.1 || self.current_owner.is_some() {
            return self.position;
        }

        const G_PER_TICK: f32 = 9.81 * 0.016;
        let vz = self.velocity.z;
        let h = self.position.z;

        // Positive root of ½g·t² − vz·t − h = 0
        let discriminant = vz * vz + 2.0 * G_PER_TICK * h;
        let time_to_ground = (vz + discriminant.sqrt()) / G_PER_TICK;

        let landing_x = self.position.x + self.velocity.x * time_to_ground;
        let landing_y = self.position.y + self.velocity.y * time_to_ground;

        let clamped_x = landing_x.clamp(0.0, self.field_width);
        let clamped_y = landing_y.clamp(0.0, self.field_height);

        Vector3::new(clamped_x, clamped_y, 0.0)
    }

    /// Check if the ball is aerial (in the air above player reach)
    pub fn is_aerial(&self) -> bool {
        const PLAYER_REACH_HEIGHT: f32 = 2.3;
        self.position.z > PLAYER_REACH_HEIGHT && self.velocity.z.abs() > 0.1
    }

    pub fn is_stands_outside(&self) -> bool {
        self.is_ball_outside()
            && self.velocity.norm_squared() < 0.25 // 0.5^2, allow tiny velocities from physics
            && self.current_owner.is_none()
    }

    pub fn is_ball_stopped_on_field(&self) -> bool {
        !self.is_ball_outside()
            && self.velocity.norm_squared() < 6.25 // 2.5^2, catch slow rolling balls that need claiming
            && self.current_owner.is_none()
    }

    pub fn is_ball_outside(&self) -> bool {
        self.position.x <= 0.0
            || self.position.x >= self.field_width
            || self.position.y <= 0.0
            || self.position.y >= self.field_height
    }

    /// Lightweight movement: just apply velocity to position (no ownership logic)
    pub fn apply_movement(&mut self) {
        self.position.x += self.velocity.x;
        self.position.y += self.velocity.y;
        self.position.z += self.velocity.z;
        if self.position.z < 0.0 {
            self.position.z = 0.0;
        }
    }

    pub fn reset(&mut self) {
        self.position.x = self.start_position.x;
        self.position.y = self.start_position.y;
        self.position.z = 0.0;

        self.velocity = Vector3::zeros();

        self.current_owner = None;
        self.previous_owner = None;
        self.ownership_duration = 0;
        self.claim_cooldown = 0;

        self.flags.reset();
        self.pass_target_player_id = None;
        self.clear_pass_history();
        self.contested_claim_count = 0;
        self.unowned_ticks = 0;
        self.cached_landing_position = self.position;
        self.pending_set_piece_teleport = None;
        self.owned_stuck_ticks = 0;
        self.owned_stuck_logged = false;
        self.stall_anchor_pos = self.position;
        self.stall_anchor_tick = 0;
        self.cached_shot_target = None;
    }

    pub fn clear_player_reference(&mut self, player_id: u32) {
        if self.current_owner == Some(player_id) {
            self.current_owner = None;
            self.ownership_duration = 0;
        }
        if self.previous_owner == Some(player_id) {
            self.previous_owner = None;
        }
        if self.pass_target_player_id == Some(player_id) {
            self.pass_target_player_id = None;
        }
        self.take_ball_notified_players
            .retain(|&id| id != player_id);
        self.recent_passers.retain(|&id| id != player_id);
    }

    /// Record a passer in the recent passers ring buffer.
    /// Skips consecutive duplicates and caps at 5 entries.
    pub fn record_passer(&mut self, passer_id: u32) {
        // Skip consecutive duplicates
        if self.recent_passers.back() == Some(&passer_id) {
            return;
        }
        if self.recent_passers.len() >= 5 {
            self.recent_passers.pop_front();
        }
        self.recent_passers.push_back(passer_id);
    }

    /// Clear the recent passers history (e.g. on tackles, interceptions, clearances).
    pub fn clear_pass_history(&mut self) {
        self.recent_passers.clear();
    }
}
