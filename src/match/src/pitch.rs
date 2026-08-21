use crate::field::Field;
use crate::net::Netting;
use crate::textures::Textures;
use bevy::asset::RenderAssetUsages;
use bevy::math::Affine2;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// One bank of seating, as the box it fills and the turn that points it at
/// the pitch.
///
/// A stand between the lens and the play is not scenery, it is a wall. The
/// rig used to have one place to stand, so there was one wall and the fix was
/// simply not to build it — the near touchline was left open because that is
/// where the gantry hangs. Now the rig walks all the way round, so all four
/// sides stand and whichever one it is inside steps aside instead: a camera
/// behind the goal is *in* the end stand, and not drawing that stand is what
/// being there looks like.
#[derive(Component)]
pub struct Bank {
    /// Turns a point from the world into the bank's own frame, where the box
    /// below is axis-aligned.
    frame: Quat,
    /// Half the width across the front.
    flank: f32,
    /// Centre spot to the front row. There is deliberately no far bound —
    /// see [`Bank::encloses`] for why the test is a half-space in depth.
    near: f32,
    /// Ceiling on the test: above this the lens is treated as looking over
    /// the bank rather than through it. Higher than the seating actually
    /// stands — see `SIGHTLINE_CLEARANCE`.
    top: f32,
}

impl Bank {
    /// Is this world-space point inside the bank, or behind it?
    ///
    /// "Behind" counts: a lens further out than the front row has the
    /// whole structure between it and the play, which is the same wall
    /// whether the rig is among the seats or up in the back row. So the
    /// test is a half-space in depth, bounded across the front and in
    /// height — not the closed box, which would pop the stand back on
    /// the moment the camera drifted past the top of it.
    fn encloses(&self, point: Vec3) -> bool {
        let local = self.frame * point;
        local.x.abs() <= self.flank && local.z >= self.near && local.y <= self.top
    }

    /// Hide whichever banks the camera is standing in.
    ///
    /// Without this the fourth stand is simply a wall across the shot for
    /// the quarter of the arc the rig spends behind it — which is why the
    /// near touchline was left unbuilt when the camera had one place to
    /// stand. Now it is built and steps aside instead.
    ///
    /// At a corner the rig is behind two front faces at once and both go,
    /// which is correct: from there you are looking out through the gap
    /// between them.
    pub fn cull(
        camera: Single<&GlobalTransform, With<Camera3d>>,
        mut banks: Query<(&Bank, &mut Visibility)>,
    ) {
        let eye = camera.translation();
        for (bank, mut visibility) in &mut banks {
            let wanted = if bank.encloses(eye) {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
            // Only write on a change: `Visibility` is change-detected and
            // touching it every frame would dirty the whole hierarchy.
            if *visibility != wanted {
                *visibility = wanted;
            }
        }
    }
}

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

/// The stadium: turf, painted markings, both goals and the ground around them.
pub struct Pitch;

impl Pitch {
    /// Mowing stripes. An odd count keeps the two halves mirror-imaged.
    const STRIPES: usize = 15;
    const LINE_WIDTH: f32 = 0.14;
    const LINE_HEIGHT: f32 = 0.012;

    /// The direction the stadium light travels — down, and a little across.
    ///
    /// Public because the contact shadows have to agree with it. There are no
    /// shadow maps in this scene (see below), so every shadow on the turf is
    /// a painted disc placed by hand; a disc placed symmetrically under a body
    /// lit from 28° off the vertical is the one thing that makes a figure look
    /// pasted onto the grass rather than standing on it. One constant, two
    /// readers, no chance of them disagreeing.
    pub const SUN: Vec3 = Vec3::new(-0.4, -1.0, 0.35);

    /// The pitch as the mower left it: the shade the grass lies in going away
    /// from the roller, and the shade of the same grass lying back toward it.
    ///
    /// Sampled off broadcast football rather than picked off a colour wheel.
    /// Real grass on camera is a YELLOW-green — red comfortably ahead of blue —
    /// and considerably less saturated than people expect, because a stadium is
    /// lit flat and the camera is looking at dust, wear and seed heads as much
    /// as at leaf. A blue-shifted green is snooker baize or AstroTurf, which is
    /// what the first pair here (0.17 / 0.44 / 0.20) came out as.
    ///
    /// The two are the same grass mown in opposite directions and NOT two
    /// different greens. Leaf bent away from you reflects more sky and looks
    /// lighter and slightly cooler; leaf bent toward you shows its shadowed
    /// side and looks darker and a touch greyer. So the pair differs by about
    /// 16% in luminance — the real figure for mowing stripes — and only
    /// slightly in hue. Making them differ by brightness alone is what makes
    /// stripes look painted on.
    ///
    /// Both are a third off the pair they replace (#497434 / #3A6230), which
    /// were lit like a midday friendly.
    ///
    /// A third off the ALBEDO is nothing like a third off the picture, and the
    /// gap is worth knowing before reaching in here: the scene is tonemapped,
    /// and the tonemapper spends most of a change this size compressing it.
    /// Measured on the same broadcast frame, four patches of turf, with the
    /// upper stand and the hoarding as controls (both moved under 0.3%, so
    /// there is no exposure shift hiding in this):
    ///
    /// - a quarter off the albedo — the first attempt — moved the rendered
    ///   turf by 12%, which does not read as a darker pitch. It reads as the
    ///   same pitch.
    /// - a third off, which is what is here, moves it by 19%.
    ///
    ///   mown  #325223    against  #284620   (16% darker, a shade greyer)
    pub(crate) const MOWN: Color = Color::srgb(0.196, 0.323, 0.137);
    const AGAINST: Color = Color::srgb(0.156, 0.273, 0.126);

    /// How much pitch one tile of [`Textures::turf`] covers, in metres.
    ///
    /// Two, which against that texture's 1024 texels is 512 to the metre. Any
    /// larger and the blades go soft before the camera is down on the deck
    /// (`CameraFlight::HEIGHT` lets it to 0.6 m); any smaller and the tile
    /// repeats often enough that the eye can start hunting for the period.
    const TURF_TILE: f32 = 2.0;

    /// The surround, as a fraction of the pitch's own light in the space the
    /// shader multiplies in.
    ///
    /// The grass beyond the touchlines is the same turf, unlit and in the
    /// shadow of the stand, so it keeps the pitch's hue and simply loses most
    /// of its value — it was a near-black blue-green (0.05 / 0.13 / 0.07) that
    /// belonged to nothing else in the scene, and a surround in a different hue
    /// family from the pitch reads as a hole cut in the world rather than as
    /// ground. A fraction rather than a colour of its own for the same reason
    /// the stripes are: it now carries the same blade texture, so the only
    /// thing that may differ is the light falling on it, and darkening the
    /// pitch has to darken this with it or the hole comes back.
    ///
    /// Blue held up against red, because a shadow outdoors is lit by sky.
    const SHADOW: Color = Color::linear_rgb(0.165, 0.139, 0.232);

    pub fn spawn(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
    ) {
        Self::spawn_turf(&mut commands, &mut meshes, &mut materials, &mut images);
        Self::spawn_markings(&mut commands, &mut meshes, &mut materials);
        // Frame and netting both. The netting is no longer scenery — it is
        // deformable, and `Netting::ripple` drives it from the ball.
        Netting::spawn(&mut commands, &mut meshes, &mut materials, &mut images);
        Self::spawn_ground(&mut commands, &mut meshes, &mut materials, &mut images);

        // A stadium is lit from four corners at once, so almost nothing on the
        // pitch falls into true shadow. One directional light standing in for
        // all of them has to be paired with a generous ambient (see the camera,
        // which carries it): on its own it leaves every surface turned away
        // from it near black, which is fine on a disc and wrong on a body with
        // limbs, once the camera is close enough to see them shaded.
        commands.spawn((
            DirectionalLight {
                color: Color::srgb(1.0, 0.98, 0.92),
                illuminance: 8_500.0,
                // Deliberately off: the only thing tall enough to cast a
                // meaningful shadow is the ball, which carries its own contact
                // disc, and cascaded shadow maps are the single most expensive
                // thing this scene could ask a WebGL2 context for.
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.0).looking_at(Self::SUN, Vec3::Y),
        ));
    }

    /// The ground itself: the grass inside the touchlines, and the grass
    /// outside them.
    ///
    /// One texture for all of it. The blades are painted in [`Self::MOWN`], and
    /// every surface here is that same grass under a different light — the
    /// stripe lying the other way, the shadow of the stand — so each is a TINT
    /// on the one sheet rather than a colour of its own. Which is the point: a
    /// second green written down anywhere in here is a green that can drift
    /// away from the first, and a pitch whose stripes are two different greens
    /// is a pitch with stripes painted on it.
    fn spawn_turf(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
    ) {
        let grass = Textures::turf(images, Self::MOWN);

        // The second stripe, as a fraction of the first channel by channel, in
        // the LINEAR space the shader multiplies in. Derived from the pair
        // rather than written down, so the 16% is the 16% and cannot be typed
        // in twice.
        let mown = LinearRgba::from(Self::MOWN);
        let against = LinearRgba::from(Self::AGAINST);
        let turned = Color::linear_rgb(
            against.red / mown.red,
            against.green / mown.green,
            against.blue / mown.blue,
        );

        let stripe_length = Field::LENGTH / Self::STRIPES as f32;
        // One tile of grass to every `TURF_TILE` metres, on both axes, so the
        // blades come out square on the ground instead of combed by the shape
        // of whichever quad is carrying them.
        //
        // Every stripe starts the tile at the same phase, which is only
        // harmless because the texture holds nothing bigger than a leaf: there
        // is no feature at 7 m for the repeat to make visible, and the seam
        // between one stripe and the next already has a change of shade across
        // it. Put anything larger in that texture and this needs a per-stripe
        // offset — and a material per stripe to carry it.
        //
        // White for the first: the sheet is already painted in `MOWN`, so the
        // stripe the mower left going away is the grass exactly as drawn.
        let stripes = [Color::WHITE, turned].map(|tint| {
            materials.add(StandardMaterial {
                base_color: tint,
                base_color_texture: Some(grass.clone()),
                uv_transform: Affine2::from_scale(Vec2::new(
                    stripe_length / Self::TURF_TILE,
                    Field::WIDTH / Self::TURF_TILE,
                )),
                perceptual_roughness: 1.0,
                ..default()
            })
        });

        let stripe = meshes.add(Plane3d::default().mesh().size(stripe_length, Field::WIDTH));
        for index in 0..Self::STRIPES {
            let centre = -Field::HALF_LENGTH + stripe_length * (index as f32 + 0.5);
            commands.spawn((
                Mesh3d(stripe.clone()),
                MeshMaterial3d(stripes[index % 2].clone()),
                Transform::from_xyz(centre, 0.0, 0.0),
            ));
        }

        // Beyond the touchlines. Same grass at the same scale, so the pitch
        // does not stop at a change of texture as well as a change of light.
        const SURROUND: Vec2 = Vec2::new(320.0, 260.0);
        let surround = materials.add(StandardMaterial {
            base_color: Self::SHADOW,
            base_color_texture: Some(grass),
            uv_transform: Affine2::from_scale(SURROUND / Self::TURF_TILE),
            perceptual_roughness: 1.0,
            ..default()
        });
        // ⚠ **Set 1 cm below the turf, not 5.** It only ever had to clear
        // the turf's own z-fighting, and 5 cm was free while nothing was
        // ever drawn out here — the ball was snapped back onto the pitch
        // the instant it crossed a line. It is not free now: a ball put out
        // of play runs into this strip and comes to rest on it (`RunOff`),
        // and the engine puts it at height zero, so at −0.05 it floated a
        // visible finger's width above the ground with its contact shadow
        // (pinned at +0.02) hanging under it. The fetching player's boots
        // did the same.
        commands.spawn((
            Mesh3d(meshes.add(Plane3d::default().mesh().size(SURROUND.x, SURROUND.y))),
            MeshMaterial3d(surround),
            Transform::from_xyz(0.0, -0.01, 0.0),
        ));
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

    /// Folds one piece of scenery into a buffer that is accumulating others,
    /// seeding the buffer if it is the first.
    ///
    /// The whole stadium is static: not one vertex of it moves for the length
    /// of a match, and nothing in it is ever culled except the bank the lens
    /// has walked into. So its natural unit is the buffer, not the entity —
    /// and the entity is what this viewer pays for. Measured on a machine
    /// where the scene renders in the same 3.9 ms at 1280x720 and at
    /// 3840x2160, so the pixels are free and the walk is not.
    fn gather(buffer: &mut Option<Mesh>, piece: Mesh) {
        match buffer {
            Some(gathered) => gathered
                .merge(&piece)
                .expect("the stadium is built out of cuboids and rectangles"),
            None => *buffer = Some(piece),
        }
    }

    /// The ground the pitch sits in: advertising hoardings around the touchlines
    /// and the low bowl of a stand behind them.
    ///
    /// Without it the turf simply stops and eleven-a-side is played in a void.
    /// All four sides are built, including the one the broadcast gantry hangs
    /// over: a rig that can be walked round the ground would otherwise find a
    /// hole in it. [`Bank`] takes the one in the way back out again.
    fn spawn_ground(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
    ) {
        const SIDE_MARGIN: f32 = 3.4;
        const END_MARGIN: f32 = 4.6;
        const HOARDING_HEIGHT: f32 = 0.95;
        const HOARDING_DEPTH: f32 = 0.14;
        // How wide one advert reads on the boards, in metres. Matched to the
        // panel `Textures::hoarding` draws — 512 texels against 64 for the
        // 0.95 m height — so a texel is square and the wordmark is neither
        // stretched nor squashed.
        const AD_PANEL: f32 = 7.6;

        // Advertising hoardings. Lifted out of near-black too, but only to a
        // mid tone: these run the full length of the touchline right at the
        // edge of the play, and a bright band there pulls the eye off the
        // ball in a way the stands behind them do not.
        let board = materials.add(StandardMaterial {
            base_color: Color::srgb(0.200, 0.230, 0.300),
            perceptual_roughness: 0.65,
            ..default()
        });
        // The lit strip along the top of the hoardings. It is the one crisp
        // line in the background, and it is what draws the edge of the playing
        // surface from a camera looking down onto it.
        // Now that the stands behind it are pale, the walkway has to be paler
        // still or it stops reading as lit — at 0.22 it would be the darkest
        // thing on a light structure, which is the opposite of a lit strip.
        let trim = materials.add(StandardMaterial {
            base_color: Color::srgb(0.86, 0.90, 0.96),
            emissive: LinearRgba::rgb(0.16, 0.19, 0.24),
            perceptual_roughness: 0.4,
            ..default()
        });

        // What the boards actually advertise. One panel, tiled — which is what
        // a ground with a single perimeter sponsor looks like, and the only
        // honest thing to put here: this is the project's own address, not an
        // invented brand.
        let advert = Textures::hoarding(images, "OPEN-FOOTBALL.ORG");

        let along = Field::HALF_LENGTH + END_MARGIN;
        let across = Field::HALF_WIDTH + SIDE_MARGIN;
        // The perimeter, accumulated rather than spawned: one buffer for the
        // boards, one for the lit strip along their tops, and one per distinct
        // advert repeat.
        let mut boards: Option<Mesh> = None;
        let mut trims: Option<Mesh> = None;
        let mut adverts: Vec<(f32, Mesh)> = Vec::new();
        // `turn` points a panel at the pitch: a `Rectangle` faces +Z, and
        // rotating about Y by `turn` carries that onto `(sin, 0, cos)`.
        for (size, position, length, turn) in [
            (
                Vec3::new(along * 2.0, HOARDING_HEIGHT, HOARDING_DEPTH),
                Vec3::new(0.0, 0.0, across),
                along * 2.0,
                PI,
            ),
            (
                Vec3::new(along * 2.0, HOARDING_HEIGHT, HOARDING_DEPTH),
                Vec3::new(0.0, 0.0, -across),
                along * 2.0,
                0.0,
            ),
            (
                Vec3::new(HOARDING_DEPTH, HOARDING_HEIGHT, across * 2.0),
                Vec3::new(along, 0.0, 0.0),
                across * 2.0,
                -FRAC_PI_2,
            ),
            (
                Vec3::new(HOARDING_DEPTH, HOARDING_HEIGHT, across * 2.0),
                Vec3::new(-along, 0.0, 0.0),
                across * 2.0,
                FRAC_PI_2,
            ),
        ] {
            // The four sides go into two buffers rather than eight entities.
            // Same reasoning as the terraces below: a perimeter board is
            // scenery that never moves and is never culled apart from its
            // neighbours, so an entity each buys nothing and costs a walk, an
            // extract and a submit apiece on every frame.
            Self::gather(
                &mut boards,
                Mesh::from(Cuboid::new(size.x, size.y, size.z))
                    .translated_by(position + Vec3::Y * HOARDING_HEIGHT * 0.5),
            );
            Self::gather(
                &mut trims,
                Mesh::from(Cuboid::new(size.x, 0.07, size.z + 0.04))
                    .translated_by(position + Vec3::Y * HOARDING_HEIGHT),
            );

            // The advert itself, as a face hung a few millimetres in front of
            // the board rather than as a texture on the box. The box is a
            // cuboid: texturing it would print the wordmark on the top, the
            // back and both ends as well, and the top is where the lit trim
            // goes.
            //
            // A whole number of panels per side, so the tiling seam lands
            // between two words instead of through the middle of one. The
            // two sides of a ground are different lengths, hence a material
            // each — the repeat count is the only thing that differs.
            //
            // The two long sides carry the same repeat as each other and so do
            // the two ends, so this is two meshes and two materials for four
            // faces rather than four of each.
            let panels = (length / AD_PANEL).round().max(1.0);
            let facing = Quat::from_rotation_y(turn);
            let face = Mesh::from(Rectangle::new(length, HOARDING_HEIGHT)).transformed_by(
                Transform::from_translation(
                    position
                        + Vec3::Y * HOARDING_HEIGHT * 0.5
                        + facing * Vec3::Z * (HOARDING_DEPTH * 0.5 + 0.006),
                )
                .with_rotation(facing),
            );
            match adverts.iter_mut().find(|(repeat, _)| *repeat == panels) {
                Some((_, gathered)) => gathered
                    .merge(&face)
                    .expect("every face is the same rectangle"),
                None => adverts.push((panels, face)),
            }
        }

        for (panels, face) in adverts {
            commands.spawn((
                Mesh3d(meshes.add(face)),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: Some(advert.clone()),
                    // Perimeter boards at a floodlit ground are lit panels,
                    // and the emissive TEXTURE is what keeps that honest: the
                    // wordmark glows and the dark ground behind it does not,
                    // where a flat emissive would have the whole board give
                    // off light like a lightbox.
                    //
                    // Held to roughly the lit trim's level above, and for the
                    // reason the boards are a mid tone rather than a bright
                    // one in the first place: this runs the full length of the
                    // touchline at the very edge of the play, and anything
                    // here that out-glows the one crisp line in the background
                    // is competing with the ball for the eye.
                    emissive: LinearRgba::rgb(0.20, 0.24, 0.30),
                    emissive_texture: Some(advert.clone()),
                    uv_transform: Affine2::from_scale(Vec2::new(panels, 1.0)),
                    perceptual_roughness: 0.55,
                    ..default()
                })),
            ));
        }

        if let Some(mesh) = boards {
            commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(board)));
        }
        if let Some(mesh) = trims {
            commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(trim.clone())));
        }

        // Four banks of seating. Empty: no crowd, just the structure, and
        // open to the sky — none of these stands is roofed.
        //
        // These were one tilted slab each — a smooth ramp, which from the
        // camera is a flat grey triangle and reads as scenery rather than as
        // a stand. What makes a stand recognisable at distance is not detail
        // but RELIEF: stepped rows catching the light one edge at a time,
        // now read against the sky rather than against a wall. That is cheap,
        // and with the lens widened it is a good deal more visible than it
        // used to be.
        // The seats themselves, as a texture across the face of every row —
        // see `Textures::seats` for why they cannot be geometry. White base
        // colour so the image supplies the colour unmodified.
        let seating = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(Textures::seats(images)),
            perceptual_roughness: 0.95,
            ..default()
        });

        let across_span = along * 2.0 + 30.0;
        let end_span = across * 2.0 + 24.0;

        // Both touchlines. The near one used to be left out because the
        // broadcast gantry hangs over it and a stand there is a wall
        // across the shot — but the rig walks all the way round the
        // ground now, so leaving it out is a hole in the stadium from
        // three quarters of the arc. It is built like the others and
        // `Bank::cull` takes out whichever one the lens is inside, which
        // is what standing in a stand actually looks like.
        for turn in [0.0, PI] {
            Self::spawn_stand(
                commands,
                meshes,
                &seating,
                &trim,
                across_span,
                across + 2.1,
                26.5,
                13.4,
                turn,
            );
        }
        // Both ends, rotated a quarter turn so their rows recede down the x
        // axis instead of the z one — one each way, which is what puts them
        // behind opposite goals.
        for turn in [FRAC_PI_2, -FRAC_PI_2] {
            Self::spawn_stand(
                commands,
                meshes,
                &seating,
                &trim,
                end_span,
                along + 2.4,
                23.5,
                11.4,
                turn,
            );
        }
    }

    /// One bank of empty seating, built in its own local space — rows
    /// receding along +Z and climbing in +Y from the front row at the origin
    /// — and then turned to face the pitch.
    ///
    /// `turn` is the rotation about Y that points the stand inward: zero for
    /// the far side, a quarter turn either way for the ends.
    #[allow(clippy::too_many_arguments)]
    fn spawn_stand(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        seating: &Handle<StandardMaterial>,
        trim: &Handle<StandardMaterial>,
        length: f32,
        from: f32,
        run: f32,
        rise: f32,
        turn: f32,
    ) {
        /// Depth of one row of seats, front to back. Real terracing is about
        /// 0.8 m; a little deeper here keeps the row count — and so the
        /// entity count — sensible for a background element that spends most
        /// of its life in the fog.
        const TREAD: f32 = 1.25;
        /// Fraction of the way up the lit walkway runs.
        const TIER: f32 = 0.35;
        /// How far above the back row the cull still counts a camera as being
        /// inside the bank.
        ///
        /// This is NOT the height of anything. The banks are open and their
        /// structure stops at the crest — but a lens above the crest is not
        /// therefore looking over the stand. The broadcast rest shot sits at
        /// `TvCamera::HEIGHT`, 18 m up and 82 m out, which clears a touchline
        /// bank's 13.4 m crest by nearly five metres and is still looking
        /// straight THROUGH its back rows at the play. So the ceiling is a
        /// sightline margin, and it has to stay above that shot: cut it back
        /// to the crest and the near stand reappears across the default view,
        /// which is the one thing [`Bank`] exists to prevent.
        ///
        /// It used to be the height of the back wall, which happened to serve.
        /// With the wall gone the number has to be justified on its own.
        const SIGHTLINE_CLEARANCE: f32 = 7.3;

        let rows = (run / TREAD).round().max(4.0);
        let riser = rise / rows;
        let rows = rows as usize;

        // Every row of this bank, in ONE buffer.
        //
        // They used to be an entity each — twenty-one to a touchline bank,
        // nineteen to an end, eighty-four across the ground — sharing a mesh
        // and a material and differing only in where they sat. That is exactly
        // the shape of thing this viewer cannot afford: the frame is spent
        // per-entity, not per-pixel (the scene renders in the same 3.9 ms at
        // 1280x720 and at 3840x2160), so eighty-four rows cost eighty-four
        // times the walk, the extract and the submit for one flight of steps.
        //
        // Nothing is lost by merging. A stand is stepped concrete that never
        // moves, and it is culled as a unit anyway — `Bank::cull` hides the
        // whole bank or none of it, and no row was ever in shot without the
        // rows either side of it.
        let mut terrace = Mesh::from(Cuboid::new(length, riser * 1.9, TREAD * 0.96));
        for row in 1..rows {
            let up = riser * row as f32;
            let back = TREAD * row as f32;
            terrace
                .merge(&Mesh::from(Cuboid::new(
                    length,
                    riser * 1.9,
                    TREAD * 0.96,
                )).translated_by(Vec3::new(0.0, up, back)))
                .expect("every row is the same cuboid");
        }
        let terrace = meshes.add(terrace);

        // The bank's own extent, so `Bank::cull` can tell whether the
        // lens has walked into this one. A metre of slack either side of the
        // seating, so a camera just off the end of a bank still counts as
        // being behind it.
        let bank_extent = Bank {
            // World → local is the inverse of the placement turn.
            frame: Quat::from_rotation_y(-turn),
            flank: length * 0.5 + 1.0,
            near: from,
            top: rise + SIGHTLINE_CLEARANCE,
        };

        let placement = Transform::from_rotation(Quat::from_rotation_y(turn));
        let anchor = commands
            .spawn((placement, Visibility::default(), bank_extent))
            .id();

        commands.entity(anchor).with_children(|bank| {
            // The merged flight, placed where its first row used to sit — the
            // rest are built off that one inside the mesh.
            bank.spawn((
                Mesh3d(terrace.clone()),
                MeshMaterial3d(seating.clone()),
                Transform::from_xyz(0.0, riser * 0.5, from + TREAD * 0.5),
            ));

            // The walkway that splits the tiers. Deliberately not along the
            // crest — the camera looks UP at these from below their top, so a
            // line drawn on the crest is never in shot. A third of the way up
            // is, and it is the one thing that stops the background reading as
            // a flat wall.
            let tier_row = rows as f32 * TIER;
            bank.spawn((
                Mesh3d(meshes.add(Cuboid::new(length, 0.5, 1.1))),
                MeshMaterial3d(trim.clone()),
                Transform::from_xyz(0.0, riser * tier_row, from + TREAD * tier_row),
            ));

            // No back wall. There used to be one — a slab of pale concrete
            // standing seven metres above the back row — on the reasoning that
            // a bank open to the sky reads as a ramp with nothing behind it.
            // In practice it read as what it was: a flat grey rectangle
            // filling the top of the frame on every side of the ground, and it
            // took more attention than the seating in front of it. The rows
            // now finish against the sky, which is where the eye expects the
            // structure of an open ground to stop.
        });
    }
}
