use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, GOALKEEPER_JADEDNESS_INCREMENT,
    GOALKEEPER_JADEDNESS_INTERVAL, GOALKEEPER_LOW_CONDITION_THRESHOLD,
};
use nalgebra::Vector3;

/// Goalkeeper-specific activity intensity configuration
pub struct GoalkeeperConfig;

impl ActivityIntensityConfig for GoalkeeperConfig {
    fn very_high_fatigue() -> f32 {
        7.0 // Lower than outfield players - explosive but infrequent
    }

    fn high_fatigue() -> f32 {
        4.5 // Lower than outfield players
    }

    fn moderate_fatigue() -> f32 {
        2.5
    }

    fn low_fatigue() -> f32 {
        0.8
    }

    fn recovery_rate() -> f32 {
        -4.0 // Better recovery than outfield players
    }

    fn sprint_multiplier() -> f32 {
        1.3 // Sprinting (less demanding than outfield players)
    }

    fn jogging_multiplier() -> f32 {
        0.5
    }

    fn walking_multiplier() -> f32 {
        0.2
    }

    fn low_condition_threshold() -> i16 {
        GOALKEEPER_LOW_CONDITION_THRESHOLD
    }

    fn jadedness_interval() -> u64 {
        GOALKEEPER_JADEDNESS_INTERVAL
    }

    fn jadedness_increment() -> i16 {
        GOALKEEPER_JADEDNESS_INCREMENT
    }
}

/// Where a keeper sets his feet to face a shot, and where he takes the
/// ball once he has gathered it.
///
/// Both save states used to steer to `(own_goal.x, goal_line_y)` — the
/// goal LINE itself. No keeper faces a shot standing on his line; he sets
/// a yard or so off it so he can attack the ball rather than carry it
/// back over the line behind him. It also had a very visible
/// consequence: the physics save snaps the ball to the keeper's
/// position, so every catch parked the ball at **x ≈ 839 of 840** at
/// glove height — hanging inside the goal frame, on the same spot, about
/// 160 times a match. Measured from a replay dump: the ball sat at
/// (839.3, 223.6, 1.20) without moving for 3.5 seconds.
///
/// UNITS: 1 unit = 0.125 m.
pub struct KeeperSetPosition;

impl KeeperSetPosition {
    /// Set position for a point-blank strike — 1.25 m off the line.
    /// Tight, because there is no time to come and meet it.
    const MIN_DEPTH: f32 = 10.0;
    /// Set position against a long-range effort — 3.5 m off, where a
    /// keeper stands when he can see it coming.
    const MAX_DEPTH: f32 = 28.0;
    /// Shot range over which the depth opens up (~11 m).
    const DEPTH_RANGE: f32 = 90.0;
    /// Where he takes the ball to release it — 10.6 m out, around the
    /// edge of the six-yard box.
    const RELEASE_DEPTH: f32 = 85.0;

    /// `+1` when the goal being defended is the left one, so "out of the
    /// goal" is `+x`; `-1` for the right-hand goal.
    fn into_pitch(own_goal: Vector3<f32>, field_width: f32) -> f32 {
        if own_goal.x <= field_width * 0.5 {
            1.0
        } else {
            -1.0
        }
    }

    /// The spot to defend a strike from `shot_distance` away, guarding
    /// `goal_line_y`.
    pub fn set_point(
        own_goal: Vector3<f32>,
        goal_line_y: f32,
        shot_distance: f32,
        field_width: f32,
    ) -> Vector3<f32> {
        let opened = (shot_distance / Self::DEPTH_RANGE).clamp(0.0, 1.0);
        let depth = Self::MIN_DEPTH + (Self::MAX_DEPTH - Self::MIN_DEPTH) * opened;
        Vector3::new(
            own_goal.x + Self::into_pitch(own_goal, field_width) * depth,
            goal_line_y,
            0.0,
        )
    }

    /// Where a keeper walks the ball once it is in his gloves. Real
    /// keepers get up and carry it out to the edge of their area to
    /// release it; this engine's stood exactly where it caught it for up
    /// to five and a half seconds.
    ///
    /// He walks OUT from his line while holding most of his lateral
    /// position, drifting gently back towards the middle. Keeping it
    /// continuous in where he gathered the ball matters: an absolute
    /// target would just move the single point everyone converges on
    /// further off the line rather than removing it.
    pub fn release_point(
        own_goal: Vector3<f32>,
        keeper: Vector3<f32>,
        field_width: f32,
    ) -> Vector3<f32> {
        Vector3::new(
            own_goal.x + Self::into_pitch(own_goal, field_width) * Self::RELEASE_DEPTH,
            keeper.y * 0.65 + own_goal.y * 0.35,
            0.0,
        )
    }
}

/// Goalkeeper condition processor (type alias for clarity)
pub type GoalkeeperCondition = ConditionProcessor<GoalkeeperConfig>;

// Re-export for convenience
pub use crate::r#match::engine::player::strategies::common::ActivityIntensity;
