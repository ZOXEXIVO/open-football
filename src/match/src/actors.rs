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
                    PlayerActor::new(player.id),
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
        // No samples here can mean two things. If the chunk covering `now` has
        // arrived, the entity really is off the pitch. If it has not, the data
        // is simply still in flight — freeze everyone where they stand rather
        // than blanking the pitch every time the viewer scrubs.
        let covered = loader.covers(now);

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
        }

        let ball_position = tracks.ball.position_at(now);
        ball_state.on_pitch = ball_position.is_some();
        if let Some([x, y, z]) = ball_position {
            let world = Field::to_world(x, y, z);
            // Ball velocity, in metres per second of match time. Derived from
            // the sample interval rather than the frame time so it means the
            // same thing at 1x and at 8x, and blanked on a seek where the jump
            // is the playhead moving, not the ball.
            ball_state.velocity = match ball_state.previous {
                Some(previous) if !playback.seeked => {
                    let dt = (playback.speed.max(0.1) * time.delta_secs()).max(1e-4);
                    (world - previous) / dt
                }
                _ => Vec3::ZERO,
            };
            ball_state.previous = Some(world);
            ball_state.position = world;

            if let Ok((mut transform, mut visibility)) = ball.single_mut() {
                transform.translation = world + Vec3::Y * Self::BALL_RADIUS;
                *visibility = Visibility::Inherited;
            }
            if let Ok((mut transform, mut visibility)) = shadow.single_mut() {
                transform.translation = Vec3::new(world.x, 0.02, world.z);
                // Fade and spread the patch with height, the way a real one does.
                let spread = 0.62 * (1.0 + (world.y * 0.08).min(0.7));
                transform.scale = Vec3::new(spread, 1.0, spread);
                *visibility = Visibility::Inherited;
            }
        } else if covered {
            ball_state.previous = None;
            ball_state.velocity = Vec3::ZERO;
            if let Ok((_, mut visibility)) = ball.single_mut() {
                *visibility = Visibility::Hidden;
            }
            if let Ok((_, mut visibility)) = shadow.single_mut() {
                *visibility = Visibility::Hidden;
            }
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
            if ball.on_pitch && !playback.seeked {
                let from_him = ball.position - position;
                let reach = Vec3::new(from_him.x, 0.0, from_him.z).length();
                let departing = ball.velocity.dot(from_him) > 0.0;
                if reach < Self::STRIKE_REACH
                    && departing
                    && ball.velocity.length() > Self::STRUCK
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

            let stride =
                (Self::STRIDE.0 + Self::STRIDE.1 * actor.speed).clamp(Self::STRIDE.0, Self::STRIDE.2);
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
    fn new(id: u32) -> Self {
        PlayerActor {
            id,
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
        }
    }
}
