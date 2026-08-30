use crate::r#match::engine::ball::ball::GROUND_FRICTION;

/// How far a ball rolling on the ground will still travel.
///
/// # Why this has to exist
///
/// [`Ball::calculate_landing_position`] answers "where will it come
/// down", and returns the ball's CURRENT position for anything already
/// on the turf. That is correct for what it is asked, and it means the
/// engine has never had an answer for the commonest loose ball there is:
/// one rolling flat across the grass. Every chaser reads
/// `landing_position`, so for a ground pass every chaser was reading
/// "where the ball is standing", and running at it.
///
/// Rolling is the one phase of a ball's life with a closed form. Ground
/// friction is a fixed proportional loss per tick, so the speed is a
/// geometric decay and the distance is its sum — no integration loop, no
/// bisection, and the same [`GROUND_FRICTION`] the physics uses, so the
/// prediction and the ball cannot drift apart.
pub struct BallRoll;

impl BallRoll {
    /// Speed at which [`Ball::update_velocity`] stops applying friction
    /// and lets the ball sit — mirrored from its `STOPPING_THRESHOLD`.
    pub const STOPPED: f32 = 0.05;

    /// Fraction of its speed a rolling ball keeps each tick.
    const KEPT: f32 = 1.0 - GROUND_FRICTION;

    /// Total ground a ball rolling at `speed` u/tick will ever cover
    /// before it comes to rest, in units.
    ///
    /// The sum of the geometric decay, stopped where the physics stops
    /// it: `(v − v_stop) / (1 − k)`. At the engine's 15%-per-second loss
    /// that is 625 × the surplus speed, so a firm ground pass genuinely
    /// runs most of the length of a pitch — which is why a chaser who
    /// aims at where it is standing never gets near it.
    #[inline]
    pub fn range(speed: f32) -> f32 {
        ((speed - Self::STOPPED) * Self::KEPT / (1.0 - Self::KEPT)).max(0.0)
    }

    /// Ticks until a ball rolling at `speed` decays to
    /// [`STOPPED`](Self::STOPPED) and sits.
    ///
    /// The inverse of the decay `distance` sums: `kᵗ = v_stop / v`. This
    /// is the natural time horizon for any question about the whole of a
    /// roll — past it the ball is a fixed point at
    /// [`range`](Self::range).
    pub fn rest_ticks(speed: f32) -> f32 {
        if speed <= Self::STOPPED {
            return 0.0;
        }
        (Self::STOPPED / speed).ln() / Self::KEPT.ln()
    }

    /// Ground covered by that ball in `ticks` ticks.
    ///
    /// Continuous in both arguments and saturating at
    /// [`range`](Self::range), so an arbitrarily distant time horizon
    /// simply returns the resting point instead of running off the map —
    /// which is what makes it safe to ask this for a chase the runner
    /// cannot win.
    pub fn distance(speed: f32, ticks: f32) -> f32 {
        if !(ticks > 0.0) || speed <= Self::STOPPED {
            return 0.0;
        }
        // The physics decays the speed BEFORE it steps the position, so
        // the first tick already moves at `v·k` and the sum carries a
        // leading `k`. Dropping it over-predicts by 0.16% — half a unit
        // over a full-length roll, which is exactly the error the
        // agreement test caught.
        let travelled = speed * Self::KEPT * (1.0 - Self::KEPT.powf(ticks)) / (1.0 - Self::KEPT);
        travelled.min(Self::range(speed))
    }
}
