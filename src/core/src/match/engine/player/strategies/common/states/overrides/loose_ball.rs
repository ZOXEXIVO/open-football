use crate::r#match::engine::ball::ball::{BallRoll, RunOff};
use crate::r#match::{StateProcessingContext, SteeringBehavior};
use nalgebra::Vector3;

/// The rule that keeps a race for a loose ball a RACE.
///
/// # Why this exists
///
/// All four `TakeBall` states add a plain repulsion from every player
/// within 25u — team-mates AND opponents — to their pursuit of the ball,
/// at up to 0.4 of top speed. So an opponent standing between the chaser
/// and the ball produces a force pointing away from that opponent, which
/// is to say **away from the ball**, and the chaser visibly hangs off
/// while a rival who started FURTHER away arrives first. Reported from
/// the viewer exactly that way: "defenders with Take Ball not running to
/// the ball, and the opponent is first even though he was further".
///
/// It applies for the whole approach, too — the states' `separation_factor`
/// is 1.0 at every distance beyond 10u and only ramps down inside that,
/// i.e. only once the chaser is already within claim range.
///
/// Real players do not give way to the man they are racing; they run the
/// same line and fight for it shoulder to shoulder. Separation still has
/// a job here — stopping four players stacking on one point — and that
/// job is entirely lateral, so keeping only the part of the force that
/// does not oppose the chase preserves it while making it impossible for
/// anybody to be pushed off the ball.
pub struct LooseBallChase;

impl LooseBallChase {
    /// Ball height below which the aim point is the ball itself.
    const GROUND_H: f32 = 1.5;
    /// Ball height above which the aim point is where it will land.
    const AERIAL_H: f32 = 3.0;

    /// Where to run for a loose ball, and the velocity to run there with.
    ///
    /// Returns `(target, velocity)` — the target is also what the caller
    /// measures its separation ramp against.
    ///
    /// # Why the aim point is blended
    ///
    /// This used to be `if ball.z > 2.3 { landing } else { ball_pos }`, with
    /// the steering switching between `Arrive` and `Pursuit` on the same
    /// test. A bouncing ball crosses 2.3 repeatedly, and the two targets can
    /// be tens of units apart in DIFFERENT directions — so the chaser's
    /// velocity **inverted on every crossing**. `dev_match trace` measured
    /// 6.6-7.8 velocity reversals per second with the player never leaving
    /// the state: a chaser visibly shivering next to a loose ball instead of
    /// collecting it. Crossing both the target and the behaviour smoothly
    /// across a height band means there is no height at which either can
    /// jump. (Smoothstep, so the aim point has zero gradient at both ends
    /// and no corner where it starts or finishes moving.)
    ///
    /// `Arrive` brakes into a landing spot; `Pursuit` leads a rolling ball.
    /// Seek alone would chase a moving ball's *current* position and always
    /// lag behind — fatal for a ground pass rolling through the chaser.
    pub fn aim(ctx: &StateProcessingContext) -> (Vector3<f32>, Vector3<f32>) {
        /// How hard the aerial branch brakes into the landing spot.
        /// Every caller passed this same number; it is here rather than
        /// in three argument lists.
        const SLOWING_DISTANCE: f32 = 10.0;

        let player = ctx.player;
        let ball = &ctx.tick_context.positions.ball;
        let (ball_pos, ball_vel, landing) = (ball.position, ball.velocity, ball.landing_position);

        let t = ((ball_pos.z - Self::GROUND_H) / (Self::AERIAL_H - Self::GROUND_H)).clamp(0.0, 1.0);
        let aerial = t * t * (3.0 - 2.0 * t);
        // The ground end of the aim point is where the ball is GOING, not
        // where it is standing — see [`Self::meeting_point`]. The aerial
        // end always was: `landing` is that same answer for a ball that
        // has to come down before anybody can play it.
        let rolling = Self::meeting_point(ctx, ball_pos, ball_vel);
        let target = rolling + (landing - rolling) * aerial;

        // ⚠ THE TWO BRANCHES ARE NOT INTERCHANGEABLE, AND COLLAPSING THEM
        // INTO ONE COSTS EVERYTHING THIS FIX BUYS.
        //
        // A ball in the air is going to come DOWN somewhere, and the only
        // useful place to be is there — `Arrive` at the landing spot, a
        // fixed point, braking into it. A ball on the grass is going to
        // keep running, and the only useful thing to do is hold the
        // bearing on it — `Intercept`, a feedback law on a moving target.
        //
        // Folding the aerial case into `Intercept` by handing it the
        // ball's own position and a zeroed velocity looks equivalent and
        // is not: it steers the chaser at the XY the ball is flying OVER
        // rather than the XY it will land on, which is a tail chase in
        // the air. Measured over 200 fixtures, that one substitution took
        // the census from 55% aimed-ahead / 34% parallel back to 48% /
        // 41% and tackles from 16.3 to 12.6 per team — i.e. it undid the
        // whole change while every line of the ground path stayed intact.
        let brake = || {
            SteeringBehavior::Arrive {
                target,
                slowing_distance: SLOWING_DISTANCE,
            }
            .calculate(player)
            .velocity
        };
        let cut_off = || {
            SteeringBehavior::Intercept {
                target: ball_pos,
                target_velocity: ball_vel,
            }
            .calculate(player)
            .velocity
        };

        let velocity = if aerial >= 1.0 {
            brake()
        } else if aerial <= 0.0 {
            cut_off()
        } else {
            cut_off() * (1.0 - aerial) + brake() * aerial
        };

        (target, velocity)
    }

    /// Diagnostic switch: with `OF_TAIL_CHASE` set, every state that runs
    /// at a loose ball reverts to aiming at where the ball IS.
    ///
    /// [`Self::meeting_point`] returns the ball's own position, and
    /// [`SteeringBehavior::Intercept`] falls back to the `Pursuit` it
    /// replaced — together, the aim point and the steering the engine had
    /// before 2026-08-22.
    ///
    /// ⚠ It is NOT the pre-2026-08-22 engine, and must not be read as
    /// one. `Pursuit` runs across the whole height band here, where
    /// [`Self::aim`] used to blend it against an `Arrive` at the landing
    /// spot, and the goalkeeper's chase was a bare `Seek` rather than
    /// either. Read it as "what does aiming at the ball do", not as
    /// "what did the engine used to score" — the same caveat
    /// `MovementEffort::chase_legacy` carries.
    ///
    /// This is the A/B control for the loose-ball chase work. It reaches
    /// every chase on every tick, so "did the interception model cause
    /// this?" cannot be read off the diff, and it must not be answered by
    /// checking out an older revision either, because the working tree
    /// moves underneath you. Same pattern and purpose as
    /// `MovementEffort::chase_legacy` and `MatchContext::shape_off`; read
    /// once per process. Debug infrastructure — do not remove.
    pub fn tail_chase() -> bool {
        use std::sync::OnceLock;
        static LEGACY: OnceLock<bool> = OnceLock::new();
        *LEGACY.get_or_init(|| std::env::var("OF_TAIL_CHASE").is_ok())
    }

    /// Where this player and a rolling ball can actually meet.
    ///
    /// # Why the engine had no answer for this
    ///
    /// `Ball::calculate_landing_position` returns the ball's own position
    /// for anything already on the turf, so for a ground pass every
    /// chaser in the engine was aiming at where the ball was standing —
    /// and `SteeringBehavior::Pursuit`, the one thing that was supposed
    /// to lead it, clamps its lead to 5 ticks (50 ms). Between them the
    /// aim point sat within half a metre of the ball on every tick of
    /// every ground chase, which is a tail chase however fast the chaser
    /// runs. `SteeringBehavior::Intercept` carries the measurement.
    ///
    /// # The solve
    ///
    /// The same decomposition the steering uses, so where a player runs
    /// and where he is steered can never become two different opinions:
    /// split the ball's travel into the part along our line of sight and
    /// the part across it, match the cross-track part, and close the gap
    /// with whatever speed is left over. What remains is the closing
    /// speed, the gap over it is the time, and [`BallRoll::distance`]
    /// says where the ball has rolled to by then.
    ///
    /// A chaser who cannot close at all is not a special case: the
    /// closing speed goes to nothing, the horizon runs away, and
    /// `BallRoll::distance` saturates at the resting point — so he is
    /// sent to where the ball will stop, which is the right answer and
    /// the same one, continuously, from either side of the boundary.
    ///
    /// Clamped to the run-off rather than to the pitch for the reason
    /// `calculate_landing_position` is: a ball on its way out of play
    /// finishes against the boards, and the man fetching it should be
    /// told where it actually comes to rest.
    pub fn meeting_point(
        ctx: &StateProcessingContext,
        ball_pos: Vector3<f32>,
        ball_vel: Vector3<f32>,
    ) -> Vector3<f32> {
        if Self::tail_chase() {
            return ball_pos;
        }
        let flat = |v: Vector3<f32>| Vector3::new(v.x, v.y, 0.0);
        let to_ball = flat(ball_pos) - flat(ctx.player.position);
        let gap = to_ball.norm();
        let ball_vel = flat(ball_vel);
        let ball_speed = ball_vel.norm();
        // Nothing to lead: a ball at rest, or one already at his feet.
        if gap < 1e-3 || ball_speed < BallRoll::STOPPED {
            return ball_pos;
        }
        let line_of_sight = to_ball / gap;
        let ball_dir = ball_vel / ball_speed;

        let speed = ctx.player.max_speed_with_condition_cached().max(1e-3);
        let across = ball_vel - line_of_sight * ball_vel.dot(&line_of_sight);
        let across_sq = across.norm_squared();
        // What he has left for the gap once he has matched the ball
        // across the line, less whatever the ball is taking along it.
        let closing = (speed * speed - across_sq).max(0.0).sqrt() - ball_vel.dot(&line_of_sight);
        let ticks = gap / closing.max(1e-3);

        let point = ball_pos + ball_dir * BallRoll::distance(ball_speed, ticks);
        let size = &ctx.context.field_size;
        let (min_x, max_x, min_y, max_y) =
            RunOff::ball_bounds(size.width as f32, size.height as f32);
        Vector3::new(
            point.x.clamp(min_x, max_x),
            point.y.clamp(min_y, max_y),
            0.0,
        )
    }

    /// Remove the component of `separation` that points against the run to
    /// the ball, leaving the lateral part untouched.
    ///
    /// A component that happens to push the chaser TOWARD the ball is left
    /// alone: it costs nothing, and stripping it as well would be its own
    /// arbitrary rule rather than a consequence of anything physical.
    pub fn keep_non_opposing(separation: Vector3<f32>, to_ball: Vector3<f32>) -> Vector3<f32> {
        let Some(chase_dir) = to_ball.try_normalize(1e-3) else {
            return separation;
        };
        let opposing = separation.dot(&chase_dir);
        if opposing < 0.0 {
            separation - chase_dir * opposing
        } else {
            separation
        }
    }
}
