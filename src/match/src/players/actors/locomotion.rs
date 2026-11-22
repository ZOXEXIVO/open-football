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
        {
            let mut walk = Walk {
                track,
                bounds,
                now,
                delta,
                ball,
                root,
                fallback_step,
                unwatched,
                reset,
                from,
                steps,
                elapsed: 0.0,
                phase_advance: 0.0,
            };
            for index in 0..steps {
                walk.tick(self, index);
            }
        }
        root.rotation = Quat::from_rotation_y(self.heading);
    }
}

struct Walk<'a> {
    track: Option<&'a Track>,
    bounds: (f64, f64),
    now: f64,
    delta: f32,
    ball: &'a BallState,
    root: &'a mut Transform,
    fallback_step: Vec3,
    unwatched: bool,
    reset: bool,
    from: f64,
    steps: usize,
    elapsed: f32,
    phase_advance: f32,
}

impl Walk<'_> {
    fn tick(&mut self, actor: &mut PlayerActor, index: usize) {
        let (dt, position, yaw) = self.drive(actor, index);
        self.turn(actor, index, dt, position, yaw);
    }

    fn drive(&mut self, actor: &mut PlayerActor, index: usize) -> (f32, Vec3, f32) {
        let dt = if index + 1 == self.steps {
            self.delta - self.elapsed
        } else {
            (1.0f32 / 120.0).min(self.delta - self.elapsed)
        };
        self.elapsed += dt;
        let at = if self.reset {
            self.now
        } else {
            self.from + f64::from(self.elapsed) * 1000.0
        };
        let position = self
            .track
            .and_then(|track| track.position_read(at))
            .map(|[x, y, z]| Field::to_world(x, y, z))
            .map(|p| Vec3::new(p.x, self.root.translation.y, p.z))
            .unwrap_or(self.root.translation);
        let gained = actor.gain_ground(position, self.reset && index == 0);
        let evidence = self
            .track
            .map(|track| track.motion_at(at, self.bounds.0, self.bounds.1));
        let velocity = if actor.at_attention.is_some() {
            Vec3::ZERO
        } else if let Some(cue) = evidence {
            if cue.placed || (!self.reset && gained.length_squared() < 1e-12) {
                Vec3::ZERO
            } else {
                cue.velocity
            }
        } else if self.reset {
            Vec3::ZERO
        } else {
            self.fallback_step / self.delta
        };
        let pace = 1.0 - (-dt / Actors::PACE_RESPONSE).exp();
        actor.gather_pace(velocity, pace, dt, self.reset && index == 0);
        if self.reset {
            actor.travel = velocity;
        } else {
            actor.travel +=
                (velocity - actor.travel) * (1.0 - (-dt / Actors::TRAVEL_RESPONSE).exp());
        }

        let wanted = Actors::facing(actor, self.ball, position, velocity * dt, self.unwatched);
        let mut yaw = 0.0;
        if let Some(direction) = Vec3::new(wanted.x, 0.0, wanted.z).try_normalize() {
            let target = direction.x.atan2(direction.z);
            let swing = (target - actor.heading + PI).rem_euclid(TAU) - PI;
            if self.reset && index == 0 {
                actor.heading += swing;
                actor.heading_rate = 0.0;
            } else {
                let before = actor.heading;
                Spring {
                    period: Actors::TURN_RESPONSE * TAU,
                    damping: 1.0,
                }
                .settle(
                    &mut actor.heading,
                    &mut actor.heading_rate,
                    before + swing,
                    dt,
                );
                let pace = (actor.speed / Actors::SPRINT).clamp(0.0, 1.0);
                let ceiling =
                    Actors::PIVOT_RATE.0 + (Actors::PIVOT_RATE.1 - Actors::PIVOT_RATE.0) * pace;
                yaw = (actor.heading - before).clamp(-ceiling * dt, ceiling * dt)
                    * (1.0 - actor.extended());
                actor.heading = before + yaw;
                actor.heading_rate =
                    actor.heading_rate.clamp(-ceiling, ceiling) * (1.0 - actor.extended());
            }
        }
        (dt, position, yaw)
    }

    fn turn(&mut self, actor: &mut PlayerActor, index: usize, dt: f32, position: Vec3, yaw: f32) {
        actor.bank_into(
            (yaw / dt / Actors::HARD_TURN).clamp(-1.0, 1.0),
            dt,
            self.reset && index == 0,
        );
        let forward = Vec3::new(actor.heading.sin(), 0.0, actor.heading.cos());
        let sideways = Vec3::new(actor.heading.cos(), 0.0, -actor.heading.sin());
        let course = actor
            .travel
            .try_normalize()
            .filter(|_| actor.speed > Actors::STEPPING * 0.5)
            .map_or(Vec2::Y, |way| {
                Vec2::new(way.dot(sideways), way.dot(forward))
            });
        if self.reset {
            actor.course = course;
        } else {
            actor.course += (course - actor.course) * (1.0 - (-dt / Actors::COURSE_RESPONSE).exp());
        }
        actor.course = actor.course.clamp_length_max(1.0);
        actor.open_legs(dt, self.reset && index == 0);
        let old_phase = actor.phase;
        let old_steps = actor.steps;
        actor.take_steps(dt, yaw, self.reset && index == 0);
        let advance = (actor.phase - old_phase + PI).rem_euclid(TAU) - PI;
        let limited = (self.phase_advance + advance).clamp(-FRAC_PI_2, FRAC_PI_2);
        if limited != self.phase_advance + advance {
            actor.phase = (old_phase + limited - self.phase_advance).rem_euclid(TAU);
            actor.steps = old_steps.wrapping_add(u32::from(
                (actor.phase / PI) as u32 != (old_phase / PI) as u32,
            ));
        }
        self.phase_advance = limited;
        actor.pose = actor.gait();
        actor.posture = Posture::of(&actor.pose);
        let frame = Transform::from_translation(position)
            .with_rotation(Quat::from_rotation_y(actor.heading));
        actor.plant_feet(&frame, dt, self.reset && index == 0);
    }
}
