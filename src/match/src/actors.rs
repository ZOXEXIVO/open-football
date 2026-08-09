use crate::config::{PlayerInfo, ViewerConfig};
use crate::field::Field;
use crate::loader::ChunkLoader;
use crate::playback::Playback;
use crate::replay::ReplayTracks;
use crate::timeline::DebugOverlay;
use bevy::prelude::*;

/// A player marker on the pitch: a flat disc in the team's colours, with the
/// name plate that tracks it drawn as a UI node (see [`PlayerLabel`]).
#[derive(Component)]
pub struct PlayerActor {
    pub id: u32,
}

/// The name plate for one player, positioned each frame from the disc's
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
    const DISC_RADIUS: f32 = 0.74;
    const RING_RADIUS: f32 = 0.92;
    const DISC_THICKNESS: f32 = 0.18;
    const BALL_RADIUS: f32 = 0.24;
    /// Keepers wear the same colour whichever side they are on, exactly as the
    /// old 2D viewer did — it is the fastest way to read a crowded box.
    const KEEPER_COLOR: Color = Color::srgb(0.97, 0.89, 0.0);

    pub fn spawn(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        config: Res<ViewerConfig>,
    ) {
        let disc = meshes.add(Cylinder::new(Self::DISC_RADIUS, Self::DISC_THICKNESS));
        let ring = meshes.add(Cylinder::new(Self::RING_RADIUS, Self::DISC_THICKNESS * 0.6));

        let home_fill = Self::fill(
            &mut materials,
            config.home.background_color(Color::srgb(0.0, 0.19, 0.49)),
        );
        let away_fill = Self::fill(
            &mut materials,
            config.away.background_color(Color::srgb(0.70, 0.25, 0.0)),
        );
        let keeper_fill = Self::fill(&mut materials, Self::KEEPER_COLOR);
        let home_ring = Self::fill(&mut materials, config.home.foreground_color(Color::WHITE));
        let away_ring = Self::fill(&mut materials, config.away.foreground_color(Color::WHITE));

        for player in &config.players {
            let (fill, outline) = match (player.is_goalkeeper(), player.is_home) {
                (true, true) => (keeper_fill.clone(), home_ring.clone()),
                (true, false) => (keeper_fill.clone(), away_ring.clone()),
                (false, true) => (home_fill.clone(), home_ring.clone()),
                (false, false) => (away_fill.clone(), away_ring.clone()),
            };

            let actor = commands
                .spawn((
                    PlayerActor { id: player.id },
                    Mesh3d(ring.clone()),
                    MeshMaterial3d(outline),
                    Transform::from_xyz(0.0, Self::DISC_THICKNESS * 0.3, 0.0),
                    Visibility::Hidden,
                ))
                .with_children(|actor| {
                    actor.spawn((
                        Mesh3d(disc.clone()),
                        MeshMaterial3d(fill),
                        Transform::from_xyz(0.0, Self::DISC_THICKNESS * 0.4, 0.0),
                    ));
                })
                .id();

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
                TextShadow::default(),
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
            Mesh3d(meshes.add(Sphere::new(Self::BALL_RADIUS))),
            MeshMaterial3d(ball_material),
            Transform::from_xyz(0.0, Self::BALL_RADIUS, 0.0),
            Visibility::Hidden,
        ));

        let shadow_material = materials.add(StandardMaterial {
            base_color: Color::srgba(0.0, 0.0, 0.0, 0.32),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        });
        commands.spawn((
            BallShadow,
            Mesh3d(meshes.add(Cylinder::new(Self::BALL_RADIUS, 0.01))),
            MeshMaterial3d(shadow_material),
            Transform::from_xyz(0.0, 0.02, 0.0),
            Visibility::Hidden,
        ));
    }

    /// Drives every disc and the ball from the recording at the current
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
                let spread = 1.0 + (world.y * 0.08).min(0.7);
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

    /// Projects each visible disc to screen space and parks its name plate just
    /// below it.
    ///
    /// Both the camera and the discs are root entities, so their local
    /// transforms are their world transforms — projecting from those rather
    /// than from `GlobalTransform` keeps the plates locked to the discs instead
    /// of trailing a frame behind the pan.
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
            // Anchor the plate a stride in front of the disc rather than a
            // fixed number of pixels below it: the camera always looks from
            // -Z, so this clears the marker by the same amount whether the
            // player is on the near touchline or the far one.
            let anchor = actor_transform.translation - Vec3::Z * (Self::RING_RADIUS + 0.7);
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

    fn fill(materials: &mut Assets<StandardMaterial>, color: Color) -> Handle<StandardMaterial> {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.75,
            metallic: 0.0,
            ..default()
        })
    }
}
