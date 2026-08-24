//! **The goalkeeper's body** — the one thing on a football pitch a ball is
//! not allowed to pass through, and the only obstacle in this engine that
//! had no volume at all.
//!
//! # What was missing
//!
//! Every other keeper model in this crate prices an ATTEMPT.
//! [`SaveModel`](super::save::SaveModel) asks how far his hands were from
//! the ball and rolls; `KeeperShotDive` asks whether to leave his feet.
//! Both can legitimately fail — a fingertip that does not quite get there
//! is real football, and most of the goals in the game come through it.
//!
//! A body is not an attempt. It occupies space, and a ball arriving at
//! that space stops being a probability question: it comes off him,
//! whether he read it or not, whether his hands were anywhere near it or
//! not. Nothing in the engine expressed that. `try_save_shot` rolls once
//! per shot (`ShotTarget::save_rolled`) and returns on a failure, and every
//! other route to the ball — a cross, a rebound, a shot already retired as
//! off-frame, a ball squirming loose in the six-yard box — never reaches a
//! keeper model at all. So a shot he was beaten by on the roll, and any
//! loose ball that happened to run at him, travelled straight through his
//! chest at full speed.
//!
//! Reported from the stands, verbatim: *"the ball passes through the
//! goalkeeper's body, no matter how he jumps; it should bounce off him."*
//!
//! # The model
//!
//! [`KeeperBody`] is a **capsule** — a segment with a radius — placed by
//! his posture, and [`Ball::try_keeper_body_block`] sweeps the ball's step
//! against it exactly the way [`GoalFrame`] sweeps it against a post. Same
//! shape of answer, same mixed-frame reflection, same
//! already-overlapping guard, so a ball that has just come off him cannot
//! grind against him for the next four ticks.
//!
//! The one thing it deliberately does NOT contain is a save roll. Whether
//! he was good enough to be there is the save model's question and it has
//! already been asked; this only asks whether he WAS there. Skill enters
//! once, and only where a body genuinely differs between keepers: how much
//! of the pace he kills rather than letting the ball spring off him.
//!
//! # Scope: the keeper, and only the keeper
//!
//! Twenty-one other bodies are on the pitch and none of them gets one of
//! these. That is deliberate rather than unfinished: an outfield player's
//! body in front of a shot is [`Ball::try_block_shot`], a calibrated
//! probability model with its own measured target band, and giving those
//! ten men hard volumes as well would resolve the same contact twice and
//! move a number the whole shooting model is tuned against. The keeper is
//! the one player whose body-in-the-way is not modelled anywhere else, is
//! stationary in front of an empty net when it matters, and is the one the
//! camera is pointed at.

use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::engine::ball::ball::boundary::frame::GoalFrame;
use crate::r#match::engine::goal::{GOAL_HEIGHT, GOAL_WIDTH};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffSkillCtx, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchContext, MatchPlayer, PassOriginRestart, PlayerSide};
use nalgebra::Vector3;

/// Where the ball met the man, and which way it has to leave.
#[derive(Debug, Clone, Copy)]
pub struct BodyHit {
    /// Contact point, in the stored mixed frame (x/y in game units, z in
    /// metres) — on the capsule's surface by construction.
    pub position: Vector3<f32>,
    /// Outward surface normal in the same mixed frame, scaled so its
    /// METRIC length is 1. See [`GoalFrame`]'s `unit` for why that is the
    /// property the reflection needs.
    pub normal: Vector3<f32>,
    /// How far through the step the contact happened, 0..1.
    pub travel: f32,
    /// Where along his body it hit, in metres from the hips and signed
    /// toward his head. Diagnostic only, and the one number that says
    /// whether these are chest saves or shin saves.
    pub along: f32,
}

/// **A goalkeeper's trunk, head and legs as a capsule**, posed.
///
/// The arms are deliberately NOT in it. A keeper's hands are his REACH,
/// and the save model already prices them as a radius about his position —
/// up to 4 m of it. Putting them in the capsule as well would make the
/// same arm both a probability and a certainty, and would turn every dive
/// into a four-metre solid wall.
pub struct KeeperBody {
    /// The capsule's axis in the stored mixed frame: `head` is the crown,
    /// `feet` the soles.
    head: Vector3<f32>,
    feet: Vector3<f32>,
}

impl KeeperBody {
    /// Horizontal game units per metre, and its inverse. Same pair and
    /// same reason as [`GoalFrame`]: the two horizontal axes are a 0.125 m
    /// grid and the vertical one is metric.
    const U_PER_M: f32 = 8.0;
    const M_PER_U: f32 = 0.125;

    /// Half the breadth of a trunk, in metres — the capsule's radius,
    /// before the ball's own is added.
    ///
    /// Read straight off the figure the viewer draws: the torso mesh's
    /// shoulder crest peaks **0.209 m** wide (see `Physique::SHOULDER`,
    /// which documents where the arm sockets are sunk relative to it), and
    /// a trunk is about the same front to back once the arms are in. A
    /// capsule about one axis cannot be a man's shoulders one way and his
    /// depth the other, so one number has to serve for both, and this is
    /// the one the eye is going to check it against.
    const TRUNK_RADIUS: f32 = 0.20;

    /// Distance from the axis at which the two surfaces touch —
    /// [`GoalFrame::BALL_RADIUS`] on top of the trunk, exactly as the
    /// woodwork does it.
    const CONTACT: f32 = Self::TRUNK_RADIUS + GoalFrame::BALL_RADIUS;

    /// Hips to crown, and hips to soles, in metres — measured to the ENDS
    /// OF THE AXIS, so the capsule's caps carry the last
    /// [`Self::TRUNK_RADIUS`] of him at each end.
    ///
    /// Which makes the figure self-consistent, and that is the point of
    /// writing them this way round. Both are pinned by the rig the viewer
    /// draws: `BELOW_HIP + TRUNK_RADIUS` is `Physique::HIP` (0.95 m — his
    /// soles are on the grass), and end to end the capsule is
    /// `ABOVE_HIP + BELOW_HIP + 2·TRUNK_RADIUS` = `Physique::STATURE`
    /// (**1.79 m**, crown to turf). So the same man stood on end and laid
    /// on his side is the same man, and he is the man on the screen —
    /// which is what [`Self::topple`] interpolates between.
    ///
    /// One figure for every keeper, deliberately: the engine's
    /// [`MatchPlayer`] carries no stature, and inventing one here would put
    /// a number into the contact test that nothing else in the match can
    /// see or check.
    const ABOVE_HIP: f32 = 0.64;
    const BELOW_HIP: f32 = 0.75;

    /// Height of the hips standing, and lying flat out, in metres.
    ///
    /// Standing is `BELOW_HIP + TRUNK_RADIUS` by construction — his soles
    /// are on the grass — and lying is the pair the replay rig pivots a
    /// dive about (`Physique::HIP` and `Carriage::LYING` in the viewer).
    /// They have to BE that pair: the engine decides whether the ball hits
    /// him and the viewer draws whether it looks like it did, and if the
    /// two disagree about where a diving keeper's middle is, one of them is
    /// lying to the man watching.
    const HIP_STANDING: f32 = Self::BELOW_HIP + Self::TRUNK_RADIUS;
    const HIP_LYING: f32 = 0.19;

    /// AI ticks a body takes to go from upright to flat out.
    ///
    /// **220 ms, and it is the replay rig's number** — `Actors::EXTENSION`,
    /// measured off recorded dives: a keeper is airborne for 390-660 ms and
    /// opens out across roughly the first half of it, reaching full stretch
    /// around the apex. The viewer draws the tilt off that ramp, so the
    /// engine has to go over on the same one or the two disagree about
    /// where his chest is for a fifth of a second — which is exactly long
    /// enough for a shot to arrive.
    ///
    /// `in_state_time` counts AI ticks at 20 ms each; see the units note in
    /// `GoalkeeperDivingState::process`. He is a little over already on the
    /// tick he leaves the ground, because a dive starts with the shoulder.
    const TOPPLE_TICKS: f32 = 11.0;
    const TOPPLE_AT_TAKEOFF: f32 = 1.0;

    /// How much of its approach speed the ball keeps off a man.
    ///
    /// Nothing like the 0.65 it keeps off aluminium: a torso gives, and a
    /// keeper is actively trying to kill the ball rather than stand still
    /// and be hit by it. 0.30 is about what a ball does off a chest — it
    /// drops in front of him rather than flying back out to the edge of
    /// the box, which is the picture everybody recognises and the one that
    /// leaves the rebound where the six-yard scramble can happen.
    const RESTITUTION: f32 = 0.30;

    /// Speed retained ACROSS the contact normal. A shirt is not slick, and
    /// a ball skidding off a hip loses most of its sideways travel.
    const TANGENTIAL: f32 = 0.55;

    /// Rotation surviving the contact — as with the woodwork, nearly none.
    const SPIN_RETAINED: f32 = 0.25;

    /// How much of the rebound a keeper's HANDLING is worth, either way.
    ///
    /// The one place skill belongs in a collision. A body is a body, but a
    /// good keeper takes the pace off what hits him — chest down, hands
    /// behind it, the ball dying at his feet — and a poor one lets it
    /// spring away into the six-yard box. Centred on the measured
    /// population mean (`SaveModel::POPULATION_HANDLING`) so a median
    /// keeper rebounds at exactly [`Self::RESTITUTION`] and the band opens
    /// around him; same shape and same reason as the `hands` multiplier in
    /// the save model.
    const HANDS_CUSHION: f32 = 0.55;

    /// Diagnostic switch: with `OF_KEEPER_BODY=off` the goalkeeper has no
    /// volume and the ball passes through him exactly as it did before
    /// this module existed.
    ///
    /// The A/B control for the body. It converts a share of the balls that
    /// used to go through him into saves and rebounds, so it moves the
    /// population save rate, and "what did this cost?" cannot be answered
    /// by reading the diff. Same pattern and purpose as `OF_FRAME_OFF` and
    /// `OF_SAVE_AT_LINE`; read once per process. Debug infrastructure — do
    /// not remove.
    pub fn disabled() -> bool {
        static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *OFF.get_or_init(|| {
            std::env::var("OF_KEEPER_BODY")
                .map(|v| v == "off" || v == "0")
                .unwrap_or(false)
        })
    }

    /// **How far over he is**, 0 upright and 1 flat out.
    ///
    /// Read off the state machine rather than off his velocity, because a
    /// keeper is at his most horizontal at the END of a dive, when he is
    /// lying on the grass travelling at nothing at all. `Diving` is the one
    /// state that means *off his feet* — which is exactly the property the
    /// viewer latches its own `dive` from (`Actors::track_flight` reads the
    /// recorded height for the same statement). `Jumping` is deliberately
    /// NOT here: a keeper going straight up at a corner stays upright, and
    /// only his hips move.
    fn topple(player: &MatchPlayer) -> f32 {
        match player.state {
            PlayerState::Goalkeeper(GoalkeeperState::Diving) => {
                ((player.in_state_time as f32 + Self::TOPPLE_AT_TAKEOFF) / Self::TOPPLE_TICKS)
                    .clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    /// The horizontal direction his body is laid out along.
    ///
    /// A body in the air travels along the vector it launched on, so his
    /// own heading IS the axis while he is still covering ground. Once the
    /// dive has run out of pace there is nothing left in the velocity to
    /// read, and the fallback is the axis a keeper's dive lies along
    /// essentially always: across his goal, which on this pitch is `y`.
    fn lie(player: &MatchPlayer) -> Vector3<f32> {
        let speed = player.velocity.x.hypot(player.velocity.y);
        if speed > 0.05 {
            Vector3::new(player.velocity.x / speed, player.velocity.y / speed, 0.0)
        } else {
            Vector3::new(0.0, 1.0, 0.0)
        }
    }

    /// Place this keeper's body, given where he is and what he is doing.
    ///
    /// `position` is his pitch coordinate and `height` the metres his hips
    /// are off the turf. The two live apart inside the engine and are only
    /// brought together at sites like this one — see [`MatchPlayer::height`].
    pub fn of(player: &MatchPlayer) -> Self {
        let over = Self::topple(player);
        // The axis, interpolated between straight up and laid out flat, in
        // the SCALED frame where a metre is the same length on all three
        // axes — so this is an honest rotation rather than a rotation
        // through a shear.
        let flat = Self::lie(player);
        let axis = {
            let v = Vector3::new(flat.x * over, flat.y * over, 1.0 - over);
            v / v.norm().max(1.0e-6)
        };
        // Where the hips end up. They travel DOWN as the body goes over —
        // a man on his side carries them a hand's width off the grass — and
        // up by whatever he has leapt. Same expression the viewer's
        // `Carriage::placed` settles a figure with.
        let hip_z = Self::HIP_STANDING - (Self::HIP_STANDING - Self::HIP_LYING) * over
            + player.height.max(0.0);
        let hips = Vector3::new(player.position.x, player.position.y, hip_z);
        // Back out of the scaled frame: the axis is a direction in metres,
        // so its horizontal parts become game units and its vertical part
        // stays metric.
        let along = |metres: f32| {
            Vector3::new(
                axis.x * metres * Self::U_PER_M,
                axis.y * metres * Self::U_PER_M,
                axis.z * metres,
            )
        };
        KeeperBody {
            head: hips + along(Self::ABOVE_HIP),
            feet: hips - along(Self::BELOW_HIP),
        }
    }

    /// **The cheap rejection, and it does nearly all the work.**
    ///
    /// The sweep below is a convex minimisation followed by a bisection —
    /// forty-odd dot products — and it would otherwise run for both keepers
    /// on every tick the ball is loose and moving, which is most of a
    /// match. Nothing about a body can reach further than his own length
    /// plus a contact radius, so one squared distance in the pitch plane
    /// rejects all but a handful of ticks a match before any of that.
    ///
    /// Horizontal because that is where the rejection is: the capsule can
    /// only lie [`Self::BELOW_HIP`] from his feet-mark, whatever he is
    /// doing with the vertical axis.
    fn within_arm_of(player: &MatchPlayer, ball: Vector3<f32>, step: Vector3<f32>) -> bool {
        let span = (Self::BELOW_HIP.max(Self::ABOVE_HIP) + Self::CONTACT) * Self::U_PER_M
            + step.x.hypot(step.y);
        let dx = ball.x - player.position.x;
        let dy = ball.y - player.position.y;
        dx * dx + dy * dy <= span * span
    }

    /// The top and bottom of him, in metres off the turf.
    ///
    /// Not used by the contact itself — the sweep works off the axis — but
    /// it is how the posture is checked against the replay rig, which is
    /// the one thing about this model that can go wrong silently. See
    /// `keeper_body_tests`.
    pub fn envelope(&self) -> (f32, f32) {
        (
            self.head.z.max(self.feet.z) + Self::TRUNK_RADIUS,
            self.head.z.min(self.feet.z) - Self::TRUNK_RADIUS,
        )
    }

    /// …and how much of the goal mouth he covers with his body alone, in
    /// metres. Half a metre standing, most of two flat out.
    pub fn reach_across(&self) -> f32 {
        (self.head.y - self.feet.y).abs() * Self::M_PER_U + 2.0 * Self::TRUNK_RADIUS
    }

    /// The contact this body makes with a ball travelling from `from` to
    /// `to`, or `None` if it never touches him inside this step.
    ///
    /// Solved in metres: the mixed frame is metric through a diagonal map,
    /// so converting once at the top is both cheaper and far less
    /// error-prone than carrying the scale through a segment-to-segment
    /// solve. What comes back out is in the stored frame, because that is
    /// what the ball is written in.
    ///
    /// A ball ALREADY overlapping him at the start of the step is not a
    /// contact, for exactly the reason `GoalFrame::first_contact` gives:
    /// it means the previous tick already resolved one and pushed the ball
    /// to the surface, and re-firing on the rounding would pin it against
    /// him. It is also what keeps his own parry, his own spill and his own
    /// punt from bouncing straight back off him.
    pub fn sweep(&self, from: Vector3<f32>, to: Vector3<f32>) -> Option<BodyHit> {
        let p0 = Self::metric(from);
        let step = Self::metric(to) - p0;
        if step.norm_squared() < 1.0e-12 {
            return None;
        }
        let feet = Self::metric(self.feet);
        let head = Self::metric(self.head);
        // Already inside him — see the note above.
        if Self::gap(p0, feet, head) <= Self::CONTACT {
            return None;
        }
        // The closest the step ever comes. `t ↦ dist(P(t), body)` is convex
        // — a distance to a convex set composed with an affine path — so if
        // the minimum clears the surface nothing else can reach it, and if
        // it does not, the entry is the single crossing below it.
        let (closest, gap) = Self::closest_approach(p0, step, feet, head);
        if gap > Self::CONTACT {
            return None;
        }
        // Bisect that convex distance down to the surface. Twenty halvings
        // of a step at most a few metres long land the contact well inside
        // a micrometre, far below anything downstream can see, and it costs
        // nothing: this runs at most once a tick for one player.
        let (mut lo, mut hi) = (0.0f32, closest);
        for _ in 0..20 {
            let mid = 0.5 * (lo + hi);
            if Self::gap(p0 + step * mid, feet, head) > Self::CONTACT {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let contact = p0 + step * hi;
        let (axis_point, along) = Self::closest_on_body(contact, feet, head);
        let out = contact - axis_point;
        let len = out.norm();
        let normal = if len > 1.0e-6 {
            out / len
        } else {
            // Dead down his spine, which the bisection can only reach on a
            // ball travelling exactly along the axis. Push it back the way
            // it came rather than picking a side at random.
            -step / step.norm()
        };
        Some(BodyHit {
            position: Self::mixed(contact),
            normal: Vector3::new(normal.x * Self::U_PER_M, normal.y * Self::U_PER_M, normal.z),
            travel: hi,
            along,
        })
    }

    /// The point on the body axis nearest `point`, and how far that is from
    /// the hips in metres, signed toward the head.
    fn closest_on_body(
        point: Vector3<f32>,
        feet: Vector3<f32>,
        head: Vector3<f32>,
    ) -> (Vector3<f32>, f32) {
        let axis = head - feet;
        let len2 = axis.norm_squared();
        if len2 < 1.0e-12 {
            return (feet, 0.0);
        }
        let s = ((point - feet).dot(&axis) / len2).clamp(0.0, 1.0);
        (
            feet + axis * s,
            s * (Self::BELOW_HIP + Self::ABOVE_HIP) - Self::BELOW_HIP,
        )
    }

    /// Distance from a point to the body segment, in metres.
    fn gap(point: Vector3<f32>, feet: Vector3<f32>, head: Vector3<f32>) -> f32 {
        let (on, _) = Self::closest_on_body(point, feet, head);
        (point - on).norm()
    }

    /// The closest approach between the ball's step and the body, as
    /// `(travel, gap)` — the parameter along the step, and the distance
    /// there in metres.
    ///
    /// A golden-section search rather than the closed-form segment-segment
    /// solve, because the closed form has four degenerate branches (either
    /// segment short, the two parallel, the closest points falling off both
    /// ends) and this has none: the function is convex, and forty
    /// evaluations of a dot product cost less than getting one of those
    /// branches subtly wrong on the one contact a match that matters.
    fn closest_approach(
        p0: Vector3<f32>,
        step: Vector3<f32>,
        feet: Vector3<f32>,
        head: Vector3<f32>,
    ) -> (f32, f32) {
        const PHI: f32 = 0.618_034;
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        let mut c = hi - PHI * (hi - lo);
        let mut d = lo + PHI * (hi - lo);
        let mut fc = Self::gap(p0 + step * c, feet, head);
        let mut fd = Self::gap(p0 + step * d, feet, head);
        for _ in 0..40 {
            if fc < fd {
                hi = d;
                d = c;
                fd = fc;
                c = hi - PHI * (hi - lo);
                fc = Self::gap(p0 + step * c, feet, head);
            } else {
                lo = c;
                c = d;
                fc = fd;
                d = lo + PHI * (hi - lo);
                fd = Self::gap(p0 + step * d, feet, head);
            }
        }
        let t = 0.5 * (lo + hi);
        (t, Self::gap(p0 + step * t, feet, head))
    }

    /// Stored mixed frame → metres.
    fn metric(v: Vector3<f32>) -> Vector3<f32> {
        Vector3::new(v.x * Self::M_PER_U, v.y * Self::M_PER_U, v.z)
    }

    /// …and back.
    fn mixed(v: Vector3<f32>) -> Vector3<f32> {
        Vector3::new(v.x * Self::U_PER_M, v.y * Self::U_PER_M, v.z)
    }

    /// Metric dot product of two mixed-frame vectors. The same one
    /// [`GoalFrame`] carries, and it has to be: the reflection below is
    /// ordinary vector algebra done in metres and written in the frame the
    /// velocity is stored in.
    fn dot_metres(a: Vector3<f32>, b: Vector3<f32>) -> f32 {
        (a.x * Self::M_PER_U) * (b.x * Self::M_PER_U)
            + (a.y * Self::M_PER_U) * (b.y * Self::M_PER_U)
            + a.z * b.z
    }

    /// Put the ball on the surface and turn its velocity, with `cushion`
    /// the share of the ordinary rebound this keeper's hands leave on it.
    fn rebound(ball: &mut Ball, hit: &BodyHit, cushion: f32) {
        /// 3 mm out along the normal, so the already-overlapping guard
        /// cannot latch on the rounding and hold the ball against him.
        const CLEARANCE_METRES: f32 = 0.003;

        let approach = Self::dot_metres(ball.velocity, hit.normal);
        if approach < 0.0 {
            let normal_part = hit.normal * approach;
            let tangent = ball.velocity - normal_part;
            ball.velocity =
                tangent * Self::TANGENTIAL - normal_part * (Self::RESTITUTION * cushion);
        }
        ball.spin *= Self::SPIN_RETAINED;
        // The rest of the tick still has to be travelled — same reasoning
        // as the woodwork's, and the same one-frame hitch if it is not.
        ball.position = hit.position
            + hit.normal * CLEARANCE_METRES
            + ball.velocity * (1.0 - hit.travel).clamp(0.0, 1.0);
        if ball.position.z < 0.0 {
            ball.position.z = 0.0;
        }
    }
}

impl Ball {
    /// Bounce the ball off a goalkeeper's body, if it has just travelled
    /// through him.
    ///
    /// Runs after ownership and before the move, and both halves matter.
    /// AFTER, because a ball he is entitled to control is his — a back-pass
    /// rolling to his feet is a reception, not a collision, and
    /// `process_ownership` is what tells the two apart. BEFORE, because the
    /// step this sweeps is the one the move is about to make, and resolving
    /// a contact the tick after it happened is how a ball ends up a third
    /// of a metre inside a man before anything notices.
    ///
    /// It is the BALL's step that is swept and not the keeper's, so a man
    /// walking into a stationary ball does not kick it away — which is
    /// correct, because that is a player arriving at a loose ball and
    /// `check_ball_ownership` is what decides those.
    pub fn try_keeper_body_block(&mut self, context: &MatchContext, players: &[MatchPlayer]) {
        // A ball somebody has is a ball under control, by definition — his
        // own included. And one waiting for a restart is out of play; see
        // `DeadBall`.
        if self.current_owner.is_some() || self.awaiting_restart.is_some() || self.in_net.is_some()
        {
            return;
        }
        let step = self.velocity;
        if step.x * step.x + step.y * step.y + (step.z * KeeperBody::U_PER_M).powi(2) < 1.0e-6 {
            return;
        }

        // Both keepers, not only the one being shot at. A ball can run at a
        // goalkeeper from anywhere — his own clearance charged down, a
        // cross-field ball he has come for — and none of those routes has a
        // defending side to read.
        let from = self.position;
        let to = self.position + step;
        let Some((keeper, hit)) = players
            .iter()
            .filter(|p| {
                p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
                    && !p.is_sent_off
                    && KeeperBody::within_arm_of(p, from, step)
                    // A ball he has just put into play and which has not
                    // gone anywhere yet: his own punt leaving his boot, his
                    // own parry leaving his hands. Bounded on both distance
                    // and time — see `blocked_recollect_player`.
                    && self.blocked_recollect_player() != Some(p.id)
                    // …and one still travelling out of him from the contact
                    // that put it there.
                    && !(self.previous_owner == Some(p.id) && self.flags.in_flight_state > 0)
            })
            .filter_map(|p| KeeperBody::of(p).sweep(from, to).map(|hit| (p, hit)))
            .min_by(|a, b| {
                a.1.travel
                    .partial_cmp(&b.1.travel)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        else {
            return;
        };

        let keeper_id = keeper.id;
        let keeper_team = keeper.team_id;
        let keeper_side = keeper.side;
        // Had he thrown himself at it, or was he simply hit? The two look
        // completely different and `apply_pending_save_credit` chooses
        // between them off `pending_save_reach` — above its dive bar he
        // stays down and gets up, below it he stands there having blocked
        // one. Reported as full stretch for a man already off his feet so
        // that site does not stand a diving keeper up mid-dive, and as none
        // at all for one who was set: a body block is never a stretch.
        let already_diving = matches!(
            keeper.state,
            PlayerState::Goalkeeper(GoalkeeperState::Diving)
        ) || keeper.is_airborne();

        // Was this a shot he was in the way of? Only then is it a SAVE — a
        // cross clipping his hip is a deflection and nothing more, and
        // booking it as a save is the same accounting leak `check_over_goal`
        // documents on the other side.
        let saved_a_shot = self
            .cached_shot_target
            .filter(|t| Some(t.defending_side) == keeper_side)
            .is_some_and(|t| {
                let goal = match t.defending_side {
                    PlayerSide::Left => context.goal_positions.left,
                    PlayerSide::Right => context.goal_positions.right,
                };
                let (frame_y, frame_z) = self.projected_crossing(goal.x);
                (frame_y - goal.y).abs() <= GOAL_WIDTH && frame_z <= GOAL_HEIGHT
            });

        // How much pace he takes off it. See `KeeperBody::HANDS_CUSHION`.
        let minute = sc::minute_from_ms(context.total_match_time);
        let handling = effective_skill(
            keeper,
            keeper.skills.goalkeeping.handling,
            EffSkillCtx::technical(minute),
        );
        let scaled_handling = ((handling - 1.0) / 19.0).clamp(0.0, 1.0);
        let cushion = (1.0
            - (scaled_handling - super::save::SaveModel::POPULATION_HANDLING)
                * KeeperBody::HANDS_CUSHION)
            .clamp(0.35, 1.65);

        // **Counted whether or not the volume is armed**, and that is the
        // point of counting it here. With `OF_KEEPER_BODY=off` the sweep
        // still runs and the census still books the contact, so the same
        // harness row reads "balls that went through him" on one side of
        // the A/B and "balls that came off him" on the other. Putting the
        // switch above this would have made the control arm report nothing
        // at all, which is the one thing it exists to report.
        #[cfg(feature = "match-logs")]
        {
            let own_goal = match keeper_side {
                Some(PlayerSide::Left) => context.goal_positions.left,
                Some(PlayerSide::Right) => context.goal_positions.right,
                None => keeper.position,
            };
            crate::mid_run_diag::KeeperBodyDiag::note_block(
                self.velocity.norm(),
                hit.along,
                hit.position.z,
                saved_a_shot,
                keeper.height,
                (hit.position.x - own_goal.x).abs(),
            );
        }
        if KeeperBody::disabled() {
            return;
        }

        KeeperBody::rebound(self, &hit, cushion);

        let tick = self.current_tick_cached;
        // Nobody played it deliberately, so it stays LOOSE and the six-yard
        // race is open to both sides on the same terms — the treatment a
        // ball off the woodwork gets, and for the same reason.
        self.pass_target_player_id = None;
        self.offside_snapshot = None;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;
        self.last_rebound_tick = tick;
        // Long enough that the rebound is genuinely away from him before
        // anyone can gather it, short enough that the scramble still
        // happens — the same 20 ticks the frame and the loose branch of a
        // block use.
        self.flags.in_flight_state = 20;
        self.claim_cooldown = 0;
        if saved_a_shot {
            if let Some(shooter_id) = self.previous_owner {
                self.pending_save_credit = Some((keeper_id, shooter_id));
                self.pending_save_reach = if already_diving { 1.0 } else { 0.0 };
                #[cfg(feature = "match-logs")]
                crate::save_accounting_stats::PENDING_STAGED
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // Its own accounting site: this is not a parry — a parry is a
                // save he MADE, and this is one made for him by standing in
                // the right place. See `save_accounting_stats::SITE_LABELS`.
                self.pending_save_site = 3; // body
            }
        }
        self.cached_shot_target = None;
        // He is the last man it came off, which is what the endline
        // resolver reads, and he is `previous_owner` so the second-ball
        // population is the realistic one — attackers pouncing on a rebound
        // rather than the ball still counting as the shooter's.
        self.previous_owner = Some(keeper_id);
        self.record_touch(keeper_id, keeper_team, tick, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A capsule for a man standing still at the origin.
    fn standing() -> KeeperBody {
        KeeperBody {
            head: Vector3::new(0.0, 0.0, KeeperBody::HIP_STANDING + KeeperBody::ABOVE_HIP),
            feet: Vector3::new(0.0, 0.0, KeeperBody::HIP_STANDING - KeeperBody::BELOW_HIP),
        }
    }

    /// …and the same man laid out flat across his goal, hips on the deck.
    fn sprawled() -> KeeperBody {
        KeeperBody {
            head: Vector3::new(
                0.0,
                KeeperBody::ABOVE_HIP * KeeperBody::U_PER_M,
                KeeperBody::HIP_LYING,
            ),
            feet: Vector3::new(
                0.0,
                -KeeperBody::BELOW_HIP * KeeperBody::U_PER_M,
                KeeperBody::HIP_LYING,
            ),
        }
    }

    /// One tick of a shot arriving from a metre in front of him.
    fn arriving_at(y: f32, z: f32) -> (Vector3<f32>, Vector3<f32>) {
        (Vector3::new(-8.0, y, z), Vector3::new(8.0, 0.0, 0.0))
    }

    /// The whole report in one test: a ball struck at a standing man's
    /// chest does not come out the other side.
    #[test]
    fn a_ball_at_his_chest_does_not_go_through_him() {
        let (from, step) = arriving_at(0.0, 1.1);
        let hit = standing()
            .sweep(from, from + step)
            .expect("a ball aimed at his chest has to hit him");
        // It stops in FRONT of him: at the surface, a trunk and a ball's
        // radius out from his spine.
        assert!(hit.position.x < 0.0, "contact at {:.2}u", hit.position.x);
        let out = (hit.position.x * KeeperBody::M_PER_U).abs();
        assert!(
            (out - KeeperBody::CONTACT).abs() < 0.01,
            "contact {out:.3} m from the axis, want {:.3}",
            KeeperBody::CONTACT
        );
        // …and the normal points back at where it came from.
        assert!(hit.normal.x < 0.0);
    }

    /// Past his shoulder is past him. The capsule is a body, not a wall —
    /// anything wider than a man is his hands' problem, and his hands are
    /// the save model.
    #[test]
    fn a_ball_wide_of_him_is_not_a_body_block() {
        // A metre to his side: well inside a dive, well outside a trunk.
        let (from, step) = arriving_at(8.0, 1.1);
        assert!(standing().sweep(from, from + step).is_none());
    }

    /// Over his head is over him, whatever the save model makes of it.
    #[test]
    fn a_ball_over_his_head_is_not_a_body_block() {
        let (from, step) = arriving_at(0.0, 2.3);
        assert!(standing().sweep(from, from + step).is_none());
    }

    /// **A dive is a low wide obstacle and a standing man a tall thin
    /// one** — the same body, and the whole reason the posture exists.
    #[test]
    fn going_over_trades_height_for_width() {
        let along_the_floor = |body: &KeeperBody| {
            let (from, step) = arriving_at(5.0, 0.10);
            body.sweep(from, from + step).is_some()
        };
        assert!(!along_the_floor(&standing()), "he is not that wide upright");
        assert!(along_the_floor(&sprawled()), "he is that wide flat out");

        let at_his_head = |body: &KeeperBody| {
            let (from, step) = arriving_at(0.0, 1.5);
            body.sweep(from, from + step).is_some()
        };
        assert!(at_his_head(&standing()), "chest height, standing");
        assert!(
            !at_his_head(&sprawled()),
            "he is on the floor — it goes over him"
        );
    }

    /// A ball already inside him is not a fresh contact. This is what keeps
    /// his own parry — which starts AT the contact point — from bouncing
    /// straight back off him for the next four ticks.
    #[test]
    fn a_ball_leaving_him_does_not_hit_him_again() {
        let inside = Vector3::new(0.0, 0.0, 1.1);
        assert!(
            standing()
                .sweep(inside, inside + Vector3::new(2.0, 0.0, 0.0))
                .is_none()
        );
    }

    /// A shot does not travel a whole tick before anybody notices: the
    /// contact resolves where it happened, part-way through the step.
    #[test]
    fn the_contact_is_solved_inside_the_step() {
        // 2.4 u/tick — an ordinary strike — from half a metre out.
        let from = Vector3::new(-4.0, 0.0, 1.1);
        let hit = standing()
            .sweep(from, from + Vector3::new(2.4, 0.0, 0.0))
            .expect("it reaches him inside the step");
        assert!(
            (0.0..1.0).contains(&hit.travel),
            "travel {:.3} is not inside the step",
            hit.travel
        );
    }

    /// The rebound comes back off him, slower, and ends up outside him.
    #[test]
    fn it_comes_off_him_slower_and_outward() {
        let body = standing();
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.position = Vector3::new(-4.0, 0.0, 1.1);
        ball.velocity = Vector3::new(2.4, 0.0, 0.0);
        let before = ball.velocity.norm();
        let hit = body
            .sweep(ball.position, ball.position + ball.velocity)
            .expect("hits him");
        KeeperBody::rebound(&mut ball, &hit, 1.0);
        assert!(ball.velocity.x < 0.0, "it has to come back out");
        assert!(
            ball.velocity.norm() < before * 0.6,
            "a body kills the pace: {:.2} -> {:.2}",
            before,
            ball.velocity.norm()
        );
        // …and it ends up outside him, so the next tick's overlap guard
        // does not fire on the rounding.
        let feet = KeeperBody::metric(body.feet);
        let head = KeeperBody::metric(body.head);
        assert!(
            KeeperBody::gap(KeeperBody::metric(ball.position), feet, head) > KeeperBody::CONTACT
        );
    }

    /// The capsule is a footballer-sized object: 1.83 m end to end,
    /// standing on the grass rather than hovering above it or buried in it.
    #[test]
    fn the_capsule_is_a_footballer_sized_object() {
        let body = standing();
        let crown = body.head.z + KeeperBody::TRUNK_RADIUS;
        let soles = body.feet.z - KeeperBody::TRUNK_RADIUS;
        assert!((crown - 1.79).abs() < 0.01, "crown at {crown:.2} m");
        assert!(soles.abs() < 0.01, "soles at {soles:.2} m");
        let length = (body.head.z - body.feet.z) + 2.0 * KeeperBody::TRUNK_RADIUS;
        assert!(
            (1.7..1.9).contains(&length),
            "a man is {length:.2} m end to end"
        );
    }
}
