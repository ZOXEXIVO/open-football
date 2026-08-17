//! The goal frame: two posts and a crossbar, and what the ball does when it
//! hits one.
//!
//! # Why the woodwork had to exist
//!
//! [`GoalPosition::is_goal`](crate::r#match::engine::goal::GoalPosition::is_goal)
//! is a rectangle test — `z <= GOAL_HEIGHT` and `|y - centre| <= GOAL_WIDTH`
//! — and nothing else in the engine knew a goal had a frame at all. A shot
//! that in the replay visibly strikes the near post either went in (a
//! whisker inside the rectangle) or ran out for a goal kick (a whisker
//! outside it), passing straight *through* the post the viewer draws. In
//! ninety minutes of football the frame is hit perhaps once, and it is one
//! of the most recognisable things that happens in the sport; the engine
//! could not produce it at all.
//!
//! Worse, the absence was doing damage the other way round. Every rebound
//! near the goal line came from somewhere else — a keeper's parry, a
//! deflected block — and those resolve at the BALL, not at the man. So the
//! only rebounds a viewer ever saw were ones with nobody at the point of
//! contact, which is the "the ball bounced off empty space on the goal
//! line" report. Giving the frame its real geometry supplies the one
//! rebound in football that legitimately comes off nothing but the
//! structure.
//!
//! # Geometry
//!
//! The Laws measure a goal between the INNER faces: 7.32 m across, 2.44 m
//! to the underside of the bar, on posts no more than 12 cm thick. So
//! [`GOAL_WIDTH`] and [`GOAL_HEIGHT`] are inner faces and each member's
//! axis sits [`GoalFrame::POST_RADIUS`] beyond them. That ordering matters:
//! put the axis ON the nominal line instead and half of every post
//! protrudes into the goal, turning a legitimate shot into the side netting
//! into a post rebound.
//!
//! # Units
//!
//! `x`/`y` are game units (1u = 0.125 m) and `z` is metres, so the swept
//! test converts to a metric frame, solves there, and converts back — a
//! post is round in metres and an ellipse in the stored mixed frame, and
//! the crossbar test spans one axis of each.
//!
//! # Tunnelling
//!
//! A ball leaves the boot at up to 3.2 u/tick = 40 cm a tick, and a post
//! plus a ball is 34 cm across. A point-in-cylinder test therefore misses
//! roughly half of all genuine contacts and misses the hardest strikes
//! preferentially — exactly the ones a rebound matters for. Every test here
//! is a SWEPT one, over the segment the ball has just travelled, which is
//! recoverable as `position - velocity` because the frame check runs
//! immediately after the move.

use super::Ball;
use crate::r#match::engine::goal::{GOAL_HEIGHT, GOAL_WIDTH, GoalPosition};
use nalgebra::Vector3;

/// Which member of the frame the ball struck. Carried on the event so the
/// commentary, the stat line and the replay can all say which it was —
/// "off the bar" and "off the post" are different pieces of football.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePart {
    /// The upright nearer to `y = 0`.
    LeftPost,
    /// The upright nearer to `y = field_height`.
    RightPost,
    Crossbar,
}

/// A contact with the frame, in the ball's own stored coordinates.
#[derive(Debug, Clone, Copy)]
pub struct FrameHit {
    pub part: FramePart,
    /// Where the ball's centre was at the moment of contact.
    pub position: Vector3<f32>,
    /// Outward normal at the contact, in the stored mixed frame,
    /// normalised in METRES (so reflecting along it is physical).
    pub normal: Vector3<f32>,
    /// Fraction of this tick's travel that had elapsed at contact.
    pub travel: f32,
}

/// One goal's frame.
pub struct GoalFrame {
    line_x: f32,
    centre_y: f32,
}

impl GoalFrame {
    /// Horizontal game units per metre, and its inverse.
    const U_PER_M: f32 = 8.0;
    const M_PER_U: f32 = 0.125;

    /// Radius of a post or the bar, in metres. The Laws cap the thickness
    /// at 12 cm; a modern elliptical post is about that across.
    pub const POST_RADIUS: f32 = 0.06;

    /// Radius of the ball, in metres. A size-5 ball is 22 cm across.
    pub const BALL_RADIUS: f32 = 0.11;

    /// Sum of the two — the distance from a member's axis at which the
    /// surfaces touch.
    const CONTACT: f32 = Self::POST_RADIUS + Self::BALL_RADIUS;

    /// How much of its approach speed the ball keeps off the frame.
    ///
    /// A goalpost is far stiffer than the ball, so this is essentially the
    /// ball's own restitution against a rigid surface — higher than the
    /// 0.30 it gets off grass, because turf absorbs and aluminium does
    /// not. 0.65 is what puts a ball smashed against the underside of the
    /// bar back out to the penalty spot, which is the shape everybody
    /// recognises.
    const RESTITUTION: f32 = 0.65;

    /// Speed retained ACROSS the contact normal. Metal is slick, so a ball
    /// that clips the post keeps nearly all of its sideways travel — this
    /// is what sends a shot off the inside of the post along the line
    /// rather than stopping it dead.
    const TANGENTIAL: f32 = 0.94;

    /// Rotation surviving the contact. Almost none: the ball's own
    /// deformation against something that does not give scrubs it off.
    const SPIN_RETAINED: f32 = 0.30;

    /// Diagnostic switch: with `OF_FRAME_OFF` set, the goal has no
    /// woodwork and the ball passes through it exactly as it did before
    /// this module existed.
    ///
    /// The A/B control for the frame. Adding the posts TIGHTENS the goal by
    /// a ball's radius on three sides — `is_goal` measures the ball's
    /// CENTRE against the inner faces, which was 11 cm too generous at each
    /// — so it moves the scoring rate, and "how much" cannot be answered by
    /// reading the diff. Same pattern and same purpose as `OF_SHAPE_OFF`
    /// and `OF_KEEPER_SERVO`; read once per process. Debug infrastructure —
    /// do not remove.
    fn disabled() -> bool {
        static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_FRAME_OFF").is_ok())
    }

    /// The frame at the left-hand (`x = 0`) or right-hand goal.
    pub fn both(goals: &GoalPosition) -> [Self; 2] {
        [
            GoalFrame {
                line_x: goals.left.x,
                centre_y: goals.left.y,
            },
            GoalFrame {
                line_x: goals.right.x,
                centre_y: goals.right.y,
            },
        ]
    }

    /// Distance from the goal centre to a post's AXIS, in game units.
    fn post_offset(&self) -> f32 {
        GOAL_WIDTH + Self::POST_RADIUS * Self::U_PER_M
    }

    /// Height of the crossbar's axis, in metres.
    fn bar_height() -> f32 {
        GOAL_HEIGHT + Self::POST_RADIUS
    }

    /// The earliest contact this frame makes with a ball travelling from
    /// `from` to `to`, if any.
    ///
    /// Each member is solved in the two-dimensional plane perpendicular to
    /// its own axis, in metres: a post is vertical, so that is the pitch
    /// plane; the bar runs across the goal, so it is the (along, up) plane.
    /// Both reduce to the same quadratic — when does a moving point first
    /// come within [`Self::CONTACT`] of a fixed point.
    pub fn sweep(&self, from: Vector3<f32>, to: Vector3<f32>) -> Option<FrameHit> {
        let mut best: Option<FrameHit> = None;
        let mut consider = |hit: Option<FrameHit>| {
            if let Some(hit) = hit.filter(|hit| best.is_none_or(|b| hit.travel < b.travel)) {
                best = Some(hit);
            }
        };

        for (part, side) in [(FramePart::LeftPost, -1.0f32), (FramePart::RightPost, 1.0)] {
            consider(self.sweep_post(part, side, from, to));
        }
        consider(self.sweep_bar(from, to));
        best
    }

    /// A post: a vertical cylinder, so the contact is decided entirely in
    /// the pitch plane, and the height only says whether the ball was
    /// below the bar when it got there.
    fn sweep_post(
        &self,
        part: FramePart,
        side: f32,
        from: Vector3<f32>,
        to: Vector3<f32>,
    ) -> Option<FrameHit> {
        let axis_y = self.centre_y + side * self.post_offset();
        // Into metres, relative to the post's axis.
        let start = (
            (from.x - self.line_x) * Self::M_PER_U,
            (from.y - axis_y) * Self::M_PER_U,
        );
        let step = (
            (to.x - from.x) * Self::M_PER_U,
            (to.y - from.y) * Self::M_PER_U,
        );
        let travel = Self::first_contact(start, step)?;

        // Where it was, in the stored frame, at that instant.
        let position = from + (to - from) * travel;
        // Below the bar's underside, or it passed over the top of the
        // post rather than against it. A ball at exactly bar height is
        // the bar's business, not the post's.
        if position.z > GOAL_HEIGHT + Self::POST_RADIUS * 2.0 || position.z < 0.0 {
            return None;
        }
        let nx = (position.x - self.line_x) * Self::M_PER_U;
        let ny = (position.y - axis_y) * Self::M_PER_U;
        let normal = Self::unit(Vector3::new(nx * Self::U_PER_M, ny * Self::U_PER_M, 0.0))?;
        Some(FrameHit {
            part,
            position,
            normal,
            travel,
        })
    }

    /// The crossbar: a cylinder running across the goal, so the contact is
    /// decided in the (along-the-pitch, up) plane and the lateral
    /// coordinate only says whether the ball was between the posts.
    fn sweep_bar(&self, from: Vector3<f32>, to: Vector3<f32>) -> Option<FrameHit> {
        let bar = Self::bar_height();
        let start = ((from.x - self.line_x) * Self::M_PER_U, from.z - bar);
        let step = ((to.x - from.x) * Self::M_PER_U, to.z - from.z);
        let travel = Self::first_contact(start, step)?;

        let position = from + (to - from) * travel;
        if (position.y - self.centre_y).abs() > self.post_offset() {
            return None;
        }
        let nx = (position.x - self.line_x) * Self::M_PER_U;
        let nz = position.z - bar;
        let normal = Self::unit(Vector3::new(nx * Self::U_PER_M, 0.0, nz))?;
        Some(FrameHit {
            part: FramePart::Crossbar,
            position,
            normal,
            travel,
        })
    }

    /// When a point starting at `start` and moving by `step` (both in
    /// metres, both relative to the member's axis) first comes within
    /// [`Self::CONTACT`] of the origin — or `None` if it never does inside
    /// this step.
    ///
    /// A ball that is ALREADY overlapping at the start of the step is not a
    /// contact: it means the previous tick resolved one and pushed the ball
    /// out to exactly the contact distance, and re-firing on the rounding
    /// would trap it against the post. Only a crossing counts.
    fn first_contact(start: (f32, f32), step: (f32, f32)) -> Option<f32> {
        let (sx, sy) = start;
        let (dx, dy) = step;
        let a = dx * dx + dy * dy;
        if a < 1.0e-12 {
            return None;
        }
        let r2 = Self::CONTACT * Self::CONTACT;
        if sx * sx + sy * sy <= r2 {
            return None;
        }
        // |start + t·step|² = r²
        let b = 2.0 * (sx * dx + sy * dy);
        let c = sx * sx + sy * sy - r2;
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            return None;
        }
        let t = (-b - disc.sqrt()) / (2.0 * a);
        if (0.0..=1.0).contains(&t) {
            Some(t)
        } else {
            None
        }
    }

    /// Rescale a mixed-frame vector so that its METRIC length is 1, and
    /// return it still in the mixed frame.
    ///
    /// That is the property the reflection below needs. The stored frame is
    /// the metric one through a diagonal map `diag(0.125, 0.125, 1)`, so
    /// with `|D·n| = 1` the whole split — `(D·v)·(D·n)` for the approach
    /// speed, `n · approach` for the normal component, `v − that` for the
    /// tangent — is the ordinary vector algebra done in metres, written in
    /// the frame the velocity is stored in. Scaling by anything else
    /// silently changes the restitution.
    fn unit(v: Vector3<f32>) -> Option<Vector3<f32>> {
        let len = Self::metres(v);
        if len < 1.0e-6 {
            return None;
        }
        Some(v / len)
    }

    /// Metric length of a mixed-frame vector.
    fn metres(v: Vector3<f32>) -> f32 {
        ((v.x * Self::M_PER_U).powi(2) + (v.y * Self::M_PER_U).powi(2) + v.z * v.z).sqrt()
    }

    /// Metric dot product of two mixed-frame vectors.
    fn dot_metres(a: Vector3<f32>, b: Vector3<f32>) -> f32 {
        (a.x * Self::M_PER_U) * (b.x * Self::M_PER_U)
            + (a.y * Self::M_PER_U) * (b.y * Self::M_PER_U)
            + a.z * b.z
    }

    /// Resolve `hit`: put the ball on the surface and turn its velocity.
    ///
    /// Reads nothing about WHICH goal it was, so it is an associated
    /// function — the hit already carries everything the response needs.
    fn rebound(ball: &mut Ball, hit: &FrameHit) {
        // `hit.position` is on the surface by construction. Push out 3 mm
        // along the normal so `first_contact`'s already-overlapping guard
        // cannot latch on the rounding and grind the ball along the post.
        const CLEARANCE_METRES: f32 = 0.003;
        ball.position = hit.position + hit.normal * CLEARANCE_METRES;

        let approach = Self::dot_metres(ball.velocity, hit.normal);
        if approach < 0.0 {
            // Split the velocity about the normal, then rebuild it with the
            // normal component reversed and damped and the tangential one
            // merely scuffed.
            let normal_part = hit.normal * approach;
            let tangent = ball.velocity - normal_part;
            ball.velocity = tangent * Self::TANGENTIAL - normal_part * Self::RESTITUTION;
        }
        ball.spin *= Self::SPIN_RETAINED;
    }
}

impl Ball {
    /// Rebound the ball off either goal frame, if it has just travelled
    /// through one.
    ///
    /// Runs immediately after the move and before every out-of-play
    /// resolver, because a ball that has hit the frame has not crossed the
    /// line, gone over the bar or gone out — it is still in play, and every
    /// one of those resolvers would claim it. Returns the contact so the
    /// caller can raise the event.
    pub(super) fn check_goal_frame(&mut self, goals: &GoalPosition) -> Option<FrameHit> {
        // Only a loose ball. A carried one is being dribbled and its
        // position is written by `move_to` from the carrier's, so the
        // segment below is not a flight and reflecting it would tear the
        // ball out of his feet; and a ball already in the net is behind
        // the frame by design.
        if self.current_owner.is_some() || self.in_net.is_some() || GoalFrame::disabled() {
            return None;
        }
        // The frame check is the first thing after the move, so the ball's
        // previous position is exactly one tick of velocity back.
        let to = self.position;
        let from = to - self.velocity;
        let hit = GoalFrame::both(goals)
            .iter()
            .filter_map(|frame| frame.sweep(from, to))
            .min_by(|a, b| {
                a.travel
                    .partial_cmp(&b.travel)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })?;
        GoalFrame::rebound(self, &hit);
        Some(hit)
    }

    /// Rebound off the frame and settle what that means for the rest of the
    /// engine.
    ///
    /// The shot is OVER — a ball off the woodwork is not on target and did
    /// not reach the keeper, so `cached_shot_target` has to be retired here
    /// or the next thing the keeper collects is booked as a save (the same
    /// leak `check_over_goal` documents). What replaces it is the rebound
    /// window, because the follow-up from six yards is the whole reason the
    /// frame matters, and the team shot-spacing gate would otherwise
    /// swallow it.
    ///
    /// Ownership is deliberately left LOOSE and the shooter left as
    /// `previous_owner`: nobody played the ball, so the second-ball race is
    /// open to both sides on the same terms as any other loose ball in the
    /// six-yard box.
    pub(super) fn check_frame_rebound(
        &mut self,
        context: &crate::r#match::MatchContext,
        events: &mut crate::r#match::events::EventCollection,
    ) {
        let Some(hit) = self.check_goal_frame(&context.goal_positions) else {
            return;
        };
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::FrameDiag::note(hit.part);
        self.cached_shot_target = None;
        self.last_rebound_tick = self.current_tick_cached;
        // Long enough that the rebound is genuinely in the air before
        // anyone can gather it, short enough that the six-yard scramble
        // still happens — the same 20 ticks the loose branch of a block
        // uses.
        self.flags.in_flight_state = 20;
        self.claim_cooldown = 0;
        self.pass_target_player_id = None;
        self.offside_snapshot = None;
        events.add_ball_event(crate::r#match::ball::events::BallEvent::HitFrame(
            hit.part,
            hit.position,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::MatchFieldSize;

    fn goals() -> GoalPosition {
        GoalPosition::from(&MatchFieldSize::new(840, 545))
    }

    fn ball_at(position: Vector3<f32>, velocity: Vector3<f32>) -> Ball {
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.position = position;
        ball.velocity = velocity;
        ball
    }

    /// The two goals do NOT share a centre: `GoalPosition` builds the left
    /// one from `height as f32 / 2.0` and the right one from
    /// `(height / 2) as f32`, so on an odd-height pitch they differ by half
    /// a unit. Every geometric test here has to read the real centre rather
    /// than assume it, or it drifts by exactly the amount that decides
    /// whether a ball clips a post.
    fn right_post_axis() -> f32 {
        let frame = GoalFrame {
            line_x: goals().right.x,
            centre_y: goals().right.y,
        };
        frame.centre_y + frame.post_offset()
    }

    fn left_post_axis() -> f32 {
        let frame = GoalFrame {
            line_x: goals().right.x,
            centre_y: goals().right.y,
        };
        frame.centre_y - frame.post_offset()
    }

    /// Lateral offset, in game units, at which the ball's surface just
    /// brushes a member's surface.
    fn brushing_offset() -> f32 {
        (GoalFrame::POST_RADIUS + GoalFrame::BALL_RADIUS) * GoalFrame::U_PER_M * 0.88
    }

    /// The headline case: a shot aimed a whisker outside the post used to
    /// run out for a goal kick, passing through the post the viewer draws.
    #[test]
    fn a_shot_onto_the_post_comes_back_off_it() {
        let g = goals();
        // Straight at the right-hand goal's near post, at knee height.
        let mut ball = ball_at(
            Vector3::new(838.0, right_post_axis(), 0.5),
            Vector3::new(2.4, 0.0, 0.0),
        );
        ball.position += ball.velocity;
        let hit = ball.check_goal_frame(&g).expect("the post is in the way");
        assert_eq!(hit.part, FramePart::RightPost);
        assert!(
            ball.velocity.x < 0.0,
            "the ball must come back off the post, vx {}",
            ball.velocity.x
        );
        assert!(
            ball.velocity.x.abs() < 2.4,
            "and slower than it arrived, vx {}",
            ball.velocity.x
        );
    }

    /// A ball driven inside the frame is a goal, not a rebound. The inner
    /// faces are the nominal dimensions — put the axes on them instead and
    /// half of every member protrudes into the goal.
    ///
    /// "Inside" is the ball's SURFACE, not its centre: the clearances below
    /// are a ball's radius plus a little, because a football whose centre
    /// passes 10 cm inside the post genuinely brushes it.
    #[test]
    fn the_frame_does_not_intrude_on_the_goal() {
        let g = goals();
        let centre = g.right.y;
        // Just clear of a brushing contact, on the goal side of each member.
        let clear = brushing_offset() * 1.25;
        for (y, z) in [
            (right_post_axis() - clear, 1.0),
            (centre, GOAL_HEIGHT + GoalFrame::POST_RADIUS - clear * 0.125),
            (left_post_axis() + clear, 2.0),
        ] {
            let mut ball = ball_at(Vector3::new(838.0, y, z), Vector3::new(2.4, 0.0, 0.0));
            ball.position += ball.velocity;
            assert!(
                ball.check_goal_frame(&g).is_none(),
                "a shot inside the frame at y={y} z={z} must not touch it"
            );
        }
    }

    /// The other half of the same rule: a shot that clips the INSIDE of the
    /// post goes in off it. That is the most common thing the woodwork
    /// does, and a model that simply sent everything back out would be
    /// worse than having no frame at all.
    #[test]
    fn a_shot_off_the_inside_of_the_post_is_turned_into_the_goal() {
        let g = goals();
        // On the goal side of the post, close enough that the ball's
        // surface catches it — a glancing clip, not a head-on strike.
        let y = right_post_axis() - brushing_offset();
        let mut ball = ball_at(Vector3::new(838.5, y, 0.6), Vector3::new(2.4, 0.0, 0.0));
        ball.position += ball.velocity;
        let hit = ball.check_goal_frame(&g).expect("it caught the post");
        assert_eq!(hit.part, FramePart::RightPost);
        assert!(
            ball.velocity.x > 0.0,
            "a glancing contact must not stop it dead, vx {}",
            ball.velocity.x
        );
        assert!(
            ball.velocity.y < 0.0,
            "and it has to be turned INTO the goal, vy {}",
            ball.velocity.y
        );
    }

    /// A ball leaves the boot at 40 cm a tick and a post plus a ball is
    /// 34 cm across, so a point test misses the hardest strikes — the ones
    /// a rebound matters for most.
    #[test]
    fn the_hardest_strike_cannot_tunnel_through_a_post() {
        let g = goals();
        let mut ball = ball_at(
            Vector3::new(838.5, left_post_axis(), 1.0),
            // The engine's ceiling, straight through the post's axis.
            Vector3::new(3.2, 0.0, 0.0),
        );
        ball.position += ball.velocity;
        assert!(
            ball.position.x > 841.4,
            "test is meaningless unless the step clears the post entirely, ended {}",
            ball.position.x
        );
        let hit = ball.check_goal_frame(&g).expect("swept test must catch it");
        assert_eq!(hit.part, FramePart::LeftPost);
        assert!(
            ball.position.x < 840.0,
            "and must leave the ball in FRONT of the line, at {}",
            ball.position.x
        );
    }

    /// Off the underside of the bar and back down into play — the one
    /// rebound in football that comes off nothing but the structure.
    #[test]
    fn a_shot_onto_the_bar_drops_back_into_play() {
        let g = goals();
        let mut ball = ball_at(
            // Rising at the bar, its centre just under the underside.
            Vector3::new(838.0, goals().right.y, GOAL_HEIGHT - 0.02),
            Vector3::new(2.6, 0.0, 0.02),
        );
        ball.position += ball.velocity;
        let hit = ball.check_goal_frame(&g).expect("the bar is in the way");
        assert_eq!(hit.part, FramePart::Crossbar);
        assert!(
            ball.velocity.x < 0.0,
            "it has to come back out, vx {}",
            ball.velocity.x
        );
        assert!(
            ball.velocity.z < 0.0,
            "and downward off the underside, vz {}",
            ball.velocity.z
        );
    }

    /// Resolving a contact must not leave the ball inside the surface, or
    /// the next tick fires again and the ball sticks to the post.
    #[test]
    fn a_rebound_never_traps_the_ball_against_the_frame() {
        let g = goals();
        let mut ball = ball_at(
            Vector3::new(838.5, right_post_axis() - 0.3, 0.6),
            Vector3::new(2.0, 0.4, 0.0),
        );
        ball.position += ball.velocity;
        assert!(ball.check_goal_frame(&g).is_some(), "first contact fires");
        for tick in 0..40 {
            ball.position += ball.velocity;
            let again = ball.check_goal_frame(&g);
            assert!(
                again.is_none() || tick > 0,
                "the same contact must not re-fire on the next tick"
            );
        }
        // And it has genuinely left the area rather than grinding along it.
        assert!(
            (ball.position.x - 840.0).abs() > 1.0,
            "the ball is still on the post at x={}",
            ball.position.x
        );
    }

    /// A ball nowhere near the frame must not pay for the test.
    #[test]
    fn open_play_never_touches_the_frame() {
        let g = goals();
        let mut ball = ball_at(
            Vector3::new(400.0, 300.0, 0.2),
            Vector3::new(-2.0, 0.5, 0.0),
        );
        ball.position += ball.velocity;
        assert!(ball.check_goal_frame(&g).is_none());
    }

    /// An owned ball is being dribbled: `move_to` writes its position from
    /// the carrier's, so `position - velocity` is not a flight and
    /// reflecting it would tear the ball out of his feet.
    #[test]
    fn a_carried_ball_is_not_swept() {
        let g = goals();
        let mut ball = ball_at(
            Vector3::new(840.0, right_post_axis(), 0.3),
            Vector3::new(2.4, 0.0, 0.0),
        );
        ball.current_owner = Some(7);
        assert!(ball.check_goal_frame(&g).is_none());
    }
}
