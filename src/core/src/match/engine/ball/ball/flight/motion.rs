//! Ball physics and movement: velocity integration with friction /
//! drag / gravity / spin / bounce, owner tracking that lets the ball
//! follow a possessing player smoothly, and the boundary-collision inset
//! that pushes the ball back inside the field after it crosses a
//! touchline.

#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::ownership::reception_diag;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::strike_diag::StrikeCensus;
use crate::r#match::engine::ball::ball::{
    Ball, GRAVITY_PER_TICK, LOOSE_CLAIM_DISTANCE, MAX_OWNER_TRACK_DISTANCE,
};
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

/// Ball rotation and the aerodynamic force it produces.
///
/// # Why the flight was a straight line before this
///
/// Every term in `update_velocity` used to be parallel or antiparallel to
/// the velocity (rolling friction, air drag) or acted on `z` alone
/// (gravity, bounce). None of them can rotate the velocity vector in the
/// XY plane, so the heading a ball left the foot with was frozen for the
/// whole flight: no curled shot, no whipped cross that bends away from
/// the keeper, no inswinging corner, no dip. The only thing called "spin"
/// in the engine was a `0.90..1.10` multiplier on a shot's apex — height
/// noise, not rotation.
///
/// The Magnus force is what is missing, and it is the only force here
/// with a component *perpendicular* to travel.
///
/// # Units
///
/// The ball's axes carry different units on purpose — `x`/`y` in game
/// units (1u = 0.125 m), `z` in metres (see [`GRAVITY_PER_TICK`]) — so a
/// cross product taken directly on the stored velocity would silently mix
/// scales. Everything here converts to a metric frame, does the physics,
/// and converts back.
///
/// Angular velocity is stored in **rad/tick**, matching the linear
/// velocity's per-tick convention. That choice makes
/// [`MAGNUS_COEFF`](Self::MAGNUS_COEFF) scale-invariant: with
/// `a = C·(ω × v)`, substituting `ω_s = ω_t/Δt` and `v_s = v_t/Δt` gives
/// `a_s = C·ω_t·v_t/Δt²`, and `a_t = a_s·Δt²` returns the same `C`. So the
/// coefficient below is a real SI number, not a tick-fitted one.
pub struct SpinModel;

impl SpinModel {
    /// Horizontal game units per metre.
    const U_PER_M: f32 = 8.0;
    /// Metres per horizontal game unit.
    const M_PER_U: f32 = 0.125;

    /// Magnus coefficient in `a = C·(ω × v)`, SI (and, per the note above,
    /// per-tick too).
    ///
    /// Calibrated against the canonical case rather than fitted: a free
    /// kick struck at 30 m/s with ~10 rev/s of sidespin (63 rad/s) bends
    /// about 3 m over a 25 m flight (~0.9 s). That needs a lateral
    /// acceleration of `2·d/t² ≈ 7.4 m/s²`, and `7.4 / (63 × 30) = 0.0039`.
    pub const MAGNUS_COEFF: f32 = 0.0039;

    /// Spin bleeds off slowly — a struck ball is still turning when it
    /// arrives. ~3% per second at 100 ticks/s.
    const DECAY_PER_TICK: f32 = 0.9997;

    /// Below this the rotation is not worth integrating.
    const NEGLIGIBLE: f32 = 1.0e-5;

    /// Ceiling on imparted rotation, rad/tick. 1.2 rad/tick is ~19 rev/s
    /// — beyond anything a human generates, so it only ever catches a bug.
    pub const MAX_SPIN: f32 = 1.2;

    /// Bring any rotation inside [`Self::MAX_SPIN`], preserving its axis.
    ///
    /// # Why this is a public helper and not just an internal detail
    ///
    /// [`Self::from_strike`] applies the ceiling itself, but it is not the
    /// only producer of spin. The cross and shot paths do not pick a
    /// rotation — they SOLVE for the one that delivers a wanted sideways
    /// deflection, inverting `d = ½·C·ω·|v|·T²` to get
    /// `ω = 2·d·|v| / (C·dist²)`. That expression has `dist²` in the
    /// denominator, so a delivery aimed to swing a couple of metres over a
    /// very short distance asks for a rotation in the thousands of
    /// rad/tick, and both sites assigned it raw.
    ///
    /// The consequence was not a slightly bent flight. Magnus acceleration
    /// is `C·(ω × v)`, so a spin three orders of magnitude too large
    /// produces a force three orders of magnitude too large, applied every
    /// tick and perpendicular to travel — the ball leaves at colossal
    /// speed in a direction nobody kicked it. That is the "ball suddenly
    /// shoots off for no reason" report, and because the force is a
    /// rotation of the velocity vector it also converts that speed into
    /// VERTICAL speed, which is how a pass ended up with a 700 m apex.
    ///
    /// Every producer must route through here.
    #[inline]
    pub fn clamped(spin: Vector3<f32>) -> Vector3<f32> {
        if !spin.x.is_finite() || !spin.y.is_finite() || !spin.z.is_finite() {
            return Vector3::zeros();
        }
        let mag = spin.norm();
        if mag > Self::MAX_SPIN {
            spin * (Self::MAX_SPIN / mag)
        } else {
            spin
        }
    }

    /// Magnus acceleration for a ball spinning at `spin` (rad/tick) and
    /// travelling at `velocity` (mixed units, as stored). Returns an
    /// acceleration in the same mixed units, ready to add to the velocity.
    pub fn magnus_accel(spin: Vector3<f32>, velocity: Vector3<f32>) -> Vector3<f32> {
        if spin.norm_squared() < Self::NEGLIGIBLE * Self::NEGLIGIBLE {
            return Vector3::zeros();
        }
        // Into metres.
        let v_m = Vector3::new(
            velocity.x * Self::M_PER_U,
            velocity.y * Self::M_PER_U,
            velocity.z,
        );
        let a_m = spin.cross(&v_m) * Self::MAGNUS_COEFF;
        // Back into the stored frame.
        Vector3::new(a_m.x * Self::U_PER_M, a_m.y * Self::U_PER_M, a_m.z)
    }

    /// Rotation imparted by a strike that met the ball off-centre.
    ///
    /// `direction` is the horizontal direction of travel. `side` is how
    /// far across the ball the foot came, and `vertical` how far under it,
    /// both roughly in ball-radii (±1 is a wild slice; ±0.3 is a normal
    /// well-struck curl). `power` scales the whole thing, because you
    /// cannot put revolutions on a ball you barely touched.
    ///
    /// Side contact spins the ball about the vertical axis, which bends
    /// the flight left or right. Contact under the ball spins it about the
    /// horizontal axis perpendicular to travel: backspin (`vertical > 0`)
    /// holds the ball up, topspin drags it down — the dip that an
    /// apex-solved parabola can never produce on its own.
    pub fn from_strike(
        direction: Vector3<f32>,
        side: f32,
        vertical: f32,
        power: f32,
    ) -> Vector3<f32> {
        let Some(dir) = Vector3::new(direction.x, direction.y, 0.0).try_normalize(1.0e-4) else {
            return Vector3::zeros();
        };
        // Rev/s a clean strike generates per unit of off-centre contact,
        // expressed straight in rad/tick: 0.55 is ~8.8 rev/s at a full
        // radius of offset, which is the whipped-free-kick end of the
        // real range.
        const SPIN_PER_OFFSET: f32 = 0.55;

        // Vertical axis — bends the flight sideways.
        let side_spin = Vector3::new(0.0, 0.0, side * SPIN_PER_OFFSET * power);

        // Horizontal axis perpendicular to travel. `ω × v` with this axis
        // has a NEGATIVE z-component for forward motion, so backspin
        // (which lifts) is the negated perpendicular.
        let perp = Vector3::new(-dir.y, dir.x, 0.0);
        let back_spin = -perp * (vertical * SPIN_PER_OFFSET * power);

        Self::clamped(side_spin + back_spin)
    }

    /// Decay applied once per airborne tick.
    pub(in crate::r#match::engine::ball::ball) fn decayed(spin: Vector3<f32>) -> Vector3<f32> {
        if spin.norm_squared() < Self::NEGLIGIBLE * Self::NEGLIGIBLE {
            Vector3::zeros()
        } else {
            spin * Self::DECAY_PER_TICK
        }
    }

    /// Horizontal velocity change when a spinning ball hits the turf, and
    /// the fraction of spin that survives the contact.
    ///
    /// A topspun ball grips and skids ON; a backspun one grips and checks
    /// back, sometimes stopping dead. This is the visible half of spin for
    /// anyone watching a ball bounce, and the old direction-preserving
    /// bounce (`v.x *= 0.95`) could express neither.
    pub(in crate::r#match::engine::ball::ball) fn bounce_kick(
        spin: Vector3<f32>,
        velocity: Vector3<f32>,
    ) -> (Vector3<f32>, f32) {
        let Some(dir) = Vector3::new(velocity.x, velocity.y, 0.0).try_normalize(1.0e-4) else {
            return (Vector3::zeros(), 0.35);
        };
        // Component of spin about the axis perpendicular to travel: the
        // sign convention from `from_strike` makes this negative for
        // backspin and positive for topspin.
        let perp = Vector3::new(-dir.y, dir.x, 0.0);
        let rolling = spin.dot(&perp);

        // Contact converts some rotation into translation.
        //
        // # This was 26 and it launched the ball into orbit
        //
        // The old value was justified as landing "a heavy backspin check
        // inside a metre or two of pace change" — a distance, where the
        // quantity being set is a SPEED. 26 u/tick per rad/tick means a
        // normally-struck ball (`from_strike` produces ~0.17 rad/tick for
        // a 0.3-radius contact) picked up 4.4 u/tick — 55 m/s — on its
        // first bounce, and the value is added AFTER `update_velocity`'s
        // speed clamp, so nothing downstream could take it back.
        //
        // Worse, it did not stay horizontal. The Magnus term is a
        // rotation of the velocity vector about the spin axis, so a ball
        // carrying a runaway horizontal speed has that speed steadily
        // turned into VERTICAL speed over a long flight. The two together
        // are what produced launches with a 640 m apex in the flight
        // census: nothing kicked the ball that hard, the bounce did, and
        // the spin then pointed it at the sky.
        //
        // Derived rather than fitted: a strongly backspun ball checks
        // roughly 30-40% off its own pace on contact. A ball arriving at
        // 15 m/s (1.2 u/tick) carrying ~0.55 rad/tick of backspin should
        // lose about 0.45 u/tick, which puts the coefficient near 0.8.
        const GRIP: f32 = 0.8;
        let along = dir * (rolling * GRIP);
        // Sidespin skews the bounce across the direction of travel.
        let across = perp * (spin.z * GRIP * 0.35);

        // Most of the rotation is scrubbed off on the first bounce.
        (along + across, 0.35)
    }
}

/// Where the man the ball is with stands and how he is moving — all the
/// carry follows.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Carrier {
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
}

/// How the ball reached the man who has it: who touched it, and its
/// velocity the moment he did — what a first-time strike is struck off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reception {
    pub by: u32,
    pub velocity: Vector3<f32>,
}

impl Carrier {
    pub fn of(player: &MatchPlayer) -> Self {
        Carrier {
            position: player.position,
            velocity: player.velocity,
        }
    }
}

impl Ball {
    /// Gain of the carry's pull toward the point the ball is carried at,
    /// per tick: first order, so the gap closes without overshooting.
    const CARRY_GAIN: f32 = 0.15;
    /// The most the pull moves a carried ball relative to its carrier on
    /// either axis across the grass: 0.1 u/tick, 1.25 m/s, a slow walk.
    pub(crate) const CARRY_BOUND: f32 = 0.1;
    /// …and a ball in a keeper's hands: 0.4 u/tick, 5 m/s, his arms.
    pub(crate) const HANDS_BOUND: f32 = 0.4;
    /// The bound on the vertical axis, m/tick: 10 m/s, as fast as a man
    /// snatches a ball into his chest and no faster.
    const CARRY_RATE: f32 = 0.10;

    pub fn update_velocity(&mut self) {
        const STOPPING_THRESHOLD: f32 = 0.05; // Lower threshold for smoother final stop
        use crate::r#match::engine::ball::ball::flight::ballistics::BOUNCE_COEFFICIENT;
        // Global ball velocity safety cap. Sits above every action-specific
        // cap (shot 3.2, pass 3.2, clearance 7.0) so it never clamps real
        // physics but still catches runaway bug velocities. Clearances are
        // the highest-magnitude legitimate action because they stack
        // meaningful horizontal AND vertical velocity for lofted hoofs.
        const MAX_VELOCITY: f32 = 8.0;
        // ...and the same for the VERTICAL axis, which the cap above
        // cannot police.
        //
        // `MAX_VELOCITY` is a norm over a vector whose axes carry
        // different units — `x`/`y` in game units, `z` in metres (see
        // `GRAVITY_PER_TICK`). 8.0 is a sane horizontal ceiling and a
        // meaningless vertical one: 8 m/tick is 800 m/s, so the safety
        // net let every mis-united kick through untouched. That is how a
        // hand-written `z` of 5.0 stayed in the engine as a 12.7 km apex
        // — nothing was in a position to notice.
        //
        // 40 m is comfortably above the highest ball in football (a
        // goalkeeper's kick from hand tops out near 30 m) so this never
        // binds on a legitimate strike, and it is a hard ceiling on how
        // wrong a future unit slip can go.
        const MAX_APEX_METRES: f32 = 40.0;
        let max_vertical =
            crate::r#match::engine::ball::ball::Ball::launch_speed_for_apex(MAX_APEX_METRES);

        // Air drag: affects aerial balls (proportional to v²). Lives on
        // the ball module beside gravity and rolling friction so the
        // trajectory SOLVERS can invert the same number the physics
        // applies — see `crate::r#match::engine::ball::ball::AIR_DRAG_PER_TICK`, which folds the
        // coefficient, the mass and the sub-step into one decay.
        const AIR_DRAG_PER_TICK: f32 = crate::r#match::engine::ball::ball::AIR_DRAG_PER_TICK;

        // Velocity-proportional rolling decay per 10ms tick. The value now
        // lives on the ball module so the physics and the pass-weighting
        // that inverts it cannot drift apart — see `GROUND_FRICTION` for
        // the derivation from the real 15%-per-second figure, and for why
        // the previous 0.006 forced the pass-overshoot hack.
        const GROUND_FRICTION_COEFFICIENT: f32 =
            crate::r#match::engine::ball::ball::GROUND_FRICTION;

        // CRITICAL: Global velocity sanity check - prevent cosmic-speed balls
        // Check for NaN or infinity and reset to zero
        if self.velocity.x.is_nan()
            || self.velocity.y.is_nan()
            || self.velocity.z.is_nan()
            || self.velocity.x.is_infinite()
            || self.velocity.y.is_infinite()
            || self.velocity.z.is_infinite()
        {
            self.velocity = Vector3::zeros();
            return;
        }

        // Clamp the vertical axis on its own terms, BEFORE the norm below
        // — trimming `z` first means the norm clamp is not asked to
        // absorb a launch that is wrong by three orders of magnitude, and
        // so cannot shrink a perfectly good horizontal component along
        // with it.
        if self.velocity.z > max_vertical {
            self.velocity.z = max_vertical;
        }

        // One unit for the cap and for the drag below — see
        // `Ball::flight_speed`.
        let mut velocity_norm = Ball::flight_speed(self.velocity);

        // Clamp velocity if it exceeds maximum
        if velocity_norm > MAX_VELOCITY {
            self.velocity *= MAX_VELOCITY / velocity_norm;
            velocity_norm = MAX_VELOCITY;
        }

        // GROUND COLLISION FIRST.
        //
        // This used to sit at the BOTTOM of the function, after the
        // ground/aerial split, which made it unreachable for any ball
        // moving at a normal speed: a descending ball reaches the deck
        // with `position.z` clamped to 0 by `apply_movement`, the "on
        // ground" branch then sees `velocity.z <= 0.0` and zeroes it, and
        // the bounce test below — which requires `velocity.z < 0.0` — can
        // no longer fire. Only a ball already slow enough to fall into the
        // near-stopped branch ever bounced. Everything else simply died
        // where it landed, which is why lofted balls read as hitting glue.
        //
        // Testing the collision before the roll/fly split restores it: a
        // ball arriving at the ground bounces, and only a ball that has
        // finished bouncing is treated as rolling.
        if self.position.z <= 0.0 && self.velocity.z < 0.0 {
            self.position.z = 0.0;
            self.velocity.z = -self.velocity.z * BOUNCE_COEFFICIENT;

            // Spin grips the turf: topspin skids the ball on, backspin
            // checks it back, sidespin skews it across. Computed BEFORE
            // the flat speed loss so it reads as a change of pace and
            // direction rather than a rescale of one.
            let incoming_horizontal =
                (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y).sqrt();
            let (kick, spin_retained) = SpinModel::bounce_kick(self.spin, self.velocity);
            self.velocity.x += kick.x;
            self.velocity.y += kick.y;
            self.spin *= spin_retained;

            // A bounce redirects momentum; it never creates it. Contact
            // friction can stop a backspun ball dead or skew a sidespun
            // one across its line, but it cannot send the ball off faster
            // than it arrived — and nothing else in the integrator is in
            // a position to say so, because this runs AFTER the speed
            // clamp above. Without this the spin term was free to add
            // pace on every bounce, and a ball with the rotation the
            // solved-spin sites used to produce simply accelerated off
            // the pitch.
            let after =
                (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y).sqrt();
            if after > incoming_horizontal && after > 1.0e-6 {
                let scale = incoming_horizontal / after;
                self.velocity.x *= scale;
                self.velocity.y *= scale;
            }

            // Horizontal speed loss on bounce.
            self.velocity.x *= 0.95;
            self.velocity.y *= 0.95;

            // If the bounce is too small, stop vertical movement. 0.012
            // m/tick = 1.2 m/s, which peaks at ~7 cm — below that the ball
            // is rolling, not bouncing. (Was 0.3, i.e. 30 m/s, from when
            // the vertical axis carried unit-scale speeds.)
            if self.velocity.z.abs() < 0.012 {
                self.velocity.z = 0.0;
            }
        }

        // **A BALL OFF THE GROUND IS NEVER "NEARLY STOPPED".**
        //
        // The settle branch at the bottom zeroes the velocity and puts the
        // ball on the deck, and it fired on total SPEED alone. A ball that
        // is airborne and slow is a ball that has yet to fall — one lying
        // against the goal netting, one at the top of a clipped lob — and
        // for those the branch did two wrong things at once: it cancelled
        // the velocity so gravity never reached it again, and it wrote
        // `position.z = 0`, teleporting it to the floor from wherever it
        // was. Measured off a real recording: a ball held under the roof
        // netting hung at 2.24 m for a third of a second and then dropped
        // 2.24 m in a single tick.
        //
        // Anything above the deck therefore takes the aerial branch however
        // slowly it is moving, and gravity brings it down at gravity's pace.
        let airborne = self.position.z > 0.1 || self.velocity.z > 0.0;
        if velocity_norm > STOPPING_THRESHOLD || airborne {
            // A ball that has just bounced is on its way UP, so it is not
            // rolling however low it is.
            let is_on_ground = self.position.z <= 0.1 && self.velocity.z <= 0.0;

            if is_on_ground {
                // GROUND PHYSICS: Rolling friction proportional to velocity (smooth deceleration)
                let horizontal_speed_sq =
                    self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y;

                if horizontal_speed_sq > STOPPING_THRESHOLD * STOPPING_THRESHOLD {
                    // Apply friction as a multiplier for smooth exponential decay
                    // friction_factor < 1.0 means the ball gradually slows down
                    let friction_factor = 1.0 - GROUND_FRICTION_COEFFICIENT;
                    self.velocity.x *= friction_factor;
                    self.velocity.y *= friction_factor;
                }

                // Keep ball on ground, but allow upward kicks to take effect
                // (positive z velocity means ball is being kicked into the air)
                if self.velocity.z <= 0.0 {
                    self.velocity.z = 0.0;
                    self.position.z = 0.0;
                }
            } else {
                // AERIAL PHYSICS: Air drag (proportional to v²) + gravity
                // + Magnus. Air drag is gentler than ground friction for
                // realistic flight.

                // Air drag: F = -C·|v|·v, integrated over the sub-step the
                // coefficient was fitted against. Gravity comes from the
                // shared per-tick constant so the pass solver and the
                // landing projection cannot drift away from the physics
                // (see `crate::r#match::engine::ball::ball::GRAVITY_PER_TICK`).
                if velocity_norm
                    > crate::r#match::engine::ball::ball::flight::ballistics::AIR_DRAG_FLOOR
                {
                    self.velocity -= (AIR_DRAG_PER_TICK * velocity_norm) * self.velocity;
                }
                self.velocity.z -= GRAVITY_PER_TICK;

                // The only force here that is not parallel to travel —
                // this is what curls a shot, whips a cross away from the
                // keeper, and makes a topspun ball dip.
                self.velocity += SpinModel::magnus_accel(self.spin, self.velocity);
                self.spin = SpinModel::decayed(self.spin);
            }
        } else {
            // Ball has nearly stopped ON THE GROUND — see `airborne` above —
            // so bring it to complete rest smoothly rather than instantly.
            self.velocity *= 0.8; // Smooth final decay

            // Only fully stop when truly negligible
            if self.velocity.norm_squared() < 0.0001 {
                // 0.01^2
                self.velocity = Vector3::zeros();
                // **…but a ball somebody is CARRYING is not lying on the
                // grass, and its height is not this branch's to write.**
                //
                // A ball carried by a man standing still has next to no
                // velocity, so it lands here, and the height it is
                // climbing towards (`carry_height`, 1.15 m in a keeper's
                // gloves) was flattened back to zero every time. The carry
                // climbs at 0.10 m a tick and the `airborne` test above
                // reads `z > 0.1`, so a gather off the deck settled into a
                // perfect two-cycle: 0 → 0.10 → 0 → 0.10, for the whole
                // two-to-five-second hold.
                //
                // On screen that is the keeper standing over the ball
                // rather than holding it, which is the artefact
                // `carry_height` was written to fix — reintroduced from
                // the other side. Seen in `dev_match gather`: z pinned at
                // 0.10 for 400 consecutive ticks of `HELD`.
                if self.current_owner.is_none() {
                    self.position.z = 0.0;
                }
                self.spin = Vector3::zeros();
            }
        }
    }

    /// **The ball is owned and still in the air.**
    ///
    /// Above [`Ball::DECK`] a ball he has taken belongs to gravity on the
    /// vertical axis; at it, to the carry. A ball in a keeper's GLOVES is
    /// excluded, and that is not an exception to the model — it is the
    /// model: a ball he has both hands on moves where his hands move, up
    /// or down, at the carry's bounded rate. That is what `carry_height`
    /// exists for.
    #[inline]
    pub(crate) fn settling_out_of_the_air(&self) -> bool {
        !self.held_in_hands && self.position.z > Self::DECK
    }

    /// **The touch** — the tick a man's foot, chest or head meets a ball he
    /// is taking.
    ///
    /// The one place a ball coming under control has its motion written:
    /// its pace across the grass drops to his, once, where it was met, and
    /// from then on it is carried ([`Self::carry`]). Nothing is moved — the
    /// ball is where he met it. A ball met above the deck stops climbing
    /// and loses its spin, and gravity brings it down to him. A heavy touch
    /// or a miscontrol is not this: it leaves the ball loose from the same
    /// spot (`apply_failed_first_touch`).
    pub(crate) fn touch(&mut self, owner: u32, carrier: Carrier) {
        self.reception = Some(Reception {
            by: owner,
            velocity: self.velocity,
        });
        self.controlled_by = Some(owner);
        self.velocity.x = carrier.velocity.x;
        self.velocity.y = carrier.velocity.y;
        self.velocity.z = self.velocity.z.min(0.0);
        self.spin = Vector3::zeros();
    }

    /// One tick of a ball he controls: it moves with him, plus a pull
    /// toward him — his feet, or the hands of a keeper holding it.
    ///
    /// The pull is toward his position, not a point led along his run: a
    /// ball carried ahead of him swung round his body on every turn and sat
    /// out of a tackler's reach, which cost two tackles a team a match. The
    /// viewer draws the ball at his boot.
    ///
    /// The motion is velocity and nothing else. The pull is first order
    /// and bounded per axis, so the gap closes at no more than a slow walk
    /// (a keeper's arms: 5 m/s) and never overshoots, and the ball is never
    /// written onto him: a ball coming from behind a man used to reverse
    /// onto his centre in one tick, which is the snap the replay showed.
    ///
    /// A ball he took above the deck is carried across the grass but falls
    /// under gravity alone, and when it reaches the deck
    /// [`finish_control`](Self::finish_control) asks whether he is still
    /// under it.
    fn carry(&mut self, carrier: Carrier) {
        let bound = if self.held_in_hands {
            Self::HANDS_BOUND
        } else {
            Self::CARRY_BOUND
        };
        let pull = |gap: f32| (gap * Self::CARRY_GAIN).clamp(-bound, bound);
        self.velocity.x = carrier.velocity.x + pull(carrier.position.x - self.position.x);
        self.velocity.y = carrier.velocity.y + pull(carrier.position.y - self.position.y);
        self.spin = Vector3::zeros();
        let falling = self.settling_out_of_the_air();
        if falling {
            self.take_control_of_the_flight();
        } else {
            self.velocity.z =
                (self.carry_height() - self.position.z).clamp(-Self::CARRY_RATE, Self::CARRY_RATE);
        }
        self.apply_movement();
        if falling {
            self.finish_control(carrier.position);
        }
    }

    /// **A ball taken out of the air comes down under gravity.** He has
    /// stopped it, not headed it on: it does not climb again. Across the
    /// grass it is carried; the fall is left to gravity alone, because a
    /// ball lowered faster than gravity — 10 m/s straight down from as high
    /// as 6.68 m, 77 times a match — was what the replay showed as a ball
    /// stopping dead in the sky over a man and dropping onto him.
    #[inline]
    fn take_control_of_the_flight(&mut self) {
        self.velocity.z = self.velocity.z.min(0.0);
        #[cfg(feature = "match-logs")]
        if !self.control_noted {
            self.control_noted = true;
            StrikeCensus::note_control(self.position.z);
        }
    }

    /// …and the landing. The ball has reached his feet: is he still
    /// under it?
    ///
    /// If he is not, it is a loose ball — the same shape as a failed
    /// first touch, which is exactly what a bad first touch is. Nothing
    /// is moved and nobody is punished; the ball simply stops being his
    /// and the ordinary claim scan decides who comes up with it.
    ///
    /// ⚠ **The threshold is [`Ball::DECK`], not zero, and it has to be.**
    /// It is the same number [`Self::settling_out_of_the_air`] hands the
    /// ball back to the owner tracking at, and when the two disagreed
    /// there was a band between them where neither ran: a fall that ended
    /// at 0.09 m left the control without ever being asked whether he was
    /// under it, and the tracking then found an owner two metres away and
    /// disowned him as too far. Measured at **7.8 `OWNER_TOO_FAR` a
    /// match against a baseline of 1.5** — the exact stranded-ball
    /// failure `CONTROL_DISTANCE` exists to prevent, reintroduced through
    /// a tenth of a metre.
    fn finish_control(&mut self, owner_position: Vector3<f32>) {
        if self.position.z > Self::DECK {
            return;
        }
        #[cfg(feature = "match-logs")]
        {
            self.control_noted = false;
        }
        let dx = owner_position.x - self.position.x;
        let dy = owner_position.y - self.position.y;
        if dx * dx + dy * dy
            <= crate::r#match::engine::ball::ball::CONTROL_DISTANCE
                * crate::r#match::engine::ball::ball::CONTROL_DISTANCE
        {
            return;
        }
        #[cfg(feature = "match-logs")]
        crate::r#match::engine::ball::ball::strike_diag::StrikeCensus::note_control_lost();
        self.previous_owner = self.current_owner;
        self.current_owner = None;
        self.ownership_duration = 0;
        // The same brief separation window `apply_failed_first_touch`
        // opens, so the ball is not re-claimed in place on the next tick
        // and the miss is visible.
        self.flags.in_flight_state = 12;
        self.claim_cooldown = 0;
    }

    /// Put the ball on the grass by giving it a descent, not by writing
    /// `position.z = 0.0`.
    ///
    /// The sibling of the carry's vertical step for a ball nobody has
    /// taken control of: a block off a shin, a mis-controlled
    /// first touch, a deflection. Those all end with the ball on the deck,
    /// and every one of them used to get there by assignment — from as
    /// high as 2.8 m, in a single 10 ms tick, on the axis a replay shows
    /// most plainly.
    ///
    /// ⚠ **`flight_diag` cannot see any of this.** Its `StageProbe`
    /// measures `sqrt(dx² + dy²)` and drops `z` entirely, so a two-metre
    /// vertical snap has always read as a relocation of zero. The
    /// whole-tick census
    /// ([`teleport`](crate::r#match::engine::ball::ball::teleport)) folds
    /// height in at 8 u/m and reports these in its `vertical` column.
    ///
    /// The rate is deliberately quicker than [`Self::CARRY_RATE`]: this is a ball
    /// knocked down, not gathered, and everything that reads the height —
    /// the block window, `is_aerial`, the receiver ceiling — must see what
    /// it always saw. An eighth of a second is under every one of their
    /// sampling windows and still draws as a fall.
    #[inline]
    pub(crate) fn sink_to_ground(&mut self) {
        /// Ticks a knocked-down ball takes to reach the grass.
        const SETTLE_TICKS: f32 = 12.0;
        self.velocity.z = -(self.position.z.max(0.0) / SETTLE_TICKS);
    }

    /// The ball has an owner. Under his touch or in his hands it is
    /// carried; a ball granted to him that his foot has just reached is
    /// touched; anything else keeps its own motion and is his to go and get
    /// (`should_force_takeball` sends him) — unless he is beyond any reach
    /// of it, when it is not his at all.
    fn follow_owner(&mut self, owner: u32, carrier: Carrier) {
        let dx = carrier.position.x - self.position.x;
        let dy = carrier.position.y - self.position.y;
        let gap_sq = dx * dx + dy * dy;
        // A ball in a keeper's GLOVES is never disowned for the distance:
        // a catch resolves wherever the save happened, up to 4 m from him,
        // and the hands bring it in. Disowning it would hand a caught shot
        // straight back to the six-yard box.
        if !self.held_in_hands && gap_sq > MAX_OWNER_TRACK_DISTANCE * MAX_OWNER_TRACK_DISTANCE {
            #[cfg(feature = "match-logs")]
            {
                reception_diag::OWNER_TOO_FAR.fetch_add(1, Ordering::Relaxed);
                reception_diag::too_far::note(
                    gap_sq.sqrt(),
                    self.held_in_hands,
                    self.cached_shot_target.is_some(),
                    self.velocity.norm(),
                );
            }
            self.previous_owner = self.current_owner;
            self.current_owner = None;
            self.controlled_by = None;
            self.ownership_duration = 0;
            self.apply_movement();
            return;
        }
        if !self.held_in_hands && self.controlled_by != Some(owner) {
            if gap_sq > LOOSE_CLAIM_DISTANCE * LOOSE_CLAIM_DISTANCE {
                self.apply_movement();
                return;
            }
            self.touch(owner, carrier);
        }
        self.carry(carrier);
    }

    pub(in crate::r#match::engine::ball::ball) fn move_to(
        &mut self,
        tick_context: &GameTickContext,
    ) {
        let has_owner = self.current_owner.is_some();
        #[cfg(feature = "match-logs")]
        if !has_owner {
            self.control_noted = false;
        }

        // Clear notifications when ball is no longer in a "take ball" scenario
        // Use a higher threshold to avoid clearing notifications set by try_notify_standing_ball
        // which uses is_ball_stopped_on_field (velocity < 2.5)
        const CLEAR_NOTIFICATION_VELOCITY: f32 = 3.0;
        let is_clearly_moving = self.velocity.norm() > CLEAR_NOTIFICATION_VELOCITY;
        if (is_clearly_moving || has_owner) && !self.take_ball_notified_players.is_empty() {
            self.take_ball_notified_players.clear();
        }

        if self.controlled_by != self.current_owner {
            self.controlled_by = None;
        }
        match self.current_owner {
            Some(owner) => {
                let (position, velocity) = tick_context.positions.players.pos_vel(owner);
                self.follow_owner(owner, Carrier { position, velocity });
            }
            None => self.apply_movement(),
        }
    }

    // `pub(crate)` rather than `pub(super)` so `keeper_save_contact_tests`
    // can drive the owner-tracking directly. The alternative is running a
    // whole `update_light`, which tests the tick orchestration rather than
    // how the ball gets into a keeper's hands. Same reason `check_goal` is.
    pub(crate) fn move_to_with_players(&mut self, players: &[MatchPlayer]) {
        #[cfg(feature = "match-logs")]
        if self.current_owner.is_none() {
            self.control_noted = false;
        }
        if self.controlled_by != self.current_owner {
            self.controlled_by = None;
        }
        match self
            .current_owner
            .and_then(|id| players.iter().find(|p| p.id == id))
        {
            Some(owner) => self.follow_owner(owner.id, Carrier::of(owner)),
            None => self.apply_movement(),
        }
    }

    pub(in crate::r#match::engine::ball::ball) fn check_boundary_collision(
        &mut self,
        context: &MatchContext,
    ) {
        // A ball in the goal is BEHIND the endline by design and the netting
        // owns it (see `net.rs`). Clamping it back onto the pitch here is
        // exactly the bug this whole path used to have — the ball reappeared
        // a metre in front of the goal the instant it went in.
        if self.in_net.is_some() {
            return;
        }

        // …and neither is a ball that has been put OUT OF PLAY. It is
        // outside the rectangle on purpose — that is the whole of the
        // reported *"the ball stops on the line behind the goal, but must
        // go beyond the goal"* — and its own bound is the run-off
        // perimeter, applied by `Ball::tick_run_out`. Clamping it here
        // would drag it 10 units back onto the pitch on the very tick it
        // crossed the line, which is the artefact in its purest form.
        //
        // Same stand-down as `in_net` directly above, and for the same
        // reason: the ball is somewhere this function was never told
        // about, and something else owns it there. See [`RunOff`](crate::r#match::engine::ball::ball::RunOff).
        if self.awaiting_restart.is_some() {
            return;
        }

        let field_width = context.field_size.width as f32;
        let field_height = context.field_size.height as f32;

        // Push ball well infield when it hits a boundary so players can reliably reach it.
        // 10m is generous enough that the Arrive steering and claim logic work smoothly,
        // while still keeping the ball in the corner/touchline area of the pitch.
        const BOUNDARY_INSET: f32 = 10.0;

        // Reaching here still past an endline and still between the posts
        // means every endline resolver declined the ball and the clamp
        // below is about to strand it in the goalmouth with no restart.
        // That must not happen — see `ENDLINE_CLAMPED_IN_MOUTH`.
        #[cfg(feature = "match-logs")]
        if self.current_owner.is_none()
            && (self.position.x <= 0.0 || self.position.x >= field_width)
        {
            let goal_center_y = if self.position.x <= 0.0 {
                context.goal_positions.left.y
            } else {
                context.goal_positions.right.y
            };
            if (self.position.y - goal_center_y).abs() <= crate::r#match::engine::goal::GOAL_WIDTH {
                crate::r#match::engine::ball::ball::ownership::reception_diag::ENDLINE_CLAMPED_IN_MOUTH
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        #[cfg(feature = "match-logs")]
        if self.position.x <= 0.0
            || self.position.x >= field_width
            || self.position.y <= 0.0
            || self.position.y >= field_height
        {
            crate::r#match::engine::ball::ball::frame_trace::FrameTrace::note(format!(
                "check_boundary_collision: CLAMP from ({:.1}, {:.1}, {:.2}) owner {:?}",
                self.position.x, self.position.y, self.position.z, self.current_owner
            ));
        }

        if self.position.x <= 0.0 {
            self.position.x = BOUNDARY_INSET;
            self.velocity = Vector3::zeros();
        } else if self.position.x >= field_width {
            self.position.x = field_width - BOUNDARY_INSET;
            self.velocity = Vector3::zeros();
        }

        if self.position.y <= 0.0 {
            self.position.y = BOUNDARY_INSET;
            self.velocity = Vector3::zeros();
        } else if self.position.y >= field_height {
            self.position.y = field_height - BOUNDARY_INSET;
            self.velocity = Vector3::zeros();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fly a ball to a given horizontal distance and report how far it
    /// drifted sideways, using the real integrator.
    fn lateral_drift_over(distance_units: f32, speed: f32, spin: Vector3<f32>) -> f32 {
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.position = Vector3::new(100.0, 272.0, 0.0);
        // Airborne and staying up for the whole run — this isolates the
        // Magnus term from the ground-friction branch.
        ball.position.z = 3.0;
        ball.velocity = Vector3::new(speed, 0.0, 0.0);
        ball.spin = spin;
        let start_y = ball.position.y;
        let target_x = ball.position.x + distance_units;
        for _ in 0..2000 {
            if ball.position.x >= target_x {
                break;
            }
            // Hold it up so the flight isn't cut short by gravity; we are
            // measuring the lateral axis only.
            ball.position.z = 3.0;
            ball.update_velocity();
            ball.velocity.z = 0.0;
            ball.apply_movement();
        }
        ball.position.y - start_y
    }

    #[test]
    fn a_ball_with_no_spin_flies_straight() {
        let drift = lateral_drift_over(200.0, 2.0, Vector3::zeros());
        assert!(
            drift.abs() < 0.01,
            "an unspun ball must not deviate laterally, drifted {drift}"
        );
    }

    #[test]
    fn sidespin_bends_the_flight_and_reverses_with_its_sign() {
        let left = lateral_drift_over(200.0, 2.0, Vector3::new(0.0, 0.0, 0.30));
        let right = lateral_drift_over(200.0, 2.0, Vector3::new(0.0, 0.0, -0.30));
        assert!(left > 1.0, "sidespin must bend the flight, got {left}");
        assert!(
            (left + right).abs() < 0.01,
            "the bend must be symmetric in the sign of the spin"
        );
    }

    /// The solver the shot and cross paths use runs backwards from the
    /// deflection they want. If the mixed axes (units horizontally,
    /// metres vertically) were mishandled anywhere in `magnus_accel`, the
    /// ball would arrive somewhere other than where it was aimed — which
    /// is silent, because the shot still goes somewhere plausible.
    #[test]
    fn solved_spin_delivers_the_deflection_it_was_solved_for() {
        for &(distance, speed, wanted) in &[
            (200.0f32, 2.0f32, 12.0f32),
            (200.0, 2.0, -8.0),
            (320.0, 1.4, 16.0),
            (120.0, 2.6, 5.0),
        ] {
            // ω_z = 2·d·|v| / (C·dist²) — the cross path's form.
            let spin_z = 2.0 * wanted * speed / (SpinModel::MAGNUS_COEFF * distance * distance);
            let drift = lateral_drift_over(distance, speed, Vector3::new(0.0, 0.0, spin_z));
            // Drag bleeds speed over the flight, so the realised bend runs
            // a little under the ideal-projectile solution. Within 25% is
            // the honest tolerance for a first-order solve.
            let error = (drift - wanted).abs() / wanted.abs();
            assert!(
                error < 0.25,
                "solved for {wanted}u of bend over {distance}u, realised {drift}u ({:.0}% off)",
                error * 100.0
            );
        }
    }

    #[test]
    fn spin_decays_while_airborne_and_dies_on_a_touch() {
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.position = Vector3::new(100.0, 272.0, 5.0);
        ball.velocity = Vector3::new(2.0, 0.0, 0.0);
        ball.spin = Vector3::new(0.0, 0.0, 0.5);
        for _ in 0..200 {
            ball.position.z = 5.0;
            ball.update_velocity();
        }
        assert!(ball.spin.z < 0.5, "spin must bleed off in flight");
        assert!(ball.spin.z > 0.4, "spin must not vanish over 2 seconds");

        ball.record_touch(7, 1, 100, true);
        assert_eq!(
            ball.spin,
            Vector3::zeros(),
            "a touch kills the rotation — the next kick decides the flight"
        );
    }

    /// The Magnus force is `C·(ω × v)`, so it scales with the rotation
    /// without limit. The cross and shot paths do not pick a spin — they
    /// invert `ω = 2·d·|v| / (C·dist²)` to get the one that delivers a
    /// wanted swing, and that expression runs away as the distance
    /// shrinks. Both used to assign it raw, which turned the ball into a
    /// projectile nobody had kicked.
    #[test]
    fn a_solved_spin_can_never_exceed_what_a_human_could_impart() {
        // The cross path's own expression, at a distance short enough to
        // make it explode: 5u of swing over 2u of travel.
        let absurd = 2.0 * 5.0 * 2.0 / (SpinModel::MAGNUS_COEFF * 2.0 * 2.0);
        assert!(
            absurd > 1000.0,
            "the solve really does run away; test is meaningless otherwise"
        );
        let clamped = SpinModel::clamped(Vector3::new(0.0, 0.0, absurd));
        assert!(
            clamped.norm() <= SpinModel::MAX_SPIN + 1.0e-4,
            "a solved spin must be clamped to something a player can produce, got {}",
            clamped.norm()
        );
        // The axis has to survive, or the ball bends the wrong way.
        assert!(clamped.z > 0.0, "clamping must preserve the direction");
        assert_eq!(
            SpinModel::clamped(Vector3::new(f32::NAN, 0.0, 0.0)),
            Vector3::zeros(),
            "a poisoned spin resolves to none, not to NaN in the physics"
        );
    }

    /// A bounce redirects momentum; it cannot manufacture it. The spin
    /// grip term is applied AFTER `update_velocity`'s speed clamp, so if
    /// it is free to add pace nothing downstream can take it back — which
    /// is how a spinning ball used to accelerate off the pitch.
    #[test]
    fn a_bounce_never_leaves_the_ball_faster_than_it_arrived() {
        for spin_z in [-1.2f32, -0.5, 0.0, 0.5, 1.2] {
            for rolling in [-1.2f32, 0.0, 1.2] {
                let mut ball = Ball::with_coord(840.0, 545.0);
                ball.position = Vector3::new(100.0, 272.0, 0.0);
                ball.velocity = Vector3::new(2.0, 0.0, -0.08);
                // Rotation about both axes at the engine's ceiling.
                ball.spin = Vector3::new(0.0, rolling, spin_z);
                let before =
                    (ball.velocity.x * ball.velocity.x + ball.velocity.y * ball.velocity.y).sqrt();
                ball.update_velocity();
                let after =
                    (ball.velocity.x * ball.velocity.x + ball.velocity.y * ball.velocity.y).sqrt();
                assert!(
                    after <= before + 1.0e-4,
                    "bounce with spin ({rolling}, {spin_z}) sped the ball up: {before} -> {after}"
                );
            }
        }
    }

    /// The vertical axis is in METRES while `x`/`y` are in game units, so
    /// the engine's `MAX_VELOCITY` norm cannot police it — 8.0 there means
    /// 800 m/s of climb. Every unit slip the flight census found was a
    /// hand-written `z` that this guard now catches.
    #[test]
    fn no_kick_can_climb_higher_than_a_football_ever_goes() {
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.position = Vector3::new(100.0, 272.0, 0.0);
        // The literal that sat in the goalkeeper's clearance: a 10 km apex.
        ball.velocity = Vector3::new(1.0, 0.0, 4.5);
        ball.update_velocity();
        let apex = Ball::apex_for_launch(ball.velocity.z);
        assert!(
            apex <= 41.0,
            "a launch must be clamped to a reachable apex, got {apex} m"
        );
        // A legitimate goalkeeper's kick from hand must pass untouched.
        // One tick of gravity and drag comes off it — that is the physics
        // doing its job, not the guard binding — so the bar is "still
        // essentially a 24 m kick" rather than an exact equality.
        ball.velocity = Vector3::new(1.0, 0.0, Ball::launch_speed_for_apex(24.0));
        ball.update_velocity();
        let realised = Ball::apex_for_launch(ball.velocity.z);
        assert!(
            realised > 23.0,
            "a real 24 m kick must not be clamped, got {realised} m"
        );
    }

    /// **A slow ball in the air must not be put on the floor.**
    ///
    /// The settle branch fired on total SPEED alone and wrote
    /// `position.z = 0`, so a ball that was airborne and barely moving — one
    /// lying against the goal netting, one at the top of a clipped lob — was
    /// teleported to ground level from wherever it was, and had its velocity
    /// zeroed so gravity never reached it again. Measured off a real
    /// recording: a ball held under the roof netting dropped 2.24 m in a
    /// single tick.
    #[test]
    fn a_slow_airborne_ball_falls_instead_of_being_dropped_to_the_deck() {
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.position = Vector3::new(100.0, 272.0, 2.2);
        // Below the settle branch's speed threshold on every axis.
        ball.velocity = Vector3::new(0.001, 0.0, 0.0);
        ball.update_velocity();
        assert!(
            ball.position.z > 2.0,
            "the ball was teleported from 2.2 m to {:.2} m",
            ball.position.z
        );
        assert!(
            ball.velocity.z < 0.0,
            "and it has to be falling, got vz {}",
            ball.velocity.z
        );
        // Over a second of it, the fall is gravity's and nothing else's.
        for _ in 0..100 {
            ball.update_velocity();
            ball.position += ball.velocity;
        }
        assert!(
            ball.position.z < 2.0,
            "a second later the ball is still at {:.2} m — it is not falling at all",
            ball.position.z
        );
    }

    /// A descending ball must actually bounce. The collision test used to
    /// run AFTER the roll/fly split, which zeroed the downward velocity
    /// first and left the branch unreachable for anything moving at a
    /// normal speed — lofted balls simply died where they landed.
    #[test]
    fn a_descending_ball_bounces_instead_of_dying_on_contact() {
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.position = Vector3::new(100.0, 272.0, 0.0);
        ball.velocity = Vector3::new(2.0, 0.0, -0.08);
        ball.update_velocity();
        assert!(
            ball.velocity.z > 0.0,
            "a ball arriving at the deck must come back up, got vz {}",
            ball.velocity.z
        );
    }

    /// Backspin checks a bouncing ball; topspin skids it on. The old
    /// bounce could express neither — it only ever scaled the horizontal
    /// velocity by 0.95.
    #[test]
    fn spin_changes_the_bounce_instead_of_just_scaling_it() {
        fn speed_after_bounce(spin_sign: f32) -> f32 {
            let mut ball = Ball::with_coord(840.0, 545.0);
            ball.position = Vector3::new(100.0, 272.0, 0.0);
            ball.velocity = Vector3::new(2.0, 0.0, -0.08);
            // Rotation about the axis perpendicular to travel: the sign
            // convention from `from_strike` makes negative = backspin.
            ball.spin =
                SpinModel::from_strike(Vector3::new(1.0, 0.0, 0.0), 0.0, spin_sign * 0.6, 1.0);
            ball.update_velocity();
            ball.velocity.x
        }
        let backspun = speed_after_bounce(1.0);
        let topspun = speed_after_bounce(-1.0);
        assert!(
            topspun > backspun,
            "topspin must skid on and backspin check back: top {topspun}, back {backspun}"
        );
    }

    /// **The touch writes the pace, never the place.** On the contact tick
    /// the ball's velocity across the grass becomes the man's, it stops
    /// climbing and spinning, and it is exactly where he met it.
    #[test]
    fn a_clean_touch_takes_his_pace_where_the_ball_is() {
        let man = Carrier {
            position: Vector3::new(400.0, 270.0, 0.0),
            velocity: Vector3::new(0.3, -0.2, 0.0),
        };
        for height in [0.0_f32, 1.7] {
            let mut ball = Ball::with_coord(840.0, 545.0);
            ball.position = Vector3::new(403.0, 268.0, height);
            ball.velocity = Vector3::new(-0.9, 0.1, 0.04);
            ball.spin = Vector3::new(0.0, 0.0, 0.3);
            let met = ball.position;
            ball.touch(7, man);
            assert_eq!(ball.position, met, "the touch moved the ball");
            assert_eq!(
                (ball.velocity.x, ball.velocity.y),
                (man.velocity.x, man.velocity.y),
                "the ball left the contact at {:?}, not at his pace",
                ball.velocity
            );
            assert!(ball.velocity.z <= 0.0, "a touched ball climbed on");
            assert_eq!(ball.spin, Vector3::zeros());
            assert_eq!(ball.controlled_by, Some(7));
        }
    }

    /// One tick of an owned ball and the man it belongs to: the ball as
    /// `follow_owner` moves it, the man by his own velocity. Asserts the
    /// ball moved by its velocity and nothing else, and returns its step
    /// relative to him.
    fn step_with(ball: &mut Ball, man: &mut Carrier) -> Vector3<f32> {
        let before = ball.position - man.position;
        ball.update_velocity();
        let from = ball.position;
        ball.follow_owner(7, *man);
        assert!(
            ((ball.position - from) - ball.velocity).norm() < 1e-3,
            "the ball was written to {:?}, not moved",
            ball.position
        );
        man.position += man.velocity;
        (ball.position - man.position) - before
    }

    fn within_the_carry_bound(relative: Vector3<f32>) -> bool {
        relative.x.abs() <= Ball::CARRY_BOUND + 1e-3 && relative.y.abs() <= Ball::CARRY_BOUND + 1e-3
    }

    /// **A received ball moves with the man who took it.** Met a stride
    /// ahead of a receiver running at 5 m/s, it leaves the contact at his
    /// pace, closes on his feet at no more than a slow walk, and is at his
    /// feet inside 30 ticks.
    #[test]
    fn a_received_ball_runs_with_him_and_settles_at_his_feet() {
        let mut man = Carrier {
            position: Vector3::new(300.0, 272.0, 0.0),
            velocity: Vector3::new(0.4, 0.0, 0.0),
        };
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.position = Vector3::new(304.5, 272.5, 0.0);
        ball.velocity = Vector3::new(-0.8, 0.0, 0.0);
        ball.current_owner = Some(7);
        ball.touch(7, man);
        let mut at_feet = None;
        for tick in 0..40 {
            let relative = step_with(&mut ball, &mut man);
            assert!(
                within_the_carry_bound(relative),
                "tick {tick}: the ball closed {relative:?} on him in one tick"
            );
            if at_feet.is_none() && Ball::at_feet(ball.position, man.position) {
                at_feet = Some(tick);
            }
        }
        let tick = at_feet.expect("the ball never reached his feet");
        assert!(tick < 30, "the ball took {tick} ticks to reach his feet");
        assert_eq!(ball.current_owner, Some(7));
    }

    /// **A turn carries the ball round; it is never written onto him.** A
    /// man running at 5 m/s who turns back the way he came takes the ball
    /// with him, never faster than a slow walk relative to his body and
    /// every tick by its own velocity, and ends with it at his feet.
    #[test]
    fn a_turn_carries_the_ball_round_without_writing_it_onto_him() {
        let mut man = Carrier {
            position: Vector3::new(300.0, 272.0, 0.0),
            velocity: Vector3::new(0.4, 0.0, 0.0),
        };
        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.position = man.position + Vector3::new(1.5, 0.0, 0.0);
        ball.current_owner = Some(7);
        ball.touch(7, man);
        for tick in 0..80 {
            if tick < 10 {
                let (sin, cos) = (std::f32::consts::PI / 10.0).sin_cos();
                let v = man.velocity;
                man.velocity = Vector3::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos, 0.0);
            }
            let relative = step_with(&mut ball, &mut man);
            assert!(
                within_the_carry_bound(relative),
                "tick {tick}: the ball jumped {relative:?} relative to him"
            );
        }
        assert!(Ball::at_feet(ball.position, man.position));
    }

    /// **A ball taken out of the air comes down at his feet.**
    ///
    /// Touched a stride from him off the deck, it loses its pace across
    /// the grass on the contact tick and the carry brings it in while
    /// gravity alone lowers it: moving for as long as it is up, never
    /// faster than it arrived, still his when it lands, and then at his
    /// feet.
    #[test]
    fn a_controlled_ball_travels_to_his_feet_instead_of_stopping_in_the_sky() {
        for (from, velocity) in [
            // Passing him at head height, a stride away.
            (
                Vector3::new(699.0, 183.0, 1.5),
                Vector3::new(-0.6, -0.4, -0.08),
            ),
            // Dropping steeply out of a punt, slower across the ground.
            (
                Vector3::new(698.0, 182.0, 2.6),
                Vector3::new(-0.2, -0.05, -0.02),
            ),
        ] {
            let mut man = Carrier {
                position: Vector3::new(696.0, 180.0, 0.0),
                velocity: Vector3::zeros(),
            };
            let mut ball = Ball::with_coord(840.0, 545.0);
            ball.position = from;
            ball.velocity = velocity;
            ball.current_owner = Some(7);
            let arrived_at = velocity.x.hypot(velocity.y);
            ball.touch(7, man);
            let mut ticks = 0;
            while ball.position.z > Ball::DECK {
                step_with(&mut ball, &mut man);
                let across = ball.velocity.x.hypot(ball.velocity.y);
                assert!(
                    across > 0.0,
                    "the ball stopped dead in the air at z={:.2}",
                    ball.position.z
                );
                assert!(
                    across <= arrived_at + 1e-4,
                    "a touch put pace ON the ball: {across:.3} > {arrived_at:.3}"
                );
                ticks += 1;
                assert!(ticks < 400, "the ball never came down");
            }
            assert_eq!(ball.current_owner, Some(7), "he was under it and lost it");
            for _ in 0..30 {
                step_with(&mut ball, &mut man);
            }
            assert!(
                Ball::at_feet(ball.position, man.position),
                "the ball settled {:.1}u from him",
                (ball.position - man.position).xy().norm()
            );
        }
    }
}
