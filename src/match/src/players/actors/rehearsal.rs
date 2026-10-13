//! **The viewer's own pipeline over a real recording, headless**: the
//! playhead, [`Actors::follow_playhead`] and [`Actors::animate`] for all
//! twenty-two at sixty frames a second, so what is measured or drawn here is
//! the gait the renderer is handed — the ball at a man's feet, the kick about
//! to leave him and the save he is making included, none of which
//! [`super::replayed::Walker`] sees.
//!
//! ```text
//! MATCH_REPLAY=<chunk.json> [MATCH_AT=<ms>] [MATCH_SPAN=<s>] cargo test --lib census_motion -- --ignored --nocapture
//! MATCH_REPLAY=<chunk.json> MATCH_PLAYER=<id> MATCH_AT=<ms> MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_rehearsal -- --ignored --nocapture
//! ```

use super::pre_roll::PreRoll;
use super::replayed::Chunk;
use super::*;
use crate::app::quality::Tier;
use crate::players::body::preview::{Canvas, Lens, ball, posed};
use crate::players::body::skeleton::{boot, crown, step};
use crate::players::body::{BodyParts, Footballer, Limb};
use crate::players::kit::{HairStyle, Outfit};
use crate::recording::playback::RecordedSpans;
use bevy::ecs::system::SystemId;
use bevy::transform::systems::{
    StaticTransformOptimizations, mark_dirty_trees, propagate_parent_transforms,
    sync_simple_transforms,
};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

/// One recording, one world, the real systems in the order the app runs them.
pub(crate) struct Rehearsal {
    world: World,
    ball: Entity,
    players: Vec<(u32, Entity)>,
    systems: Vec<(&'static str, SystemId)>,
    /// What each system cost on the last frame, in microseconds.
    laps: Vec<f32>,
    /// How many `GlobalTransform`s the last frame changed.
    changed: usize,
    /// Match seconds a drawn frame covers, as the speed chip sets it.
    speed: f32,
    now: f64,
    pub start: f64,
    pub until: f64,
}

impl Rehearsal {
    pub const FRAME_MS: f64 = 1000.0 / 60.0;

    pub fn open() -> Option<Rehearsal> {
        let mut tracks = Chunk::open()?;
        let (start, until) = tracks.ball.span()?;
        let keepers = Chunk::keepers(&mut tracks, start);
        let sides = Chunk::sides(&mut tracks, start);
        let mut ids: Vec<u32> = tracks.players.keys().copied().collect();
        ids.sort_unstable();

        let mut world = World::new();
        let mut playback = Playback::new(until);
        playback.playing = true;
        playback.time_ms = start;
        world.insert_resource(playback);
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_secs_f64(Self::FRAME_MS / 1000.0));
        world.insert_resource(time);
        world.insert_resource(ChunkLoader::default());
        world.insert_resource(BallState::default());
        world.insert_resource(Aftermath::default());
        world.insert_resource(Lineup::default());
        world.insert_resource(tracks);
        let ball = world
            .spawn((BallActor, Transform::default(), Visibility::Hidden))
            .id();
        let players = ids
            .into_iter()
            .map(|id| {
                let actor = PlayerActor::new(
                    id,
                    keepers.contains(&id),
                    sides.get(&id).copied().unwrap_or(true),
                );
                (
                    id,
                    world
                        .spawn((actor, Transform::default(), Visibility::Inherited))
                        .id(),
                )
            })
            .collect();
        world.insert_resource(RecordedSpans::default());
        let follow = world.register_system(Actors::follow_playhead);
        let animate = world.register_system(Actors::animate);
        world.insert_resource(PreRoll::new(vec![follow, animate]));
        let systems = vec![
            ("follow", follow),
            ("animate", animate),
            ("pre-roll", world.register_system(PreRoll::run)),
            ("pose", world.register_system(Actors::pose_joints)),
            ("end", world.register_system(Playback::end_frame)),
        ];
        Some(Rehearsal {
            world,
            ball,
            players,
            systems,
            laps: Vec::new(),
            changed: 0,
            speed: 1.0,
            now: start,
            start,
            until,
        })
    }

    /// Hangs a body on every man, as [`Actors::take_the_field`] does, and adds
    /// what the app runs on those bodies every frame: the carriage, and Bevy's
    /// transform propagation behind the whole of `Update`.
    pub fn dress(&mut self, grain: Grain) {
        let mut meshes = Assets::<Mesh>::default();
        let mut binds = Assets::<SkinnedMeshInverseBindposes>::default();
        let mut parts = BodyParts::new(&mut meshes, grain);
        parts.bind_clothes(&mut binds);
        // Prints and hair as a dressed man wears them, so the entity count is
        // the app's; nothing here is ever drawn, so no handle has to resolve.
        let outfit = Outfit {
            kit: Handle::default(),
            limb: Handle::default(),
            strip: 0,
            boots: Handle::default(),
            skin: Handle::default(),
            face: Handle::default(),
            hands: Handle::default(),
            hair: Handle::default(),
            hair_style: HairStyle::Short,
            number: Some(Handle::default()),
            name: Some(Handle::default()),
            name_front: Some(Handle::default()),
        };
        for &(_, entity) in &self.players {
            let keeper = self
                .world
                .get::<PlayerActor>(entity)
                .is_some_and(|actor| actor.is_goalkeeper);
            Footballer::assemble(&mut self.world.commands(), entity, &parts, &outfit, keeper);
        }
        self.world.flush();
        self.world.insert_resource(meshes);
        self.world.insert_resource(binds);
        self.world.init_resource::<StaticTransformOptimizations>();
        let carry = self.world.register_system(Actors::carry_body);
        let after_pose = self
            .systems
            .iter()
            .position(|(name, _)| *name == "pose")
            .map_or(self.systems.len(), |at| at + 1);
        self.systems.insert(after_pose, ("carry", carry));
        let propagation = [
            ("dirty", self.world.register_system(mark_dirty_trees)),
            (
                "propagate",
                self.world.register_system(propagate_parent_transforms),
            ),
            ("sync", self.world.register_system(sync_simple_transforms)),
        ];
        self.systems.extend(propagation);
    }

    /// Cuts to `at` the way a scrub does, so every filter starts from rest.
    pub fn seek(&mut self, at: f64) {
        self.now = at;
        let mut playback = self.world.resource_mut::<Playback>();
        playback.time_ms = at;
        playback.seeked = true;
        playback.advanced_ms = 0.0;
        self.run();
    }

    /// Plays at `speed` from here on, as the speed chip does.
    pub fn at_speed(&mut self, speed: f32) {
        self.speed = speed;
        self.world.resource_mut::<Playback>().speed = speed;
    }

    /// One frame of play.
    pub fn tick(&mut self) {
        self.now += Self::FRAME_MS * f64::from(self.speed);
        let mut playback = self.world.resource_mut::<Playback>();
        playback.time_ms = self.now;
        playback.playing = true;
        playback.advanced_ms = Self::FRAME_MS * f64::from(self.speed);
        self.run();
    }

    /// One frame with the playhead stopped, as the transport bar's pause has it.
    pub fn hold(&mut self) {
        let mut playback = self.world.resource_mut::<Playback>();
        playback.playing = false;
        playback.advanced_ms = 0.0;
        self.run();
    }

    /// Runs the frame and clears the change trackers behind it, as the app does
    /// at the end of every update — so `changed` counts this frame's writes.
    fn run(&mut self) {
        self.laps.clear();
        for index in 0..self.systems.len() {
            let (_, system) = self.systems[index];
            let clock = Instant::now();
            self.world.run_system(system).expect("a pipeline system");
            self.laps.push(clock.elapsed().as_secs_f32() * 1e6);
        }
        self.changed = self
            .world
            .query_filtered::<(), Changed<GlobalTransform>>()
            .iter(&self.world)
            .count();
        self.world.clear_trackers();
    }

    pub fn stages(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.systems.iter().map(|(name, _)| *name)
    }

    pub fn laps(&self) -> &[f32] {
        &self.laps
    }

    pub fn changed(&self) -> usize {
        self.changed
    }

    pub fn joints(&mut self) -> usize {
        self.world.query::<&Joint>().iter(&self.world).count()
    }

    /// Every joint's rotation this frame, for comparing two frames.
    pub fn joint_rotations(&mut self) -> HashMap<Entity, Quat> {
        self.world
            .query_filtered::<(Entity, &Transform), With<Joint>>()
            .iter(&self.world)
            .map(|(entity, transform)| (entity, transform.rotation))
            .collect()
    }

    /// Every pose in the squad folded into `hasher`: each man's root, carriage
    /// and joints, walked down his own hierarchy so the order is the rig's and
    /// not the order the archetypes happen to be stored in.
    pub fn fingerprint(&self, hasher: &mut DefaultHasher) {
        for &(_, entity) in &self.players {
            self.fold(entity, hasher);
        }
    }

    fn fold(&self, entity: Entity, hasher: &mut DefaultHasher) {
        let node = self.world.entity(entity);
        let posed = node.contains::<PlayerActor>()
            || node.contains::<Carriage>()
            || node.contains::<Joint>();
        if let Some(transform) = node.get::<Transform>().filter(|_| posed) {
            transform
                .translation
                .to_array()
                .map(f32::to_bits)
                .hash(hasher);
            transform.rotation.to_array().map(f32::to_bits).hash(hasher);
            transform.scale.to_array().map(f32::to_bits).hash(hasher);
        }
        if let Some(children) = node.get::<Children>() {
            for child in children.iter() {
                self.fold(child, hasher);
            }
        }
    }

    pub fn now(&self) -> f64 {
        self.now
    }

    pub fn players(&self) -> impl Iterator<Item = u32> + '_ {
        self.players.iter().map(|(id, _)| *id)
    }

    fn entity(&self, id: u32) -> Entity {
        self.players
            .iter()
            .find(|(each, _)| *each == id)
            .map(|(_, entity)| *entity)
            .expect("a player in this chunk")
    }

    /// The actor, if the recording has him on the pitch this frame. A default
    /// loader covers nothing, so the pipeline leaves a man with no samples
    /// standing where he was rather than hiding him; the track is the answer.
    pub fn actor(&mut self, id: u32) -> Option<&PlayerActor> {
        let now = self.now;
        let entity = self.entity(id);
        let recorded = self
            .world
            .resource_mut::<ReplayTracks>()
            .players
            .get_mut(&id)
            .and_then(|track| track.position_at(now))
            .is_some();
        if !recorded || self.world.get::<Visibility>(entity) == Some(&Visibility::Hidden) {
            return None;
        }
        self.world.get::<PlayerActor>(entity)
    }

    pub fn position(&self, id: u32) -> Vec3 {
        self.world
            .get::<Transform>(self.entity(id))
            .map_or(Vec3::ZERO, |transform| transform.translation)
    }

    /// Where the ball is drawn, and whether there is one — off the ball the
    /// pipeline itself placed, every offset it draws the ball at included.
    pub fn ball(&self) -> Option<Vec3> {
        self.world
            .resource::<BallState>()
            .on_pitch
            .then(|| self.world.get::<Transform>(self.ball))
            .flatten()
            .map(|transform| transform.translation)
    }
}

/// The joints a census reads, and the family each one is reported under.
const JOINTS: [(&str, Limb, f32); 14] = [
    ("trunk", Limb::Torso, 0.0),
    ("trunk", Limb::Pelvis, 0.0),
    ("head", Limb::Head, 0.0),
    ("shoulder", Limb::Shoulder, -1.0),
    ("shoulder", Limb::Shoulder, 1.0),
    ("elbow", Limb::Elbow, -1.0),
    ("elbow", Limb::Elbow, 1.0),
    ("wrist", Limb::Wrist, -1.0),
    ("wrist", Limb::Wrist, 1.0),
    ("hip", Limb::Hip, -1.0),
    ("hip", Limb::Hip, 1.0),
    ("knee", Limb::Knee, -1.0),
    ("knee", Limb::Knee, 1.0),
    ("ankle", Limb::Ankle, 1.0),
];

fn origin(limb: Limb, side: f32) -> Vec3 {
    match limb {
        Limb::Torso | Limb::Pelvis => Vec3::new(0.0, Physique::HIP, 0.0),
        Limb::Head => Vec3::new(0.0, Physique::TORSO, 0.0),
        Limb::Shoulder => Vec3::new(side * Physique::SHOULDER_SPREAD, Physique::SHOULDER, 0.0),
        Limb::Elbow => Vec3::new(0.0, -Physique::UPPER_ARM, 0.0),
        Limb::Wrist => Vec3::new(0.0, -Physique::FOREARM - Physique::WRIST_DROP, 0.0),
        Limb::Hip => Vec3::new(side * Physique::HIP_SPREAD, Physique::HIP, 0.0),
        Limb::Knee => Vec3::new(0.0, -Physique::THIGH, 0.0),
        _ => Physique::ANKLE,
    }
}

/// What a man is doing on one frame, as far as the census buckets him.
fn doing(actor: &PlayerActor) -> &'static str {
    let gait = actor.pose;
    if gait.dive > 0.02 || gait.jump > 0.02 || gait.hop > 0.02 {
        "off his feet"
    } else if gait.power + gait.header + gait.throw_in + gait.throwing + gait.trap > 0.02 {
        "striking"
    } else if gait.save > 0.02 || gait.carry > 0.02 {
        "handling"
    } else if actor.speed > 1.5 {
        "running"
    } else {
        "standing"
    }
}

/// Every channel that is on, for naming what a spike came out of.
fn channels(actor: &PlayerActor) -> String {
    let gait = actor.pose;
    let named = [
        ("run", gait.run),
        ("power", gait.power),
        ("header", gait.header),
        ("throw_in", gait.throw_in),
        ("throwing", gait.throwing),
        ("trap", gait.trap),
        ("save", gait.save),
        ("set", gait.set),
        ("reach", gait.reach),
        ("dive", gait.dive),
        ("stretch", gait.stretch),
        ("grounded", gait.grounded),
        ("rising", gait.rising),
        ("land", gait.land),
        ("hop", gait.hop),
        ("jump", gait.jump),
        ("carry", gait.carry),
        ("carrying", gait.carrying),
        ("pivot", gait.pivot),
        ("urging", gait.urging),
        ("hips", gait.hands_on_hips),
        ("doubled", gait.doubled_over),
        ("despair", gait.despair),
        ("pointing", gait.pointing),
        ("head", gait.hands_to_head),
        ("elation", gait.elation),
        ("beaten", gait.beaten),
    ];
    let mut line = format!(
        "{:.1}m/s {:?} swing {:+.2}",
        actor.speed, actor.attitude, gait.swing
    );
    for (name, value) in named {
        if value.abs() > 0.02 {
            line.push_str(&format!(" {name} {value:.2}"));
        }
    }
    line
}

#[derive(Default)]
struct Spread {
    values: Vec<f32>,
}

impl Spread {
    fn note(&mut self, value: f32) {
        self.values.push(value);
    }

    fn line(&mut self) -> String {
        if self.values.is_empty() {
            return "—".into();
        }
        self.values.sort_by(f32::total_cmp);
        let at = |share: f64| self.values[((self.values.len() - 1) as f64 * share) as usize];
        format!(
            "{:7.1} {:7.1} {:7.1} {:8.1}",
            at(0.5),
            at(0.95),
            at(0.999),
            self.values[self.values.len() - 1]
        )
    }
}

struct Spike {
    at: f64,
    id: u32,
    joint: &'static str,
    side: f32,
    acceleration: f32,
    speed: f32,
    context: String,
}

/// **How smoothly the rig moves over a real match**, joint by joint: angular
/// speed and angular acceleration of every joint the eye follows, at sixty
/// frames a second, bucketed by what the man is doing — and the worst single
/// frames, named by the channels that were on, which is where a pop comes
/// from.
#[test]
#[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
fn census_motion() {
    let Some(mut rehearsal) = Rehearsal::open() else {
        panic!("set MATCH_REPLAY to a decompressed chunk");
    };
    let from: f64 = std::env::var("MATCH_AT")
        .ok()
        .and_then(|at| at.parse().ok())
        .unwrap_or(rehearsal.start + 5_000.0);
    let span: f64 = std::env::var("MATCH_SPAN")
        .ok()
        .and_then(|span| span.parse().ok())
        .unwrap_or(120.0);
    // `MATCH_STRIKES` lists the pops inside a strike too.
    let strikes = std::env::var("MATCH_STRIKES").is_ok();
    let until = (from + span * 1000.0).min(rehearsal.until);
    rehearsal.seek(from - 2_000.0);
    while rehearsal.now() < from {
        rehearsal.tick();
    }

    let dt = (Rehearsal::FRAME_MS / 1000.0) as f32;
    let ids: Vec<u32> = rehearsal.players().collect();
    let mut last: HashMap<(u32, usize), (Quat, Vec3)> = HashMap::new();
    let mut speeds: HashMap<(&str, &str), Spread> = HashMap::new();
    let mut accelerations: HashMap<(&str, &str), Spread> = HashMap::new();
    let mut spikes: Vec<Spike> = Vec::new();
    let mut frames = 0u64;
    while rehearsal.now() < until {
        rehearsal.tick();
        frames += 1;
        let now = rehearsal.now();
        for &id in &ids {
            let Some(actor) = rehearsal.actor(id) else {
                last.retain(|(each, _), _| *each != id);
                continue;
            };
            let bucket = doing(actor);
            let listed = bucket != "striking" || strikes;
            // A leg's own cycle turns over far harder than anything above
            // the waist does, so it gets its own bar for what a pop is.
            let pop = |family: &str| match family {
                "hip" | "knee" | "ankle" => 1_500.0,
                _ => 250.0,
            };
            for (index, (family, limb, side)) in JOINTS.into_iter().enumerate() {
                let rotation = step(limb, side, origin(limb, side), actor.pose).rotation;
                let key = (id, index);
                if let Some((was, spun)) = last.get(&key).copied() {
                    let turned = (was.inverse() * rotation).to_scaled_axis() / dt;
                    let jolt = (turned - spun).length() / dt;
                    if spun != Vec3::ZERO || turned != Vec3::ZERO {
                        speeds
                            .entry((family, bucket))
                            .or_default()
                            .note(turned.length());
                        accelerations
                            .entry((family, bucket))
                            .or_default()
                            .note(jolt);
                    }
                    if jolt > pop(family) && listed {
                        spikes.push(Spike {
                            at: now,
                            id,
                            joint: family,
                            side,
                            acceleration: jolt,
                            speed: turned.length(),
                            context: channels(actor),
                        });
                    }
                    last.insert(key, (rotation, turned));
                } else {
                    last.insert(key, (rotation, Vec3::ZERO));
                }
            }
        }
    }

    println!("{frames} frames from {from:.0} ms, {} players", ids.len());
    println!(
        "{:<10} {:<13} {:>31}   {:>32}",
        "joint", "doing", "rad/s  p50 p95 p99.9 max", "rad/s² p50 p95 p99.9 max"
    );
    let mut keys: Vec<(&str, &str)> = speeds.keys().copied().collect();
    keys.sort();
    for key in keys {
        let speed = speeds.get_mut(&key).unwrap().line();
        let acceleration = accelerations.get_mut(&key).unwrap().line();
        println!("{:<10} {:<13} {speed}   {acceleration}", key.0, key.1);
    }
    spikes.sort_by(|a, b| b.acceleration.total_cmp(&a.acceleration));
    println!(
        "{} joint-frames over the pop bar (250 rad/s² upper body, 1500 legs){}; worst:",
        spikes.len(),
        if strikes { "" } else { " outside a strike" }
    );
    let mut by_family: HashMap<&str, usize> = HashMap::new();
    for spike in &spikes {
        *by_family.entry(spike.joint).or_default() += 1;
    }
    println!("  by joint: {by_family:?}");
    let only: Option<String> = std::env::var("MATCH_JOINT").ok();
    for spike in spikes
        .iter()
        .filter(|spike| only.as_deref().is_none_or(|joint| joint == spike.joint))
        .take(40)
    {
        println!(
            "  {:>9.0} ms  #{:<4} {:<9} {:+.0}  {:7.0} rad/s²  {:5.1} rad/s  {}",
            spike.at,
            spike.id,
            spike.joint,
            spike.side,
            spike.acceleration,
            spike.speed,
            spike.context
        );
    }
}

/// Where one man's figure is in the world this frame: his position and
/// facing, then the carriage a dive or a leap puts him under.
fn placed(actor: &PlayerActor, position: Vec3) -> Transform {
    let (pitch, roll) = actor.topple();
    Transform::from_translation(Vec3::new(position.x, 0.0, position.z))
        .with_rotation(Quat::from_rotation_y(actor.heading))
        * Carriage::placed(pitch, roll, actor.lift())
}

/// A glove inside his own chest: the wrist, in the torso's frame, inside the
/// shirt's cross-section between the waist and the collar.
fn inside_the_chest(gait: Gait, side: f32) -> bool {
    let torso = step(Limb::Torso, 0.0, Vec3::new(0.0, Physique::HIP, 0.0), gait);
    let wrist = torso
        .to_matrix()
        .inverse()
        .transform_point3(Physique::glove(side, gait));
    (0.10..0.52).contains(&wrist.y) && (wrist.x / 0.16).powi(2) + (wrist.z / 0.10).powi(2) < 1.0
}

/// **Where his feet and hands are over a real match**: soles through the
/// turf, planted soles skating across it, a man on his feet with both soles
/// off it, and a glove inside his own chest — by what he is doing, with the
/// worst frames named.
#[test]
#[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
fn census_contact() {
    let Some(mut rehearsal) = Rehearsal::open() else {
        panic!("set MATCH_REPLAY to a decompressed chunk");
    };
    let from: f64 = std::env::var("MATCH_AT")
        .ok()
        .and_then(|at| at.parse().ok())
        .unwrap_or(rehearsal.start + 5_000.0);
    let span: f64 = std::env::var("MATCH_SPAN")
        .ok()
        .and_then(|span| span.parse().ok())
        .unwrap_or(120.0);
    let until = (from + span * 1000.0).min(rehearsal.until);
    rehearsal.seek(from - 2_000.0);
    while rehearsal.now() < from {
        rehearsal.tick();
    }
    let dt = (Rehearsal::FRAME_MS / 1000.0) as f32;
    let ids: Vec<u32> = rehearsal.players().collect();
    let mut planted: HashMap<(u32, i8), Vec3> = HashMap::new();
    let mut bodies: HashMap<u32, (Vec3, f32)> = HashMap::new();
    let mut jitter = Spread::default();
    let mut slips: HashMap<&str, Spread> = HashMap::new();
    let mut floating: HashMap<&str, Spread> = HashMap::new();
    let mut frames: HashMap<&str, u64> = HashMap::new();
    let mut buried: Vec<(f32, f64, u32, String)> = Vec::new();
    let mut skating: Vec<(f32, f64, u32, String)> = Vec::new();
    let mut hovering: Vec<(f32, f64, u32, String)> = Vec::new();
    let mut hugging: Vec<(f64, u32, String)> = Vec::new();
    // Planted slip by what moved the body under the sole, with the count of
    // those frames skating past 1.5 m/s.
    let mut causes: HashMap<&str, (Spread, u64)> = HashMap::new();
    while rehearsal.now() < until {
        rehearsal.tick();
        let now = rehearsal.now();
        for &id in &ids {
            let position = rehearsal.position(id);
            let Some(actor) = rehearsal.actor(id) else {
                planted.retain(|(each, _), _| *each != id);
                continue;
            };
            let gait = actor.pose;
            let bucket = doing(actor);
            *frames.entry(bucket).or_default() += 1;
            let world = placed(actor, position);
            // The body's own step and turn this frame, so a skating sole can
            // be told apart from a pose moving it.
            let (moved, turned) = bodies.get(&id).map_or((0.0, 0.0), |(was, heading)| {
                (
                    Vec2::new(position.x - was.x, position.z - was.z).length() / dt,
                    (actor.heading - heading).abs() / dt,
                )
            });
            bodies.insert(id, (position, actor.heading));
            // Faster than anybody runs is the recording moving him; a body
            // moving over legs that stand is the engine holding him on a
            // spot; then a turn; then a pace the legs have not caught yet.
            // What is left is the cycle itself.
            let cause = if moved > 12.0 {
                "hop"
            } else if actor.tread < Actors::STEPPING * 0.5 && moved > 0.3 {
                "dither"
            } else if turned > 2.0 {
                "turn"
            } else if (moved - actor.tread).abs() > 1.0 {
                "pace"
            } else {
                "gait"
            };
            // A man the legs say is standing still, and how fast his body
            // is being moved about by the recording anyway — short of a
            // teleport, which is cut to rather than drawn moving.
            if actor.tread < Actors::STEPPING * 0.5
                && actor.height <= Actors::AIRBORNE_FEET
                && moved <= Actors::TELEPORT
            {
                jitter.note(moved);
            }
            let soles = [-1.0f32, 1.0].map(|side| world.transform_point(boot(side, gait)));
            let lowest = soles[0].y.min(soles[1].y);
            if lowest < -0.03 {
                buried.push((-lowest, now, id, channels(actor)));
            }
            // On his feet as the recording has him: no height, no dive.
            if actor.height <= Actors::AIRBORNE_FEET && gait.dive < 0.02 {
                floating.entry(bucket).or_default().note(lowest.max(0.0));
                if lowest > 0.08 {
                    hovering.push((lowest, now, id, channels(actor)));
                }
            }
            for (index, sole) in soles.into_iter().enumerate() {
                let key = (id, index as i8);
                // The boot swinging through a ball brushes the grass at
                // contact; it is not a planted foot. On the grass is under
                // 2 cm: a standing man's boot rests 17 mm up.
                let striking = gait.power + gait.trap + gait.header > 0.02
                    && (index as f32 * 2.0 - 1.0) * gait.foot > 0.0;
                if sole.y < 0.02 && !striking {
                    if let Some(was) = planted.get(&key) {
                        let slip = Vec2::new(sole.x - was.x, sole.z - was.z).length() / dt;
                        slips.entry(bucket).or_default().note(slip);
                        let (by_cause, skated) = causes.entry(cause).or_default();
                        by_cause.note(slip);
                        if slip > 1.5 {
                            *skated += 1;
                            skating.push((
                                slip,
                                now,
                                id,
                                format!(
                                    "body {moved:.1} m/s turn {turned:.1} rad/s | {}",
                                    channels(actor)
                                ),
                            ));
                        }
                    }
                    planted.insert(key, sole);
                } else {
                    planted.remove(&key);
                }
            }
            for side in [-1.0f32, 1.0] {
                if inside_the_chest(gait, side) {
                    hugging.push((now, id, channels(actor)));
                }
            }
        }
    }
    println!("from {from:.0} ms for {span:.0} s");
    println!(
        "body speed while his legs stand, teleports left out, m/s p50 p95 p99.9 max: {}",
        jitter.line()
    );
    println!(
        "{:<13} {:>8} {:>34}   {:>34}",
        "doing", "frames", "planted slip m/s p50 p95 p99.9 max", "lower sole m p50 p95 p99.9 max"
    );
    let mut keys: Vec<&str> = frames.keys().copied().collect();
    keys.sort();
    for key in keys {
        let slip = slips.get_mut(key).map_or("—".into(), Spread::line);
        let float = floating.get_mut(key).map_or("—".into(), Spread::line);
        println!("{key:<13} {:>8} {slip:>34}   {float:>34}", frames[key]);
    }
    println!(
        "{:<13} {:>8} {:>9} {:>34}",
        "cause", "soles", "> 1.5", "planted slip m/s p50 p95 p99.9 max"
    );
    for cause in ["hop", "dither", "turn", "pace", "gait"] {
        if let Some((slip, skated)) = causes.get_mut(cause) {
            println!(
                "{cause:<13} {:>8} {skated:>9} {:>34}",
                slip.values.len(),
                slip.line()
            );
        }
    }
    let worst = |name: &str, list: &mut Vec<(f32, f64, u32, String)>| {
        list.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("{} {name}; worst:", list.len());
        for (value, at, id, context) in list.iter().take(12) {
            println!("  {at:>9.0} ms  #{id:<4} {value:6.2}  {context}");
        }
    };
    worst("frames with a sole over 3 cm under the turf", &mut buried);
    worst("planted-sole frames skating over 1.5 m/s", &mut skating);
    worst(
        "frames on his feet with both soles over 8 cm up",
        &mut hovering,
    );
    println!(
        "{} glove-frames inside his own chest; first:",
        hugging.len()
    );
    for (at, id, context) in hugging.iter().take(12) {
        println!("  {at:>9.0} ms  #{id:<4} {context}");
    }
}

/// **Does the man striking the ball meet it?** At the instant of every
/// contact the recording names — a kick, a trap, a header, a keeper's throw
/// and a throw-in — how far the part doing it is from where the ball is
/// drawn: the boot, the crown, the palm, the pair of hands.
#[test]
#[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
fn census_strikes() {
    let Some(mut rehearsal) = Rehearsal::open() else {
        panic!("set MATCH_REPLAY to a decompressed chunk");
    };
    let from: f64 = std::env::var("MATCH_AT")
        .ok()
        .and_then(|at| at.parse().ok())
        .unwrap_or(rehearsal.start + 5_000.0);
    let span: f64 = std::env::var("MATCH_SPAN")
        .ok()
        .and_then(|span| span.parse().ok())
        .unwrap_or(120.0);
    let until = (from + span * 1000.0).min(rehearsal.until);
    rehearsal.seek(from - 2_000.0);
    while rehearsal.now() < from {
        rehearsal.tick();
    }
    let ids: Vec<u32> = rehearsal.players().collect();
    let mut swings: HashMap<u32, f32> = HashMap::new();
    let mut kinds: HashMap<u32, Strike> = HashMap::new();
    let mut switches: HashMap<String, usize> = HashMap::new();
    let mut gaps: HashMap<&str, Spread> = HashMap::new();
    let mut misses: Vec<(f32, f64, u32, &str, String)> = Vec::new();
    while rehearsal.now() < until {
        rehearsal.tick();
        let now = rehearsal.now();
        let ball = rehearsal.ball();
        let recorded = rehearsal.world.resource::<BallState>().position.y;
        for &id in &ids {
            let position = rehearsal.position(id);
            let Some(actor) = rehearsal.actor(id) else {
                swings.remove(&id);
                continue;
            };
            let Some(kick) = actor.kick else {
                swings.remove(&id);
                kinds.remove(&id);
                continue;
            };
            // A swing under way handed to another set of limbs.
            if let (Some(kind), Some(swing)) = (kinds.insert(id, kick.kind), swings.get(&id))
                && kind != kick.kind
                && *swing < 0.0
            {
                *switches
                    .entry(format!("{kind:?} to {:?}", kick.kind))
                    .or_default() += 1;
                println!(
                    "  {now:>9.0} ms  #{id:<4} {kind:?} at {swing:+.2} handed to {:?}",
                    kick.kind
                );
            }
            let was = swings.insert(id, kick.swing);
            let (Some(was), Some(ball)) = (was, ball) else {
                continue;
            };
            if !(was < 0.0 && kick.swing >= 0.0) {
                continue;
            }
            let gait = actor.pose;
            let world = placed(actor, position);
            let side = if kick.foot < 0.0 { -1.0 } else { 1.0 };
            let (kind, part) = match kick.kind {
                Strike::Boot => ("boot", boot(side, gait) + Vec3::Y * Actors::BALL_RADIUS),
                Strike::Trap => ("trap", boot(side, gait) + Vec3::Y * Actors::BALL_RADIUS),
                Strike::Head => ("head", crown(gait)),
                Strike::Throw => ("throw", Physique::palm(side, gait)),
                Strike::ThrowIn => ("throw-in", Physique::hands(gait)),
            };
            let gap = world.transform_point(part).distance(ball);
            gaps.entry(kind).or_default().note(gap);
            if kick.kind == Strike::Boot {
                let band = match recorded {
                    height if height < 0.3 => "boot <0.3",
                    height if height < 0.7 => "boot <0.7",
                    height if height < 1.1 => "boot <1.1",
                    _ => "boot high",
                };
                gaps.entry(band).or_default().note(gap);
            }
            misses.push((gap, now, id, kind, channels(actor)));
        }
    }
    println!("from {from:.0} ms for {span:.0} s");
    println!(
        "{:<9} {:>6} {:>34}",
        "strike", "count", "gap to the ball m p50 p95 p99.9 max"
    );
    let mut kinds: Vec<&str> = gaps.keys().copied().collect();
    kinds.sort();
    for kind in kinds {
        let spread = gaps.get_mut(kind).unwrap();
        let count = spread.values.len();
        println!("{kind:<9} {count:>6} {:>34}", spread.line());
    }
    println!("swings handed to other limbs before contact: {switches:?}");
    misses.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!("worst:");
    for (gap, at, id, kind, context) in misses.iter().take(20) {
        println!("  {at:>9.0} ms  #{id:<4} {kind:<8} {gap:5.2} m  {context}");
    }
}

/// **The ball as it is drawn against the ball as it was recorded**: how much
/// harder the drawn ball changes its motion in a frame than the recorded one
/// does. A strike changes both alike; a ball handed between two men's claims
/// on it, or let go of all at once, jerks the drawn one alone. The worst
/// frames are named by who was on it either side.
#[test]
#[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
fn census_ball() {
    let Some(mut rehearsal) = Rehearsal::open() else {
        panic!("set MATCH_REPLAY to a decompressed chunk");
    };
    let from: f64 = std::env::var("MATCH_AT")
        .ok()
        .and_then(|at| at.parse().ok())
        .unwrap_or(rehearsal.start + 5_000.0);
    let span: f64 = std::env::var("MATCH_SPAN")
        .ok()
        .and_then(|span| span.parse().ok())
        .unwrap_or(120.0);
    let until = (from + span * 1000.0).min(rehearsal.until);
    rehearsal.seek(from - 2_000.0);
    while rehearsal.now() < from {
        rehearsal.tick();
    }
    let on = |state: &BallState| {
        let by = |impact: Option<Impact>| impact.map(|impact| impact.by);
        format!(
            "feet {:?} struck {:?} taken {:?}",
            state.led_by,
            by(state.impact),
            by(state.reception)
        )
    };
    // The last two frames' drawn and recorded ball, and who was on it.
    let mut was: Vec<(Vec3, Vec3, String)> = Vec::new();
    let mut excess = Spread::default();
    let mut underfoot = Spread::default();
    let mut worst: Vec<(f32, f64, String, String)> = Vec::new();
    while rehearsal.now() < until {
        rehearsal.tick();
        let (recorded, now_on) = {
            let state = rehearsal.world.resource::<BallState>();
            if state.led_by.is_some() && state.lead > 0.5 {
                underfoot.note(state.position.y * 100.0);
            }
            (state.position, on(state))
        };
        let Some(drawn) = rehearsal.ball() else {
            was.clear();
            continue;
        };
        let drawn = drawn - Vec3::Y * Actors::BALL_RADIUS;
        if let [(drawn_2, recorded_2, _), (drawn_1, recorded_1, before)] = was.as_slice() {
            let jerk = (drawn - 2.0 * *drawn_1 + *drawn_2).length();
            let own = (recorded - 2.0 * *recorded_1 + *recorded_2).length();
            // A teleport in the recording is a restart, drawn as one.
            if own < 1.0 {
                let more = (jerk - own).max(0.0);
                excess.note(more * 100.0);
                worst.push((more, rehearsal.now(), before.clone(), now_on.clone()));
            }
        }
        if was.len() == 2 {
            was.remove(0);
        }
        was.push((drawn, recorded, now_on));
    }
    let frames = excess.values.len();
    let over = |cm: f32| excess.values.iter().filter(|&&more| more > cm).count();
    let (five, fifteen, forty) = (over(5.0), over(15.0), over(40.0));
    println!("from {from:.0} ms for {span:.0} s, {frames} frames");
    println!(
        "jerk beyond the recording's, cm  p50 p95 p99.9 max: {}",
        excess.line()
    );
    println!("frames over 5 cm {five}, over 15 cm {fifteen}, over 40 cm {forty}");
    println!(
        "recorded height of a ball drawn at his feet, cm: {}",
        underfoot.line()
    );
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!("worst:");
    for (more, at, before, after) in worst.iter().take(15) {
        println!("  {at:>9.0} ms  {more:5.2} m  {before}  ->  {after}");
    }
}

/// **One man, frame by frame**: the state a pop in [`census_motion`] came
/// out of, `MATCH_FRAMES` frames either side of `MATCH_AT`.
#[test]
#[ignore = "needs MATCH_REPLAY, MATCH_PLAYER and MATCH_AT"]
fn trace_rehearsal() {
    let read = |name: &str| std::env::var(name).ok();
    let id: u32 = read("MATCH_PLAYER")
        .and_then(|id| id.parse().ok())
        .expect("MATCH_PLAYER");
    let at: f64 = read("MATCH_AT")
        .and_then(|at| at.parse().ok())
        .expect("MATCH_AT, in ms");
    let frames: f64 = read("MATCH_FRAMES")
        .and_then(|n| n.parse().ok())
        .unwrap_or(8.0);
    let Some(mut rehearsal) = Rehearsal::open() else {
        panic!("set MATCH_REPLAY to a decompressed chunk");
    };
    // `MATCH_FROM` replays from where a census started, so the frame it
    // named is reached with the same history behind it.
    let from: f64 = read("MATCH_FROM")
        .and_then(|from| from.parse().ok())
        .unwrap_or(at - frames * Rehearsal::FRAME_MS - 2_000.0);
    rehearsal.seek(from);
    while rehearsal.now() < at - frames * Rehearsal::FRAME_MS {
        rehearsal.tick();
    }
    let hips = Vec3::new(0.0, Physique::HIP, 0.0);
    while rehearsal.now() < at + frames * Rehearsal::FRAME_MS {
        rehearsal.tick();
        let now = rehearsal.now();
        let position = rehearsal.position(id);
        let drawn = rehearsal.ball();
        let (recorded, led, held) = {
            let state = rehearsal.world.resource::<BallState>();
            (
                state.position,
                state.led_by,
                (state.held_by, state.cradle, state.lead),
            )
        };
        let Some(actor) = rehearsal.actor(id) else {
            continue;
        };
        let gait = actor.pose;
        let (yaw, pitch, roll) = step(Limb::Torso, 0.0, hips, gait)
            .rotation
            .to_euler(EulerRot::YXZ);
        let (hip_yaw, hip_pitch, _) = step(Limb::Hip, 1.0, Vec3::ZERO, gait)
            .rotation
            .to_euler(EulerRot::YXZ);
        println!(
            "{now:>9.0} speed {:.2} tread {:.2} heading {:+.2} course ({:+.2},{:+.2}) open {:+.2} \
             underfoot ({:+.2},{:+.2}) phase {:.2} run {:.2} pivot {:+.2} stance {:.2} land {:.2} \
             | torso y{:+.2} p{:+.2} r{:+.2} | hip y{:+.2} p{:+.2} | {}",
            actor.speed,
            actor.tread,
            actor.heading,
            actor.course.x,
            actor.course.y,
            actor.open,
            actor.underfoot.x,
            actor.underfoot.y,
            actor.phase,
            gait.run,
            actor.pivot,
            actor.stance,
            gait.land,
            yaw,
            pitch,
            roll,
            hip_yaw,
            hip_pitch,
            channels(actor),
        );
        let knee = |side: f32| {
            step(Limb::Knee, side, origin(Limb::Knee, side), gait)
                .rotation
                .to_euler(EulerRot::XYZ)
                .0
        };
        println!(
            "          push {:.2} load {:.2} takeoff {:?} air {:.3} flat {:.2} jump {:.2} crouched set {:.2} | knees {:+.2} {:+.2}",
            gait.push,
            actor.loading(),
            actor.takeoff.map(|takeoff| takeoff.delay),
            actor.air,
            actor.flat,
            gait.jump,
            gait.set,
            knee(-1.0),
            knee(1.0),
        );
        let meeting = actor.meeting().map(|claim| {
            let placed = Transform::from_translation(Vec3::new(position.x, 0.0, position.z))
                .with_rotation(Quat::from_rotation_y(actor.heading));
            claim.placed(&placed)
        });
        println!(
            "          ball drawn {:?} recorded {:?} led by {:?} held {:?} | kick {:?} | meeting {:?}",
            drawn.map(|ball| ball - position),
            recorded - position,
            led,
            held,
            actor.kick.map(|kick| (kick.kind, kick.swing, kick.blend)),
            meeting.map(|claim| (claim.point - position, claim.approach, claim.due)),
        );
        let shoulder =
            |side: f32| step(Limb::Shoulder, side, origin(Limb::Shoulder, side), gait).rotation;
        println!(
            "          lead {:+.2} smother {:.2} stretch {:.2} reach {:.2} | shoulders {:?} {:?}",
            gait.lead,
            gait.smother,
            gait.stretch,
            gait.reach,
            shoulder(-1.0),
            shoulder(1.0),
        );
    }
}

/// **A strip of one player as the viewer draws him**, off the whole pipeline:
/// `MATCH_COLUMNS` figures `MATCH_EVERY` frames apart from `MATCH_AT`, two
/// rows from two camera bearings held still in the WORLD, so a turn is seen
/// as a turn. The ball is drawn where it is drawn in the viewer.
#[test]
#[ignore = "needs MATCH_REPLAY, MATCH_PLAYER, MATCH_AT and MATCH_FIGURE_DUMP"]
fn dump_rehearsal() {
    const BOTTOM: f32 = -0.08;
    const TOP: f32 = 2.6;
    let read = |name: &str| std::env::var(name).ok();
    // `MATCH_GRAIN` cuts him at another grain and `MATCH_PIXELS` draws him that
    // many pixels tall, so two neighbouring grains can be compared at the
    // height a man is cut over from one to the other.
    let grain = Grain::of(Tier::Multisampled, read("MATCH_GRAIN").as_deref());
    let pixels: f32 = read("MATCH_PIXELS")
        .and_then(|pixels| pixels.parse().ok())
        .unwrap_or(240.0);
    let tall = (pixels * (TOP - BOTTOM) / Physique::STATURE).round() as usize;
    let wide = tall * 2 / 3;
    let directory = std::path::PathBuf::from(read("MATCH_FIGURE_DUMP").expect("MATCH_FIGURE_DUMP"));
    let id: u32 = read("MATCH_PLAYER")
        .and_then(|id| id.parse().ok())
        .expect("MATCH_PLAYER");
    let at: f64 = read("MATCH_AT")
        .and_then(|at| at.parse().ok())
        .expect("MATCH_AT, in ms");
    let every: usize = read("MATCH_EVERY")
        .and_then(|n| n.parse().ok())
        .unwrap_or(4);
    let columns: usize = read("MATCH_COLUMNS")
        .and_then(|n| n.parse().ok())
        .unwrap_or(16);
    let bearing: f32 = read("MATCH_BEARING")
        .and_then(|b| b.parse().ok())
        .unwrap_or(0.0);
    let Some(mut rehearsal) = Rehearsal::open() else {
        panic!("set MATCH_REPLAY to a decompressed chunk");
    };
    let mut meshes = Assets::<Mesh>::default();
    let parts = BodyParts::tailor(&mut meshes, grain);
    rehearsal.seek(at - 2_000.0);
    while rehearsal.now() < at {
        rehearsal.tick();
    }
    let rows = [bearing, bearing + FRAC_PI_2];
    let mut sheet = vec![0u8; wide * columns * tall * rows.len() * 4];
    let mut report = String::from(
        "ms,speed,height,dive,stretch,grounded,rising,reach,save,set,land,power,swing,jump,flat,lead,smother,push,thud\n",
    );
    for column in 0..columns {
        for _ in 0..every {
            rehearsal.tick();
        }
        let since = rehearsal.now() - at;
        let from = rehearsal.position(id);
        let ball_at = rehearsal
            .ball()
            .map(|ball| Vec3::new(ball.x - from.x, ball.y, ball.z - from.z))
            .filter(|offset| Vec2::new(offset.x, offset.z).length() < 4.0);
        let Some(actor) = rehearsal.actor(id) else {
            continue;
        };
        let gait = actor.pose;
        let (pitch, roll) = actor.topple();
        let facing = Transform::from_rotation(Quat::from_rotation_y(actor.heading));
        let carriage = facing * Carriage::placed(pitch, roll, actor.lift());
        report.push_str(&format!(
            "{:.0},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:+.2},{:.2},{:.2},{:+.2},{:.2},{:.2},{:+.2}\n",
            since,
            actor.speed,
            actor.height,
            gait.dive,
            gait.stretch,
            gait.grounded,
            gait.rising,
            gait.reach,
            gait.save,
            gait.set,
            gait.land,
            gait.power,
            gait.swing,
            gait.jump,
            actor.flat,
            gait.lead,
            gait.smother,
            gait.push,
            gait.thud,
        ));
        for (row, bearing) in rows.into_iter().enumerate() {
            let lens = Lens {
                bearing,
                bottom: BOTTOM,
                top: TOP,
            };
            let mut canvas = Canvas::new(wide, tall);
            posed(
                &mut canvas,
                &lens,
                &meshes,
                &parts,
                gait,
                carriage,
                actor.is_goalkeeper,
            );
            if let Some(offset) = ball_at {
                ball(&mut canvas, &lens, offset);
            }
            let pixels = canvas.pixels();
            for line in 0..tall {
                let from = line * wide * 4;
                let to = ((row * tall + line) * wide * columns + column * wide) * 4;
                sheet[to..to + wide * 4].copy_from_slice(&pixels[from..from + wide * 4]);
            }
        }
    }
    std::fs::write(directory.join("rehearsal.rgba"), sheet).expect("wrote the sheet");
    std::fs::write(directory.join("rehearsal.csv"), report).expect("wrote the report");
    println!("{}x{}", wide * columns, tall * rows.len());
}

/// What each system cost, frame by frame, over one stretch of a run.
struct Laps {
    stages: Vec<&'static str>,
    cost: Vec<Spread>,
    total: Spread,
    changed: Spread,
}

impl Laps {
    fn new(rehearsal: &Rehearsal) -> Laps {
        let stages: Vec<&'static str> = rehearsal.stages().collect();
        Laps {
            cost: stages.iter().map(|_| Spread::default()).collect(),
            stages,
            total: Spread::default(),
            changed: Spread::default(),
        }
    }

    fn note(&mut self, rehearsal: &Rehearsal) {
        for (spread, lap) in self.cost.iter_mut().zip(rehearsal.laps()) {
            spread.note(*lap);
        }
        self.total.note(rehearsal.laps().iter().sum());
        self.changed.note(rehearsal.changed() as f32);
    }

    fn print(&mut self, title: &str) {
        println!(
            "{title} ({} frames)            µs p50     p95   p99.9      max",
            self.total.values.len()
        );
        for (stage, spread) in self.stages.iter().zip(self.cost.iter_mut()) {
            println!("  {stage:<26} {}", spread.line());
        }
        println!("  {:<26} {}", "frame", self.total.line());
        println!(
            "  {:<26} {}",
            "GlobalTransforms changed",
            self.changed.line()
        );
    }
}

/// **What a frame of the squad costs the CPU**, system by system, over the
/// real bodies: the playhead, the posing, the carriage and Bevy's transform
/// propagation — playing, on the frame of a seek and the frames after it, and
/// paused. Also how many `GlobalTransform`s each frame dirties, and a
/// fingerprint of every pose played, so a change that claims to leave the
/// figures alone can prove it.
///
/// Native, so it is an A/B instrument rather than a figure for the browser:
/// run both sides on the same machine and compare.
///
/// ```text
/// MATCH_REPLAY=<chunk.json> [MATCH_AT=<ms>] [MATCH_SPAN=<s>] [MATCH_GRAIN=spare] cargo test --release --lib bench_frame -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
fn bench_frame() {
    let grain = Grain::of(
        Tier::Multisampled,
        std::env::var("MATCH_GRAIN").ok().as_deref(),
    );
    let Some(mut rehearsal) = Rehearsal::open() else {
        panic!("set MATCH_REPLAY to a decompressed chunk");
    };
    let from: f64 = std::env::var("MATCH_AT")
        .ok()
        .and_then(|at| at.parse().ok())
        .unwrap_or(rehearsal.start + 5_000.0);
    let span: f64 = std::env::var("MATCH_SPAN")
        .ok()
        .and_then(|span| span.parse().ok())
        .unwrap_or(60.0);
    let until = (from + span * 1000.0).min(rehearsal.until);
    rehearsal.dress(grain);
    rehearsal.seek(from - 2_000.0);
    while rehearsal.now() < from {
        rehearsal.tick();
    }

    let mut playing = Laps::new(&rehearsal);
    let mut fingerprint = DefaultHasher::new();
    while rehearsal.now() < until {
        rehearsal.tick();
        playing.note(&rehearsal);
        rehearsal.fingerprint(&mut fingerprint);
    }

    let mut seeking = Laps::new(&rehearsal);
    let mut after = Laps::new(&rehearsal);
    let mut at = from;
    while at < until {
        rehearsal.seek(at);
        seeking.note(&rehearsal);
        for _ in 0..5 {
            rehearsal.tick();
            after.note(&rehearsal);
        }
        at += 10_000.0;
    }

    let mut paused = Laps::new(&rehearsal);
    for _ in 0..120 {
        rehearsal.hold();
        paused.note(&rehearsal);
    }

    println!(
        "grain {}, {} players, {} joints, from {from:.0} ms for {:.0} s",
        grain.describe(),
        rehearsal.players().count(),
        rehearsal.joints(),
        (until - from) / 1000.0
    );
    playing.print("playing");
    seeking.print("seek frame");
    after.print("five frames after a seek");
    paused.print("paused");
    println!("pose fingerprint {:016x}", fingerprint.finish());
}

/// An angle folded into a half turn either way.
fn folded(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

/// The angle between two flat directions, in degrees; `None` if either has
/// no length to speak of.
fn apart(a: Vec2, b: Vec2) -> Option<f32> {
    let (a, b) = (a.try_normalize()?, b.try_normalize()?);
    Some(a.dot(b).clamp(-1.0, 1.0).acos().to_degrees())
}

/// The window a census reads: `MATCH_AT` (or five seconds in) for
/// `MATCH_SPAN` seconds (or `span`), after two seconds of play so every
/// filter has settled.
fn window(rehearsal: &mut Rehearsal, span: f64) -> (f64, f64) {
    let from: f64 = std::env::var("MATCH_AT")
        .ok()
        .and_then(|at| at.parse().ok())
        .unwrap_or(rehearsal.start + 5_000.0);
    let span: f64 = std::env::var("MATCH_SPAN")
        .ok()
        .and_then(|span| span.parse().ok())
        .unwrap_or(span);
    let until = (from + span * 1000.0).min(rehearsal.until);
    rehearsal.seek(from - 2_000.0);
    while rehearsal.now() < from {
        rehearsal.tick();
    }
    (from, until)
}

/// **Which way the outfielders face while they move**: the angle between the
/// chest and the ball by pace and by where the run points against the ball;
/// how often a man's facing swings round by more than a right angle inside
/// half a second; how fast he turns; and how far off his own facing his legs
/// travel. Keepers are left out, since they square to the ball by design.
#[test]
#[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
fn census_facing() {
    const PACES: [(&str, f32); 5] = [
        ("< 1.1", 1.1),
        ("1.1-2", 2.0),
        ("2-3", 3.0),
        ("3-5", 5.0),
        ("5+", f32::INFINITY),
    ];
    const HALF_SECOND: usize = 30;
    let Some(mut rehearsal) = Rehearsal::open() else {
        panic!("set MATCH_REPLAY to a decompressed chunk");
    };
    let (from, until) = window(&mut rehearsal, 120.0);
    let dt = (Rehearsal::FRAME_MS / 1000.0) as f32;
    let ids: Vec<u32> = rehearsal.players().collect();
    // [pace][toward, across, away] = (frames, back to the ball, sum of degrees)
    let mut cells = [[(0u64, 0u64, 0.0f64); 3]; 5];
    let mut retreating = (0u64, 0u64);
    let mut headings: HashMap<u32, Vec<f32>> = HashMap::new();
    let mut cooling: HashMap<u32, usize> = HashMap::new();
    let mut reversals = 0u64;
    let mut outfield = 0u64;
    let mut rates = Spread::default();
    let mut spun = 0u64;
    let mut off_facing = [0u64; 4];
    let mut moving = 0u64;
    while rehearsal.now() < until {
        rehearsal.tick();
        let ball = rehearsal.ball();
        for &id in &ids {
            let position = rehearsal.position(id);
            let Some(actor) = rehearsal.actor(id) else {
                headings.remove(&id);
                continue;
            };
            if actor.is_goalkeeper {
                continue;
            }
            outfield += 1;
            let heading = actor.heading;
            let past = headings.entry(id).or_default();
            if let Some(&was) = past.last() {
                let rate = folded(heading - was).abs().to_degrees() / dt;
                rates.note(rate);
                if rate > 360.0 {
                    spun += 1;
                }
            }
            past.push(heading);
            if past.len() > HALF_SECOND {
                past.remove(0);
            }
            let cool = cooling.entry(id).or_default();
            if *cool > 0 {
                *cool -= 1;
            } else if past.len() == HALF_SECOND && folded(heading - past[0]).abs() > FRAC_PI_2 {
                reversals += 1;
                *cool = HALF_SECOND;
            }
            if actor.speed > Actors::MOVING {
                moving += 1;
                let off = actor.course.x.atan2(actor.course.y).abs().to_degrees();
                let band = [30.0, 60.0, 100.0]
                    .iter()
                    .take_while(|&&edge| off >= edge)
                    .count();
                off_facing[band] += 1;
            }
            let Some(ball) = ball else {
                continue;
            };
            let to_ball = Vec2::new(ball.x - position.x, ball.z - position.z);
            let travel = Vec2::new(actor.travel.x, actor.travel.z);
            if to_ball.length() < 0.5 || travel.length() < 0.3 {
                continue;
            }
            let chest = Vec2::new(heading.sin(), heading.cos());
            let (Some(to_chest), Some(run)) = (apart(chest, to_ball), apart(travel, to_ball))
            else {
                continue;
            };
            let pace = PACES
                .iter()
                .position(|(_, top)| actor.speed < *top)
                .unwrap_or(PACES.len() - 1);
            let way = [60.0, 120.0]
                .iter()
                .take_while(|&&edge| run >= edge)
                .count();
            let cell = &mut cells[pace][way];
            cell.0 += 1;
            cell.1 += u64::from(to_chest > 120.0);
            cell.2 += f64::from(to_chest);
            if way == 2 && actor.speed > 0.5 && actor.speed < 3.0 {
                retreating.0 += 1;
                retreating.1 += u64::from(to_chest > 120.0);
            }
        }
    }
    let share = |part: u64, whole: u64| 100.0 * part as f64 / whole.max(1) as f64;
    println!(
        "from {from:.0} ms for {:.0} s, {outfield} outfield player-frames",
        (until - from) / 1000.0
    );
    println!("chest to ball by pace and run: frames, back turned (>120°) %, mean °");
    println!(
        "{:<8} {:>26} {:>26} {:>26}",
        "m/s", "toward (<60°)", "across (60-120°)", "away (>120°)"
    );
    for (row, (name, _)) in PACES.iter().enumerate() {
        let line: Vec<String> = cells[row]
            .iter()
            .map(|(frames, back, sum)| {
                format!(
                    "{frames:>8} {:>7.1}% {:>7.0}°",
                    share(*back, *frames),
                    sum / (*frames).max(1) as f64
                )
            })
            .collect();
        println!("{name:<8} {}", line.join("  "));
    }
    println!(
        "retreating at 0.5-3 m/s: {} frames, {:.1}% with the back more than 120° to the ball",
        retreating.0,
        share(retreating.1, retreating.0)
    );
    println!(
        "facing reversals past 90° inside half a second: {reversals}, {:.2} per player-minute",
        reversals as f64 / (outfield as f64 / 3600.0).max(1e-9)
    );
    println!(
        "turn rate °/s p50 p95 p99.9 max: {}; {:.2}% of frames past 360°/s",
        rates.line(),
        share(spun, outfield)
    );
    println!(
        "moving past {:.1} m/s, legs off the facing: <30° {:.1}%, 30-60° {:.1}%, 60-100° {:.1}%, >100° {:.1}%",
        Actors::MOVING,
        share(off_facing[0], moving),
        share(off_facing[1], moving),
        share(off_facing[2], moving),
        share(off_facing[3], moving)
    );
}

/// **Whether a cut lands in stride**: every ten seconds the playhead is cut to
/// an instant that a second run reaches by playing straight through, and each
/// man's pace, heading and lean are compared on the frame of the cut and the
/// ten after it.
#[test]
#[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
fn census_cut() {
    const AFTER: usize = 10;
    let (Some(mut through), Some(mut cut)) = (Rehearsal::open(), Rehearsal::open()) else {
        panic!("set MATCH_REPLAY to a decompressed chunk");
    };
    let (from, until) = window(&mut through, 120.0);
    let ids: Vec<u32> = through.players().collect();
    // [frame after the cut] = [pace m/s, heading °, drive ×100, bank ×100]
    let mut errors: Vec<[Spread; 4]> = (0..=AFTER).map(|_| Default::default()).collect();
    let mut cuts = 0;
    while through.now() + (AFTER as f64 + 1.0) * Rehearsal::FRAME_MS < until {
        let at = through.now();
        cut.seek(at);
        for (frame, error) in errors.iter_mut().enumerate() {
            if frame > 0 {
                through.tick();
                cut.tick();
            }
            for &id in &ids {
                let (Some(played), Some(jumped)) = (through.actor(id), cut.actor(id)) else {
                    continue;
                };
                error[0].note((played.speed - jumped.speed).abs());
                error[1].note(folded(played.heading - jumped.heading).abs().to_degrees());
                error[2].note((played.drive - jumped.drive).abs() * 100.0);
                error[3].note((played.turn - jumped.turn).abs() * 100.0);
            }
        }
        cuts += 1;
        let next = at + 10_000.0;
        while through.now() < next {
            through.tick();
        }
    }
    println!("{cuts} cuts from {from:.0} ms; error against playing through, p50 p95 p99.9 max");
    for frame in [0, 1, 2, 5, AFTER] {
        let [pace, heading, drive, bank] = &mut errors[frame];
        println!("frame {frame:>2}");
        println!("  pace m/s   {}", pace.line());
        println!("  heading °  {}", heading.line());
        println!("  drive ×100 {}", drive.line());
        println!("  bank ×100  {}", bank.line());
    }
}

/// **Whether a paused replay holds still**: plays two seconds, pauses for two,
/// and counts every joint that moves while paused; then compares the first
/// frame after it with an ordinary frame of play.
#[test]
#[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
fn census_pause() {
    const HOLD: usize = 120;
    let Some(mut rehearsal) = Rehearsal::open() else {
        panic!("set MATCH_REPLAY to a decompressed chunk");
    };
    rehearsal.dress(Grain::SPARE);
    window(&mut rehearsal, 0.0);
    // Zero only when the rotation is the same bits: `angle_between` of a
    // quaternion and itself rounds to a few hundredths of a degree.
    let turned = |before: &HashMap<Entity, Quat>, after: &HashMap<Entity, Quat>| {
        after
            .iter()
            .filter_map(|(joint, now)| {
                before.get(joint).map(|was| {
                    if was == now {
                        0.0
                    } else {
                        was.angle_between(*now).to_degrees().max(f32::MIN_POSITIVE)
                    }
                })
            })
            .collect::<Vec<f32>>()
    };
    let mut ordinary = Spread::default();
    let mut before = rehearsal.joint_rotations();
    for _ in 0..HOLD {
        rehearsal.tick();
        let after = rehearsal.joint_rotations();
        for angle in turned(&before, &after) {
            ordinary.note(angle);
        }
        before = after;
    }
    let mut paused = Spread::default();
    let mut moving_frames = 0;
    let mut changed = 0;
    for _ in 0..HOLD {
        rehearsal.hold();
        changed += rehearsal.changed();
        let after = rehearsal.joint_rotations();
        let moved: Vec<f32> = turned(&before, &after)
            .into_iter()
            .filter(|angle| *angle > 0.0)
            .collect();
        moving_frames += usize::from(!moved.is_empty());
        for angle in moved {
            paused.note(angle);
        }
        before = after;
    }
    rehearsal.tick();
    let mut resumed = Spread::default();
    for angle in turned(&before, &rehearsal.joint_rotations()) {
        resumed.note(angle);
    }
    println!("joint turn between two frames, ° p50 p95 p99.9 max");
    println!("  playing             {}", ordinary.line());
    println!(
        "  paused              {}  ({moving_frames} of {HOLD} frames moved a joint, {} joint moves, {changed} GlobalTransforms changed)",
        paused.line(),
        paused.values.len()
    );
    println!("  first frame after   {}", resumed.line());
}

/// **How far the legs come round between two drawn frames**, at each playback
/// speed: the advance the gait asks for, and what the eye sees. The second is
/// the change of phase folded into a half turn either way, so a leg drawn
/// wheeling backwards while its man runs forwards reads as negative.
#[test]
#[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
fn census_stride() {
    let frames = 30 * 60;
    println!("legs per drawn frame at 60 fps, ° of cycle p50 p95 p99.9 max, men moving past 1 m/s");
    for speed in [1.0f32, 2.0, 4.0, 8.0, 16.0] {
        let Some(mut rehearsal) = Rehearsal::open() else {
            panic!("set MATCH_REPLAY to a decompressed chunk");
        };
        rehearsal.at_speed(speed);
        window(&mut rehearsal, 0.0);
        let dt = (Rehearsal::FRAME_MS / 1000.0) as f32 * speed;
        let ids: Vec<u32> = rehearsal.players().collect();
        let mut phases: HashMap<u32, f32> = HashMap::new();
        let mut asked = Spread::default();
        let mut seen = Spread::default();
        let mut backwards = 0u64;
        let mut past_quarter = 0u64;
        for _ in 0..frames {
            if rehearsal.now() >= rehearsal.until {
                break;
            }
            rehearsal.tick();
            for &id in &ids {
                let Some(actor) = rehearsal.actor(id) else {
                    phases.remove(&id);
                    continue;
                };
                let was = phases.insert(id, actor.phase);
                if actor.speed <= 1.0 {
                    continue;
                }
                let (stride, _) = Actors::stride_of(actor.id, actor.speed, actor.underfoot);
                asked.note((actor.tread * dt * PI / stride).to_degrees());
                if let Some(was) = was {
                    let step = folded(actor.phase - was);
                    seen.note(step.to_degrees());
                    backwards += u64::from(step < 0.0);
                    past_quarter += u64::from(step.abs() > FRAC_PI_2);
                }
            }
        }
        let moving = seen.values.len().max(1) as f64;
        println!("{speed:>4}x  asked {}", asked.line());
        println!(
            "       seen  {}  ({:.1}% backwards, {:.1}% past a quarter turn)",
            seen.line(),
            100.0 * backwards as f64 / moving,
            100.0 * past_quarter as f64 / moving
        );
    }
}
