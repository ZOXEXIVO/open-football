//! Shared match-time movement response. Root positions still belong to the replay.
use super::*;

impl PlayerActor {
    pub(super) fn reset_locomotion(&mut self, position: Vec3) {
        self.heading_rate = 0.0;
        self.speed = 0.0;
        self.tread = 0.0;
        self.tread_vec = Vec3::ZERO;
        self.travel = Vec3::ZERO;
        self.leash = Some(Vec3::new(position.x, 0.0, position.z));
        self.course = Vec2::Y;
        self.underfoot = Vec2::Y;
        self.open = 0.0;
        self.open_rate = 0.0;
        self.turn = 0.0;
        self.turn_rate = 0.0;
        self.drive = 0.0;
        self.drive_rate = 0.0;
        self.accelerating = 0.0;
        self.plants = [None; 2];
        self.pose.hold = [Hold::default(); 2];
        self.brace = 0.0;
        self.brace_rate = 0.0;
        self.pivot = 0.0;
        self.stance = 0.0;
        self.still = 0.0;
        self.phase = Complexion::carriage(self.id) * PI;
        self.steps = 0;
        self.jitter = Vec2::ZERO;
        self.strike = None;
        self.kick = None;
        self.flight = Vec3::ZERO;
        self.previous_height = self.height;
        self.vertical_speed = 0.0;
    }

    pub(super) fn advance_locomotion(
        &mut self,
        track: Option<&Track>,
        bounds: (f64, f64),
        now: f64,
        delta: f32,
        ball: &BallState,
        root: &mut Transform,
        fallback_step: Vec3,
        unwatched: bool,
        reset: bool,
    ) {
        // The ceremony supplies its own roots while replay tracks may already
        // be loaded. Its feet must be anchored to the line, not to kick-off.
        let track = track.filter(|_| self.at_attention.is_none());
        let steps = if reset {
            1
        } else {
            ((delta * 120.0 - 1e-4).ceil() as usize).clamp(1, 8)
        };
        let from = now - f64::from(delta) * 1000.0;
        let mut elapsed = 0.0f32;
        let mut phase_advance = 0.0f32;
        for index in 0..steps {
            // Keep normal steps small. The final, analytical response consumes
            // any remaining time after a long frame, without an unbounded loop.
            let dt = if index + 1 == steps {
                delta - elapsed
            } else {
                (1.0f32 / 120.0).min(delta - elapsed)
            };
            elapsed += dt;
            let at = if reset {
                now
            } else {
                from + f64::from(elapsed) * 1000.0
            };
            let position = track
                .and_then(|track| track.position_read(at))
                .map(|[x, y, z]| Field::to_world(x, y, z))
                .map(|p| Vec3::new(p.x, root.translation.y, p.z))
                .unwrap_or(root.translation);
            let gained = self.gain_ground(position, reset && index == 0);
            let evidence = track.map(|track| track.motion_at(at, bounds.0, bounds.1));
            let velocity = if self.at_attention.is_some() {
                Vec3::ZERO
            } else if let Some(cue) = evidence {
                if cue.placed || (!reset && gained.length_squared() < 1e-12) {
                    Vec3::ZERO
                } else {
                    cue.velocity
                }
            } else if reset {
                Vec3::ZERO
            } else {
                fallback_step / delta
            };
            let pace = 1.0 - (-dt / Actors::PACE_RESPONSE).exp();
            self.gather_pace(velocity, pace, dt, reset && index == 0);
            if reset {
                self.travel = velocity;
            } else {
                self.travel +=
                    (velocity - self.travel) * (1.0 - (-dt / Actors::TRAVEL_RESPONSE).exp());
            }

            let wanted = Actors::facing(self, ball, position, velocity * dt, unwatched);
            let mut yaw = 0.0;
            if let Some(direction) = Vec3::new(wanted.x, 0.0, wanted.z).try_normalize() {
                let target = direction.x.atan2(direction.z);
                let swing = (target - self.heading + PI).rem_euclid(TAU) - PI;
                if reset && index == 0 {
                    self.heading += swing;
                    self.heading_rate = 0.0;
                } else {
                    let before = self.heading;
                    Spring {
                        period: Actors::TURN_RESPONSE * TAU,
                        damping: 1.0,
                    }
                    .settle(
                        &mut self.heading,
                        &mut self.heading_rate,
                        before + swing,
                        dt,
                    );
                    let pace = (self.speed / Actors::SPRINT).clamp(0.0, 1.0);
                    let ceiling =
                        Actors::PIVOT_RATE.0 + (Actors::PIVOT_RATE.1 - Actors::PIVOT_RATE.0) * pace;
                    yaw = (self.heading - before).clamp(-ceiling * dt, ceiling * dt)
                        * (1.0 - self.extended());
                    self.heading = before + yaw;
                    self.heading_rate =
                        self.heading_rate.clamp(-ceiling, ceiling) * (1.0 - self.extended());
                }
            }
            self.bank_into(
                (yaw / dt / Actors::HARD_TURN).clamp(-1.0, 1.0),
                dt,
                reset && index == 0,
            );
            let forward = Vec3::new(self.heading.sin(), 0.0, self.heading.cos());
            let sideways = Vec3::new(self.heading.cos(), 0.0, -self.heading.sin());
            let course = self
                .travel
                .try_normalize()
                .filter(|_| self.speed > Actors::STEPPING * 0.5)
                .map_or(Vec2::Y, |way| {
                    Vec2::new(way.dot(sideways), way.dot(forward))
                });
            if reset {
                self.course = course;
            } else {
                self.course +=
                    (course - self.course) * (1.0 - (-dt / Actors::COURSE_RESPONSE).exp());
            }
            self.course = self.course.clamp_length_max(1.0);
            self.open_legs(dt, reset && index == 0);
            let old_phase = self.phase;
            let old_steps = self.steps;
            self.take_steps(dt, yaw, reset && index == 0);
            let advance = (self.phase - old_phase + PI).rem_euclid(TAU) - PI;
            let limited = (phase_advance + advance).clamp(-FRAC_PI_2, FRAC_PI_2);
            if limited != phase_advance + advance {
                self.phase = (old_phase + limited - phase_advance).rem_euclid(TAU);
                self.steps = old_steps.wrapping_add(u32::from(
                    (self.phase / PI) as u32 != (old_phase / PI) as u32,
                ));
            }
            phase_advance = limited;
            self.pose = self.gait();
            self.posture = Posture::of(&self.pose);
            let frame = Transform::from_translation(position)
                .with_rotation(Quat::from_rotation_y(self.heading));
            self.plant_feet(&frame, dt, reset && index == 0);
        }
        root.rotation = Quat::from_rotation_y(self.heading);
    }
}
