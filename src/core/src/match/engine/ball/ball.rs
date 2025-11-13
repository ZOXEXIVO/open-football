use std::collections::HashMap;
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
        }
    }

    pub fn update(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        self.update_velocity();
        self.check_goal(context, events);
        self.check_boundary_collision(context);

        self.try_intercept(players, events);
        self.try_notify_standing_ball(players, events);

        // NUCLEAR OPTION: Force claiming if ball unowned and stopped for too long
        self.force_claim_if_deadlock(players, events);

        self.process_ownership(context, players, events);

        self.move_to(tick_context);
    }

    pub fn process_ownership(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // prevent pass tackling
        if self.flags.in_flight_state > 0 {
            self.flags.in_flight_state -= 1;
        } else {
            self.check_ball_ownership(context, players, events);
        }

        self.flags.running_for_ball = self.is_players_running_to_ball(players);
    }

    pub fn try_notify_standing_ball(
        &mut self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
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

                // If ball is at boundary, set cooldown and record position
                if self.is_ball_outside() {
                    self.notification_cooldown = 10; // Reduced from 30 to 10 ticks (~0.16 seconds) for faster response
                    self.last_boundary_position = Some(self.position);
                }
            }
        } else if !self.take_ball_notified_players.is_empty() {
            // Increment timeout counter
            self.notification_timeout += 1;

            // If players haven't claimed the ball within reasonable time, reset and try again
            const MAX_NOTIFICATION_TIMEOUT: u32 = 200; // ~3.2 seconds - increased to give players more time
            if self.notification_timeout > MAX_NOTIFICATION_TIMEOUT {
                self.take_ball_notified_players.clear();
                self.notification_timeout = 0;
                // Clear boundary position to allow re-notification even if ball hasn't moved
                self.last_boundary_position = None;
                return; // Will re-notify on next tick
            }
            // Check if any notified player reached the ball (increased for easier claiming)
            const CLAIM_DISTANCE: f32 = 20.0;

            // For aerial balls, check distance to landing position
            let target_position = if self.is_aerial() {
                self.calculate_landing_position()
            } else {
                self.position
            };

            // Find the first player who reached the ball
            let mut claiming_player_id: Option<u32> = None;
            let mut all_players_missing = true;

            for notified_player_id in &self.take_ball_notified_players {
                if let Some(player) = players.iter().find(|p| p.id == *notified_player_id) {
                    all_players_missing = false;

                    // Calculate proper 3D distance
                    let dx = player.position.x - target_position.x;
                    let dy = player.position.y - target_position.y;
                    let dz = target_position.z;
                    let distance_3d = (dx * dx + dy * dy + dz * dz).sqrt();

                    if distance_3d < CLAIM_DISTANCE && self.current_owner.is_none() {
                        // Player reached the ball (or landing position), give them ownership when ball arrives
                        // For aerial balls, only claim when ball is low enough
                        if !self.is_aerial() || self.position.z < 2.5 {
                            claiming_player_id = Some(*notified_player_id);
                            break; // Ball claimed, no need to check other players
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
            && self.velocity.norm() < 1.0 // Increased from 0.1 to catch slow rolling balls
            && self.current_owner.is_none()
    }

    pub fn is_ball_outside(&self) -> bool {
        self.position.x <= 0.0
            || self.position.x >= self.field_width
            || self.position.y <= 0.0
            || self.position.y >= self.field_height
    }

    /// NUCLEAR OPTION: Force the nearest player to claim the ball if it's been sitting unowned for too long
    /// This is a last-resort failsafe to prevent deadlocks where no one claims the ball
    fn force_claim_if_deadlock(
        &mut self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        const DEADLOCK_VELOCITY_THRESHOLD: f32 = 10.0; // Catch slow-rolling balls in duels (increased from 3.0)
        const DEADLOCK_HEIGHT_THRESHOLD: f32 = 0.5; // Ball must be low to ground
        const DEADLOCK_TICK_THRESHOLD: u32 = 5; // Faster intervention (reduced from 10)
        const DEADLOCK_SEARCH_RADIUS: f32 = 300.0; // Increased search radius for force claiming

        // Check if ball is slow-moving/stopped and unowned
        let is_slow = self.velocity.norm() < DEADLOCK_VELOCITY_THRESHOLD;
        let is_low = self.position.z < DEADLOCK_HEIGHT_THRESHOLD;
        let is_unowned = self.current_owner.is_none();

        if is_slow && is_low && is_unowned {
            self.unowned_stopped_ticks += 1;

            // If ball has been slow and unowned for 5 ticks (~0.08 seconds), FORCE claiming
            if self.unowned_stopped_ticks >= DEADLOCK_TICK_THRESHOLD {
                // Find the nearest player within search radius
                if let Some(nearest_player) = players.iter()
                    .filter(|p| {
                        let distance = (p.position - self.position).magnitude();
                        distance <= DEADLOCK_SEARCH_RADIUS
                    })
                    .min_by(|a, b| {
                        let dist_a = (a.position - self.position).magnitude();
                        let dist_b = (b.position - self.position).magnitude();
                        dist_a.partial_cmp(&dist_b).unwrap()
                    }) {
                    // FORCE this player to go for the ball
                    events.add_ball_event(BallEvent::TakeMe(nearest_player.id));

                    // Reset counter
                    self.unowned_stopped_ticks = 0;

                    // Clear any existing notifications to avoid conflicts
                    self.take_ball_notified_players.clear();
                }
            }
        } else {
            // Ball is moving fast, airborne, or owned - reset counter
            self.unowned_stopped_ticks = 0;
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

        // Check if ball hits the boundary and reverse its velocity if it does
        if self.position.x <= 0.0 {
            self.position.x = 0.0;
            self.velocity = Vector3::zeros();
        }

        if self.position.x >= field_width {
            self.position.x = field_width;
            self.velocity = Vector3::zeros();
        }

        if self.position.y <= 0.0 {
            self.position.y = 0.0;
            self.velocity = Vector3::zeros();
        }

        if self.position.y >= field_height {
            self.position.y = field_height;
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
        // Increased to 15.0 for easier ball claiming - players can claim from further away
        const BALL_DISTANCE_THRESHOLD: f32 = 15.0;
        const BALL_DISTANCE_THRESHOLD_SQUARED: f32 = BALL_DISTANCE_THRESHOLD * BALL_DISTANCE_THRESHOLD;
        const PLAYER_HEIGHT: f32 = 1.8; // Average player height in meters
        const PLAYER_REACH_HEIGHT: f32 = PLAYER_HEIGHT + 0.5; // Player can reach ~2.3m when standing
        const PLAYER_JUMP_REACH: f32 = PLAYER_HEIGHT + 1.0; // Player can reach ~2.8m when jumping
        const MAX_BALL_HEIGHT: f32 = PLAYER_JUMP_REACH + 0.5; // Absolute max reachable height

        // Ball is too high to be claimed by any player (flying over everyone's heads)
        if self.position.z > MAX_BALL_HEIGHT {
            return;
        }

        // Check if previous owner is still within range (use 3D distance)
        // This prevents immediate ball reclaim after passing/shooting
        // BUT: Allow opponents to contest the ball even if previous owner is still close
        if let Some(previous_owner_id) = self.previous_owner {
            let owner = context.players.by_id(previous_owner_id).unwrap();

            // Calculate proper 3D distance
            let dx = owner.position.x - self.position.x;
            let dy = owner.position.y - self.position.y;
            let dz = self.position.z; // Ball height from ground (player is at z=0)
            let distance_3d = (dx * dx + dy * dy + dz * dz).sqrt();

            if distance_3d > BALL_DISTANCE_THRESHOLD {
                self.previous_owner = None;
            } else {
                // Previous owner still in range
                // Check if there are any opponents nearby who might contest the ball
                let has_opponent_contesting = players
                    .iter()
                    .filter(|p| p.team_id != owner.team_id)  // Opponents only
                    .any(|opponent| {
                        let dx = opponent.position.x - self.position.x;
                        let dy = opponent.position.y - self.position.y;
                        let horizontal_distance_squared = dx * dx + dy * dy;

                        // Check if opponent is close enough to contest
                        horizontal_distance_squared <= BALL_DISTANCE_THRESHOLD_SQUARED
                    });

                // If no opponents are contesting, return early to prevent previous owner from reclaiming
                // If opponents ARE contesting, continue to ownership check to allow fair challenge
                if !has_opponent_contesting {
                    return;
                }
                // Otherwise, continue to normal ownership check below
            }
        }

        // Find all players within ball distance threshold with proper 3D collision detection
        let nearby_players: Vec<&MatchPlayer> = players
            .iter()
            .filter(|player| {
                let dx = player.position.x - self.position.x;
                let dy = player.position.y - self.position.y;
                let horizontal_distance_squared = dx * dx + dy * dy;
                let horizontal_distance = horizontal_distance_squared.sqrt();

                // Early exit if horizontally too far
                if horizontal_distance_squared > BALL_DISTANCE_THRESHOLD_SQUARED {
                    return false;
                }

                // Calculate reachable height based on horizontal distance
                // Closer = easier to reach higher balls (can jump)
                // Further = harder to reach higher balls
                let effective_reach_height = if horizontal_distance < 1.5 {
                    // Very close - can jump and reach high
                    PLAYER_JUMP_REACH
                } else if horizontal_distance < 3.0 {
                    // Medium distance - can reach with feet/body
                    PLAYER_REACH_HEIGHT
                } else {
                    // Far distance - only low balls (sliding tackle range)
                    PLAYER_HEIGHT * 0.6
                };

                // Check if ball is at reachable height for this horizontal distance
                self.position.z <= effective_reach_height
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
                // Current owner is still close to the ball - maintain ownership
                self.ownership_duration += 1;
                return;
            }

            // Current owner is NOT nearby - clear ownership so ball can be claimed
            // This prevents the ball from being "owned" by a player who is far away
            self.previous_owner = self.current_owner;
            self.current_owner = None;

            // If only teammates are nearby, they can now claim the ball
            // If opponents are nearby, they compete for it
            // This prevents the rapid position changes caused by inconsistent ownership state
        }

        // Ownership stability constants
        const MIN_OWNERSHIP_DURATION: u32 = 8; // Minimum ticks before ownership can change (reduced from 15 to prevent stuck duels)
        const TAKEOVER_ADVANTAGE_THRESHOLD: f32 = 1.08; // Challenger must be 8% better (reduced from 1.15 to resolve duels faster)

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

                // Ownership change approved - reset duration
                self.previous_owner = self.current_owner;
                self.current_owner = Some(player.id);
                self.ownership_duration = 0;
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
        const DRAG_COEFFICIENT: f32 = 0.25;
        const ROLLING_RESISTANCE_COEFFICIENT: f32 = 0.02;
        const STOPPING_THRESHOLD: f32 = 0.1;
        const TIME_STEP: f32 = 0.01;
        const BOUNCE_COEFFICIENT: f32 = 0.6; // Ball keeps 60% of velocity when bouncing

        let velocity_norm = self.velocity.norm();

        if velocity_norm > STOPPING_THRESHOLD {
            // Apply air drag when ball is in the air or rolling
            let drag_force =
                -0.5 * DRAG_COEFFICIENT * velocity_norm * velocity_norm * self.velocity.normalize();

            // Apply rolling resistance only when ball is on the ground (z ~= 0)
            let rolling_resistance_force = if self.position.z <= 0.1 {
                Vector3::new(
                    -ROLLING_RESISTANCE_COEFFICIENT * BALL_MASS * GRAVITY * self.velocity.x.signum(),
                    -ROLLING_RESISTANCE_COEFFICIENT * BALL_MASS * GRAVITY * self.velocity.y.signum(),
                    0.0
                )
            } else {
                Vector3::zeros()
            };

            // Apply gravity force in z-direction (downward)
            let gravity_force = Vector3::new(0.0, 0.0, -BALL_MASS * GRAVITY);

            let total_force = drag_force + rolling_resistance_force + gravity_force;
            let acceleration = total_force / BALL_MASS;

            self.velocity += acceleration * TIME_STEP;
        } else {
            // Ball has stopped - zero velocity and settle on ground
            self.velocity = Vector3::zeros();
            self.position.z = 0.0;
        }

        // Check ground collision and bounce
        if self.position.z <= 0.0 && self.velocity.z < 0.0 {
            // Ball hit the ground
            self.position.z = 0.0;
            self.velocity.z = -self.velocity.z * BOUNCE_COEFFICIENT;

            // If bounce is too small, stop vertical movement
            if self.velocity.z.abs() < 0.5 {
                self.velocity.z = 0.0;
            }
        }
    }

    fn move_to(&mut self, tick_context: &GameTickContext) {
        // Clear notified players only when ball state changes significantly:
        // 1. Ball starts moving (not stopped anymore)
        // 2. Ball has an owner (claimed)
        const MOVEMENT_THRESHOLD: f32 = 0.5; // Ball is considered moving above this velocity

        let is_moving = self.velocity.norm() > MOVEMENT_THRESHOLD;
        let has_owner = self.current_owner.is_some();

        // Clear notifications when ball is no longer in a "take ball" scenario
        if (is_moving || has_owner) && !self.take_ball_notified_players.is_empty() {
            self.take_ball_notified_players.clear();
        }

        if let Some(owner_player_id) = self.current_owner {
            self.position = tick_context.positions.players.position(owner_player_id);
            // When player has the ball, it should be at ground level (or slightly above for dribbling)
            self.position.z = 0.0;
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
    }
}
