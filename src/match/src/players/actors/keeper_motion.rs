//! Goalkeeper-only timing derived from the recording; no simulation changes.
use super::*;

impl Actors {
    /// Load the legs in the last 150 ms before a recorded take-off. Looking
    /// beyond that point rejects the tiny split-step hops. Never extrapolate
    /// across missing chunks or a restart teleport.
    pub(super) fn keeper_coil(track: &mut Track, now: f64) -> f32 {
        let Some(here) = track.position_ahead(now) else {
            return 0.0;
        };
        if here[2] > Self::AIRBORNE_FEET {
            return 0.0;
        }
        for step in 1..=5 {
            let delay = step as f64 * 30.0;
            let Some(next) = track.position_ahead(now + delay) else {
                return 0.0;
            };
            if next[2] <= Self::AIRBORNE_FEET {
                continue;
            }
            let Some(up) = track.position_ahead(now + delay + 90.0) else {
                return 0.0;
            };
            let distance =
                Vec2::new(next[0] - here[0], next[1] - here[1]).length() * Field::METERS_PER_UNIT;
            if up[2] < Self::HOP_CEILING || distance > Self::TELEPORT * delay as f32 * 0.001 {
                return 0.0;
            }
            return 0.65 * Self::ease(1.0 - delay as f32 / 180.0);
        }
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::players::body::skeleton::{boot, glove};
    use crate::recording::replay::Sample;
    use bevy::ecs::system::RunSystemOnce;
    use std::time::Duration;

    fn world(speed: f32, frame: f32) -> (World, Entity) {
        let mut world = World::new();
        let mut playback = Playback::new(100_000.0);
        playback.playing = true;
        playback.speed = speed;
        playback.seeked = false;
        world.insert_resource(playback);
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_secs_f32(frame));
        world.insert_resource(time);
        world.insert_resource(BallState::default());
        world.insert_resource(Aftermath::default());
        let actor = world
            .spawn((
                PlayerActor::new(100, true, true),
                Transform::default(),
                Visibility::Inherited,
            ))
            .id();
        (world, actor)
    }

    // Exercise the production system: the older flight harness computed
    // touchdown before track_flight and therefore missed the actual bug.
    #[test]
    fn playback_landing_absorbs_weight_and_returns_to_running() {
        let (mut world, entity) = world(1.0, 1.0 / 60.0);
        let mut maximum = 0.0f32;
        for frame in 0..110 {
            let t = frame as f32 / 60.0;
            let h = (3.2 * t - 4.905 * t * t).max(0.0);
            {
                let mut actor = world.get_mut::<PlayerActor>(entity).unwrap();
                actor.height = h;
                actor.declared = KeeperFlight::Leap;
            }
            world.get_mut::<Transform>(entity).unwrap().translation.z =
                t * 3.0 + (t - 0.9).max(0.0);
            world.resource_mut::<Playback>().time_ms = t as f64 * 1000.0;
            world.run_system_once(Actors::animate).unwrap();
            let actor = world.get::<PlayerActor>(entity).unwrap();
            if frame > 40 {
                maximum = maximum.max(actor.pose.land);
            }
            if frame > 52 {
                assert!(
                    actor.pose.jump < 0.001,
                    "flight pose returned during recovery"
                );
                assert!(actor.pose.reach < 0.001, "still reaching after landing");
            }
        }
        assert!(maximum > 0.12, "landing was not absorbed: {maximum}");
        let actor = world.get::<PlayerActor>(entity).unwrap();
        assert!(
            actor.pose.run > 0.5,
            "running after landing is still suppressed"
        );
        assert!(actor.pose.carry_ground > 0.25);
    }

    #[test]
    fn keeper_jump_drives_one_knee_then_extends_both_legs() {
        let mut actor = PlayerActor::new(100, true, true);
        actor.declared = KeeperFlight::Leap;
        let mut apex = None;
        let mut descending = None;
        for frame in 1..39 {
            let t = frame as f32 / 60.0;
            actor.height = (3.2 * t - 4.905 * t * t).max(0.0);
            actor.track_flight(1.0 / 60.0, 4.0, 4.0, false);
            if frame == 20 {
                apex = Some(actor.gait());
            }
            if frame == 38 {
                descending = Some(actor.gait());
            }
        }
        let apex = apex.unwrap();
        let descending = descending.unwrap();
        let mut without_dive_arch = apex;
        without_dive_arch.stretch = 0.0;
        assert!(
            crate::players::body::skeleton::crown(apex)
                .distance(crate::players::body::skeleton::crown(without_dive_arch))
                < 0.02,
            "upright jump is also receiving the dive's back arch"
        );
        let left = boot(-1.0, apex);
        let right = boot(1.0, apex);
        assert!(
            (left.z - right.z).abs() > 0.15,
            "both legs have the same pose"
        );
        for side in [-1.0, 1.0] {
            assert!(
                boot(side, descending).y < boot(side, apex).y - 0.04,
                "leg {side} never unfolds before touchdown"
            );
        }
    }

    #[test]
    fn replay_speed_preserves_running_and_turning_in_match_time() {
        let sample = |speed, frame| {
            let (mut world, entity) = world(speed, frame);
            for step in 0..90 {
                let t = step as f32 / 60.0;
                let z = if t < 0.8 {
                    t * 3.0
                } else {
                    2.4 - (t - 0.8) * 2.0
                };
                world.get_mut::<Transform>(entity).unwrap().translation =
                    Vec3::new(t * 1.5, 0.0, z);
                world.resource_mut::<Playback>().time_ms = t as f64 * 1000.0;
                world.run_system_once(Actors::animate).unwrap();
            }
            let actor = world.get::<PlayerActor>(entity).unwrap();
            (
                actor.speed,
                actor.heading,
                actor.course,
                boot(1.0, actor.pose),
            )
        };
        let normal = sample(1.0, 1.0 / 60.0);
        for speed in [0.25, 2.0, 8.0] {
            let fast = sample(speed, 1.0 / (60.0 * speed));
            assert!((normal.0 - fast.0).abs() < 0.005);
            assert!((normal.1 - fast.1).abs() < 0.005);
            assert!(normal.2.distance(fast.2) < 0.005);
            assert!(normal.3.distance(fast.3) < 0.005);
        }
    }

    #[test]
    fn recorded_takeoff_loads_knees_but_small_hops_and_missing_data_do_not() {
        let track = |height| {
            let mut track = Track::default();
            track.merge(
                (0..=12)
                    .map(|i| Sample {
                        t: i * 30,
                        x: 20.0,
                        y: 275.0,
                        z: if i < 6 { 0.0 } else { height },
                    })
                    .collect(),
            );
            track
        };
        let mut leap = track(0.3);
        assert_eq!(Actors::keeper_coil(&mut leap, 0.0), 0.0);
        let early = Actors::keeper_coil(&mut leap, 60.0);
        let late = Actors::keeper_coil(&mut leap, 120.0);
        assert!(late > early && late > 0.25);
        assert_eq!(Actors::keeper_coil(&mut leap, 360.0), 0.0);
        assert_eq!(Actors::keeper_coil(&mut track(0.05), 120.0), 0.0);
        assert_eq!(Actors::keeper_coil(&mut Track::default(), 0.0), 0.0);
    }

    #[test]
    fn save_contact_has_follow_through_without_anticipating_impact() {
        let mut actor = PlayerActor::new(100, true, true);
        actor.save_time = Some(10.0);
        actor.reaction = 1.0;
        actor.clock = 9.99;
        assert_eq!(actor.gait().save_recoil, 0.0);
        let before = glove(1.0, actor.gait());
        actor.clock = 10.08;
        assert!(actor.gait().save_recoil > 0.9);
        assert!(before.distance(glove(1.0, actor.gait())) > 0.025);
        actor.clock = 10.4;
        assert_eq!(actor.gait().save_recoil, 0.0);
        actor.clock = 10.08;
        actor.carry = 1.0;
        assert_eq!(actor.gait().save_recoil, 0.0);
    }

    #[test]
    fn seeking_into_a_named_leap_clears_previous_dive_direction() {
        let mut actor = PlayerActor::new(100, true, true);
        actor.tip = Vec2::X;
        actor.flight = Vec3::X;
        actor.declared = KeeperFlight::Leap;
        actor.height = 0.08;
        actor.track_flight(0.03, 0.0, 0.0, true);
        assert!(!actor.bounce);
        assert!(actor.gait().jump > 0.99);
        assert_eq!(actor.topple(), (0.0, 0.0));
    }

    #[test]
    fn only_a_recorded_punch_closes_the_gloves_and_it_releases_smoothly() {
        let (mut world, entity) = world(1.0, 1.0 / 60.0);
        {
            let mut actor = world.get_mut::<PlayerActor>(entity).unwrap();
            actor.parry = 1.0;
            actor.reaction = 1.0;
        }
        world.run_system_once(Actors::animate).unwrap();
        assert_eq!(world.get::<PlayerActor>(entity).unwrap().pose.punch, 0.0);
        world.get_mut::<PlayerActor>(entity).unwrap().punching = true;
        for _ in 0..12 {
            world.run_system_once(Actors::animate).unwrap();
        }
        assert!(world.get::<PlayerActor>(entity).unwrap().pose.punch > 0.98);
        world.get_mut::<PlayerActor>(entity).unwrap().punching = false;
        world.run_system_once(Actors::animate).unwrap();
        assert!(world.get::<PlayerActor>(entity).unwrap().pose.punch > 0.8);
        for _ in 0..60 {
            world.run_system_once(Actors::animate).unwrap();
        }
        assert!(world.get::<PlayerActor>(entity).unwrap().pose.punch < 0.01);
    }

    #[test]
    fn goal_reaction_develops_then_releases_a_stationary_keeper() {
        let mut actor = PlayerActor::new(100, true, true);
        actor.despair = 1.0;
        actor.goal_since = Some(0.0);
        assert_eq!(actor.keeper_gesture(), 0.0);
        actor.goal_since = Some(2.0);
        assert_eq!(actor.keeper_gesture(), 1.0);
        actor.dive = 1.0;
        actor.stretch = 1.0;
        actor.tip = Vec2::X;
        actor.flat = 1.0;
        for frame in 1..=600 {
            actor.goal_since = Some(frame as f32 / 60.0);
            actor.track_flight(1.0 / 60.0, 0.0, 0.0, false);
        }
        assert!(
            actor.dive < 0.01,
            "held kneeling until the whole celebration ended"
        );
        assert_eq!(actor.keeper_gesture(), 0.0);
        assert!(
            actor.gait().despair > 0.9,
            "getting up erased the disappointment"
        );
    }

    /// Render the actual animation system from a streamed recording, including
    /// the approach, flight, and landing. Output RGBA for the local review tool.
    /// MATCH_REPLAY=chunk.json MATCH_KEEPER_START=2687190 MATCH_FIGURE_DUMP=dir
    #[test]
    #[ignore = "requires a recording and writes a rendered motion strip"]
    fn dump_recorded_keeper_motion() {
        use crate::players::body::{
            BodyParts,
            preview::{Canvas, Lens, posed},
        };
        let mut tracks = super::super::replayed::Chunk::open().expect("MATCH_REPLAY");
        let start: f64 = std::env::var("MATCH_KEEPER_START")
            .expect("MATCH_KEEPER_START")
            .parse()
            .unwrap();
        let directory = std::path::PathBuf::from(
            std::env::var("MATCH_FIGURE_DUMP").expect("MATCH_FIGURE_DUMP"),
        );
        let keeper: u32 = std::env::var("MATCH_KEEPER_ID")
            .unwrap_or("100".into())
            .parse()
            .unwrap();
        let (mut world, entity) = world(1.0, 1.0 / 60.0);
        world.get_mut::<PlayerActor>(entity).unwrap().id = keeper;
        world.get_mut::<PlayerActor>(entity).unwrap().heading = FRAC_PI_2;
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::tailor(&mut meshes, Grain::FULL);
        let lens = Lens {
            bearing: 1.0,
            bottom: -0.08,
            top: 3.0,
        };
        const W: usize = 240;
        const H: usize = 360;
        let captures = [0, 8, 16, 24, 32, 40, 48, 56, 64, 78];
        let mut sheet = vec![0u8; W * captures.len() * H * 4];
        let mut report = String::from("seconds,height,jump,phase,landing,run,reach\n");
        for frame in -120..=90 {
            let now = start + frame as f64 * 1000.0 / 60.0;
            let p = tracks
                .players
                .get_mut(&keeper)
                .unwrap()
                .position_at(now)
                .unwrap();
            let position = Field::to_world(p[0], p[1], p[2]);
            let named = tracks
                .states
                .get_mut(&keeper)
                .and_then(|track| track.name_at(now));
            {
                let mut actor = world.get_mut::<PlayerActor>(entity).unwrap();
                actor.height = position.y;
                actor.declared = Actors::declared(named, position.y, actor.declared);
                actor.arrival = Actors::next_arrival(&mut tracks.ball, now, position);
                if position.y <= Actors::AIRBORNE_FEET {
                    actor.coil = Actors::keeper_coil(tracks.players.get_mut(&keeper).unwrap(), now);
                }
            }
            world.get_mut::<Transform>(entity).unwrap().translation =
                Vec3::new(position.x, 0.0, position.z);
            if let Some(b) = tracks.ball.position_at(now) {
                let mut ball = world.resource_mut::<BallState>();
                ball.on_pitch = true;
                ball.position = Field::to_world(b[0], b[1], b[2]);
                let reach =
                    Vec2::new(ball.position.x - position.x, ball.position.z - position.z).length();
                ball.nearest = Some((keeper, reach));
                ball.held_by = Actors::in_his_hands(reach, b[2], true).then_some(keeper);
                ball.cradle = f32::from(ball.held_by.is_some());
            }
            world.resource_mut::<Playback>().time_ms = now;
            world.run_system_once(Actors::animate).unwrap();
            if frame < 0 {
                continue;
            }
            let actor = world.get::<PlayerActor>(entity).unwrap();
            let gait = actor.pose;
            let (pitch, roll) = actor.topple();
            let mut canvas = Canvas::new(W, H);
            posed(
                &mut canvas,
                &lens,
                &meshes,
                &parts,
                gait,
                Carriage::placed(pitch, roll, actor.lift()),
                true,
            );
            let pixels = canvas.pixels();
            std::fs::write(directory.join(format!("frame-{frame:03}.rgba")), &pixels).unwrap();
            if let Some(column) = captures.iter().position(|&capture| capture == frame) {
                for row in 0..H {
                    let to = (row * W * captures.len() + column * W) * 4;
                    sheet[to..to + W * 4].copy_from_slice(&pixels[row * W * 4..(row + 1) * W * 4]);
                }
                report.push_str(&format!(
                    "{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}\n",
                    frame as f32 / 60.0,
                    actor.height,
                    gait.jump,
                    gait.jump_phase,
                    gait.land,
                    gait.run,
                    gait.reach
                ));
            }
        }
        std::fs::write(directory.join("recorded-keeper.rgba"), sheet).unwrap();
        std::fs::write(directory.join("recorded-keeper.csv"), report).unwrap();
        println!(
            "Rendered 91 frames, {W}x{H}; strip {}x{H}",
            W * captures.len()
        );
    }
}

impl PlayerActor {
    /// Landing ends flight even while the recovery envelope is decaying.
    /// Using `1 - settling()` here revives the flight pose during recovery,
    /// because settling includes that same decaying envelope.
    pub(super) fn flight_blend(&self) -> f32 {
        let landing = if self.flat < 0.2 {
            0.08
        } else {
            Actors::GROUNDING
        };
        1.0 - Actors::ease(self.down / landing)
    }

    /// Ballistic progress: knees drive up during ascent, then unfold before
    /// touchdown. The height gate handles short, low flights as well.
    pub(super) fn jump_progress(&self) -> f32 {
        if self.vertical_speed < 0.0 {
            0.5 + 0.5 * Actors::ease(1.0 - self.height / 0.28)
        } else if self.climb > 0.1 {
            0.5 * (1.0 - self.vertical_speed / self.climb).clamp(0.0, 1.0)
        } else {
            0.5
        }
    }

    /// A short give in the elbows and chest after contact. Suppressed once
    /// another action owns the hands. Never anticipates impact.
    pub(super) fn save_recoil(&self) -> f32 {
        let Some(contact) = self.save_time else {
            return 0.0;
        };
        let since = self.clock - contact;
        if !(0.0..0.32).contains(&since) {
            return 0.0;
        }
        let pulse = Actors::ease(since / 0.07) * (1.0 - Actors::ease((since - 0.07) / 0.25));
        pulse * self.reaction * (1.0 - self.carry) * (1.0 - self.despair.max(self.elation))
    }

    /// Disbelief, a held gesture, then arms dropping as he exhales. The
    /// individual reaction stays the same; its timing is no longer a statue.
    pub(super) fn keeper_gesture(&self) -> f32 {
        let Some(since) = self.goal_since.filter(|_| self.is_goalkeeper) else {
            return 1.0;
        };
        let delay = 0.25 + 0.15 * Complexion::carriage(self.id);
        Actors::ease((since - delay) / 0.7) * (1.0 - Actors::ease((since - 3.8 - delay) / 2.0))
    }

    /// Release the kneeling hold before the restart, even when the engine
    /// leaves his position unchanged through the celebration.
    pub(super) fn keeper_grief(&self) -> f32 {
        self.goal_since
            .map_or(1.0, |since| 1.0 - Actors::ease((since - 4.0) / 2.5))
    }

    pub(super) fn keeper_head_shake(&self) -> f32 {
        let Some(since) = self.goal_since.filter(|_| self.is_goalkeeper) else {
            return 0.0;
        };
        let t = since - 1.0;
        if !(0.0..1.8).contains(&t) {
            return 0.0;
        }
        0.22 * (t * TAU / 0.9).sin()
            * (PI * t / 1.8).sin().powi(2)
            * self.despair
            * (1.0 - self.carry)
            * (1.0 - self.dive)
            * (1.0 - Actors::ease(self.speed / Actors::MOVING))
    }
}
