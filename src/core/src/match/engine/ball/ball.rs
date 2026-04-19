use std::collections::VecDeque;
use crate::r#match::ball::events::{BallEvent, BallGoalEventMetadata, GoalSide};
use crate::r#match::events::EventCollection;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer, PlayerSide};
use nalgebra::Vector3;

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

    pub fn process_ownership(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        if self.flags.in_flight_state > 0 {
            self.flags.in_flight_state -= 1;
            // Allow pass target to claim during flight
            self.try_pass_target_claim(players, events);
        } else {
            self.check_ball_ownership(context, players, events);
        }

        self.flags.running_for_ball = self.is_players_running_to_ball(players);
    }

    fn try_pass_target_claim(
        &mut self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // Check if pass target can claim the ball
        if let Some(target_id) = self.pass_target_player_id {
            if let Some(target_player) = players.iter().find(|p| p.id == target_id) {
                // Use cached landing position for aerial balls, current position for ground balls
                let effective_ball_pos = if self.is_aerial() {
                    self.cached_landing_position
                } else {
                    self.position
                };

                let dx = target_player.position.x - effective_ball_pos.x;
                let dy = target_player.position.y - effective_ball_pos.y;
                let dist_sq = dx * dx + dy * dy;

                // Generous claim radius for intended receiver (3.5m vs normal 2.0m)
                const RECEIVER_CLAIM_DISTANCE_SQ: f32 = 3.5 * 3.5;
                const RECEIVER_MAX_HEIGHT: f32 = 2.8;

                if dist_sq < RECEIVER_CLAIM_DISTANCE_SQ && self.position.z <= RECEIVER_MAX_HEIGHT {
                    self.current_owner = Some(target_id);
                    self.pass_target_player_id = None;
                    self.ownership_duration = 0;
                    self.flags.in_flight_state = 0;
                    self.claim_cooldown = 15;
                    events.add_ball_event(BallEvent::Claimed(target_id));
                    return;
                }
            }
        }

        // Also allow previous owner (passer) to reclaim if ball bounced back
        // BUT only after the ball has had time to travel away (in_flight_state < 10)
        // This prevents the passer from immediately reclaiming on low-force passes
        if self.flags.in_flight_state < 10 {
            if let Some(prev_id) = self.previous_owner {
                if let Some(prev_player) = players.iter().find(|p| p.id == prev_id) {
                    let dx = prev_player.position.x - self.position.x;
                    let dy = prev_player.position.y - self.position.y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist < 2.0 && self.position.z <= 2.8 {
                        // Check ball is moving toward passer (bounced back)
                        let ball_speed = self.velocity.norm();
                        if ball_speed > 0.1 {
                            let to_passer_x = dx / dist;
                            let to_passer_y = dy / dist;
                            let dot = (self.velocity.x / ball_speed) * to_passer_x
                                + (self.velocity.y / ball_speed) * to_passer_y;
                            if dot > 0.3 {
                                // Ball moving toward passer
                                self.current_owner = Some(prev_id);
                                self.pass_target_player_id = None;
                                self.ownership_duration = 0;
                                self.flags.in_flight_state = 0;
                                self.claim_cooldown = 15;
                                events.add_ball_event(BallEvent::Claimed(prev_id));
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn try_notify_standing_ball(
        &mut self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // Don't treat ball as "standing" during in-flight (just passed)
        // Short passes have low velocity that triggers is_ball_stopped_on_field(),
        // but the ball is still in transit to the intended receiver
        if self.flags.in_flight_state > 0 {
            return;
        }

        // Decrement cooldown timer
        if self.notification_cooldown > 0 {
            self.notification_cooldown -= 1;
        }

        // Check if ball is stopped (either outside or inside field) and no one owns it
        let is_ball_stopped = self.is_stands_outside() || self.is_ball_stopped_on_field();

        // Check if ball has moved significantly from last boundary position
        let has_escaped_boundary = if let Some(last_pos) = self.last_boundary_position {
            let dist_sq = (self.position - last_pos).norm_squared();
            dist_sq > 4.0 // 2.0^2
        } else {
            true // No previous boundary position recorded
        };

        if (is_ball_stopped)
            && self.take_ball_notified_players.is_empty()
            && self.current_owner.is_none()
            && self.notification_cooldown == 0 // Only notify if cooldown expired
            && has_escaped_boundary // Only notify if ball escaped from previous boundary loop
        {
            let notified_players = self.notify_nearest_player(players, events);
            if !notified_players.is_empty() {
                self.take_ball_notified_players = notified_players;
                self.notification_timeout = 0; // Reset timeout when new players are notified

                // If ball is at boundary, set short cooldown and record position
                if self.is_ball_outside() {
                    self.notification_cooldown = 5; // Short cooldown to prevent spam
                    self.last_boundary_position = Some(self.position);
                }
            }
        } else if !self.take_ball_notified_players.is_empty() {
            // Increment timeout counter
            self.notification_timeout += 1;

            // If players haven't claimed the ball within reasonable time, reset and try again
            const MAX_NOTIFICATION_TIMEOUT: u32 = 60; // ~1 second - reduced from 200 for faster response
            if self.notification_timeout > MAX_NOTIFICATION_TIMEOUT {
                self.take_ball_notified_players.clear();
                self.notification_timeout = 0;
                self.notification_cooldown = 0; // Clear cooldown to allow immediate re-notification
                // Clear boundary position to allow re-notification even if ball hasn't moved
                self.last_boundary_position = None;
                return; // Will re-notify on next tick
            }
            // Check if any notified player reached the ball
            const CLAIM_DISTANCE: f32 = 5.0; // Claim distance for notified players (generous to avoid corner deadlocks)
            const MAX_CLAIM_VELOCITY: f32 = 5.0; // Ball must be slow enough to claim

            let target_position = if self.is_aerial() {
                self.cached_landing_position
            } else {
                self.position
            };

            let ball_speed = self.velocity.norm();
            let can_claim_by_speed = ball_speed < MAX_CLAIM_VELOCITY;

            let mut claiming_player_id: Option<u32> = None;
            let mut all_players_missing = true;

            for notified_player_id in &self.take_ball_notified_players {
                if let Some(player) = players.iter().find(|p| p.id == *notified_player_id) {
                    all_players_missing = false;

                    let dx = player.position.x - target_position.x;
                    let dy = player.position.y - target_position.y;
                    let dz = target_position.z;
                    let distance_3d = (dx * dx + dy * dy + dz * dz).sqrt();

                    // Simple distance check - if close enough and ball is slow, claim it
                    if distance_3d < CLAIM_DISTANCE && self.current_owner.is_none() && can_claim_by_speed {
                        if !self.is_aerial() || self.position.z < 2.5 {
                            claiming_player_id = Some(*notified_player_id);
                            break;
                        }
                    }
                }
            }

            // If all notified players are missing from the players slice, clear the list
            // This can happen if players were substituted or if there's a data inconsistency
            if all_players_missing {
                self.take_ball_notified_players.clear();
            }

            // Process the claim after iteration to avoid borrow checker issues
            if let Some(player_id) = claiming_player_id {
                self.current_owner = Some(player_id);
                self.pass_target_player_id = None;
                self.take_ball_notified_players.clear();
                self.notification_timeout = 0;
                events.add_ball_event(BallEvent::Claimed(player_id));

                // Reset boundary tracking when ball is claimed
                if has_escaped_boundary {
                    self.last_boundary_position = None;
                }
            }
        }
    }

    /// Opposing players near the ball's flight path can intercept passes.
    /// Interception chance depends on tackling, anticipation, positioning skills
    /// and proximity to the ball's trajectory.
    pub fn try_intercept(&mut self, players: &[MatchPlayer], events: &mut EventCollection) {
        // Only intercept unowned balls that are in flight (active pass)
        if self.current_owner.is_some() || self.flags.in_flight_state == 0 {
            return;
        }

        // Don't intercept aerial balls above player reach
        if self.position.z > 2.5 {
            return;
        }

        // Need to know who passed to determine the opposing team
        let passer_team = match self.previous_owner {
            Some(prev_id) => players.iter().find(|p| p.id == prev_id).map(|p| p.team_id),
            None => return,
        };
        let passer_team = match passer_team {
            Some(t) => t,
            None => return,
        };

        // Ball velocity determines the interception corridor width
        let ball_speed_sq = self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y;
        if ball_speed_sq < 1.0 {
            return; // Ball too slow, normal claiming handles it
        }

        // Interception corridor: how close a player must be to the ball's path
        const INTERCEPT_RADIUS: f32 = 2.5;
        const INTERCEPT_RADIUS_SQ: f32 = INTERCEPT_RADIUS * INTERCEPT_RADIUS;

        let mut best_interceptor: Option<u32> = None;
        let mut best_chance: f32 = 0.0;

        for player in players {
            // Only opposing team players can intercept
            if player.team_id == passer_team {
                continue;
            }

            // Don't let the pass target's team intercept their own pass target
            if Some(player.id) == self.pass_target_player_id {
                continue;
            }

            // Distance from player to ball
            let dx = player.position.x - self.position.x;
            let dy = player.position.y - self.position.y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq > INTERCEPT_RADIUS_SQ {
                continue;
            }

            // Calculate interception probability from player skills
            let tackling = player.skills.technical.tackling;
            let anticipation = player.skills.mental.anticipation;
            let positioning = player.skills.mental.positioning;
            let concentration = player.skills.mental.concentration;

            // Base chance: average of key defensive skills (0-20 scale → 0-1)
            let skill_factor = (tackling + anticipation + positioning + concentration) / (4.0 * 20.0);

            // Proximity factor: closer = higher chance (1.0 at 0m, 0.3 at max radius)
            let dist = dist_sq.sqrt();
            let proximity_factor = 1.0 - (dist / INTERCEPT_RADIUS) * 0.7;

            // Fast passes are harder to intercept
            let speed_penalty = 1.0 / (1.0 + ball_speed_sq.sqrt() * 0.02);

            // Final interception chance (very low per tick — happens across many ticks)
            let chance = skill_factor * proximity_factor * speed_penalty * 0.08;

            if chance > best_chance {
                best_chance = chance;
                best_interceptor = Some(player.id);
            }
        }

        // Deterministic threshold: only intercept if chance exceeds threshold
        // This avoids needing RNG in the match engine
        if best_chance > 0.04 {
            if let Some(interceptor_id) = best_interceptor {
                self.current_owner = Some(interceptor_id);
                self.pass_target_player_id = None;
                self.flags.in_flight_state = 0;
                self.claim_cooldown = 15;
                self.velocity *= 0.3; // Ball slows down on interception
                events.add_ball_event(BallEvent::Intercepted(interceptor_id, self.previous_owner));
            }
        }
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

    /// Deadlock resolution: Force the nearest player to claim the ball if it's been sitting unowned for too long
    /// Uses progressive radius - starts strict, expands if stuck to ensure game never deadlocks
    fn force_claim_if_deadlock(
        &mut self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        const DEADLOCK_VELOCITY_ENTER: f32 = 3.0;
        const DEADLOCK_VELOCITY_EXIT: f32 = 4.0;
        const DEADLOCK_HEIGHT_THRESHOLD: f32 = 1.5;

        // Progressive timing thresholds — faster initial response prevents corner deadlocks
        const TICK_PHASE_1: u32 = 15;   // ~0.25s - try close range quickly
        const TICK_PHASE_2: u32 = 35;   // ~0.6s - expand range
        const TICK_PHASE_3: u32 = 60;   // ~1.0s - further expand
        const TICK_PHASE_4: u32 = 100;  // ~1.6s - last resort

        // Progressive claim distances — generous to handle boundary/corner situations
        const CLAIM_DISTANCE_PHASE_1: f32 = 5.0;   // Close range - matches notification claim distance
        const CLAIM_DISTANCE_PHASE_2: f32 = 8.0;   // Medium range - acceptable
        const CLAIM_DISTANCE_PHASE_3: f32 = 12.0;  // Extended range - noticeable but not terrible
        const CLAIM_DISTANCE_PHASE_4: f32 = 15.0;  // Last resort - better than stuck forever

        let is_unowned = self.current_owner.is_none();

        if !is_unowned {
            self.unowned_stopped_ticks = 0;
            return;
        }

        // Don't interfere with passed/kicked balls
        if self.flags.in_flight_state > 0 {
            self.unowned_stopped_ticks = 0;
            return;
        }

        let velocity_threshold = if self.unowned_stopped_ticks > 0 {
            DEADLOCK_VELOCITY_EXIT
        } else {
            DEADLOCK_VELOCITY_ENTER
        };

        let velocity_sq = self.velocity.norm_squared();
        let threshold_sq = velocity_threshold * velocity_threshold;
        let is_slow = velocity_sq < threshold_sq;
        let is_low = self.position.z < DEADLOCK_HEIGHT_THRESHOLD;

        if is_slow && is_low {
            self.unowned_stopped_ticks += 1;

            // Determine claim distance based on how long we've been waiting
            let (should_claim, claim_distance) = if self.unowned_stopped_ticks >= TICK_PHASE_4 {
                (true, CLAIM_DISTANCE_PHASE_4)
            } else if self.unowned_stopped_ticks >= TICK_PHASE_3 {
                (true, CLAIM_DISTANCE_PHASE_3)
            } else if self.unowned_stopped_ticks >= TICK_PHASE_2 {
                (true, CLAIM_DISTANCE_PHASE_2)
            } else if self.unowned_stopped_ticks >= TICK_PHASE_1 {
                (true, CLAIM_DISTANCE_PHASE_1)
            } else {
                (false, 0.0)
            };

            if should_claim {
                // Find nearest player within current claim distance (use squared to avoid sqrt)
                let claim_distance_sq = claim_distance * claim_distance;
                if let Some(nearest_player) = players.iter()
                    .filter_map(|p| {
                        let dx = p.position.x - self.position.x;
                        let dy = p.position.y - self.position.y;
                        let dist_sq = dx * dx + dy * dy;
                        if dist_sq <= claim_distance_sq {
                            Some((p, dist_sq))
                        } else {
                            None
                        }
                    })
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                    .map(|(p, _)| p)
                {
                    // Grant ownership
                    self.current_owner = Some(nearest_player.id);
                    self.previous_owner = None;
                    self.pass_target_player_id = None;
                    self.ownership_duration = 0;
                    self.flags.in_flight_state = 0;
                    self.take_ball_notified_players.clear();
                    self.notification_timeout = 0;
                    self.claim_cooldown = 15; // Prevent immediate re-claiming by another player

                    if self.position.z > 0.1 && self.position.z < DEADLOCK_HEIGHT_THRESHOLD {
                        self.position.z = 0.0;
                        self.velocity.z = 0.0;
                    }

                    self.unowned_stopped_ticks = 0;
                    events.add_ball_event(BallEvent::Claimed(nearest_player.id));
                } else if self.unowned_stopped_ticks >= TICK_PHASE_2 && self.take_ball_notified_players.is_empty() {
                    // No one close enough - notify nearest players to come get it
                    let notified = self.notify_nearest_player(players, events);
                    self.take_ball_notified_players = notified;
                    self.notification_timeout = 0;
                }
            }
        } else {
            if velocity_sq >= threshold_sq {
                self.unowned_stopped_ticks = 0;
            }
        }
    }

    /// Unconditional safety net: if ball has been unowned for too long (regardless of speed/height),
    /// force the nearest player from each team into TakeBall state.
    fn force_takeball_if_unowned_too_long(
        &mut self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        const UNOWNED_THRESHOLD: u32 = 300;
        /// Only a genuinely stuck stall is interesting — a single pass is
        /// unowned briefly by design. 300 ticks is the same threshold as
        /// the nuclear-option force-claim: if the system needed that
        /// intervention, the stall was real. Emitted once per stall, on
        /// resolution, with the full duration.
        const STALL_RESOLVE_LOG_THRESHOLD: u32 = 300;

        if self.current_owner.is_some() {
            if self.unowned_ticks >= STALL_RESOLVE_LOG_THRESHOLD {
                let claimed_by = self.current_owner.unwrap_or(0);
                // Include the snapshot captured at period start so the
                // whole stall is reported in one log line: start-state
                // + duration + who resolved it.
                let snapshot = self.stall_start_snapshot.as_deref().unwrap_or("<no snapshot>");
                log::warn!(
                    "ball stall resolved: uncontrolled for {} ticks, claimed by player {} at ({:.1}, {:.1})\n  [start of period]\n{}",
                    self.unowned_ticks,
                    claimed_by,
                    self.position.x, self.position.y,
                    snapshot,
                );
            }
            self.unowned_ticks = 0;
            self.stall_start_snapshot = None;
            return;
        }

        self.unowned_ticks += 1;

        // Period just started — capture the state snapshot while it's
        // still fresh. Every transition from owned → unowned triggers
        // this, so routine passes also allocate; negligible for match
        // runtime (~100 passes × 1–2KB string ≈ 200KB discarded across
        // a 90-minute match). Held until resolution, then discarded if
        // below the log threshold or emitted with the log otherwise.
        if self.unowned_ticks == 1 {
            self.stall_start_snapshot = Some(self.format_stall_snapshot(players));
        }

        // Force-takeball fires every UNOWNED_THRESHOLD ticks while the
        // stall persists. The counter is NOT reset — it keeps climbing
        // so the resolution log reports the true total duration.
        if self.unowned_ticks > 0 && self.unowned_ticks % UNOWNED_THRESHOLD == 0 {
            let notified = self.notify_nearest_player(players, events);
            if !notified.is_empty() {
                self.take_ball_notified_players = notified;
                self.notification_timeout = 0;
            }
        }
    }

    /// Serialise the current tick's ball + player state into a compact
    /// human-readable block. Used by the stall-resolution log to explain
    /// "what did the pitch look like when this got stuck".
    /// Owned-ball stall detector + resolver. The unowned warning misses
    /// this: owner is set, `unowned_ticks` stays 0, but visually the
    /// ball is glued to a player who isn't moving, passing, or
    /// dribbling. After the threshold we warn AND force-release the
    /// ball with a small forward velocity so the pack can reset.
    /// Without the force-release the stall persists indefinitely —
    /// state-machine bugs don't self-heal.
    fn detect_owned_stuck(&mut self, players: &[MatchPlayer]) {
        // 600 ticks = 6 seconds of match time. Real football has
        // legitimate pauses up to ~2-3 sec (set-piece setup, holding
        // under pressure); only log/force-release above that.
        const OWNED_STUCK_THRESHOLD: u32 = 600;
        const MIN_MOTION: f32 = 0.5;
        const MIN_MOTION_SQ: f32 = MIN_MOTION * MIN_MOTION;

        let Some(owner_id) = self.current_owner else {
            self.owned_stuck_ticks = 0;
            self.owned_stuck_logged = false;
            return;
        };

        let Some(owner) = players.iter().find(|p| p.id == owner_id) else {
            self.owned_stuck_ticks = 0;
            self.owned_stuck_logged = false;
            return;
        };

        let owner_speed_sq = owner.velocity.norm_squared();
        let ball_speed_sq = self.velocity.norm_squared();
        let stationary = owner_speed_sq < MIN_MOTION_SQ && ball_speed_sq < MIN_MOTION_SQ;

        if !stationary {
            self.owned_stuck_ticks = 0;
            self.owned_stuck_logged = false;
            return;
        }

        self.owned_stuck_ticks += 1;

        if self.owned_stuck_ticks >= OWNED_STUCK_THRESHOLD && !self.owned_stuck_logged {
            log::warn!(
                "owned-ball stuck: player {} (team {}) has held still for {} ticks at ({:.1}, {:.1}) state={:?} — force-releasing",
                owner.id,
                owner.team_id,
                self.owned_stuck_ticks,
                owner.position.x,
                owner.position.y,
                owner.state,
            );
            self.owned_stuck_logged = true;

            // Force-release with protected flight — a small push gets
            // re-claimed the same tick by the nearest player (usually
            // the stuck owner themselves). `in_flight_state` blocks
            // normal claims for 40 ticks so the ball physically
            // leaves the stall zone.
            let push_x: f32 = match owner.side {
                Some(PlayerSide::Left) => 7.0,
                Some(PlayerSide::Right) => -7.0,
                None => 7.0,
            };
            self.velocity = Vector3::new(push_x, 0.0, 1.5);
            self.previous_owner = self.current_owner;
            self.current_owner = None;
            self.ownership_duration = 0;
            self.claim_cooldown = 0;
            self.flags.in_flight_state = 40;
            self.pass_target_player_id = None;
            self.owned_stuck_ticks = 0;
        }
    }

    /// Position-based stall: the ball hasn't left a small region in N
    /// ticks, regardless of who owns it. Catches the case where
    /// ownership rapidly flips between teammates (each flip resets
    /// owned/unowned counters) but the ball physically stays put.
    /// The anchor resets whenever the ball travels outside the radius,
    /// so normal play keeps advancing the anchor every few ticks.
    fn detect_position_stall(&mut self, players: &[MatchPlayer]) {
        // Raised thresholds so normal possession play doesn't trigger.
        // A team can legitimately keep the ball in a 15-unit zone for
        // 8-10 seconds during sideline passing or defensive possession;
        // 1000 ticks = 10 sec is the floor for "genuinely stuck".
        const STALL_RADIUS: f32 = 15.0;
        const STALL_RADIUS_SQ: f32 = STALL_RADIUS * STALL_RADIUS;
        const STALL_TICKS: u32 = 1000;

        let ball_xy = Vector3::new(self.position.x, self.position.y, 0.0);
        let anchor_xy = Vector3::new(self.stall_anchor_pos.x, self.stall_anchor_pos.y, 0.0);
        let drift_sq = (ball_xy - anchor_xy).norm_squared();

        if drift_sq > STALL_RADIUS_SQ {
            self.stall_anchor_pos = self.position;
            self.stall_anchor_tick = 0;
            return;
        }

        self.stall_anchor_tick += 1;

        if self.stall_anchor_tick == STALL_TICKS {
            let owner_str = self.current_owner
                .map(|id| format!("Some({})", id))
                .unwrap_or_else(|| "None".to_string());
            let owner_state = self.current_owner
                .and_then(|id| players.iter().find(|p| p.id == id))
                .map(|p| format!("{:?}", p.state))
                .unwrap_or_else(|| "-".to_string());
            log::warn!(
                "ball position-stall: stayed within {}u of ({:.1}, {:.1}) for {} ticks — owner={} state={} ball_vel=({:.2}, {:.2})",
                STALL_RADIUS,
                self.stall_anchor_pos.x,
                self.stall_anchor_pos.y,
                STALL_TICKS,
                owner_str,
                owner_state,
                self.velocity.x,
                self.velocity.y,
            );
            // Force-kick out of the zone. Previous attempts with a
            // small push got immediately re-claimed by the same player
            // in `process_ownership` the SAME tick — ball never
            // escaped the 12-unit radius. Solution: kick harder AND
            // set `in_flight_state` so normal ownership checks are
            // suppressed long enough for the ball to actually leave.
            let owner_side = self.current_owner
                .and_then(|id| players.iter().find(|p| p.id == id))
                .and_then(|p| p.side);
            let push_x: f32 = match owner_side {
                Some(PlayerSide::Left) => 7.0,
                Some(PlayerSide::Right) => -7.0,
                _ => 7.0,
            };
            self.velocity = Vector3::new(push_x, 0.0, 1.5);
            self.previous_owner = self.current_owner;
            self.current_owner = None;
            self.ownership_duration = 0;
            self.claim_cooldown = 0;
            // 40 ticks of protected flight — matches a short pass,
            // long enough for the ball to clear the stall radius.
            self.flags.in_flight_state = 40;
            self.pass_target_player_id = None;
            self.owned_stuck_ticks = 0;
            self.owned_stuck_logged = false;
            self.stall_anchor_tick = 0;
            // Teleport anchor so post-release ball travel advances
            // the anchor naturally instead of re-triggering.
            self.stall_anchor_pos = self.position;
        }
    }

    fn format_stall_snapshot(&self, players: &[MatchPlayer]) -> String {
        let mut out = String::with_capacity(2048);
        out.push_str(&format!(
            "  ball pos=({:.1}, {:.1}, {:.1}) velocity=({:.2}, {:.2}, {:.2}) in_flight={} previous_owner={:?}",
            self.position.x, self.position.y, self.position.z,
            self.velocity.x, self.velocity.y, self.velocity.z,
            self.flags.in_flight_state,
            self.previous_owner,
        ));
        for p in players {
            if p.is_sent_off {
                continue;
            }
            out.push_str(&format!(
                "\n  id={} team={} pos=({:.1}, {:.1}) vel=({:.2}, {:.2}) state={} tactical={:?}",
                p.id,
                p.team_id,
                p.position.x, p.position.y,
                p.velocity.x, p.velocity.y,
                p.state,
                p.tactical_position.current_position,
            ));
        }
        out
    }

    fn notify_nearest_player(
        &self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) -> Vec<u32> {
        let ball_position = self.position;
        const NOTIFICATION_RADIUS_SQ: f32 = 500.0 * 500.0;

        // Only 2 teams — use fixed variables instead of HashMap
        let mut team_a_id: u32 = 0;
        let mut team_a_best: Option<(u32, f32)> = None; // (player_id, dist_sq)
        let mut team_b_best: Option<(u32, f32)> = None;

        for player in players {
            let dx = player.position.x - ball_position.x;
            let dy = player.position.y - ball_position.y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq >= NOTIFICATION_RADIUS_SQ {
                continue;
            }

            // Assign first team encountered as team_a
            if team_a_best.is_none() {
                team_a_id = player.team_id;
            }

            let slot = if player.team_id == team_a_id {
                &mut team_a_best
            } else {
                &mut team_b_best
            };

            match slot {
                Some((_, best_dist)) if dist_sq < *best_dist => {
                    *slot = Some((player.id, dist_sq));
                }
                None => {
                    *slot = Some((player.id, dist_sq));
                }
                _ => {}
            }
        }

        let mut notified_players = Vec::new();
        if let Some((id, _)) = team_a_best {
            events.add_ball_event(BallEvent::TakeMe(id));
            notified_players.push(id);
        }
        if let Some((id, _)) = team_b_best {
            events.add_ball_event(BallEvent::TakeMe(id));
            notified_players.push(id);
        }

        notified_players
    }

    fn check_boundary_collision(&mut self, context: &MatchContext) {
        let field_width = context.field_size.width as f32;
        let field_height = context.field_size.height as f32;

        // Push ball well infield when it hits a boundary so players can reliably reach it.
        // 10m is generous enough that the Arrive steering and claim logic work smoothly,
        // while still keeping the ball in the corner/touchline area of the pitch.
        const BOUNDARY_INSET: f32 = 10.0;

        if self.position.x <= 0.0 {
            self.position.x = BOUNDARY_INSET;
            self.velocity = Vector3::zeros();
        } else if self.position.x >= field_width {
            self.position.x = field_width - BOUNDARY_INSET;
            self.velocity = Vector3::zeros();
        }

        if self.position.y <= 0.0 {
            self.position.y = BOUNDARY_INSET;
            self.velocity = Vector3::zeros();
        } else if self.position.y >= field_height {
            self.position.y = field_height - BOUNDARY_INSET;
            self.velocity = Vector3::zeros();
        }
    }

    fn is_players_running_to_ball(&self, players: &[MatchPlayer]) -> bool {
        let ball_position = self.position;

        for player in players {
            let vel_sq = player.velocity.norm_squared();
            if vel_sq < 0.001 {
                continue; // Standing still
            }
            let to_ball = ball_position - player.position;
            let dot_product = to_ball.dot(&player.velocity);
            if dot_product > 0.0 {
                return true;
            }
        }

        false
    }

    fn check_ball_ownership(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // COOLDOWN CHECK: If cooldown is active and there's an owner, skip ownership checks
        // This prevents rapid ping-pong between players
        if self.claim_cooldown > 0 && self.current_owner.is_some() {
            // Just increment ownership duration and return
            self.ownership_duration += 1;
            return;
        }

        // Distance threshold for claiming ball.
        // Bumped from 3.5 → 5.0: a clearance that lands and bounces
        // travels 4-5 units/tick horizontally. With a 3.5-unit claim
        // zone the closest chaser only has a 1-tick window to touch
        // the ball as it flies past, and a small positional error
        // (plus Arrive's braking at the landing spot) means they
        // routinely miss by 4-6 units and the ball runs free for
        // 300+ ticks through multiple bounces before anyone catches
        // up. 5.0 is still a realistic first-touch distance (one step
        // to the ball) and gives a wider interception window without
        // affecting actual contact semantics. Genuinely fast balls
        // (> 10 u/t) still get the tighter 1-unit rule below.
        const BALL_DISTANCE_THRESHOLD: f32 = 5.0;
        const BALL_DISTANCE_THRESHOLD_SQUARED: f32 = BALL_DISTANCE_THRESHOLD * BALL_DISTANCE_THRESHOLD;
        const PLAYER_HEIGHT: f32 = 1.8; // Average player height in meters
        #[allow(dead_code)]
        const PLAYER_REACH_HEIGHT: f32 = PLAYER_HEIGHT + 0.5; // Player can reach ~2.3m when standing
        // 3.5m includes a proper jump header reach — real elite leapers
        // win aerials closer to 3m, and a chest/thigh trap works all
        // the way up to about shoulder height on the way down. The
        // tighter 2.8 was missing any ball descending through 2.8-3.5,
        // which with the bouncy 0.6 coefficient was most of the window.
        const PLAYER_JUMP_REACH: f32 = PLAYER_HEIGHT + 1.7;
        const MAX_BALL_HEIGHT: f32 = PLAYER_JUMP_REACH + 0.5; // Absolute max reachable height

        // CRITICAL: Early validation - if current owner is too far AND ball is moving, clear ownership
        // This catches cases where ball flies away from owner but ownership wasn't properly cleared
        const MAX_OWNERSHIP_DISTANCE: f32 = 2.0; // Maximum distance to maintain ownership (tightened)
        const MAX_OWNERSHIP_DISTANCE_SQUARED: f32 = MAX_OWNERSHIP_DISTANCE * MAX_OWNERSHIP_DISTANCE;
        const MIN_VELOCITY_FOR_DISTANCE_CHECK: f32 = 0.5; // Check distance if ball is moving at all

        if let Some(current_owner_id) = self.current_owner {
            if let Some(owner) = context.players.by_id(current_owner_id) {
                let dx = owner.position.x - self.position.x;
                let dy = owner.position.y - self.position.y;
                let distance_squared = dx * dx + dy * dy;
                let ball_speed_sq = self.velocity.norm_squared();
                let min_vel_sq = MIN_VELOCITY_FOR_DISTANCE_CHECK * MIN_VELOCITY_FOR_DISTANCE_CHECK;

                // Only clear ownership if ball is moving AND far from owner
                // This prevents interference with deadlock claiming and boundary situations
                if distance_squared > MAX_OWNERSHIP_DISTANCE_SQUARED && ball_speed_sq > min_vel_sq {
                    // Use unnormalized dot product — sign is what matters
                    // dot(velocity, to_owner) < 0 means ball moving away from owner
                    let dot = self.velocity.x * dx + self.velocity.y * dy;

                    if distance_squared > 0.01 {
                        if dot < 0.0 { // Ball is moving away from owner
                            // Owner is too far and ball is flying away - clear ownership
                            self.previous_owner = self.current_owner;
                            self.current_owner = None;
                            self.ownership_duration = 0;
                            // Don't return - continue to allow new ownership claim
                        }
                    }
                }
            } else {
                // Owner player not found - clear ownership
                self.previous_owner = self.current_owner;
                self.current_owner = None;
                self.ownership_duration = 0;
            }
        }

        // Ball is too high to be claimed by any player (flying over everyone's heads)
        if self.position.z > MAX_BALL_HEIGHT {
            return;
        }

        // Check if previous owner is still within range
        // Clear previous_owner once they're far enough away to allow normal claiming
        if let Some(previous_owner_id) = self.previous_owner {
            if let Some(owner) = context.players.by_id(previous_owner_id) {
                let dx = owner.position.x - self.position.x;
                let dy = owner.position.y - self.position.y;
                let dz = self.position.z;
                let dist_3d_sq = dx * dx + dy * dy + dz * dz;

                // Clear previous owner once they're far enough
                if dist_3d_sq > BALL_DISTANCE_THRESHOLD_SQUARED {
                    self.previous_owner = None;
                }
                // Don't block claiming - just track who previously had the ball
            } else {
                self.previous_owner = None;
            }
        }

        // Priority claim for pass target receiver (larger radius before normal competition)
        if let Some(target_id) = self.pass_target_player_id {
            if let Some(target_player) = players.iter().find(|p| p.id == target_id) {
                let dx = target_player.position.x - self.position.x;
                let dy = target_player.position.y - self.position.y;
                let dist_sq = dx * dx + dy * dy;

                const RECEIVER_PRIORITY_DISTANCE_SQ: f32 = 3.5 * 3.5;
                const RECEIVER_MAX_HEIGHT: f32 = 2.8;

                if dist_sq < RECEIVER_PRIORITY_DISTANCE_SQ && self.position.z <= RECEIVER_MAX_HEIGHT {
                    self.previous_owner = self.current_owner;
                    self.current_owner = Some(target_id);
                    self.pass_target_player_id = None;
                    self.ownership_duration = 0;
                    self.claim_cooldown = 15;
                    events.add_ball_event(BallEvent::Claimed(target_id));
                    return;
                }
            }
        }

        // Velocity thresholds (squared for comparison without sqrt)
        const MAX_CLAIMABLE_VELOCITY_SQ: f32 = 10.0 * 10.0;
        const SLOW_BALL_VELOCITY_SQ: f32 = 4.0 * 4.0;

        let ball_speed_sq = self.velocity.norm_squared();

        // Collect nearby player IDs into a small inline buffer (no heap allocation)
        const MAX_NEARBY: usize = 8;
        let mut nearby_ids: [u32; MAX_NEARBY] = [0; MAX_NEARBY];
        let mut nearby_count: usize = 0;

        let ball_height_reachable = self.position.z <= PLAYER_JUMP_REACH;

        for player in players.iter() {
            let dx = player.position.x - self.position.x;
            let dy = player.position.y - self.position.y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq > BALL_DISTANCE_THRESHOLD_SQUARED {
                continue;
            }

            if ball_speed_sq <= SLOW_BALL_VELOCITY_SQ {
                if !ball_height_reachable { continue; }
            } else if ball_speed_sq > MAX_CLAIMABLE_VELOCITY_SQ {
                if dist_sq > 1.0 { continue; }
                if !ball_height_reachable { continue; }
            } else {
                if !ball_height_reachable { continue; }
            }

            if nearby_count < MAX_NEARBY {
                nearby_ids[nearby_count] = player.id;
                nearby_count += 1;
            }
        }

        // Early exit if no nearby players
        if nearby_count == 0 {
            return;
        }

        let nearby_slice = &nearby_ids[..nearby_count];

        // Check if current owner is nearby
        if let Some(current_owner_id) = self.current_owner {
            let current_owner_nearby = nearby_slice.contains(&current_owner_id);

            if current_owner_nearby {
                let owner_team_id = context.players.by_id(current_owner_id)
                    .map(|p| p.team_id);

                let opponent_nearby = owner_team_id.is_some_and(|team_id| {
                    nearby_slice.iter().any(|&id| {
                        context.players.by_id(id).is_some_and(|p| p.team_id != team_id)
                    })
                });

                if !opponent_nearby {
                    self.ownership_duration += 1;
                    return;
                }
            } else {
                self.previous_owner = self.current_owner;
                self.current_owner = None;
            }
        }

        // Ownership stability constants
        let min_ownership_duration: u32 = if self.contested_claim_count > 3 {
            60
        } else {
            25
        };
        const TAKEOVER_ADVANTAGE_THRESHOLD: f32 = 1.25;

        // Determine the best tackler from nearby players (no Vec allocation)
        let best_tackler = if nearby_count == 1 {
            players.iter().find(|p| p.id == nearby_ids[0])
        } else {
            let mut best: Option<&MatchPlayer> = None;
            let mut best_score: f32 = -1.0;
            for &pid in nearby_slice {
                if let Some(p) = players.iter().find(|p| p.id == pid) {
                    let score = Self::calculate_tackling_score(p);
                    if score > best_score {
                        best_score = score;
                        best = Some(p);
                    }
                }
            }
            best
        };

        // Transfer ownership to the best tackler (with stability checks)
        if let Some(player) = best_tackler {
            // Check if this is a new owner or maintaining current ownership
            let is_ownership_change = self.current_owner.map_or(true, |id| id != player.id);

            if is_ownership_change {
                // Prevent rapid ownership changes by requiring significant advantage
                if self.ownership_duration < min_ownership_duration {
                    if let Some(current_owner_id) = self.current_owner {
                        // Check if current owner is among nearby players
                        if nearby_slice.contains(&current_owner_id) {
                            if let (Some(current_owner_full), Some(challenger_full)) = (
                                context.players.by_id(current_owner_id),
                                context.players.by_id(player.id),
                            ) {
                                let current_score = Self::calculate_tackling_score(current_owner_full);
                                let challenger_score = Self::calculate_tackling_score(challenger_full);

                                // Require challenger to be significantly better
                                if challenger_score < current_score * TAKEOVER_ADVANTAGE_THRESHOLD {
                                    // Challenger not strong enough - maintain current ownership
                                    self.ownership_duration += 1;
                                    return;
                                }
                            }
                        }
                    }
                }

                // Ownership change approved - reset duration and set cooldown
                self.previous_owner = self.current_owner;
                self.current_owner = Some(player.id);
                self.pass_target_player_id = None;
                self.ownership_duration = 0;

                // Track contested ownership changes and escalate cooldown
                self.contested_claim_count += 1;
                let cooldown = if self.contested_claim_count > 6 {
                    90 // ~1.5s - force resolution
                } else if self.contested_claim_count > 3 {
                    45 // ~0.75s
                } else {
                    15 // Normal cooldown
                };
                self.claim_cooldown = cooldown;
                // Also set in_flight to prevent ClaimBall events from tackling states
                self.flags.in_flight_state = cooldown as usize;

                events.add_ball_event(BallEvent::Claimed(player.id));
            } else {
                // Same owner - just increment duration
                self.ownership_duration += 1;
                // Gradually decay contested counter when ownership is truly stable
                // Require long stability AND no opponents nearby to consider it resolved
                if self.ownership_duration > 100 && self.contested_claim_count > 0 {
                    self.contested_claim_count = self.contested_claim_count.saturating_sub(1);
                }
            }
        }
    }

    fn calculate_tackling_score(player: &MatchPlayer) -> f32 {
        let technical_skills = &player.skills.technical;
        let mental_skills = &player.skills.mental;
        let physical_skills = &player.skills.physical;

        let tackling_weight = 0.45;
        let aggression_weight = 0.15;
        let bravery_weight = 0.10;
        let strength_weight = 0.20;
        let agility_weight = 0.10;

        technical_skills.tackling * tackling_weight
            + mental_skills.aggression * aggression_weight
            + mental_skills.bravery * bravery_weight
            + physical_skills.strength * strength_weight
            + physical_skills.agility * agility_weight
    }

    fn check_goal(&mut self, context: &MatchContext, result: &mut EventCollection) {
        // Guard: don't detect another goal if one was already scored this tick
        if self.goal_scored {
            return;
        }

        // Don't detect goals when ball is attached to a player (ball follows owner).
        // Goals only happen when the ball crosses the line freely (shot, deflection, etc.).
        // This prevents defenders "carrying" the ball into their own goal via boundary clamping.
        if self.current_owner.is_some() {
            return;
        }

        if let Some(goal_side) = context.goal_positions.is_goal(self.position) {
            // Prefer current_owner (e.g. player carrying ball into goal)
            // Fall back to previous_owner (e.g. shooter or passer whose ball went in)
            if let Some(goalscorer) = self.current_owner.or(self.previous_owner) {
                let Some(player) = context.players.by_id(goalscorer) else {
                    return;
                };
                let is_auto_goal = match player.side {
                    Some(PlayerSide::Left) => goal_side == GoalSide::Home,
                    Some(PlayerSide::Right) => goal_side == GoalSide::Away,
                    _ => false
                };

                // Deflection fix: if this would be an own goal but the player only just
                // touched the ball (deflection/failed save), credit the goal to the
                // previous owner (the attacker who actually shot) instead.
                // A genuine own goal requires the defender to have had meaningful possession.
                let (final_scorer, final_is_auto_goal) = if is_auto_goal
                    && self.ownership_duration < 30
                {
                    // Check if previous_owner is from the opposing team (the attacker)
                    let attacker = if self.current_owner == Some(goalscorer) {
                        self.previous_owner
                    } else {
                        // goalscorer came from previous_owner, check recent_passers
                        self.recent_passers.iter().rev()
                            .find(|&&id| id != goalscorer)
                            .copied()
                    };

                    if let Some(attacker_id) = attacker {
                        if let Some(attacker_player) = context.players.by_id(attacker_id) {
                            // Verify attacker is from the other team
                            let attacker_would_score = match attacker_player.side {
                                Some(PlayerSide::Left) => goal_side != GoalSide::Home,
                                Some(PlayerSide::Right) => goal_side != GoalSide::Away,
                                _ => false,
                            };
                            if attacker_would_score {
                                // Credit the attacker — this was a deflection, not a real own goal
                                (attacker_id, false)
                            } else {
                                (goalscorer, true)
                            }
                        } else {
                            (goalscorer, true)
                        }
                    } else {
                        (goalscorer, true)
                    }
                } else {
                    (goalscorer, is_auto_goal)
                };

                // Find assist provider: most recent passer who isn't the goalscorer
                let assist_player_id = if !final_is_auto_goal {
                    self.recent_passers
                        .iter()
                        .rev()
                        .find(|&&id| id != final_scorer)
                        .copied()
                } else {
                    None
                };

                let goal_event_metadata = BallGoalEventMetadata {
                    side: goal_side,
                    goalscorer_player_id: final_scorer,
                    assist_player_id,
                    auto_goal: final_is_auto_goal,
                };

                result.add_ball_event(BallEvent::Goal(goal_event_metadata));
            }

            // Determine which side should kick off (the conceding team)
            // Home goal (x=0) = Left side conceded → Left kicks off
            // Away goal (x=field_width) = Right side conceded → Right kicks off
            self.kickoff_team_side = match goal_side {
                GoalSide::Home => Some(PlayerSide::Left),
                GoalSide::Away => Some(PlayerSide::Right),
            };

            self.goal_scored = true;
            self.reset();
        }
    }

    /// Ball crossed goal line within goal width but above crossbar — goal kick.
    /// Place ball near the 6-yard box and give it to the defending goalkeeper.
    fn check_over_goal(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        let over_side = match context.goal_positions.is_over_goal(self.position) {
            Some(side) => side,
            None => return,
        };

        // Determine which side's goalkeeper defends this goal
        // GoalSide::Home = left goal (x=0) → defended by PlayerSide::Left
        // GoalSide::Away = right goal (x=field_width) → defended by PlayerSide::Right
        let defending_side = match over_side {
            GoalSide::Home => PlayerSide::Left,
            GoalSide::Away => PlayerSide::Right,
        };

        // Find the goalkeeper on the defending side
        if let Some(gk) = players.iter().find(|p| {
            p.side == Some(defending_side)
                && p.tactical_position.current_position.is_goalkeeper()
        }) {
            // Place ball at the 6-yard area in front of the goal
            let goal_kick_x = match over_side {
                GoalSide::Home => 50.0,  // ~6 yards from left goal line
                GoalSide::Away => self.field_width - 50.0,
            };

            self.position.x = goal_kick_x;
            self.position.y = context.goal_positions.left.y; // Center of goal
            self.position.z = 0.0;
            self.velocity = Vector3::zeros();

            // Give ball to goalkeeper
            self.current_owner = Some(gk.id);
            self.previous_owner = None;
            self.ownership_duration = 0;
            self.claim_cooldown = 30; // Protection so no one steals immediately
            self.flags.in_flight_state = 30;
            self.pass_target_player_id = None;

            events.add_ball_event(BallEvent::Claimed(gk.id));
        }
    }

    /// Ball crossed the endline (x <= 0 or x >= field_width) but OUTSIDE the goal posts.
    /// In real football this is a goal kick OR a corner kick — depending on
    /// which team last touched the ball.
    fn check_wide_of_goal(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        let field_width = context.field_size.width as f32;
        let goal_half_width = crate::r#match::engine::goal::GOAL_WIDTH;

        // Check left endline
        let crossed_side = if self.position.x <= 0.0 {
            let goal_center_y = context.goal_positions.left.y;
            // Only trigger if OUTSIDE the goal posts (inside is handled by check_goal/check_over_goal)
            if self.position.y < goal_center_y - goal_half_width
                || self.position.y > goal_center_y + goal_half_width
            {
                Some(GoalSide::Home)
            } else {
                None
            }
        } else if self.position.x >= field_width {
            let goal_center_y = context.goal_positions.right.y;
            if self.position.y < goal_center_y - goal_half_width
                || self.position.y > goal_center_y + goal_half_width
            {
                Some(GoalSide::Away)
            } else {
                None
            }
        } else {
            None
        };

        let side = match crossed_side {
            Some(s) => s,
            None => return,
        };

        let defending_side = match side {
            GoalSide::Home => PlayerSide::Left,
            GoalSide::Away => PlayerSide::Right,
        };
        let attacking_side = match defending_side {
            PlayerSide::Left => PlayerSide::Right,
            PlayerSide::Right => PlayerSide::Left,
        };

        // Decide corner vs goal kick from the last player who touched the
        // ball. If the defending team put it out, it's a corner for the
        // attacking team. Unknown last-touch defaults to goal kick.
        let last_toucher_side: Option<PlayerSide> = self
            .previous_owner
            .or(self.current_owner)
            .and_then(|pid| players.iter().find(|p| p.id == pid))
            .and_then(|p| p.side);

        let is_corner = last_toucher_side == Some(defending_side);

        if is_corner {
            // Attacking team gets a corner. Place ball at the nearest corner
            // flag and hand it to the attacking team's best corner taker.
            let corner_x = match side {
                GoalSide::Home => 2.0,
                GoalSide::Away => field_width - 2.0,
            };
            let field_height = context.field_size.height as f32;
            // Pick the near corner based on where the ball went out
            let near_top = self.position.y < field_height * 0.5;
            let corner_y = if near_top { 2.0 } else { field_height - 2.0 };

            // Find the attacking team's designated corner taker — score by
            // (crossing, technique, corners) like SetPieceSetup::choose, but
            // restricted to players currently on the pitch.
            let taker = players
                .iter()
                .filter(|p| {
                    p.side == Some(attacking_side)
                        && !p.tactical_position.current_position.is_goalkeeper()
                })
                .max_by(|a, b| {
                    let sa = a.skills.technical.crossing * 0.6
                        + a.skills.technical.technique * 0.3
                        + a.skills.technical.corners * 0.1;
                    let sb = b.skills.technical.crossing * 0.6
                        + b.skills.technical.technique * 0.3
                        + b.skills.technical.corners * 0.1;
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                });

            if let Some(taker) = taker {
                let taker_id = taker.id;
                self.position.x = corner_x;
                self.position.y = corner_y;
                self.position.z = 0.0;
                self.velocity = Vector3::zeros();

                self.current_owner = Some(taker_id);
                self.previous_owner = None;
                self.ownership_duration = 0;
                self.claim_cooldown = 30;
                self.flags.in_flight_state = 30;
                self.pass_target_player_id = None;
                self.recent_passers.clear();

                events.add_ball_event(BallEvent::Claimed(taker_id));
                // Teleport the taker onto the ball so `move_to`'s
                // distance check doesn't immediately null ownership
                // on the next tick. The ball struct only has a &[MatchPlayer]
                // here — record the teleport and let the engine apply
                // it when it has &mut field.players.
                self.pending_set_piece_teleport = Some((taker_id, self.position));
                return;
            }
            // If no eligible outfielder was found, fall through to goal kick
        }

        // Goal kick: give ball to defending goalkeeper
        if let Some(gk) = players.iter().find(|p| {
            p.side == Some(defending_side)
                && p.tactical_position.current_position.is_goalkeeper()
        }) {
            let gk_id = gk.id;
            let goal_kick_x = match side {
                GoalSide::Home => 50.0,
                GoalSide::Away => field_width - 50.0,
            };

            self.position.x = goal_kick_x;
            self.position.y = context.goal_positions.left.y;
            self.position.z = 0.0;
            self.velocity = Vector3::zeros();

            self.current_owner = Some(gk_id);
            self.previous_owner = None;
            self.ownership_duration = 0;
            self.claim_cooldown = 30;
            self.flags.in_flight_state = 30;
            self.pass_target_player_id = None;
            self.recent_passers.clear();

            events.add_ball_event(BallEvent::Claimed(gk_id));
            // Same as corner kick: put the GK onto the ball so the
            // distance check in `move_to` doesn't immediately null
            // ownership because the GK was ~35 units away at the goal
            // line when the ball crossed the end line.
            self.pending_set_piece_teleport = Some((gk_id, self.position));
        }
    }

    pub fn update_velocity(&mut self) {
        const GRAVITY: f32 = 9.81;
        const BALL_MASS: f32 = 0.43;
        const STOPPING_THRESHOLD: f32 = 0.05; // Lower threshold for smoother final stop
        // Football bounce retention on grass is ~25-35%. The previous
        // 0.6 produced trampoline bounces where a lofted clearance
        // bounced to 30m+ and stayed airborne (above PLAYER_JUMP_REACH)
        // for 3-5 cycles before a defender could claim. 0.3 keeps the
        // second bounce low enough to reach on the return trip.
        const BOUNCE_COEFFICIENT: f32 = 0.3;
        const MAX_VELOCITY: f32 = 15.0; // Maximum realistic ball velocity per tick

        // Physics constants for realistic ball behavior
        // Air drag: affects aerial balls (proportional to v²)
        const AIR_DRAG_COEFFICIENT: f32 = 0.04; // Reduced for more realistic air resistance

        // Ground friction: affects rolling balls (proportional to v for smooth deceleration)
        // A real football on grass decelerates at about 0.5-1.5 m/s² depending on grass conditions
        const GROUND_FRICTION_COEFFICIENT: f32 = 0.015; // Smooth velocity-proportional friction

        // CRITICAL: Global velocity sanity check - prevent cosmic-speed balls
        // Check for NaN or infinity and reset to zero
        if self.velocity.x.is_nan() || self.velocity.y.is_nan() || self.velocity.z.is_nan()
            || self.velocity.x.is_infinite() || self.velocity.y.is_infinite() || self.velocity.z.is_infinite()
        {
            self.velocity = Vector3::zeros();
            return;
        }

        let mut velocity_norm_sq = self.velocity.norm_squared();

        // Clamp velocity if it exceeds maximum
        if velocity_norm_sq > MAX_VELOCITY * MAX_VELOCITY {
            let velocity_norm = velocity_norm_sq.sqrt();
            self.velocity = self.velocity * (MAX_VELOCITY / velocity_norm);
            velocity_norm_sq = MAX_VELOCITY * MAX_VELOCITY;
        }

        if velocity_norm_sq > STOPPING_THRESHOLD * STOPPING_THRESHOLD {
            let velocity_norm = velocity_norm_sq.sqrt();
            let is_on_ground = self.position.z <= 0.1;

            if is_on_ground {
                // GROUND PHYSICS: Rolling friction proportional to velocity (smooth deceleration)
                let horizontal_speed_sq = self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y;

                if horizontal_speed_sq > STOPPING_THRESHOLD * STOPPING_THRESHOLD {
                    // Apply friction as a multiplier for smooth exponential decay
                    // friction_factor < 1.0 means the ball gradually slows down
                    let friction_factor = 1.0 - GROUND_FRICTION_COEFFICIENT;
                    self.velocity.x *= friction_factor;
                    self.velocity.y *= friction_factor;
                }

                // Keep ball on ground, but allow upward kicks to take effect
                // (positive z velocity means ball is being kicked into the air)
                if self.velocity.z <= 0.0 {
                    self.velocity.z = 0.0;
                    self.position.z = 0.0;
                }
            } else {
                // AERIAL PHYSICS: Air drag (proportional to v²) + gravity
                // Air drag is gentler than ground friction for realistic flight

                // Air drag force: F = -0.5 * C * v² * direction
                let air_drag_force = if velocity_norm > 0.1 {
                    -AIR_DRAG_COEFFICIENT * velocity_norm * self.velocity
                } else {
                    Vector3::zeros()
                };

                // Gravity force (constant downward)
                let gravity_force = Vector3::new(0.0, 0.0, -GRAVITY);

                // Apply forces
                let acceleration = air_drag_force / BALL_MASS + gravity_force;
                self.velocity += acceleration * 0.016; // ~60fps timestep
            }
        } else {
            // Ball has nearly stopped - bring to complete rest smoothly
            // Use gradual decay instead of instant stop
            self.velocity = self.velocity * 0.8; // Smooth final decay

            // Only fully stop when truly negligible
            if self.velocity.norm_squared() < 0.0001 { // 0.01^2
                self.velocity = Vector3::zeros();
                self.position.z = 0.0;
            }
        }

        // Check ground collision and bounce
        if self.position.z <= 0.0 && self.velocity.z < 0.0 {
            // Ball hit the ground
            self.position.z = 0.0;
            self.velocity.z = -self.velocity.z * BOUNCE_COEFFICIENT;

            // Apply some horizontal speed loss on bounce (realistic)
            self.velocity.x *= 0.95;
            self.velocity.y *= 0.95;

            // If bounce is too small, stop vertical movement
            if self.velocity.z.abs() < 0.3 {
                self.velocity.z = 0.0;
            }
        }
    }

    fn move_to(&mut self, tick_context: &GameTickContext) {
        // Clear notified players only when ball state changes significantly:
        // 1. Ball starts moving (not stopped anymore)
        // 2. Ball has an owner (claimed)
        // Maximum distance owner can be from ball - must match deadlock claim distances
        // This allows deadlock resolution while preventing truly absurd teleports
        const MAX_OWNER_TELEPORT_DISTANCE: f32 = 15.0;
        const MAX_OWNER_TELEPORT_DISTANCE_SQUARED: f32 = MAX_OWNER_TELEPORT_DISTANCE * MAX_OWNER_TELEPORT_DISTANCE;

        // Ball moves toward owner at this speed (units/tick) instead of teleporting
        const BALL_TRACK_SPEED: f32 = 1.5;
        // Snap to owner if within this distance (avoids jitter)
        const SNAP_DISTANCE: f32 = 2.0;
        const SNAP_DISTANCE_SQUARED: f32 = SNAP_DISTANCE * SNAP_DISTANCE;

        let has_owner = self.current_owner.is_some();

        // Clear notifications when ball is no longer in a "take ball" scenario
        // Use a higher threshold to avoid clearing notifications set by try_notify_standing_ball
        // which uses is_ball_stopped_on_field (velocity < 2.5)
        const CLEAR_NOTIFICATION_VELOCITY: f32 = 3.0;
        let is_clearly_moving = self.velocity.norm() > CLEAR_NOTIFICATION_VELOCITY;
        if (is_clearly_moving || has_owner) && !self.take_ball_notified_players.is_empty() {
            self.take_ball_notified_players.clear();
        }

        if let Some(owner_player_id) = self.current_owner {
            let owner_position = tick_context.positions.players.position(owner_player_id);

            let dx = owner_position.x - self.position.x;
            let dy = owner_position.y - self.position.y;
            let distance_squared = dx * dx + dy * dy;

            if distance_squared <= MAX_OWNER_TELEPORT_DISTANCE_SQUARED {
                if distance_squared <= SNAP_DISTANCE_SQUARED {
                    // Close enough - snap to owner
                    self.position = owner_position;
                    self.position.z = 0.0;
                    self.velocity = Vector3::zeros();
                } else {
                    // Move ball toward owner smoothly instead of teleporting
                    let distance = distance_squared.sqrt();
                    let dir_x = dx / distance;
                    let dir_y = dy / distance;
                    self.position.x += dir_x * BALL_TRACK_SPEED;
                    self.position.y += dir_y * BALL_TRACK_SPEED;
                    self.position.z = 0.0;
                    self.velocity = Vector3::zeros();
                }
            } else {
                // Owner is too far - this shouldn't happen but is a safety net
                // Clear ownership and let ball move naturally
                self.previous_owner = self.current_owner;
                self.current_owner = None;
                self.ownership_duration = 0;

                // Move ball normally
                self.position.x += self.velocity.x;
                self.position.y += self.velocity.y;
                self.position.z += self.velocity.z;

                if self.position.z < 0.0 {
                    self.position.z = 0.0;
                }
            }
        } else {
            self.position.x += self.velocity.x;
            self.position.y += self.velocity.y;
            self.position.z += self.velocity.z;

            // Ensure ball doesn't go below ground
            if self.position.z < 0.0 {
                self.position.z = 0.0;
            }
        }
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
        self.process_ownership(context, players, events);

        // Move ball: find owner position from players slice directly
        self.move_to_with_players(players);
        self.check_goal(context, events);
        self.check_over_goal(context, players, events);
        self.check_wide_of_goal(context, players, events);
        self.check_boundary_collision(context);
        self.update_landing_cache();
    }

    fn move_to_with_players(&mut self, players: &[MatchPlayer]) {
        const MAX_OWNER_TELEPORT_DISTANCE_SQUARED: f32 = 15.0 * 15.0;
        const BALL_TRACK_SPEED: f32 = 1.5;
        const SNAP_DISTANCE_SQUARED: f32 = 2.0 * 2.0;

        if let Some(owner_id) = self.current_owner {
            if let Some(owner) = players.iter().find(|p| p.id == owner_id) {
                let dx = owner.position.x - self.position.x;
                let dy = owner.position.y - self.position.y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= MAX_OWNER_TELEPORT_DISTANCE_SQUARED {
                    if dist_sq <= SNAP_DISTANCE_SQUARED {
                        self.position = owner.position;
                        self.position.z = 0.0;
                        self.velocity = Vector3::zeros();
                    } else {
                        let dist = dist_sq.sqrt();
                        self.position.x += (dx / dist) * BALL_TRACK_SPEED;
                        self.position.y += (dy / dist) * BALL_TRACK_SPEED;
                        self.position.z = 0.0;
                        self.velocity = Vector3::zeros();
                    }
                } else {
                    self.previous_owner = self.current_owner;
                    self.current_owner = None;
                    self.ownership_duration = 0;
                    self.apply_movement();
                }
            } else {
                self.apply_movement();
            }
        } else {
            self.apply_movement();
        }
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
        self.take_ball_notified_players.retain(|&id| id != player_id);
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
