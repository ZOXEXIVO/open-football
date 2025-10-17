use crate::r#match::ball::events::{BallEvent, BallGoalEventMetadata, GoalSide};
use crate::r#match::events::EventCollection;
use crate::r#match::result::VectorExtensions;
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
    pub take_ball_notified_player: Option<u32>,
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
            take_ball_notified_player: None,
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
        if self.is_stands_outside()
            && self.take_ball_notified_player.is_none()
            && self.current_owner.is_none()
        {
            if let Some(notified_player) = self.notify_nearest_player(players, events) {
                self.take_ball_notified_player = Some(notified_player);
            }
        } else if let Some(notified_player_id) = self.take_ball_notified_player {
            // Check if the notified player reached the ball
            if let Some(player) = players.iter().find(|p| p.id == notified_player_id) {
                const CLAIM_DISTANCE: f32 = 8.0; // Slightly larger than normal ownership distance
                let distance = player.position.distance_to(&self.position);

                if distance < CLAIM_DISTANCE && self.current_owner.is_none() {
                    // Player reached the ball, give them ownership
                    self.current_owner = Some(notified_player_id);
                    self.take_ball_notified_player = None;
                    events.add_ball_event(BallEvent::Claimed(notified_player_id));
                }
            }
        }
    }

    pub fn try_intercept(&mut self, _players: &[MatchPlayer], _events: &mut EventCollection) {}

    pub fn is_stands_outside(&self) -> bool {
        self.is_ball_outside()
            && self.velocity.x == 0.0
            && self.velocity.y == 0.0
            && self.current_owner.is_none()
    }

    pub fn is_ball_outside(&self) -> bool {
        self.position.x == 0.0
            || self.position.x >= self.field_width
            || self.position.y == 0.0
            || self.position.y >= self.field_height
    }

    fn notify_nearest_player(
        &self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) -> Option<u32> {
        let ball_position = self.position;

        // Find the nearest player to the ball
        let nearest_player = players.iter().min_by(|a, b| {
            let dist_a = a.position.distance_to(&ball_position);
            let dist_b = b.position.distance_to(&ball_position);
            dist_a
                .partial_cmp(&dist_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if let Some(player) = nearest_player {
            events.add_ball_event(BallEvent::TakeMe(player.id));

            return Some(player.id);
        }

        None
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
        const BALL_DISTANCE_THRESHOLD: f32 = 5.0;
        const BALL_DISTANCE_THRESHOLD_SQUARED: f32 = BALL_DISTANCE_THRESHOLD * BALL_DISTANCE_THRESHOLD;
        const PLAYER_HEIGHT: f32 = 1.8; // Average player height in meters
        const BALL_HEIGHT_THRESHOLD: f32 = PLAYER_HEIGHT * 1.2; // Ball must be within reachable height

        // Ball is too high to be claimed by players (flying over their heads)
        if self.position.z > BALL_HEIGHT_THRESHOLD {
            return;
        }

        // Check if previous owner is still within range
        if let Some(previous_owner_id) = self.previous_owner {
            let owner = context.players.by_id(previous_owner_id).unwrap();
            if owner.position.distance_to(&self.position) > BALL_DISTANCE_THRESHOLD {
                self.previous_owner = None;
            } else {
                // Previous owner still in range, no need to check for new ownership
                return;
            }
        }

        // Find all players within ball distance threshold (2D distance on ground)
        let nearby_players: Vec<&MatchPlayer> = players
            .iter()
            .filter(|player| {
                let dx = player.position.x - self.position.x;
                let dy = player.position.y - self.position.y;
                let horizontal_distance_squared = dx * dx + dy * dy;

                // Check if ball is within horizontal range and at reachable height
                horizontal_distance_squared < BALL_DISTANCE_THRESHOLD_SQUARED
                    && self.position.z <= BALL_HEIGHT_THRESHOLD
            })
            .collect();

        // Early exit if no nearby players
        if nearby_players.is_empty() {
            return;
        }

        // Check if current owner is nearby and prevent teammate takeover
        if let Some(current_owner_id) = self.current_owner {
            let current_owner = context.players.by_id(current_owner_id).unwrap();

            // Check if current owner is still nearby
            let current_owner_nearby = nearby_players
                .iter()
                .any(|player| player.id == current_owner_id);

            if current_owner_nearby {
                return;
            }

            // Check if any nearby player is a teammate of the current owner
            let same_team_nearby = nearby_players
                .iter()
                .any(|p| p.team_id == current_owner.team_id);

            if same_team_nearby {
                // Don't transfer ownership to teammates - they should maintain positions
                return;
            }
        }

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

        // Transfer ownership to the best tackler
        if let Some(player) = best_tackler {
            self.previous_owner = self.current_owner;
            self.current_owner = Some(player.id);
            events.add_ball_event(BallEvent::Claimed(player.id));
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
            self.velocity = Vector3::zeros();
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
        if !self.is_stands_outside() {
            self.take_ball_notified_player = None;
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
