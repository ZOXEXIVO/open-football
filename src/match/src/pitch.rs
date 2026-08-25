use crate::field::Field;
use crate::net::Netting;
use crate::textures::{Textures, Turf};
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

/// The playing surface as one mesh, with the state of the grass written into
/// its vertices.
///
/// [`Textures::turf`] draws a leaf and nothing larger, and says so at length:
/// the tile repeats every two metres, and anything in it above blade scale
/// comes back as a two-metre lattice of blotches, because the eye finds a grid
/// at any contrast once it repeats. That note ends by saying the unevenness a
/// real pitch has is at the scale of a penalty area, which is not something a
/// tile can hold.
///
/// This is where it goes instead. A grid of vertices over the playing surface
/// carries a field authored ONCE across the whole ground in world space — so
/// there is nothing to repeat, and features may be any size they like. Three
/// things ride on it:
///
/// - **The mow.** The stripe shade was a material each and a mesh each; it is
///   a vertex colour now, which is what lets the rest of this share one
///   surface with it.
/// - **Wear.** A pitch that has had a match played on it is not uniform: the
///   goalmouths are scuffed through, the penalty spots and the centre spot are
///   worn discs, and those three are most of what tells a played pitch from a
///   painted rectangle at the distance a broadcast camera watches from.
/// - **Unevenness.** Slow variation at ten and twenty metres — a sward is
///   laid, drained and shaded unevenly, and no real one is a single tone.
///
/// The stripes keep a hard edge because each block gets its OWN vertices along
/// the seam: a mown edge is far sharper than a half-metre cell could
/// interpolate, and sharing vertices across it would smear the one boundary in
/// here that is genuinely crisp.
///
/// Cost: this replaces fifteen entities with one, and the grid is some fifty
/// thousand triangles the GPU never notices — the frame is spent per-entity,
/// not per-pixel or per-vertex (see `perf`, and the measurement that the scene
/// renders in the same time at 720p and at 4K).
struct Sward {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    /// Which mow stripe this vertex is in, as the multiplier on the sheet.
    tints: Vec<Vec3>,
    /// What the ground itself is doing here — wear and unevenness together,
    /// before the pass that takes the average back out.
    ground: Vec<Vec3>,
    indices: Vec<u32>,
}

impl Sward {
    /// How far apart the vertices are, in metres.
    ///
    /// Set by the smallest thing the field has to hold, which is the worn disc
    /// around a penalty spot — a couple of metres across, so half a metre puts
    /// four or five cells through it and it arrives round rather than
    /// diamond-shaped. Halving this again quadruples a triangle count that is
    /// already free; it would not make anything look better.
    const CELL: f32 = 0.5;

    /// What wearing the grass away does to its colour, per channel, at the
    /// point where it is worn through completely.
    ///
    /// Up in red, barely up in green, DOWN in blue. Worn turf is not simply
    /// darker grass or lighter grass: it is thin grass with the dry stuff
    /// under it showing through, so it loses its blue and gains a little
    /// overall value. The same axis [`Textures::turf`] runs a drying leaf
    /// along, at the scale of a goalmouth rather than of a leaf.
    ///
    /// Deliberately short of what a February goalmouth looks like. This is a
    /// multiplier on a pitch whose colour has been calibrated against
    /// broadcast footage, and the mud-bath version of it reads as a different
    /// green rather than as the same green worn.
    const WORN: Vec3 = Vec3::new(0.22, 0.10, -0.06);

    /// The playing surface, mown in [`Pitch::STRIPES`] bands.
    ///
    /// `turned` is the second stripe as a fraction of the first, and `tile` is
    /// how much ground one repeat of the blade sheet covers.
    fn mow(turned: Vec3, tile: f32) -> Mesh {
        let mut sward = Sward {
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            uvs: Vec::new(),
            tints: Vec::new(),
            ground: Vec::new(),
            indices: Vec::new(),
        };
        let stripe = Field::LENGTH / Pitch::STRIPES as f32;
        for index in 0..Pitch::STRIPES {
            let from = -Field::HALF_LENGTH + stripe * index as f32;
            // White for the even bands: the sheet is already painted in
            // `Pitch::MOWN`, so the stripe the mower left going away is the
            // grass exactly as drawn.
            let tint = if index % 2 == 0 { Vec3::ONE } else { turned };
            sward.block(from, from + stripe, tint, tile);
        }
        sward.build()
    }

    /// The grass beyond the touchlines: the same field with the mow left out
    /// of it.
    ///
    /// It gets the treatment for the same reason the pitch does and rather
    /// more urgently — it is a single flat quad three hundred metres across,
    /// which is as flat as anything in the scene can be, and half of every low
    /// shot is made of it. No stripes, because a mower does not go out here,
    /// and no wear, because nobody plays on it.
    ///
    /// Coarser than the playing surface by a factor of eight: there is nothing
    /// out here at the scale of a penalty spot to resolve, and this quad is
    /// eleven times the area of the pitch.
    fn rough(size: Vec2, tile: f32) -> Mesh {
        let mut sward = Sward {
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            uvs: Vec::new(),
            tints: Vec::new(),
            ground: Vec::new(),
            indices: Vec::new(),
        };
        sward.grid(
            Vec2::new(-size.x * 0.5, -size.y * 0.5),
            Vec2::new(size.x * 0.5, size.y * 0.5),
            Self::CELL * 8.0,
            Vec3::ONE,
            tile,
            false,
        );
        sward.build()
    }

    /// One mown band, with its own vertices along both seams.
    fn block(&mut self, from: f32, to: f32, tint: Vec3, tile: f32) {
        self.grid(
            Vec2::new(from, -Field::HALF_WIDTH),
            Vec2::new(to, Field::HALF_WIDTH),
            Self::CELL,
            tint,
            tile,
            true,
        );
    }

    /// A rectangle of ground, with its own vertices all the way round.
    fn grid(&mut self, from: Vec2, to: Vec2, cell: f32, tint: Vec3, tile: f32, played: bool) {
        let span = to - from;
        let down = (span.x / cell).ceil().max(1.0) as usize;
        let across = (span.y / cell).ceil().max(1.0) as usize;
        let base = self.positions.len() as u32;

        for row in 0..=down {
            let x = from.x + span.x * (row as f32 / down as f32);
            for column in 0..=across {
                let z = from.y + span.y * (column as f32 / across as f32);
                self.positions.push([x, 0.0, z]);
                self.normals.push([0.0, 1.0, 0.0]);
                // The surface is flat and axis-aligned, so the tangent frame
                // is a constant: U runs along +X with the sheet, V along +Z,
                // and the handedness that puts the bitangent on +Z with a +Y
                // normal is negative. Written out rather than generated
                // because `generate_tangents` would solve a system to arrive
                // at the same four numbers for every vertex on the pitch.
                self.tangents.push([1.0, 0.0, 0.0, -1.0]);
                // Straight off the world position, so one continuous sheet
                // runs across the whole surface. The stripes used to restart
                // the tile at every band, which was safe only as long as the
                // sheet held nothing bigger than a leaf.
                self.uvs.push([x / tile, z / tile]);
                self.tints.push(tint);
                self.ground.push(Self::ground(Vec2::new(x, z), played));
            }
        }

        let stride = (across + 1) as u32;
        for row in 0..down as u32 {
            for column in 0..across as u32 {
                let corner = base + row * stride + column;
                self.indices.extend_from_slice(&[
                    corner,
                    corner + 1,
                    corner + stride,
                    corner + 1,
                    corner + stride + 1,
                    corner + stride,
                ]);
            }
        }
    }

    /// What the grass is doing at one point, as a multiplier on the mown
    /// shade — before normalisation, so this is free to have any average it
    /// likes and [`Self::build`] takes it back out.
    fn ground(point: Vec2, played: bool) -> Vec3 {
        // `played` rather than trusting the wear field to be zero out here:
        // the surround runs UNDER the pitch, so its grid samples the
        // goalmouths through it. Nothing of that is ever seen — it is a
        // centimetre below opaque turf — but it would land in the average
        // this is normalised against, and quietly grade the ground outside
        // the touchlines by what happens inside them.
        let worn = if played { Self::wear(point) } else { 0.0 };
        Vec3::new(
            1.0 + Self::WORN.x * worn,
            1.0 + Self::WORN.y * worn,
            1.0 + Self::WORN.z * worn,
        ) * (1.0 + Self::unevenness(point))
    }

    /// How hard the grass here has been used, nought to one.
    ///
    /// Three sources, and `max` rather than a sum because they overlap: the
    /// penalty spot sits inside the goalmouth, and adding them would take that
    /// one patch past bare earth while the rest of the pitch stayed green.
    fn wear(point: Vec2) -> f32 {
        // The goalmouth. An ellipse rather than a disc, and wider across than
        // it is deep, because what wears is the ground a keeper covers and the
        // ground defenders turn on in front of him — the goal area and a good
        // way either side of it. Centred four metres off the line: the very
        // back of it is behind the keeper and sees less traffic than the front
        // edge of the six-yard box.
        let goalmouths = [-1.0f32, 1.0]
            .map(|side| {
                Self::blob(
                    point - Vec2::new(side * (Field::HALF_LENGTH - 4.0), 0.0),
                    Vec2::new(8.0, 12.0),
                )
            })
            .into_iter()
            .fold(0.0f32, f32::max);

        // The penalty spots, which are stood on, run up to and dug out of.
        let spots = [-1.0f32, 1.0]
            .map(|side| {
                Self::blob(
                    point
                        - Vec2::new(
                            side * (Field::HALF_LENGTH - Field::PENALTY_SPOT_DISTANCE),
                            0.0,
                        ),
                    Vec2::splat(2.6),
                ) * 0.55
            })
            .into_iter()
            .fold(0.0f32, f32::max);

        // And the centre spot: every kickoff, and every restart after a goal.
        let centre = Self::blob(point, Vec2::splat(3.6)) * 0.45;

        goalmouths.max(spots).max(centre)
    }

    /// A soft patch: one at the middle, nought at `radius` and beyond, with no
    /// corner at either end.
    fn blob(offset: Vec2, radius: Vec2) -> f32 {
        let reach = (offset / radius).length();
        if reach >= 1.0 {
            return 0.0;
        }
        let fade = 1.0 - reach;
        fade * fade * (3.0 - 2.0 * fade)
    }

    /// The aimless variation of a real sward, as a fraction either way.
    ///
    /// Two halves, and they are doing different jobs:
    ///
    /// **The drift**, two sinusoid pairs at twenty and ten metres, at periods
    /// with no common factor between them or with the seven-metre mow — so the
    /// eye finds no grid in it, which is the entire failure this had to avoid.
    /// This is drainage and shade: the slow business of one end of a ground
    /// being a little greener than the other.
    ///
    /// **The mottle**, patchiness at two to five metres. This is the half that
    /// actually stops the pitch reading as printed card, and the half a
    /// texture could never have supplied: it lives at exactly the scale that
    /// makes a two-metre tile visible as a lattice. Value noise rather than
    /// more sinusoids, because real turf is patchy and not wavy — the sward
    /// takes better in one place than another and the join between them is not
    /// a smooth curve.
    ///
    /// Seven per cent at the very extreme and three or four typically. On a
    /// surface this large that is the difference between ground and a sheet of
    /// paper, and it stays under half the sixteen per cent the mow carries so
    /// it can never be mistaken for a stripe — which
    /// `the_sward_never_shouts_over_the_mow` holds it to.
    fn unevenness(point: Vec2) -> f32 {
        let drift = 0.022 * (point.x / 23.7 + 0.6).sin() * (point.y / 17.3 - 1.1).cos()
            + 0.014 * (point.x / 9.1 - 2.2).sin() * (point.y / 11.7 + 0.4).cos();
        let mottle = 0.026 * Self::grain(point / 4.7)
            + 0.014 * Self::grain(point / 1.9 + Vec2::new(31.4, 17.2));
        drift + mottle
    }

    /// Value noise on a one-unit lattice: a hashed height at every corner,
    /// eased between. Comes back in −1..1.
    fn grain(point: Vec2) -> f32 {
        let cell = point.floor();
        let across = point - cell;
        let ease = across * across * (Vec2::splat(3.0) - 2.0 * across);
        let corner = |x: f32, y: f32| Self::hash(cell + Vec2::new(x, y));
        let near = corner(0.0, 0.0) + (corner(1.0, 0.0) - corner(0.0, 0.0)) * ease.x;
        let far = corner(0.0, 1.0) + (corner(1.0, 1.0) - corner(0.0, 1.0)) * ease.x;
        (near + (far - near) * ease.y) * 2.0 - 1.0
    }

    /// One lattice corner's value, nought to one.
    ///
    /// Integer coordinates go through a wrapping cast on purpose: a pitch runs
    /// either side of the origin, and a hash that folded −3 onto 3 would
    /// mirror the whole field about the halfway line.
    fn hash(cell: Vec2) -> f32 {
        let mut state = (cell.x as i32 as u32)
            .wrapping_mul(374_761_393)
            .wrapping_add((cell.y as i32 as u32).wrapping_mul(668_265_263));
        state ^= state >> 13;
        state = state.wrapping_mul(1_274_126_177);
        state ^= state >> 16;
        state as f32 / u32::MAX as f32
    }

    /// Normalises the ground term to an average of one and folds it into the
    /// mow.
    ///
    /// The pitch's colour is calibrated — see [`Pitch::MOWN`], and the note
    /// there about a third off the albedo being nineteen per cent off the
    /// picture. Wear and unevenness are meant to say where the grass is
    /// different, not what colour the grass is, so whatever they do to the
    /// average is taken back out here. The mow is untouched by it: the pair of
    /// shades and their sixteen per cent survive exactly as written.
    fn build(mut self) -> Mesh {
        let mut mean = Vec3::ZERO;
        for ground in &self.ground {
            mean += *ground;
        }
        mean /= self.ground.len().max(1) as f32;
        let gain = Vec3::new(1.0 / mean.x, 1.0 / mean.y, 1.0 / mean.z);

        let colours: Vec<[f32; 4]> = self
            .tints
            .iter()
            .zip(&self.ground)
            .map(|(tint, ground)| {
                let colour = *tint * *ground * gain;
                [colour.x, colour.y, colour.z, 1.0]
            })
            .collect();

        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            std::mem::take(&mut self.positions),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, std::mem::take(&mut self.normals))
        .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, std::mem::take(&mut self.tangents))
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, std::mem::take(&mut self.uvs))
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colours)
        .with_inserted_indices(Indices::U32(std::mem::take(&mut self.indices)))
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

    /// **How much ground there is outside the playing surface**, in metres:
    /// across the touchlines, and behind the goal lines. The advertising
    /// hoardings stand at the end of it and the banks of seating begin two
    /// metres beyond them, so this pair is the whole run-off — everything
    /// between the paint and the first thing a camera can walk into.
    ///
    /// Read by `ChangeoverShot`, which puts a lens on the grass a few metres
    /// behind a man and has to know where the ground stops.
    pub const SIDE_MARGIN: f32 = 3.4;
    pub const END_MARGIN: f32 = 4.6;

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

    /// The playing surface and the light that falls on it — the first course
    /// of the bring-up.
    ///
    /// Split from the rest of the stadium on purpose: this is one of the four
    /// materials whose shader takes about six seconds to link in a browser
    /// that has not seen it before, and a course per frame is what gives the
    /// page the thread back between them. See [`crate::bringup`].
    pub fn lay_turf(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
    ) {
        let grass = Textures::turf(&mut images, Self::MOWN);
        Self::spawn_playing_surface(&mut commands, &mut meshes, &mut materials, &grass);
        // Kept for the surround, which is laid on the next frame off the same
        // sheet — generating it twice would cost a second 1024-square texture
        // and its mip chain for a picture nobody could tell apart.
        commands.insert_resource(grass);

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
    fn spawn_playing_surface(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        grass: &Turf,
    ) {
        // The second stripe, as a fraction of the first channel by channel, in
        // the LINEAR space the shader multiplies in. Derived from the pair
        // rather than written down, so the 16% is the 16% and cannot be typed
        // in twice.
        let mown = LinearRgba::from(Self::MOWN);
        let against = LinearRgba::from(Self::AGAINST);
        let turned = Vec3::new(
            against.red / mown.red,
            against.green / mown.green,
            against.blue / mown.blue,
        );

        // One material for the whole playing surface. Both mow shades, the
        // wear and the unevenness are vertex colours on [`Sward`] — which is
        // what a second material could never have carried, since the thing
        // that varies varies from metre to metre and not from stripe to
        // stripe.
        //
        // White base colour, because the sheet is already painted in `MOWN`
        // and every vertex says what to do to it.
        let playing_surface = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(grass.albedo.clone()),
            normal_map_texture: Some(grass.relief.clone()),
            perceptual_roughness: 1.0,
            ..default()
        });
        commands.spawn((
            Mesh3d(meshes.add(Sward::mow(turned, Self::TURF_TILE))),
            MeshMaterial3d(playing_surface),
        ));
    }

    /// The grass beyond the touchlines — the second course of the bring-up.
    ///
    /// A frame of its own rather than a few lines under the playing surface,
    /// and for one reason: it is the same vertex layout with a DIFFERENT
    /// shader — see the note on the relief below — so it is a second program
    /// for the driver to compile, and the two of them next to each other are
    /// what made the first frame of a match twelve seconds long.
    pub fn lay_surround(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        grass: Res<Turf>,
    ) {
        // Same grass at the same scale, so the pitch does not stop at a change
        // of texture as well as a change of light.
        const SURROUND: Vec2 = Vec2::new(320.0, 260.0);
        // Same sheet, same scale, and deliberately WITHOUT the relief the
        // playing surface carries.
        //
        // A normal map earns its place by shading: it is what stops a lit
        // plane returning one value everywhere. Out here there is no light to
        // shade with — `SHADOW` puts this ground at about a sixth of the
        // pitch's value, because it is in the lee of a stand — so the term the
        // second sheet contributes is a sixth of a shading difference that was
        // subtle at full brightness, which is to say nothing anybody has ever
        // seen. What it costs is not nothing: this quad is eleven times the
        // area of the pitch and, by the note on `Sward::rough`, half of every
        // low shot, so dropping it takes a full anisotropic texture fetch and
        // the whole tangent-space path out of the fragment shader over most of
        // the lower frame. The blades stay — that is the albedo, and it is
        // what keeps the surround from reading as a hole cut in the world.
        let surround = materials.add(StandardMaterial {
            base_color: Self::SHADOW,
            base_color_texture: Some(grass.albedo.clone()),
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
            Mesh3d(meshes.add(Sward::rough(SURROUND, Self::TURF_TILE))),
            MeshMaterial3d(surround),
            Transform::from_xyz(0.0, -0.01, 0.0),
        ));
    }

    /// The paint — the third course of the bring-up.
    pub fn paint_markings(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
    ) {
        Self::spawn_markings(&mut commands, &mut meshes, &mut materials);
    }

    /// Both goals, frame and netting — the fourth course. The netting is not
    /// scenery: it is deformable, and [`Netting::ripple`] drives it from the
    /// ball.
    pub fn raise_goals(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
    ) {
        Netting::spawn(&mut commands, &mut meshes, &mut materials, &mut images);
    }

    /// Hoardings, stands and the ground they stand on — the last course.
    pub fn build_stands(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
    ) {
        Self::spawn_ground(&mut commands, &mut meshes, &mut materials, &mut images);
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

        let along = Field::HALF_LENGTH + Self::END_MARGIN;
        let across = Field::HALF_WIDTH + Self::SIDE_MARGIN;
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
                .merge(
                    &Mesh::from(Cuboid::new(length, riser * 1.9, TREAD * 0.96))
                        .translated_by(Vec3::new(0.0, up, back)),
                )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the vertex colours back off a built sward.
    fn colours(mesh: &Mesh) -> Vec<Vec3> {
        match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(bevy::mesh::VertexAttributeValues::Float32x4(values)) => values
                .iter()
                .map(|colour| Vec3::new(colour[0], colour[1], colour[2]))
                .collect(),
            _ => panic!("the sward carries no vertex colours"),
        }
    }

    fn turned() -> Vec3 {
        let mown = LinearRgba::from(Pitch::MOWN);
        let against = LinearRgba::from(Pitch::AGAINST);
        Vec3::new(
            against.red / mown.red,
            against.green / mown.green,
            against.blue / mown.blue,
        )
    }

    /// The contract the whole field rests on, and the same one
    /// [`Textures::turf`] holds for the sheet: wear and unevenness say where
    /// the grass is different, NOT what colour the grass is.
    ///
    /// `Pitch::MOWN` is calibrated against broadcast footage and the note on
    /// it records what a third off the albedo did to the picture. Anything in
    /// here that moved the average would be re-grading the pitch by the back
    /// door — and would do it silently, because the pitch would still look
    /// like a pitch.
    #[test]
    fn wearing_the_grass_does_not_repaint_it() {
        let turned = turned();
        let mesh = Sward::mow(turned, Pitch::TURF_TILE);
        let colours = colours(&mesh);

        let mut worn = Vec3::ZERO;
        for colour in &colours {
            worn += *colour;
        }
        worn /= colours.len() as f32;

        // What the same mow comes to with a perfectly even sward under it:
        // every vertex is either the sheet as drawn or the sheet turned, and
        // the bands are equal in area, so the average is decided by the
        // stripe count alone.
        let bands = Pitch::STRIPES as f32;
        let against = (Pitch::STRIPES / 2) as f32;
        let flat = (Vec3::ONE * (bands - against) + turned * against) / bands;

        for channel in 0..3 {
            assert!(
                (worn[channel] - flat[channel]).abs() < 0.004,
                "channel {channel} averaged {} against a flat sward's {}",
                worn[channel],
                flat[channel]
            );
        }
    }

    /// …and having established that it does not move the average, that it
    /// does actually do something. A field that normalised itself to nothing
    /// would pass the test above perfectly.
    #[test]
    fn a_played_pitch_is_worn_where_it_is_played_on() {
        let goalmouth = Sward::wear(Vec2::new(Field::HALF_LENGTH - 4.0, 0.0));
        let spot = Sward::wear(Vec2::new(
            Field::HALF_LENGTH - Field::PENALTY_SPOT_DISTANCE,
            0.0,
        ));
        let centre = Sward::wear(Vec2::ZERO);
        // A corner is the one part of a pitch nobody spends a match on.
        let corner = Sward::wear(Vec2::new(Field::HALF_LENGTH - 1.0, Field::HALF_WIDTH - 1.0));

        assert!(goalmouth > 0.9, "the goalmouth goes bare: {goalmouth}");
        assert!(spot > 0.2, "the penalty spot is stood on: {spot}");
        assert!(centre > 0.2, "every kickoff is here: {centre}");
        assert!(corner < 0.05, "nothing happens in the corner: {corner}");
    }

    /// The mow has to stay the loudest thing on the surface. Unevenness that
    /// approached the stripe's own contrast would stop reading as ground and
    /// start reading as a second, wrong set of stripes.
    #[test]
    fn the_sward_never_shouts_over_the_mow() {
        let stripe = 1.0 - turned().y;
        let mut worst = 0.0f32;
        // Every half metre of the playing surface, which is where the field is
        // sampled in earnest.
        let mut x = -Field::HALF_LENGTH;
        while x <= Field::HALF_LENGTH {
            let mut z = -Field::HALF_WIDTH;
            while z <= Field::HALF_WIDTH {
                worst = worst.max(Sward::unevenness(Vec2::new(x, z)).abs());
                z += 0.5;
            }
            x += 0.5;
        }
        assert!(
            worst < stripe * 0.5,
            "unevenness reached {worst} against a mow of {stripe}"
        );
    }
}
