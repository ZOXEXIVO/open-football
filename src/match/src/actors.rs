use crate::body::{BodyParts, Footballer, Gait, Joint, Physique};
use crate::config::{PlayerInfo, ViewerConfig};
use crate::field::Field;
use crate::kit::{Complexion, Wardrobe};
use crate::loader::ChunkLoader;
use crate::playback::Playback;
use crate::replay::ReplayTracks;
use crate::textures::Textures;
use crate::timeline::DebugOverlay;
use bevy::prelude::*;
use std::f32::consts::{PI, TAU};

/// A player on the pitch: the root of one footballer's rig, carrying where they
/// are, which way they are facing and where they have got to in their stride.
/// The name plate that tracks them is drawn as a UI node (see [`PlayerLabel`]).
#[derive(Component)]
pub struct PlayerActor {
    pub id: u32,
    /// Where this player stood last frame. How they are moving — heading,
    /// stride, effort — is read back out of the replay from this, which is what
    /// keeps the animation in step with the playhead however it is being
    /// driven: playing, scrubbed or run at 8x.
    previous: Option<Vec3>,
    /// Smoothed facing, in radians about the world Y axis.
    heading: f32,
    /// Position in the run cycle, in radians. Advanced by ground covered rather
    /// than by time, so the feet keep pace with the turf at any playback speed.
    phase: f32,
    /// Smoothed ground speed in metres per second of *match* time.
    speed: f32,
    /// Direction this player last struck the ball in, and how long he stays
    /// turned that way. Nobody passes or shoots across their own body without
    /// opening up to the ball first; before this a player kicking sideways
    /// while running stayed square to his RUN, so the ball left at an angle
    /// his body never acknowledged.
    strike: Option<(Vec3, f32)>,
    /// A slow clock-driven cycle for the movement a player makes when he is
    /// NOT running — breathing, shifting his weight, letting his arms drift.
    idle: f32,
    /// Smoothed turn rate, −1..1, so the body can bank into a change of
    /// direction instead of pivoting on the spot.
    turn: f32,
    /// Smoothed yaw from his facing to the ball: where he is looking.
    look: f32,
    /// Whether this is a goalkeeper. Only a keeper ever takes the ball in his
    /// hands, so only a keeper is a candidate for the cradle.
    is_goalkeeper: bool,
    /// How far into the hold he is, 0..1. See [`Gait::carry`].
    carry: f32,
}

/// The name plate for one player, positioned each frame from the rig's
/// projected screen position.
#[derive(Component)]
pub struct PlayerLabel {
    pub actor: Entity,
}

/// The engine-state line under a name plate. Debug overlay only — this is the
/// whole reason the match harness has a viewer at all.
#[derive(Component)]
pub struct PlayerStateLabel {
    pub id: u32,
}

#[derive(Component)]
pub struct BallActor;

/// The contact patch under the ball. Without it a lofted ball is impossible to
/// place on the turf from a broadcast angle.
#[derive(Component)]
pub struct BallShadow;

/// Where the ball is right now, in world space, so the camera does not have to
/// go looking for it.
#[derive(Resource, Default)]
pub struct BallState {
    pub position: Vec3,
    pub on_pitch: bool,
    /// Where the ball was on the previous frame, and how fast it is going in
    /// metres per second of MATCH time. The recording carries positions only,
    /// so — exactly as with the players — movement is read back out of it.
    ///
    /// Wanted by the strike detector in [`Actors::animate`]: a player who has
    /// just hit the ball has to turn and face where he hit it.
    pub previous: Option<Vec3>,
    pub velocity: Vec3,
    /// Angular velocity in radians per second of match time, and the rotation
    /// it has accumulated so far. See [`BallSpin`].
    pub spin: Vec3,
    pub rotation: Quat,
    /// What the ball was doing when it was last struck, so the bend it has put
    /// on since can be measured against it. `None` whenever it is not in
    /// flight.
    pub flight: Option<Flight>,
    /// The goalkeeper holding it, and how far his gloves are from where the
    /// recording says the ball is.
    ///
    /// An OFFSET rather than the glove position itself, and that is the whole
    /// design: `held_by` clears on the frame he lets go, but the ramp below
    /// takes a few more to run down, and an absolute point left behind by a
    /// keeper who has just thrown the ball twenty metres drags it visibly
    /// backwards out of his hands. A displacement applied to wherever the ball
    /// actually is now shrinks to nothing in the same few frames without ever
    /// pulling against its flight.
    pub held_by: Option<u32>,
    pub cradle_offset: Vec3,
    /// 0..1 ramp on the hold, shared by the ball's position and the keeper's
    /// arms so the two can never disagree about whether he has it.
    pub cradle: f32,
}

/// The state of a ball in the air, kept so its rotation can be derived from
/// the whole flight rather than from one frame of it.
#[derive(Clone, Copy)]
pub struct Flight {
    /// Heading it was struck on, as `atan2(x, z)`.
    heading: f32,
    /// Seconds of match time since, and the smoothed sidespin read off the
    /// bend so far.
    elapsed: f32,
    sidespin: f32,
}

/// The rotation the ball is carrying, read back out of the path it takes.
///
/// The recording holds positions and nothing else — no spin, no owner — so the
/// ball's rotation is derived here the same way a player's stride is derived
/// from the ground he covers. Without it the ball is a painted sphere sliding
/// through the air on a frozen orientation: the one object on the pitch that
/// never looks alive, and the more so for being the one everybody is watching.
///
/// Three things put rotation on it, and all three are in the trajectory:
///
/// * **Rolling.** A ball on the deck turns at exactly `v / r` about the axis
///   across its travel. Nothing to estimate — this one is not a model.
/// * **Backspin.** A strike gets under the ball, so the rate is read off the
///   launch ANGLE: a driven pass carries little, a ball scooped up under the
///   laces carries a lot, and the same act produces both the loft and the
///   rotation.
/// * **Sidespin.** The engine curls a ball with a Magnus force,
///   `a = C·(ω × v)`, so the bend in the recorded path *is* the rotation that
///   caused it and can be inverted for it.
pub struct BallSpin;

impl BallSpin {
    /// Magnus coefficient in `a = C·(ω × v)`, SI. The engine's own — see
    /// `SpinModel::MAGNUS_COEFF` in the core crate. It is only used here to
    /// run the relation backwards, so the two have to be the same number.
    const MAGNUS: f32 = 0.0039;
    /// Ground speed, in metres per second, below which a ball is not really
    /// rolling — it is being nudged about at somebody's feet, and spinning it
    /// up reads as jitter rather than as motion.
    const CREEP: f32 = 0.35;
    /// Backspin a strike leaves on the ball, as a fraction of the rate it
    /// would be turning at if it were rolling at the same speed: floor for a
    /// ball hit flat, and how much more it picks up as the launch goes
    /// vertical.
    const BACKSPIN: (f32, f32) = (0.12, 0.55);
    /// Ceiling on any single axis, rad/s. 90 is about fourteen turns a second,
    /// past anything a human puts on a football, so it only ever catches an
    /// estimate that has run away.
    const MAX_RATE: f32 = 90.0;
    /// Rotation bleeds off slowly in flight — a struck ball is still turning
    /// when it arrives. Per SECOND of match time, matching the engine's own
    /// `SpinModel::DECAY_PER_TICK` of 0.9997 over its hundred ticks.
    const AIR_DECAY: f32 = 0.97;
    /// Seconds of flight before the bend is worth reading. The recording is
    /// quantised to 0.1 units horizontally and re-sampled every 30 ms, so the
    /// frame-to-frame turn is mostly noise; the total turn over a baseline
    /// this long is not.
    const BEND_WINDOW: f32 = 0.10;
    /// Seconds for the sidespin estimate to take up a new reading, and for a
    /// landing ball to swap flight rotation for rolling contact.
    const BEND_RESPONSE: f32 = 0.18;
    const GRIP_RESPONSE: f32 = 0.06;
    /// And for a ball that has been gathered or trapped to give its rotation
    /// up.
    const SETTLE_RESPONSE: f32 = 0.10;

    /// Rotation of a ball rolling on the turf: no slip, so the contact patch
    /// stands still and `ω = v / r` about the axis across the direction of
    /// travel.
    ///
    /// Against the DRAWN radius, not the regulation one. The viewer's ball is
    /// half again as big so it survives the broadcast distance
    /// ([`Actors::BALL_RADIUS`]), and what the eye checks is whether the
    /// surface it can see is keeping pace with the grass under it.
    fn rolling(velocity: Vec3) -> Vec3 {
        let flat = Vec3::new(velocity.x, 0.0, velocity.z);
        let speed = flat.length();
        if speed < Self::CREEP {
            return Vec3::ZERO;
        }
        match flat.try_normalize() {
            Some(heading) => Vec3::Y.cross(heading) * (speed / Actors::BALL_RADIUS),
            None => Vec3::ZERO,
        }
    }

    /// Rotation a strike leaves on the ball, from the velocity it left with.
    fn struck(velocity: Vec3) -> Vec3 {
        let speed = velocity.length();
        let Some(heading) = Vec3::new(velocity.x, 0.0, velocity.z).try_normalize() else {
            return Vec3::ZERO;
        };
        // Sine of the launch angle: 0 flat along the deck, 1 straight up.
        let climb = (velocity.y / speed.max(1e-3)).clamp(0.0, 1.0);
        let fraction = Self::BACKSPIN.0 + Self::BACKSPIN.1 * climb;
        // Backspin runs against the rolling sense: the top of the ball turns
        // back into the direction of travel, which is what holds it up.
        heading.cross(Vec3::Y) * (speed / Actors::BALL_RADIUS * fraction).min(Self::MAX_RATE)
    }

    /// Sidespin, inverted out of how far the flight has bent since it was
    /// struck.
    ///
    /// `a = C·ω·|v|` for a rotation about the vertical, and a path bending at
    /// `dθ/dt` at speed `|v|` has lateral acceleration `|v|·dθ/dt` — so the
    /// speed cancels and the whole estimate is the turn rate over the Magnus
    /// coefficient. Which is a division by 0.0039, so the baseline it is
    /// measured over has to be long enough to be signal.
    fn sidespin(turned: f32, elapsed: f32) -> f32 {
        if elapsed < Self::BEND_WINDOW {
            return 0.0;
        }
        (turned / elapsed / Self::MAGNUS).clamp(-Self::MAX_RATE, Self::MAX_RATE)
    }

    /// Exponential catch-up over `response` seconds, framerate independent.
    fn approach(response: f32, delta: f32) -> f32 {
        1.0 - (-delta / response).exp()
    }
}

pub struct Actors;

impl Actors {
    /// A match ball is 22 cm across. This one is half again as big: at the
    /// distance a broadcast camera sits, a regulation ball is four pixels.
    const BALL_RADIUS: f32 = 0.16;
    /// Width of the shadow and the team ring on the turf, in metres.
    const FOOTPRINT: f32 = 1.32;
    /// Ground speed, in metres per second, that counts as flat out.
    const SPRINT: f32 = 6.0;
    /// Above this a player faces where they are going; below it they turn to
    /// watch the ball, which is what footballers standing still actually do.
    const MOVING: f32 = 1.1;
    /// No footballer covers ground this fast. Anything quicker is a seek, a
    /// substitution or a restart, and has to be cut to rather than run.
    const TELEPORT: f32 = 25.0;
    /// How close the ball has to be to count as struck by this player — within
    /// a stride of him.
    const STRIKE_REACH: f32 = 1.7;
    /// And how fast it has to be leaving. Below this it is a touch, a trap or
    /// a ball rolling past, none of which a player opens his body up for.
    const STRUCK: f32 = 7.0;
    /// Seconds of match time a player stays turned toward what he just hit —
    /// the follow through, before he picks his running line back up.
    const STRIKE_HOLD: f32 = 0.45;
    /// Radians per second of the standing-still cycle: a weight shift roughly
    /// every three and a half seconds.
    const IDLE_RATE: f32 = 1.8;
    /// Turn rate, in radians per second, that counts as changing direction as
    /// hard as a footballer can. Sets the scale for the lean into it.
    const HARD_TURN: f32 = 4.0;
    /// How far a player will turn his head off his own facing before he has
    /// to turn his shoulders with it.
    const NECK: f32 = 1.05;
    /// Stride length: how far a player travels per step, walking and per extra
    /// metre per second of pace. A sprinter's stride tops out around 2 m.
    const STRIDE: (f32, f32, f32) = (0.75, 0.13, 2.10);
    /// Seconds for a player to come round onto a new heading.
    const TURN_RESPONSE: f32 = 0.13;
    /// Seconds for the run cycle to take up a change of pace.
    const PACE_RESPONSE: f32 = 0.18;
    /// Gap between a player's boots and their name plate, as a fraction of how
    /// tall they are drawn. Measuring it against the player rather than in
    /// metres or in pixels is what keeps the plate clear of the boots at any
    /// distance and under any camera: a world-space offset shrinks to nothing
    /// as the rig flattens, and a pixel offset crowds whoever is nearest.
    const LABEL_GAP: f32 = 0.15;
    /// The band of heights, in metres, that means a ball is in a goalkeeper's
    /// gloves.
    ///
    /// Nothing in the recording says who owns the ball, let alone whether it
    /// has been picked up — but it does not have to. The engine carries a
    /// gathered ball at 1.15 m and every other ball at the height its own
    /// physics put it, so a ball sitting in this band ON TOP OF a keeper is a
    /// ball in his hands and nothing else can be.
    ///
    /// Wider than it needs to be on purpose. The exact carry height is one
    /// constant in the engine and this is a viewer reading its consequences
    /// from the far side of a recording; a band that only just fits it would
    /// break silently the day it moves.
    const GLOVE_HEIGHT: (f32, f32) = (0.85, 1.45);
    /// And how close to him, horizontally, in metres. The engine snaps a
    /// gathered ball to its owner's exact position, so this is really only
    /// tolerance for the moment of the claim; anything larger starts catching
    /// shots that pass him at chest height.
    const GLOVE_REACH: f32 = 0.55;
    /// Seconds of match time to take the ball up into the hold, and to let it
    /// go again. The release is quicker: he throws it.
    const CRADLE_RESPONSE: (f32, f32) = (0.14, 0.06);
    /// Above this, in metres, the ball is in the air rather than on the deck.
    /// The engine's own roll/fly split sits at 0.1 m; this is under it so a
    /// ball is spinning as it flies rather than as it lands.
    const AIRBORNE: f32 = 0.05;

    pub fn spawn(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
        config: Res<ViewerConfig>,
    ) {
        let parts = BodyParts::new(&mut meshes);
        let wardrobe = Wardrobe::new(&mut materials, &mut images, &config);
        let patch = meshes.add(Plane3d::default().mesh().size(1.0, 1.0));

        for player in &config.players {
            let actor = commands
                .spawn((
                    PlayerActor::new(player.id, player.is_goalkeeper()),
                    // Height and build are separate axes, so the squad is a
                    // spread of physiques rather than one model at twenty-two
                    // sizes. `splat` gave everybody an identical shape.
                    Transform::from_scale(Vec3::new(
                        Complexion::build(player.id),
                        Complexion::height(player.id),
                        Complexion::build(player.id),
                    )),
                    Visibility::Hidden,
                ))
                .id();

            // Drawn under the boots, in this order: the shadow that roots the
            // player on the turf, then their team's ring around it.
            commands.entity(actor).with_children(|marks| {
                marks.spawn((
                    Mesh3d(patch.clone()),
                    MeshMaterial3d(wardrobe.shadow()),
                    Transform::from_xyz(0.0, 0.018, 0.0)
                        .with_scale(Vec3::splat(Self::FOOTPRINT * 0.86)),
                ));
                marks.spawn((
                    Mesh3d(patch.clone()),
                    MeshMaterial3d(wardrobe.marker(player.is_home)),
                    Transform::from_xyz(0.0, 0.022, 0.0).with_scale(Vec3::splat(Self::FOOTPRINT)),
                ));
            });
            Footballer::assemble(&mut commands, actor, &parts, &wardrobe.outfit(player));

            let mut plate = commands.spawn((
                PlayerLabel { actor },
                // The trailing newline is what puts the state on its own line
                // below the name when the debug span is attached.
                Text::new(Self::label_for(player, config.debug)),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.96, 1.0)),
                // Tight enough to read as an outline rather than as a second
                // copy of the name: these plates sit over grass, over white
                // paint and over each other.
                TextShadow {
                    offset: Vec2::splat(1.0),
                    color: Color::srgba(0.0, 0.0, 0.0, 0.85),
                },
                TextLayout::justify(Justify::Center),
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                Visibility::Hidden,
            ));

            if config.debug {
                plate.with_child((
                    PlayerStateLabel { id: player.id },
                    TextSpan::default(),
                    TextFont {
                        font_size: FontSize::Px(11.0),
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 0.93, 0.4)),
                ));
            }
        }

        let ball_material = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(Textures::football(&mut images)),
            // Trimmed from 0.25. The emissive is there to keep the ball
            // legible against the turf, but it is added uniformly — at 0.25
            // it lifted the black panels to grey and the ball was white
            // again, which defeats the point of painting it.
            emissive: LinearRgba::rgb(0.08, 0.08, 0.08),
            perceptual_roughness: 0.6,
            ..default()
        });
        commands.spawn((
            BallActor,
            Mesh3d(meshes.add(Sphere::new(Self::BALL_RADIUS).mesh().uv(16, 10))),
            MeshMaterial3d(ball_material),
            Transform::from_xyz(0.0, Self::BALL_RADIUS, 0.0),
            Visibility::Hidden,
        ));

        commands.spawn((
            BallShadow,
            Mesh3d(patch),
            MeshMaterial3d(wardrobe.shadow()),
            Transform::from_xyz(0.0, 0.02, 0.0),
            Visibility::Hidden,
        ));
    }

    /// Drives every player and the ball from the recording at the current
    /// playhead. Players with no samples around `now` were substituted off (or
    /// have not come on yet) and simply vanish.
    pub fn follow_playhead(
        playback: Res<Playback>,
        time: Res<Time>,
        loader: Res<ChunkLoader>,
        mut tracks: ResMut<ReplayTracks>,
        mut ball_state: ResMut<BallState>,
        mut players: Query<(&PlayerActor, &mut Transform, &mut Visibility)>,
        mut ball: Query<(&mut Transform, &mut Visibility), (With<BallActor>, Without<PlayerActor>)>,
        mut shadow: Query<
            (&mut Transform, &mut Visibility),
            (With<BallShadow>, Without<PlayerActor>, Without<BallActor>),
        >,
    ) {
        let now = playback.time_ms;
        // Seconds of MATCH time this frame covered. Everything derived from
        // the recording — velocity, rotation, the hold ramp — is measured
        // against this rather than against the wall clock, so it all means the
        // same thing at 1x and at 8x.
        let delta = (playback.speed.max(0.1) * time.delta_secs()).max(1e-4);
        // No samples here can mean two things. If the chunk covering `now` has
        // arrived, the entity really is off the pitch. If it has not, the data
        // is simply still in flight — freeze everyone where they stand rather
        // than blanking the pitch every time the viewer scrubs.
        let covered = loader.covers(now);

        let ball_position = tracks
            .ball
            .position_at(now)
            .map(|[x, y, z]| Field::to_world(x, y, z));

        // Who has it in his gloves, and where those gloves are. Resolved in
        // the same pass that places the players, because the answer depends on
        // where they have just been put.
        let mut holder: Option<(u32, Vec3)> = None;
        for (actor, mut transform, mut visibility) in &mut players {
            let position = tracks
                .players
                .get_mut(&actor.id)
                .and_then(|track| track.position_at(now));
            match position {
                Some([x, y, _]) => {
                    let world = Field::to_world(x, y, 0.0);
                    transform.translation.x = world.x;
                    transform.translation.z = world.z;
                    *visibility = Visibility::Inherited;
                }
                None if covered => *visibility = Visibility::Hidden,
                None => {}
            }

            if !actor.is_goalkeeper || holder.is_some() || *visibility == Visibility::Hidden {
                continue;
            }
            if let Some(ball) = ball_position {
                let reach = Vec2::new(
                    ball.x - transform.translation.x,
                    ball.z - transform.translation.z,
                )
                .length();
                if reach < Self::GLOVE_REACH
                    && ball.y > Self::GLOVE_HEIGHT.0
                    && ball.y < Self::GLOVE_HEIGHT.1
                {
                    // Players are root entities, so the local transform is the
                    // world one and the cradle can be carried straight out of
                    // the rig's own space through the man's height and build.
                    holder = Some((actor.id, transform.transform_point(Physique::CRADLE)));
                }
            }
        }

        ball_state.on_pitch = ball_position.is_some();
        if let Some(world) = ball_position {
            // Ball velocity, in metres per second of match time, read off the
            // RAW recorded path — never off the drawn position, which is
            // displaced while a keeper has it. Blanked on a seek, where the
            // jump is the playhead moving and not the ball.
            ball_state.velocity = match ball_state.previous {
                Some(previous) if !playback.seeked => (world - previous) / delta,
                _ => Vec3::ZERO,
            };
            ball_state.previous = Some(world);
            ball_state.position = world;

            // Ramp the hold. One ramp, two consumers — the ball's position
            // here and the keeper's arms in `animate` — so the gloves can
            // never close a frame before or after the ball arrives in them.
            if let Some((keeper, gloves)) = holder {
                ball_state.held_by = Some(keeper);
                ball_state.cradle_offset = gloves - world;
            } else {
                ball_state.held_by = None;
            }
            let response = if holder.is_some() {
                Self::CRADLE_RESPONSE.0
            } else {
                Self::CRADLE_RESPONSE.1
            };
            let wanted = f32::from(holder.is_some());
            ball_state.cradle += (wanted - ball_state.cradle)
                * if playback.seeked {
                    1.0
                } else {
                    BallSpin::approach(response, delta)
                };

            let drawn = if ball_state.cradle > 1e-3 {
                world + ball_state.cradle_offset * ball_state.cradle
            } else {
                world
            };
            Self::turn_ball(&mut ball_state, delta, playback.seeked);

            if let Ok((mut transform, mut visibility)) = ball.single_mut() {
                transform.translation = drawn + Vec3::Y * Self::BALL_RADIUS;
                transform.rotation = ball_state.rotation;
                *visibility = Visibility::Inherited;
            }
            if let Ok((mut transform, mut visibility)) = shadow.single_mut() {
                transform.translation = Vec3::new(drawn.x, 0.02, drawn.z);
                // Fade and spread the patch with height, the way a real one does.
                let spread = 0.62 * (1.0 + (drawn.y * 0.08).min(0.7));
                transform.scale = Vec3::new(spread, 1.0, spread);
                *visibility = Visibility::Inherited;
            }
        } else if covered {
            ball_state.previous = None;
            ball_state.velocity = Vec3::ZERO;
            ball_state.spin = Vec3::ZERO;
            ball_state.flight = None;
            ball_state.held_by = None;
            ball_state.cradle = 0.0;
            if let Ok((_, mut visibility)) = ball.single_mut() {
                *visibility = Visibility::Hidden;
            }
            if let Ok((_, mut visibility)) = shadow.single_mut() {
                *visibility = Visibility::Hidden;
            }
        }
    }

    /// Advances the ball's rotation for this frame, from where its own path
    /// says it should be turning. See [`BallSpin`].
    fn turn_ball(ball: &mut BallState, delta: f32, seeked: bool) {
        if seeked {
            // The jump is the playhead's, not the ball's: there is no
            // trajectory across it to read a rotation from.
            ball.spin = Vec3::ZERO;
            ball.flight = None;
            return;
        }

        if ball.held_by.is_some() {
            // In the gloves. Whatever it was doing, it has stopped.
            //
            // Keyed off the holder and not off the ramp: the recorded ball
            // climbs a metre into his hands inside a single frame, which is a
            // launch by every test below, and waiting for the ramp to cross a
            // threshold lets it spin up hard for a tenth of a second first.
            ball.spin *= 1.0 - BallSpin::approach(BallSpin::SETTLE_RESPONSE, delta);
            ball.flight = None;
        } else if ball.position.y > Self::AIRBORNE {
            // A ball with no measurable heading — the top of a vertical lob,
            // or the first frame after a chunk landed — has no trajectory to
            // read. It keeps turning as it was; the estimate picks up again
            // the moment it is going somewhere.
            if let Some(travel) = Vec3::new(ball.velocity.x, 0.0, ball.velocity.z).try_normalize() {
                let heading = travel.x.atan2(travel.z);
                match &mut ball.flight {
                    // Already up. Hold the rotation it left with, less the
                    // little the air takes back, and keep refining the
                    // sidespin from how far it has bent since.
                    Some(flight) => {
                        flight.elapsed += delta;
                        let turned = (heading - flight.heading + PI).rem_euclid(TAU) - PI;
                        let reading = BallSpin::sidespin(turned, flight.elapsed);
                        flight.sidespin += (reading - flight.sidespin)
                            * BallSpin::approach(BallSpin::BEND_RESPONSE, delta);
                        ball.spin *= BallSpin::AIR_DECAY.powf(delta);
                        ball.spin.y = flight.sidespin;
                    }
                    // Just left the deck — or a boot, or a bounce. Whatever
                    // put it up there decided the rotation, and the launch
                    // velocity is the only record of it.
                    None => {
                        if ball.velocity.length() > BallSpin::CREEP {
                            ball.spin = BallSpin::struck(ball.velocity);
                        }
                        ball.flight = Some(Flight {
                            heading,
                            elapsed: 0.0,
                            sidespin: 0.0,
                        });
                    }
                }
            }
        } else {
            // On the grass. Rolling contact takes over within a few
            // hundredths of a second of touching down, which is what turns a
            // backspun ball round and checks it.
            ball.flight = None;
            let rolling = BallSpin::rolling(ball.velocity);
            ball.spin += (rolling - ball.spin) * BallSpin::approach(BallSpin::GRIP_RESPONSE, delta);
        }

        // Integrated in world space, so the ball keeps turning about a fixed
        // axis rather than about one that its own rotation drags round with
        // it — pre-multiplied for that reason. Renormalised every frame: this
        // runs a few hundred thousand times over a full replay.
        if ball.spin.length_squared() > 1e-6 {
            ball.rotation = (Quat::from_scaled_axis(ball.spin * delta) * ball.rotation).normalize();
        }
    }

    /// Turns each player's change of position into a heading and a stride, then
    /// poses their limbs from it.
    ///
    /// The recording holds positions and nothing else — no facing, no speed, no
    /// animation track — so everything a footballer's body does is derived here
    /// from the ground they cover. Driving the stride by distance rather than by
    /// time is what stops the feet from skating: however fast the playhead is
    /// running, a player still takes one step per stride length of turf.
    pub fn animate(
        playback: Res<Playback>,
        ball: Res<BallState>,
        time: Res<Time>,
        mut actors: Query<(&mut PlayerActor, &mut Transform, &Visibility)>,
        mut joints: Query<(&Joint, &mut Transform), Without<PlayerActor>>,
    ) {
        let delta = time.delta_secs().max(1e-4);
        // Exponential catch-up, framerate independent.
        let turn = 1.0 - (-delta / Self::TURN_RESPONSE).exp();
        let pace = 1.0 - (-delta / Self::PACE_RESPONSE).exp();

        for (mut actor, mut transform, visibility) in &mut actors {
            if *visibility == Visibility::Hidden {
                actor.previous = None;
                continue;
            }

            let position = transform.translation;
            let step = match actor.previous {
                Some(previous) if !playback.seeked => position - previous,
                _ => Vec3::ZERO,
            };
            actor.previous = Some(position);

            // Playback speed belongs to the viewer, not to the player: divide
            // it back out or everybody sprints at 8x.
            let ground = step.length();
            let observed = ground / (delta * playback.speed.max(0.1));
            let (ground, observed) = if observed > Self::TELEPORT {
                (0.0, actor.speed)
            } else {
                (ground, observed)
            };
            actor.speed += (observed - actor.speed) * pace;

            // Did he just hit it? The ball has to be leaving him at pace and
            // from within reach. Requiring it to be moving AWAY is what tells
            // a strike from a reception — a player taking a ball in is just as
            // close to just as fast a ball, and without the test he would spin
            // to face the way it arrived.
            //
            // Never for the keeper who is gathering it: the ball climbing a
            // metre into his gloves inside one frame is a huge upward velocity
            // pointing straight away from his boots, which is a strike by
            // every test here and by none in reality.
            let gathering = ball.held_by == Some(actor.id) || actor.carry > 1e-3;
            if ball.on_pitch && !playback.seeked && !gathering {
                let from_him = ball.position - position;
                let reach = Vec3::new(from_him.x, 0.0, from_him.z).length();
                let departing = ball.velocity.dot(from_him) > 0.0;
                if reach < Self::STRIKE_REACH && departing && ball.velocity.length() > Self::STRUCK
                {
                    if let Some(direction) =
                        Vec3::new(ball.velocity.x, 0.0, ball.velocity.z).try_normalize()
                    {
                        actor.strike = Some((direction, Self::STRIKE_HOLD));
                    }
                }
            }
            // Tick the hold down in match time, so a strike does not hold for
            // eight times as long when the replay is run at 8x.
            if let Some((_, remaining)) = &mut actor.strike {
                *remaining -= delta * playback.speed.max(0.1);
                if *remaining <= 0.0 || playback.seeked {
                    actor.strike = None;
                }
            }

            let facing = if let Some((direction, _)) = actor.strike {
                // Opened up to where he played it, for as long as the follow
                // through lasts. Outranks the run: this is the one moment a
                // footballer is not facing where he is going.
                direction
            } else if actor.speed > Self::MOVING {
                Vec3::new(step.x, 0.0, step.z)
            } else if ball.on_pitch {
                ball.position - position
            } else {
                Vec3::ZERO
            };
            let mut turn_signal = 0.0_f32;
            if let Some(facing) = Vec3::new(facing.x, 0.0, facing.z).try_normalize() {
                // Rotating about Y by `atan2(x, z)` carries +Z onto the facing,
                // and the model is built looking down +Z.
                let wanted = facing.x.atan2(facing.z);
                let swing = (wanted - actor.heading + PI).rem_euclid(TAU) - PI;
                let applied = swing * if playback.seeked { 1.0 } else { turn };
                actor.heading += applied;
                // In radians per second of match time, normalised against a
                // hard change of direction, so the lean is the same at any
                // frame rate or playback speed.
                let rate = applied / (delta * playback.speed.max(0.1));
                turn_signal = (rate / Self::HARD_TURN).clamp(-1.0, 1.0);
            }
            actor.turn += (turn_signal - actor.turn) * pace;
            transform.rotation = Quat::from_rotation_y(actor.heading);

            // Idle cycle runs on the clock, not on ground covered, because it
            // exists precisely for the player who is covering none.
            actor.idle =
                (actor.idle + delta * playback.speed.max(0.1) * Self::IDLE_RATE).rem_euclid(TAU);

            // Where he is looking. Clamped to what a neck can do — past that a
            // real player turns his whole body, which he is already doing.
            let wanted_look = if ball.on_pitch {
                let to_ball = ball.position - position;
                match Vec3::new(to_ball.x, 0.0, to_ball.z).try_normalize() {
                    Some(bearing) => {
                        let angle = bearing.x.atan2(bearing.z);
                        (((angle - actor.heading + PI).rem_euclid(TAU)) - PI)
                            .clamp(-Self::NECK, Self::NECK)
                    }
                    None => 0.0,
                }
            } else {
                0.0
            };
            actor.look += (wanted_look - actor.look) * if playback.seeked { 1.0 } else { turn };

            // The one man with the ball in his gloves takes the ramp that the
            // ball itself is riding; everybody else lets whatever they were
            // holding fall away. Following a ramp with a second one is
            // deliberate — it puts the arms a fraction behind the ball, so a
            // keeper throwing it out has a follow through rather than
            // snapping back to his sides on the frame it leaves him.
            let wanted = if ball.held_by == Some(actor.id) {
                ball.cradle
            } else {
                0.0
            };
            actor.carry += (wanted - actor.carry) * if playback.seeked { 1.0 } else { pace };

            let stride = (Self::STRIDE.0 + Self::STRIDE.1 * actor.speed)
                .clamp(Self::STRIDE.0, Self::STRIDE.2);
            // Half a cycle per step: the other leg takes the next one.
            actor.phase = (actor.phase + ground * PI / stride).rem_euclid(TAU);
        }

        for (joint, mut transform) in &mut joints {
            let Ok((actor, _, _)) = actors.get(joint.owner) else {
                continue;
            };
            let gait = actor.gait();
            transform.rotation = joint.pose(gait);
            transform.translation = joint.place(gait);
        }
    }

    /// Writes each player's current engine state under their name. Debug
    /// overlay only, and only for recordings that carry state tracking.
    pub fn follow_states(
        playback: Res<Playback>,
        overlay: Res<DebugOverlay>,
        mut tracks: ResMut<ReplayTracks>,
        mut labels: Query<(&PlayerStateLabel, &mut TextSpan)>,
    ) {
        let now = playback.time_ms;
        for (label, mut span) in &mut labels {
            let wanted = if overlay.states {
                tracks
                    .states
                    .get_mut(&label.id)
                    .and_then(|track| track.name_at(now))
                    .unwrap_or_default()
            } else {
                ""
            };
            if span.as_str() != wanted {
                **span = wanted.to_string();
            }
        }
    }

    /// Projects each visible player to screen space and parks their name plate
    /// just below their feet.
    ///
    /// Both the camera and the players are root entities, so their local
    /// transforms are their world transforms — projecting from those rather
    /// than from `GlobalTransform` keeps the plates locked to the players
    /// instead of trailing a frame behind the pan.
    pub fn place_labels(
        camera: Single<(&Camera, &Transform), With<Camera3d>>,
        actors: Query<(&Transform, &Visibility), With<PlayerActor>>,
        mut labels: Query<(&PlayerLabel, &mut Node, &mut Visibility), Without<PlayerActor>>,
    ) {
        let (camera, camera_transform) = *camera;
        let camera_transform = GlobalTransform::from(*camera_transform);

        for (label, mut node, mut visibility) in &mut labels {
            let Ok((actor_transform, actor_visibility)) = actors.get(label.actor) else {
                *visibility = Visibility::Hidden;
                continue;
            };
            if *actor_visibility == Visibility::Hidden {
                *visibility = Visibility::Hidden;
                continue;
            }
            // Project the player twice — once at the boots, once at the crown —
            // and hang the plate below the boots by a share of the height
            // between them.
            let boots = actor_transform.translation;
            let crown = boots + Vec3::Y * Physique::STATURE * actor_transform.scale.y;
            let (Ok(boots), Ok(crown)) = (
                camera.world_to_viewport(&camera_transform, boots),
                camera.world_to_viewport(&camera_transform, crown),
            ) else {
                *visibility = Visibility::Hidden;
                continue;
            };

            // The plate is centred by hand: UI nodes are positioned by their
            // top-left corner and the text width is not known here.
            let stature = (boots.y - crown.y).abs().max(6.0);
            node.left = Val::Px(boots.x - 44.0);
            node.top = Val::Px(boots.y + stature * Self::LABEL_GAP);
            node.width = Val::Px(88.0);
            node.justify_content = JustifyContent::Center;
            *visibility = Visibility::Inherited;
        }
    }

    /// The name plate. Just the surname: the shirt number is on the player's
    /// back, where a viewer reads it from, and repeating it in front of every
    /// name only crowds the pitch.
    fn label_for(player: &PlayerInfo, debug: bool) -> String {
        if debug {
            format!("{}\n", player.last_name)
        } else {
            player.last_name.clone()
        }
    }
}

impl PlayerActor {
    fn new(id: u32, is_goalkeeper: bool) -> Self {
        PlayerActor {
            id,
            is_goalkeeper,
            carry: 0.0,
            previous: None,
            heading: 0.0,
            // Start everyone at a different point in the run cycle. The
            // phase advances with ground covered, so two players moving at
            // the same speed from the same start stay in step for the whole
            // match — which is why the squad used to move as one organism.
            phase: Complexion::carriage(id) * std::f32::consts::PI,
            speed: 0.0,
            strike: None,
            // Offset so twenty-two players are not all breathing in unison,
            // which would be its own kind of robot.
            idle: Complexion::carriage(id) * std::f32::consts::PI,
            turn: 0.0,
            look: 0.0,
        }
    }

    fn gait(&self) -> Gait {
        Gait {
            phase: self.phase,
            run: (self.speed / Actors::SPRINT).clamp(0.0, 1.0),
            signature: Complexion::carriage(self.id),
            idle: self.idle,
            turn: self.turn,
            look: self.look,
            carry: self.carry,
        }
    }
}
