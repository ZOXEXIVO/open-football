use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, GOALKEEPER_JADEDNESS_INCREMENT,
    GOALKEEPER_JADEDNESS_INTERVAL, GOALKEEPER_LOW_CONDITION_THRESHOLD,
};
use crate::r#match::{PlayerSide, StateProcessingContext};
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
/// Where the keeper stands during OPEN PLAY — as distinct from
/// [`KeeperSetPosition`], which is where he sets himself to face a strike.
///
/// # Why this exists
///
/// Two states owned a keeper's resting position — `Standing` and
/// `Walking` — and each had its own copy of the model with different
/// constants, so the same keeper wanted to be in two different places
/// depending on which state he happened to be in. Both copies were also
/// wrong in the same way: their whole depth range came to about 1-4 m, so
/// a keeper never left his line. He stood motionless for **88% of the
/// match**, which is the "only the GK stays in the goal" report.
///
/// # The model
///
/// Depth is set by **where the ball is** and **how high his own defence
/// is**; lateral position by **the angle** — he stands on the line from
/// the middle of his goal to the ball, which is what narrowing the angle
/// physically means, and which gives him his side-to-side movement for
/// free.
///
/// Real reference points, which the constants reproduce:
///   * ball in the opponent's half behind a high line — 20-28 m out,
///     effectively a sweeper;
///   * ball in midfield — 12-18 m;
///   * ball entering our final third — 6-10 m;
///   * ball in the box — 2-5 m, on the angle.
pub struct KeeperRestPosition;

impl KeeperRestPosition {
    /// Off his line with the ball at the far end and the line high.
    /// 220u = 27.5 m.
    const SWEEP_DEPTH: f32 = 220.0;
    /// …and with the ball on top of him. 18u = 2.25 m.
    const NEAR_DEPTH: f32 = 18.0;
    /// He never closes to within this of his own back line — the space
    /// in behind is his to cover, not a free run for a striker.
    const BEHIND_LINE_GAP: f32 = 150.0;
    /// Inside this he is where he wants to be and stands set. A keeper
    /// standing still, set, is a real and common thing; without a
    /// deadzone he chases a target that moves every tick with the ball
    /// and covers more ground than a midfielder.
    ///
    /// 26u = 3.25 m. A keeper repositions in STEPS — he shuffles, then
    /// sets, then shuffles again — rather than gliding continuously after
    /// the ball. At 10u he tracked it every tick and covered 10.2 km
    /// against a real ~5 km.
    pub const SET_DEADZONE: f32 = 26.0;

    /// The spot, for a keeper defending `own_goal`, given where the ball
    /// is and where his side's defensive line is sitting.
    pub fn point(
        own_goal: Vector3<f32>,
        ball: Vector3<f32>,
        side: PlayerSide,
        defensive_line_x: f32,
        field_width: f32,
        command_of_area: f32,
        positioning: f32,
    ) -> Vector3<f32> {
        let to_ball = ball - own_goal;
        let ball_distance = to_ball.magnitude();

        // Depth rises with the ball's distance. SQUARED, so he drops onto
        // his line quickly as the ball comes into the final third and only
        // drifts back up slowly once it has gone — the asymmetry a keeper
        // actually plays with: getting back is urgent, pushing up is not.
        let far = (ball_distance / field_width).clamp(0.0, 1.0);
        let mut depth = Self::NEAR_DEPTH + (Self::SWEEP_DEPTH - Self::NEAR_DEPTH) * far * far;
        // Temperament: a commanding keeper sweeps, a line-keeper does not.
        depth *= 0.65 + command_of_area.clamp(0.0, 1.0) * 0.55;

        // Tethered to his own defensive line. A keeper 30 m off his line
        // behind a deep block is lost, not brave; behind a high line, one
        // on his goal line leaves 40 m of grass nobody covers.
        // `defensive_line_x` is the same reference the back four uses, so
        // keeper and defence agree where the space in behind is.
        let line_progress = side.attacking_progress_x(defensive_line_x, field_width);
        let line_depth =
            (line_progress * field_width - Self::BEHIND_LINE_GAP).max(Self::NEAR_DEPTH);
        depth = depth.min(line_depth).min(field_width * 0.5 - 20.0);

        // On the goal-to-ball line. A better-positioned keeper sits truer
        // on the angle; a poorer one hedges centrally and can be beaten at
        // his near post.
        let toward_ball = if ball_distance > 1.0 {
            to_ball / ball_distance
        } else {
            Vector3::new(side.forward_dir_x(), 0.0, 0.0)
        };
        let on_angle = own_goal + toward_ball * depth;
        let fidelity = 0.55 + positioning.clamp(0.0, 1.0) * 0.45;
        Vector3::new(
            on_angle.x,
            own_goal.y + (on_angle.y - own_goal.y) * fidelity,
            0.0,
        )
    }

    /// How fast he travels to it — set by how near the BALL is, not by how
    /// far he has to go. That is the difference between a keeper strolling
    /// to the edge of his box while play is at the other end and the same
    /// keeper sprinting to the same spot with a striker running through.
    pub fn pace(ball_distance: f32, field_width: f32) -> f32 {
        let urgency = (1.0 - ball_distance / (field_width * 0.55)).clamp(0.0, 1.0);
        0.18 + urgency * urgency * 1.12
    }
}

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

/// Whether a loose ball is the keeper's to claim, or somebody else's.
///
/// Every claim gate asked only about the BALL — is it loose, is it near,
/// is it on our side — and never about the people standing round it. In
/// a crowded six-yard box that is always true of something, because
/// possession there flickers constantly: a touch, a challenge or an
/// owner drifting past the tracking cutoff leaves the ball momentarily
/// unowned, and the keeper is by definition the man standing next to it.
/// So he collected it, distributed it, an attacker took it two metres
/// away, and the next stray touch handed it straight back to him — the
/// ball ping-ponging between the keeper and the players in front of him
/// on one spot, which is the reported symptom. Measured: 47.7 gathers a
/// match against a real 8-12.
///
/// A keeper does not grab a ball an opponent is on. He comes for the one
/// he can get to first, and lets his defenders deal with the rest. That
/// is the question this asks, and it is the same question the attacker
/// is implicitly asking, so the two cannot both claim it.
pub struct KeeperBallClaim;

impl KeeperBallClaim {
    /// How much nearer the keeper is allowed to let an opponent be while
    /// still going for it. He has hands and a dive, so he wins a ball
    /// he is marginally further from — but only marginally. 8u = 1 m.
    const HANDS_ADVANTAGE: f32 = 8.0;

    /// Is this keeper favourite for the loose ball in front of him?
    pub fn is_favourite(ctx: &StateProcessingContext) -> bool {
        let ball = ctx.tick_context.positions.ball.position;
        let mine = (ball - ctx.player.position).magnitude() - Self::HANDS_ADVANTAGE;
        !ctx.players()
            .opponents()
            .all()
            .any(|opp| (ball - opp.position).magnitude() < mine)
    }
}

/// Goalkeeper condition processor (type alias for clarity)
pub type GoalkeeperCondition = ConditionProcessor<GoalkeeperConfig>;

// Re-export for convenience
pub use crate::r#match::engine::player::strategies::common::ActivityIntensity;
