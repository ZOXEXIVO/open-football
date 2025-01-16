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

    #[inline]
    pub fn speed(&self) -> f32 {
        self.ctx.tick_context.positions.ball.velocity.norm()
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
