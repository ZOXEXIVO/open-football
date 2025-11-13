use crate::r#match::result::VectorExtensions;
use crate::r#match::{BallSide, PlayerSide, StateProcessingContext};
use nalgebra::Vector3;

pub struct BallOperationsImpl<'b> {
    ctx: &'b StateProcessingContext<'b>,
}

impl<'b> BallOperationsImpl<'b> {
    pub fn new(ctx: &'b StateProcessingContext<'b>) -> Self {
        BallOperationsImpl { ctx }
    }
}

impl<'b> BallOperationsImpl<'b> {
    pub fn on_own_side(&self) -> bool {
        match self.side() {
            BallSide::Left => self.ctx.player.side == Some(PlayerSide::Left),
            BallSide::Right => self.ctx.player.side == Some(PlayerSide::Right),
        }
    }

    pub fn distance(&self) -> f32 {
        self.ctx
            .tick_context
            .positions
            .ball
            .position
            .distance_to(&self.ctx.player.position)
    }

    pub fn velocity(&self) -> Vector3<f32> {
        self.ctx
            .tick_context
            .positions
            .ball
            .velocity        
    }

    #[inline]
    pub fn speed(&self) -> f32 {
        self.ctx.tick_context.positions.ball.velocity.norm()
    }

    #[inline]
    pub fn stopped(&self) -> bool {
        let velocity = self.ctx.tick_context.positions.ball.velocity;
        velocity.x == 0.0 && velocity.y == 0.0
    }

    #[inline]
    pub fn is_owned(&self) -> bool {
        self.ctx.tick_context.ball.is_owned
    }

    #[inline]
    pub fn is_in_flight(&self) -> bool {
        self.ctx.tick_context.ball.is_in_flight_state > 0
    }

    #[inline]
    pub fn owner_id(&self) -> Option<u32> {
        self.ctx.tick_context.ball.current_owner
    }

    #[inline]
    pub fn previous_owner_id(&self) -> Option<u32> {
        self.ctx.tick_context.ball.last_owner
    }

    pub fn is_towards_player(&self) -> bool {
        let (is_towards, _) = MatchBallLogic::is_heading_towards_player(
            &self.ctx.tick_context.positions.ball.position,
            &self.ctx.tick_context.positions.ball.velocity,
            &self.ctx.player.position,
            0.95,
        );
        is_towards
    }

    pub fn is_towards_player_with_angle(&self, angle: f32) -> bool {
        let (is_towards, _) = MatchBallLogic::is_heading_towards_player(
            &self.ctx.tick_context.positions.ball.position,
            &self.ctx.tick_context.positions.ball.velocity,
            &self.ctx.player.position,
            angle,
        );
        is_towards
    }

    pub fn distance_to_own_goal(&self) -> f32 {
        let target_goal = match self.ctx.player.side {
            Some(PlayerSide::Left) => Vector3::new(
                self.ctx.context.goal_positions.left.x,
                self.ctx.context.goal_positions.left.y,
                0.0,
            ),
            Some(PlayerSide::Right) => Vector3::new(
                self.ctx.context.goal_positions.right.x,
                self.ctx.context.goal_positions.right.y,
                0.0,
            ),
            _ => Vector3::new(0.0, 0.0, 0.0),
        };

        self.ctx
            .tick_context
            .positions
            .ball
            .position
            .distance_to(&target_goal)
    }

    pub fn direction_to_own_goal(&self) -> Vector3<f32> {
        match self.ctx.player.side {
            Some(PlayerSide::Left) => self.ctx.context.goal_positions.left,
            Some(PlayerSide::Right) => self.ctx.context.goal_positions.right,
            _ => Vector3::new(0.0, 0.0, 0.0),
        }
    }

    pub fn direction_to_opponent_goal(&self) -> Vector3<f32> {
        self.ctx.player().opponent_goal_position()
    }

    pub fn distance_to_opponent_goal(&self) -> f32 {
        let target_goal = match self.ctx.player.side {
            Some(PlayerSide::Left) => Vector3::new(
                self.ctx.context.goal_positions.right.x,
                self.ctx.context.goal_positions.right.y,
                0.0,
            ),
            Some(PlayerSide::Right) => Vector3::new(
                self.ctx.context.goal_positions.left.x,
                self.ctx.context.goal_positions.left.y,
                0.0,
            ),
            _ => Vector3::new(0.0, 0.0, 0.0),
        };

        self.ctx
            .tick_context
            .positions
            .ball
            .position
            .distance_to(&target_goal)
    }

    #[inline]
    pub fn on_own_third(&self) -> bool {
        let field_length = self.ctx.context.field_size.width as f32;
        let ball_x = self.ctx.tick_context.positions.ball.position.x;

        if self.ctx.player.side == Some(PlayerSide::Left) {
            // Home team defends the left side (negative X)
            ball_x < -field_length / 3.0
        } else {
            // Away team defends the right side (positive X)
            ball_x > field_length / 3.0
        }
    }

    pub fn in_own_penalty_area(&self) -> bool {
        // TODO
        let penalty_area = self
            .ctx
            .context
            .penalty_area(self.ctx.player.side == Some(PlayerSide::Left));

        let ball_position = self.ctx.tick_context.positions.ball.position;

        (penalty_area.min.x..=penalty_area.max.x).contains(&ball_position.x)
            && (penalty_area.min.y..=penalty_area.max.y).contains(&ball_position.y)
    }

    #[inline]
    pub fn side(&self) -> BallSide {
        if (self.ctx.tick_context.positions.ball.position.x as usize)
            <= self.ctx.context.field_size.half_width
        {
            return BallSide::Left;
        }

        BallSide::Right
    }

    /// Check if current player has been notified to take the ball
    #[inline]
    pub fn is_player_notified(&self) -> bool {
        self.ctx.tick_context.ball.notified_players.contains(&self.ctx.player.id)
    }

    /// Check if ball should be taken immediately (emergency situation)
    /// Common pattern: ball is nearby/notified, unowned, and stopped/slow-moving
    /// Increased distance and velocity thresholds to catch slow rolling balls
    pub fn should_take_ball_immediately(&self) -> bool {
        self.should_take_ball_immediately_with_distance(50.0) // Increased from 33.3
    }

    /// Check if ball should be taken immediately with custom distance threshold
    pub fn should_take_ball_immediately_with_distance(&self, distance_threshold: f32) -> bool {
        let is_nearby = self.distance() < distance_threshold;
        let is_notified = self.is_player_notified();

        if (is_nearby || is_notified) && !self.is_owned() {
            let ball_velocity = self.speed();
            if ball_velocity < 3.0 { // Increased from 1.0 to catch slow rolling balls
                return true;
            }
        }
        false
    }

    /// Check if ball is nearby and available (unowned or slow moving)
    pub fn is_nearby_and_available(&self, distance: f32) -> bool {
        self.distance() < distance && (!self.is_owned() || self.speed() < 2.0)
    }

    /// Check if ball is in attacking third relative to player's team
    pub fn in_attacking_third(&self) -> bool {
        let field_length = self.ctx.context.field_size.width as f32;
        self.distance_to_opponent_goal() < field_length / 3.0
    }

    /// Check if ball is in middle third of the field
    pub fn in_middle_third(&self) -> bool {
        !self.on_own_third() && !self.in_attacking_third()
    }

    /// Get field position as percentage (0.0 = own goal, 1.0 = opponent goal)
    pub fn field_position_percentage(&self) -> f32 {
        let field_length = self.ctx.context.field_size.width as f32;
        let distance_to_own = self.distance_to_own_goal();
        (distance_to_own / field_length).clamp(0.0, 1.0)
    }
}

pub struct MatchBallLogic;

impl MatchBallLogic {
    pub fn is_heading_towards_player(
        ball_position: &Vector3<f32>,
        ball_velocity: &Vector3<f32>,
        player_position: &Vector3<f32>,
        angle: f32,
    ) -> (bool, f32) {
        let velocity_xy = Vector3::new(ball_velocity.x, ball_velocity.y, 0.0);
        let ball_to_player_xy = Vector3::new(
            player_position.x - ball_position.x,
            player_position.y - ball_position.y,
            0.0,
        );

        let velocity_norm = velocity_xy.norm();
        let direction_norm = ball_to_player_xy.norm();

        let normalized_velocity = velocity_xy / velocity_norm;
        let normalized_direction = ball_to_player_xy / direction_norm;
        let dot_product = normalized_velocity.dot(&normalized_direction);

        (dot_product >= angle, dot_product)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::Vector3;

    #[test]
    fn test_is_heading_towards_player_true() {
        let ball_position = Vector3::new(0.0, 0.0, 0.0);
        let ball_velocity = Vector3::new(1.0, 1.0, 0.0);
        let player_position = Vector3::new(5.0, 5.0, 0.0);
        let angle = 0.9;

        let (result, dot_product) = MatchBallLogic::is_heading_towards_player(
            &ball_position,
            &ball_velocity,
            &player_position,
            angle,
        );

        assert!(result);
        assert!(dot_product > angle);
    }

    #[test]
    fn test_is_heading_towards_player_false() {
        let ball_position = Vector3::new(0.0, 0.0, 0.0);
        let ball_velocity = Vector3::new(1.0, 1.0, 0.0);
        let player_position = Vector3::new(-5.0, -5.0, 0.0);
        let angle = 0.9;

        let (result, dot_product) = MatchBallLogic::is_heading_towards_player(
            &ball_position,
            &ball_velocity,
            &player_position,
            angle,
        );

        assert!(!result);
        assert!(dot_product < angle);
    }

    #[test]
    fn test_is_heading_towards_player_perpendicular() {
        let ball_position = Vector3::new(0.0, 0.0, 0.0);
        let ball_velocity = Vector3::new(1.0, 0.0, 0.0);
        let player_position = Vector3::new(0.0, 5.0, 0.0);
        let angle = 0.9;

        let (result, dot_product) = MatchBallLogic::is_heading_towards_player(
            &ball_position,
            &ball_velocity,
            &player_position,
            angle,
        );

        assert!(!result);
        assert!(dot_product < angle);
    }
}
