//! **The ground outside the lines.**
//!
//! Report, 2026-08-20: *"the ball stops on the line behind the goal, but
//! must go beyond the goal, and the goalkeeper must put it into play. The
//! same applies elsewhere — the ball must not stop on the edge of the
//! field, but go beyond it."*
//!
//! Every out-of-play resolver used to write the ball onto its restart
//! spot on the tick it crossed the line, two units *inside* the pitch,
//! with the velocity and the spin zeroed:
//!
//! ```text
//!  366977   -1.8  235.5  0.31   v(-0.66, -0.09, 0.00)   crossed the byline
//! *366978    2.0  235.5  0.00   v( 0.00,  0.00, 0.00)   snapped back inside, dead
//! ```
//!
//! So a shot that missed by a foot stopped dead a hand's width the wrong
//! side of the line, and the keeper walked to a ball that had never left
//! the pitch. That is the report verbatim, and it is the last of the
//! ball's relocations: the goal kick's six-yard jump, the corner's
//! twenty-seven metres and the offside's through-ball run have all gone
//! ([[goal-kick-restart-teleport]], [[restarts-throw-in-and-offside]]),
//! and this one survived because it is *small* — 25 cm — not because it
//! is right. It is the one you actually watch, every time.
//!
//! # What the run-off is
//!
//! A pitch sits in a stadium, and the ball leaves the pitch before it
//! stops. This module owns that strip of ground: how far out it goes,
//! what stands at the far edge of it, and where a ball that has been put
//! out of play comes to rest in it.
//!
//! The two margins are **the viewer's own**, and that is the point of
//! taking them from there rather than inventing them. `Pitch::spawn_ground`
//! (`src/match/src/pitch.rs`) builds advertising hoardings at 3.4 m beyond
//! each touchline and 4.6 m behind each goal, and those boards are what a
//! ball rolling out of play actually hits. Put the physics anywhere else
//! and the ball either stops in open grass for no reason a watcher can
//! see, or rolls through the boards into the stand.
//!
//! # Why there has to be an outer bound at all
//!
//! Because the ball will not otherwise stop inside the ground. Rolling
//! friction is `GROUND_FRICTION = 0.0016` per 10 ms tick, applied
//! multiplicatively, so the free roll-out from a crossing speed `v0` is
//! about `(v0 - 0.05) / 0.0016` units:
//!
//! | crossing speed | | free roll |
//! |---|---|---|
//! | 0.66 u/tick (8.3 m/s) | the measured mean for a non-shot | **47 m** |
//! | 3.0 u/tick (37 m/s) | a shot going wide | **230 m** |
//!
//! Both leave the stadium, and both blow [`AwaitedRestart::CEILING`]. The
//! perimeter is not a safety net bolted onto the physics — it *is* the
//! mechanism that makes a run-out bounded, and the ball reaches it in
//! well under half a second, which is why the whole change costs almost
//! no dead-ball time.
//!
//! # The wall absorbs, it does not rebound
//!
//! A board that returned the ball to the pitch would re-open every
//! resolver and turn one restart into a loop. Hoardings and the ball-stop
//! netting behind them do not do that either: they take the pace off and
//! the ball drops. So contact kills the horizontal velocity and the spin
//! and leaves the vertical alone — the ball finishes falling, bounces
//! out on the spot, and settles.
//!
//! [`AwaitedRestart::CEILING`]: super::AwaitedRestart

use super::{AwaitedRestart, Ball};
use crate::r#match::engine::goal::GOAL_WIDTH;
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::RestartCensus;
use nalgebra::Vector3;

/// Which of the two horizontal axes a boundary plane lies across.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitAxis {
    /// The goal lines, at `x = 0` and `x = field_width`.
    Endline,
    /// The touchlines, at `y = 0` and `y = field_height`.
    Touchline,
}

/// What the perimeter did to the ball this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Perimeter {
    /// Still inside the run-off, travelling.
    Inside,
    /// It reached the boards and they took the pace off it.
    Boards,
    /// It went OVER them. The boards are a metre high; a ball above that
    /// is in the stand, and it is not coming back — see
    /// [`RunOff::BOARD_HEIGHT`].
    Crowd,
}

/// The strip of ground outside the pitch, and the perimeter at the end of
/// it.
pub struct RunOff;

impl RunOff {
    /// Behind each goal line, in game units. 36.8 u = 4.6 m — `END_MARGIN`
    /// in `Pitch::spawn_ground`, i.e. exactly where the hoardings stand.
    pub const END: f32 = 36.8;

    /// Beyond each touchline. 27.2 u = 3.4 m — `SIDE_MARGIN`, likewise.
    pub const SIDE: f32 = 27.2;

    /// **How high the boards are, in metres.** `HOARDING_HEIGHT` in
    /// `Pitch::spawn_ground`, and the reason it has to be here is the
    /// same reason [`Self::END`] and [`Self::SIDE`] are: the perimeter
    /// the physics uses and the perimeter the viewer draws are one wall.
    ///
    /// ⚠ **Without it the perimeter is a wall of infinite height**, and
    /// that is what a ball skied over the bar hits. Traced off a real
    /// run-out (`dev_match overbar`):
    ///
    /// ```text
    ///   392964  -35.44  300.75  3.80   v(-2.19, 1.22, -0.01)   crossing the run-off
    /// * 392965  -36.80  301.97  3.79   v( 0.00, 0.00, -0.01)   stopped dead, 3.8 m up
    ///   392966  -36.80  301.97  3.77   v( 0.00, 0.00, -0.02)   …and falls straight down
    /// ```
    ///
    /// — the ball hits a metre-high advertising board four metres above
    /// it, loses every scrap of horizontal pace and drops like a plumb
    /// line for two seconds. Above this the boards are not there.
    pub const BOARD_HEIGHT: f32 = 0.95;

    /// How much wider than the goal the ground behind it is treated as
    /// unreachable, in units. 4 u = 50 cm — a post's radius and a little
    /// clearance, so a ball resting against the outside of a post is not
    /// asked to be squeezed past.
    const GOAL_SHADOW_MARGIN: f32 = 4.0;

    /// How far short of the boards a player is allowed to go, in units.
    ///
    /// 8 u = 1 m. The hoardings are a metre high and 14 cm thick and the
    /// fetching player is drawn as a body, so standing him *on* the
    /// perimeter puts him inside them. He does not need to get there:
    /// [`AwaitedRestart::REACH`] is 12 u, so a ball against the boards is
    /// still comfortably inside the arm of a man held a metre short.
    ///
    /// [`AwaitedRestart::REACH`]: super::AwaitedRestart::REACH
    pub const PLAYER_INSET: f32 = 8.0;

    /// Below this the ball has stopped rolling, in units per tick.
    /// Matches `update_velocity`'s own `STOPPING_THRESHOLD`, so a ball the
    /// integrator considers stationary is one this considers settled.
    pub const STOPPED: f32 = 0.05;

    /// Longest a run-out may take **on the ground** before it is settled
    /// where it lies, in ticks. 300 = 3 s.
    ///
    /// The perimeter bounds the run-out by *distance* and normally
    /// resolves it inside 40 ticks. This bounds it by *time*, for the ball
    /// that trickles over the line at walking pace and has four metres of
    /// grass to cross: three seconds of a taker jogging out to meet it is
    /// football, ten is a stalled match.
    pub const TRAVEL_CEILING: u64 = 300;

    /// …and the absolute one, which an AIRBORNE ball is held to instead.
    ///
    /// A skied shot is not slow, it is high, and the perimeter cannot
    /// bound the axis it is travelling on. At the engine's own launch
    /// ceiling (`launch_speed_for_apex(40.0)`, 0.28 m/tick) a ball takes
    /// **570 ticks** to come down, and settling it before it lands would
    /// pin it in mid-air and then walk it to the grass at the restart's
    /// 10 cm/tick — thirty seconds of a ball descending like a lift, which
    /// is a worse artefact than the one being fixed.
    ///
    /// 900 ticks = 9 s is longer than any legal ball hangs and short
    /// enough that a runaway cannot stall the match behind it.
    pub const FLIGHT_CEILING: u64 = 900;

    /// False when `OF_RUN_OUT=off` — every restart is written onto its
    /// spot on the crossing tick, exactly as it used to be.
    ///
    /// The A/B is the same shape as [`DeadBall`](super::DeadBall) and
    /// [`CornerWalk`](super::CornerWalk), and for the same reason: the
    /// harness has to be able to measure the arm without a rebuild.
    pub fn armed() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            std::env::var("OF_RUN_OUT")
                .map(|v| v != "off" && v != "0")
                .unwrap_or(true)
        })
    }

    /// The rectangle a ball out of play may occupy, as
    /// `(min_x, max_x, min_y, max_y)`.
    #[inline]
    pub fn ball_bounds(field_width: f32, field_height: f32) -> (f32, f32, f32, f32) {
        (
            -Self::END,
            field_width + Self::END,
            -Self::SIDE,
            field_height + Self::SIDE,
        )
    }

    /// …and the one a player fetching it may, which is the same rectangle
    /// held [`Self::PLAYER_INSET`] short of the boards.
    #[inline]
    pub fn player_bounds(field_width: f32, field_height: f32) -> (f32, f32, f32, f32) {
        let end = Self::END - Self::PLAYER_INSET;
        let side = Self::SIDE - Self::PLAYER_INSET;
        (-end, field_width + end, -side, field_height + side)
    }

    /// How far outside the pitch rectangle a point is, in units — 0 for
    /// anything still on the field of play.
    #[inline]
    pub fn outside_by(position: Vector3<f32>, field_width: f32, field_height: f32) -> f32 {
        let x = (-position.x).max(position.x - field_width).max(0.0);
        let y = (-position.y).max(position.y - field_height).max(0.0);
        (x * x + y * y).sqrt()
    }

    /// Bring a rolling ball back inside the perimeter, killing the
    /// horizontal pace it hit the boards with.
    ///
    /// The vertical axis is deliberately untouched: a ball that reaches
    /// the boards half a metre up has to finish falling, and zeroing
    /// `velocity.z` here would leave it hanging against them.
    ///
    /// ⚠ **The boards are a metre high, so they only stop a ball a metre
    /// high.** See [`Self::BOARD_HEIGHT`] for the trace of what the
    /// unbounded version did to a ball that had gone over the bar. One
    /// that clears them is in the crowd, and [`Perimeter::Crowd`] is not
    /// a failure to contain it — it is the answer.
    #[inline]
    pub fn contain(
        position: &mut Vector3<f32>,
        velocity: &mut Vector3<f32>,
        spin: &mut Vector3<f32>,
        field_width: f32,
        field_height: f32,
    ) -> Perimeter {
        let (min_x, max_x, min_y, max_y) = Self::ball_bounds(field_width, field_height);
        let past = position.x <= min_x
            || position.x >= max_x
            || position.y <= min_y
            || position.y >= max_y;
        if !past {
            return Perimeter::Inside;
        }
        if position.z > Self::BOARD_HEIGHT {
            return Perimeter::Crowd;
        }
        position.x = position.x.clamp(min_x, max_x);
        position.y = position.y.clamp(min_y, max_y);
        velocity.x = 0.0;
        velocity.y = 0.0;
        *spin = Vector3::zeros();
        Perimeter::Boards
    }

    /// **Is the ball behind a goal?** — the strip of run-off directly
    /// behind the frame, which is the one part of the ground a fetching
    /// player cannot walk into.
    ///
    /// There is no path round a goal in the engine: a taker sent to a
    /// ball lying four metres behind the line and level with the six-yard
    /// box walks *through* the netting to get it, and then carries it
    /// back out through the netting at chest height. That is the reported
    /// *"it appears on the goal line"* — the ball sliding through the
    /// mesh and over the line on its way to the restart spot.
    ///
    /// Measured against the goal's own half-width plus
    /// [`Self::GOAL_SHADOW_MARGIN`], and against the byline rather than
    /// the goal's depth: a ball behind the BACK bar is still behind the
    /// net from every direction a keeper standing on his line can come at
    /// it from.
    #[inline]
    pub fn behind_a_goal(position: Vector3<f32>, field_width: f32, field_height: f32) -> bool {
        if position.x > 0.0 && position.x < field_width {
            return false;
        }
        (position.y - field_height * 0.5).abs() <= GOAL_WIDTH + Self::GOAL_SHADOW_MARGIN
    }

    /// Whether the taker can actually be sent to fetch the ball from
    /// where it has come to rest. False behind a goal — see
    /// [`Self::behind_a_goal`].
    #[inline]
    pub fn retrievable(position: Vector3<f32>, field_width: f32, field_height: f32) -> bool {
        !Self::behind_a_goal(position, field_width, field_height)
    }
}

impl Ball {
    /// **Where the ball actually crossed the line**, reconstructed from
    /// the tick that carried it over.
    ///
    /// Every out-of-play resolver runs immediately after `move_to`, so
    /// `self.position` is not the crossing point — it is the crossing
    /// point plus whatever was left of that tick's travel. At the measured
    /// mean non-shot speed that is 8 cm; on a struck shot it is 40 cm, and
    /// at the `MAX_VELOCITY` ceiling a full metre. The resolvers used to
    /// take it as-is and then inset it, so a ball crossing the touchline
    /// diagonally got its throw-in up to a metre down the line from where
    /// it went out.
    ///
    /// That mattered little while the ball was written onto the spot
    /// anyway. It matters now: the spot is where the taker has to bring
    /// the ball back to, and the run-out means the ball is no longer
    /// sitting on it to hide the error.
    ///
    /// Same swept solve `check_goal_frame` uses on the woodwork —
    /// `from = position - velocity`, solve the axis fraction, interpolate
    /// — and it returns `None` on the three cases where that
    /// reconstruction is not valid:
    ///
    /// * the ball is OWNED, so `move_to` tracked its owner rather than
    ///   integrating the velocity and `position - velocity` is fiction;
    /// * the velocity across the plane is below the stopping threshold, so
    ///   the solve is numerically meaningless (and the ball is barely past
    ///   the line anyway);
    /// * the solved fraction is outside this tick, which means the ball
    ///   did not cross on it — it was already out.
    ///
    /// Every caller falls back to the position it used before.
    pub(super) fn line_crossing(&self, axis: ExitAxis, bound: f32) -> Option<Vector3<f32>> {
        if self.current_owner.is_some() {
            return None;
        }
        let (from_axis, velocity_axis) = match axis {
            ExitAxis::Endline => (self.position.x - self.velocity.x, self.velocity.x),
            ExitAxis::Touchline => (self.position.y - self.velocity.y, self.velocity.y),
        };
        if velocity_axis.abs() < RunOff::STOPPED {
            return None;
        }
        let travel = (bound - from_axis) / velocity_axis;
        if !(0.0..=1.0).contains(&travel) {
            return None;
        }
        let from = self.position - self.velocity;
        Some(from + self.velocity * travel)
    }

    /// One tick of a ball that has been put out of play and is still
    /// travelling.
    ///
    /// The physics is the ordinary physics — `update_velocity` then
    /// `apply_movement`, the same pair `Ball::update` runs for a live ball
    /// — because a ball rolling across the run-off is doing nothing
    /// special. What is special is only that nobody may touch it and that
    /// the pitch clamp has stood down, and both of those are decided by
    /// `awaiting_restart` being set rather than by anything here.
    ///
    /// Returns true once the ball has come to rest, at which point the
    /// restart's `spot` is final and the caller may start measuring the
    /// taker against it.
    pub(super) fn tick_run_out(&mut self, await_state: &mut AwaitedRestart, now: u64) -> bool {
        self.update_velocity();
        self.apply_movement();
        let perimeter = RunOff::contain(
            &mut self.position,
            &mut self.velocity,
            &mut self.spin,
            self.field_width,
            self.field_height,
        );

        // **Into the stand.** The boards are a metre high and it went over
        // them, so it is in the crowd and nobody on the pitch is fetching
        // it. A fresh ball goes on the spot — which is what a ball into
        // the crowd gets in a real match, and the only alternative here is
        // the wall of infinite height that stopped a skied shot dead four
        // metres up. See [`RunOff::BOARD_HEIGHT`].
        if perimeter == Perimeter::Crowd {
            self.replace_dead_ball(await_state);
            #[cfg(feature = "match-logs")]
            RestartCensus::note_run_out_replaced(true);
            return true;
        }
        let boards = perimeter == Perimeter::Boards;

        // The spot follows the ball. `tick_awaited_restart` pins the ball
        // to it once this returns true, so the two have to agree on the
        // tick they change hands or the ball jumps by one tick of travel
        // at the moment it stops — which is the artefact in miniature.
        await_state.spot.x = self.position.x;
        await_state.spot.y = self.position.y;

        let horizontal =
            (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y).sqrt();
        // 0.012 m/tick is `update_velocity`'s own "this is a roll, not a
        // bounce" threshold — a ball still bouncing has not come to rest,
        // however slowly it is moving across the ground.
        let grounded = self.position.z <= 0.1 && self.velocity.z.abs() < 0.012;
        let elapsed = now.saturating_sub(await_state.awarded_tick);
        // A ball still in the air is held to the longer bound — see
        // [`RunOff::FLIGHT_CEILING`]. Gravity terminates that axis on its
        // own; the short ceiling exists for a ball trickling across grass,
        // and applying it to a skied one would pin it in mid-flight.
        let ceiling = if self.position.z > 0.1 {
            RunOff::FLIGHT_CEILING
        } else {
            RunOff::TRAVEL_CEILING
        };
        let expired = elapsed >= ceiling;
        if !expired && !(horizontal < RunOff::STOPPED && grounded) {
            return false;
        }

        // **…and behind the goal.** It has stopped somewhere there is no
        // walking to: the frame and its netting are between the taker and
        // the ball from every direction he can come at it from. Same
        // answer as the crowd, and for the same reason — the alternative
        // is a keeper walking through his own net and carrying the ball
        // back out through it at chest height.
        if !RunOff::retrievable(self.position, self.field_width, self.field_height) {
            self.replace_dead_ball(await_state);
            #[cfg(feature = "match-logs")]
            RestartCensus::note_run_out_replaced(false);
            return true;
        }

        await_state.settled = true;
        await_state.spot.z = 0.0;
        self.velocity = Vector3::zeros();
        self.spin = Vector3::zeros();
        let _ = boards;
        #[cfg(feature = "match-logs")]
        {
            // Two different numbers, and the difference is the point. How
            // far OUTSIDE the pitch it finished is whether the ball left
            // at all; how far it finished from the spot the restart is
            // taken from is what the taker then has to carry, and a ball
            // that ran a long way down the byline has a short first and a
            // long second.
            let outside = RunOff::outside_by(await_state.spot, self.field_width, self.field_height);
            let carry = await_state
                .take_from
                .map(|legal| (await_state.spot - legal).magnitude())
                .unwrap_or(0.0);
            RestartCensus::note_run_out_settled(outside, elapsed, boards, expired, carry);
        }
        true
    }

    /// **A new ball, on the spot.**
    ///
    /// The one the game was being played with has gone somewhere nobody
    /// is going to go and get it — into the stand, or behind the goal
    /// where the netting is in the way — and in a real match that is not
    /// a problem anyone solves by walking: the fourth official or a ball
    /// boy puts another one down and play restarts. The engine has one
    /// ball, so the swap is a relocation, and it is a *deliberate* one:
    /// it happens the moment the old ball dies, four metres or more
    /// outside the pitch with the camera on the players, and it replaces
    /// a fetch that had the taker walking through his own goal.
    ///
    /// Both legs collapse into one. There is nothing to carry, so
    /// `take_from` is dropped and the taker walks to the ball where it
    /// now lies — which is exactly where the restart is taken from.
    fn replace_dead_ball(&mut self, await_state: &mut AwaitedRestart) {
        let spot = await_state.take_from.take().unwrap_or(await_state.spot);
        await_state.spot = Vector3::new(spot.x, spot.y, 0.0);
        await_state.settled = true;
        // The height goes with it. `tick_awaited_restart` lowers a ball
        // onto its spot at 10 cm a tick so a live one is never dropped out
        // of the air — but this ball is not the one that was in the air,
        // and sinking the replacement through two metres of nothing is the
        // artefact that dressing it up would create.
        self.position = await_state.spot;
        self.velocity = Vector3::zeros();
        self.spin = Vector3::zeros();
        // Named in the trace so the tally does not read the swap as a
        // relocation nobody owns — `FrameTrace::tally` exempts a tick that
        // carries a note, and this one is exactly the kind it exists to
        // tell apart from a bug.
        #[cfg(feature = "match-logs")]
        super::frame_trace::FrameTrace::note(format!(
            "run-out: nobody can fetch it — fresh ball on ({:.1}, {:.1})",
            await_state.spot.x, await_state.spot.y
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::super::AwaitedRestart;
    use super::{GOAL_WIDTH, Perimeter, RunOff};
    use nalgebra::Vector3;

    /// The engine's run-off and the viewer's hoardings are the same wall.
    /// If these drift the ball stops in open grass or rolls through the
    /// boards, and neither is visible from inside this crate — so the
    /// numbers are pinned here against `Pitch::spawn_ground`'s own.
    #[test]
    fn the_margins_are_the_viewers_hoardings() {
        const METRES_PER_UNIT: f32 = 0.125;
        assert!(
            (RunOff::END * METRES_PER_UNIT - 4.6).abs() < 1.0e-4,
            "END must be pitch.rs END_MARGIN = 4.6 m, got {} m",
            RunOff::END * METRES_PER_UNIT
        );
        assert!(
            (RunOff::SIDE * METRES_PER_UNIT - 3.4).abs() < 1.0e-4,
            "SIDE must be pitch.rs SIDE_MARGIN = 3.4 m, got {} m",
            RunOff::SIDE * METRES_PER_UNIT
        );
        assert!(
            (RunOff::BOARD_HEIGHT - 0.95).abs() < 1.0e-4,
            "BOARD_HEIGHT must be pitch.rs HOARDING_HEIGHT = 0.95 m, got {} m",
            RunOff::BOARD_HEIGHT
        );
    }

    /// A fetching player has to be able to reach a ball lying against the
    /// boards from as close as he is allowed to stand to them, or every
    /// run-out ends in the backstop teleport.
    #[test]
    fn a_player_held_short_of_the_boards_can_still_reach_the_ball() {
        assert!(
            RunOff::PLAYER_INSET < AwaitedRestart::REACH,
            "a player stopped {}u short of a ball cannot pick it up at {}u",
            RunOff::PLAYER_INSET,
            AwaitedRestart::REACH
        );
    }

    #[test]
    fn the_boards_take_the_pace_off_and_let_the_ball_fall() {
        let mut position = Vector3::new(-50.0, 200.0, 0.6);
        let mut velocity = Vector3::new(-1.2, 0.1, -0.05);
        let mut spin = Vector3::new(0.0, 0.0, 0.4);
        let hit = RunOff::contain(&mut position, &mut velocity, &mut spin, 840.0, 545.0);
        assert_eq!(
            hit,
            Perimeter::Boards,
            "a ball 6 m behind the byline at knee height is stopped by the boards"
        );
        assert_eq!(position.x, -RunOff::END);
        assert_eq!(velocity.x, 0.0);
        assert_eq!(velocity.y, 0.0);
        assert_eq!(spin, Vector3::zeros());
        assert!(
            velocity.z < 0.0,
            "the ball has to finish falling down the boards, not hang against them"
        );
    }

    /// The one this was all for. A skied shot reaches the perimeter with
    /// metres of air under it, and the boards are a metre high — so it
    /// goes OVER them, and it must not be snatched out of the air and
    /// dropped down their face. Traced on a real run-out before the fix:
    /// the ball stopped dead at 3.79 m and fell like a plumb line for two
    /// seconds. See [`RunOff::BOARD_HEIGHT`].
    #[test]
    fn a_ball_above_the_boards_flies_over_them() {
        let mut position = Vector3::new(-50.0, 200.0, 3.8);
        let mut velocity = Vector3::new(-2.2, 1.2, -0.01);
        let mut spin = Vector3::new(0.0, 0.0, 0.4);
        let hit = RunOff::contain(&mut position, &mut velocity, &mut spin, 840.0, 545.0);
        assert_eq!(hit, Perimeter::Crowd, "3.8 m clears a 0.95 m board");
        assert_eq!(position.x, -50.0, "nothing may move it");
        assert_eq!(velocity.x, -2.2, "and nothing may take its pace off");
    }

    #[test]
    fn a_ball_inside_the_run_off_is_left_alone() {
        let mut position = Vector3::new(-10.0, 200.0, 0.0);
        let mut velocity = Vector3::new(-0.4, 0.0, 0.0);
        let mut spin = Vector3::zeros();
        assert_eq!(
            RunOff::contain(&mut position, &mut velocity, &mut spin, 840.0, 545.0),
            Perimeter::Inside
        );
        assert_eq!(position.x, -10.0);
        assert_eq!(velocity.x, -0.4, "still rolling out");
    }

    /// The strip behind the frame is the one place a taker cannot walk
    /// to: there is no path round a goal in the engine, so a keeper sent
    /// there walks through his own netting and carries the ball back out
    /// through it.
    #[test]
    fn the_ground_behind_a_goal_is_not_retrievable() {
        let centre = 545.0 / 2.0;
        for (x, y, reachable, what) in [
            (-30.0, centre, false, "dead behind the goal"),
            (-30.0, centre + GOAL_WIDTH, false, "behind a post"),
            (
                -30.0,
                centre + GOAL_WIDTH + 12.0,
                true,
                "behind the byline but clear of the frame",
            ),
            (870.0, centre - 10.0, false, "behind the other goal"),
            (400.0, -20.0, true, "over the touchline, nothing in the way"),
            (200.0, centre, true, "still on the pitch"),
        ] {
            assert_eq!(
                RunOff::retrievable(Vector3::new(x, y, 0.0), 840.0, 545.0),
                reachable,
                "({x}, {y}) — {what}"
            );
        }
    }
}
