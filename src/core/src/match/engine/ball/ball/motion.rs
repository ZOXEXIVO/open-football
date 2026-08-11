//! Ball physics and movement: velocity integration with friction /
//! drag / gravity / spin / bounce, owner tracking that lets the ball
//! follow a possessing player smoothly, and the boundary-collision inset
//! that pushes the ball back inside the field after it crosses a
//! touchline.

use super::{Ball, GRAVITY_PER_TICK};
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer};
use nalgebra::Vector3;

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
    const MAX_SPIN: f32 = 1.2;

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

        let total = side_spin + back_spin;
        let mag = total.norm();
        if mag > Self::MAX_SPIN {
            total * (Self::MAX_SPIN / mag)
        } else {
            total
        }
    }

    /// Decay applied once per airborne tick.
    fn decayed(spin: Vector3<f32>) -> Vector3<f32> {
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
    fn bounce_kick(spin: Vector3<f32>, velocity: Vector3<f32>) -> (Vector3<f32>, f32) {
        let Some(dir) = Vector3::new(velocity.x, velocity.y, 0.0).try_normalize(1.0e-4) else {
            return (Vector3::zeros(), 0.35);
        };
        // Component of spin about the axis perpendicular to travel: the
        // sign convention from `from_strike` makes this negative for
        // backspin and positive for topspin.
        let perp = Vector3::new(-dir.y, dir.x, 0.0);
        let rolling = spin.dot(&perp);

        // Contact converts some rotation into translation. 26 u/tick per
        // rad/tick lands a heavy backspin check inside a metre or two of
        // pace change, which is what one looks like.
        const GRIP: f32 = 26.0;
        let along = dir * (rolling * GRIP);
        // Sidespin skews the bounce across the direction of travel.
        let across = perp * (spin.z * GRIP * 0.35);

        // Most of the rotation is scrubbed off on the first bounce.
        (along + across, 0.35)
    }
}

impl Ball {
    pub fn update_velocity(&mut self) {
        const BALL_MASS: f32 = 0.43;
        const STOPPING_THRESHOLD: f32 = 0.05; // Lower threshold for smoother final stop
        // Football bounce retention on grass is ~25-35%. The previous
        // 0.6 produced trampoline bounces where a lofted clearance
        // bounced to 30m+ and stayed airborne (above PLAYER_JUMP_REACH)
        // for 3-5 cycles before a defender could claim. 0.3 keeps the
        // second bounce low enough to reach on the return trip.
        const BOUNCE_COEFFICIENT: f32 = 0.3;
        // Global ball velocity safety cap. Sits above every action-specific
        // cap (shot 3.2, pass 3.2, clearance 7.0) so it never clamps real
        // physics but still catches runaway bug velocities. Clearances are
        // the highest-magnitude legitimate action because they stack
        // meaningful horizontal AND vertical velocity for lofted hoofs.
        const MAX_VELOCITY: f32 = 8.0;

        // Physics constants for realistic ball behavior
        // Air drag: affects aerial balls (proportional to v²)
        const AIR_DRAG_COEFFICIENT: f32 = 0.04; // Reduced for more realistic air resistance

        // Velocity-proportional rolling decay per 10ms tick. The value now
        // lives on the ball module so the physics and the pass-weighting
        // that inverts it cannot drift apart — see `GROUND_FRICTION` for
        // the derivation from the real 15%-per-second figure, and for why
        // the previous 0.006 forced the pass-overshoot hack.
        const GROUND_FRICTION_COEFFICIENT: f32 = super::GROUND_FRICTION;

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

        let mut velocity_norm_sq = self.velocity.norm_squared();

        // Clamp velocity if it exceeds maximum
        if velocity_norm_sq > MAX_VELOCITY * MAX_VELOCITY {
            let velocity_norm = velocity_norm_sq.sqrt();
            self.velocity = self.velocity * (MAX_VELOCITY / velocity_norm);
            velocity_norm_sq = MAX_VELOCITY * MAX_VELOCITY;
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
            let (kick, spin_retained) = SpinModel::bounce_kick(self.spin, self.velocity);
            self.velocity.x += kick.x;
            self.velocity.y += kick.y;
            self.spin *= spin_retained;

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

        if velocity_norm_sq > STOPPING_THRESHOLD * STOPPING_THRESHOLD {
            let velocity_norm = velocity_norm_sq.sqrt();
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

                // Air drag force: F = -0.5 * C * v² * direction
                let air_drag_force = if velocity_norm > 0.1 {
                    -AIR_DRAG_COEFFICIENT * velocity_norm * self.velocity
                } else {
                    Vector3::zeros()
                };

                // Drag is integrated over the same sub-step the drag
                // coefficient was fitted against; gravity comes from the
                // shared per-tick constant so the pass solver and the
                // landing projection cannot drift away from the physics
                // (see `super::GRAVITY_PER_TICK`).
                self.velocity += (air_drag_force / BALL_MASS) * 0.016;
                self.velocity.z -= GRAVITY_PER_TICK;

                // The only force here that is not parallel to travel —
                // this is what curls a shot, whips a cross away from the
                // keeper, and makes a topspun ball dip.
                self.velocity += SpinModel::magnus_accel(self.spin, self.velocity);
                self.spin = SpinModel::decayed(self.spin);
            }
        } else {
            // Ball has nearly stopped - bring to complete rest smoothly
            // Use gradual decay instead of instant stop
            self.velocity = self.velocity * 0.8; // Smooth final decay

            // Only fully stop when truly negligible
            if self.velocity.norm_squared() < 0.0001 {
                // 0.01^2
                self.velocity = Vector3::zeros();
                self.position.z = 0.0;
                self.spin = Vector3::zeros();
            }
        }
    }

    pub(super) fn move_to(&mut self, tick_context: &GameTickContext) {
        // Clear notified players only when ball state changes significantly:
        // 1. Ball starts moving (not stopped anymore)
        // 2. Ball has an owner (claimed)
        // Maximum distance owner can be from ball - must match deadlock claim distances
        // This allows deadlock resolution while preventing truly absurd teleports
        const MAX_OWNER_TELEPORT_DISTANCE: f32 = super::MAX_OWNER_TRACK_DISTANCE;
        const MAX_OWNER_TELEPORT_DISTANCE_SQUARED: f32 =
            MAX_OWNER_TELEPORT_DISTANCE * MAX_OWNER_TELEPORT_DISTANCE;

        // Ball moves toward owner at this speed (units/tick) instead of teleporting
        const BALL_TRACK_SPEED: f32 = 1.5;
        // Snap to owner if within this distance (avoids jitter)
        const SNAP_DISTANCE: f32 = 2.0;
        const SNAP_DISTANCE_SQUARED: f32 = SNAP_DISTANCE * SNAP_DISTANCE;

        let has_owner = self.current_owner.is_some();

        // Clear notifications when ball is no longer in a "take ball" scenario
        // Use a higher threshold to avoid clearing notifications set by try_notify_standing_ball
        // which uses is_ball_stopped_on_field (velocity < 2.5)
        const CLEAR_NOTIFICATION_VELOCITY: f32 = 3.0;
        let is_clearly_moving = self.velocity.norm() > CLEAR_NOTIFICATION_VELOCITY;
        if (is_clearly_moving || has_owner) && !self.take_ball_notified_players.is_empty() {
            self.take_ball_notified_players.clear();
        }

        if let Some(owner_player_id) = self.current_owner {
            let owner_position = tick_context.positions.players.position(owner_player_id);

            let dx = owner_position.x - self.position.x;
            let dy = owner_position.y - self.position.y;
            let distance_squared = dx * dx + dy * dy;

            if distance_squared <= MAX_OWNER_TELEPORT_DISTANCE_SQUARED {
                if distance_squared <= SNAP_DISTANCE_SQUARED {
                    // Close enough - snap to owner, at whatever height he is
                    // carrying it: on the deck at his feet, or into his chest
                    // if he is a keeper with it in his gloves.
                    let carry = self.carry_height();
                    self.position = owner_position;
                    self.position.z = carry;
                    self.velocity = Vector3::zeros();
                    self.spin = Vector3::zeros();
                } else {
                    // Move ball toward owner smoothly instead of teleporting
                    let distance = distance_squared.sqrt();
                    let dir_x = dx / distance;
                    let dir_y = dy / distance;
                    let carry = self.carry_height();
                    self.position.x += dir_x * BALL_TRACK_SPEED;
                    self.position.y += dir_y * BALL_TRACK_SPEED;
                    self.position.z = carry;
                    self.velocity = Vector3::zeros();
                    self.spin = Vector3::zeros();
                }
            } else {
                // Owner is too far - this shouldn't happen but is a safety net
                // Clear ownership and let ball move naturally
                #[cfg(feature = "match-logs")]
                super::ownership::reception_diag::OWNER_TOO_FAR
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.previous_owner = self.current_owner;
                self.current_owner = None;
                self.ownership_duration = 0;

                // Move ball normally
                self.position.x += self.velocity.x;
                self.position.y += self.velocity.y;
                self.position.z += self.velocity.z;

                if self.position.z < 0.0 {
                    self.position.z = 0.0;
                }
            }
        } else {
            self.position.x += self.velocity.x;
            self.position.y += self.velocity.y;
            self.position.z += self.velocity.z;

            // Ensure ball doesn't go below ground
            if self.position.z < 0.0 {
                self.position.z = 0.0;
            }
        }
    }

    pub(super) fn move_to_with_players(&mut self, players: &[MatchPlayer]) {
        const MAX_OWNER_TELEPORT_DISTANCE_SQUARED: f32 =
            super::MAX_OWNER_TRACK_DISTANCE * super::MAX_OWNER_TRACK_DISTANCE;
        const BALL_TRACK_SPEED: f32 = 1.5;
        const SNAP_DISTANCE_SQUARED: f32 = 2.0 * 2.0;

        if let Some(owner_id) = self.current_owner {
            if let Some(owner) = players.iter().find(|p| p.id == owner_id) {
                let dx = owner.position.x - self.position.x;
                let dy = owner.position.y - self.position.y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= MAX_OWNER_TELEPORT_DISTANCE_SQUARED {
                    if dist_sq <= SNAP_DISTANCE_SQUARED {
                        let carry = self.carry_height();
                        self.position = owner.position;
                        self.position.z = carry;
                        self.velocity = Vector3::zeros();
                    } else {
                        let carry = self.carry_height();
                        let dist = dist_sq.sqrt();
                        self.position.x += (dx / dist) * BALL_TRACK_SPEED;
                        self.position.y += (dy / dist) * BALL_TRACK_SPEED;
                        self.position.z = carry;
                        self.velocity = Vector3::zeros();
                    }
                } else {
                    #[cfg(feature = "match-logs")]
                    super::ownership::reception_diag::OWNER_TOO_FAR
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    self.previous_owner = self.current_owner;
                    self.current_owner = None;
                    self.ownership_duration = 0;
                    self.apply_movement();
                }
            } else {
                self.apply_movement();
            }
        } else {
            self.apply_movement();
        }
    }

    pub(super) fn check_boundary_collision(&mut self, context: &MatchContext) {
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
                super::ownership::reception_diag::ENDLINE_CLAMPED_IN_MOUTH
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
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
}
