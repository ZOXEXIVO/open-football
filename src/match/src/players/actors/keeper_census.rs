//! **What the rig draws for a goalkeeper over a real recording**, on the two
//! axes every report about him has been on: whether his legs carry the
//! ground he covers, and what each flight peaks as.
//!
//! [`super::keeper::census_keeper`] measures the same man, but without the
//! save he is about to make on his feet — it never asks
//! [`Actors::next_arrival`] — and the save turned out to be where the legs
//! were lost. This walks the recording through the whole of the integrator
//! [`Actors::animate`] runs, the reach included, builds the real [`Gait`]
//! and reads the real forward kinematics off it.
//!
//! ```text
//! MATCH_REPLAY=<chunk.json> cargo test --lib keeper_census -- --ignored --nocapture
//! ```

use super::replayed::Chunk;
use super::*;
use crate::players::body::skeleton::{boot, crown};

/// Below this share of the ground he covers, his boots are not carrying him:
/// a planted foot alone travels back under a body at the body's own speed.
const SLIDING: f32 = 0.3;
/// A keeper covering ground faster than this is running, whatever else he
/// is doing — a walk is 1.5 m/s.
const RUNNING: f32 = 1.5;

#[derive(Default, Clone, Copy)]
struct Legwork {
    frames: u64,
    boots: f64,
    ground: f64,
    sliding: u64,
}

impl Legwork {
    fn note(&mut self, boots: f32, ground: f32) {
        self.frames += 1;
        self.boots += boots as f64;
        self.ground += ground as f64;
        if boots < SLIDING * ground {
            self.sliding += 1;
        }
    }

    fn report(&self, what: &str) {
        if self.frames == 0 {
            println!("  {what:<22} no frames");
            return;
        }
        println!(
            "  {what:<22} {:6} frames   boots/ground {:.2}   sliding {:5.1}%",
            self.frames,
            self.boots / self.ground.max(1e-6),
            self.sliding as f64 * 100.0 / self.frames as f64
        );
    }
}

#[test]
#[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
fn keeper_glide() {
    let Some(mut tracks) = Chunk::open() else {
        panic!("set MATCH_REPLAY to a decompressed chunk");
    };
    let (start, until) = tracks.ball.span().expect("a recorded chunk");
    let ids = Chunk::keepers(&mut tracks, start);
    let frame = 1.0f32 / 60.0;
    let frames = ((until - start) / (frame as f64 * 1000.0)) as u32;
    let pace = 1.0 - (-frame / Actors::PACE_RESPONSE).exp();
    let turn = 1.0 - (-frame / Actors::TURN_RESPONSE).exp();

    for id in ids {
        let mut actor = PlayerActor::new(id, true, true);
        let mut previous: Option<Vec3> = None;
        let mut boots: Option<(Vec3, Vec3)> = None;
        // [running with a save live, running while getting up, plain running]
        let mut work = [Legwork::default(); 3];
        // The lean into an acceleration, against the acceleration itself:
        // [with it, against it, sideways of it] over frames he is driving
        // off or pulling up in. Isolated from every other lean the trunk
        // carries by posing the same gait with the drive zeroed.
        let mut lean = [0u64; 3];
        // (time, state at take-off, apex height, jump at apex, tilt at apex,
        // ground covered in the air)
        let mut flights: Vec<(f64, String, f32, f32, f32, f32)> = Vec::new();
        let mut in_air: Option<(f64, String, f32, f32, f32, Vec3)> = None;

        for f in 0..frames {
            let now = start + f as f64 * frame as f64 * 1000.0;
            let Some(p) = tracks.players.get_mut(&id).and_then(|t| t.position_at(now)) else {
                previous = None;
                boots = None;
                continue;
            };
            let position = Field::to_world(p[0], p[1], p[2]);
            let ball = tracks
                .ball
                .position_at(now)
                .map(|b| Field::to_world(b[0], b[1], b[2]));
            let step = match previous {
                Some(prev) => Vec3::new(position.x - prev.x, 0.0, position.z - prev.z),
                None => Vec3::ZERO,
            };
            previous = Some(position);
            if f == 0 {
                continue;
            }

            // ——— the integrator, as `animate` runs it ———
            let ground = step.length();
            let observed = ground / frame;
            let (ground, observed) = if observed > Actors::TELEPORT {
                (0.0, actor.speed)
            } else {
                (ground, observed)
            };
            let urge = ((observed - actor.speed) / Actors::PACE_RESPONSE / Actors::DRIVING)
                .clamp(-1.0, 1.0);
            actor.drive += (urge - actor.drive) * (1.0 - (-frame / Actors::DRIVE_RESPONSE).exp());
            actor.speed += (observed - actor.speed) * pace;
            let travelling = if ground <= 0.0 {
                Vec3::ZERO
            } else {
                step / frame
            };
            let was = actor.travel;
            actor.travel =
                was + (travelling - was) * (1.0 - (-frame / Actors::TRAVEL_RESPONSE).exp());
            actor.height = position.y;
            actor.declared =
                Actors::declared(&mut tracks.states, id, now, position.y, actor.declared);
            let launch = actor.speed.max(observed);
            let was_airborne = actor.air > 0.0;
            let airborne = actor.track_flight(frame, launch, observed, false);
            if airborne {
                if !was_airborne {
                    actor.flight = Vec3::ZERO;
                }
                actor.flight += step;
                let forward = Vec3::new(actor.heading.sin(), 0.0, actor.heading.cos());
                let right = Vec3::new(actor.heading.cos(), 0.0, -actor.heading.sin());
                actor.tip = match actor.flight.try_normalize() {
                    Some(going) => Vec2::new(going.dot(right), going.dot(forward)) * actor.flat,
                    None => Vec2::ZERO,
                };
            }

            let mut state = BallState::default();
            if let Some(b) = ball {
                state.on_pitch = true;
                state.position = b;
                let range = Vec3::new(b.x - position.x, 0.0, b.z - position.z).length();
                state.nearest = Some((id, range));
            }
            let facing = Actors::facing(&actor, &state, position, step, false);
            let mut turn_signal = 0.0f32;
            if let Some(facing) = Vec3::new(facing.x, 0.0, facing.z).try_normalize() {
                let wanted = facing.x.atan2(facing.z);
                let swing = (wanted - actor.heading + PI).rem_euclid(TAU) - PI;
                let eased = (actor.speed / Actors::SPRINT).clamp(0.0, 1.0);
                let ceiling = (Actors::PIVOT_RATE.0
                    + (Actors::PIVOT_RATE.1 - Actors::PIVOT_RATE.0) * eased)
                    * frame;
                let applied = (swing * turn).clamp(-ceiling, ceiling);
                actor.heading += applied;
                turn_signal = (applied / frame / Actors::HARD_TURN).clamp(-1.0, 1.0);
            }
            actor.turn += (turn_signal - actor.turn) * pace;

            let forward = Vec3::new(actor.heading.sin(), 0.0, actor.heading.cos());
            let sideways = Vec3::new(actor.heading.cos(), 0.0, -actor.heading.sin());
            let wanted_course = match actor.travel.try_normalize() {
                Some(way) if actor.speed > Actors::STEPPING * 0.5 => {
                    Vec2::new(way.dot(sideways), way.dot(forward))
                }
                _ => Vec2::Y,
            };
            let settle = 1.0 - (-frame / Actors::COURSE_RESPONSE).exp();
            let was = actor.course;
            actor.course = (was + (wanted_course - was) * settle).clamp_length_max(1.0);
            actor.open = Actors::opening(actor.speed, actor.course, true);
            actor.underfoot = Actors::underfoot(actor.course, actor.open);
            actor.idle = (actor.idle + frame * Actors::IDLE_RATE).rem_euclid(TAU);
            actor.clock = (now * 1e-3) as f32;

            let wanted_set = match ball {
                Some(b) => {
                    Actors::nearing(Vec3::new(b.x - position.x, 0.0, b.z - position.z).length())
                }
                None => 0.0,
            };
            actor.set += (wanted_set - actor.set) * pace;

            // The save he is about to make on his feet, exactly as the
            // renderer reads it ahead of the playhead.
            actor.arrival = Actors::next_arrival(&mut tracks.ball, now, position);
            let wanted_reaction = actor.arrival.map_or(0.0, |save| {
                Actors::ease(1.0 - save.delay / Actors::SAVE_ONSET)
            });
            if wanted_reaction > actor.reaction {
                actor.reaction = wanted_reaction;
            } else {
                actor.reaction -= actor.reaction * (1.0 - (-frame / Actors::SAVE_RELEASE).exp());
            }
            if let Some(save) = actor.arrival {
                let to_ball = save.at - position;
                let across =
                    Vec3::new(to_ball.x, 0.0, to_ball.z).dot(sideways) / Actors::SAVE_REACH;
                let up = (save.at.y - Actors::SAVE_GATHER)
                    / (Actors::SAVE_OVERHEAD - Actors::SAVE_GATHER)
                    * 2.0
                    - 1.0;
                let wanted_aim = Vec2::new(across.clamp(-1.0, 1.0), up.clamp(-1.0, 1.0));
                actor.aim += (wanted_aim - actor.aim) * pace;
                actor.parry += (f32::from(!save.held) - actor.parry) * pace;
            }
            // …and the ball in his gloves, off the same rule the renderer uses.
            let wanted_carry = ball.is_some_and(|b| {
                let reach = Vec2::new(b.x - position.x, b.z - position.z).length();
                Actors::in_his_hands(reach, b.y, true)
            });
            actor.carry += (f32::from(wanted_carry) - actor.carry) * pace;

            let (stride, carry_ground) = Actors::stride_of(id, actor.speed, actor.underfoot);
            actor.phase = (actor.phase + ground * PI / stride).rem_euclid(TAU);
            actor.carry_ground = carry_ground;
            let gait = actor.gait();

            // ——— what he is drawn doing with his legs ———
            let (left, right) = (boot(-1.0, gait), boot(1.0, gait));
            if let Some((was_left, was_right)) = boots {
                if !airborne && actor.speed > RUNNING && ground > 1e-4 {
                    let travelled =
                        |now: Vec3, then: Vec3| Vec2::new(now.x - then.x, now.z - then.z).length();
                    let legs = travelled(left, was_left) + travelled(right, was_right);
                    let band = if gait.save > 0.3 {
                        0
                    } else if actor.dive > 0.3 {
                        1
                    } else {
                        2
                    };
                    work[band].note(legs, ground);
                }
            }
            boots = Some((left, right));

            // ——— which way the trunk leans into a change of pace ———
            if !airborne && gait.drive.abs() > 0.4 && actor.speed > 0.7 && gait.save < 0.1 {
                let mut coasting = gait;
                coasting.drive = 0.0;
                let tilt = crown(gait) - crown(coasting);
                let world = sideways * tilt.x + forward * tilt.z;
                if let (Some(tilt), Some(going)) = (
                    Vec3::new(world.x, 0.0, world.z).try_normalize(),
                    actor.travel.try_normalize(),
                ) {
                    let along = tilt.dot(going) * gait.drive.signum();
                    lean[if along > 0.5 {
                        0
                    } else if along < -0.5 {
                        1
                    } else {
                        2
                    }] += 1;
                }
            }

            // ——— and what each flight peaks as ———
            if airborne {
                let (pitch, roll) = actor.topple();
                let tilt = Carriage::tilt(pitch, roll)
                    .clamp(0.0, 1.0)
                    .asin()
                    .to_degrees();
                let name = tracks
                    .states
                    .get_mut(&id)
                    .and_then(|t| t.name_at(now))
                    .unwrap_or("?")
                    .to_string();
                match &mut in_air {
                    None => in_air = Some((now, name, position.y, gait.jump, tilt, position)),
                    Some(flight) => {
                        if position.y > flight.2 {
                            flight.2 = position.y;
                            flight.3 = gait.jump;
                            flight.4 = tilt;
                        }
                    }
                }
            } else if let Some(flight) = in_air.take() {
                let covered = Vec2::new(position.x - flight.5.x, position.z - flight.5.z).length();
                flights.push((flight.0, flight.1, flight.2, flight.3, flight.4, covered));
            }
        }

        println!("KEEPER {id}: running (> {RUNNING} m/s, on his feet)");
        work[0].report("with a save live");
        work[1].report("while getting up");
        work[2].report("plain running");
        let driving = lean.iter().sum::<u64>().max(1) as f64;
        println!(
            "  lean into a change of pace: with it {:.0}%, AGAINST it {:.0}%, across it {:.0}% ({} frames)",
            lean[0] as f64 * 100.0 / driving,
            lean[1] as f64 * 100.0 / driving,
            lean[2] as f64 * 100.0 / driving,
            lean.iter().sum::<u64>()
        );
        println!("  flights: {}", flights.len());
        println!(
            "    {:>8} {:<20} {:>6} {:>6} {:>6} {:>7}",
            "t(s)", "state", "apex", "jump", "tilt°", "ground"
        );
        for (t, name, apex, jump, tilt, covered) in &flights {
            println!(
                "    {:8.1} {:<20} {:6.2} {:6.2} {:6.0} {:7.2}",
                t / 1000.0,
                name,
                apex,
                jump,
                tilt,
                covered
            );
        }
    }
}
