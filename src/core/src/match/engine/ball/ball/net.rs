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
//! annotated with the axis it lives on. The drags
//! ([`GoalNet::DRAG_SLACK`], [`GoalNet::DRAG_TAUT`],
//! [`GoalNet::DRAG_RELEASE`]) are pure fractions and read the same on
//! either axis; anything with a LENGTH in it does not, so
//! [`GoalNet::RECOIL`] and [`GoalNet::RELAX_RATE`] each carry a metric
//! twin. One constant serving both axes made the roof net shove eight
//! times harder than the back one.

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
    /// **Slack the mesh is holding, latched at the moment the ball went
    /// past each panel and released when it comes back off.**
    ///
    /// ⚠ THE GIVE MUST NOT BE RE-READ WHILE THE BALL IS IN THE MESH.
    /// [`GoalNet::slack_at`] reads the ball's HEIGHT, the panel's position
    /// clamp is derived from it, and the ball's height changes every tick —
    /// so recomputing it MOVES THE BALL. Measured on a real recording: a
    /// ball that bagged the back netting at knee height and then dropped to
    /// the floor was yanked **19 cm forward on the tick it landed**,
    /// because the slack at ground level is zero by design. That pop, once
    /// per goal, is the reported "ping-pong between the goal and the back
    /// of the net".
    ///
    /// A membrane is pushed where it was struck, so the honest model takes
    /// the slack once, at contact, and holds it for the contact.
    give_back: Option<f32>,
    give_side: Option<f32>,
    give_roof: Option<f32>,
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

    /// Drag on the way BACK OUT of the mesh.
    ///
    /// A membrane resists being STRETCHED; it does not resist unloading.
    /// [`Self::press`] used to apply the stretch drag in both directions,
    /// which glued the ball to the netting: a ball lying against the roof
    /// net had 95% of its FALL taken off it every tick, so it hung
    /// motionless under the crossbar until something else put it on the
    /// floor. Measured at 18 frozen-in-mid-air ticks over 40 captured
    /// goals, each ending in a one-tick drop to ground level.
    const DRAG_RELEASE: f32 = 0.03;

    /// Restoring push the stretched mesh gives back, per tick², at full
    /// stretch. Deliberately tiny: the netting returns the ball to the
    /// goalmouth at a walking pace, not to the penalty spot.
    ///
    /// Applied on whichever axis the panel faces, which is why it is stated
    /// twice: `x`/`y` are game units and `z` is metres, so the same
    /// physical push is a different NUMBER on each. One constant serving
    /// both made the roof net shove eight times harder than the back one.
    const RECOIL: f32 = 0.010;
    const RECOIL_METRES: f32 = Self::RECOIL * 0.125;

    /// Motion ACROSS a panel while the ball lies against it. A net wraps
    /// round the ball rather than letting it slide, so this is nearly as
    /// lossy as the stretch itself.
    ///
    /// It used to apply only on the ticks a panel's own position clamp
    /// fired — and the recoil pushes a settled ball a hair clear of the
    /// panel plane, so it stopped after a tick or two and a ball that hit
    /// the back netting then SLID up to four metres across the back of the
    /// goal at full pace before pinning itself in the corner. See
    /// [`Self::CONTACT_SKIN`] for what makes the contact persist.
    const TANGENTIAL_DAMP: f32 = 0.75;

    /// How far off a panel the ball still counts as lying against it: 0.2u
    /// = 2.5 cm, and the same distance in metres for the roof.
    ///
    /// The mesh drag has to survive the recoil nudging the ball off the
    /// panel plane, or it is not a drag at all — see
    /// [`Self::TANGENTIAL_DAMP`].
    const CONTACT_SKIN: f32 = 0.2;
    const CONTACT_SKIN_METRES: f32 = Self::CONTACT_SKIN * 0.125;

    /// How fast a stretched panel gives its travel BACK, in game units per
    /// tick, with a metric twin for the roof. 0.06u = 7.5 mm a tick, i.e.
    /// 0.75 m/s — a mesh visibly unloading, not a ball being moved.
    ///
    /// This is the other half of latching the give (see
    /// [`BallInNet::give_back`]). The latch is what stops the netting
    /// yanking the ball when its height changes; without a relax the latch
    /// would hold the bag forever, and a ball driven in at knee height and
    /// then dropping to the floor stayed 2.4 m deep — behind the mesh the
    /// replay draws, which is the bug `slack_at` exists to prevent. So the
    /// held give eases toward the slack at the ball's PRESENT height, and
    /// the ball rides the mesh back out to the panel over about a second.
    ///
    /// Only the shrink is rate-limited. A panel that can suddenly give MORE
    /// (the ball rising to where the mesh is slacker) only loosens a clamp,
    /// and loosening a clamp never moves the ball.
    const RELAX_RATE: f32 = 0.06;
    const RELAX_RATE_METRES: f32 = Self::RELAX_RATE * 0.125;

    /// Ease a held give toward the one the ball's present position asks for.
    #[inline]
    fn relax(held: f32, live: f32, rate: f32) -> f32 {
        if held > live {
            (held - rate).max(live)
        } else {
            live
        }
    }

    /// The ball's radius, in game units.
    ///
    /// **The panels are the mesh; what rests against them is the ball's
    /// SURFACE.** Every plane below is therefore pulled in by this. Without
    /// it the ball settles with its CENTRE on the netting and half of it
    /// drawn outside the goal — measured resting at exactly
    /// `(centre_y + GOAL_WIDTH, DEPTH)`, the back corner, on two of three
    /// captured goals. Read off [`GoalFrame`](super::frame::GoalFrame) so
    /// the woodwork and the netting can never disagree about how big a
    /// football is.
    const BALL_RADIUS_UNITS: f32 = super::frame::GoalFrame::BALL_RADIUS * 8.0;

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
        let Some(mut mesh) = ball.in_net else {
            return;
        };

        // Back panel. The deepest the ball's CENTRE may be is a radius
        // short of the mesh; beyond that it is bagging it, and at
        // `plane + give` the mesh is taut.
        let back_plane = Self::DEPTH - Self::BALL_RADIUS_UNITS;
        let depth = self.depth_at(ball.position.x);
        if depth < 0.0 {
            // Out through the mouth again — the netting has spat it back
            // into the goalmouth. It is on the pitch now and the ordinary
            // physics own it. It must still stay on the pitch: the boundary
            // clamp stands down for a ball in the goal (see
            // `check_boundary_collision`), so this is the only thing left
            // holding it in. Every latch is released: it is out of the mesh.
            ball.position.x = ball.position.x.clamp(0.0, ball.field_width);
            ball.position.y = ball.position.y.clamp(0.0, ball.field_height);
            ball.in_net = Some(BallInNet {
                give_back: None,
                give_side: None,
                give_roof: None,
                ..mesh
            });
            return;
        }

        let over_back = depth - back_plane;
        let on_back = over_back > -Self::CONTACT_SKIN;
        if over_back > 0.0 {
            // Slack where the ball MET it — see `slack_at` for the shape and
            // `BallInNet::give_back` for why it is latched rather than read
            // afresh. The back panel is tied to the ground below and the
            // back bar above, so a ball on the deck stops at the netting and
            // one at chest height bags it.
            let live = Self::GIVE_BACK * Self::slack_at(ball.position.z, Self::BACK_HEIGHT);
            let held = mesh.give_back.get_or_insert(live);
            *held = Self::relax(*held, live, Self::RELAX_RATE);
            let give = *held;
            let mut along = ball.velocity.x * self.inward;
            let mut over = over_back;
            Self::press(&mut over, &mut along, give, Self::RECOIL);
            ball.position.x = self.line_x + (back_plane + over) * self.inward;
            ball.velocity.x = along * self.inward;
        } else {
            mesh.give_back = None;
        }

        // Side panels, one either side of the mouth.
        let side_plane = GOAL_WIDTH - Self::BALL_RADIUS_UNITS;
        let across = ball.position.y - self.centre_y;
        let over_side = across.abs() - side_plane;
        let on_side = over_side > -Self::CONTACT_SKIN;
        if over_side > 0.0 {
            let outward = across.signum();
            // Same rule as the back panel, over this panel's own height:
            // the side netting runs from the ground to the sloping roof, so
            // the height the ball met it at decides how much of the give it
            // has.
            let panel_height = self.roof_at(depth.max(0.0));
            let live = Self::GIVE_SIDE * Self::slack_at(ball.position.z, panel_height);
            let held = mesh.give_side.get_or_insert(live);
            *held = Self::relax(*held, live, Self::RELAX_RATE);
            let give = *held;
            let mut lateral = ball.velocity.y * outward;
            let mut over = over_side;
            Self::press(&mut over, &mut lateral, give, Self::RECOIL);
            ball.position.y = self.centre_y + (side_plane + over) * outward;
            ball.velocity.y = lateral * outward;
        } else {
            mesh.give_side = None;
        }

        // Roof netting, sloping back from the crossbar.
        let roof_plane = self.roof_at(self.depth_at(ball.position.x).max(0.0))
            - super::frame::GoalFrame::BALL_RADIUS;
        let over_roof = ball.position.z - roof_plane;
        let on_roof = over_roof > -Self::CONTACT_SKIN_METRES;
        if over_roof > 0.0 {
            let give = *mesh.give_roof.get_or_insert(Self::GIVE_ROOF_METRES);
            let mut rise = ball.velocity.z;
            let mut over = over_roof;
            Self::press(&mut over, &mut rise, give, Self::RECOIL_METRES);
            ball.position.z = roof_plane + over;
            ball.velocity.z = rise;
        } else {
            mesh.give_roof = None;
        }

        // A ball LYING AGAINST the netting is wrapped by it: the mesh drags
        // on everything it does horizontally and scrubs the spin off it.
        //
        // Keyed on contact rather than on a panel's clamp having fired,
        // which is the whole fix — the clamp only fires while the ball is
        // stretching the mesh, and a ball that has stopped stretching it is
        // still lying in it. Keyed the other way, the drag lasted a tick or
        // two and a ball that hit the back netting slid four metres across
        // the goal.
        //
        // The vertical axis is left to gravity: a ball dropping down the
        // inside of the netting is falling, not sliding, and damping `z`
        // here gives it a terminal velocity of a quarter of a metre a
        // second. Its own panel — the roof — still governs it through
        // `press`.
        if on_back || on_side || on_roof {
            ball.velocity.x *= Self::TANGENTIAL_DAMP;
            ball.velocity.y *= Self::TANGENTIAL_DAMP;
            ball.spin *= Self::SPIN_RETAINED;
        }

        ball.in_net = Some(mesh);
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
    fn press(over: &mut f32, speed: &mut f32, give: f32, recoil: f32) {
        // `give` can legitimately be zero — that is a panel pinned at the
        // point the ball met it — and the ratio below must not become a NaN
        // when it is. A hair of travel keeps the arithmetic finite and
        // resolves to the same thing: taut on the first tick, ball stopped.
        let give = give.max(1.0e-3);
        let stretch = (*over / give).clamp(0.0, 1.0);
        // Only the STRETCH is resisted. A ball on its way back out is
        // unloading the mesh, and charging it the stretch drag is what held
        // one motionless against the roof netting — see `DRAG_RELEASE`.
        if *speed > 0.0 {
            let resistance =
                Self::DRAG_SLACK + (Self::DRAG_TAUT - Self::DRAG_SLACK) * stretch * stretch;
            *speed *= 1.0 - resistance;
        } else {
            *speed *= 1.0 - Self::DRAG_RELEASE;
        }
        *speed -= recoil * stretch * stretch;

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
        #[cfg(feature = "match-logs")]
        if super::frame_trace::FrameTrace::captures_goals() {
            super::frame_trace::FrameTrace::open(format!(
                "=== INTO THE NET ({side:?}) at ({:.2}, {:.2}, {:.2})  v ({:.2}, {:.2}, {:.2}) ===",
                self.position.x,
                self.position.y,
                self.position.z,
                self.velocity.x,
                self.velocity.y,
                self.velocity.z,
            ));
        }
        self.clear_for_dead_ball();
        self.in_net = Some(BallInNet {
            side,
            scorer_id,
            auto_goal,
            give_back: None,
            give_side: None,
            give_roof: None,
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
    use crate::r#match::engine::ball::ball::GRAVITY_PER_TICK;

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

    /// **The netting must never MOVE the ball.**
    ///
    /// The give is a function of the ball's HEIGHT and the panel's position
    /// clamp is derived from it, so a ball that bagged the mesh at knee
    /// height and then dropped to the floor — where the netting is pegged
    /// and the slack is zero — was clamped to a shallower depth on the tick
    /// it landed and yanked forward 19 cm in one tick. Once a goal, that is
    /// the "ping-pong between the goal and the back of the net" a viewer
    /// sees. See [`BallInNet::give_back`] and [`GoalNet::relax`].
    ///
    /// The bound is the ball's own travel: whatever the mesh does, the ball
    /// may not move further in a tick than its velocity carried it, plus
    /// the rate the membrane is allowed to unload at.
    #[test]
    fn the_netting_never_yanks_the_ball() {
        let net = goals();
        for (launch, height) in [
            // The case that produced the measured pop: driven in low and
            // hard, lands inside the mesh.
            (Vector3::new(-4.0, 0.0, 0.0), 0.15),
            (Vector3::new(-3.0, -1.5, 0.0), 0.40),
            // Under the bar, so the roof panel takes it first.
            (Vector3::new(-3.5, 0.0, 0.1), 2.30),
            // Across the mouth, so a side panel does.
            (Vector3::new(-2.0, 3.0, 0.0), 0.60),
        ] {
            let mut ball = ball_entering(launch, height);
            let mut previous = ball.position;
            for tick in 0..900 {
                let expected = ball.velocity;
                ball.tick_net(&net);
                let moved = ball.position - previous;
                let allowance = (expected.x.abs() + GoalNet::RELAX_RATE).max(0.1);
                assert!(
                    moved.x.abs() <= allowance,
                    "launched at {launch:?} from {height}m, tick {tick}: the netting moved the \
                     ball {:.3}u along the goal on a tick its own speed was {:.3}u",
                    moved.x,
                    expected.x.abs(),
                );
                // Plus two allowances that are not the ball being moved.
                // The rolling threshold: `update_velocity` settles a ball
                // below 10 cm onto the deck, a long-standing simplification
                // worth half a ball radius. And the ROOF SLOPE: a ball
                // running back under the roof netting follows it down, and
                // at 3.5 u/tick along a panel falling 8.5 cm a unit that is
                // 30 cm of honest descent in a tick. The drop this guards
                // against was 2.24 m with the ball standing still.
                let slope = (GOAL_HEIGHT - GoalNet::BACK_HEIGHT) / GoalNet::DEPTH;
                let vertical_allowance =
                    expected.z.abs() + GRAVITY_PER_TICK * 2.0 + 0.11 + slope * moved.x.abs();
                assert!(
                    moved.z.abs() <= vertical_allowance,
                    "launched at {launch:?} from {height}m, tick {tick}: the ball fell {:.3}m in \
                     one tick with {:.4}m/tick of vertical speed behind it",
                    -moved.z,
                    expected.z,
                );
                previous = ball.position;
            }
        }
    }

    /// A ball the netting has caught in the air has to COME DOWN.
    ///
    /// `press` resisted motion in both directions, so a ball lying against
    /// the roof mesh had 95% of its fall taken off it every tick and hung
    /// motionless under the crossbar — until the settle branch in
    /// `update_velocity` zeroed its height and dropped it 2.24 m in a single
    /// tick. Both halves are fixed together; either alone still reads wrong.
    #[test]
    fn a_ball_caught_by_the_roof_netting_falls_out_of_it() {
        let net = goals();
        // Driven in hard just under the bar, still climbing — the shape that
        // buries a ball in the roof mesh.
        let mut ball = ball_entering(Vector3::new(-4.0, 0.0, 0.05), GOAL_HEIGHT - 0.1);
        let mut highest: f32 = 0.0;
        let mut stalled = 0;
        let mut worst_stall = 0;
        for _ in 0..400 {
            ball.tick_net(&net);
            highest = highest.max(ball.position.z);
            // Aloft and not accelerating downward is the mesh holding it.
            // The release drag leaves a terminal velocity of 3 cm a tick,
            // thirty times gravity's step, so a healthy fall is never
            // anywhere near this.
            if ball.position.z > 0.5 && ball.velocity.z.abs() < GRAVITY_PER_TICK {
                stalled += 1;
                worst_stall = worst_stall.max(stalled);
            } else {
                stalled = 0;
            }
        }
        assert!(
            highest > 1.5,
            "the test is meaningless unless the ball reached the roof netting, peaked at \
             {highest:.2}m"
        );
        assert!(
            worst_stall < 5,
            "the ball hung in the roof netting for {worst_stall} ticks with no fall in it — \
             the mesh is resisting the RETURN, not just the stretch"
        );
        assert!(
            ball.position.z < 0.3,
            "a ball caught by the roof netting must drop into the goal, still at {:.2}m after \
             four seconds",
            ball.position.z
        );
    }

    /// A ball that hits the back of the net drops; it does not skate across
    /// the back of the goal.
    ///
    /// The mesh drag only ran on the ticks a panel's own position clamp
    /// fired, and the recoil pushes a settled ball a hair clear of the
    /// panel — so it stopped after a tick or two, and a ball arriving with
    /// any lateral pace slid the width of the goal and pinned itself in the
    /// corner. Measured at 30u of travel across the back netting.
    #[test]
    fn a_ball_in_the_netting_does_not_slide_across_the_goal() {
        let net = goals();
        // Into the middle of the mouth, hard, well across — all of the
        // lateral pace has to be spent in the mesh.
        let mut ball = ball_entering(Vector3::new(-3.0, -2.0, 0.0), 0.2);
        let back_plane = GoalNet::DEPTH - GoalNet::BALL_RADIUS_UNITS;
        // Measured from the moment it MEETS the mesh, not from the goal
        // line: the run across the empty goal in between is flight, and
        // folding it in would hide the thing being measured.
        let mut met_the_mesh: Option<f32> = None;
        for _ in 0..900 {
            ball.tick_net(&net);
            if met_the_mesh.is_none() && ball.net_depth(&net).unwrap() >= back_plane {
                met_the_mesh = Some(ball.position.y);
            }
        }
        let contact_y = met_the_mesh.expect("the ball has to reach the back netting");
        let slid = (ball.position.y - contact_y).abs();
        assert!(
            slid < 6.0,
            "the ball travelled {slid:.1}u across the goal after meeting the netting — it is \
             skating on the mesh rather than being caught by it"
        );
    }

    /// The ball rests against the mesh on its SURFACE, not its centre.
    ///
    /// Clamping the centre to the panel draws half the ball outside the
    /// goal, and two of three captured goals finished with the ball pinned
    /// exactly on the side netting at the back corner.
    #[test]
    fn a_settled_ball_rests_on_its_surface_and_not_through_the_mesh() {
        let net = goals();
        let mut ball = ball_entering(Vector3::new(-5.0, 2.5, 0.0), 0.3);
        for _ in 0..1200 {
            ball.tick_net(&net);
        }
        let depth = ball.net_depth(&net).expect("ball is in the net");
        assert!(
            depth <= GoalNet::DEPTH - GoalNet::BALL_RADIUS_UNITS + 0.5,
            "the ball's centre finished {depth:.2}u deep, so its surface is through the back \
             netting at {:.2}u",
            GoalNet::DEPTH
        );
        let across = (ball.position.y - 545.0 / 2.0).abs();
        assert!(
            across <= GOAL_WIDTH - GoalNet::BALL_RADIUS_UNITS + 0.5,
            "the ball's centre finished {across:.2}u across, so its surface is through the side \
             netting at {GOAL_WIDTH:.2}u"
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
