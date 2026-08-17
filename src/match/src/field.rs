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
    /// The goal, as the PHYSICS keeps it — which is what the frame has to be
    /// drawn on, and the only goal dimension in this crate.
    ///
    /// The engine's `GOAL_WIDTH` is 29 game units, i.e. 3.625 m to a post:
    /// the 0.125 m grid's nearest step to the regulation 3.66, so its goal is
    /// 7 cm narrower than a real one. `GOAL_HEIGHT` is the metric 2.44 and is
    /// exact. Since `ball/frame.rs` now rebounds the ball off real posts and
    /// a real crossbar, drawing the regulation figures instead would put
    /// every post 3.5 cm wide of where the physics keeps one — a ball seen
    /// bouncing off thin air beside the woodwork, which is the class of
    /// artefact the frame exists to remove. One source of truth wins over
    /// one per cent of accuracy.
    ///
    /// Both are restated here rather than shared, for the same reason
    /// [`Self::NET_DEPTH`] is: this crate cannot depend on `core`. If either
    /// moves there, it moves here.
    pub const PHYSICS_GOAL_HALF_WIDTH: f32 = 29.0 * Self::METERS_PER_UNIT;
    pub const PHYSICS_GOAL_HEIGHT: f32 = 2.44;
    /// Radius of a post or the bar, matching `GoalFrame::POST_RADIUS`. The
    /// Laws cap the thickness at 12 cm.
    pub const POST_RADIUS: f32 = 0.06;

    /// Goal-line to the back of the net, and the height of the netting where
    /// it gets there.
    ///
    /// These must agree with the engine's `GoalNet::DEPTH` (15.2 game units)
    /// and `GoalNet::BACK_HEIGHT`, because the engine settles the ball inside
    /// that volume and this is where the volume gets drawn. If they drift,
    /// the ball comes to rest somewhere the netting isn't.
    pub const NET_DEPTH: f32 = 1.9;
    pub const NET_BACK_HEIGHT: f32 = 1.15;

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
