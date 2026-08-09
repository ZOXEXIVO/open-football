use crate::body::{BodyParts, Footballer, Gait, Joint};
use crate::config::{PlayerInfo, ViewerConfig};
use crate::field::Field;
use crate::kit::{Complexion, Wardrobe};
use crate::loader::ChunkLoader;
use crate::playback::Playback;
use crate::replay::ReplayTracks;
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
    /// Stride length: how far a player travels per step, walking and per extra
    /// metre per second of pace. A sprinter's stride tops out around 2 m.
    const STRIDE: (f32, f32, f32) = (0.75, 0.13, 2.10);
    /// Seconds for a player to come round onto a new heading.
    const TURN_RESPONSE: f32 = 0.13;
    /// Seconds for the run cycle to take up a change of pace.
    const PACE_RESPONSE: f32 = 0.18;
    /// How far in front of a player the name plate is anchored — far enough to
    /// clear their own boots from a camera that always looks from −Z.
    const LABEL_STANDOFF: f32 = 1.15;

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
                    Transform::from_scale(Vec3::splat(Complexion::height(player.id))),
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
            emissive: LinearRgba::rgb(0.25, 0.25, 0.25),
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

            let facing = if actor.speed > Self::MOVING {
                Vec3::new(step.x, 0.0, step.z)
            } else if ball.on_pitch {
                ball.position - position
            } else {
                Vec3::ZERO
            };
            if let Some(facing) = Vec3::new(facing.x, 0.0, facing.z).try_normalize() {
                // Rotating about Y by `atan2(x, z)` carries +Z onto the facing,
                // and the model is built looking down +Z.
                let wanted = facing.x.atan2(facing.z);
                let swing = (wanted - actor.heading + PI).rem_euclid(TAU) - PI;
                actor.heading += swing * if playback.seeked { 1.0 } else { turn };
            }
            transform.rotation = Quat::from_rotation_y(actor.heading);

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
            // Anchor the plate a stride in front of the player rather than a
            // fixed number of pixels below them: the camera always looks from
            // -Z, so this clears the boots by the same amount whether the
            // player is on the near touchline or the far one.
            let anchor = actor_transform.translation - Vec3::Z * Self::LABEL_STANDOFF;
            let Ok(screen) = camera.world_to_viewport(&camera_transform, anchor) else {
                *visibility = Visibility::Hidden;
                continue;
            };

            // The plate is centred by hand: UI nodes are positioned by their
            // top-left corner and the text width is not known here.
            node.left = Val::Px(screen.x - 40.0);
            node.top = Val::Px(screen.y + 2.0);
            node.width = Val::Px(80.0);
            node.justify_content = JustifyContent::Center;
            *visibility = Visibility::Inherited;
        }
    }

    fn label_for(player: &PlayerInfo, debug: bool) -> String {
        let name = if player.shirt_number > 0 {
            format!("{} {}", player.shirt_number, player.last_name)
        } else {
            player.last_name.clone()
        };
        if debug { format!("{}\n", name) } else { name }
    }
}

impl PlayerActor {
    fn new(id: u32) -> Self {
        PlayerActor {
            id,
            previous: None,
            heading: 0.0,
            phase: 0.0,
            speed: 0.0,
        }
    }

    fn gait(&self) -> Gait {
        Gait {
            phase: self.phase,
            run: (self.speed / Actors::SPRINT).clamp(0.0, 1.0),
        }
    }
}
