use nalgebra::Vector3;
use crate::r#match::Ball;

pub struct BallFieldData {
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
}

impl From<&Ball> for BallFieldData {
    #[inline]
    fn from(ball: &Ball) -> Self {
        BallFieldData {
            position: ball.position,
            velocity: ball.velocity,
        }
    }
}