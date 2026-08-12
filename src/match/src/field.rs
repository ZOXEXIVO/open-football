use bevy::prelude::Vec3;

/// The pitch as the match engine sees it: an 840 x 545 grid with the origin in
/// one corner. One grid unit is 0.125 m, so the playing surface is
/// 105 x 68.125 m — a regulation pitch.
///
/// Everything downstream of the recording works in metres with the pitch
/// centred on the world origin, Y up, so [`Field::to_world`] is the single
/// place the two coordinate systems meet.
pub struct Field;

impl Field {
    pub const UNITS_X: f32 = 840.0;
    pub const UNITS_Y: f32 = 545.0;
    pub const METERS_PER_UNIT: f32 = 0.125;

    /// Goal-line to goal-line, in metres.
    pub const LENGTH: f32 = Self::UNITS_X * Self::METERS_PER_UNIT;
    /// Touchline to touchline, in metres.
    pub const WIDTH: f32 = Self::UNITS_Y * Self::METERS_PER_UNIT;

    pub const HALF_LENGTH: f32 = Self::LENGTH * 0.5;
    pub const HALF_WIDTH: f32 = Self::WIDTH * 0.5;

    /// Regulation markings, in metres.
    pub const PENALTY_AREA_DEPTH: f32 = 16.5;
    pub const PENALTY_AREA_WIDTH: f32 = 40.32;
    pub const GOAL_AREA_DEPTH: f32 = 5.5;
    pub const GOAL_AREA_WIDTH: f32 = 18.32;
    pub const PENALTY_SPOT_DISTANCE: f32 = 11.0;
    pub const CENTRE_CIRCLE_RADIUS: f32 = 9.15;
    pub const CORNER_ARC_RADIUS: f32 = 1.0;
    pub const GOAL_WIDTH: f32 = 7.32;
    pub const GOAL_HEIGHT: f32 = 2.44;

    /// Recorded engine coordinates to world space (metres).
    ///
    /// `x` and `y` are grid units and get scaled; **`z` is already in
    /// metres** and does not. The engine's vertical axis is metric — its
    /// crossbar is 2.44, its jump reach 3.5 — while the horizontal plane is
    /// the 0.125 m grid. Scaling the height as if it were a grid unit made
    /// every ball fly at an eighth of its recorded altitude.
    ///
    /// The recorder omits `z` for anything standing on the ground, which
    /// deserialises back to zero.
    pub fn to_world(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(
            (x - Self::UNITS_X * 0.5) * Self::METERS_PER_UNIT,
            z,
            (y - Self::UNITS_Y * 0.5) * Self::METERS_PER_UNIT,
        )
    }
}
