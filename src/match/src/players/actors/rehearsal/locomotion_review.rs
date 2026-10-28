//! Deterministic tracks passed through the production actor and rig systems.
use super::*;
use crate::recording::replay::{Sample, Track};

fn fixture() -> Rehearsal {
    let mut tracks = ReplayTracks::default();
    tracks.ball.merge(vec![
        Sample {
            t: 0,
            x: 450.0,
            y: 272.5,
            z: 0.0,
        },
        Sample {
            t: 12_000,
            x: 450.0,
            y: 272.5,
            z: 0.0,
        },
    ]);
    for id in 1..=5 {
        let mut track = Track::default();
        track.merge(
            (0..=400)
                .map(|i| {
                    let t = i as f32 * 0.03;
                    let (x, z) = match id {
                        // A quantized straight jog, then a stop.
                        1 => (3.0 * t.min(8.0), 0.0),
                        // Sub-leash position noise, with no net travel.
                        2 => (0.0125 * (i % 3) as f32, 0.0125 * (i % 2) as f32),
                        // Accelerate, carry speed, brake, and rest.
                        3 => {
                            let x = if t < 2.0 {
                                1.5 * t * t
                            } else if t < 5.0 {
                                6.0 + 6.0 * (t - 2.0)
                            } else if t < 7.0 {
                                24.0 + 6.0 * (t - 5.0) - 1.5 * (t - 5.0).powi(2)
                            } else {
                                30.0
                            };
                            (x, 0.0)
                        }
                        // A continuous curved run and reversal.
                        4 => (4.0 * (t * 0.7).sin(), 4.0 * (1.0 - (t * 0.7).cos())),
                        // Gradually alternate between lateral and backward travel.
                        _ => (2.0 * (t * 0.8).sin(), 2.0 * t),
                    };
                    Sample {
                        t: i * 30,
                        x: ((240.0 + x / Field::METERS_PER_UNIT) * 10.0).round() / 10.0,
                        y: ((120.0 + id as f32 * 48.0 + z / Field::METERS_PER_UNIT) * 10.0).round()
                            / 10.0,
                        z: 0.0,
                    }
                })
                .collect(),
        );
        tracks.players.insert(id, track);
    }
    Rehearsal::from_tracks(tracks, 0.0, 12_000.0, &[], &HashMap::new())
}

#[test]
fn locomotion_fixture_is_reproducible_at_each_cadence() {
    for fps in [30.0, 60.0, 120.0] {
        for speed in [0.25, 0.5, 1.0, 2.0] {
            let run = || {
                let mut rehearsal = fixture();
                rehearsal.at_fps(fps);
                rehearsal.at_speed(speed);
                rehearsal.seek(0.0);
                for _ in 0..(4.0 * fps / f64::from(speed)).round() as usize {
                    rehearsal.tick();
                }
                assert!((rehearsal.now() - 4_000.0).abs() < 1e-6);
                let mut hash = DefaultHasher::new();
                rehearsal.fingerprint(&mut hash);
                hash.finish()
            };
            assert_eq!(run(), run(), "{fps} fps at {speed}x");
        }
    }
}

#[derive(Clone)]
struct Picture {
    speed: f32,
    heading: f32,
    joints: Vec<Quat>,
    phase: f32,
    capped: bool,
}

fn picture(actor: &PlayerActor, delta: f32) -> Picture {
    let stride = Actors::stride_of(actor.id, actor.speed, actor.underfoot).0;
    Picture {
        speed: actor.speed,
        heading: actor.heading,
        joints: JOINTS
            .into_iter()
            .filter(|(family, _, _)| ["pelvis", "trunk", "torso", "hip", "knee"].contains(family))
            .map(|(_, limb, side)| step(limb, side, origin(limb, side), actor.pose).rotation)
            .collect(),
        phase: actor.phase,
        capped: actor.tread * delta * PI / stride >= FRAC_PI_2,
    }
}

fn pictures(fps: f64, speed: f32) -> Vec<Picture> {
    let mut rehearsal = fixture();
    rehearsal.at_fps(fps);
    rehearsal.at_speed(speed);
    rehearsal.seek(0.0);
    let mut pictures = Vec::new();
    // Common instants every 1/15 match second, after two seconds of warm-up.
    for at in 1..=150 {
        let target = at as f64 * 1000.0 / 15.0;
        while rehearsal.now() < target - 1e-6 {
            rehearsal.tick();
        }
        assert!((rehearsal.now() - target).abs() < 1e-5);
        if at <= 30 {
            continue;
        }
        for id in 1..=5 {
            pictures.push(picture(
                rehearsal.actor(id).unwrap(),
                (1000.0 / fps * f64::from(speed) / 1000.0) as f32,
            ));
        }
    }
    pictures
}

/// Common-time contract, including the pose produced by the support solver.
#[test]
fn report_locomotion_cadence() {
    let reference = pictures(60.0, 1.0);
    for fps in [30.0, 60.0, 120.0] {
        for speed in [0.25, 0.5, 1.0, 2.0] {
            let mut worst = [0.0f32; 4];
            let mut excluded = 0;
            for (a, b) in reference.iter().zip(pictures(fps, speed)) {
                if a.capped || b.capped {
                    excluded += 1;
                    continue;
                }
                worst[0] = worst[0].max((a.speed - b.speed).abs());
                worst[1] = worst[1].max(folded(a.heading - b.heading).abs().to_degrees());
                for (a, b) in a.joints.iter().zip(&b.joints) {
                    worst[2] = worst[2].max(a.angle_between(*b).to_degrees());
                }
                worst[3] = worst[3].max(folded(a.phase - b.phase).abs().to_degrees());
            }
            println!(
                "{fps}fps {speed}x: speed {:.4}m/s heading {:.3}deg joints {:.3}deg phase {:.3}deg; cap exclusions {excluded}",
                worst[0], worst[1], worst[2], worst[3]
            );
            assert!(worst[0] <= 0.2, "pace differs at {fps}fps/{speed}x");
            assert!(worst[1] <= 5.0, "facing differs at {fps}fps/{speed}x");
            assert!(worst[2] <= 10.0, "joint pose differs at {fps}fps/{speed}x");
        }
    }
}

#[test]
fn locomotion_stationary_jitter_does_not_start_a_stride() {
    let mut rehearsal = fixture();
    rehearsal.seek(0.0);
    for _ in 0..180 {
        rehearsal.tick();
    }
    let initial = rehearsal.actor(2).unwrap().phase;
    for _ in 0..180 {
        rehearsal.tick();
        let actor = rehearsal.actor(2).unwrap();
        assert!(actor.tread < Actors::STEPPING);
        assert!(folded(actor.phase - initial).abs() < 0.01);
    }
}

#[test]
fn locomotion_kept_support_does_not_slide() {
    let mut rehearsal = fixture();
    rehearsal.seek(0.0);
    let mut kept = 0;
    let mut worst = 0.0f32;
    for _ in 0..600 {
        rehearsal.tick();
        for id in 1..=5 {
            let position = rehearsal.position(id);
            let actor = rehearsal.actor(id).unwrap();
            let frame = placed(actor, position);
            let legs = Physique::legs(&actor.pose, &actor.posture);
            for (leg, plant) in legs.iter().zip(actor.plants.iter()) {
                if let Some(at) = plant.and_then(|plant| plant.held_spot()) {
                    kept += 1;
                    let actual = frame.transform_point(leg.sole);
                    worst = worst.max(Vec2::new(actual.x - at.x, actual.z - at.z).length());
                }
            }
        }
    }
    println!("{kept} held soles; worst anchor drift {worst:.4}m");
    assert!(kept > 100);
    assert!(worst <= 0.02, "held sole drifted {worst}m");
}

#[test]
fn locomotion_newly_loaded_neighbors_do_not_restart_the_stride() {
    let mut rehearsal = fixture();
    let rows = |from: u32, to: u32| {
        (from..=to)
            .map(|i| Sample {
                t: i * 30,
                x: ((240.0 + 0.72 * i as f32) * 10.0).round() / 10.0,
                y: 168.0,
                z: 0.0,
            })
            .collect()
    };
    let mut partial = Track::default();
    partial.merge(rows(0, 100));
    rehearsal
        .world
        .resource_mut::<ReplayTracks>()
        .players
        .insert(1, partial);
    rehearsal.seek(0.0);
    for _ in 0..180 {
        rehearsal.tick();
    }
    let before = picture(rehearsal.actor(1).unwrap(), 1.0 / 60.0);
    rehearsal
        .world
        .resource_mut::<ReplayTracks>()
        .players
        .get_mut(&1)
        .unwrap()
        .merge(rows(101, 400));
    rehearsal.tick();
    let after = picture(rehearsal.actor(1).unwrap(), 1.0 / 60.0);
    assert!((before.speed - after.speed).abs() < 0.2);
    assert!(folded(after.phase - before.phase).abs() < 0.3);
    assert!(folded(after.heading - before.heading).abs() < 5.0f32.to_radians());
    assert!((rehearsal.now() - 3016.6666667).abs() < 0.01);
}

#[test]
fn locomotion_long_frame_is_bounded_and_stride_is_capped_once() {
    let mut rehearsal = fixture();
    rehearsal.seek(0.0);
    for _ in 0..180 {
        rehearsal.tick();
    }
    let before = rehearsal.actor(1).unwrap().phase;
    rehearsal.at_fps(15.0);
    rehearsal.at_speed(16.0);
    rehearsal.tick();
    let actor = rehearsal.actor(1).unwrap();
    assert!(folded(actor.phase - before).abs() <= FRAC_PI_2 + 1e-5);
    assert!(actor.speed.is_finite() && actor.heading.is_finite());
    assert!(actor.pose.phase.is_finite());
    assert!((rehearsal.now() - 4066.6666667).abs() < 0.01);
}

#[test]
fn locomotion_reset_discards_source_motion_and_support() {
    let mut rehearsal = fixture();
    rehearsal.seek(0.0);
    for _ in 0..180 {
        rehearsal.tick();
    }
    let entity = rehearsal.entity(1);
    let mut actor = rehearsal.world.get_mut::<PlayerActor>(entity).unwrap();
    actor.reset_locomotion(Vec3::new(20.0, 0.0, 10.0));
    assert_eq!(actor.heading_rate, 0.0);
    assert_eq!(actor.drive_rate, 0.0);
    assert_eq!(actor.turn_rate, 0.0);
    assert_eq!(actor.travel, Vec3::ZERO);
    assert_eq!(actor.tread_vec, Vec3::ZERO);
    assert!(actor.plants.iter().all(Option::is_none));
}

fn passing_fixture() -> Rehearsal {
    let mut tracks = ReplayTracks::default();
    for id in [1, 2] {
        let mut track = Track::default();
        track.merge(
            (0..=300)
                .map(|i| Sample {
                    t: i * 30,
                    x: 240.0 + 120.0 * (id - 1) as f32 + 0.72 * i as f32,
                    y: 180.0,
                    z: 0.0,
                })
                .collect(),
        );
        tracks.players.insert(id, track);
    }
    tracks.ball.merge(
        (0..=300)
            .map(|i| {
                let t = i as f32 * 0.03;
                let x = if t < 4.0 {
                    240.0 + 24.0 * t
                } else if t < 5.25 {
                    336.0 + 120.0 * (t - 4.0)
                } else {
                    486.0 + 24.0 * (t - 5.25)
                };
                Sample {
                    t: i * 30,
                    x,
                    y: 180.0,
                    z: 0.0,
                }
            })
            .collect(),
    );
    Rehearsal::from_tracks(tracks, 0.0, 9000.0, &[], &HashMap::new())
}

#[test]
fn locomotion_running_pass_and_reception_preserve_the_step() {
    let mut rehearsal = passing_fixture();
    rehearsal.seek(0.0);
    let mut phases = [None; 2];
    let mut strikes = 0;
    let mut receptions = 0;
    let mut supported_actions = 0;
    for _ in 0..480 {
        rehearsal.tick();
        for id in [1, 2] {
            let now = rehearsal.now();
            let actor = rehearsal.actor(id).unwrap();
            let gait = actor.pose;
            if let Some(was) = phases[id as usize - 1] {
                assert!(folded(actor.phase - was).abs() <= FRAC_PI_2 + 1e-5);
            }
            phases[id as usize - 1] = Some(actor.phase);
            if gait.power > 0.05 {
                strikes += 1;
                assert!((now - 4000.0).abs() < 900.0);
            }
            if gait.trap > 0.05 {
                receptions += 1;
                assert!((now - 5250.0).abs() < 900.0);
            }
            if gait.power + gait.trap > 0.05 {
                let foot = usize::from(gait.foot > 0.0);
                assert_eq!(gait.hold[foot].weight, 0.0, "action foot stayed planted");
                supported_actions += usize::from(gait.hold[1 - foot].weight > 0.5);
            }
        }
    }
    assert!(strikes > 3, "no running pass: {strikes}");
    assert!(receptions > 3, "no reception: {receptions}");
    assert!(
        supported_actions > 0,
        "no supporting contact during either action"
    );
}

#[test]
fn locomotion_seek_reconstructs_destination_motion_without_source_contacts() {
    let mut played = fixture();
    played.seek(0.0);
    for _ in 0..360 {
        played.tick();
    }
    let mut jumped = fixture();
    jumped.seek(0.0);
    for _ in 0..60 {
        jumped.tick();
    }
    jumped.seek(6000.0);
    for id in 1..=5 {
        let position = jumped.position(id);
        let a = picture(played.actor(id).unwrap(), 1.0 / 60.0);
        let actor = jumped.actor(id).unwrap();
        assert!((actor.speed - a.speed).abs() < 0.2, "pace #{id}");
        assert!(
            folded(actor.heading - a.heading).abs() < 5.0f32.to_radians(),
            "heading #{id}"
        );
        for plant in actor.plants.iter().flatten() {
            if let Some(at) = plant.held_spot() {
                assert!((at - position).length() < 1.5, "source plant survived seek");
            }
        }
    }
}

#[test]
fn locomotion_curves_obey_yaw_ceiling_and_stops_finish_supported() {
    let mut rehearsal = fixture();
    rehearsal.seek(0.0);
    let mut heading = rehearsal.actor(4).unwrap().heading;
    for _ in 0..690 {
        rehearsal.tick();
        let actor = rehearsal.actor(4).unwrap();
        let ceiling = Actors::PIVOT_RATE.0.max(Actors::PIVOT_RATE.1);
        assert!(folded(actor.heading - heading).abs() <= ceiling / 60.0 + 1e-4);
        heading = actor.heading;
    }
    for id in [1, 3] {
        let actor = rehearsal.actor(id).unwrap();
        assert!(actor.tread < Actors::STEPPING);
        assert!(
            actor
                .plants
                .iter()
                .flatten()
                .any(|plant| plant.held_spot().is_some())
        );
    }
}

fn keeper_fixture(moving: bool) -> Rehearsal {
    let mut tracks = ReplayTracks::default();
    let mut keeper = Track::default();
    keeper.merge(
        (0..=300)
            .map(|i| {
                let t = i as f32 * 0.03;
                let flight = (t - 4.0).clamp(0.0, 0.6);
                let x = 48.0
                    + 48.0 * flight
                    + if moving {
                        24.0 * (t - 4.6).max(0.0)
                    } else {
                        0.0
                    };
                let z = if t > 4.0 && t < 4.6 {
                    0.35 * (PI * flight / 0.6).sin()
                } else {
                    0.0
                };
                Sample {
                    t: i * 30,
                    x,
                    y: 272.5,
                    z,
                }
            })
            .collect(),
    );
    tracks.players.insert(7, keeper);
    tracks.ball.merge(
        (0..=300)
            .map(|i| {
                let t = i as f32 * 0.03;
                Sample {
                    t: i * 30,
                    x: (420.0 - 90.0 * t).max(20.0),
                    y: 272.5,
                    z: 0.1,
                }
            })
            .collect(),
    );
    Rehearsal::from_tracks(tracks, 0.0, 9000.0, &[7], &HashMap::new())
}

#[test]
fn locomotion_keeper_recovers_into_the_recorded_ground_movement() {
    for moving in [false, true] {
        let mut rehearsal = keeper_fixture(moving);
        rehearsal.seek(0.0);
        let mut airborne = 0;
        let mut dragged = 0;
        let mut previous = None;
        for _ in 0..480 {
            rehearsal.tick();
            let position = rehearsal.position(7);
            let ground = previous.map_or(0.0, |was: Vec3| (position - was).length() * 60.0);
            previous = Some(position);
            let actor = rehearsal.actor(7).unwrap();
            airborne += usize::from(actor.height > Actors::AIRBORNE_FEET && actor.dive > 0.1);
            dragged += usize::from(
                actor.height <= Actors::AIRBORNE_FEET && actor.dive > 0.7 && ground > 2.0,
            );
        }
        let actor = rehearsal.actor(7).unwrap();
        assert!(airborne > 10, "no recorded dive");
        assert!(dragged <= 12, "sprawled body dragged for {dragged} frames");
        assert!(actor.dive < 0.01);
        if moving {
            assert!(actor.speed > 2.5);
        } else {
            assert!(actor.speed < 0.1);
        }
    }
}

#[test]
fn locomotion_pause_freezes_all_movement_state() {
    let mut rehearsal = fixture();
    rehearsal.seek(0.0);
    for _ in 0..180 {
        rehearsal.tick();
    }
    let fingerprint = |rehearsal: &Rehearsal| {
        let mut hash = DefaultHasher::new();
        rehearsal.fingerprint(&mut hash);
        for &(_, entity) in &rehearsal.players {
            let actor = rehearsal.world.get::<PlayerActor>(entity).unwrap();
            for value in [
                actor.phase,
                actor.speed,
                actor.heading,
                actor.heading_rate,
                actor.drive,
                actor.turn,
                actor.stance,
                actor.look,
                actor.idle,
                actor.tread,
            ] {
                value.to_bits().hash(&mut hash);
            }
        }
        hash.finish()
    };
    let before = fingerprint(&rehearsal);
    for _ in 0..30 {
        rehearsal.hold();
    }
    assert_eq!(before, fingerprint(&rehearsal));
}

#[test]
fn locomotion_lineup_uses_ceremony_roots_even_with_loaded_tracks() {
    let mut actor = PlayerActor::new(7, false, true);
    actor.stand_to_attention(0.0);
    let mut track = Track::default();
    track.merge(vec![
        Sample {
            t: 0,
            x: 20.0,
            y: 20.0,
            z: 0.0,
        },
        Sample {
            t: 1000,
            x: 20.0,
            y: 20.0,
            z: 0.0,
        },
    ]);
    let mut root = Transform::from_translation(Vec3::new(2.0, 0.0, 3.0));
    actor.advance_locomotion(
        Some(&track),
        (0.0, 1000.0),
        500.0,
        1.0 / 60.0,
        &BallState::default(),
        &mut root,
        Vec3::ZERO,
        true,
        true,
    );
    let held: Vec<_> = actor
        .plants
        .iter()
        .flatten()
        .filter_map(|plant| plant.held_spot())
        .collect();
    assert!(!held.is_empty());
    assert!(held.iter().all(|at| at.distance(root.translation) < 1.5));
}
