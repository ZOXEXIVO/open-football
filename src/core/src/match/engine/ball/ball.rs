use std::collections::HashMap;
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
    pub notification_timeout: u32,  // Ticks since players were notified
    pub last_boundary_position: Option<Vector3<f32>>,
    pub unowned_stopped_ticks: u32,  // How long ball has been stopped without owner
    pub ownership_duration: u32,  // How many ticks current owner has had the ball
    pub claim_cooldown: u32,  // Cooldown ticks before another player can claim the ball
    pub pass_target_player_id: Option<u32>,  // Intended receiver of a pass
    pub recent_passers: VecDeque<u32>,  // Ring buffer of recent passers (up to 5)
}

#[derive(Default)]
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
        }
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

        self.process_ownership(context, players, events);

        // Move ball FIRST, then check goal/boundary on new position
        self.move_to(tick_context);
        self.check_goal(context, events);
        self.check_boundary_collision(context);
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
                // Use landing position for aerial balls, current position for ground balls
                let effective_ball_pos = if self.is_aerial() {
                    self.calculate_landing_position()
                } else {
                    self.position
                };

                let dx = target_player.position.x - effective_ball_pos.x;
                let dy = target_player.position.y - effective_ball_pos.y;
                let distance = (dx * dx + dy * dy).sqrt();

                // Generous claim radius for intended receiver (3.5m vs normal 2.0m)
                const RECEIVER_CLAIM_DISTANCE: f32 = 3.5;
                const RECEIVER_MAX_HEIGHT: f32 = 2.8;

                if distance < RECEIVER_CLAIM_DISTANCE && self.position.z <= RECEIVER_MAX_HEIGHT {
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
            let distance_from_boundary = (self.position - last_pos).magnitude();
            distance_from_boundary > 2.0 // Reduced from 5.0 to allow re-notification for slow rolling balls
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
                self.calculate_landing_position()
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

    pub fn try_intercept(&mut self, _players: &[MatchPlayer], _events: &mut EventCollection) {}

    /// Calculate where an aerial ball will land (when z reaches 0)
    /// Returns the predicted landing position using simple projection
    pub fn calculate_landing_position(&self) -> Vector3<f32> {
        // If ball is already on ground or owned, return current position
        if self.position.z <= 0.1 || self.current_owner.is_some() {
            return self.position;
        }

        // If ball is moving up or not moving vertically, estimate it will land near current position
        if self.velocity.z >= 0.0 {
            return Vector3::new(self.position.x, self.position.y, 0.0);
        }

        // Simple projection: calculate time until ball reaches ground (z = 0)
        // time = current_height / vertical_speed
        let time_to_ground = self.position.z / self.velocity.z.abs();

        // Project horizontal position
        let landing_x = self.position.x + self.velocity.x * time_to_ground;
        let landing_y = self.position.y + self.velocity.y * time_to_ground;

        // Clamp to field boundaries
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
            && self.velocity.norm() < 0.5 // Changed from exact 0.0 to allow tiny velocities from physics
            && self.current_owner.is_none()
    }

    pub fn is_ball_stopped_on_field(&self) -> bool {
        !self.is_ball_outside()
            && self.velocity.norm() < 2.5 // Increased to catch slow rolling balls that need claiming
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

        let velocity_norm = self.velocity.norm();
        let is_slow = velocity_norm < velocity_threshold;
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
                // Find nearest player within current claim distance
                if let Some(nearest_player) = players.iter()
                    .filter_map(|p| {
                        let dx = p.position.x - self.position.x;
                        let dy = p.position.y - self.position.y;
                        let distance = (dx * dx + dy * dy).sqrt();
                        if distance <= claim_distance {
                            Some((p, distance))
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
            if velocity_norm >= velocity_threshold {
                self.unowned_stopped_ticks = 0;
            }
        }
    }

    fn notify_nearest_player(
        &self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) -> Vec<u32> {
        let ball_position = self.position;
        const NOTIFICATION_RADIUS: f32 = 500.0; // Cover entire field - all players can be notified

        // Group players by team and find nearest from each team
        let mut team_nearest: HashMap<u32, (&MatchPlayer, f32)> = HashMap::new();

        for player in players {
            let dx = player.position.x - ball_position.x;
            let dy = player.position.y - ball_position.y;
            let distance_squared = dx * dx + dy * dy;

            // Only consider players within notification radius
            if distance_squared < NOTIFICATION_RADIUS * NOTIFICATION_RADIUS {
                // Check if this is the nearest player for their team
                team_nearest
                    .entry(player.team_id)
                    .and_modify(|current| {
                        if distance_squared < current.1 {
                            *current = (player, distance_squared);
                        }
                    })
                    .or_insert((player, distance_squared));
            }
        }

        // Notify one player from each team and collect their IDs
        let mut notified_players = Vec::new();
        for (player, _) in team_nearest.values() {
            events.add_ball_event(BallEvent::TakeMe(player.id));
            notified_players.push(player.id);
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
        let player_positions: Vec<(Vector3<f32>, Vector3<f32>)> = players
            .iter()
            .map(|player| (player.position, player.velocity))
            .collect();

        for (player_position, player_velocity) in player_positions {
            let direction_to_ball = (ball_position - player_position).normalize();
            let player_direction = player_velocity.normalize();
            let dot_product = direction_to_ball.dot(&player_direction);

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

        // Distance threshold for claiming ball
        const BALL_DISTANCE_THRESHOLD: f32 = 3.5; // Players can claim within 3.5m (generous for corners/boundaries)
        const BALL_DISTANCE_THRESHOLD_SQUARED: f32 = BALL_DISTANCE_THRESHOLD * BALL_DISTANCE_THRESHOLD;
        const PLAYER_HEIGHT: f32 = 1.8; // Average player height in meters
        const PLAYER_REACH_HEIGHT: f32 = PLAYER_HEIGHT + 0.5; // Player can reach ~2.3m when standing
        const PLAYER_JUMP_REACH: f32 = PLAYER_HEIGHT + 1.0; // Player can reach ~2.8m when jumping
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
                let ball_speed = self.velocity.norm();

                // Only clear ownership if ball is moving AND far from owner
                // This prevents interference with deadlock claiming and boundary situations
                if distance_squared > MAX_OWNERSHIP_DISTANCE_SQUARED && ball_speed > MIN_VELOCITY_FOR_DISTANCE_CHECK {
                    // Check if ball is moving AWAY from owner (not towards them)
                    let ball_dir_x = self.velocity.x / ball_speed;
                    let ball_dir_y = self.velocity.y / ball_speed;

                    // Vector from ball to owner
                    let to_owner_x = dx;
                    let to_owner_y = dy;
                    let to_owner_dist = (dx * dx + dy * dy).sqrt();

                    if to_owner_dist > 0.1 {
                        let to_owner_norm_x = to_owner_x / to_owner_dist;
                        let to_owner_norm_y = to_owner_y / to_owner_dist;

                        // Dot product: negative means ball is moving away from owner
                        let dot = ball_dir_x * to_owner_norm_x + ball_dir_y * to_owner_norm_y;

                        if dot < -0.3 { // Ball is clearly moving away from owner
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
                let distance_3d = (dx * dx + dy * dy + dz * dz).sqrt();

                // Clear previous owner once they're far enough - allow normal claiming to proceed
                if distance_3d > BALL_DISTANCE_THRESHOLD {
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
                let distance = (dx * dx + dy * dy).sqrt();

                const RECEIVER_PRIORITY_DISTANCE: f32 = 3.5;
                const RECEIVER_MAX_HEIGHT: f32 = 2.8;

                if distance < RECEIVER_PRIORITY_DISTANCE && self.position.z <= RECEIVER_MAX_HEIGHT {
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

        // Velocity thresholds
        const MAX_CLAIMABLE_VELOCITY: f32 = 10.0; // Ball moving faster than 10 m/s is hard to claim
        const SLOW_BALL_VELOCITY: f32 = 4.0; // Ball moving slower than 4 m/s is easy to claim

        let ball_speed = self.velocity.norm();

        // Find all players within ball distance threshold
        let nearby_players: Vec<&MatchPlayer> = players
            .iter()
            .filter(|player| {
                let dx = player.position.x - self.position.x;
                let dy = player.position.y - self.position.y;
                let horizontal_distance_squared = dx * dx + dy * dy;
                let horizontal_distance = horizontal_distance_squared.sqrt();

                // Check if within claiming distance
                if horizontal_distance_squared > BALL_DISTANCE_THRESHOLD_SQUARED {
                    return false;
                }

                // For slow/stopped balls, always allow claiming if close enough
                if ball_speed <= SLOW_BALL_VELOCITY {
                    return self.position.z <= PLAYER_JUMP_REACH;
                }

                // For fast balls, apply direction check
                if ball_speed > MAX_CLAIMABLE_VELOCITY {
                    // Very fast ball - must be very close
                    if horizontal_distance > 1.0 {
                        return false;
                    }
                }

                // Check ball height is reachable
                self.position.z <= PLAYER_JUMP_REACH
            })
            .collect();

        // Early exit if no nearby players
        if nearby_players.is_empty() {
            return;
        }

        // Check if current owner is nearby
        if let Some(current_owner_id) = self.current_owner {
            // Check if current owner is still nearby
            let current_owner_nearby = nearby_players
                .iter()
                .any(|player| player.id == current_owner_id);

            if current_owner_nearby {
                // Check if any opponent is also nearby and could challenge
                let owner_team_id = context.players.by_id(current_owner_id)
                    .map(|p| p.team_id);

                let opponent_nearby = owner_team_id.is_some_and(|team_id| {
                    nearby_players.iter().any(|p| p.team_id != team_id)
                });

                if !opponent_nearby {
                    // No opponents close - owner keeps the ball unchallenged
                    self.ownership_duration += 1;
                    return;
                }
                // Opponent is close enough to challenge — fall through to tackling logic
            } else {
                // Current owner is NOT nearby - clear ownership so ball can be claimed
                self.previous_owner = self.current_owner;
                self.current_owner = None;
            }
        }

        // Ownership stability constants — give the ball holder time to act
        const MIN_OWNERSHIP_DURATION: u32 = 25; // ~0.4s minimum before ownership can change
        const TAKEOVER_ADVANTAGE_THRESHOLD: f32 = 1.25; // Challenger must be 25% better to steal

        // Determine the best tackler from nearby players
        let best_tackler = if nearby_players.len() == 1 {
            nearby_players.first().copied()
        } else {
            nearby_players
                .iter()
                .max_by(|player_a, player_b| {
                    let player_a_full = context.players.by_id(player_a.id).unwrap();
                    let player_b_full = context.players.by_id(player_b.id).unwrap();

                    let tackling_score_a = Self::calculate_tackling_score(player_a_full);
                    let tackling_score_b = Self::calculate_tackling_score(player_b_full);

                    tackling_score_a
                        .partial_cmp(&tackling_score_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied()
        };

        // Transfer ownership to the best tackler (with stability checks)
        if let Some(player) = best_tackler {
            // Check if this is a new owner or maintaining current ownership
            let is_ownership_change = self.current_owner.map_or(true, |id| id != player.id);

            if is_ownership_change {
                // Prevent rapid ownership changes by requiring significant advantage
                if self.ownership_duration < MIN_OWNERSHIP_DURATION {
                    if let Some(current_owner_id) = self.current_owner {
                        // Find current owner in nearby players
                        if let Some(_current_owner) = nearby_players.iter()
                            .find(|p| p.id == current_owner_id)
                        {
                            let current_owner_full = context.players.by_id(current_owner_id).unwrap();
                            let challenger_full = context.players.by_id(player.id).unwrap();

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

                // Ownership change approved - reset duration and set cooldown
                self.previous_owner = self.current_owner;
                self.current_owner = Some(player.id);
                self.pass_target_player_id = None;
                self.ownership_duration = 0;
                self.claim_cooldown = 15; // Same as CLAIM_COOLDOWN_TICKS
                events.add_ball_event(BallEvent::Claimed(player.id));
            } else {
                // Same owner - just increment duration
                self.ownership_duration += 1;
            }
        }
    }

    fn calculate_tackling_score(player: &MatchPlayer) -> f32 {
        let technical_skills = &player.skills.technical;
        let mental_skills = &player.skills.mental;
        let physical_skills = &player.skills.physical;

        let tackling_weight = 0.4;
        let aggression_weight = 0.2;
        let bravery_weight = 0.1;
        let strength_weight = 0.2;
        let agility_weight = 0.1;

        technical_skills.tackling * tackling_weight
            + mental_skills.aggression * aggression_weight
            + mental_skills.bravery * bravery_weight
            + physical_skills.strength * strength_weight
            + physical_skills.agility * agility_weight
    }

    fn check_goal(&mut self, context: &MatchContext, result: &mut EventCollection) {
        if let Some(goal_side) = context.goal_positions.is_goal(self.position) {
            if let Some(goalscorer) = self.previous_owner.or(self.current_owner) {
                let player = context.players.by_id(goalscorer).unwrap();
                let is_auto_goal = match player.side {
                    Some(PlayerSide::Left) => goal_side == GoalSide::Home,
                    Some(PlayerSide::Right) => goal_side == GoalSide::Away,
                    _ => false
                };

                let goal_event_metadata = BallGoalEventMetadata {
                    side: goal_side,
                    goalscorer_player_id: goalscorer,
                    auto_goal: is_auto_goal,
                };

                result.add_ball_event(BallEvent::Goal(goal_event_metadata));
            }

            self.reset();
        }
    }

    fn update_velocity(&mut self) {
        const GRAVITY: f32 = 9.81;
        const BALL_MASS: f32 = 0.43;
        const STOPPING_THRESHOLD: f32 = 0.05; // Lower threshold for smoother final stop
        const BOUNCE_COEFFICIENT: f32 = 0.6; // Ball keeps 60% of velocity when bouncing
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

        let velocity_norm = self.velocity.norm();

        // Clamp velocity if it exceeds maximum
        if velocity_norm > MAX_VELOCITY {
            self.velocity = self.velocity * (MAX_VELOCITY / velocity_norm);
        }

        let velocity_norm = self.velocity.norm();

        if velocity_norm > STOPPING_THRESHOLD {
            let is_on_ground = self.position.z <= 0.1;

            if is_on_ground {
                // GROUND PHYSICS: Rolling friction proportional to velocity (smooth deceleration)
                // This creates exponential decay: v(t) = v0 * e^(-kt), which is very smooth
                let horizontal_velocity = Vector3::new(self.velocity.x, self.velocity.y, 0.0);
                let horizontal_speed = horizontal_velocity.norm();

                if horizontal_speed > STOPPING_THRESHOLD {
                    // Apply friction as a multiplier for smooth exponential decay
                    // friction_factor < 1.0 means the ball gradually slows down
                    let friction_factor = 1.0 - GROUND_FRICTION_COEFFICIENT;
                    self.velocity.x *= friction_factor;
                    self.velocity.y *= friction_factor;
                }

                // Keep ball on ground
                self.velocity.z = 0.0;
                self.position.z = 0.0;
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
            if self.velocity.norm() < 0.01 {
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

            // SANITY CHECK: Validate owner is actually close to ball before teleporting
            let dx = owner_position.x - self.position.x;
            let dy = owner_position.y - self.position.y;
            let distance_squared = dx * dx + dy * dy;

            if distance_squared <= MAX_OWNER_TELEPORT_DISTANCE_SQUARED {
                // Owner is close enough - ball follows owner
                self.position = owner_position;
                self.position.z = 0.0;
                self.velocity = Vector3::zeros();
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

    pub fn reset(&mut self) {
        self.position.x = self.start_position.x;
        self.position.y = self.start_position.y;

        self.velocity = Vector3::zeros();

        self.flags.reset();
        self.pass_target_player_id = None;
        self.clear_pass_history();
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
