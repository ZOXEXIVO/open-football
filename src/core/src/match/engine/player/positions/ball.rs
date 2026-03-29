use crate::r#match::Ball;
use nalgebra::Vector3;

pub struct BallFieldData {
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    pub landing_position: Vector3<f32>,
}

impl BallFieldData {
    #[inline]
    pub fn update_from(&mut self, ball: &Ball) {
        self.position = ball.position;
        self.velocity = ball.velocity;
        self.landing_position = ball.cached_landing_position;
    }
}

impl From<&Ball> for BallFieldData {
    #[inline]
    fn from(ball: &Ball) -> Self {
        BallFieldData {
            position: ball.position,
            velocity: ball.velocity,
            landing_position: ball.cached_landing_position,
        }
    }
}