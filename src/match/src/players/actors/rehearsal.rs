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

use super::replayed::Chunk;
use super::*;
use crate::players::body::preview::{Canvas, Lens, ball, posed};
use crate::players::body::skeleton::{boot, crown, step};
use crate::players::body::{BodyParts, Limb};
use bevy::ecs::system::SystemId;
use std::collections::HashMap;
use std::time::Duration;

/// One recording, one world, the real systems in the order the app runs them.
pub(crate) struct Rehearsal {
    world: World,
    ball: Entity,
    players: Vec<(u32, Entity)>,
    systems: [SystemId; 3],
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
        let systems = [
            world.register_system(Actors::follow_playhead),
            world.register_system(Actors::animate),
            world.register_system(Playback::end_frame),
        ];
        Some(Rehearsal {
            world,
            ball,
            players,
            systems,
            now: start,
            start,
            until,
        })
    }

    /// Cuts to `at` the way a scrub does, so every filter starts from rest.
    pub fn seek(&mut self, at: f64) {
        self.now = at;
        let mut playback = self.world.resource_mut::<Playback>();
        playback.time_ms = at;
        playback.seeked = true;
        self.run();
    }

    /// One frame of play.
    pub fn tick(&mut self) {
        self.now += Self::FRAME_MS;
        self.world.resource_mut::<Playback>().time_ms = self.now;
        self.run();
    }

    fn run(&mut self) {
        for system in self.systems {
            self.world.run_system(system).expect("a pipeline system");
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
            // A man the legs say is standing still, and how fast his body
            // is being moved about by the recording anyway.
            if actor.tread < Actors::STEPPING * 0.5 && actor.height <= Actors::AIRBORNE_FEET {
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
                // contact; it is not a planted foot.
                let striking = gait.power + gait.trap + gait.header > 0.02
                    && (index as f32 * 2.0 - 1.0) * gait.foot > 0.0;
                if sole.y < 0.015 && !striking {
                    if let Some(was) = planted.get(&key) {
                        let slip = Vec2::new(sole.x - was.x, sole.z - was.z).length() / dt;
                        slips.entry(bucket).or_default().note(slip);
                        if slip > 1.5 {
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
        "body speed while his legs stand, m/s p50 p95 p99.9 max: {}",
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
    const WIDE: usize = 240;
    const TALL: usize = 360;
    let read = |name: &str| std::env::var(name).ok();
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
    let parts = BodyParts::tailor(&mut meshes, Grain::FULL);
    rehearsal.seek(at - 2_000.0);
    while rehearsal.now() < at {
        rehearsal.tick();
    }
    let rows = [bearing, bearing + FRAC_PI_2];
    let mut sheet = vec![0u8; WIDE * columns * TALL * rows.len() * 4];
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
                bottom: -0.08,
                top: 2.6,
            };
            let mut canvas = Canvas::new(WIDE, TALL);
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
            for line in 0..TALL {
                let from = line * WIDE * 4;
                let to = ((row * TALL + line) * WIDE * columns + column * WIDE) * 4;
                sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
            }
        }
    }
    std::fs::write(directory.join("rehearsal.rgba"), sheet).expect("wrote the sheet");
    std::fs::write(directory.join("rehearsal.csv"), report).expect("wrote the report");
    println!("{}x{}", WIDE * columns, TALL * rows.len());
}
