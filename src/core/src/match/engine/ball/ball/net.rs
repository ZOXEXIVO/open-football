//! What happens to the ball *after* it crosses the line.
//!
//! # Why the ball used to stop dead on the goal line
//!
//! [`Ball::check_goal`](super::Ball::check_goal) detected the goal and
//! immediately called `Ball::reset()` — the ball was teleported to the centre
//! spot on the same tick it crossed the line, and the engine loop then skipped
//! 45-75 s of ticks for the post-goal dead time without recording anything.
//! The replay therefore held the LAST sample before the goal — the ball an
//! inch short of the line — on screen for the whole celebration. Nothing was
//! wrong with the detection; the ball simply had no life after it.
//!
//! # The net is a physical volume, not a plane
//!
//! A goal is a box behind the line: 7.32 m wide, 2.44 m at the crossbar, ~1.9 m
//! deep, with the roof netting sloping back to about waist height at the back
//! bar. This module models that box and the netting that closes four of its
//! faces, so a ball that goes in behaves the way one does — it stretches the
//! mesh, gets thrown back a little, drops, and rolls to a stop inside the goal.
//!
//! ## The netting is compliant, which is the whole visual
//!
//! A rigid wall would bounce the ball back out at speed; a hard clamp would
//! stop it dead on the panel. Real netting does neither: it is a membrane
//! pinned to a frame, so it takes the ball some way past the nominal panel —
//! [`GoalNet::GIVE_BACK`] — resists more and more steeply the further it is
//! pushed, spends nearly all of the ball's energy on the mesh, and rolls it
//! back down into the goalmouth. The bulge that produces is exactly what a
//! viewer needs in order to ripple the net, because the ball's own position
//! IS the deflection.
//!
//! ## Units
//!
//! `x`/`y` are game units (1u = 0.125 m) and `z` is metres — see
//! [`GRAVITY_PER_TICK`](super::GRAVITY_PER_TICK). Every constant here is
//! annotated with the axis it lives on; the two that apply to both
//! ([`GoalNet::DRAG_TAUT`], [`GoalNet::RECOIL`]) work out to the same
//! physical quantity on either because the unit conversion appears on both
//! sides of them.

use super::Ball;
use crate::r#match::ball::events::GoalSide;
use crate::r#match::engine::goal::{GOAL_HEIGHT, GOAL_WIDTH, GoalPosition};
use nalgebra::Vector3;

/// The ball is in the goal. Set the instant it crosses the line and held
/// until the restart — including the part of the celebration where a
/// goalkeeper has picked it out and is carrying it back — because every
/// resolver that would otherwise treat a ball behind the goal line as a
/// corner, a goal kick or an out-of-bounds clamp keys off this.
#[derive(Debug, Clone, Copy)]
pub struct BallInNet {
    /// Which goal it went into.
    pub side: GoalSide,
    /// Player credited with the goal, and whether it was into his own net.
    /// Carried here so the celebration knows who to mob without the flow
    /// layer having to re-derive it from the event stream.
    pub scorer_id: u32,
    pub auto_goal: bool,
}

/// One goal's netting, as a volume the ball is kept inside.
///
/// Built per use from [`GoalPosition`] rather than stored: it is three
/// floats of arithmetic and the alternative is a second copy of the goal
/// geometry that can drift away from the one `is_goal` reads.
pub struct GoalNet {
    /// x of the goal line this net sits behind.
    line_x: f32,
    /// +1 when the net lies toward increasing x (the right-hand goal),
    /// -1 for the left-hand one. Every "deeper into the net" quantity in
    /// here is measured along this.
    inward: f32,
    /// y of the middle of the goal mouth.
    centre_y: f32,
}

impl GoalNet {
    /// Depth of the goal, in game units. 15.2u = 1.9 m, matching the net the
    /// replay viewer draws (`Field::NET_DEPTH`); the two must agree or the
    /// ball settles somewhere the netting isn't.
    pub const DEPTH: f32 = 15.2;

    /// Height of the netting at the BACK bar, in metres. A goal's roof net
    /// slopes down from the 2.44 m crossbar to roughly waist height at the
    /// back, which is why a ball driven in under the bar dips as it goes.
    pub const BACK_HEIGHT: f32 = 1.15;

    /// How far the netting lets the ball travel past its nominal panel, at
    /// the point where it is slackest.
    ///
    /// The back net is hung slack and BAGS — a rocket puts a metre of it
    /// into the stanchion, which is the shape everyone recognises as a goal.
    /// The side and roof panels are pulled far tighter, so they give about
    /// half as much. 8u = 1 m, 4u = 50 cm.
    pub const GIVE_BACK: f32 = 8.0;
    pub const GIVE_SIDE: f32 = 4.0;
    const GIVE_ROOF_METRES: f32 = Self::GIVE_SIDE * 0.125;

    /// **The give is not the same everywhere on a panel, and that is what
    /// decides where the ball comes to rest.**
    ///
    /// A net is a membrane tied along its edges: to the back bar at the top,
    /// to the ground at the bottom. Pushed in the middle it bags; pushed at
    /// an edge it barely moves. Treating the give as a constant meant a ball
    /// that had rolled to a stop on the GRASS still had a metre of travel
    /// available to it, so it settled a mean 21u — **2.63 m** — behind the
    /// goal line, measured off a real recording. The replay draws the
    /// netting at its nominal 1.9 m ([`Field::NET_DEPTH`] in the viewer), so
    /// the ball finished three quarters of a metre BEHIND the net: the
    /// reported "the ball flies off and stops outside the net".
    ///
    /// The first mode of a membrane pinned at two edges is `4t(1-t)` — zero
    /// at both, one in the middle — and that is exactly the shape wanted
    /// here. A ball driven in at chest height bags the mesh; a ball rolling
    /// along the floor of the goal stops at the netting, which is where a
    /// ball in a real goal lies.
    #[inline]
    fn slack_at(height: f32, panel_height: f32) -> f32 {
        let t = (height / panel_height.max(1.0e-3)).clamp(0.0, 1.0);
        4.0 * t * (1.0 - t)
    }

    /// Speed the mesh takes out of the ball per tick, as a fraction, when
    /// the netting is barely stretched.
    const DRAG_SLACK: f32 = 0.10;
    /// …and when it is at the end of its travel, where the mesh is a wall
    /// and the ball stops in centimetres. The ramp between the two goes as
    /// the SQUARE of the stretch, because a membrane pinned to a frame gets
    /// stiffer the further it is pushed — which is the property that makes
    /// the net swallow a shot instead of bouncing it back out.
    const DRAG_TAUT: f32 = 0.95;

    /// Restoring push the stretched mesh gives back, per tick², at full
    /// stretch. Deliberately tiny: the netting returns the ball to the
    /// goalmouth at a walking pace, not to the penalty spot. Applied on
    /// whichever axis the panel faces — the horizontal reading is u/tick²
    /// and the vertical m/tick², which come to the same physical push
    /// because the unit conversion appears on both sides.
    const RECOIL: f32 = 0.010;

    /// Motion ACROSS a panel while the ball is in the mesh. A net wraps
    /// round the ball rather than letting it slide, so this is nearly as
    /// lossy as the stretch itself.
    const TANGENTIAL_DAMP: f32 = 0.88;

    /// Contact with the mesh scrubs rotation off almost completely.
    const SPIN_RETAINED: f32 = 0.45;

    /// The netting behind `side`.
    pub fn for_side(side: GoalSide, goals: &GoalPosition) -> Self {
        match side {
            GoalSide::Home => GoalNet {
                line_x: goals.left.x,
                inward: -1.0,
                centre_y: goals.left.y,
            },
            GoalSide::Away => GoalNet {
                line_x: goals.right.x,
                inward: 1.0,
                centre_y: goals.right.y,
            },
        }
    }

    /// How far past the goal line `x` is, measured into the goal. Negative
    /// in front of the line, i.e. still on the pitch.
    #[inline]
    pub fn depth_at(&self, x: f32) -> f32 {
        (x - self.line_x) * self.inward
    }

    /// Height of the roof netting `depth` units into the goal — the slope
    /// from the crossbar down to the back bar.
    #[inline]
    pub fn roof_at(&self, depth: f32) -> f32 {
        let t = (depth / Self::DEPTH).clamp(0.0, 1.0);
        GOAL_HEIGHT + (Self::BACK_HEIGHT - GOAL_HEIGHT) * t
    }

    /// Keep `ball` inside the netting for one tick, spending the energy it
    /// arrives with on the mesh.
    ///
    /// Called after the ball's own physics have moved it, so this is a
    /// position-and-velocity correction, not a force accumulator: each panel
    /// that the ball is past pushes it back and takes speed off it.
    pub fn contain(&self, ball: &mut Ball) {
        let mut touched = false;

        // Back panel. The deepest the ball may be is DEPTH; beyond that it
        // is bagging the mesh, and at DEPTH + GIVE_BACK the mesh is taut.
        let depth = self.depth_at(ball.position.x);
        if depth > Self::DEPTH {
            let mut along = ball.velocity.x * self.inward;
            let mut over = depth - Self::DEPTH;
            // Slack where the ball is meeting it — see `slack_at`. The back
            // panel is tied to the ground below and the back bar above, so a
            // ball on the deck stops at the netting and one at chest height
            // bags it.
            let give = Self::GIVE_BACK * Self::slack_at(ball.position.z, Self::BACK_HEIGHT);
            Self::press(&mut over, &mut along, give);
            ball.position.x = self.line_x + (Self::DEPTH + over) * self.inward;
            ball.velocity.x = along * self.inward;
            touched = true;
        } else if depth < 0.0 {
            // Out through the mouth again — the netting has spat it back
            // into the goalmouth. It is on the pitch now and the ordinary
            // physics own it. It must still stay on the pitch: the boundary
            // clamp stands down for a ball in the goal (see
            // `check_boundary_collision`), so this is the only thing left
            // holding it in.
            ball.position.x = ball.position.x.clamp(0.0, ball.field_width);
            ball.position.y = ball.position.y.clamp(0.0, ball.field_height);
            return;
        }

        // Side panels, one either side of the mouth.
        let across = ball.position.y - self.centre_y;
        if across.abs() > GOAL_WIDTH {
            let outward = across.signum();
            let mut lateral = ball.velocity.y * outward;
            let mut over = across.abs() - GOAL_WIDTH;
            // Same rule as the back panel, over this panel's own height:
            // the side netting runs from the ground to the sloping roof, so
            // the ball's height decides how much of the give it can use.
            let panel_height = self.roof_at(depth.max(0.0));
            let give = Self::GIVE_SIDE * Self::slack_at(ball.position.z, panel_height);
            Self::press(&mut over, &mut lateral, give);
            ball.position.y = self.centre_y + (GOAL_WIDTH + over) * outward;
            ball.velocity.y = lateral * outward;
            touched = true;
        }

        // Roof netting, sloping back from the crossbar.
        let roof = self.roof_at(self.depth_at(ball.position.x).max(0.0));
        if ball.position.z > roof {
            let mut rise = ball.velocity.z;
            let mut over = ball.position.z - roof;
            Self::press(&mut over, &mut rise, Self::GIVE_ROOF_METRES);
            ball.position.z = roof + over;
            ball.velocity.z = rise;
            touched = true;
        }

        if touched {
            // Everything the mesh does that isn't along the panel normal:
            // it wraps round the ball, drags on its travel, and takes the
            // spin off it.
            ball.velocity.x *= Self::TANGENTIAL_DAMP;
            ball.velocity.y *= Self::TANGENTIAL_DAMP;
            ball.spin *= Self::SPIN_RETAINED;
        }
    }

    /// Resolve one panel on its own axis.
    ///
    /// `over` is how far past the panel the ball is and `speed` its velocity
    /// along the OUTWARD normal; both are updated in place. `give` is the
    /// netting's travel on this axis, in the axis's own units.
    ///
    /// # Why the resistance is progressive rather than a spring plus a wall
    ///
    /// The first version of this was a linear spring with a hard stop at the
    /// travel limit, and it fired the ball back out of the goal and six
    /// metres up the pitch. Two things were wrong and they are the same
    /// thing: a hard stop has to express the entire energy loss in one
    /// restitution number, and a linear spring stores everything it takes
    /// and hands it straight back on the way out. Neither is what a net is.
    ///
    /// A net is a membrane pinned to a frame. Its resistance rises steeply
    /// with stretch — slack at first, a wall at the limit — and almost all
    /// the work goes into the mesh rather than into a return. Modelling the
    /// resistance as a stretch-dependent DRAG puts the loss where the
    /// physics puts it, across the whole contact, and leaves only a token
    /// push to roll the ball back down into the goalmouth.
    fn press(over: &mut f32, speed: &mut f32, give: f32) {
        // `give` can legitimately be zero — that is a panel pinned at the
        // point the ball met it — and the ratio below must not become a NaN
        // when it is. A hair of travel keeps the arithmetic finite and
        // resolves to the same thing: taut on the first tick, ball stopped.
        let give = give.max(1.0e-3);
        let stretch = (*over / give).clamp(0.0, 1.0);
        let resistance =
            Self::DRAG_SLACK + (Self::DRAG_TAUT - Self::DRAG_SLACK) * stretch * stretch;
        *speed *= 1.0 - resistance;
        *speed -= Self::RECOIL * stretch * stretch;

        if *over > give {
            // Taut. Nothing goes through the back of a goal net.
            *over = give;
            if *speed > 0.0 {
                *speed = 0.0;
            }
        }
    }
}

impl Ball {
    /// The ball has crossed the line for a goal: hand it to the netting.
    ///
    /// Everything a live ball carries is dropped here — the same set
    /// [`Ball::reset`] drops, so a goal leaves exactly as clean a ball as
    /// the old teleport-to-the-centre-spot did. What survives is the
    /// position, the velocity and the shot bookkeeping the goal event has
    /// not been dispatched against yet (`last_shot_xgot`, the pending error
    /// and failed-claim charges), which the goal handler reads a few
    /// microseconds later.
    pub(super) fn enter_net(&mut self, side: GoalSide, scorer_id: u32, auto_goal: bool) {
        self.clear_for_dead_ball();
        self.in_net = Some(BallInNet {
            side,
            scorer_id,
            auto_goal,
        });
    }

    /// One tick of a ball that is in the goal: its own physics, then the
    /// netting.
    ///
    /// A no-op once somebody has picked it up — the keeper walking it back
    /// to the halfway line is carrying it, and the carrier owns its position
    /// from that point.
    pub fn tick_net(&mut self, goals: &GoalPosition) {
        let Some(state) = self.in_net else {
            return;
        };
        if self.current_owner.is_some() {
            return;
        }

        self.update_velocity();
        self.apply_movement();
        GoalNet::for_side(state.side, goals).contain(self);
    }

    /// How far into the goal the ball currently is, in game units, or `None`
    /// when it isn't in one. Negative once it has been carried back out over
    /// the line.
    pub fn net_depth(&self, goals: &GoalPosition) -> Option<f32> {
        self.in_net
            .map(|state| GoalNet::for_side(state.side, goals).depth_at(self.position.x))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::MatchFieldSize;

    fn goals() -> GoalPosition {
        GoalPosition::from(&MatchFieldSize::new(840, 545))
    }

    fn ball_entering(velocity: Vector3<f32>, z: f32) -> Ball {
        let mut ball = Ball::with_coord(840.0, 545.0);
        // A whisker over the left-hand goal line, dead centre of the mouth.
        ball.position = Vector3::new(-0.5, 545.0 / 2.0, z);
        ball.velocity = velocity;
        ball.enter_net(GoalSide::Home, 1, false);
        ball
    }

    /// The headline bug: the ball has to end up IN the goal, not on the line.
    #[test]
    fn a_goal_finishes_inside_the_netting() {
        let net = goals();
        let mut ball = ball_entering(Vector3::new(-2.5, 0.0, 0.0), 0.3);
        for _ in 0..600 {
            ball.tick_net(&net);
        }
        let depth = ball.net_depth(&net).expect("ball is in the net");
        assert!(
            depth > 1.0,
            "the ball must come to rest behind the goal line, ended {depth:.2}u past it"
        );
        assert!(
            depth <= GoalNet::DEPTH + GoalNet::GIVE_BACK + 0.01,
            "and must not go through the back of the net, ended {depth:.2}u past the line"
        );
    }

    /// A net that returned the ball at anything like the pace it went in
    /// would fire it back out over the line.
    #[test]
    fn the_netting_absorbs_the_strike() {
        let net = goals();
        let mut ball = ball_entering(Vector3::new(-5.0, 0.0, 1.2), 1.0);
        for _ in 0..900 {
            ball.tick_net(&net);
        }
        let speed = ball.velocity.norm();
        assert!(
            speed < 0.05,
            "the ball must settle in the goal, still moving at {speed:.3} u/tick"
        );
        assert!(
            ball.net_depth(&net).unwrap() > 0.0,
            "and settle behind the line, not in front of it"
        );
    }

    /// The hardest strike the engine can produce must not end up in the
    /// stand behind the goal.
    #[test]
    fn even_a_rocket_stays_in_the_goal() {
        let net = goals();
        let mut ball = ball_entering(Vector3::new(-8.0, 1.0, 0.0), 2.0);
        for _ in 0..900 {
            ball.tick_net(&net);
            let depth = ball.net_depth(&net).unwrap();
            assert!(
                depth <= GoalNet::DEPTH + GoalNet::GIVE_BACK + 0.01,
                "ball reached {depth:.2}u past the line, past the back of the net"
            );
            let across = (ball.position.y - 545.0 / 2.0).abs();
            assert!(
                across <= GOAL_WIDTH + GoalNet::GIVE_SIDE + 0.01,
                "ball reached {across:.2}u across, past the side netting"
            );
        }
    }

    /// The roof net slopes, so a ball driven in just under the bar drops as
    /// it travels — it cannot stay at crossbar height at the back of the goal.
    #[test]
    fn the_roof_netting_slopes_back_from_the_crossbar() {
        let net = GoalNet::for_side(GoalSide::Home, &goals());
        assert!((net.roof_at(0.0) - GOAL_HEIGHT).abs() < 1.0e-6);
        assert!(net.roof_at(GoalNet::DEPTH) < GOAL_HEIGHT * 0.6);
        assert!(net.roof_at(GoalNet::DEPTH * 0.5) < net.roof_at(0.0));
    }

    /// **Where the ball ENDS UP is the whole of what a viewer sees**, and it
    /// has to be inside the netting the replay draws.
    ///
    /// The viewer hangs the back panel at a fixed `Field::NET_DEPTH` = 1.9 m
    /// = [`GoalNet::DEPTH`]. With a constant give the ball settled 5.8u
    /// beyond that — measured at 21.0u on a real recording, three quarters
    /// of a metre behind the drawn mesh, which is the reported "the ball
    /// flies off and stops outside the net". A ball at rest is on the
    /// GROUND, where the netting is pegged and has no give at all, so it
    /// must finish at the panel.
    #[test]
    fn a_settled_ball_lies_inside_the_netting_the_viewer_draws() {
        let net = goals();
        for launch in [
            Vector3::new(-2.5, 0.0, 0.0),
            Vector3::new(-5.0, 0.0, 1.2),
            Vector3::new(-8.0, 1.0, 0.0),
            Vector3::new(-3.0, -0.6, 0.4),
        ] {
            let mut ball = ball_entering(launch, 0.4);
            for _ in 0..1200 {
                ball.tick_net(&net);
            }
            let depth = ball.net_depth(&net).expect("ball is in the net");
            assert!(
                depth > 1.0,
                "launched at {launch:?} the ball must finish behind the line, ended {depth:.2}u"
            );
            // A hair of tolerance for the tick it settles on; anything more
            // is the ball resting behind the mesh.
            assert!(
                depth <= GoalNet::DEPTH + 0.5,
                "launched at {launch:?} the ball settled {depth:.2}u past the line, \
                 outside the {:.2}u netting the replay draws",
                GoalNet::DEPTH
            );
        }
    }

    /// The give is a property of WHERE on the panel the ball is. A ball
    /// driven in at the height the net is slackest has to bag it; the same
    /// ball rolling along the floor of the goal must not.
    #[test]
    fn the_netting_bags_at_chest_height_and_not_on_the_ground() {
        let slack_mid = GoalNet::slack_at(GoalNet::BACK_HEIGHT * 0.5, GoalNet::BACK_HEIGHT);
        let slack_floor = GoalNet::slack_at(0.0, GoalNet::BACK_HEIGHT);
        let slack_top = GoalNet::slack_at(GoalNet::BACK_HEIGHT, GoalNet::BACK_HEIGHT);
        assert!((slack_mid - 1.0).abs() < 1.0e-5, "slackest in the middle");
        assert!(slack_floor < 1.0e-6, "pegged at the ground");
        assert!(slack_top < 1.0e-6, "tied to the back bar");
        // …and it is a curve, not a step.
        let quarter = GoalNet::slack_at(GoalNet::BACK_HEIGHT * 0.25, GoalNet::BACK_HEIGHT);
        assert!(
            quarter > slack_floor && quarter < slack_mid,
            "the taper has to be continuous, got {quarter} at a quarter height"
        );
    }

    /// The mesh has to give, or there is nothing for the replay to ripple.
    #[test]
    fn the_mesh_stretches_before_it_stops_the_ball() {
        let net = goals();
        let mut ball = ball_entering(Vector3::new(-3.0, 0.0, 0.0), 0.4);
        let mut deepest: f32 = 0.0;
        for _ in 0..200 {
            ball.tick_net(&net);
            deepest = deepest.max(ball.net_depth(&net).unwrap());
        }
        assert!(
            deepest > GoalNet::DEPTH,
            "a driven ball must stretch the back netting, reached {deepest:.2}u"
        );
    }
}
