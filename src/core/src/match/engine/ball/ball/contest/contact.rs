//! The switch that decides whether a contact writes the ball onto
//! the man who made it, or leaves it where it was.

/// The A/B arm for "a contact resolves where the BALL is".
///
/// Three resolvers used to finish by writing the ball onto the man who
/// made the contact — `Ball::try_block_shot`'s block, `Ball::try_intercept`
/// and `Ball::apply_failed_first_touch` — in the same way
/// `secure_ball_for` did before it, and for the same reason: the outcome
/// was decided first and the ball was moved to it. Armed (the default),
/// each of them leaves the ball where it was and lets the existing
/// machinery close the gap: `move_to` draws an owned ball in at 1.5
/// u/tick, and [`Ball::sink_to_ground`] gives a knocked-down one a descent
/// instead of an assignment.
///
/// It carries a switch of its own, separately from the corner's, because
/// it is the one part of this work that touches the numbers the second-ball
/// model is calibrated on — where a blocked or mis-controlled ball lands
/// decides who gets it next. `OF_TOUCH_IN_PLACE=off` restores the writes.
pub struct ContactInPlace;

impl ContactInPlace {
    /// False only when `OF_TOUCH_IN_PLACE=off`.
    pub fn armed() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            std::env::var("OF_TOUCH_IN_PLACE")
                .map(|v| v != "off" && v != "0")
                .unwrap_or(true)
        })
    }
}
