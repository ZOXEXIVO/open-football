use crate::field::Field;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// Accumulates every pitch marking into a single flat mesh.
///
/// Painted lines are all the same thing — a strip of white a few centimetres
/// wide lying on the turf — so they are built as quads in one buffer rather
/// than as a few hundred entities. Circles and arcs are just segmented strips.
struct LineMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    height: f32,
    half_width: f32,
}

impl LineMesh {
    fn new(height: f32, width: f32) -> Self {
        LineMesh {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            height,
            half_width: width * 0.5,
        }
    }

    /// A straight strip between two points on the turf, given as `(x, z)`.
    /// Both ends overshoot by half the line width so corners close up.
    fn segment(&mut self, from: Vec2, to: Vec2) {
        let direction = (to - from).normalize_or_zero();
        if direction == Vec2::ZERO {
            return;
        }
        let across = Vec2::new(-direction.y, direction.x) * self.half_width;
        let from = from - direction * self.half_width;
        let to = to + direction * self.half_width;

        let base = self.positions.len() as u32;
        for corner in [from - across, from + across, to + across, to - across] {
            self.push_vertex(corner);
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn rectangle(&mut self, centre: Vec2, size: Vec2) {
        let half = size * 0.5;
        let corners = [
            centre + Vec2::new(-half.x, -half.y),
            centre + Vec2::new(half.x, -half.y),
            centre + Vec2::new(half.x, half.y),
            centre + Vec2::new(-half.x, half.y),
        ];
        for index in 0..4 {
            self.segment(corners[index], corners[(index + 1) % 4]);
        }
    }

    /// Angles run from the +X axis toward +Z.
    fn arc(&mut self, centre: Vec2, radius: f32, from: f32, to: f32) {
        let steps = (((to - from).abs() / TAU) * 96.0).ceil().max(2.0) as usize;
        let mut previous = centre + Vec2::from_angle(from) * radius;
        for step in 1..=steps {
            let angle = from + (to - from) * (step as f32 / steps as f32);
            let point = centre + Vec2::from_angle(angle) * radius;
            self.segment(previous, point);
            previous = point;
        }
    }

    fn circle(&mut self, centre: Vec2, radius: f32) {
        self.arc(centre, radius, 0.0, TAU);
    }

    /// A filled spot — penalty and centre marks.
    fn spot(&mut self, centre: Vec2, radius: f32) {
        const STEPS: usize = 16;
        let hub = self.positions.len() as u32;
        self.push_vertex(centre);
        for step in 0..=STEPS {
            let angle = TAU * (step as f32 / STEPS as f32);
            self.push_vertex(centre + Vec2::from_angle(angle) * radius);
        }
        for step in 0..STEPS as u32 {
            self.indices
                .extend_from_slice(&[hub, hub + step + 1, hub + step + 2]);
        }
    }

    fn push_vertex(&mut self, point: Vec2) {
        self.positions.push([point.x, self.height, point.y]);
        self.normals.push([0.0, 1.0, 0.0]);
        self.uvs.push([0.0, 0.0]);
    }

    fn build(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// One bank of terracing, resolved into the numbers a tilted slab needs.
struct Terrace {
    length: f32,
    /// Distance from the middle of the pitch to the middle of the slab.
    reach: f32,
    height: f32,
    /// Angle it is tilted up from the flat, in radians.
    pitch: f32,
}

/// The stadium: turf, painted markings, both goals and the ground around them.
pub struct Pitch;

impl Pitch {
    /// Mowing stripes. An odd count keeps the two halves mirror-imaged.
    const STRIPES: usize = 15;
    const LINE_WIDTH: f32 = 0.14;
    const LINE_HEIGHT: f32 = 0.012;

    pub fn spawn(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
    ) {
        let surround = materials.add(StandardMaterial {
            base_color: Color::srgb(0.05, 0.13, 0.07),
            perceptual_roughness: 1.0,
            ..default()
        });
        commands.spawn((
            Mesh3d(meshes.add(Plane3d::default().mesh().size(320.0, 260.0))),
            MeshMaterial3d(surround),
            Transform::from_xyz(0.0, -0.05, 0.0),
        ));

        Self::spawn_turf(&mut commands, &mut meshes, &mut materials);
        Self::spawn_markings(&mut commands, &mut meshes, &mut materials);
        Self::spawn_goals(&mut commands, &mut meshes, &mut materials);
        Self::spawn_ground(&mut commands, &mut meshes, &mut materials);

        commands.spawn((
            DirectionalLight {
                color: Color::srgb(1.0, 0.98, 0.92),
                illuminance: 11_000.0,
                // Deliberately off: the only thing tall enough to cast a
                // meaningful shadow is the ball, which carries its own contact
                // disc, and cascaded shadow maps are the single most expensive
                // thing this scene could ask a WebGL2 context for.
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.0).looking_at(Vec3::new(-0.4, -1.0, 0.35), Vec3::Y),
        ));
    }

    fn spawn_turf(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
    ) {
        let shades = [
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.17, 0.44, 0.20),
                perceptual_roughness: 1.0,
                ..default()
            }),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.14, 0.38, 0.17),
                perceptual_roughness: 1.0,
                ..default()
            }),
        ];

        let stripe_length = Field::LENGTH / Self::STRIPES as f32;
        let stripe = meshes.add(Plane3d::default().mesh().size(stripe_length, Field::WIDTH));
        for index in 0..Self::STRIPES {
            let centre = -Field::HALF_LENGTH + stripe_length * (index as f32 + 0.5);
            commands.spawn((
                Mesh3d(stripe.clone()),
                MeshMaterial3d(shades[index % 2].clone()),
                Transform::from_xyz(centre, 0.0, 0.0),
            ));
        }
    }

    fn spawn_markings(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
    ) {
        let mut lines = LineMesh::new(Self::LINE_HEIGHT, Self::LINE_WIDTH);

        lines.rectangle(Vec2::ZERO, Vec2::new(Field::LENGTH, Field::WIDTH));
        lines.segment(
            Vec2::new(0.0, -Field::HALF_WIDTH),
            Vec2::new(0.0, Field::HALF_WIDTH),
        );
        lines.circle(Vec2::ZERO, Field::CENTRE_CIRCLE_RADIUS);
        lines.spot(Vec2::ZERO, 0.16);

        for side in [-1.0f32, 1.0] {
            let goal_line = side * Field::HALF_LENGTH;

            lines.rectangle(
                Vec2::new(goal_line - side * Field::PENALTY_AREA_DEPTH * 0.5, 0.0),
                Vec2::new(Field::PENALTY_AREA_DEPTH, Field::PENALTY_AREA_WIDTH),
            );
            lines.rectangle(
                Vec2::new(goal_line - side * Field::GOAL_AREA_DEPTH * 0.5, 0.0),
                Vec2::new(Field::GOAL_AREA_DEPTH, Field::GOAL_AREA_WIDTH),
            );

            let spot = Vec2::new(goal_line - side * Field::PENALTY_SPOT_DISTANCE, 0.0);
            lines.spot(spot, 0.16);

            // The penalty arc is the part of a 9.15 m circle around the spot
            // that falls outside the box.
            let reach = Field::PENALTY_AREA_DEPTH - Field::PENALTY_SPOT_DISTANCE;
            let half_sweep = (reach / Field::CENTRE_CIRCLE_RADIUS)
                .clamp(-1.0, 1.0)
                .acos();
            let facing = if side > 0.0 { PI } else { 0.0 };
            lines.arc(
                spot,
                Field::CENTRE_CIRCLE_RADIUS,
                facing - half_sweep,
                facing + half_sweep,
            );

            // Corner arcs sweep the quarter circle that faces into the pitch:
            // from the goal line round to the touchline.
            for touchline in [-1.0f32, 1.0] {
                let corner = Vec2::new(goal_line, touchline * Field::HALF_WIDTH);
                let from = -touchline * FRAC_PI_2;
                let to = if side > 0.0 { -touchline * PI } else { 0.0 };
                lines.arc(corner, Field::CORNER_ARC_RADIUS, from, to);
            }
        }

        let paint = materials.add(StandardMaterial {
            base_color: Color::srgb(0.93, 0.95, 0.93),
            perceptual_roughness: 0.9,
            cull_mode: None,
            ..default()
        });
        commands.spawn((Mesh3d(meshes.add(lines.build())), MeshMaterial3d(paint)));
    }

    /// The ground the pitch sits in: advertising hoardings around the touchlines
    /// and the low bowl of a stand behind them.
    ///
    /// Without it the turf simply stops and eleven-a-side is played in a void.
    /// Only three sides are built — the near one is where the broadcast gantry
    /// itself sits, so a stand there would be between the camera and the play.
    fn spawn_ground(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
    ) {
        const SIDE_MARGIN: f32 = 3.4;
        const END_MARGIN: f32 = 4.6;
        const HOARDING_HEIGHT: f32 = 0.95;
        const HOARDING_DEPTH: f32 = 0.14;

        let board = materials.add(StandardMaterial {
            base_color: Color::srgb(0.07, 0.09, 0.13),
            perceptual_roughness: 0.65,
            ..default()
        });
        // The lit strip along the top of the hoardings. It is the one crisp
        // line in the background, and it is what draws the edge of the playing
        // surface from a camera looking down onto it.
        let trim = materials.add(StandardMaterial {
            base_color: Color::srgb(0.22, 0.27, 0.36),
            emissive: LinearRgba::rgb(0.10, 0.13, 0.19),
            perceptual_roughness: 0.4,
            ..default()
        });
        let stand = materials.add(StandardMaterial {
            base_color: Color::srgb(0.055, 0.062, 0.085),
            perceptual_roughness: 1.0,
            ..default()
        });

        let along = Field::HALF_LENGTH + END_MARGIN;
        let across = Field::HALF_WIDTH + SIDE_MARGIN;
        for (size, position) in [
            (
                Vec3::new(along * 2.0, HOARDING_HEIGHT, HOARDING_DEPTH),
                Vec3::new(0.0, 0.0, across),
            ),
            (
                Vec3::new(along * 2.0, HOARDING_HEIGHT, HOARDING_DEPTH),
                Vec3::new(0.0, 0.0, -across),
            ),
            (
                Vec3::new(HOARDING_DEPTH, HOARDING_HEIGHT, across * 2.0),
                Vec3::new(along, 0.0, 0.0),
            ),
            (
                Vec3::new(HOARDING_DEPTH, HOARDING_HEIGHT, across * 2.0),
                Vec3::new(-along, 0.0, 0.0),
            ),
        ] {
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
                MeshMaterial3d(board.clone()),
                Transform::from_translation(position + Vec3::Y * HOARDING_HEIGHT * 0.5),
            ));
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(size.x, 0.07, size.z + 0.04))),
                MeshMaterial3d(trim.clone()),
                Transform::from_translation(position + Vec3::Y * HOARDING_HEIGHT),
            ));
        }

        // Terracing, as three slabs tilted up out of the surround. At this
        // distance the fog has most of them, which is the point: they give the
        // ground a horizon without ever competing with the play for attention.
        let far = Self::terrace(across + 2.1, 26.5, 13.4);
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(along * 2.0 + 30.0, 0.8, far.length))),
            MeshMaterial3d(stand.clone()),
            Transform::from_xyz(0.0, far.height, far.reach)
                .with_rotation(Quat::from_rotation_x(-far.pitch)),
        ));

        let end = Self::terrace(along + 2.4, 23.5, 11.4);
        for side in [-1.0f32, 1.0] {
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(end.length, 0.8, across * 2.0 + 24.0))),
                MeshMaterial3d(stand.clone()),
                Transform::from_xyz(side * end.reach, end.height, 0.0)
                    .with_rotation(Quat::from_rotation_z(side * end.pitch)),
            ));
        }
    }

    /// Where to put a slab of terracing that starts `from` metres out and
    /// climbs `rise` metres over `run`.
    fn terrace(from: f32, run: f32, rise: f32) -> Terrace {
        Terrace {
            length: (run * run + rise * rise).sqrt(),
            reach: from + run * 0.5,
            height: rise * 0.5 + 0.6,
            pitch: rise.atan2(run),
        }
    }

    fn spawn_goals(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
    ) {
        const POST_RADIUS: f32 = 0.07;
        const NET_DEPTH: f32 = 1.9;

        let frame = materials.add(StandardMaterial {
            base_color: Color::srgb(0.97, 0.97, 0.97),
            perceptual_roughness: 0.5,
            ..default()
        });
        let netting = materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 1.0, 1.0, 0.09),
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            double_sided: true,
            perceptual_roughness: 1.0,
            ..default()
        });

        let post = meshes.add(Cylinder::new(POST_RADIUS, Field::GOAL_HEIGHT));
        let bar = meshes.add(Cylinder::new(
            POST_RADIUS,
            Field::GOAL_WIDTH + POST_RADIUS * 2.0,
        ));
        let half_goal = Field::GOAL_WIDTH * 0.5;

        for side in [-1.0f32, 1.0] {
            let goal_line = side * Field::HALF_LENGTH;

            for post_side in [-1.0f32, 1.0] {
                commands.spawn((
                    Mesh3d(post.clone()),
                    MeshMaterial3d(frame.clone()),
                    Transform::from_xyz(goal_line, Field::GOAL_HEIGHT * 0.5, post_side * half_goal),
                ));
            }

            commands.spawn((
                Mesh3d(bar.clone()),
                MeshMaterial3d(frame.clone()),
                Transform::from_xyz(goal_line, Field::GOAL_HEIGHT, 0.0)
                    .with_rotation(Quat::from_rotation_x(FRAC_PI_2)),
            ));

            let behind = goal_line + side * NET_DEPTH;
            let middle = goal_line + side * NET_DEPTH * 0.5;
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.04, Field::GOAL_HEIGHT, Field::GOAL_WIDTH))),
                MeshMaterial3d(netting.clone()),
                Transform::from_xyz(behind, Field::GOAL_HEIGHT * 0.5, 0.0),
            ));
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(NET_DEPTH, 0.04, Field::GOAL_WIDTH))),
                MeshMaterial3d(netting.clone()),
                Transform::from_xyz(middle, Field::GOAL_HEIGHT, 0.0),
            ));
            for post_side in [-1.0f32, 1.0] {
                commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(NET_DEPTH, Field::GOAL_HEIGHT, 0.04))),
                    MeshMaterial3d(netting.clone()),
                    Transform::from_xyz(middle, Field::GOAL_HEIGHT * 0.5, post_side * half_goal),
                ));
            }
        }
    }
}
