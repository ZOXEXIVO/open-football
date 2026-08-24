use crate::r#match::Ball;
use nalgebra::Vector3;

pub struct BallFieldData {
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    /// Rotation on the ball, in the same frame `SpinModel` stores it.
    ///
    /// Here because a state that wants to know where a ball in flight is
    /// GOING has to integrate the same forces the physics applies, and on
    /// a struck ball Magnus is not a rounding term — it is the dip that
    /// puts a shot under the bar. Without it `KeeperShotSave::roll` would
    /// project a different flight from the one `Ball::try_save_shot`
    /// projects, and the two must never disagree about the same ball.
    /// See [`Ball::ballistic_crossing`].
    pub spin: Vector3<f32>,
    pub landing_position: Vector3<f32>,
}

impl BallFieldData {
    #[inline]
    pub fn update_from(&mut self, ball: &Ball) {
        self.position = ball.position;
        self.velocity = ball.velocity;
        self.spin = ball.spin;
        self.landing_position = ball.cached_landing_position;
    }
}

impl From<&Ball> for BallFieldData {
    #[inline]
    fn from(ball: &Ball) -> Self {
        BallFieldData {
            position: ball.position,
            velocity: ball.velocity,
            spin: ball.spin,
            landing_position: ball.cached_landing_position,
        }
    }
}
