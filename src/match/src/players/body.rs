use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
#[cfg(test)]
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::art::textures::FaceLayout;
use crate::players::actors::Actors;
use crate::players::kit::Outfit;

/// One cross-section of a body part: an ellipse of half-widths `x` (across the
/// body) and `z` (front to back) at height `y`, in the part's own space, and
/// how far the section as a whole sits FORWARD of the part's own axis.
///
/// That last number is what stops every part being a body of revolution. A
/// head is not a barrel: the chin and the brow stand in front of the ear line
/// and the occiput hangs a good three centimetres behind it. Neither is a
/// chest, or a seat, or a calf. Without an offset the only shape a profile can
/// describe is a tube, which is exactly what a figure assembled out of them
/// reads as — and no amount of extra rings fixes it, because the problem is
/// the symmetry rather than the resolution.
#[derive(Clone, Copy)]
struct Ring {
    y: f32,
    x: f32,
    z: f32,
    offset: f32,
    /// How SQUARE the section is: 2 is a true ellipse, and above that the
    /// outline fills out toward the corners of the box that bounds it.
    ///
    /// The second thing that stops a part being a barrel, and the one the
    /// offset cannot say. An ellipse is the section of a bottle. A ribcage is
    /// flat across the front and flat across the back and turns hard at the
    /// sides, a shoulder is nearly a slab, and a leg of a pair of shorts is
    /// cloth hanging in a rounded box — none of them ellipses, and all of them
    /// drawn as one here until now. What that reads as, on a torso, is a
    /// smooth blown shape with no bones in it, which is most of what makes a
    /// figure look like a moulded toy rather than a person in a shirt.
    ///
    /// The semi-axes are untouched at any value: `x` is still exactly the
    /// half-width and `z` still exactly the half-depth, so every measurement
    /// taken along an axis — and every part derived from another — means the
    /// same thing it did. Only the quarters between them move.
    edge: f32,
}

impl Ring {
    /// A plain ellipse, which is what most of a body is.
    const ROUNDED: f32 = 2.0;

    const fn oval(y: f32, x: f32, z: f32) -> Self {
        Ring {
            y,
            x,
            z,
            offset: 0.0,
            edge: Self::ROUNDED,
        }
    }

    /// An ellipse carried `offset` metres in front of the axis.
    const fn set(y: f32, x: f32, z: f32, offset: f32) -> Self {
        Ring {
            y,
            x,
            z,
            offset,
            edge: Self::ROUNDED,
        }
    }

    /// …and the same, squared off by `edge` — see the field.
    const fn squared(y: f32, x: f32, z: f32, offset: f32, edge: f32) -> Self {
        Ring {
            y,
            x,
            z,
            offset,
            edge,
        }
    }

    /// The point `angle` round this section, as (across, forward) in the
    /// part's own space, with the offset already in it.
    ///
    /// The Lamé curve |x/a|^edge + |z/b|^edge = 1, walked by angle. Vertices
    /// bunch toward the corners as the section squares off, which is where a
    /// lathe wants them: the corner is the only part of a squared section
    /// whose curvature the eye can pick out.
    /// How far out a point lies in units of this section: 1.0 is exactly on
    /// it.
    ///
    /// The inverse of [`Ring::at`] along a ray, and the only honest ruler for
    /// a section that is not an ellipse — measured as an ellipse, a point
    /// sitting on a squared-off corner reads as 8% proud of a surface it is
    /// in fact touching.
    fn radius(self, across: f32, forward: f32) -> f32 {
        let side = (across / self.x).abs();
        let along = ((forward - self.offset) / self.z).abs();
        (side.powf(self.edge) + along.powf(self.edge)).powf(1.0 / self.edge)
    }

    fn at(self, angle: f32) -> (f32, f32) {
        let (sin, cos) = angle.sin_cos();
        if (self.edge - Self::ROUNDED).abs() < 1e-3 {
            return (cos * self.x, self.offset + sin * self.z);
        }
        let bulge = 2.0 / self.edge;
        (
            cos.signum() * cos.abs().powf(bulge) * self.x,
            self.offset + sin.signum() * sin.abs().powf(bulge) * self.z,
        )
    }

    /// The section `t` of the way from this one to `other`, on the smooth
    /// curve that also passes through the sections either side of them.
    ///
    /// Catmull-Rom on the radii, straight on `y`. A radius is clamped at zero
    /// because a curve is free to overshoot and a profile that pinches to a
    /// point — the crown of a head, the tip of a nose — would otherwise come
    /// back out the far side as an inside-out spike.
    fn through(self, before: Ring, next: Ring, after: Ring, t: f32) -> Ring {
        let curve = |a: f32, b: f32, c: f32, d: f32| {
            0.5 * ((2.0 * b)
                + (c - a) * t
                + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t
                + (3.0 * b - a - 3.0 * c + d) * t * t * t)
        };
        Ring {
            y: self.y + (next.y - self.y) * t,
            x: curve(before.x, self.x, next.x, after.x).max(0.0),
            z: curve(before.z, self.z, next.z, after.z).max(0.0),
            offset: curve(before.offset, self.offset, next.offset, after.offset),
            // Straight, not splined. A curve through four squarenesses is free
            // to overshoot below 2, and an `edge` under 2 is not a rounder
            // section — it is a four-pointed star.
            edge: self.edge + (next.edge - self.edge) * t,
        }
    }

    /// The section `t` of the way from this one to `other`.
    fn lerp(self, other: Ring, t: f32) -> Ring {
        Ring {
            y: self.y + (other.y - self.y) * t,
            x: self.x + (other.x - self.x) * t,
            z: self.z + (other.z - self.z) * t,
            offset: self.offset + (other.offset - self.offset) * t,
            edge: self.edge + (other.edge - self.edge) * t,
        }
    }

    /// The same section, `swell` metres bigger all round.
    ///
    /// How everything worn ON a body part is derived from the part itself —
    /// hair off the skull, a collar off the shirt — rather than written out
    /// again as its own list of numbers that then has to be kept in step.
    fn swollen(self, swell: f32) -> Ring {
        Ring {
            y: self.y,
            x: self.x + swell,
            z: self.z + swell,
            offset: self.offset,
            edge: self.edge,
        }
    }

    /// The section a cap of hair takes over this one: `swell` metres proud
    /// across and behind, and `clearance` metres SHORT of the front.
    ///
    /// The two have to be separate. Setting a hair ring back the obvious way —
    /// shifting the whole ellipse — buries the forehead and drags the back of
    /// the cap out behind the skull by the same amount, which at the height of
    /// the ears is a mullet. What a hairline actually is, is the cap coming
    /// forward over the top of the head while the FRONT of it stays behind the
    /// face; only the front edge moves.
    fn capped(self, swell: f32, clearance: f32) -> Ring {
        let front = self.offset + self.z - clearance;
        let back = self.offset - self.z - swell;
        let mut cap = Ring {
            y: self.y,
            x: self.x + swell,
            z: ((front - back) * 0.5).max(1e-3),
            offset: (front + back) * 0.5,
            edge: self.edge,
        };
        // Once the cap is meant to be OVER the head rather than buried in it,
        // make sure it actually is.
        //
        // The two ellipses do not share a centre — the cap's is pushed back to
        // put its front edge where the hairline wants it — so giving each axis
        // the margin it was asked for does not contain the skull between the
        // axes. The shortfall is under a millimetre and it shows: a ring of
        // bare scalp comes through the hair at the crown, forty-five degrees
        // round from the face, and every player takes the field wearing a
        // tonsure. Measured round the section rather than reasoned about,
        // because reasoning about it is exactly what missed it.
        if clearance <= 0.0 {
            let mut worst: f32 = 1.0;
            for step in 0..16 {
                let (across, along) = self.at(TAU * step as f32 / 16.0);
                worst = worst.max(cap.radius(across, along));
            }
            cap.x *= worst;
            cap.z *= worst;
        }
        cap
    }
}

/// Lathes stacked ellipses into a closed, smooth-shaded mesh.
///
/// Every part of a footballer — a thigh, the torso, a skull — is a tube through
/// a handful of cross-sections, so one lathe covers the whole model and the
/// shapes stay editable as a list of numbers rather than as geometry. Nothing
/// here is loaded from disk: a glTF character would cost more to ship than the
/// entire viewer.
struct Sculptor {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    /// Pairs of vertices that are the same point on the surface, written twice
    /// so the texture can have a seam. See [`Sculptor::loft`].
    seams: Vec<(u32, u32)>,
    /// Sides around each ring of THIS part. A limb and a head do not want the
    /// same number: nobody has ever looked at a shin, and a face is the one
    /// surface on the pitch the eye goes to first.
    sides: usize,
}

impl Sculptor {
    /// Sides around each ring.
    ///
    /// Sixteen was chosen against a broadcast camera a hundred metres away,
    /// where the silhouette error is a third of a pixel and nobody could have
    /// seen it. The camera can now be flown to arm's length, and at that range
    /// the count that matters is not the silhouette's — the shading is smooth
    /// across a facet and creased AT it, so sixteen flat panels round a torso
    /// read as sixteen flat panels. Thirty-two halves the crease.
    ///
    /// It costs almost nothing here. Every mesh is shared by all twenty-two
    /// players, so this doubles a few thousand vertices ONCE; what a scene of
    /// footballers actually costs is the ~350 draw calls, and that number does
    /// not move.
    /// Sixty-four now, and the reason is the SECTION rather than the
    /// silhouette. A squared-off section (see [`Ring::edge`]) spends most of
    /// its curvature in the four corners, so a facet count that read as smooth
    /// on an ellipse leaves a visible chamfer there — and the corners of a
    /// chest and a pair of shorts are exactly where the eye looks for the
    /// shape. Every mesh is shared by all twenty-two, and the frame is spent
    /// per-ENTITY rather than per-triangle (see [`Sculptor::joined`]), so what
    /// this buys is paid for once in memory.
    const SIDES: usize = 64;
    /// And for the head, which carries a face.
    ///
    /// At sixteen the front of a skull was four vertices wide, so an eye
    /// landed between two of them and the texture sheared across the facets.
    /// A head is also the one part of a footballer anybody looks AT.
    const HEAD_SIDES: usize = 80;
    /// And for the small round parts — see [`Sculptor::ellipsoid`].
    const BLOB_SIDES: usize = 32;
    /// Rings from pole to pole on a sphere.
    const STACKS: usize = 18;
    /// How many rings are lathed between each pair a profile is written with.
    ///
    /// The control points are the shape as it is AUTHORED — a dozen numbers a
    /// human can read and edit — and the mesh is the smooth curve through
    /// them rather than the polyline between them. That distinction is most of
    /// what separates a limb from a stack of truncated cones: the silhouette
    /// error of a straight span is second order and invisible, but the CREASE
    /// where two spans meet is first order and is exactly what the eye reads
    /// as "assembled out of parts".
    ///
    /// Five rather than three, for the same money as [`Sculptor::SIDES`] and
    /// for the same reason. It matters most where a span is long and the
    /// profile is turning through it — the roll of the shirt's hem, the seat,
    /// the curl of the fingers — which is everywhere a piece of clothing
    /// changes direction.
    const CURVE: usize = 5;

    fn new(sides: usize) -> Self {
        Sculptor {
            positions: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            seams: Vec::new(),
            sides,
        }
    }

    /// A part described by its profile, from either end.
    fn part(rings: &[Ring]) -> Mesh {
        Self::part_at(rings, Self::SIDES)
    }

    /// The same, at a chosen resolution.
    fn part_at(rings: &[Ring], sides: usize) -> Mesh {
        Self::lathe(&Self::curved(rings), sides)
    }

    /// And a profile that is ALREADY dense, lathed as written.
    ///
    /// For anything sampled off another part rather than authored — the hair
    /// and the sock's turnover — where the samples are millimetres apart and
    /// running a curve through them again would triple the triangles for a
    /// shape that is already the curve.
    fn lathe(rings: &[Ring], sides: usize) -> Mesh {
        let mut sculptor = Sculptor::new(sides);
        sculptor.loft(rings, Vec3::ZERO);
        sculptor.build()
    }

    /// A profile resampled through a smooth curve — see [`Sculptor::CURVE`].
    ///
    /// Catmull-Rom through the control points, on the RADII only: `y` is the
    /// variable the profile is a function of rather than part of its shape,
    /// and splining it would let a section overshoot its neighbours and leave
    /// the list no longer ascending, which is the one thing
    /// [`Sculptor::section`] cannot survive.
    ///
    /// Everything derived from a part must come through here too, or the
    /// derivation is measuring a shape the mesh does not have.
    fn curved(rings: &[Ring]) -> Vec<Ring> {
        if rings.len() < 3 {
            return rings.to_vec();
        }
        let last = rings.len() - 1;
        let mut out = Vec::with_capacity(last * Self::CURVE + 1);
        for span in 0..last {
            let before = rings[span.saturating_sub(1)];
            let start = rings[span];
            let end = rings[span + 1];
            let after = rings[(span + 2).min(last)];
            for step in 0..Self::CURVE {
                out.push(start.through(before, end, after, step as f32 / Self::CURVE as f32));
            }
        }
        out.push(rings[last]);
        out
    }

    /// Two parts as one mesh, for a pair that never move relative to each
    /// other and are painted the same colour.
    ///
    /// Not a modelling convenience — a cost one. Every mesh on a player is an
    /// entity, and the viewer's frame is spent almost entirely per-entity:
    /// measured on a machine where the pitch renders identically at 720p and
    /// at 4K, twenty-two players cost 2.3 ms of a 3.9 ms frame and the whole
    /// of that is Bevy walking, extracting and submitting their seven hundred
    /// parts. Anything rigidly fixed to its parent and wearing its parent's
    /// material is therefore an entity bought for nothing, and belongs in the
    /// parent's buffer instead — see the joint balls in [`BodyParts::new`],
    /// which is what this exists for.
    ///
    /// Both sides come out of [`Self::build`], so they carry the same
    /// attributes in the same order and the merge cannot fail.
    fn joined(mut base: Mesh, fixed: Mesh) -> Mesh {
        base.merge(&fixed)
            .expect("every sculpted part carries position, normal and UV");
        base
    }

    /// The same, for a piece that has to be turned and carried into place
    /// first: a finger off a knuckle, a thumb off the side of a palm.
    ///
    /// A lathe can only be built about its own axis, and five digits pointing
    /// five different ways are five axes. They are still ONE mesh, though,
    /// because they never move relative to the hand on an outfield player —
    /// which is the whole reason they can be afforded at all. See
    /// [`Self::joined`] for what an entity costs.
    fn placed(base: Mesh, part: Mesh, at: Transform) -> Mesh {
        Self::joined(base, part.transformed_by(at))
    }

    /// A rounded lump — a hand, a boot, the ball of a joint — sized on each
    /// axis.
    ///
    /// Coarser than the lathed parts on purpose: there are ten of these on a
    /// footballer and none of them is more than six centimetres across, where
    /// twenty-four sides put the silhouette error under half a millimetre.
    fn ellipsoid(radii: Vec3) -> Mesh {
        Self::ellipsoid_at(radii, Vec3::ZERO)
    }

    /// The same, carried off the origin it is modelled about.
    fn ellipsoid_at(radii: Vec3, at: Vec3) -> Mesh {
        let mut sculptor = Sculptor::new(Self::BLOB_SIDES);
        sculptor.loft(&Self::sphere(radii), at);
        sculptor.build()
    }

    /// Sphere profile, bottom pole to top pole.
    fn sphere(radii: Vec3) -> Vec<Ring> {
        (0..=Self::STACKS)
            .map(|stack| {
                let angle = PI * (1.0 - stack as f32 / Self::STACKS as f32);
                Ring {
                    y: radii.y * angle.cos(),
                    x: radii.x * angle.sin(),
                    z: radii.z * angle.sin(),
                    offset: 0.0,
                    edge: Ring::ROUNDED,
                }
            })
            .collect()
    }

    /// The cross-section a profile has at `y`, interpolated between the two
    /// rings either side of it and clamped at both ends.
    ///
    /// Wants an ASCENDING profile, which the two it is used on — the skull and
    /// the shirt — both are. Everything derived from a part rather than
    /// written out again comes through here.
    fn section(rings: &[Ring], y: f32) -> Ring {
        let last = rings.len() - 1;
        if y <= rings[0].y {
            return Ring { y, ..rings[0] };
        }
        for step in 0..last {
            let (below, above) = (rings[step], rings[step + 1]);
            if y <= above.y {
                let span = (above.y - below.y).max(f32::EPSILON);
                return below.lerp(above, (y - below.y) / span);
            }
        }
        Ring { y, ..rings[last] }
    }

    /// A band lathed over an existing profile — a collar over a shirt —
    /// between two heights.
    fn band(rings: &[Ring], from: f32, to: f32, steps: usize, swell: f32) -> Vec<Ring> {
        (0..=steps)
            .map(|step| {
                let y = from + (to - from) * step as f32 / steps as f32;
                Self::section(rings, y).swollen(swell)
            })
            .collect()
    }

    /// A curved sheet lying ON the surface of a lathed part: the panel a
    /// shirt number or a name is printed on.
    ///
    /// It has to be curved. A flat rectangle wide enough to carry a number
    /// stands the better part of four centimetres proud of the shirt at its
    /// corners, because a torso's cross-section falls away fast — which is
    /// what made the number read as a card pinned to a footballer rather than
    /// as something printed on him. Sampling the shirt's own profile puts the
    /// ink on the shirt.
    ///
    /// `bearing` is the angle its middle sits at (−π/2 is the back), `arc` how
    /// far round it wraps, and `lift` how far clear of the cloth it floats so
    /// the two never fight for depth.
    fn decal(rings: &[Ring], centre: f32, height: f32, bearing: f32, arc: f32, lift: f32) -> Mesh {
        /// Rows down the panel: the profile it follows is narrowing the whole
        /// way, so the panel is a saddle rather than a cylinder.
        const ROWS: usize = 8;
        const COLUMNS: usize = 20;

        let mut sculptor = Sculptor::new(0);
        for row in 0..=ROWS {
            // 0 at the TOP: image rows are written top down, and this is the
            // one place in the crate where a lathe's own bottom-up `v` would
            // print the text upside down.
            let down = row as f32 / ROWS as f32;
            let y = centre + height * (0.5 - down);
            let section = Self::section(rings, y);
            for column in 0..=COLUMNS {
                let across = column as f32 / COLUMNS as f32;
                // Left to right as the panel is READ, which is the opposite
                // way round the body: seen from behind, a player's right is on
                // the reader's left.
                let angle = bearing + (0.5 - across) * arc;
                let (side, forward) = section.swollen(lift).at(angle);
                sculptor.positions.push([side, y, forward]);
                sculptor.uvs.push([across, down]);
            }
        }

        let stride = COLUMNS as u32 + 1;
        for row in 0..ROWS as u32 {
            for column in 0..COLUMNS as u32 {
                let top_left = row * stride + column;
                let top_right = top_left + 1;
                let bottom_left = top_left + stride;
                let bottom_right = bottom_left + 1;
                // Wound so the face points AWAY from the body: rows descend
                // and columns run against the lathe's own winding, and the two
                // reversals cancel back to the order `loft` uses.
                sculptor.indices.extend_from_slice(&[
                    top_left,
                    bottom_left,
                    top_right,
                    top_right,
                    bottom_left,
                    bottom_right,
                ]);
            }
        }
        sculptor.build()
    }

    fn loft(&mut self, rings: &[Ring], offset: Vec3) {
        if rings.len() < 2 {
            return;
        }
        // Wound as though the profile always climbed, so a part described from
        // the top down — which every limb is — still ends up with its faces
        // pointing outward.
        let mut profile = rings.to_vec();
        if profile[0].y > profile[profile.len() - 1].y {
            profile.reverse();
        }

        let base = self.positions.len() as u32;
        let foot = profile[0].y;
        let span = (profile[profile.len() - 1].y - foot).max(f32::EPSILON);
        // One vertex more per ring than there are sides: the last one sits on
        // top of the first but carries `u = 1` instead of `u = 0`.
        //
        // Closing the ring back onto vertex zero — which this used to do — is
        // free right up until a part carries a texture, and then the last
        // facet interpolates from 0.97 back to 0.0 and squeezes the ENTIRE
        // image into one strip a few millimetres wide. Nothing lathed was
        // textured before the face was, so the seam had never shown.
        let stride = self.sides as u32 + 1;
        for ring in &profile {
            for side in 0..=self.sides {
                let angle = TAU * side as f32 / self.sides as f32;
                let (across, forward) = ring.at(angle);
                self.positions
                    .push([offset.x + across, offset.y + ring.y, offset.z + forward]);
                self.uvs
                    .push([side as f32 / self.sides as f32, (ring.y - foot) / span]);
            }
        }
        // The two halves of that seam are the same point on the model, so they
        // have to end up with the same normal — otherwise the split shows as a
        // hard crease straight down the side of every part.
        for step in 0..profile.len() as u32 {
            let row = base + step * stride;
            self.seams.push((row, row + self.sides as u32));
        }

        for step in 0..profile.len() as u32 - 1 {
            for side in 0..self.sides as u32 {
                let a = base + step * stride + side;
                let b = a + 1;
                let d = a + stride;
                let c = d + 1;
                self.indices.extend_from_slice(&[a, d, b, b, d, c]);
            }
        }

        self.cap(profile[0], offset, false);
        self.cap(profile[profile.len() - 1], offset, true);
    }

    /// Closes one end of a loft. Skipped where the profile has already pinched
    /// to a point, as the crown of a head does.
    fn cap(&mut self, ring: Ring, offset: Vec3, upward: bool) {
        if ring.x.abs() < 1e-4 && ring.z.abs() < 1e-4 {
            return;
        }
        let hub = self.positions.len() as u32;
        self.positions
            .push([offset.x, offset.y + ring.y, offset.z + ring.offset]);
        self.uvs.push([0.5, 0.5]);
        for side in 0..self.sides {
            let angle = TAU * side as f32 / self.sides as f32;
            let (across, forward) = ring.at(angle);
            self.positions
                .push([offset.x + across, offset.y + ring.y, offset.z + forward]);
            self.uvs
                .push([0.5 + angle.cos() * 0.5, 0.5 + angle.sin() * 0.5]);
        }

        let sides = self.sides as u32;
        for side in 0..sides {
            let next = (side + 1) % sides;
            if upward {
                self.indices
                    .extend_from_slice(&[hub, hub + 1 + next, hub + 1 + side]);
            } else {
                self.indices
                    .extend_from_slice(&[hub, hub + 1 + side, hub + 1 + next]);
            }
        }
    }

    /// Smooth normals from the faces meeting at each vertex. Ring vertices are
    /// shared the whole way round, so shading runs continuously over a limb;
    /// the caps carry their own copies, which keeps their rims crisp.
    fn build(self) -> Mesh {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for triangle in self.indices.chunks_exact(3) {
            let corners = [
                Vec3::from(self.positions[triangle[0] as usize]),
                Vec3::from(self.positions[triangle[1] as usize]),
                Vec3::from(self.positions[triangle[2] as usize]),
            ];
            // Left unnormalised on purpose: the cross product's length is twice
            // the triangle's area, which is the weighting a smooth normal wants.
            let face = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
            for index in triangle {
                normals[*index as usize] += face;
            }
        }
        // The UV seam is one point on the surface written down twice, so each
        // copy has only picked up the faces on its own side of the join. Add
        // them together and both get the whole neighbourhood back.
        for (first, second) in &self.seams {
            let shared = normals[*first as usize] + normals[*second as usize];
            normals[*first as usize] = shared;
            normals[*second as usize] = shared;
        }

        let normals: Vec<[f32; 3]> = normals
            .into_iter()
            .map(|normal| normal.try_normalize().unwrap_or(Vec3::Y).to_array())
            .collect();

        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// Where a footballer's joints sit, in metres above the turf. Both the meshes
/// and the rig that hangs them together read these, so the model can be made
/// taller or leggier by editing one block of numbers.
pub struct Physique;

impl Physique {
    pub const HIP: f32 = 0.95;
    pub const HIP_SPREAD: f32 = 0.088;
    pub const THIGH: f32 = 0.455;
    pub const SHIN: f32 = 0.455;
    /// The ankle, in the shin's own space — and therefore where the boot
    /// hangs, since the boot mesh is modelled about that point (its sole is
    /// 38 mm below it).
    ///
    /// One constant instead of the literal that used to be repeated at all
    /// three sites that walk the leg: the renderer, the software preview
    /// and the skeleton tests. They have to agree, and a number written
    /// down three times does not stay agreed — see the note on
    /// `Physique::CRADLE`.
    pub const ANKLE: Vec3 = Vec3::new(0.0, -Self::SHIN + 0.005, 0.035);
    /// Hip joint to the sole of the boot, straight-legged: the lever a
    /// stride swings through.
    ///
    /// It is what turns ground covered into an angle at the hip and back again,
    /// so it is the crossing between the stride model in
    /// [`crate::players::actors::Actors::stride_of`] and the geometry here —
    /// and, like [`Self::CRADLE`], a number that has to be one number.
    pub const LEG: f32 = Self::THIGH + Self::SHIN - 0.005 + 0.038;
    /// Hip to the base of the neck.
    pub const TORSO: f32 = 0.58;
    /// Shoulder joints, in the torso's own space.
    ///
    /// Deliberately sunk BENEATH and INSIDE the torso's shoulder crest
    /// (which peaks 0.203 wide at y=0.514). An arm socket level with the
    /// crest, as this was, leaves the top cap of the upper arm standing
    /// proud of the shirt as a separate rounded lump — the single thing
    /// that made the figures read as assembled out of parts. Buried, the
    /// only bit of arm you see above the armpit is the deltoid, which is
    /// how a shoulder actually looks.
    ///
    /// It is also the only spread the SLEEVE has to work with, and the two
    /// are 2.7 cm apart: everything the sleeve does over the top of the
    /// shoulder happens in that gap.
    pub const SHOULDER: f32 = 0.470;
    pub const SHOULDER_SPREAD: f32 = 0.176;
    pub const UPPER_ARM: f32 = 0.30;
    pub const FOREARM: f32 = 0.26;
    /// Crown to turf. Only the name plates want this, to size themselves
    /// against the player they belong to.
    pub const STATURE: f32 = 1.79;

    /// Where a goalkeeper holds the ball once he has gathered it, in his own
    /// space and before his build is applied.
    ///
    /// Not a free choice. It is worked forward down the arm from the shoulder
    /// socket through [`Joint::CRADLE_SHOULDER`] and [`Joint::CRADLE_ELBOW`],
    /// which put the wrists at (±0.184, 1.151, 0.325); the ball sits in the
    /// fork just above and between them. The viewer draws the ball here rather
    /// than at its recorded position while the keeper has it, so the two MUST
    /// come out of the same numbers — move one of the three and the gloves
    /// close on empty air.
    pub const CRADLE: Vec3 = Vec3::new(0.0, 1.20, 0.32);

    /// And where he holds it at full stretch, in the same space.
    ///
    /// Same discipline as [`Physique::CRADLE`], worked down the arm from
    /// [`Joint::REACH_SHOULDER`] / [`Joint::REACH_ELBOW`] with the claim
    /// closing the gloves to [`Joint::CLAIM_SPREAD`]: those put the wrists at
    /// (±0.072, 1.953, 0.054), and the ball sits a glove's length further
    /// along the arms, between the palms.
    ///
    /// Without it the ball stayed at the chest point through a diving catch —
    /// a keeper thrown full length with the ball hanging in mid-air where his
    /// sternum would have been if he had stayed on his feet.
    const CATCH: Vec3 = Vec3::new(0.0, 2.03, 0.06);
    /// How far the claim travels across his body as he turns onto the ball,
    /// in metres at a dive flat across the goal. The chest twist
    /// ([`Joint::DIVE_TWIST`]) carries both shoulders round with it, and the
    /// gloves go too.
    const CATCH_TURN: f32 = 0.13;

    /// Where a ball claimed at full stretch is drawn, for a dive with this
    /// much of a leading side. See [`Gait::lead`].
    pub fn catch(lead: f32) -> Vec3 {
        Self::CATCH + Vec3::X * (lead.clamp(-1.0, 1.0) * Self::CATCH_TURN)
    }

    /// Where one glove is, in the figure's own space, for a player posed like
    /// this — walked forward over the real [`Joint::pose`].
    ///
    /// The other two hold points on this struct are constants because they are
    /// each ONE pose a keeper holds a ball in, and a number can be checked
    /// against the arms that put it there. A throw-in is not one pose: the
    /// ball travels the whole sweep of both arms, from behind his head to out
    /// in front of it, and any constant would be right at a single instant of
    /// it and wrong for the rest. So this asks the rig instead — four
    /// quaternions, for at most one player on the pitch at a time.
    ///
    /// The WRIST, which is where the glove begins — and what
    /// [`Physique::CRADLE`] and [`Physique::CATCH`] are both worked down the
    /// arm to, so it is what the tests on them have to measure. Anything
    /// drawing a ball wants [`Physique::palm`] instead.
    #[cfg(test)]
    pub fn glove(side: f32, gait: Gait) -> Vec3 {
        Self::hand(side, gait).translation
    }

    /// Where a ball held in ONE hand is drawn: out along the hand rather than
    /// on the wrist joint, which is where the glove BEGINS. The drawn ball is
    /// 32 cm across — see [`crate::players::actors::Actors::BALL_RADIUS`] —
    /// so a centre on the wrist swallows the whole glove and half the forearm
    /// with it.
    pub fn palm(side: f32, gait: Gait) -> Vec3 {
        Self::hand(side, gait).transform_point(Vec3::new(0.0, -Self::PALM, 0.0))
    }

    /// And where a ball held in BOTH is: between them.
    pub fn hands(gait: Gait) -> Vec3 {
        (Self::palm(-1.0, gait) + Self::palm(1.0, gait)) * 0.5
    }

    /// **The parts of him that can end up inside the pitch**, in the
    /// figure's own space — the ENDS of his four limbs, and his head.
    ///
    /// Deliberately NOT the hips or the shoulders. The trunk's own height is
    /// [`Carriage::SETTLE`] and it is exact at both ends of its range: a man
    /// standing carries his hips at 0.95 and one lying on his side at
    /// [`Carriage::LYING`], which is the thickness of a body. Constrained on
    /// the hip JOINT a figure lying flat would be shoved a hand's width into
    /// the air.
    ///
    /// What that settle does not know about is a LIMB. `SETTLE·sin(tilt)` is
    /// right at 0° and at 90° and wrong in between, and the error goes the
    /// dangerous way: measured, a keeper landing twelve degrees off vertical
    /// had his hips at exactly the right height and a boot **14 cm inside the
    /// turf**, because a body rotating about its own hip drops eight times as
    /// fast as a straight leg swings up. See
    /// [`PlayerActor::lift`](crate::players::actors::PlayerActor), which is
    /// where the ground gets the last word.
    pub(crate) fn underside(gait: Gait) -> [Vec3; 9] {
        let hung = |limb: Limb, side: f32, origin: Vec3| {
            let joint = Joint::new(Entity::PLACEHOLDER, limb, side, origin);
            Transform::from_translation(joint.place(gait)).with_rotation(joint.pose(gait))
        };
        let leg = |side: f32| {
            let thigh = hung(
                Limb::Hip,
                side,
                Vec3::new(side * Self::HIP_SPREAD, Self::HIP, 0.0),
            );
            let knee = thigh.translation + thigh.rotation * Vec3::new(0.0, -Self::THIGH, 0.0);
            let shin = thigh * hung(Limb::Knee, side, Vec3::new(0.0, -Self::THIGH, 0.0));
            // Through the ankle, and to the sole of the boot rather than to
            // the joint: 38 mm, the same figure `skeleton::boot` measures.
            let sole = (shin * hung(Limb::Ankle, side, Self::ANKLE))
                .transform_point(Vec3::new(0.0, -0.038, 0.0));
            (knee, sole)
        };
        let arm = |side: f32| {
            let torso = hung(Limb::Torso, 0.0, Vec3::new(0.0, Self::HIP, 0.0));
            let upper = torso
                * hung(
                    Limb::Shoulder,
                    side,
                    Vec3::new(side * Self::SHOULDER_SPREAD, Self::SHOULDER, 0.0),
                );
            let elbow = upper.transform_point(Vec3::new(0.0, -Self::UPPER_ARM, 0.0));
            (elbow, Self::hand(side, gait).translation)
        };
        let (left_knee, left_boot) = leg(-1.0);
        let (right_knee, right_boot) = leg(1.0);
        let (left_elbow, left_glove) = arm(-1.0);
        let (right_elbow, right_glove) = arm(1.0);
        let head = (hung(Limb::Torso, 0.0, Vec3::new(0.0, Self::HIP, 0.0))
            * hung(Limb::Head, 0.0, Vec3::new(0.0, Self::TORSO, 0.0)))
        .transform_point(Vec3::new(0.0, 0.10, 0.0));
        [
            left_knee,
            right_knee,
            left_boot,
            right_boot,
            left_elbow,
            right_elbow,
            left_glove,
            right_glove,
            head,
        ]
    }

    /// One hand, walked forward down the arm.
    pub(crate) fn hand(side: f32, gait: Gait) -> Transform {
        let hung = |limb: Limb, origin: Vec3| {
            let joint = Joint::new(Entity::PLACEHOLDER, limb, side, origin);
            Transform::from_translation(joint.place(gait)).with_rotation(joint.pose(gait))
        };
        hung(Limb::Torso, Vec3::new(0.0, Self::HIP, 0.0))
            * hung(
                Limb::Shoulder,
                Vec3::new(side * Self::SHOULDER_SPREAD, Self::SHOULDER, 0.0),
            )
            * hung(Limb::Elbow, Vec3::new(0.0, -Self::UPPER_ARM, 0.0))
            * hung(
                Limb::Wrist,
                Vec3::new(0.0, -Self::FOREARM - Self::WRIST_DROP, 0.0),
            )
    }

    /// How far below the elbow the wrist hangs, past the forearm's own length,
    /// and how far past the wrist the middle of the palm is.
    pub const WRIST_DROP: f32 = 0.03;
    const PALM: f32 = 0.075;
}

/// Every mesh a footballer is made of, built once and shared by all
/// twenty-two: only the materials differ from player to player.
#[derive(Resource)]
pub struct BodyParts {
    torso: Handle<Mesh>,
    /// The neck of the shirt, in the club's trim colour. Derived from the
    /// torso rather than written out again — see [`Sculptor::band`].
    collar: Handle<Mesh>,
    pelvis: Handle<Mesh>,
    /// The skull, which is the whole head.
    ///
    /// A NOSE used to hang off the front of it and a pair of EARS off the
    /// sides, and both went for the same reason: a face is a picture of a
    /// real man now, and a photograph of a man has his own nose and his own
    /// ears in it, lit and shaded and the right shape. A lathe standing in
    /// front of that is a second nose in a slightly wrong colour, and an
    /// ellipsoid stuck to the side is a second ear — a pale button, since
    /// nothing round it had told it what colour he is (user, 2026-08-19:
    /// "remove ears from model because photo already has it"). The picture
    /// carries them now, as it carries his eyes and his mouth.
    head: Handle<Mesh>,
    /// One cap per hair style; `None` for the shaved head, which is the scalp
    /// itself with the stubble drawn onto the face texture.
    hair: [Option<Handle<Mesh>>; 4],
    upper_arm: Handle<Mesh>,
    sleeve: Handle<Mesh>,
    /// The band round the end of a sleeve, in the trim colour.
    cuff: Handle<Mesh>,
    forearm: Handle<Mesh>,
    /// A bare hand, left and right — see [`BodyParts::hand`] for why it is a
    /// pair rather than one mesh used twice. Indexed by side, `[0]` left.
    hand: [Handle<Mesh>; 2],
    /// A keeper's glove. Its own mesh rather than a scaled hand: the whole
    /// point of the thing is that it is broad and flat, and the two of them
    /// splayed at the end of a dive are what says *catching* from the stand.
    ///
    /// The back of the hand only, as far as the knuckles — the fingers are
    /// separate, because a mitt is a mitt however many rings you lathe it
    /// out of, and four fingers and a thumb are the whole difference between
    /// a glove and a bag on the end of an arm. They are the one part of a
    /// keeper the eye tracks through a save, so they are the one part worth
    /// the triangles.
    glove: Handle<Mesh>,
    /// One finger of it, instanced four times across the knuckles — in two
    /// segments, because an open hand and a closed one are the whole point
    /// and one hinge cannot draw the second — and the thumb, which is
    /// shorter and set forward and in.
    finger: Handle<Mesh>,
    fingertip: Handle<Mesh>,
    thumb: Handle<Mesh>,
    /// A keeper wears long sleeves. Two parts, because the arm is two: this
    /// one takes over from [`Self::sleeve`] on the upper arm and runs to the
    /// elbow, and [`Self::sleeve_forearm`] carries on from there.
    sleeve_long: Handle<Mesh>,
    sleeve_forearm: Handle<Mesh>,
    /// …and the trim band at the wrist end of it, which on a long sleeve is
    /// where a cuff actually is.
    cuff_forearm: Handle<Mesh>,
    shorts_leg: Handle<Mesh>,
    thigh: Handle<Mesh>,
    shin: Handle<Mesh>,
    sock_top: Handle<Mesh>,
    boot: Handle<Mesh>,
    /// The curved panels the shirt number and the player's name are printed
    /// on, both lying on the shirt's own profile.
    number: Handle<Mesh>,
    name: Handle<Mesh>,
}

impl BodyParts {
    /// Hips, waist, chest, shoulders — the shirt. A footballer's torso is a V:
    /// the waist is the narrowest point and the shoulders carry most of the
    /// width, which is most of what reads as "athlete" in a silhouette this
    /// size.
    ///
    /// A constant rather than an argument to one call, because four other
    /// things are derived from it: the collar, the two printed panels, and
    /// nothing may be allowed to drift out of step with the cloth it sits on.
    ///
    /// **The waist and the hips are what tell a man from a woman**, and this
    /// had them the wrong way round. The shoulder crest was 0.209 and the seat
    /// of the shorts 0.192 — a ratio of 1.09, which is the FEMALE figure
    /// almost exactly (biacromial over bi-iliac runs 1.03-1.10 on women and
    /// 1.35-1.50 on men). Worked as circumferences it was worse: a 70 cm waist
    /// under a 100 cm hip is a waist-to-hip of 0.70, and 0.70 is the number
    /// quoted for a woman's figure in every textbook that quotes one. A man's
    /// is 0.85-0.95. So the waist came up and out and the seat came in: 82 cm
    /// over 95, and a shoulder — measured where the eye measures it, across
    /// the sleeves — of 52 cm over a 35 cm hip.
    ///
    /// The other half of it is the HEM, which used to stop at 0.050 — world
    /// 1.00 m, above the navel — with the shorts' waistband standing five
    /// centimetres higher still. High-waisted, tucked in and nipped: three
    /// separate reasons for the same complaint. The shirt now hangs OVER the
    /// shorts and ends at the hip joint, with the roll of the hem standing
    /// clear of the seat inside it, which is where a footballer's shirt ends
    /// and is also the one horizontal cloth edge on the whole figure.
    ///
    /// The offsets are the profile, and they are an S: the chest stands 13 cm
    /// in front of the hip axis and the small of the back only 9.8 behind it,
    /// while the seat under the hem goes back 14. A chest is not centred on
    /// the spine and neither is a seat; lathed on the axis — which is what a
    /// symmetric `z` amounts to — the same widths describe a slab, and a slab
    /// in profile is the other half of what read as a toy.
    ///
    /// The shoulder CREST is deliberately narrower than the sleeve that
    /// crosses it. Two surfaces that meet tangentially draw a long crease and
    /// no depth buffer can help, and at 0.221 the crest was tangent to the
    /// sleeve within half a millimetre for the whole width of the shoulder —
    /// which is what put a rounded pad on each side of every player, with a
    /// valley between it and the neck. At 0.203 the sleeve is decisively
    /// outside from world 1.47 down, the two cross at fifty degrees, and what
    /// is left of the join is a seam over the top of the arm, which is where a
    /// shirt has one. The shoulder the eye measures is the sleeve's own
    /// 52 cm, not the crest.
    const SHIRT: [Ring; 15] = [
        Ring::squared(-0.014, 0.1700, 0.1240, -0.016, 2.45),
        Ring::squared(0.000, 0.1785, 0.1260, -0.014, 2.45),
        Ring::squared(0.028, 0.1715, 0.1240, -0.010, 2.50),
        Ring::squared(0.080, 0.1580, 0.1150, -0.002, 2.55),
        Ring::squared(0.150, 0.1500, 0.1055, 0.0075, 2.55),
        Ring::squared(0.235, 0.1565, 0.1095, 0.0095, 2.55),
        Ring::squared(0.315, 0.1690, 0.1175, 0.0095, 2.50),
        Ring::squared(0.382, 0.1820, 0.1230, 0.007, 2.40),
        Ring::squared(0.442, 0.1935, 0.1220, 0.000, 2.30),
        Ring::squared(0.484, 0.2000, 0.1140, -0.006, 2.15),
        Ring::squared(0.514, 0.2030, 0.1070, -0.007, 2.05),
        Ring::squared(0.534, 0.1995, 0.1010, -0.007, 2.00),
        Ring::squared(0.552, 0.1760, 0.0930, -0.007, 2.00),
        Ring::squared(0.568, 0.1230, 0.0820, -0.008, 2.00),
        Ring::squared(0.582, 0.0745, 0.0690, -0.011, 2.00),
    ];

    /// Neck, jaw and skull, hung off the base of the neck.
    /// Neck, jaw and skull, hung off the base of the neck.
    ///
    /// Rebuilt around the offsets, which a head needs more than any other part
    /// of a footballer. The neck sits BEHIND the head it carries, the chin and
    /// the cheekbones stand in front of the ear line, and the occiput hangs
    /// two centimetres out at the back — none of which a body of revolution
    /// can say, and all of which are what the eye uses to tell one profile
    /// from another. It is also narrower across than it is deep, which a real
    /// skull is by about a fifth and this one used not to be at all.
    ///
    /// The face texture is laid out against these numbers (see
    /// [`BodyParts::face_layout`]), so the two move together.
    ///
    /// **A PHOTOGRAPH is what checks these numbers**, and it checks them
    /// against the one ruler a picture of a man carries: the distance between
    /// his pupils, which is 63 mm on nearly everybody. Measured that way over
    /// the club head shots the site serves — see `portrait::measure_pictures`,
    /// which is what produced these — a head is 2.38 pupil widths across and
    /// carries 1.7 of them between the eye line and the top of the hair.
    ///
    /// The width was the first thing that ratio caught: this skull used to be
    /// 180 mm across where a head is 150, and a face laid onto it had two
    /// choices, both wrong — stretch to fill it, or sit in the middle of it
    /// like a mask on an egg.
    ///
    /// The CROWN is the second, and it is the one that made a footballer look
    /// like an egg with a face painted low on it. There used to be 125 mm of
    /// skull above the eye line, against the 108 a photograph of a man
    /// actually shows, and no picture of anybody reached the top of it: what
    /// filled the last two centimetres was flat paint, which reads as a bald
    /// dome standing over the man's own hairline. The rings above the eyes
    /// now follow a real cranium — widest just over the ear, then a spherical
    /// cap of about 80 mm radius over the top of it, which is what stops the
    /// crown coming to the POINT it used to: a lathe closing straight to its
    /// axis draws a cone, and there is a cone on nobody's head.
    ///
    /// **The NECK is part of this profile and it was a woman's**, 10.4 cm
    /// across where a man's is 12-13 and an athlete's more. A thin neck under
    /// a wide collar is also what left a ring of shadow round it — the shirt
    /// stood three and a half centimetres off the throat all the way round,
    /// which reads as a doll's head dropped into its socket rather than as a
    /// neck coming out of a shirt. Only the three rings below the jaw moved,
    /// and only across: their heights are what the face sheet's `v` is
    /// measured from ([`BodyParts::face_layout`]), so moving one slides the
    /// whole face down the head.
    const SKULL: [Ring; 14] = [
        Ring::set(-0.075, 0.0610, 0.0650, -0.004),
        Ring::set(-0.030, 0.0595, 0.0635, -0.006),
        Ring::set(0.005, 0.0590, 0.0650, -0.008),
        Ring::set(0.028, 0.0615, 0.0740, 0.000),
        Ring::set(0.048, 0.066, 0.082, 0.004),
        Ring::set(0.072, 0.073, 0.090, 0.004),
        Ring::set(0.100, 0.076, 0.096, 0.000),
        Ring::set(0.130, 0.076, 0.099, -0.002),
        Ring::set(0.150, 0.076, 0.0985, -0.004),
        Ring::set(0.174, 0.0745, 0.0955, -0.006),
        Ring::set(0.195, 0.0685, 0.0870, -0.008),
        Ring::set(0.216, 0.0545, 0.0670, -0.010),
        Ring::set(0.232, 0.0300, 0.0355, -0.0115),
        Ring::set(0.238, 0.0000, 0.0000, -0.012),
    ];

    /// Shin and sock in one: socks cover a footballer's leg to the knee.
    ///
    /// The calf sits high and at the BACK — which the offsets can now actually
    /// say — and that is what stops the lower leg reading as a broom handle
    /// when a stride opens out. The ankle steps forward again under it, the
    /// way a leg does.
    ///
    /// Written bottom-up so the turnover at the top of the sock can be lathed
    /// over it; the loft takes a profile from either end.
    const SHIN: [Ring; 8] = [
        Ring::set(-0.455, 0.033, 0.035, 0.003),
        Ring::set(-0.440, 0.034, 0.036, 0.002),
        Ring::set(-0.390, 0.036, 0.038, 0.000),
        Ring::set(-0.300, 0.042, 0.044, -0.002),
        Ring::set(-0.200, 0.050, 0.053, -0.004),
        Ring::set(-0.110, 0.059, 0.063, -0.006),
        Ring::set(-0.040, 0.062, 0.064, -0.004),
        Ring::set(0.020, 0.060, 0.060, 0.000),
    ];

    /// The seat of the shorts, bottom-up, in the pelvis's own space.
    ///
    /// A constant for the same reason [`Self::SHIRT`] is one: the shirt hangs
    /// over it, and the ONE thing that arrangement can get wrong — the seat
    /// coming out through the cloth when a player leans — is only checkable if
    /// both profiles can be read rather than looked at. See
    /// `the_shirt_hangs_clear_of_the_shorts`.
    const SEAT: [Ring; 5] = [
        Ring::squared(-0.152, 0.1300, 0.0960, -0.012, 2.25),
        Ring::squared(-0.118, 0.1640, 0.1130, -0.022, 2.35),
        Ring::squared(-0.075, 0.1745, 0.1190, -0.024, 2.40),
        Ring::squared(-0.030, 0.1600, 0.1120, -0.019, 2.45),
        Ring::squared(0.020, 0.1420, 0.1030, -0.012, 2.50),
    ];

    /// Heights of the features on that skull, in its own space. The mesh puts
    /// a nose at one of them and the texture draws the rest.
    const EYES: f32 = 0.130;
    const BROW: f32 = 0.158;
    /// The base of the nose, where the nostrils are — not the tip.
    const NOSTRILS: f32 = 0.096;
    const MOUTH: f32 = 0.070;
    const CHIN: f32 = 0.030;
    /// Where the hair caps below leave the forehead, so the texture can shade
    /// the line rather than letting a mesh edge sit on bare skin.
    ///
    /// Came down with the crown. A hairline sits about nine tenths of the
    /// distance between a man's pupils above them — which is where it sits in
    /// the head shots the skull was measured from — and the old one was a
    /// full pupil width up, which was only ever right on a skull with two
    /// centimetres of spare dome above the eyes.
    const HAIRLINE: f32 = 0.188;

    /// Where the print goes on the back of the shirt: the name across the
    /// shoulders and the number under it, both in the torso's own space.
    ///
    /// The arcs decide the panels' widths, since the ink follows the cloth
    /// round the body — 1.45 radians at the number's height is a chord of
    /// 23 cm and an arc of 23.3, which is a real shirt number. Whoever draws
    /// the textures has to match that aspect or the glyphs come out stretched;
    /// see [`crate::art::textures::Textures::number`].
    const NAME_AT: f32 = 0.464;
    const NAME_HEIGHT: f32 = 0.058;
    const NAME_ARC: f32 = 1.34;
    const NUMBER_AT: f32 = 0.316;
    const NUMBER_HEIGHT: f32 = 0.190;
    /// Came in from 1.45 when the back of the shirt stopped being an ellipse.
    /// The arc is a PARAMETER angle, and on a squared-off section (see
    /// [`Ring::edge`]) the same angle reaches further round: at the number's
    /// height 1.45 rad now spans 24.3 cm where it used to span 22.3, and the
    /// texture is drawn to one aspect. 1.28 puts the chord back at 22.4.
    const NUMBER_ARC: f32 = 1.28;
    /// How far the print floats off the cloth. Four millimetres: enough that
    /// no depth buffer this scene will ever run on can put the two in the
    /// wrong order, small enough that it is invisible at the panel's edges.
    const PRINT_LIFT: f32 = 0.004;

    pub fn new(meshes: &mut Assets<Mesh>) -> Self {
        BodyParts {
            torso: meshes.add(Sculptor::part(&Self::SHIRT)),
            // The neck of the shirt: the top of the torso swollen by five
            // millimetres, with a rim of its own standing a little above the
            // cloth. Every kit on earth has one, and without it the shirt
            // simply stops at a hole with a neck coming out of it.
            // A crew neck: twenty-three millimetres of it, hugging the neck
            // hole and nothing else.
            //
            // It starts at 0.5775 rather than a comfortable-looking way down,
            // because the torso is still 24 cm across at 0.568 and a band
            // taken from there is not a collar, it is a yoke across the
            // shoulders. The whole taper from the shoulder crest to the neck
            // happens inside seven centimetres, so the collar has to live in
            // the top two of them.
            //
            // Brought in with the neck it goes round. A collar has to CLOSE
            // on a throat — seven millimetres of daylight is a rib-knit neck,
            // and the two and a half centimetres this had were a socket with
            // a head standing in it.
            collar: meshes.add(Sculptor::part(&[
                Ring::set(0.5775, 0.0930, 0.0775, -0.0105),
                Ring::set(0.5845, 0.0810, 0.0705, -0.0115),
                Ring::set(0.5905, 0.0725, 0.0655, -0.0120),
                Ring::set(0.5955, 0.0675, 0.0620, -0.0122),
            ])),
            // The seat of the shorts, which stays put while the legs swing.
            //
            // Wide enough to CONTAIN the tops of the two legs of the shorts,
            // which it was not. The hips are 88 mm apart and a leg of cloth
            // round a thigh is nearly ninety more, so two tubes that reach
            // their full width up at the seat cut out through its sides — and
            // two nearly tangent surfaces crossing draw as a hard rectangular
            // notch, which is what sat on the front of every pair of shorts on
            // the pitch. Solved from the other end now that the seat is a
            // man's rather than a woman's: the legs are pulled IN at the top
            // and only reach their width below the crotch, so they emerge from
            // under the seat's own hem, which is where a leg of a pair of
            // shorts emerges.
            //
            // Narrower than it was by two centimetres a side, squarer in
            // section, and carried BACK: a man's seat projects behind him and
            // his front stays flat, where an ellipse on the axis gives the
            // same curve fore and aft and draws a hip. The waistband end of it
            // is pulled well in — nothing above the shirt's hem is ever seen,
            // and the narrower it is up there the more room the legs have to
            // stay inside it.
            //
            // **It also stops two centimetres above the hip joint**, where it
            // used to stand ten. The shirt leans and the seat does not — the
            // torso is a joint and [`Limb::Pelvis`] deliberately is not — so
            // every centimetre of shorts modelled ABOVE the pivot swings out
            // through the back of the shirt the moment a player leans into a
            // run. Measured at the lean the run cycle actually uses, the seat
            // came seven millimetres out through the cloth and drew a dark
            // crescent across the small of his back. Nothing up there is ever
            // seen — the hem covers it — so the fix is to not model it.
            pelvis: meshes.add(Sculptor::lathe(&Self::seat(), Sculptor::SIDES)),
            head: meshes.add(Sculptor::part_at(&Self::SKULL, Sculptor::HEAD_SIDES)),
            // Shaved, a crop, short back and sides, and a mop: the three caps
            // start lower at the temple, carry more volume and leave less
            // forehead in turn, which is the whole axis a squad varies along.
            //
            // All three come down PAST the ear, and they can afford to because
            // the burial is on the front edge alone: the cap covers the nape
            // and the temples all the way down while the face stays clear, so
            // the hair stops where hair stops rather than in a level line
            // ruled round the head at ear height.
            hair: [
                None,
                Some(meshes.add(Self::cap(0.100, 0.005, 0.0498))),
                Some(meshes.add(Self::cap(0.092, 0.009, 0.0586))),
                Some(meshes.add(Self::cap(0.084, 0.017, 0.0790))),
            ],
            // Deltoid, bicep, taper to the elbow.
            //
            // The top ring is now narrow (0.030) and sits high, so it ends up
            // inside the torso and its cap is never seen; the deltoid swells
            // 20 mm BELOW the socket, which is where a shoulder muscle
            // actually sits. Previously the widest ring was 5 mm below the
            // top, so the arm's broadest point was level with its own
            // socket and the join showed as a seam.
            //
            // It also does NOT pinch into the elbow any more. An arm that
            // narrows by a fifth at the joint and swells again below it is a
            // hinge with a bead on it, which is the single thing anybody
            // notices about a limb at close range; a real arm loses about a
            // tenth between the bicep and the elbow and the forearm's belly
            // picks straight up from there.
            upper_arm: meshes.add(Sculptor::part(&[
                Ring::oval(0.050, 0.0195, 0.0192),
                Ring::oval(0.026, 0.0400, 0.0388),
                Ring::oval(-0.006, 0.0568, 0.0545),
                Ring::oval(-0.060, 0.0568, 0.0532),
                Ring::oval(-0.130, 0.0522, 0.0488),
                Ring::oval(-0.205, 0.0472, 0.0448),
                Ring::oval(-0.268, 0.0448, 0.0432),
                Ring::oval(-0.300, 0.0396, 0.0384),
            ])),
            // A short sleeve, riding on the shoulder joint so it swings with
            // it. Dropped to sit over the deltoid rather than above it —
            // level with the socket it stood off the shoulder like a pad.
            //
            // Cut four millimetres looser than it was, all round. At its old
            // size the top of it ran within a millimetre of the shirt at the
            // shoulder and the two z-fought along the seam; a sleeve is a
            // separate piece of cloth OVER a body, and it has to be drawn
            // outside everything it covers rather than level with it.
            // It also has to CLOSE over the top of the shoulder rather than
            // stopping at its widest ring. Ending flat, its top cap was a
            // horizontal disc four centimetres wider than the torso beneath
            // it — a hard rectangular tab standing off each shoulder, which is
            // the single most robotic thing on the whole figure and the first
            // thing anybody looking at one saw.
            //
            // **The top of it is a SPHERE on the joint** — see
            // [`Self::SHOULDER_CAP`], which is the whole of why an arm stops
            // reading as a hinge.
            sleeve: meshes.add(Sculptor::part(&Self::sleeve(&[
                Ring::squared(-0.018, 0.0788, 0.0722, 0.001, 2.30),
                Ring::squared(-0.048, 0.0800, 0.0730, 0.002, 2.32),
                Ring::squared(-0.085, 0.0736, 0.0684, 0.003, 2.28),
                Ring::squared(-0.118, 0.0668, 0.0628, 0.004, 2.25),
                Ring::squared(-0.142, 0.0605, 0.0570, 0.004, 2.22),
            ]))),
            // And the band round the end of it. The same trim as the collar,
            // and the pair of them together are what say "kit" rather than
            // "coloured shape" at any distance a face is legible from. Rolled
            // under at the hem, the way a sewn edge is, so it does not end in
            // a flat washer hanging round the arm.
            cuff: meshes.add(Sculptor::part(&[
                Ring::squared(-0.108, 0.0712, 0.0668, 0.0034, 2.26),
                Ring::squared(-0.136, 0.0666, 0.0628, 0.0040, 2.24),
                Ring::squared(-0.152, 0.0630, 0.0594, 0.0040, 2.22),
                Ring::squared(-0.160, 0.0568, 0.0536, 0.0040, 2.20),
            ])),
            // The elbow is the NARROW point of an arm and the forearm's belly
            // sits below it. Started at its widest, as this did, the forearm
            // was wider than the arm it hangs off and the join showed as a
            // step right round the elbow — a hinge, on every player, at the
            // one place a limb is supposed to look continuous.
            // The forearm, with the ball of the elbow already in it.
            //
            // A ball at each of the two joints that bend, filling the gap the
            // two tapers leave between them when they do. Without it an arm
            // bent through a right angle opens a wedge at the outside of the
            // elbow, and a knee lifted into a stride opens the same wedge in
            // front of the kneecap.
            // Sized to sit just inside the two tapers when the limb is
            // straight — the thigh is 0.056 across two centimetres above the
            // knee and the sock 0.060 — so a standing player never shows one,
            // and a bent one shows nothing else.
            //
            // Both used to be entities of their own, hung off the limb they
            // fill at an identity transform and painted out of the same
            // material — which is four entities a player, eighty-eight over a
            // match, bought for nothing. See [`Sculptor::joined`].
            forearm: meshes.add(Sculptor::joined(
                // The top ring starts NARROW and well up inside the upper arm,
                // and the forearm's own surface comes out through it lower
                // down. Two lathes carry their own smooth normals, so wherever
                // they cross, the shading breaks — the trick is to cross where
                // both are running the same way, rather than at a rim where
                // one is 2 mm proud of the other and the break is a ring right
                // round the elbow.
                Sculptor::part(&[
                    // The offsets tilt the curve those two surfaces cross on
                    // so that it is not a ring ruled level round the arm: the
                    // shading break falls a few millimetres lower at the back
                    // than at the front, which is where the crease of an elbow
                    // actually runs.
                    Ring::set(0.046, 0.0250, 0.0245, -0.0055),
                    Ring::set(0.018, 0.0405, 0.0392, -0.0030),
                    Ring::set(-0.006, 0.0455, 0.0435, -0.0010),
                    Ring::oval(-0.052, 0.0470, 0.0445),
                    Ring::oval(-0.115, 0.0420, 0.0378),
                    Ring::oval(-0.180, 0.0350, 0.0310),
                    Ring::oval(-0.225, 0.0292, 0.0258),
                    Ring::oval(-0.252, 0.0268, 0.0240),
                ]),
                // Set back and down off the joint, because an olecranon is —
                // the point of an elbow is behind the hinge, not on it.
                // Sized to sit CLEAR inside both tapers when the arm is
                // straight, not flush with them: two surfaces a fraction of a
                // millimetre apart over a band do not draw as one surface,
                // they draw as a dotted line of z-fighting right round the
                // joint, which is worse than the bead it replaced.
                Sculptor::ellipsoid_at(
                    Vec3::new(0.0400, 0.0420, 0.0378),
                    Vec3::new(0.0, -0.006, -0.001),
                ),
            )),
            hand: [Self::hand(-1.0), Self::hand(1.0)].map(|mesh| meshes.add(mesh)),
            // Cuff, back of the hand, then the padded palm out to the
            // fingertips. Half again as long as a bare hand and nearly twice
            // as wide, which is what a keeper's glove is: at this range the
            // pair of them are the only part of him the eye actually tracks
            // through a save.
            // The cuff strap, the padded back of the hand, and then the
            // knuckle line the fingers hang off. Half again as wide as a bare
            // hand and squared off at the end, which is what a keeper's glove
            // is — a flat surface, deliberately, because the point of it is
            // to be in the way.
            glove: meshes.add(Sculptor::part(&[
                Ring::oval(0.062, 0.034, 0.030),
                Ring::oval(0.038, 0.045, 0.036),
                Ring::oval(0.014, 0.052, 0.038),
                Ring::oval(-0.020, 0.060, 0.038),
                Ring::oval(-0.058, 0.063, 0.036),
                Ring::oval(-0.082, 0.062, 0.033),
                Ring::oval(-0.092, 0.058, 0.029),
            ])),
            // One finger, in two segments: the proximal phalanx off the
            // knuckle, and the other two past it as one part. Both modelled
            // about their own root so the splay and the curl are rotations
            // rather than a second set of numbers — see [`Limb::Finger`].
            //
            // They OVERLAP by a centimetre on purpose. A knuckle is a bulge,
            // not a hinge with daylight through it, and two tapers that meet
            // exactly show the join the moment the finger bends.
            finger: meshes.add(Sculptor::part_at(
                &[
                    Ring::oval(0.008, 0.0155, 0.0135),
                    Ring::oval(-0.012, 0.0165, 0.0145),
                    Ring::oval(-0.034, 0.0155, 0.0135),
                    Ring::oval(-0.050, 0.0138, 0.0118),
                ],
                Sculptor::BLOB_SIDES,
            )),
            fingertip: meshes.add(Sculptor::part_at(
                &[
                    Ring::oval(0.010, 0.0145, 0.0125),
                    Ring::oval(-0.008, 0.0150, 0.0130),
                    Ring::oval(-0.026, 0.0132, 0.0112),
                    Ring::oval(-0.036, 0.0100, 0.0085),
                    Ring::oval(-0.042, 0.0055, 0.0050),
                ],
                Sculptor::BLOB_SIDES,
            )),
            // The thumb: shorter, thicker, and it comes off the side rather
            // than the end.
            thumb: meshes.add(Sculptor::part_at(
                &[
                    Ring::oval(0.010, 0.018, 0.016),
                    Ring::oval(-0.014, 0.019, 0.017),
                    Ring::oval(-0.040, 0.016, 0.014),
                    Ring::oval(-0.052, 0.011, 0.010),
                ],
                Sculptor::BLOB_SIDES,
            )),
            // The long sleeve, in two parts for the two halves of the arm.
            // Cut the same four millimetres looser than the limb inside it
            // that the short one is — two nearly tangent surfaces crossing
            // each other draw as a ragged sawtooth no depth buffer can fix.
            sleeve_long: meshes.add(Sculptor::part(&Self::sleeve(&[
                Ring::squared(-0.018, 0.0788, 0.0722, 0.001, 2.30),
                Ring::squared(-0.048, 0.0782, 0.0716, 0.002, 2.28),
                Ring::squared(-0.090, 0.0648, 0.0614, 0.002, 2.20),
                Ring::squared(-0.155, 0.0578, 0.0548, 0.002, 2.15),
                Ring::squared(-0.228, 0.0526, 0.0504, 0.002, 2.10),
                Ring::squared(-0.292, 0.0484, 0.0468, 0.002, 2.10),
            ]))),
            sleeve_forearm: meshes.add(Sculptor::part(&[
                Ring::squared(0.050, 0.0448, 0.0432, 0.000, 2.10),
                Ring::squared(0.008, 0.0512, 0.0482, 0.000, 2.15),
                Ring::squared(-0.055, 0.0525, 0.0488, 0.001, 2.15),
                Ring::squared(-0.120, 0.0472, 0.0430, 0.002, 2.10),
                Ring::squared(-0.185, 0.0402, 0.0362, 0.002, 2.10),
                Ring::squared(-0.235, 0.0342, 0.0308, 0.002, 2.05),
                Ring::squared(-0.268, 0.0300, 0.0272, 0.002, 2.00),
            ])),
            cuff_forearm: meshes.add(Sculptor::part(&[
                Ring::squared(-0.212, 0.0392, 0.0370, 0.002, 2.10),
                Ring::squared(-0.242, 0.0356, 0.0336, 0.002, 2.05),
                Ring::squared(-0.260, 0.0322, 0.0304, 0.002, 2.00),
                Ring::squared(-0.270, 0.0262, 0.0248, 0.002, 2.00),
            ])),
            // The leg of the shorts: it belongs to the thigh, not to the hips.
            //
            // Rolled under at the hem for the same reason the cuff is: a leg
            // of cloth that simply stops is a tube with a hole in the end of
            // it, and from below that hole is what you see.
            //
            // **And it was an A-LINE, which is a skirt.** Waist in, hem out,
            // one unbroken bell from the belt to the knee — the two legs met
            // in the middle and there was no seeing where either began. Shorts
            // are the opposite shape: the fullest point is the HIP, and from
            // there down the cloth hangs in a straight column that is a little
            // narrower at the hem than it is at the top of the thigh. Squared
            // in section, because cloth over a leg is a rounded box rather
            // than a tube.
            //
            // It also starts LOWER than it did — a hand's width below the
            // waist rather than up at it — so that everything above the seat's
            // own widest ring is the seat, and the two tubes only appear where
            // a leg of a pair of shorts actually appears.
            shorts_leg: meshes.add(Sculptor::part(&[
                Ring::squared(-0.062, 0.0790, 0.0770, 0.000, 2.30),
                Ring::squared(-0.100, 0.0862, 0.0838, 0.002, 2.38),
                Ring::squared(-0.150, 0.0890, 0.0862, 0.004, 2.40),
                Ring::squared(-0.180, 0.0876, 0.0845, 0.005, 2.40),
                Ring::squared(-0.202, 0.0838, 0.0800, 0.005, 2.40),
                Ring::squared(-0.212, 0.0762, 0.0726, 0.005, 2.40),
            ])),
            // Quadriceps high on the thigh, narrowing into the knee — and the
            // whole muscle carried a few millimetres forward of the bone,
            // which is where it is.
            thigh: meshes.add(Sculptor::part(&[
                Ring::set(-0.085, 0.0805, 0.0800, 0.000),
                Ring::set(-0.150, 0.0812, 0.0805, 0.002),
                Ring::set(-0.245, 0.0770, 0.0748, 0.003),
                Ring::set(-0.340, 0.0685, 0.0672, 0.002),
                Ring::set(-0.420, 0.0590, 0.0585, 0.000),
                Ring::set(-0.455, 0.0530, 0.0530, 0.000),
            ])),
            // The shin, with the ball of the knee in it — see `forearm` above.
            shin: meshes.add(Sculptor::joined(
                Sculptor::part(&Self::SHIN),
                Sculptor::ellipsoid(Vec3::splat(0.048)),
            )),
            // The turnover at the top of the sock, in the shorts colour — the
            // one piece of kit detail that survives at this distance.
            //
            // Taken off the sock underneath it rather than written out again.
            // Hand-written, the two profiles agreed to within a tenth of a
            // millimetre at the bottom of the turnover and the pair of them
            // z-fought along a band right across the shin: at any range the
            // sock tops came out ragged and speckled, and the closer the
            // camera got the worse it looked.
            sock_top: meshes.add(Sculptor::lathe(
                &{
                    let mut band = Sculptor::band(&Self::shin(), -0.078, 0.012, 4, 0.0038);
                    // Rolled under at the bottom edge, where a turnover is turned
                    // over.
                    band.insert(0, Sculptor::section(&Self::shin(), -0.090).swollen(0.0012));
                    band
                },
                Sculptor::SIDES,
            )),
            // A boot, rather than the stretched egg it was.
            //
            // A lathe about the leg's own axis cannot make a foot symmetric
            // about anything but that axis — but it does not have to, because
            // the offsets carry each section forward: the sole is long and
            // well in front of the ankle, the widest part is across the ball
            // of the foot, and the whole thing draws back and narrows into the
            // heel as it rises.
            boot: meshes.add(Sculptor::part(&[
                Ring::set(-0.040, 0.026, 0.062, 0.024),
                Ring::set(-0.032, 0.040, 0.086, 0.022),
                Ring::set(-0.018, 0.047, 0.097, 0.016),
                Ring::set(0.000, 0.048, 0.096, 0.006),
                Ring::set(0.018, 0.044, 0.080, -0.008),
                Ring::set(0.034, 0.036, 0.058, -0.018),
                Ring::set(0.046, 0.026, 0.038, -0.022),
            ])),
            // A real shirt number covers most of the upper back, and a real
            // name runs across the shoulders above it. Both lie ON the shirt:
            // see [`Sculptor::decal`] for why a flat rectangle cannot.
            number: meshes.add(Sculptor::decal(
                &Self::shirt(),
                Self::NUMBER_AT,
                Self::NUMBER_HEIGHT,
                -FRAC_PI_2,
                Self::NUMBER_ARC,
                Self::PRINT_LIFT,
            )),
            name: meshes.add(Sculptor::decal(
                &Self::shirt(),
                Self::NAME_AT,
                Self::NAME_HEIGHT,
                -FRAC_PI_2,
                Self::NAME_ARC,
                Self::PRINT_LIFT,
            )),
        }
    }

    /// A cap of hair over the skull, coming down to `from` at the sides,
    /// standing `swell` proud of the head, and buried `recede` inside the face
    /// where it starts.
    ///
    /// The burial FADES with height and then reverses, and that is the whole
    /// trick: down at the temple the cap's front edge is inside the head and
    /// by the crown it is out over it, so the curve where the two surfaces
    /// cross IS the hairline — a real one, higher up the middle of the
    /// forehead than at the temples, because the skull is rounder there. A
    /// constant burial either sinks the hair to the crown or sits it down over
    /// the eyebrows, and there is no value in between that works.
    fn cap(from: f32, swell: f32, recede: f32) -> Mesh {
        Sculptor::lathe(&Self::cap_rings(from, swell, recede), Sculptor::HEAD_SIDES)
    }

    /// The profile of that cap, apart from the lathe so the hairline it
    /// produces can be measured rather than looked at.
    fn cap_rings(from: f32, swell: f32, recede: f32) -> Vec<Ring> {
        const STEPS: usize = 20;
        /// Where the cap stops following the skull and pinches to its own
        /// crown, as a DEPTH below the crown.
        ///
        /// A depth rather than a height, which it used to be. Written out as a
        /// height it was a number that happened to sit under a skull 255 mm
        /// tall, and the moment the crown came down to where a photographed
        /// head actually ends it was above it — so the cap climbed past the
        /// top of the head and then dived back to close, and every player
        /// with hair took the field with a chimney on him.
        const SHOULDER: f32 = 0.009;
        /// How many spans the dome over the top of it closes in.
        const CLOSE: usize = 4;

        let crown = Self::SKULL[Self::SKULL.len() - 1];
        // Sampled finely, and it has to be. Both profiles are lofted through
        // curves rather than in straight lines, but a cap that took only a
        // handful of samples of the head it covers would still cut inside it
        // between them — and where it does, a ring of bare scalp comes through
        // the hair. Every player took the field wearing a tonsure once
        // already; see `the_hair_leaves_a_hairline`.
        let skull = Self::skull();
        let span = (crown.y - SHOULDER) - from;
        let mut rings: Vec<Ring> = (0..=STEPS)
            .map(|step| {
                let y = from + span * step as f32 / STEPS as f32;
                let along = ((y - from) / span).clamp(0.0, 1.0);
                let fade = (1.0 - along) * (1.0 - along);
                // Buried by `recede` at the bottom of the cap, standing
                // `swell` proud of the face by the crown, and the height where
                // that crosses zero is where the hair appears.
                let clearance = recede * fade - swell * (1.0 - fade);
                Sculptor::section(&skull, y).capped(swell, clearance)
            })
            .collect();
        // A spherical closure over the top of the skull.
        //
        // Run straight from the shoulder to a point — which is what this used
        // to do — and a centimetre of profile carries the whole turn from the
        // side of a head to the top of it: a cone so shallow it is a lid, and
        // a straight-sided lid cannot follow a dome, so a ring of bare scalp
        // comes through between the two. Every player with hair took the
        // field wearing a tonsure once already. Two rings placed by hand just
        // under the crown covered the scalp and left a tip standing on it,
        // and one more between those left a crease ruled across it. What
        // closes a dome is a DOME: the sphere through the shoulder ring that
        // meets the axis at the apex, sampled.
        let apex = crown.y + swell * 0.9;
        if let Some(last) = rings.last().copied() {
            let rise = (apex - last.y).max(1e-4);
            let radius = (last.x * last.x + rise * rise) / (2.0 * rise);
            for step in 1..CLOSE {
                let deep = rise * (1.0 - step as f32 / CLOSE as f32);
                let across = (2.0 * radius * deep - deep * deep).max(0.0).sqrt() / last.x.max(1e-4);
                let along = step as f32 / CLOSE as f32;
                rings.push(Ring::set(
                    apex - deep,
                    last.x * across,
                    last.z * across,
                    last.offset + (crown.offset - last.offset) * along,
                ));
            }
        }
        rings.push(Ring::set(apex, 0.0, 0.0, crown.offset));

        // And one BELOW the cap, tucked inside the head all the way round.
        // A cap that simply stops leaves a rim of cloth-thick hair standing
        // off the skin at the nape, which from the side is a hard horizontal
        // line ruled round the back of the head. Diving the last ring inside
        // the skull moves the visible edge to where the two surfaces cross,
        // which is a curve — the same trick the hairline itself is.
        rings.insert(
            0,
            Sculptor::section(&skull, from - 0.024).capped(-0.005, recede + 0.006),
        );
        rings
    }

    /// The two profiles other things are derived from, as the mesh actually
    /// has them — the smooth curve through the control points, not the
    /// polyline between them (see [`Sculptor::curved`]).
    ///
    /// Everything that reads a section off a part has to come through these,
    /// or it is measuring a shape the model does not have: the collar, the
    /// printed panels, the hair and its hairline are all placed to within a
    /// few millimetres of the surface they sit on, and the gap between a
    /// chord and its arc is that same order.
    fn skull() -> Vec<Ring> {
        Sculptor::curved(&Self::SKULL)
    }

    fn shirt() -> Vec<Ring> {
        Sculptor::curved(&Self::SHIRT)
    }

    fn seat() -> Vec<Ring> {
        Sculptor::curved(&Self::SEAT)
    }

    /// **The shoulder is a BALL on the joint**, and the sleeve is that ball
    /// with an arm hanging out of the bottom of it.
    ///
    /// The one thing a lathe about the arm's own axis cannot draw is a
    /// shoulder. A tube — which is what every sleeve here was — is wide at the
    /// joint and stays wide going up, so it has to be cut off somewhere, and
    /// wherever it is cut off it drives into the side of the shirt as a wall.
    /// Reported 2026-08-28 off the live renderer: *"the arms are attached to
    /// the body like hinges"*. That is exactly what it was — a rounded pod
    /// bolted onto each side of the chest, with a hard dark groove between the
    /// pod and the cloth running from the collar down to the armpit, because
    /// at the join the sleeve's surface faced INWARD and the shirt's faced out
    /// and the two met at something near a hundred and fifty degrees.
    ///
    /// A sphere centred on the joint fixes it for two separate reasons:
    ///
    /// * It is the only shape that is wide at the joint and narrows BOTH ways,
    ///   so it can be buried in the shirt at the top and take over from it
    ///   further out. Near its pole the surface is nearly horizontal, which is
    ///   what the top of a shoulder is — so it comes out of the cloth at about
    ///   twenty-five degrees instead of a hundred and fifty, and what is left
    ///   of the join is a soft crease over the top of the arm running down
    ///   into the armpit. Which is where a shirt has a sleeve seam.
    /// * A sphere about the pivot is **invariant under the joint it hangs
    ///   from**. The arm swings, the ball does not move, and the shoulder
    ///   stays filled at every angle — including the ones that used to open
    ///   the armpit, a man with both arms over his head.
    ///
    /// The HEIGHT of it is not free. The pole sits that far above the joint on
    /// the arm's own axis, which is [`Physique::SHOULDER_SPREAD`] out from the
    /// middle, and it has to stay INSIDE the shirt or it draws a pimple on the
    /// top of the shoulder: the shirt passes 0.176 wide at world 1.498, so
    /// anything over 0.078 breaks out. 0.074 leaves a centimetre of burial.
    ///
    /// Across, it can afford six millimetres more, and takes them — 0.256 out
    /// from the middle, which is 51 cm over the pair and the figure a man's
    /// shoulders are actually measured by. That the cap is a near-sphere
    /// rather than a sphere costs nothing, because the axis it is stretched on
    /// is the one the arm's SWING leaves alone: a rotation about x mixes the
    /// other two. Only the spread turns it, and the spread is small.
    const SHOULDER_CAP: Vec3 = Vec3::new(0.0800, 0.0740, 0.0760);

    /// **A hand with fingers on it**, which is the only version of a hand that
    /// survives a camera at arm's length.
    ///
    /// It was an ellipsoid, then a tapered paddle with a thumb-shaped lump on
    /// the front of it, and neither is a hand: the thing the eye counts is
    /// FOUR gaps, and no amount of shaping one lump produces them. Four
    /// fingers and a thumb, each its own lathe about its own axis, turned into
    /// place and merged into the palm's buffer — so the whole hand is still
    /// ONE entity and one draw call per arm, which is the only reason twenty
    /// outfield players can afford it (see [`Sculptor::joined`] on what the
    /// frame is actually spent on, and [`Limb::Finger`] for the keeper, whose
    /// digits have to articulate and therefore cannot be merged).
    ///
    /// **It is chiral, and that is why there are two of them.** A relaxed hand
    /// hangs with the palm turned in to the thigh and the fingers falling
    /// toward it — inward, which is `−x` on the right arm and `+x` on the
    /// left. The wrist's frame is not mirrored between the sides (nothing in
    /// [`Joint::pose`] rolls the arm), so one mesh cannot serve both: built
    /// once for each `side`, the curl and the thumb come out as mirror images
    /// the way hands do. A negative scale would have done it in one mesh and
    /// would also have turned every triangle inside out.
    fn hand(side: f32) -> Mesh {
        /// One finger, root at the knuckle and pointing down its own −y.
        const FINGER: [Ring; 7] = [
            Ring::oval(0.012, 0.0096, 0.0100),
            Ring::oval(-0.014, 0.0100, 0.0104),
            Ring::oval(-0.040, 0.0092, 0.0095),
            Ring::oval(-0.062, 0.0084, 0.0087),
            Ring::oval(-0.078, 0.0074, 0.0076),
            Ring::oval(-0.087, 0.0058, 0.0060),
            Ring::oval(-0.092, 0.0028, 0.0029),
        ];
        /// Index, middle, ring, little: where along the knuckle line it sits,
        /// how far down the knuckle is, how long the digit is against the
        /// middle one, and how far it is curled.
        ///
        /// The knuckle line runs front to back here, not across, because the
        /// palm faces the thigh — the index is the digit nearest the thumb and
        /// the thumb is forward. The four are not the same length and they do
        /// not curl the same amount; a hand where they are is a comb.
        const DIGITS: [(f32, f32, f32, f32); 4] = [
            (0.0280, -0.062, 0.86, 0.34),
            (0.0094, -0.068, 1.00, 0.37),
            (-0.0094, -0.066, 0.94, 0.41),
            (-0.0275, -0.058, 0.76, 0.47),
        ];
        /// And the thumb, which comes off the SIDE of the palm rather than the
        /// end of it, is shorter and thicker than a finger, and opposes the
        /// other four. It is what says hand rather than mitten.
        const THUMB: [Ring; 5] = [
            Ring::oval(0.014, 0.0126, 0.0130),
            Ring::oval(-0.016, 0.0130, 0.0134),
            Ring::oval(-0.042, 0.0116, 0.0119),
            Ring::oval(-0.060, 0.0098, 0.0100),
            Ring::oval(-0.070, 0.0060, 0.0062),
        ];

        // The palm: from up inside the forearm to the heads of the
        // metacarpals, where the fingers take over.
        //
        // The top two rings cross the forearm's own last one from inside to
        // outside, so the hand covers the rim the forearm ends on instead of
        // stepping in from it. Written the other way round — which it was —
        // the wrist draws as a bright ring and the hand as a separate object
        // hung under it.
        let mut hand = Sculptor::part(&[
            Ring::squared(0.056, 0.0210, 0.0205, 0.000, 2.05),
            Ring::squared(0.038, 0.0300, 0.0298, 0.001, 2.15),
            Ring::squared(0.012, 0.0252, 0.0332, 0.002, 2.35),
            Ring::squared(-0.024, 0.0206, 0.0392, 0.004, 2.50),
            Ring::squared(-0.052, 0.0192, 0.0416, 0.006, 2.55),
            Ring::squared(-0.072, 0.0172, 0.0400, 0.009, 2.50),
            Ring::squared(-0.086, 0.0138, 0.0344, 0.011, 2.40),
        ]);
        for (along, drop, length, curl) in DIGITS {
            let digit: Vec<Ring> = FINGER
                .iter()
                .map(|ring| Ring {
                    y: ring.y * length,
                    ..*ring
                })
                .collect();
            hand = Sculptor::placed(
                hand,
                Sculptor::part_at(&digit, Sculptor::BLOB_SIDES),
                Transform::from_translation(Vec3::new(0.0, drop, along))
                    .with_rotation(Quat::from_rotation_z(-side * curl)),
            );
        }
        Sculptor::placed(
            hand,
            Sculptor::part_at(&THUMB, Sculptor::BLOB_SIDES),
            Transform::from_translation(Vec3::new(-side * 0.004, -0.020, 0.0250))
                .with_rotation(Quat::from_rotation_z(-side * 0.52) * Quat::from_rotation_x(-0.62)),
        )
    }

    /// A sleeve: the cap above, and whatever hangs below it.
    ///
    /// The cap's rings are the sphere's own profile rather than a hand-written
    /// approximation of it, because the two properties that make it work —
    /// the pole landing inside the shirt, and the surface being invariant as
    /// the arm swings — are properties of a SPHERE and of nothing that merely
    /// looks like one.
    fn sleeve(arm: &[Ring]) -> Vec<Ring> {
        let cap = Self::SHOULDER_CAP;
        let mut rings: Vec<Ring> = [1.000f32, 0.892, 0.703, 0.432, 0.108]
            .into_iter()
            .map(|height| {
                let round = (1.0 - height * height).max(0.0).sqrt();
                Ring::oval(cap.y * height, cap.x * round, cap.z * round)
            })
            .collect();
        rings.extend_from_slice(arm);
        rings
    }

    fn shin() -> Vec<Ring> {
        Sculptor::curved(&Self::SHIN)
    }

    /// Where the features of a face land on this skull, for whoever is drawing
    /// the texture that wraps it.
    ///
    /// The only crossing between the mesh and the paint, and deliberately the
    /// only one: an eye drawn at a height the skull does not have there ends
    /// up on a cheekbone, and nothing downstream can tell.
    /// Where the five digits of a keeper's glove sit, in the wrist's own
    /// space: four across the knuckle line and a thumb off the inside edge.
    ///
    /// One place, because [`Footballer::assemble`] and the software preview
    /// both walk this hierarchy — a part added to one and not the other
    /// simply does not appear in the pictures the tests draw.
    ///
    /// **Only the rest POSITION lives here now.** The splay that used to be
    /// baked into these transforms has moved into [`Joint::pose`], because a
    /// hand that holds one splay forever is the paddle the fingers were
    /// added to stop being — see [`Limb::Finger`]. `pose` reads the knuckle's
    /// own `x` back out of this origin rather than being handed the number a
    /// second time.
    ///
    /// **The four are not interchangeable.** They were: one mesh, one length,
    /// four even steps across the knuckles, which is a comb rather than a
    /// hand. Ordered from the thumb outward, so the index is the one next to
    /// it on either hand, and scaled UNIFORMLY — a little finger is thinner
    /// as well as shorter, and a scale that only shortens shears the segment
    /// hanging off it the moment the finger curls.
    pub fn digits(side: f32) -> [(Limb, Transform); 5] {
        // Across the knuckles, out from the thumb; how far down the knuckle
        // sits, since the line of them is an arc and not a rule; and how big
        // the finger is against the middle one.
        const FINGERS: [(f32, f32, f32); 4] = [
            (0.042, -0.002, 0.98),
            (0.014, 0.000, 1.05),
            (-0.014, 0.004, 0.97),
            (-0.042, 0.012, 0.80),
        ];
        let knuckle = |index: usize| {
            let (across, along, size) = FINGERS[index];
            (
                Limb::Finger(index as u8),
                Transform::from_translation(Vec3::new(
                    -side * across,
                    Self::KNUCKLES + along,
                    0.004,
                ))
                .with_scale(Vec3::splat(size)),
            )
        };
        [
            knuckle(0),
            knuckle(1),
            knuckle(2),
            knuckle(3),
            (
                Limb::Thumb,
                Transform::from_translation(Vec3::new(-side * 0.046, -0.026, 0.016)),
            ),
        ]
    }

    /// The second segment of a finger, in the first one's own space: the two
    /// phalanges past the knuckle, as one part.
    ///
    /// See [`Limb::Finger`] for why there are two of them at all. The offset
    /// is where the proximal segment ends, so the joint is a knuckle and not
    /// a gap.
    pub const KNUCKLE_JOINT: Vec3 = Vec3::new(0.0, -0.044, 0.0);

    /// The knuckle line, in the wrist's own space — where the glove ends and
    /// the fingers start — and how far a finger splays per metre it sits off
    /// the middle of the hand.
    const KNUCKLES: f32 = -0.086;
    pub const SPLAY: f32 = 3.4;

    pub fn face_layout() -> FaceLayout {
        let foot = Self::SKULL[0].y;
        let crown = Self::SKULL[Self::SKULL.len() - 1].y;
        FaceLayout {
            foot,
            span: crown - foot,
            cheek: Sculptor::section(&Self::skull(), Self::EYES).x,
            eyes: Self::EYES,
            brow: Self::BROW,
            nostrils: Self::NOSTRILS,
            mouth: Self::MOUTH,
            chin: Self::CHIN,
            hairline: Self::HAIRLINE,
        }
    }
}

/// Which limb a joint drives. The rig is small enough that the run cycle can be
/// written out per joint rather than sampled from an animation clip.
#[derive(Clone, Copy)]
pub enum Limb {
    /// The seat of the shorts. It never turns — it is only here so that it
    /// rides the bob with the rest of the body instead of staying behind.
    Pelvis,
    Torso,
    Head,
    Shoulder,
    Elbow,
    /// The hand. Only ever visibly articulated on a goalkeeper — but a hand
    /// welded in line with its forearm is exactly what makes a save read as a
    /// mannequin being swung about, so the joint exists for everybody and the
    /// run cycle simply asks very little of it.
    Wrist,
    /// One finger of a keeper's glove, `0` the index and `3` the little,
    /// and the second segment of the same finger.
    ///
    /// **A hand is the one part of a goalkeeper an eye tracks**, and until
    /// now the five digits were welded to the mitt at a fixed splay: the
    /// same open paddle whether he was spreading his hands at a shot,
    /// closing them round a ball he had caught, punching a cross away or
    /// walking back to his line with nothing to do. Which is to say the rig
    /// had a hand-shaped object and no hand — reported as *"he just sticks
    /// them out"*.
    ///
    /// Two segments rather than one because the difference between an OPEN
    /// hand and a CLOSED one is most of the point, and a single hinge cannot
    /// draw a fist: rotated far enough to close, one rigid finger is a blade
    /// lying flat across the palm. Two put the tip back where a knuckle
    /// would.
    Finger(u8),
    Knuckle(u8),
    /// …and the thumb, which is the digit that says *hand* rather than
    /// *mitten*: it comes off the side, it opposes the other four, and it is
    /// the one that closes last.
    Thumb,
    Hip,
    Knee,
    /// The foot.
    ///
    /// A boot welded in line with its shin is to a run what a hand welded
    /// in line with its forearm is to a save — see [`Limb::Wrist`], which
    /// exists for exactly the same reason. It is also the joint a viewer
    /// reads FIRST, because it is the one touching the ground: a leg that
    /// swings through and plants a flat, rigid foot is the difference
    /// between a footballer and a marionette, and "very stiff in their
    /// movements" is what it looks like from the stand.
    Ankle,
}

/// One articulated joint of one player.
#[derive(Component)]
pub struct Joint {
    /// The actor this limb belongs to; the run cycle is kept there.
    pub owner: Entity,
    limb: Limb,
    /// −1 on the left of the body, +1 on the right, 0 down the middle.
    side: f32,
    /// Where the joint sits at rest, so the bob can be added back onto it.
    origin: Vec3,
}

/// How a player is moving right now, in the only two terms the pose needs:
/// where in the stride they are, and how hard they are running.
#[derive(Clone, Copy)]
pub struct Gait {
    pub phase: f32,
    /// 0 standing, 1 flat out.
    pub run: f32,
    /// −1..1, fixed for the life of a player: how he carries himself.
    ///
    /// Twenty-two figures running one identical animation is most of what
    /// reads as "robots" — more than any amount of geometry. Real players
    /// are individually recognisable at distance almost entirely by
    /// carriage: how wide the arms are held, how bent the elbows, how far
    /// forward the shoulders go. One number per player, applied to those
    /// three, is enough to break the lockstep.
    pub signature: f32,
    /// A slow cycle in radians that runs on the clock rather than on ground
    /// covered, so it keeps going when a player is not.
    ///
    /// Standing still was literally standing still: every amplitude in this
    /// rig is scaled by `run`, so at rest a footballer froze into a statue.
    /// The path trace says that is most of the pitch most of the time —
    /// forwards are stationary 68% of a match and keepers 77% — so a frozen
    /// idle is not an edge case, it is the default state of the scene.
    pub idle: f32,
    /// How hard he is turning, −1..1, for the lean into it.
    pub turn: f32,
    /// Where he is looking, as a yaw off his own facing in radians. A player
    /// watches the ball; his head does not sit welded to his chest.
    pub look: f32,
    /// And how far up or down, in radians: positive when the ball is above
    /// his eyeline. Without it a keeper tracks a cross along the floor and
    /// then catches it above his head without ever having looked at it.
    pub look_pitch: f32,
    /// 0 for everybody all match; ramps to 1 for the one player who has the
    /// ball in his hands — a keeper who has gathered it, or the man walking
    /// it back to the centre circle after a goal.
    ///
    /// A keeper who had gathered the ball was posed exactly like one who had
    /// not: arms hanging at his sides, swinging with the run cycle, while the
    /// ball hung at chest height inside his own torso. This is the signal that
    /// brings the forearms up and settles the chest so it can sit in his
    /// hands instead.
    ///
    /// Not a throw-in, which is also a ball in both hands: there the arms
    /// belong to the throw. See [`Self::throw_in`].
    pub carry: f32,
    /// 0..1: he is off his feet and committed. Only ever non-zero for a
    /// goalkeeper — see [`crate::players::actors::Actors::animate`] for how a
    /// dive is told from a run.
    ///
    /// The topple itself is not here: a body going horizontal is a rotation
    /// of the WHOLE figure and belongs on [`Carriage`]. What this drives is
    /// everything the limbs do differently once he has left his feet — the
    /// run cycle cut dead rather than faded out.
    ///
    /// On the instant, and held long after he lands: a keeper does not stand
    /// straight back up.
    pub dive: f32,
    /// 0..1: how far through the extension he is, ramped across the flight.
    ///
    /// The single most important number in a save, and the one this rig used
    /// not to have. `dive` says he has left the ground, and it is true within
    /// a frame of take-off; a man in the air is then in exactly one pose for
    /// the next four hundred milliseconds, which is most of what made a save
    /// look like a photograph being slid across the grass. Measured off a
    /// recorded match, a keeper is airborne for 390–660 ms with a median of
    /// 450 — long enough that every part of the extension has to be drawn:
    /// he leaves the ground gathered, opens out, and is at full stretch by
    /// the time he arrives.
    ///
    /// Ratchets: it climbs over the flight and is given back only by the
    /// recovery, because nobody folds himself up again mid-air.
    pub stretch: f32,
    /// 0..1: how long he has been down since the landing.
    ///
    /// The pose he lands in is not the pose he lies in. This is what takes
    /// him from full stretch to a man curled on his side with the ball
    /// pulled in — and it is scaled by `dive`, so it goes away as he gets up
    /// rather than pinning him to the turf.
    pub grounded: f32,
    /// −1..1: which way he went, as a share of his travel across his own
    /// body. ±1 is a dive flat across the goal, 0 one straight down the
    /// pitch at a striker's feet.
    ///
    /// Nothing in this rig used to know, so both arms and both legs did the
    /// same thing as each other — the superman, and the one pose no
    /// goalkeeper has ever adopted. A save has a lead side: the top arm goes
    /// through the ball, the bottom one trails, the underneath leg stays
    /// straight and the top one folds.
    pub lead: f32,
    /// 0..1: both arms thrown out to their full length, for the dive and for
    /// the standing leap at a cross. Held apart from `dive` because a keeper
    /// jumping at a corner reaches without toppling at all.
    pub reach: f32,
    /// 0..1: on his toes with a shot on — knees bent, weight forward, gloves
    /// up and out.
    ///
    /// A keeper spends the overwhelming majority of a match on his feet
    /// (measured: `Standing` alone is 8770 s of one recording against 48 s
    /// of `Diving`), and until now he spent all of it standing exactly like
    /// a centre-forward waiting for a throw-in. The set position is the
    /// posture the save comes out of, so a dive with nothing in front of it
    /// arrives from nowhere.
    pub set: f32,
    /// 0..1: he has the ball, at full stretch, in the air.
    ///
    /// The difference between a save and a catch, and they do not look alike:
    /// a keeper covering his goal has his gloves half a metre apart because
    /// he is trying to be in two places, and one who has actually got the
    /// thing brings both hands onto it. Without this, a ball claimed at full
    /// stretch hangs between two gloves that never close.
    pub claimed: f32,
    /// Where he is in a kick: −1 at the top of the backswing, 0 at the
    /// instant the boot meets the ball, +1 at the end of the follow through.
    /// 0 for everybody not kicking, which is why it must be read together
    /// with `power`.
    ///
    /// The ball is struck 14.8 times a minute in a recorded match and none of
    /// it used to be drawn: the ball left, the player kept running, and the
    /// single most repeated action in football was a thing that happened to
    /// the ball rather than a thing anybody did.
    pub swing: f32,
    /// How hard, 0..1 — a five-yard pass and a shot from twenty are the same
    /// movement at very different amplitudes. Doubles as the gate on the
    /// whole kick: at zero none of it applies.
    pub power: f32,
    /// Which boot: −1 left, +1 right, 0 nobody.
    pub foot: f32,
    /// How big a run cycle this player has, about 1.0 — hips, knees, arms
    /// and the bob, together. See `Complexion::spring`.
    ///
    /// Held apart from [`Self::signature`] on purpose: that one carries how
    /// he holds his ARMS, and a squad whose stride amplitude is cut from
    /// the same bits as its arm carriage has half as many kinds of runner
    /// in it as it looks like it has.
    pub spring: f32,
    /// **How bent he runs at the elbow**, −1 nearly straight to +1 folded
    /// tight.
    ///
    /// The most individual thing about a runner seen from the touchline, and
    /// until now it was `signature` — the same number that set how WIDE he
    /// holds his arms, how far forward he leans and which of his two arms
    /// does more. Nine separate things came off that one hash, so the whole
    /// squad varied along ONE axis: a man with his arms held wide always
    /// also ran bent-elbowed and leaning. It is the argument [`Self::spring`]
    /// already makes about [`Self::signature`], made twice more — one hash
    /// gives two kinds of runner and four independent hashes give sixteen.
    pub elbows: f32,
    /// **…and how far forward he carries himself**, −1 upright to +1 over
    /// his own feet. Same argument.
    pub lean: f32,
    /// **How far his feet point OUT**, in radians, and 0 for a man who runs
    /// dead straight.
    ///
    /// [`Joint::TOE_OUT`] is a constant 0.20 for the whole squad and — the
    /// part that matters — it is scaled by `course.x`, so it is only ever
    /// non-zero in a SIDE-STEP. Measured, an outfielder is travelling
    /// forwards on 93% of the frames he is moving in, so essentially every
    /// boot on the pitch pointed dead along its own run, all match. The
    /// ankle exists precisely because a welded foot is the first thing an
    /// eye picks out (see [`Limb::Ankle`]), and how far a man toes out is
    /// one of the most visible things about him.
    pub toes: f32,
    /// 0..1 over a keeper throwing the ball out, which is the same swing
    /// routed to his shoulder instead of his hip. Peaks at the release.
    pub throwing: f32,
    /// −1..1: driving off the mark at +1, pulling up short at −1.
    ///
    /// A footballer changes pace far more often than he changes direction,
    /// and the rig had a lean for the turn but nothing at all for this — so
    /// a man going from a standstill to a sprint did it bolt upright, and one
    /// stopping dead just stopped.
    pub drive: f32,
    /// 0..1: the ball is at his feet.
    ///
    /// Measured, somebody is within a stride of a slow ball in 72% of frames,
    /// so this is the standing condition of one player on the pitch rather
    /// than a rare event — and the man with the ball ran identically to the
    /// twenty-one without it.
    pub carrying: f32,
    /// 0..1: an outfielder off the ground — a header, not a save.
    ///
    /// Its own signal because the sprawl is not one: twelve outfield players
    /// in a recorded match leave the turf, up to 1.13 m, and every one of
    /// them was being drawn toppling sideways with both arms over his head
    /// like a keeper going full length.
    pub jump: f32,
    /// 0..1: he is heading the ball rather than kicking it, read together
    /// with `swing` exactly as `power` is.
    ///
    /// A ball arriving above head height is met with a head, and every one of
    /// them used to be drawn as a leg swing — a man at the back post hooking
    /// his boot up past his own ear at a cross. The action is nothing like a
    /// kick: it comes from the spine rather than the hip, the arms go out
    /// instead of counter-swinging, and the contact is at the top of the
    /// player rather than the bottom.
    pub header: f32,
    /// 0..1: a two-handed throw-in, ditto.
    ///
    /// The other strike a footballer makes that is not a kick, and there are
    /// forty-odd of them in a match. Both arms, over the head, feet planted —
    /// which is why it cannot borrow the keeper's throw, that one being
    /// pointedly one-armed.
    pub throw_in: f32,
    /// 0..1: he has just conceded.
    ///
    /// The only thing in this rig that is not derived from the position track,
    /// and it cannot be: eleven men standing still because they are sick and
    /// eleven standing still because they are waiting look identical from
    /// above. See [`Aftermath`](crate::players::aftermath::Aftermath) for where
    /// the signal comes from.
    ///
    /// Drives a slump — head down, shoulders forward, and the hands either
    /// on the head or on the hips. Nobody in football reacts to conceding by
    /// standing normally, and until this existed everybody did.
    pub despair: f32,
    /// …and 0..1 for the other eleven, who are sprinting to a corner flag
    /// with their arms up. Held apart rather than signed into one number
    /// because they are not opposites: one is a collapse and the other is an
    /// extension, and blending through zero would put a man who has just
    /// scored briefly into the pose of a man who has just conceded.
    pub elation: f32,
    /// **Hands on the head**, 0..1 — and it is the whole weight, mood
    /// included, not a selector that only means anything multiplied by
    /// [`Self::despair`].
    ///
    /// One of FOUR things a man does when the ball hits his net, and they do
    /// not blend into one another: the interpolation between hands on the
    /// head and arms hanging is a man holding them out sideways, which is
    /// neither. So the reaction is picked per player and the rest stay at
    /// zero — see [`Self::hands_on_hips`] and [`Self::doubled_over`], and
    /// the arms simply hanging, which is what is left when all three are
    /// zero.
    pub hands_to_head: f32,
    /// **Hands on the hips**, 0..1 — the other half of how a beaten player
    /// stands, and what a goalkeeper does for most of a match.
    ///
    /// ⚠ This was TRIED AND DROPPED in August 2026 with the note *"a pose
    /// the skeleton cannot reach is not a pose"*: with the elbow out, the
    /// forearm could only point forward and out, and what it drew was a man
    /// holding an invisible tray. That was true of the rig as it stood — it
    /// had no rotation about an arm's own long axis. It has one now, because
    /// the standing save needed a yaw at the shoulder
    /// ([`Joint::SAVE_ACROSS`]), and a yaw applied to an arm that is still
    /// hanging IS the long-axis roll. Composed innermost, so it turns the
    /// plane the elbow bends in without moving the upper arm at all. See
    /// [`Joint::HIPS_TURN`].
    pub hands_on_hips: f32,
    /// **Bent double with his hands on his knees**, 0..1 — the third, and
    /// the one that reads from furthest away, because it changes the
    /// silhouette rather than the arms.
    pub doubled_over: f32,
    /// **A goalkeeper with nothing to do**, 0..1: gloves up in front of him,
    /// clapping, shouting his back four up the pitch.
    ///
    /// The one thing in this rig that is neither derived from the recording
    /// nor handed over by the page — and it does not need to be. A keeper is
    /// out of the game for most of a match (measured: `Standing` alone is
    /// 8770 s of one recording), and what a real one does with those minutes
    /// is organise people. Drawn as a statue with his arms by his sides he
    /// is the only man on the pitch doing nothing at all, which is both
    /// wrong and, since the camera is often on him, conspicuous.
    ///
    /// Runs on the match clock and a per-player offset, so no two keepers
    /// are ever doing it at once and nothing has to be recorded.
    pub urging: f32,
    /// …and pointing somebody into position, SIGNED: negative for the left
    /// arm, positive for the right, magnitude 0..1.
    ///
    /// One field rather than two because only one arm ever goes, and which
    /// one is as much a part of the gesture as how far.
    pub pointing: f32,
    /// **How far off the floor he has pushed**, 0 flat out and 1 back on his
    /// feet — and 0 for anybody who never went down.
    ///
    /// A body coming up off the grass used to be the topple angle decaying
    /// to nothing with every limb frozen in the pose it landed in: a plank
    /// on a hinge at the hips, which is exactly how it was reported —
    /// *"he gets up like a robot"*. Getting up is a SEQUENCE. He rolls onto
    /// his front, gets a hand and a knee under himself, and pushes; and the
    /// hand and the knee are the whole of what makes it read as a man rather
    /// than a rotation.
    pub rising: f32,
    /// **How far from upright the CARRIAGE has him**, in radians — 0
    /// standing, π/2 flat on the grass.
    ///
    /// The one thing the pose never knew about the transform it is drawn
    /// under, and three separate faults came out of not knowing it. Every
    /// angle in this rig is measured against the body's own frame, which is
    /// right for a footballer standing on the turf and useless for one
    /// halfway through getting off it: an arm told to hang points wherever
    /// the trunk is pointing, and the trunk is somewhere between horizontal
    /// and vertical and moving. The limbs that have to find the GROUND —
    /// the thigh he kneels on, the arm he pushes with — take their angle
    /// off this instead, and then keep a constant angle to the world while
    /// the body turns under them.
    ///
    /// Zero for twenty-one players out of twenty-two, all match.
    pub over: f32,
    /// **He is on the grass AND he has just conceded**, 0..1.
    ///
    /// [`Self::despair`] is switched off by [`Self::dive`], and has to be —
    /// every slump in this rig is a pose for a man standing up, and applied
    /// to one lying down they put his arms through the turf. The upshot was
    /// that the four seconds a beaten keeper spends face down in his own
    /// six-yard box, which is the most recognisable image in the sport, had
    /// no reaction in them at all: he lay there in the neutral landing curl
    /// and then stood up. This is the channel for what he does DOWN THERE.
    pub beaten: f32,
    /// **Which way he is going, in the frame his LEGS are in**: `x` across
    /// his own body to his right, `y` out in front of him, together a unit
    /// vector while he is moving and zero while he is not.
    ///
    /// ⚠ **In the frame of his legs, not of his chest** — the two differ by
    /// [`Self::open`], and the whole of the lateral gait is what is left
    /// over once his legs have turned onto his run. Everybody who is running
    /// faces the way he is going, so for twenty-one players out of
    /// twenty-two this is `(0, 1)` and every term it drives collapses to the
    /// plain run cycle. The exception is the man the whole of this rig was
    /// hardest on: measured over a recorded match, of the frames where a
    /// keeper is moving with the ball inside forty metres, **47% are
    /// BACKWARD and 19% SIDEWAYS** — he retreats onto his line and shuffles
    /// across it facing the play, and he does the overwhelming majority of
    /// it slowly. Drawn as a forward run cycle that is one third short of
    /// the ground he covers, that is a man being slid across the grass,
    /// which is exactly how it was reported.
    pub course: Vec2,
    /// **How far his legs have turned off his chest onto the way he is
    /// going**, in radians, positive to his right.
    ///
    /// The single thing that separates a footballer from a goalkeeper in
    /// lateral movement, and the rig had no representation of it at all. A
    /// keeper covering his line is square to the play and MEANS to be: he
    /// side-steps, feet never crossing, and every constant in
    /// [`Joint::SHUFFLE_STANCE`] and below is about him. Nobody else on the
    /// pitch is doing that. An outfielder is across his own body for one of two
    /// reasons — he is jockeying, at walking pace, or his heading has not
    /// finished coming round onto a run he is already on, which
    /// [`crate::players::actors::Actors::PIVOT_RATE`] guarantees will happen
    /// every time anyone changes direction at speed — and the second is the
    /// overwhelming majority of it. **A man arcing round a turn is running, not
    /// shuffling.** Drawn as a keeper's shuffle he crouched a foot and a half
    /// with his feet a metre and a half apart at thirteen steps a second, which
    /// is how it was reported: *"they move sideways like invalids"*.
    ///
    /// So the legs turn onto the course and the chest does not, which is
    /// what "opening the hips" means and what the hips are for. It is not a
    /// second gait with a switch between them: the course is rotated by this
    /// same angle before anything reads it (see
    /// [`crate::players::actors::Actors::underfoot`]), so at a full opening the
    /// lateral terms are all multiplied by a residual `course.x` of nearly
    /// nothing and collapse on their own, and a jockeying defender at a walk
    /// keeps every one of them.
    pub open: f32,
    /// **The hip amplitude the ground he is covering demands**, in radians.
    ///
    /// The stride phase advances by ground covered, so the CADENCE has
    /// always been right; the amplitude did not know about it. A foot planted
    /// on the turf has to travel backwards relative to the body at exactly
    /// the speed the body is travelling forwards, and the sinusoid this rig
    /// swings a leg through does that at mid-stance when its amplitude is
    /// `stride / PI` of ground — which works out at 0.24 m at a walk and
    /// 0.49 m at a sprint, against the 0.19 m and 0.60 m the run cycle
    /// actually drew.
    ///
    /// So it is a FLOOR under the run cycle's own amplitude, not a
    /// replacement: at three metres a second and above the tuned sprint
    /// wins (and should, because a runner has a flight phase and his feet
    /// genuinely do go back faster than the ground), and below it the
    /// ground wins. See [`Joint::HIP_SWING`].
    pub carry_ground: f32,
    /// 0..1: he is playing a ball with his hands, on his feet.
    ///
    /// The save that is not a dive, and the one thing a goalkeeper does more
    /// often than any other that this rig could not draw at all. Measured
    /// over a recorded match, **84% of the balls that arrive at a keeper at
    /// pace arrive at a keeper who is on his feet** — and with `dive` read
    /// off recorded height and `carry` off the ball already being in the
    /// hold band, both of those are zero until after it has stopped. What
    /// was drawn was a ball hitting a man standing with his arms down.
    pub save: f32,
    /// Where he is reaching, in his own frame: `x` across his body, `y` up
    /// and down, each −1..1 over the envelope a set keeper can cover
    /// standing.
    ///
    /// The whole difference between a save low to his right and one up over
    /// his left shoulder, and the reason the reach cannot be one pose: a
    /// keeper's hands go to the ball, and the ball is somewhere different
    /// every time.
    pub save_aim: Vec2,
    /// 0..1: he did not catch it. Fists rather than gloves, and the arms
    /// snap through the ball instead of closing on it.
    pub parry: f32,
    /// 1 for the two goalkeepers, 0 for the other twenty.
    ///
    /// Most of what is keeper-only in this rig is already gated by a signal
    /// only a keeper ever has — `dive`, `reach`, `claimed`, `set`. The
    /// side-step is not: a defender drifting sideways at a walk takes one
    /// too, and should. What he must not also do is put his hands up in a
    /// goalkeeper's set, which is what [`Joint::armed`] would give him.
    pub keeper: f32,
}

impl Gait {
    /// A player doing nothing at all: standing still, with every layer in
    /// [`Joint::pose`] switched off.
    ///
    /// Exists so an actor can carry a gait before it has ever been posed —
    /// see [`PlayerActor::pose`](crate::players::actors::PlayerActor). Not a
    /// `Default`, because three of these fields are 1 and one is a unit
    /// vector, and a zeroed `Gait` is a man with no stride length standing
    /// on a course of nowhere.
    pub fn resting() -> Gait {
        Gait {
            phase: 0.0,
            run: 0.0,
            signature: 0.0,
            idle: 0.0,
            turn: 0.0,
            look: 0.0,
            look_pitch: 0.0,
            carry: 0.0,
            dive: 0.0,
            stretch: 0.0,
            grounded: 0.0,
            lead: 0.0,
            claimed: 0.0,
            reach: 0.0,
            set: 0.0,
            jump: 0.0,
            swing: 0.0,
            power: 0.0,
            foot: 0.0,
            spring: 1.0,
            elbows: 0.0,
            lean: 0.0,
            toes: 0.0,
            throwing: 0.0,
            header: 0.0,
            throw_in: 0.0,
            drive: 0.0,
            carrying: 0.0,
            despair: 0.0,
            elation: 0.0,
            hands_to_head: 0.0,
            hands_on_hips: 0.0,
            doubled_over: 0.0,
            urging: 0.0,
            pointing: 0.0,
            rising: 0.0,
            over: 0.0,
            beaten: 0.0,
            // Straight ahead, which is where everybody who is running is
            // going: the decomposition only has anything to say about the
            // man travelling one way and pointed another.
            course: Vec2::Y,
            // Legs square under him: nobody standing still is opening up.
            open: 0.0,
            carry_ground: 0.0,
            save: 0.0,
            save_aim: Vec2::ZERO,
            parry: 0.0,
            keeper: 0.0,
        }
    }

    /// **How far into the push off the floor he is**, 0 at both ends of the
    /// recovery and 1 in the middle.
    ///
    /// [`Self::rising`] is how far up he has come and [`Self::grounded`] is
    /// how far down he still is, and the whole of getting up lives in the
    /// product: flat out there is nothing to push with, standing there is
    /// nothing to push against, and in between he is on his knees. Times
    /// four because a product of two complements peaks at a quarter, so this
    /// is a weight and not a fraction of one.
    ///
    /// It also does the right thing for a beaten keeper, who stops halfway up
    /// on purpose ([`Actors::KNEELING`](crate::players::actors::Actors)): the
    /// recovery parks exactly where this is largest, and what he holds is the
    /// kneel.
    pub fn kneeling(self) -> f32 {
        (4.0 * self.rising * self.grounded).clamp(0.0, 1.0)
    }

    /// **…and how far into the part of it his HANDS are doing**, which
    /// peaks a third of the way up and not halfway.
    ///
    /// The two are separate because the arm is not long enough for them to
    /// be one. Measured through a real recovery: at a third of the way up
    /// his shoulder is 0.46 m off the grass, which a bent arm can just
    /// reach the turf from; by halfway it is 0.64 m, and the whole arm is
    /// 0.59 m long, so a keeper drawn planting his palms at the peak of the
    /// kneel is planting them a hand's width UNDER the pitch. Which is
    /// exactly what the first version did, by a quarter of a metre.
    ///
    /// `r·(1−r)²` peaks at a third; the 27/4 makes it a weight.
    ///
    /// ⚠ **…and he takes them off again when he arrives.** The curve is
    /// shaped like a transient, and on the kneel shelf it is not one: a
    /// beaten keeper's recovery parks with [`Self::rising`] and
    /// [`Self::grounded`] both pinned, so a man who had finished pushing
    /// was left with his palms planted on the turf and his back folded over
    /// them for as long as he stayed down — several seconds, on the one
    /// pose in the match the camera is certainly pointed at. Rendered, that
    /// is the *"stuck"* half of *"he falls to the side but not completely
    /// and looks stuck in the textures"*.
    ///
    /// What ends the push is being up on his knees, which is exactly what
    /// [`Self::kneeling`] says — so the two are complements and the shelf,
    /// which is where `kneeling` is largest, is where this is smallest.
    /// `PROP_PEAK` puts the peak back where the geometry above wants it
    /// after the complement has taken a bite out of it.
    pub fn propping(self) -> f32 {
        const PROP_PEAK: f32 = 2.6;
        (PROP_PEAK * 6.75 * self.rising * self.grounded * self.grounded)
            .clamp(0.0, 1.0)
            .min(1.0 - self.kneeling())
            .max(0.0)
    }
}

impl Joint {
    /// The angle the leading leg swings through, standing and at a sprint.
    const HIP_SWING: (f32, f32) = (0.10, 0.62);
    /// How much further than the ground he covers a runner's legs may swing,
    /// as a share — see [`Joint::stepping`], where the flight phase is
    /// bounded by it.
    ///
    /// ⚠ **It is the same claim as the 105% floor at 6 m/s in
    /// `the_planted_foot_carries_the_ground_across_its_whole_stance`**, and
    /// the two have to be read together: that test measures what the boot
    /// does with the ground while it is down, and a runner's foot genuinely
    /// goes back faster than the turf because for part of the cycle neither
    /// foot is on it. At 0.10 the measured figure came out at 104% and the
    /// foot skated by a hair.
    const FLIGHT_BONUS: f32 = 0.14;
    const KNEE_FLEX: (f32, f32) = (0.16, 1.55);
    /// The shoulder through the run, and the elbow that goes with it.
    ///
    /// A sprinter drives his arms from a bent elbow held at roughly a right
    /// angle THROUGHOUT — the swing is at the shoulder and the forearm goes
    /// along with it. 0.75 of shoulder against 1.05 of elbow threw the arm
    /// out until the forearm was horizontal at the front of the stride,
    /// which is a man reaching for something rather than one running.
    /// Less shoulder and more elbow is the same energy in the right joint.
    const ARM_SWING: (f32, f32) = (0.10, 0.52);
    /// …and how much more of it a man who is genuinely driving adds. See
    /// [`Joint::arming`].
    const ARM_DRIVE: f32 = 0.22;
    /// **How far apart two men's elbows and two men's leans are**, as a
    /// share either side of the squad mean.
    ///
    /// Both were 0.24 and 0.16 of [`Gait::signature`], which is also how
    /// wide he holds his arms and where in the cycle he starts. Widened as
    /// well as separated: at ±24% every runner in the squad had visibly the
    /// same arm, and the elbow is the thing a touchline eye tells them apart
    /// by. See [`Gait::elbows`].
    const ELBOW_SPREAD: f32 = 0.40;
    const LEAN_SPREAD: f32 = 0.34;
    const ELBOW_FLEX: (f32, f32) = (0.25, 1.25);
    /// **How far the elbow opens behind him and closes in front**, in
    /// radians at a flat sprint.
    ///
    /// The shoulder says how far the arm swings; this is most of how far the
    /// HAND goes, because a folded arm is a short one and the fold is what
    /// changes through the cycle. A sprinter's hand comes past his hip on a
    /// nearly straight arm and arrives at his chin on a tight one — the arm
    /// lengthens into the drive and shortens on the recovery, which is the
    /// same trick a leg plays and for the same reason.
    const ELBOW_DRIVE: f32 = 0.34;
    const LEAN: (f32, f32) = (0.045, 0.20);
    /// How far a running player's whole body rises as the stride closes up,
    /// in metres.
    const BOB: f32 = 0.075;
    /// And how far a standing one rises and falls just breathing. Small on
    /// purpose — this is the difference between a statue and a man waiting,
    /// not a visible bounce.
    const BREATHE: f32 = 0.013;
    /// How far a standing player rolls as his weight goes from one foot to
    /// the other, in radians.
    const WEIGHT_SHIFT: f32 = 0.038;
    /// Roll of the torso through the stride — a runner rocks, he does not
    /// stay square.
    const ROCK: f32 = 0.055;
    /// How far the hips counter-rotate against the shoulders.
    ///
    /// Raised with `CHEST_TWIST` below: 0.065 against 0.14 was about a
    /// third of the separation a running body actually shows, and a trunk
    /// that barely turns is most of what "very stiff" is. The pair is what
    /// carries the arm swing — the arms counter the legs THROUGH the
    /// shoulders, so under-rotating here makes the arms read as swinging
    /// off a fixed post.
    const HIP_TWIST: f32 = 0.105;
    /// …and the chest against them, the other half of the same separation.
    const CHEST_TWIST: f32 = 0.22;
    /// How much of the opening the CHEST takes, the rest being left to the
    /// hips. See [`Gait::open`].
    ///
    /// Small on purpose. The hips lead a turn and the shoulders follow, and
    /// the separation between them is most of what makes a crossover read as
    /// one — but the chest is also where this rig hangs the head, and a
    /// player's eyes stay on the ball. A fifth of it is the difference
    /// between a plausible trunk and a man twisted square off his own legs;
    /// it costs about fifteen degrees of gaze at the very hardest turn,
    /// which is fifteen degrees a real player loses too.
    const OPEN_CHEST: f32 = 0.22;
    /// How far a player leans into a turn at full tilt.
    const BANK: f32 = 0.30;
    /// The hold, as rotations about X in the shoulder's and the elbow's own
    /// frames. The upper arm eases forward off the ribs and the forearm comes
    /// most of the way up, so the two make a shelf in front of the chest —
    /// which is how a goalkeeper carries a ball he has just claimed, elbows in
    /// and gloves under it, not out in front of him at arm's length.
    ///
    /// [`Physique::CRADLE`] is where these two angles land the wrists. Keep
    /// them together.
    const CRADLE_SHOULDER: f32 = -0.12;
    const CRADLE_ELBOW: f32 = -1.55;
    /// A little outward roll at the shoulder, so the gloves close on the sides
    /// of the ball rather than inside it.
    const CRADLE_SPREAD: f32 = 0.03;
    /// And the wrists under it: the gloves come up around the ball rather
    /// than hanging off the ends of the forearms.
    const CRADLE_WRIST: f32 = -1.15;
    /// Where the arms are on the way up, before the extension opens them
    /// out. A keeper leaves the ground with his elbows still bent and his
    /// hands in front of his chest — the reach happens in the air, which is
    /// the whole reason [`Gait::stretch`] exists.
    const LAUNCH_SHOULDER: f32 = -1.05;
    const LAUNCH_ELBOW: f32 = -1.00;
    /// The reach, as rotations in the shoulder's and elbow's own frames. The
    /// leading arm goes past the head and its elbow comes nearly straight — a
    /// keeper at full stretch is measured from his fingertips, and every
    /// degree left in the elbow is a centimetre he does not cover.
    ///
    /// Read together with the topple on [`Carriage`]: rolled onto his side,
    /// arms along his own up-axis point ACROSS the goal, which is what makes
    /// the same two angles serve both the dive and the standing leap.
    const REACH_SHOULDER: f32 = -2.48;
    const REACH_ELBOW: f32 = -0.07;
    /// Hands apart rather than together — he is covering an area, not
    /// catching a pass.
    ///
    /// Applied with the side NEGATED, unlike every other spread in this rig,
    /// and it has to be: a roll about +Z carries a hanging arm outward and a
    /// RAISED one inward, because the arm has swung through the vertical and
    /// taken its sense of "out" with it. Signed like the others, this pulled
    /// the two gloves in to 11 cm apart — inside the shoulders, which is the
    /// one thing a keeper covering his goal is not doing.
    const REACH_SPREAD: f32 = 0.30;
    /// What the TRAILING arm does instead, as offsets on the three above.
    ///
    /// Both arms at full stretch is the pose from the front of a cereal box.
    /// The real one is asymmetric: the top arm is thrown through the ball,
    /// and the bottom one stays out at shoulder height with the elbow soft —
    /// partly for balance, mostly because it is the one that hits the ground
    /// first and he knows it.
    const TRAIL_SHOULDER: f32 = 0.80;
    const TRAIL_ELBOW: f32 = -0.55;
    const TRAIL_SPREAD: f32 = 0.24;
    /// And what the same two arms do once they have the ball: brought in off
    /// the spread until the gloves are a ball's width apart, and both of
    /// them, because the trailing arm comes onto it too.
    const CLAIM_SPREAD: f32 = -0.22;
    /// Gloves at full stretch: broken back off the forearm and turned out, so
    /// the palms face what is coming rather than each other.
    const REACH_WRIST: f32 = -0.55;
    const REACH_WRIST_SPREAD: f32 = 0.40;
    /// The legs in a dive: trailing behind the hips and all but straight. A
    /// footballer in the air has nothing to push against, so the run cycle
    /// has to stop dead rather than fade out over a stride.
    const DIVE_HIP: f32 = 0.22;
    const DIVE_KNEE: f32 = 0.12;
    /// And how the two of them differ, scaled by how square the dive is. The
    /// leg on the side he is going to is the one he pushed off: it finishes
    /// straight and trailing. The far one folds up over it.
    const DIVE_SCISSOR_HIP: f32 = 0.34;
    const DIVE_SCISSOR_KNEE: f32 = 1.35;
    /// The chest in flight: arched away from the ball and turned onto it.
    ///
    /// The torso used to be cancelled outright by a dive — mathematically
    /// upright, which reads as a shop dummy laid on its side. A keeper in the
    /// air is the most extended a footballer ever gets: the spine is in
    /// extension and the leading shoulder is rotated hard through the line of
    /// the ball.
    const DIVE_ARCH: f32 = -0.30;
    const DIVE_TWIST: f32 = 0.38;
    /// And on the grass: the curl over the impact, the legs coming up, the
    /// arms pulling in. Landing is not a freeze-frame of the flight.
    const DOWN_CURL: f32 = 0.34;
    const DOWN_HIP: f32 = -0.58;
    const DOWN_KNEE: f32 = 1.10;
    const DOWN_SHOULDER: f32 = -0.85;
    const DOWN_SPREAD: f32 = 0.04;
    const DOWN_ELBOW: f32 = -1.35;
    /// …and what the arm he came down ON does instead, which is not that.
    ///
    /// The underneath shoulder is at the height of the grass once the body is
    /// genuinely flat — that is what lying on your side means — so the pose
    /// above, which folds an arm across the chest, put that glove seven
    /// centimetres into the turf. It cannot hang either: a standing man
    /// carries his arms eight degrees out from his sides, and on the floor
    /// "out" is straight down. The ground holds it, so it goes out along the
    /// ground: swung up past his head, near enough straight, and lifted clear
    /// of the turf by the spread. Which is also where the dive left it.
    const GRASS_SHOULDER: f32 = -2.45;
    const GRASS_SPREAD: f32 = 0.28;
    const GRASS_ELBOW: f32 = -0.42;
    /// **Getting up**, which nothing in this rig used to draw at all.
    ///
    /// The recovery was the topple angle decaying to nothing with every limb
    /// frozen in the pose it landed in — a plank on a hinge at the hips, and
    /// reported as exactly that. A man comes off the floor in a shape: he
    /// rolls onto his front ([`Actors::ROLLS_OVER`]), draws his knees under
    /// himself, plants his hands on the turf and pushes.
    ///
    /// **These are worked against the carriage he is under in the middle of
    /// the movement**, which is where [`Gait::rising`] against
    /// [`Gait::grounded`] peaks: about 60° off upright, with his hips 0.29 m
    /// off the grass. That is not a man on one knee with his chest up — from
    /// that height the thigh cannot reach the ground without going through
    /// it — it is a man on both knees with his hands down, and the numbers
    /// are the angles that put a knee and a palm on the turf from there.
    /// **The leg that finds the ground is measured against the GROUND**, not
    /// against the trunk — see [`Gait::over`]. The thigh holds 20° forward
    /// of vertical however far over the body is, so the knee is always the
    /// same 0.43 m below the hips instead of swinging from under him to out
    /// behind him as the carriage comes up; the shin then folds back from
    /// there until it lies flat, which is a constant, because a knee is
    /// hinged to its own thigh and does not care which way the world is.
    ///
    /// Both were fixed angles at first. What that draws is a keeper whose
    /// knees are tucked to his chest while his body is nearly upright and
    /// his boots are 16 cm under the pitch — which is what the measurement
    /// said, and which no amount of adjusting the height could have fixed,
    /// because the fault was the leg pointing the wrong way rather than the
    /// hips being at the wrong height.
    const RISE_THIGH: f32 = 0.35;
    const RISE_KNEE: f32 = 1.92;
    /// **The arms are on a different clock from the legs** — see
    /// [`Gait::propping`]. Their moment is a THIRD of the way up, where the
    /// shoulder is still low enough for a hand to reach the turf; the
    /// shoulder cancels the carriage exactly, so the arm hangs straight
    /// DOWN whatever the trunk is doing, and the elbow keeps the last of it
    /// off the grass.
    const RISE_SPREAD: f32 = 0.20;
    /// …and the elbow FOLDS and then straightens across the push, because a
    /// fixed one cannot do it: at a quarter of the way up his shoulder is
    /// 0.44 m off the grass and the arm is 0.59 m long, so a straight one
    /// reaches through the pitch; by half way the shoulder is 0.85 m up and
    /// a folded one leaves his hands in mid-air. Interpolated on the rise
    /// itself, which is the only thing that knows.
    const RISE_ELBOW: (f32, f32) = (-1.55, 0.65);
    /// The palm flat on the turf, which is what he is pushing against.
    const RISE_WRIST: f32 = 0.75;
    /// …and the head, which comes up FIRST. A man looks where he is going
    /// before he goes there, and the neck is the cheapest thing in the rig
    /// that says the movement is his idea rather than something happening
    /// to him.
    const RISE_HEAD: f32 = -0.65;
    const RISE_CURL: f32 = 0.10;
    /// **And the man who has just been beaten**, still on the grass.
    ///
    /// [`Gait::despair`] is switched off by the dive, so the four seconds
    /// after a goal that the camera actually holds on had no reaction in
    /// them: he lay in the neutral landing curl and then stood up. Face into
    /// the turf, the trunk folded further round, and the free arm over the
    /// top of his head rather than tucked across his chest — which is the
    /// picture, and is also where it can go, since the top shoulder is the
    /// one that is clear of the ground.
    const BEATEN_CURL: f32 = 0.12;
    const BEATEN_HEAD: f32 = 0.42;
    const BEATEN_SHOULDER: f32 = -2.30;
    const BEATEN_SPREAD: f32 = -0.12;
    const BEATEN_ELBOW: f32 = -2.20;
    /// The set: knees bent, chest over the toes, gloves up and out in front.
    const SET_HIP: f32 = -0.30;
    const SET_KNEE: f32 = 0.55;
    const SET_LEAN: f32 = 0.26;
    /// How far the whole figure settles as those knees bend, in metres.
    ///
    /// Not a style choice — it is what the leg angles above cost in height,
    /// and without it a crouching keeper's boots hang three and a half
    /// centimetres over the grass.
    const SET_DROP: f32 = 0.035;
    /// **The arms of the set, and the elbow is the whole of it.**
    ///
    /// These used to be −0.62 at the shoulder and −1.10 at the elbow, which
    /// leaves 135° between the two bones: an arm that is very nearly
    /// STRAIGHT, held out in front with a flat hand on the end of it. That
    /// is the picture the user reported — *"he does nothing but stick them
    /// out"* — and it was literally what the numbers said. A keeper's ready
    /// position is the opposite shape: the upper arm hangs, barely forward
    /// of his ribs, and the elbow is bent to a right angle so the forearms
    /// come UP. The hands end up in front of his waist rather than at the
    /// end of his reach, which is where they can go anywhere from.
    ///
    /// Worked as positions, since angles are unarguable and 93° at the
    /// elbow is not: they put the gloves at (±0.26, 1.25, 0.36) — just above
    /// the waist, a third of a metre in front of him, outside his own hips.
    /// Pinned by `the_set_bends_his_arms`.
    const SET_SHOULDER: f32 = -0.30;
    const SET_SPREAD: f32 = 0.46;
    const SET_ELBOW: f32 = -1.62;
    /// …and the wrists cocked back, so the fingers point UP and the palms
    /// face the shot rather than the grass.
    ///
    /// The forearm is only 20° above horizontal, so a hand carried in line
    /// with it is a plank held out at the ball. This is what turns the pair
    /// of them into the two flat surfaces the whole posture exists to
    /// present.
    const SET_WRIST: f32 = -0.85;
    /// **How far a keeper's raised gloves answer the step he is taking**, in
    /// radians: the alternating pump of a walk, and the sway of a side-step
    /// that carries the pair across him together.
    ///
    /// Small on purpose. The point is not a swing — a man holding his hands
    /// ready is deliberately keeping them where they are useful — it is that
    /// they are ATTACHED to a body that is working. Rendered, a keeper whose
    /// gloves hold one pixel through eight frames of a shuffle is the whole
    /// of *"he moves like a disabled person"*, and the fix is a couple of
    /// centimetres rather than an animation.
    const READY_PUMP: f32 = 0.26;
    const READY_SWAY: f32 = 0.20;
    /// **The side-shuffle.** How wide he sets his feet to move across, and
    /// how far each one then travels.
    ///
    /// A goalkeeper covers his goal sideways, and he does it square to the
    /// play with his feet never crossing — which is a different gait from a
    /// run, not a run drawn at an angle. Measured over a recorded match,
    /// 19% of the frames in which a keeper is moving with the ball inside
    /// forty metres are travelling across his own body.
    ///
    /// The STEP is not a constant at all: it is the same hip amplitude the
    /// stride uses, so the foot's excursion runs along the course he is
    /// travelling on and carries exactly the ground he covers, whichever way
    /// he is going. This is how much wider than that he plants his feet.
    ///
    /// ⚠ **A RATIO of the step, and it has to exceed 1, or his feet cross.**
    /// The two legs are half a cycle apart, so their lateral offsets are
    /// equal and opposite and the base between them is the only thing
    /// keeping the swinging foot from passing through the planted one. A
    /// fixed angle will not do it — the step grows with his pace and the
    /// base has to grow with the step.
    const SHUFFLE_STANCE: f32 = 1.08;
    /// **How much of the cycle a side-stepping foot spends ON THE GROUND.**
    ///
    /// The number that decides whether he reads as a man or as a pair of
    /// dividers. Rendered, the sinusoid this replaces was unmistakable: the
    /// two legs splayed together and closed together, every frame a mirror
    /// of itself, and — because a sinusoid is stationary at exactly one
    /// INSTANT of its stance and moving everywhere else — **both feet skated
    /// for the whole cycle**. Seen from the front, where a lateral gait is
    /// nothing but the feet, that is the entire picture.
    ///
    /// Past a half there is double support: for `2·DUTY − 1` of the cycle
    /// both feet are down, which is what makes a shuffle a shuffle rather
    /// than a run.
    pub(crate) const SHUFFLE_DUTY: f32 = 0.62;
    /// …and what that costs in amplitude. A sinusoid matches the turf at
    /// mid-stance when its excursion is `stride / PI`; a tread that is
    /// LINEAR across the stance has to cover the same ground at a constant
    /// rate over `DUTY` of the cycle, which takes `DUTY × stride`. The two
    /// differ by exactly `DUTY × PI`, and `Gait::carry_ground` is quoted in
    /// the first, so the lateral step is scaled into the second.
    const TREAD_GAIN: f32 = Self::SHUFFLE_DUTY * PI;
    /// How high the swinging foot picks up, in radians of knee and hip.
    ///
    /// It has to leave the grass. A foot that slides out and slides back is
    /// the whole of what "not a human" looks like from the front, and it is
    /// the one thing a stance/swing split makes drawable at all: with both
    /// legs mirrored there was no swing to lift.
    const SHUFFLE_PICKUP: f32 = 0.16;
    const SHUFFLE_HIP_PICKUP: f32 = -0.13;
    /// **How far a side-stepping player carries his knees bent**, in
    /// radians — the carriage the push works either side of.
    ///
    /// The one thing a lateral gait genuinely does ask the knees for.
    /// Everything else in this rig bends a knee through `tuck`, which is
    /// keyed to where the leg is in a STRIDE — right for a run, where the
    /// knee folds through the swing and straightens onto the plant, and
    /// wrong for a side-step, where there is no swing through the sagittal
    /// plane to fold into. Driven off `tuck` at the share of a run cycle a
    /// side-step used to be given, the stance knee swung between 4° and 53°
    /// on every step, which is a man's legs buckling under him.
    ///
    /// ⚠ It was a flat CONSTANT for a while after that, and a constant is a
    /// strut: rendered, the legs were two straight poles swinging from the
    /// hips, which is the other way to have no athleticism in a gait made
    /// almost entirely of legs. [`Joint::SHUFFLE_PUSH`] is how far either
    /// side of it the working leg goes.
    ///
    /// Paid for in height by [`Joint::stance_drop`], like every other bend
    /// in this rig that is a real loss rather than a pose.
    const SHUFFLE_KNEE: f32 = 0.30;
    /// …and how much of that the push takes off it and the landing puts
    /// back, as a share. See [`Joint::driving`]: 0.55 runs the working knee
    /// between 8° and 27°.
    const SHUFFLE_PUSH: f32 = 0.55;
    /// The feet turn out toward the way he is going, and roll through the
    /// push. Neither axis existed: the ankle only pitches, and its pitch is
    /// signed by `course.y`, so travelling sideways every boot in the squad
    /// held one angle for the whole cycle.
    const TOE_OUT: f32 = 0.20;
    const FOOT_ROLL: f32 = 0.13;
    /// The pelvis lists over the foot that is carrying him and the chest
    /// rides with it. A trunk that holds still while the legs work is a
    /// mechanism with a body bolted on top.
    const SHUFFLE_LIST: f32 = 0.085;
    const SHUFFLE_TWIST: f32 = 0.16;
    /// …and the arms come across with the step rather than being held out
    /// at a constant width.
    const SHUFFLE_ARM: f32 = 0.18;
    /// **How much shorter a step is going sideways, and going backwards.**
    ///
    /// Nobody takes running strides across himself: a side-step is short and
    /// quick, a backpedal somewhere between the two, so the same ground has
    /// to be spent on more steps. Lives here rather than with the stride
    /// model because both ends need it — the model, to advance the phase
    /// faster, and the amplitude floor above, which is a claim about a
    /// FORWARD run and has no business setting the size of a side-step.
    const SIDE_STEP: f32 = 0.30;
    const BACK_STEP: f32 = 0.78;
    /// **A set goalkeeper is never still.** He dances: small alternating
    /// steps on the spot, so that whichever way the ball goes he is already
    /// moving. Every other standing pose in this rig is a statue plus a
    /// breath, which is defensible for a centre-half waiting for a throw-in
    /// and is exactly wrong for the one man on the pitch whose whole job is
    /// to be about to move.
    ///
    /// Alternating, so one boot is always on the grass and the body does not
    /// have to pay for it in height. Off the idle clock rather than the
    /// stride, because he is covering no ground — the same reason the breath
    /// and the weight shift are. `TOES_RATE` against `Actors::IDLE_RATE`
    /// puts it at about 1.3 steps a second.
    const TOES_RATE: f32 = 4.5;
    const TOES_KNEE: f32 = 0.30;
    const TOES_HIP: f32 = -0.16;
    /// And he leans the way he is going, which is the whole reason a
    /// side-step reads as urgent rather than as a man sliding.
    const SHUFFLE_LEAN: f32 = 0.12;
    /// **The backpedal.** Nearly half of a keeper's travel near his own
    /// goal is AWAY from the ball — he is dropping onto his line while
    /// watching the play, and no footballer does that by running backwards
    /// with a forward run cycle.
    ///
    /// The stride reverses on its own (the hip swing is signed by
    /// [`Gait::course`]); these are what a body does differently going the
    /// other way — knees higher in front, weight back over the heels, and
    /// up on the balls of the feet, because a man moving backwards never
    /// puts a heel down.
    const BACKPEDAL_KNEE: f32 = 0.42;
    const BACKPEDAL_LEAN: f32 = -0.16;
    const BACKPEDAL_ANKLE: f32 = 0.30;
    /// **The save he makes on his feet**, which is most of them: measured,
    /// 84% of the balls that arrive at a keeper at pace arrive at one who
    /// never leaves the ground.
    ///
    /// The two ends of the arm's travel, from a ball at his boots to one
    /// over his head, interpolated on [`Gait::save_aim`]`.y`. Both arms go,
    /// because a keeper takes everything he can two-handed — the one-handed
    /// save is what a dive is for.
    const SAVE_LOW: f32 = -0.42;
    const SAVE_HIGH: f32 = -2.52;
    /// …and how far across his body they travel, as a YAW at the shoulder.
    ///
    /// ⚠ Not the Z-roll every other reach in this rig uses. A roll about the
    /// body's forward axis only moves a hand that is UP: an arm held
    /// straight out in front lies along the roll axis, so rolling it does
    /// nothing at all — and chest height is exactly where a keeper's hands
    /// are for most of the saves he makes. A yaw swings the arm sideways
    /// from any elevation, which is what the pose needs.
    const SAVE_ACROSS: f32 = 0.85;
    /// The elbow through the save: soft at the chest, straight at full
    /// stretch, and folded in as he takes the pace off it.
    const SAVE_ELBOW: f32 = -0.30;
    const SAVE_WRIST: f32 = -0.62;
    /// Going down to one at his feet: he folds at the waist, drops his
    /// knees under him and steps across. All three off the same aim, so a
    /// ball at head height gets none of them.
    const SAVE_STOOP: f32 = 0.72;
    const SAVE_KNEE: f32 = 0.62;
    /// …and the thigh comes forward under him with it.
    const SAVE_HIP: f32 = -0.45;
    const SAVE_DROP: f32 = 0.085;
    const SAVE_STEP: f32 = 0.22;
    const SAVE_LEAN: f32 = 0.34;
    /// **Going UP to one, which is the half of a save this rig did not
    /// have.** He comes off his heels, drives the knees straight and takes
    /// the hips through — the same movement as a jump, made without leaving
    /// the ground.
    ///
    /// The legs used to take the save at [`Joint::SET_HIP`] and
    /// [`Joint::SET_KNEE`] whatever it was aimed at, because the only term
    /// on them was [`Joint::stooping`] and that is zero for a ball above his
    /// chest. So a keeper reaching over his own head was drawn standing in
    /// the set crouch with his knees bent — the one thing a man extending
    /// upward is certainly not doing, and most of what *"he has none of the
    /// spring of a real goalkeeper"* is. It matters more than a dive does:
    /// **84% of the balls that arrive at a keeper at pace arrive at one who
    /// never leaves the ground**, and a recorded match takes him off it
    /// twice in five minutes.
    ///
    /// `SAVE_RISE` is the height it actually buys him, paid at the hips —
    /// the knees straightening gives back [`Joint::SET_DROP`] and the toes
    /// find the rest.
    const SAVE_RISE_HIP: f32 = 0.34;
    const SAVE_RISE_KNEE: f32 = -0.62;
    const SAVE_RISE_TOE: f32 = 0.62;
    const SAVE_RISE: f32 = 0.085;
    /// Reaching over his own head he arches BACK, and comes out of the set's
    /// forward lean to do it — the two together are what puts his gloves above
    /// the crossbar instead of in front of his face.
    const SAVE_ARCH: f32 = -0.22;
    /// A parry is not a catch. The gloves turn out into a flat surface, the
    /// arms stay long, and the hands never come together — everything the
    /// catch does is the other way round.
    const PARRY_SPREAD: f32 = 0.30;
    const PARRY_WRIST: f32 = 0.55;
    /// An outfielder in the air: knees folded under him, arms out from his
    /// sides for balance. A header, and nothing like a save.
    const JUMP_HIP: f32 = -0.22;
    const JUMP_KNEE: f32 = 1.15;
    const JUMP_SHOULDER: f32 = -0.62;
    const JUMP_SPREAD: f32 = 0.52;
    const JUMP_ELBOW: f32 = -0.50;
    /// **The slump.** A man who has just conceded, in FOUR variants.
    ///
    /// All of them fold the trunk forward and drop the chin; what differs is
    /// what the arms do, which is the whole read at broadcast distance.
    ///
    /// **Hands to the head** is the one everybody pictures: upper arms up
    /// past the ears, forearms folded back so the gloves come onto the
    /// crown. **Limp** is arms simply hanging, shoulders rolled in, head
    /// down, walking. **Hands on the hips** is the commonest of the four in
    /// life and was missing from this rig entirely — see [`Self::HIPS_TURN`]
    /// for what changed. **Doubled over**, hands on the knees, is the one
    /// that reads from furthest away, because it is a different silhouette
    /// rather than a different pair of arms.
    ///
    /// One reaction per player, picked off his own hash and held for the
    /// match. They are NOT blended: the interpolation between a man with his
    /// hands on his head and a man bent double is a man doing neither, and
    /// eleven players caught halfway between two reactions is exactly the
    /// crowd-waiting-for-a-bus this whole layer exists to stop being.
    ///
    /// Sign convention as everywhere else here: negative X at the shoulder
    /// carries the hand forward and up; negative X at the elbow folds the
    /// forearm toward the front of the upper arm.
    const SLUMP_STOOP: f32 = 0.36;
    const SLUMP_HEAD_DOWN: f32 = 0.40;
    const SLUMP_KNEE: f32 = 0.22;
    /// …and the height that knee costs, in metres. Same bookkeeping as
    /// [`Self::SET_DROP`], scaled off it by how much less the knee bends:
    /// anything in this rig that folds a leg has to pay for it, or the
    /// boots hang above the grass.
    const SLUMP_DROP: f32 = 0.006;
    const SLUMP_LIMP_SHOULDER: f32 = 0.12;
    const SLUMP_LIMP_SPREAD: f32 = 0.04;
    const SLUMP_LIMP_ELBOW: f32 = -0.28;
    const SLUMP_HEAD_SHOULDER: f32 = -2.15;
    const SLUMP_HEAD_SPREAD: f32 = -0.06;
    const SLUMP_HEAD_ELBOW: f32 = -2.70;
    /// Gloves folded over the crown rather than pointing off the end of the
    /// forearm — the wrist is what makes it read as hands on the head
    /// instead of two arms waving.
    const SLUMP_WRIST: f32 = -0.75;
    /// **Hands on the hips, and `HIPS_TURN` is the whole reason it exists.**
    ///
    /// The arm hangs — it is barely moved from where it hangs anyway — and
    /// the elbow bends to seventy degrees. On its own that points the
    /// forearm forward and out, which is the tray this pose was abandoned
    /// over in August 2026. `HIPS_TURN` is a yaw at the shoulder composed
    /// INNERMOST, before the pitch and the spread: applied to an arm that is
    /// still hanging along the Y axis it moves the upper arm not at all, and
    /// turns the plane the elbow bends in through 66°. The forearm then goes
    /// down and IN, and the wrist arrives on the crest of the hip.
    ///
    /// ⚠ Innermost is not a detail. The save's yaw ([`Self::SAVE_ACROSS`])
    /// is the outermost rotation of its shoulder because it means "swing the
    /// whole arm across him"; this one means "roll the arm in its socket",
    /// and the two are the same quaternion in a different order.
    ///
    /// Worked as positions: they put the wrists at (±0.18, 0.94, 0.06) —
    /// which is the top of the hip bone, an inch outside the shorts.
    const HIPS_SHOULDER: f32 = 0.12;
    const HIPS_SPREAD: f32 = 0.55;
    const HIPS_TURN: f32 = -1.15;
    const HIPS_ELBOW: f32 = -1.20;
    const HIPS_WRIST: f32 = -0.30;
    const HIPS_WRIST_TURN: f32 = 0.25;
    /// **Bent double with his hands on his knees**, worked as positions
    /// because none of the five angles means anything on its own.
    ///
    /// The trunk folds a full radian, which carries the shoulders 0.40 m
    /// forward of the hips and drops them to 1.06 m. The arm then hangs
    /// VERTICALLY out of that — which relative to a chest lying at 57° is a
    /// shoulder of −1.00, cancelling the fold exactly, and not the +0.72
    /// this had first, which pointed both arms out behind him like a diver
    /// on a board. The elbow takes the last 10 cm back toward his own legs.
    ///
    /// The knee is the half that is easy to get wrong twice over. Bending it
    /// alone swings the shin BACKWARD and lifts the boot off the grass — the
    /// legs hang from the hips and there is nothing under them — so the hip
    /// has to flex with it, and then the pair of them shorten the leg by
    /// 0.147 m and the body has to come down by exactly that. Together they
    /// put the knee at (±0.088, 0.455, 0.293) and the wrist at (±0.14,
    /// 0.485, 0.297), which is a hand on a knee.
    const DOUBLED_STOOP: f32 = 1.00;
    const DOUBLED_SHOULDER: f32 = -1.00;
    const DOUBLED_SPREAD: f32 = -0.06;
    const DOUBLED_ELBOW: f32 = 0.35;
    const DOUBLED_WRIST: f32 = -0.30;
    const DOUBLED_HIP: f32 = -0.70;
    const DOUBLED_KNEE: f32 = 1.15;
    const DOUBLED_DROP: f32 = 0.147;
    /// …and the neck, which comes UP out of the fold: a man blowing with his
    /// hands on his knees is looking at the grass a yard in front of him,
    /// not at his own boots.
    const DOUBLED_HEAD: f32 = -0.55;
    /// **A goalkeeper organising his defence.** Gloves up in front of his
    /// CHEST, elbows in and down, clapping — see [`Gait::urging`].
    ///
    /// The upper arm barely leaves his side and the elbow does the work,
    /// which puts the gloves at (±0.09, 1.30, 0.35). A shoulder of −1.05 was
    /// the first try and it lifted the whole arm to bring the gloves up in
    /// front of his FACE, which is a man surrendering. The spread is
    /// negative because they have to come TOGETHER to meet.
    const URGE_SHOULDER: f32 = -0.35;
    const URGE_ELBOW: f32 = -1.80;
    const URGE_WRIST: f32 = -0.30;
    /// **The clap is a YAW at the shoulder, not a roll**, and it is the same
    /// trap [`Self::SAVE_ACROSS`] documents: a roll about the body's forward
    /// axis moves a hand that is UP and does nothing at all to one held out
    /// in FRONT, which is exactly where these are. Written as a roll the two
    /// gloves travelled seven centimetres and never got closer than a
    /// shoulder's width — a man conducting.
    ///
    /// `URGE_YAW` is where they meet and `CLAP_OPEN` is how far back out
    /// they go; between them the gloves run 0.03 m to 0.24 m apart.
    const URGE_YAW: f32 = 0.40;
    const CLAP_OPEN: f32 = 0.30;
    /// …and how many times a second. Two beats and a pause is what a real
    /// one is, so the wave is rectified rather than a plain sinusoid — a
    /// pair of hands that spends half the cycle travelling evenly back out
    /// is not clapping.
    const CLAP_RATE: f32 = 7.0;
    /// …and pointing somebody into position: one arm out at shoulder
    /// height, near enough straight, index finger extended. Forward as much
    /// as sideways, because the men he is shouting at are in front of him.
    const POINT_SHOULDER: f32 = -1.52;
    const POINT_YAW: f32 = 0.45;
    const POINT_ELBOW: f32 = -0.22;
    const POINT_WRIST: f32 = -0.28;
    /// How closed the other four fingers are behind a pointed one.
    const HAND_POINTING: f32 = 0.88;
    /// **And the other eleven.** Arms up and open, chest out, head up.
    ///
    /// Deliberately smaller than the dive's [`Self::REACH_SHOULDER`]: a man
    /// celebrating runs with his arms up, he does not hold them rigidly
    /// overhead, and the run cycle underneath still shows through because
    /// the layer is blended rather than substituted.
    const CHEER_SHOULDER: f32 = -2.05;
    const CHEER_SPREAD: f32 = 0.34;
    const CHEER_ELBOW: f32 = -0.42;
    const CHEER_ARCH: f32 = -0.16;
    const CHEER_HEAD_UP: f32 = -0.22;
    /// **The foot through the stride**, as the two ends of its travel.
    ///
    /// Sign, worked from the rig's own convention: the toe sits FORWARD of
    /// the ankle, and a rotation about +X carries a part's far end back and
    /// down — so positive is plantarflexion, the toe pointing down and back
    /// at the end of the drive, and negative is dorsiflexion, the toe
    /// pulled up as the leg reaches out to land.
    ///
    /// They are deliberately asymmetric. A runner's ankle goes a long way
    /// into the push and only a little the other way; equal ends read as a
    /// flipper.
    const ANKLE_PLANTAR: f32 = 0.55;
    const ANKLE_DORSI: f32 = 0.22;
    /// A body in the air has nothing to push against, so the feet fall into
    /// a soft point rather than staying flexed — the same reason
    /// `DIVE_KNEE` exists.
    const DIVE_ANKLE: f32 = 0.34;
    /// How much of a hand's rest angle is this particular player's, so
    /// twenty-two pairs of hands are not all cocked identically.
    const WRIST_REST: f32 = 0.12;
    /// **How far a finger folds through a full fist**, at the knuckle and at
    /// the joint past it.
    ///
    /// Between them they come to 170°, which is a finger whose tip is back
    /// against the base it grew from. Split unevenly on purpose: the second
    /// joint goes further than the first, which is why a real fist is round
    /// at the knuckles rather than square.
    const GRIP_FINGER: f32 = 1.42;
    const GRIP_KNUCKLE: f32 = 1.54;
    /// …and how much of that each of the four takes, index outward.
    ///
    /// A relaxed hand is not four fingers at one angle. The little finger
    /// curls furthest and the index least, and the ramp between them is
    /// most of what makes a hand hanging by a man's side read as flesh
    /// rather than as a rake.
    const FINGER_CURL: [f32; 4] = [0.86, 1.0, 1.06, 1.14];
    /// How much wider than its rest splay a fanned finger goes.
    ///
    /// A keeper spreading his hands at a shot is trying to occupy area, and
    /// the whole hand opens to do it — which is a gesture nothing else in
    /// football makes, and the one that says *goalkeeper* at any distance
    /// the gloves are visible at all.
    const FAN_WIDE: f32 = 1.35;
    /// The thumb: how far it stands off the hand at rest, how much further
    /// it goes when the hand fans, and how far across the palm it comes as
    /// the hand closes.
    const THUMB_OUT: f32 = 0.95;
    const THUMB_FAN: f32 = 0.30;
    const THUMB_IN: f32 = 0.66;
    /// …and its own fold, which is shallower than a finger's — a thumb
    /// closes across a fist rather than into it.
    const THUMB_REST: f32 = -0.30;
    const THUMB_CURL: f32 = 1.05;
    /// **How closed a hand is with nothing else going on**, and how much
    /// more it closes at a run.
    ///
    /// Nobody's hand is flat. A footballer at rest holds his fingers half
    /// curled and a running one closes them further — the flat open palm
    /// this rig used to draw everywhere is a pose a person adopts to show
    /// you something.
    const HAND_REST: f32 = 0.30;
    const HAND_RUNNING: f32 = 0.46;
    /// The hand a keeper SETS with: open, and barely bent at all.
    const HAND_READY: f32 = 0.07;
    /// Behind a ball he is catching — nearly as open, because the point is
    /// still to be big — and closed round one he has.
    const HAND_SAVING: f32 = 0.12;
    const HAND_HOLDING: f32 = 0.58;
    /// And the fist. A parry is a punch: there is no version of it with the
    /// fingers out, and a splayed hand meeting a ball at thirty metres a
    /// second is how a keeper breaks them.
    const HAND_FIST: f32 = 0.97;
    /// A hand taking a man's weight on the turf is not flat either — it is
    /// on the heel of the palm with the fingers loose.
    const HAND_GRASSED: f32 = 0.40;
    /// Curled over the crest of a hip, and laid flat on a knee.
    const HAND_ON_HIP: f32 = 0.48;
    const HAND_FLAT: f32 = 0.20;
    /// The kick, as the three angles the kicking leg passes through: the top
    /// of the backswing, the instant of contact, and the end of the follow
    /// through.
    ///
    /// Hip first — positive carries the thigh BACK, so the backswing is
    /// positive and everything after it is negative. The leg finishes higher
    /// than it was at contact because a struck ball is hit through, not at:
    /// the boot keeps going and takes the hip with it.
    const KICK_HIP: (f32, f32, f32) = (0.85, -0.72, -1.30);
    /// And the knee, which folds hard on the way back, snaps straight through
    /// the ball, and stays nearly straight into the follow through. The whip
    /// of that fold is most of where a kick's power reads from.
    const KICK_KNEE: (f32, f32, f32) = (1.70, 0.06, 0.34);
    /// What the standing leg does: bends to take the whole body's weight as
    /// the other one comes through, and pushes back up out of it.
    const PLANT_KNEE: f32 = 0.58;
    /// And how far the whole figure sinks over it, in metres — the same
    /// bookkeeping as [`Joint::SET_DROP`]. A bent standing leg is shorter than
    /// a straight one, and without paying for it he kicks the ball while
    /// hovering six centimetres above the pitch.
    const KICK_DROP: f32 = 0.058;
    /// How much of each end of the swing is given over to blending into and
    /// out of the run cycle, as a fraction of it.
    const KICK_BLEND: f32 = 0.20;
    /// How far the hips and then the chest rotate through the ball. The hips
    /// lead and the shoulders follow, which is the sequence that makes a kick
    /// look like it came from the ground up rather than from the knee.
    const KICK_TWIST: f32 = 0.44;
    /// How far ahead of the swing the hips run, and how much more of the same
    /// turn the shoulders take. Together they are the separation between the
    /// two: coiled the other way at the top of the backswing, hips through
    /// first at contact.
    const KICK_HIP_LEAD: f32 = 0.35;
    const KICK_CHEST: f32 = 0.90;
    /// And how far he leans away from the kicking side, which is what keeps a
    /// man upright while one leg is over his own head.
    const KICK_ROLL: f32 = 0.26;
    /// The counter-swing, as the same three keys again: the arm OPPOSITE the
    /// kicking leg comes forward and up through the ball, and the one on the
    /// kicking side goes back. Without it the whole action reads as a
    /// mechanism hinged at one joint.
    const KICK_COUNTER: (f32, f32, f32) = (-0.35, -1.25, -0.95);
    const KICK_TRAIL: (f32, f32, f32) = (0.20, 0.60, 0.40);
    /// And how far across his own chest the counter-arm comes.
    const KICK_ARM_SPREAD: f32 = 0.55;
    /// A header, as the three keys the spine passes through: arched away from
    /// the ball, square at the moment of contact, and driven through it.
    ///
    /// A header is generated in the trunk and finished with the neck — the
    /// chest opens, the head is snapped through, and the arms go out to hold
    /// the space rather than to counter-swing. Sequenced the same way a kick
    /// is, with the neck arriving after the chest: it is the same
    /// ground-upward chain, just one joint higher.
    const NOD_CHEST: (f32, f32, f32) = (-0.34, 0.14, 0.40);
    const NOD_NECK: (f32, f32, f32) = (-0.30, 0.18, 0.30);
    /// And what the arms do about it: out and slightly back, elbows soft. A
    /// player attacking a cross has both of them wide, which is half of what
    /// makes the action legible from a hundred metres away.
    const NOD_SHOULDER: f32 = -0.30;
    const NOD_SPREAD: f32 = 0.46;
    const NOD_ELBOW: f32 = -0.62;
    /// A throw-in: both arms through one sweep past a half turn, for the same
    /// reason the keeper's throw is written that way — these interpolate as
    /// scalars, and going the short way round between "behind the head" and
    /// "out in front" takes the arms down past the hips.
    const TOSS_SHOULDER: (f32, f32, f32) = (2.55, 4.05, 4.70);
    const TOSS_ELBOW: (f32, f32, f32) = (-1.85, -0.30, -0.12);
    /// Hands together over the head rather than out at the sides: a throw-in
    /// is taken with both of them on the ball.
    const TOSS_SPREAD: f32 = -0.10;
    /// The trunk arches back over the wind-up and whips forward through the
    /// release, and the knees give a little under it.
    const TOSS_CHEST: (f32, f32, f32) = (-0.40, 0.16, 0.44);
    const TOSS_KNEE: (f32, f32, f32) = (0.34, 0.20, 0.10);
    /// A keeper's throw: the same three keys as a kick, sent to his shoulder
    /// instead of his hip — cocked behind the ear, over the top, and down
    /// across the follow through.
    ///
    /// Written as one INCREASING sweep past a half turn rather than as the
    /// angles it would be natural to write, because these are interpolated as
    /// scalars: cocking at +2.35 and releasing at −1.98 is the same pair of
    /// poses but takes the arm down past his hip to get between them, which is
    /// an underarm bowl.
    const THROW_SHOULDER: (f32, f32, f32) = (2.35, 4.30, 5.15);
    const THROW_ELBOW: (f32, f32, f32) = (-1.60, -0.25, -0.15);
    /// Driving off the mark and pulling up short: how far the chest goes over
    /// the toes at full acceleration, and how far it sits back at a full
    /// stop. Braking is the bigger angle — stopping is more violent than
    /// starting, and it is the one every footballer does at the end of every
    /// run.
    const DRIVE_LEAN: (f32, f32) = (0.34, -0.30);
    /// And what his legs do about it: feet driving out behind under
    /// acceleration, planted out in front under the brakes.
    const DRIVE_HIP: f32 = 0.16;
    /// The man on the ball: down over it, arms out from his sides for
    /// balance, strides shortened. All small — he is a footballer running,
    /// not a man in a crouch — but between them they are what makes the
    /// player with the ball findable in a crowd.
    const CARRY_LEAN: f32 = 0.16;
    const CARRY_KNEE: f32 = 0.16;
    const CARRY_DROP: f32 = 0.022;
    const CARRY_SPREAD: f32 = 0.16;
    const CARRY_ELBOW: f32 = -0.30;

    fn new(owner: Entity, limb: Limb, side: f32, origin: Vec3) -> Self {
        Joint {
            owner,
            limb,
            side,
            origin,
        }
    }

    /// Where the joint sits this frame.
    ///
    /// A runner rides highest with their legs underneath them and drops as the
    /// stride opens out, twice per cycle. The bob only ever lifts — taking the
    /// body below its standing height would put the boots through the turf —
    /// and pelvis, hips and torso all take the same offset, so the whole
    /// footballer rises as one.
    pub fn place(&self, gait: Gait) -> Vec3 {
        match self.limb {
            // One hip can sit lower than the other, and has to: see
            // [`Joint::hip_list`]. Only the leg joints take it — the pelvis
            // and the chest take the ROTATION it amounts to instead, in
            // `pose`, because they are drawn about their own axis rather
            // than hung off a socket.
            Limb::Hip => self.place_body(gait) + Vec3::Y * Self::hip_list(gait, self.side),
            Limb::Pelvis | Limb::Torso => self.place_body(gait),
            _ => self.origin,
        }
    }

    /// Where the hips ride this frame, before anything that is about one leg
    /// rather than about the body.
    fn place_body(&self, gait: Gait) -> Vec3 {
        match self.limb {
            Limb::Pelvis | Limb::Torso | Limb::Hip => {
                // ⚠ **[`Joint::stepping`] and not [`Joint::cycling`].** The
                // bob only ever rises — it has no trough — which is a claim
                // about a SAGITTAL cycle, where the body is highest at
                // mid-stance over a straight leg and there is a flight phase
                // to pay for it. A side-step has neither, and its vertical
                // motion is already drawn where it actually comes from: the
                // splay, the pelvic list and the knee. Given the lateral
                // cycle as well, a shuffling keeper floated 2.7 cm and BOTH
                // his feet left the grass — the fault
                // `a_side_step_takes_one_foot_at_a_time` exists to catch.
                // ⚠ **It DIPS between steps; it does not rise at mid-stance.**
                //
                // Nothing in this rig works out how long the stance leg is
                // and settles the body onto it — the lateral gait does
                // ([`Joint::stance_drop`]) and the sagittal one uses a tuned
                // constant instead. So the bob is a FREE LIFT of the whole
                // figure, and at mid-stance, where it used to peak, the leg
                // is straight and vertical and has nothing left to give: the
                // foot simply left the turf by the whole of it. Raising
                // `BOB` from 0.06 to a realistic figure put a jogging squad
                // 1.8 cm above the grass, which
                // `a_runner_puts_his_foot_on_the_grass` now catches.
                //
                // Written as a dip the peak-to-peak is the same, mid-stance
                // is untouched — so the planted foot is exactly where the leg
                // puts it — and the body sinks at the crossover, which is
                // where both knees are bent and it genuinely is lower. That
                // is also the right phase for a WALK, where the centre of
                // mass vaults over a straight leg and drops through double
                // support, and a walk is where contact matters: there is no
                // flight phase to hide a foot in.
                let bob = -Self::BOB
                    * Self::stepping(gait)
                    * gait.spring
                    * (1.0 - (gait.phase * 2.0).cos())
                    * 0.5;
                // Breathing, for a player who is not running. Fades out as he
                // does, where the stride bob takes over.
                let breathe =
                    Self::BREATHE * (1.0 - Self::cycling(gait)) * (0.5 + 0.5 * gait.idle.sin());
                // And the settle onto bent knees, which is a real loss of
                // height rather than a pose: see [`Joint::SET_DROP`]. The man
                // on the ball sinks over it the same way, by less.
                //
                // **The sockets turn with the legs.** Hips are a pair of
                // joints 176 mm apart, and once the legs have opened onto
                // the run (see [`Gait::open`]) that line is no longer square
                // across him — at a full crossover it is fore-and-aft along
                // the way he is going, which is what stops two yawed legs
                // swinging through each other. Identity for the pelvis and
                // the torso, whose origins are on the centreline anyway.
                Quat::from_rotation_y(Self::opened(gait)) * self.origin
                    + Vec3::Y
                        * (bob + breathe
                            - Self::SET_DROP * Self::crouched(gait)
                            - Self::CARRY_DROP * gait.carrying
                            - Self::SLUMP_DROP * gait.despair
                            - Self::DOUBLED_DROP * gait.doubled_over
                            // A wide base is a low one, and a keeper going
                            // down to a ball at his boots loses most of a
                            // hand's width of height doing it. Both are real
                            // losses rather than poses, so they are paid for
                            // here — the same reason `SET_DROP` is.
                            - Self::stance_drop(gait)
                            - Self::SAVE_DROP * gait.save * Self::stooping(gait)
                            // …and the height a keeper going UP to one
                            // actually gains, which is the other end of the
                            // same axis. See [`Joint::SAVE_RISE_HIP`].
                            + Self::SAVE_RISE * Self::reaching(gait)
                            - Self::KICK_DROP * gait.power * Self::taper(gait.swing))
            }
            _ => self.origin,
        }
    }

    /// The joint's rotation at this point in the stride.
    ///
    /// Positive rotation about X carries the top of a part forward and its
    /// far end back, so a leg swings forward on a negative angle and a knee
    /// folds on a positive one. Every amplitude is interpolated out of
    /// standing still, which is what lets a player come to a stop without an
    /// idle animation to blend into.
    pub fn pose(&self, gait: Gait) -> Quat {
        // The left leg is half a cycle behind the right.
        let leg = gait.phase + if self.side < 0.0 { PI } else { 0.0 };
        let swing = leg.sin();

        // Weight going from one foot to the other, and back. Half the idle
        // rate, because a shift is a whole cycle where a breath is half of
        // one. Fades out the moment he starts running.
        let standing = 1.0 - Self::cycling(gait);
        let weight = (gait.idle * 0.5).sin() * standing;

        // Which of a pair this limb is, relative to the way he went: +1 on
        // the leading side of the dive, −1 on the trailing side, 0 for a man
        // going straight forward — and 0 for everybody who is not diving,
        // since `lead` is only ever set as a keeper leaves the ground.
        let leading = (self.side * gait.lead).clamp(-1.0, 1.0);
        // How much of the asymmetry this limb takes. Scaled by how sideways
        // the dive was, so a smother at a striker's feet stays square.
        // Squared off again once he has the ball: both arms come onto it.
        let trailing = (1.0 - leading) * 0.5 * gait.lead.abs() * (1.0 - gait.claimed);
        // The arms on the grass. Held apart from `grounded` because what they
        // do down there depends entirely on whether he came up with the ball:
        // with it, he curls around it and the cradle already has them; without
        // it, they come in under him and take his weight.
        //
        // …but only the free one. He came down on the arm he dived with, and
        // that shoulder IS the ground once the body is genuinely flat — so
        // folding it in front of his chest like the other one buried the
        // glove ten centimetres into the turf. The lead arm stays out along
        // the grass where the dive left it; the top arm comes across him.
        // Square on — a smother at a striker's feet, where he lands on his
        // chest and both shoulders are clear — both still come in.
        let landed = gait.grounded * (1.0 - gait.carry);
        let bracing = landed * (1.0 - leading.max(0.0));
        // …and the complement: the arm that is ON the grass. Exactly one of
        // the two is ever the underneath one, and for a square landing this
        // is zero and `bracing` has both.
        let grassed = landed * leading.max(0.0);
        // How much of the kick this limb takes, 0..1. Peaks at contact and
        // eases away at both ends, so the swing arrives out of the run cycle
        // and returns to it rather than being cut in and out.
        // Full authority across the middle of the swing and a quick blend to
        // the run cycle at each end, so the kick arrives and leaves without a
        // pop. NOT a taper across the whole swing: the keys below put the
        // backswing at −1 and the follow through at +1, so fading the
        // authority out toward them cancels the two halves of a kick that are
        // not the moment of contact — which is to say, all of it.
        let taper = Self::taper(gait.swing);
        let kicking = gait.power * taper;
        let throwing = gait.throwing * taper;
        // The two strikes a footballer makes that are not kicks. Same phase,
        // same taper, different limbs — which is the whole reason `swing` is
        // one number and the amplitudes are several.
        let heading = gait.header * taper;
        let tossing = gait.throw_in * taper;
        // +1 if this is the kicking side of the body, −1 if it is the standing
        // side. Zero for everybody not kicking, which leaves both halves equal
        // and every term below at rest.
        let striking = self.side * gait.foot;
        // Which of the four ways he took it, split rather than blended — see
        // [`Gait::hands_to_head`]. All four are zero for every player for all
        // but the few seconds after a goal, so every layer they drive
        // short-circuits inside [`Self::held`]. `limp` is what is left over,
        // which makes "arms hanging" the default reaction rather than a
        // fourth thing to pick.
        let on_head = gait.hands_to_head;
        let on_hips = gait.hands_on_hips;
        let doubled = gait.doubled_over;
        let limp = (gait.despair - on_head - on_hips - doubled).clamp(0.0, 1.0);
        // …and the two a keeper does with nothing else going on. Not moods:
        // these run on the match clock, and the only thing they are gated on
        // is having nothing better to do with his hands.
        let urging = gait.urging;
        // Only one arm points, and which one is the sign of the signal.
        let pointing = (self.side * gait.pointing).clamp(0.0, 1.0);

        match self.limb {
            // Hips counter-rotate against the shoulders — the thing that
            // makes a run read as a person rather than a mechanism — and
            // carry the weight shift when he is standing.
            Limb::Pelvis => {
                // The hips lead a kick and the shoulders follow it: the pelvis
                // is already opening toward the target while the boot is still
                // travelling, which is the sequence that makes a strike look
                // as though it came from the ground up rather than from the
                // knee. The phase offset is what puts it ahead — at contact
                // the hips have turned through and the chest has not.
                //
                // Signed off the kicking foot, so a left-footer turns the
                // other way.
                let opening = Self::KICK_TWIST
                    * gait.foot
                    * (gait.swing + Self::KICK_HIP_LEAD).clamp(-1.0, 1.0)
                    * kicking;
                Quat::from_rotation_y(
                    Self::HIP_TWIST * Self::cycling(gait) * gait.spring * gait.phase.sin() * gait.course.y
                        - opening
                        // Opened toward the way he is going. A shuffle is not
                        // square to the last millimetre — the hips lead.
                        + Self::SHUFFLE_TWIST * gait.course.x * Self::sidling(gait)
                        // …and the whole pelvis turns with the legs, because
                        // it IS the legs: the seat of the shorts belongs to
                        // the hip sockets, and [`Joint::place_body`] has
                        // already swung those round. See [`Gait::open`].
                        + Self::opened(gait),
                ) * Quat::from_rotation_z(
                    Self::WEIGHT_SHIFT * weight
                        // …and listed exactly as far as the two hips are
                        // apart in height, so the seat of the shorts sits on
                        // the legs rather than across them.
                        + Self::pelvic_roll(gait),
                )
            }
            Limb::Torso => {
                // Rock through the stride, a quarter cycle off the twist so
                // the shoulder drops as the opposite leg drives; the weight
                // shift when standing; and the bank into a turn.
                //
                // Bank sign: heading increases turning toward +X, and a
                // positive Z rotation carries the head toward −X, so leaning
                // INTO a left turn is the negative of the turn signal. If it
                // ever looks like a motorbike falling the wrong way, this is
                // the sign to flip.
                let roll = Self::rocking(gait) + Self::WEIGHT_SHIFT * 0.8 * weight
                    - Self::BANK * gait.turn;
                // A keeper with the ball stands up out of all of it. Two
                // reasons, and the second is the load-bearing one: he does
                // straighten up, and the arm angles above are measured off an
                // upright chest — leave the lean and the rock in and the ball
                // drifts a couple of centimetres out of his gloves every
                // stride.
                //
                // A diving keeper comes out of it for the same reason and one
                // more: the lean is the forward pitch of a man driving off the
                // ground, and there is no ground under him.
                let settle = Self::upright(gait);
                let running = Quat::from_rotation_x(Self::leaning(gait))
                    * Quat::from_rotation_y(
                        -Self::CHEST_TWIST
                            * Self::cycling(gait)
                            * gait.spring
                            * gait.phase.sin()
                            * gait.course.y
                            * settle
                            // …and a share of the opening, so that eighty
                            // degrees between a man's hips and his shoulders
                            // is not asked of his lumbar spine alone. The
                            // hips still LEAD it — that is what opening up
                            // is, and the order from where he was pointed to
                            // where he is going runs chest, then hips, then
                            // travel. See [`Gait::open`].
                            + Self::OPEN_CHEST * Self::opened(gait),
                    )
                    * Quat::from_rotation_z(roll * settle);
                // Then, in order: the set's forward lean over bent knees, the
                // arch and the turn onto the ball in flight, and the curl
                // over the landing. Each one is scaled by its own signal, so
                // for twenty-one players out of twenty-two every term after
                // the first is identity.
                running
                    * Quat::from_rotation_x(
                        Self::SET_LEAN * Self::crouched(gait) * (1.0 - gait.save * gait.save_aim.y.max(0.0)),
                    )
                    * Quat::from_rotation_x(
                        Self::DIVE_ARCH * gait.stretch * (1.0 - gait.grounded),
                    )
                    * Quat::from_rotation_y(
                        Self::DIVE_TWIST * gait.lead * gait.stretch * (1.0 - 0.5 * gait.grounded),
                    )
                    * Quat::from_rotation_x(
                        Self::DOWN_CURL * gait.grounded
                            // Curled tighter by having conceded, and still
                            // folded as he comes up off it.
                            + Self::BEATEN_CURL * gait.beaten
                            + Self::RISE_CURL * Self::kneeling(gait),
                    )
                    // An outfielder's leap is a much smaller version of the
                    // same arch, and none of the turn: he is going up at a
                    // ball, not across a goal.
                    * Quat::from_rotation_x(Self::DIVE_ARCH * 0.45 * gait.jump)
                    // Over his toes off the mark, back on his heels under the
                    // brakes — and lower over the ball when he has it.
                    * Quat::from_rotation_x(
                        Self::DRIVE_LEAN.0 * gait.drive.max(0.0)
                            + Self::DRIVE_LEAN.1 * (-gait.drive).max(0.0)
                            + Self::CARRY_LEAN * gait.carrying,
                    )
                    // Travelling ACROSS himself and travelling BACKWARDS —
                    // the two thirds of a goalkeeper's movement that a
                    // forward run cycle has nothing to say about. He leans
                    // into a side-step the way anybody changing direction
                    // does, and going backwards he keeps his chest up, where
                    // the run's own forward lean would have him over his own
                    // heels. Both are identity for a man running forwards,
                    // which is everybody else on the pitch.
                    * Quat::from_rotation_z(
                        -Self::SHUFFLE_LEAN * Self::afoot(gait) * gait.course.x
                            // The chest takes a QUARTER of the pelvis's list
                            // — the spine absorbs the rest — and rides the
                            // weight shift on top of it, a beat behind,
                            // which is where the whole rocking quality of a
                            // side-step comes from.
                            + Self::pelvic_roll(gait) * Self::SPINE_ABSORBS
                            - Self::SHUFFLE_LIST * Self::carried(gait) * Self::sidling(gait),
                    )
                    * Quat::from_rotation_y(
                        -Self::SHUFFLE_TWIST * 0.6 * gait.course.x * Self::sidling(gait),
                    )
                    * Quat::from_rotation_x(Self::BACKPEDAL_LEAN * Self::backing(gait))
                    // And the save he makes on his feet: down over a ball at
                    // his boots, across at one beside him.
                    * Quat::from_rotation_x(
                        Self::SAVE_STOOP * gait.save * Self::stooping(gait),
                    )
                    * Quat::from_rotation_z(-Self::SAVE_LEAN * gait.save * gait.save_aim.x)
                    * Quat::from_rotation_x(
                        Self::SAVE_ARCH * gait.save * gait.save_aim.y.max(0.0),
                    )
                    // The chest coils further than the hips going back and
                    // arrives after them coming through — the separation
                    // between the two is where a kick's whip comes from — and
                    // he leans away from the leg that is over his own head.
                    * Quat::from_rotation_y(
                        -Self::KICK_TWIST
                            * Self::KICK_CHEST
                            * gait.foot
                            * (gait.swing - Self::KICK_HIP_LEAD).clamp(-1.0, 1.0)
                            * kicking,
                    )
                    * Quat::from_rotation_z(Self::KICK_ROLL * gait.foot * kicking)
                    // And the two strikes that come out of the trunk rather
                    // than out of a leg. Both are identity for anybody not
                    // making them, which is twenty-two players out of
                    // twenty-two for most of a match.
                    * Quat::from_rotation_x(Self::through(Self::NOD_CHEST, gait.swing) * heading)
                    * Quat::from_rotation_x(Self::through(Self::TOSS_CHEST, gait.swing) * tossing)
                    // And how he took the goal. Composed on the end rather
                    // than blended in like the arms: this is a fold at the
                    // waist ON TOP of whatever else the trunk is doing, and
                    // the legs hang off the carriage rather than off the
                    // torso, so it bends the man without moving his feet.
                    // The two folds are the SAME fold at two depths, so the
                    // deeper one replaces the shallower rather than adding
                    // to it: composed, a man bent double over his knees was
                    // folded through seventy-seven degrees and looking
                    // backwards between his own legs.
                    * Quat::from_rotation_x(
                        Self::SLUMP_STOOP * (gait.despair - gait.doubled_over).max(0.0)
                            + Self::DOUBLED_STOOP * gait.doubled_over
                            + Self::CHEER_ARCH * gait.elation,
                    )
            }
            // He watches the ball. The head hangs off the torso, so this yaw
            // is already relative to his chest — turning it is the single
            // cheapest thing in the whole rig that reads as attention, and
            // without it twenty-two players stare rigidly down their own
            // running line all match.
            //
            // Still kept level in pitch against his own forward lean — a
            // runner leans from the hips and looks up the pitch, not at his
            // own boots — and then tipped onto the ball, which for a keeper
            // is most of what a save is: he does not take his eyes off it,
            // and up to now he never once looked up at one.
            //
            // The twist is subtracted back out because the head hangs off the
            // torso: without that, rotating the chest onto the ball in a dive
            // swings the face straight past it.
            Limb::Head => {
                // A runner's head stays level while his chest rocks under it —
                // the eyes are on the ball and the neck does the work. So the
                // torso's own roll is subtracted back out, the same way the
                // dive's twist is: without it the whole head tips side to side
                // once per stride, which is the one thing nobody's does.
                let steady = -Self::rocking(gait);
                Quat::from_rotation_y(
                    gait.look
                        - Self::DIVE_TWIST * gait.lead * gait.stretch * (1.0 - 0.5 * gait.grounded),
                ) * Quat::from_rotation_x(-Self::leaning(gait) * Self::HEAD_LEVEL - gait.look_pitch)
                    * Quat::from_rotation_z(steady)
                    // The neck through a header, arriving after the chest.
                    // Heading a ball is the one thing a footballer does with
                    // his head that is not looking at something.
                    * Quat::from_rotation_x(Self::through(Self::NOD_NECK, gait.swing) * heading)
                    // Chin on the chest, or up at the sky. On top of the
                    // trunk's own fold above, which the head inherits by
                    // hanging off it — together they come to about forty
                    // degrees, which is a man looking at the grass.
                    * Quat::from_rotation_x(
                        Self::SLUMP_HEAD_DOWN * (gait.despair - gait.doubled_over).max(0.0)
                            // …and OUT of the fold when he is bent over his
                            // own knees: the chest has already carried the
                            // head through a right angle, and leaving the
                            // chin on it too puts him looking at his own
                            // shins.
                            + Self::DOUBLED_HEAD * gait.doubled_over
                            + Self::CHEER_HEAD_UP * gait.elation
                            // Face into the turf, and then up again before
                            // the rest of him moves — unless he has just
                            // conceded, and then it stays down. A man looks
                            // where he is going before he goes there, and a
                            // beaten keeper is not going anywhere he wants
                            // to be.
                            + Self::BEATEN_HEAD * gait.beaten
                            + Self::RISE_HEAD * Self::kneeling(gait) * (1.0 - gait.beaten),
                    )
            }
            Limb::Shoulder => {
                // Arms swing against the leg on the same side, and are carried
                // wider the harder the player is running — and wider again, or
                // tighter, depending on the man.
                // — and wider again with the ball at his feet, which is a
                // man balancing over it rather than running freely.
                let carriage = 0.15
                    + 0.07 * gait.run
                    + 0.055 * gait.signature
                    + Self::CARRY_SPREAD * gait.carrying
                    // Out from his sides across a side-step, which is a man
                    // balancing rather than a man running — and ANSWERING the
                    // step rather than held at a constant width, which is a
                    // man holding a pose while his legs work.
                    + 0.22 * Self::sidling(gait)
                    + Self::SHUFFLE_ARM
                        * Self::sidling(gait)
                        * self.side
                        * Self::carried(gait);
                // Nobody's two arms do the same thing. Tied to the signature
                // so it is this player's asymmetry, the same one every time.
                let asymmetry = 1.0 + 0.11 * gait.signature * self.side;
                // Standing, they drift instead of locking. Offset by side so
                // the two do not move as a pair.
                let drift = 0.055 * standing * (gait.idle + self.side).sin();
                // The counter-swing: the arm OPPOSITE the kicking leg comes
                // forward and up through the ball while the one on the kicking
                // side goes back. Every rotation a footballer puts into a ball
                // has to be paid for somewhere, and this is where — without it
                // the whole action reads as a mechanism hinged at one joint.
                let counter = Self::through(
                    if striking < 0.0 {
                        Self::KICK_COUNTER
                    } else {
                        Self::KICK_TRAIL
                    },
                    gait.swing,
                ) * kicking
                    * striking.abs();
                // Signed with the stride, like the leg it answers: arms pump
                // against a run and hang wide across a shuffle, where there
                // is no stride for them to counter.
                let arm = Self::arming(gait) * swing * gait.course.y * asymmetry + drift + counter;
                // And the counter-arm alone comes across his chest, which is
                // the other half of paying for the turn.
                let across = self.side * Self::KICK_ARM_SPREAD * kicking * (-striking).max(0.0);
                let swinging = Quat::from_rotation_z(self.side * carriage - across)
                    * Quat::from_rotation_x(arm);
                // How he took the goal, layered straight onto the run cycle
                // and UNDER everything else — a mood is a modification of
                // standing about, and anything he is actually doing (a save,
                // a throw, a kick) has to beat it.
                let slumped = Self::held(
                    swinging,
                    Quat::from_rotation_z(self.side * Self::SLUMP_LIMP_SPREAD)
                        * Quat::from_rotation_x(Self::SLUMP_LIMP_SHOULDER),
                    limp,
                );
                // Sign NEGATED for the raised arms, the same inversion
                // [`Self::REACH_SPREAD`] documents: a roll about +Z carries a
                // hanging arm outward and a raised one inward, so signing
                // these like the pose above crosses his own wrists over his
                // head instead of putting the elbows out.
                let slumped = Self::held(
                    slumped,
                    Quat::from_rotation_z(-self.side * Self::SLUMP_HEAD_SPREAD)
                        * Quat::from_rotation_x(Self::SLUMP_HEAD_SHOULDER),
                    on_head,
                );
                // Hands on the hips: the arm barely moves and the YAW does
                // all the work, composed innermost so it rolls the arm in
                // its socket rather than swinging it. See
                // [`Self::HIPS_TURN`] — this is the pose the rig could not
                // reach until the standing save gave the shoulder a yaw.
                let slumped = Self::held(
                    slumped,
                    Quat::from_rotation_z(self.side * Self::HIPS_SPREAD)
                        * Quat::from_rotation_x(Self::HIPS_SHOULDER)
                        * Quat::from_rotation_y(self.side * Self::HIPS_TURN),
                    on_hips,
                );
                // …and bent double, where the arms hang vertically out of a
                // chest that is horizontal, which relative to that chest is
                // a positive pitch.
                let slumped = Self::held(
                    slumped,
                    Quat::from_rotation_z(self.side * Self::DOUBLED_SPREAD)
                        * Quat::from_rotation_x(Self::DOUBLED_SHOULDER),
                    doubled,
                );
                // The two a keeper does when the ball is at the other end.
                // Above the slump, below everything he might actually be
                // doing — which is right, because they are what he does when
                // there is nothing to do.
                let barking = Self::held(
                    slumped,
                    Quat::from_rotation_y(
                        -self.side * (Self::URGE_YAW - Self::CLAP_OPEN * Self::clap(gait)),
                    ) * Quat::from_rotation_x(Self::URGE_SHOULDER),
                    urging,
                );
                let organising = Self::held(
                    barking,
                    Quat::from_rotation_y(self.side * Self::POINT_YAW)
                        * Quat::from_rotation_x(Self::POINT_SHOULDER),
                    pointing,
                );
                let cheering = Self::held(
                    organising,
                    Quat::from_rotation_z(-self.side * Self::CHEER_SPREAD)
                        * Quat::from_rotation_x(Self::CHEER_SHOULDER),
                    gait.elation,
                );
                // …and no two keepers set alike. The same signature that
                // carries how wide a man holds his arms running: one stands
                // with his gloves high and narrow, another low and wide, and
                // two goalkeepers in identical postures is the same lockstep
                // the run cycle was fixed for.
                let ready = Self::held(
                    cheering,
                    Quat::from_rotation_z(
                        self.side * Self::SET_SPREAD * (1.0 + 0.22 * gait.signature),
                    ) * Quat::from_rotation_x(Self::SET_SHOULDER * (1.0 - 0.30 * gait.signature)),
                    Self::armed(gait),
                );
                // **And a keeper with his gloves up is not welded to them.**
                //
                // The same fault as the legs, in the same shape and one line
                // apart: `held` SLERPS the shoulder onto a fixed angle, so
                // whatever weight the set is given is that much of the arm
                // swing gone. Measured, a glove travelled 1.8 cm through a
                // walk with his hands up against 20.8 cm with them down —
                // and a keeper is at his most watched with his hands up. A
                // ready posture is a CARRIAGE, so the cycle goes back on top
                // of the hold rather than being erased by it.
                //
                // Two motions, because a keeper has two gaits. The pump
                // ALTERNATES — it is signed by `swing`, which is already
                // half a cycle apart on the two arms — and is what answers a
                // walk. The sway moves the pair TOGETHER, which is what a
                // side-step does with them: no `self.side` on it, because
                // the two shoulder frames differ by a roll and a yaw of the
                // same sign carries both arms the same way. ⚠ A yaw and not
                // a roll, the trap [`Joint::SAVE_ACROSS`] documents: with
                // the forearms up in front of him a roll about the arm's own
                // axis moves a glove barely at all.
                let poise = Self::armed(gait);
                let ready = ready
                    * Quat::from_rotation_x(
                        Self::READY_PUMP * poise * Self::cycling(gait) * swing * asymmetry,
                    )
                    * Quat::from_rotation_y(
                        Self::READY_SWAY * poise * Self::sidling(gait) * Self::carried(gait),
                    );
                // **The save he makes on his feet**, and the most repeated
                // thing a goalkeeper does. Both arms go to the ball: the
                // PITCH of the shoulder is how high it is and the YAW is how
                // far across him, so one pair of numbers covers a ball at his
                // boots, one at his chest and one over his shoulder.
                //
                // **The near arm leads and the far one barely goes.** Two arms
                // travelling the same distance across him is the superman that
                // [`Gait::lead`] exists to prevent in a dive, and it reads no
                // better standing up — rendered, it is a man pointing at
                // something. The far shoulder stays in front of his chest,
                // which is both what a keeper does and where the second glove
                // is any use.
                //
                // Above the set, because this is what the set turns into, and
                // below the leap, the hold and the reach, all three of which
                // are a keeper who is no longer on his feet.
                let leading_hand = 0.55 + 0.45 * (self.side * gait.save_aim.x).clamp(-1.0, 1.0);
                let saving = Self::held(
                    ready,
                    Quat::from_rotation_y(
                        Self::SAVE_ACROSS * gait.save_aim.x * leading_hand
                            + self.side * Self::PARRY_SPREAD * gait.parry,
                    ) * Quat::from_rotation_x(
                        Self::SAVE_LOW
                            + (Self::SAVE_HIGH - Self::SAVE_LOW)
                                * (gait.save_aim.y * 0.5 + 0.5).clamp(0.0, 1.0),
                    ),
                    gait.save,
                );
                let leaping = Self::held(
                    saving,
                    Quat::from_rotation_z(self.side * Self::JUMP_SPREAD)
                        * Quat::from_rotation_x(Self::JUMP_SHOULDER),
                    gait.jump,
                );
                let holding = Self::held(
                    leaping,
                    Quat::from_rotation_z(self.side * Self::CRADLE_SPREAD)
                        * Quat::from_rotation_x(Self::CRADLE_SHOULDER),
                    gait.carry,
                );
                // The reach itself, opening out across the flight rather than
                // arriving whole: gathered at take-off, past the head by the
                // apex — and this arm only gets all the way there if it is
                // the leading one.
                let shoulder = Self::LAUNCH_SHOULDER
                    + (Self::REACH_SHOULDER - Self::LAUNCH_SHOULDER) * gait.stretch
                    + Self::TRAIL_SHOULDER * trailing * gait.stretch;
                let spread = (Self::REACH_SPREAD + Self::TRAIL_SPREAD * trailing)
                    * (1.0 - gait.claimed)
                    + Self::CLAIM_SPREAD * gait.claimed;
                // Reach after the cradle, so a keeper who takes the ball
                // cleanly at the top of a leap has his arms come down into
                // the hold rather than stay up around a ball he already has.
                let out = Self::held(
                    holding,
                    Quat::from_rotation_z(-self.side * spread) * Quat::from_rotation_x(shoulder),
                    gait.reach,
                );
                // The landing: whatever he was doing up there, the top arm
                // comes in as he hits the grass and the underneath one goes
                // out along it. Sign negated on the second, the same
                // inversion [`Self::REACH_SPREAD`] documents — a roll about
                // +Z carries a hanging arm outward and a raised one inward,
                // and this one is raised.
                let down = Self::held(
                    out,
                    Quat::from_rotation_z(self.side * Self::DOWN_SPREAD)
                        * Quat::from_rotation_x(Self::DOWN_SHOULDER),
                    bracing,
                );
                let down = Self::held(
                    down,
                    Quat::from_rotation_z(self.side * Self::GRASS_SPREAD)
                        * Quat::from_rotation_x(Self::GRASS_SHOULDER),
                    grassed,
                );
                // Face in his hands on the turf: the TOP arm only, since the
                // other one is underneath him and the ground has it.
                let down = Self::held(
                    down,
                    Quat::from_rotation_z(self.side * Self::BEATEN_SPREAD)
                        * Quat::from_rotation_x(Self::BEATEN_SHOULDER),
                    gait.beaten * (1.0 - leading.max(0.0)),
                );
                // …and then both hands onto the grass to push himself off
                // it. Above the landing poses because it is what replaces
                // them: an arm folded across his chest cannot take his
                // weight.
                let down = Self::held(
                    down,
                    Quat::from_rotation_z(self.side * Self::RISE_SPREAD)
                        * Quat::from_rotation_x(-gait.over),
                    gait.propping(),
                );
                // And a keeper's throw, which is the same swing as a kick sent
                // to the arm instead: cocked behind his ear and hurled
                // overarm. Only the throwing side moves.
                let hurled = Self::held(
                    down,
                    Quat::from_rotation_x(Self::through(Self::THROW_SHOULDER, gait.swing)),
                    throwing * striking.max(0.0),
                );
                // Attacking a cross: both arms out, holding the space he is
                // about to jump into.
                let attacking = Self::held(
                    hurled,
                    Quat::from_rotation_z(self.side * Self::NOD_SPREAD)
                        * Quat::from_rotation_x(Self::NOD_SHOULDER),
                    heading,
                );
                // A throw-in, where BOTH arms go over the head together.
                Self::held(
                    attacking,
                    Quat::from_rotation_z(self.side * Self::TOSS_SPREAD)
                        * Quat::from_rotation_x(Self::through(Self::TOSS_SHOULDER, gait.swing)),
                    tossing,
                )
            }
            // Elbow carriage is the most individual thing about a runner:
            // some hold them almost straight, some at a right angle.
            Limb::Elbow => {
                let running = Quat::from_rotation_x(
                    -Self::blend(Self::ELBOW_FLEX, gait.run) * (1.0 + Self::ELBOW_SPREAD * gait.elbows)
                        // **The elbow OPENS behind him and closes in front**,
                        // which is what an arm drive is and is most of how
                        // far a hand actually travels.
                        //
                        // ⚠ This term used to carry the other sign — the
                        // elbow folding TIGHTEST at the back of the swing,
                        // where the upper arm is already behind him. That is
                        // a chicken wing, and it worked against the shoulder:
                        // the hand's path came out a third shorter than the
                        // shoulder amplitude was asking for. `swing > 0` is
                        // the arm going BACK (positive X carries a part's far
                        // end back), and a negative elbow angle is a flexed
                        // one, so opening at the back is a positive term.
                        + Self::ELBOW_DRIVE * gait.run * swing
                        // Elbows come up over a ball he is carrying, and the
                        // counter-arm bends hard through a kick.
                        + Self::CARRY_ELBOW * gait.carrying
                        - 0.55 * kicking * (-striking).max(0.0),
                );
                // Hanging soft, or folded right back onto the crown. Under
                // everything else, as at the shoulder.
                let slumped =
                    Self::held(running, Quat::from_rotation_x(Self::SLUMP_LIMP_ELBOW), limp);
                let slumped = Self::held(
                    slumped,
                    Quat::from_rotation_x(Self::SLUMP_HEAD_ELBOW),
                    on_head,
                );
                let slumped = Self::held(slumped, Quat::from_rotation_x(Self::HIPS_ELBOW), on_hips);
                let slumped =
                    Self::held(slumped, Quat::from_rotation_x(Self::DOUBLED_ELBOW), doubled);
                let slumped = Self::held(slumped, Quat::from_rotation_x(Self::URGE_ELBOW), urging);
                let slumped =
                    Self::held(slumped, Quat::from_rotation_x(Self::POINT_ELBOW), pointing);
                let running = Self::held(
                    slumped,
                    Quat::from_rotation_x(Self::CHEER_ELBOW),
                    gait.elation,
                );
                let ready = Self::held(
                    running,
                    Quat::from_rotation_x(Self::SET_ELBOW * (1.0 + 0.14 * gait.signature)),
                    Self::armed(gait),
                );
                // Soft behind a catch, all but straight behind a parry: one
                // takes the pace off the ball and the other puts pace into it.
                let saving = Self::held(
                    ready,
                    Quat::from_rotation_x(
                        Self::SAVE_ELBOW
                            * (1.0 - 0.8 * gait.parry)
                            * (1.0 - 0.7 * gait.save_aim.y.max(0.0)),
                    ),
                    gait.save,
                );
                let leaping =
                    Self::held(saving, Quat::from_rotation_x(Self::JUMP_ELBOW), gait.jump);
                let holding = Self::held(
                    leaping,
                    Quat::from_rotation_x(Self::CRADLE_ELBOW),
                    gait.carry,
                );
                let elbow = Self::LAUNCH_ELBOW
                    + (Self::REACH_ELBOW - Self::LAUNCH_ELBOW) * gait.stretch
                    + Self::TRAIL_ELBOW * trailing * gait.stretch;
                let out = Self::held(holding, Quat::from_rotation_x(elbow), gait.reach);
                let down = Self::held(out, Quat::from_rotation_x(Self::DOWN_ELBOW), bracing);
                let down = Self::held(down, Quat::from_rotation_x(Self::GRASS_ELBOW), grassed);
                let down = Self::held(
                    down,
                    Quat::from_rotation_x(Self::BEATEN_ELBOW),
                    gait.beaten * (1.0 - leading.max(0.0)),
                );
                let down = Self::held(
                    down,
                    Quat::from_rotation_x(
                        (Self::RISE_ELBOW.0 + Self::RISE_ELBOW.1 * gait.rising).min(-0.30),
                    ),
                    gait.propping(),
                );
                let hurled = Self::held(
                    down,
                    Quat::from_rotation_x(Self::through(Self::THROW_ELBOW, gait.swing)),
                    throwing * striking.max(0.0),
                );
                let attacking = Self::held(hurled, Quat::from_rotation_x(Self::NOD_ELBOW), heading);
                Self::held(
                    attacking,
                    Quat::from_rotation_x(Self::through(Self::TOSS_ELBOW, gait.swing)),
                    tossing,
                )
            }
            // The hands. Loose on the run, up and open in the set, broken
            // back and turned out at full stretch, cupped under a ball he has
            // claimed. A glove that stays in line with its forearm reads as
            // the end of a stick, however good the arm above it is.
            Limb::Wrist => {
                let loose = Quat::from_rotation_x(
                    Self::WRIST_REST * (1.0 + 0.5 * gait.signature * self.side),
                );
                // Palms flat on the crown. Without it the gloves point off
                // the ends of the forearms and the pose reads as two arms
                // waving rather than as hands on a head.
                let slumped = Self::held(loose, Quat::from_rotation_x(Self::SLUMP_WRIST), on_head);
                let slumped = Self::held(
                    slumped,
                    Quat::from_rotation_z(self.side * Self::HIPS_WRIST_TURN)
                        * Quat::from_rotation_x(Self::HIPS_WRIST),
                    on_hips,
                );
                let slumped =
                    Self::held(slumped, Quat::from_rotation_x(Self::DOUBLED_WRIST), doubled);
                let slumped = Self::held(slumped, Quat::from_rotation_x(Self::URGE_WRIST), urging);
                let slumped =
                    Self::held(slumped, Quat::from_rotation_x(Self::POINT_WRIST), pointing);
                let ready = Self::held(
                    slumped,
                    Quat::from_rotation_z(self.side * 0.28)
                        * Quat::from_rotation_x(Self::SET_WRIST),
                    Self::armed(gait),
                );
                // Behind the ball: the gloves break back off the forearms so
                // the palms face it, and turn out flat for a parry, where the
                // whole point is a surface rather than a pair of hands.
                let saving = Self::held(
                    ready,
                    Quat::from_rotation_x(Self::SAVE_WRIST + Self::PARRY_WRIST * gait.parry),
                    gait.save,
                );
                let holding = Self::held(
                    saving,
                    Quat::from_rotation_z(-self.side * 0.20)
                        * Quat::from_rotation_x(Self::CRADLE_WRIST),
                    gait.carry,
                );
                // Splayed to cover an area, or turned in around a ball he has
                // actually got hold of.
                let out = Self::held(
                    holding,
                    Quat::from_rotation_z(
                        self.side * Self::REACH_WRIST_SPREAD * (1.0 - 2.0 * gait.claimed),
                    ) * Quat::from_rotation_x(Self::REACH_WRIST),
                    gait.reach,
                );
                let down = Self::held(out, Quat::from_rotation_x(Self::CRADLE_WRIST), bracing);
                // Palm flat on the turf, taking his weight.
                Self::held(
                    down,
                    Quat::from_rotation_x(Self::RISE_WRIST),
                    gait.propping(),
                )
            }
            // The fingers, which are the difference between a hand and a
            // mitten — see [`Limb::Finger`].
            //
            // Two numbers drive all ten of them: how CLOSED the hand is and
            // how far the fingers are FANNED apart, and between them they
            // cover everything a keeper's hands ever do. The splay that used
            // to be baked into the spawn transform is the `across` term
            // here, read back out of the knuckle's own position so the two
            // cannot drift apart.
            Limb::Finger(index) | Limb::Knuckle(index) => {
                let across = self.origin.x;
                // **A pointed hand is the one gesture where the four fingers
                // do different things.** The index goes straight and the
                // other three shut, which is the whole of what pointing is —
                // an open hand held out is a man showing you his palm.
                let grip = if index == 0 {
                    Self::grip(gait) * (1.0 - pointing)
                } else {
                    Self::grip(gait).max(Self::HAND_POINTING * pointing)
                };
                let fan = Self::fan(gait) * (1.0 - pointing);
                // Not every finger does the same thing, and that is most of
                // what separates a hand from a comb. A relaxed little finger
                // curls further than an index; a spread one goes wider.
                let bias = Self::FINGER_CURL[usize::from(index).min(3)];
                let curl = match self.limb {
                    Limb::Knuckle(_) => Self::GRIP_KNUCKLE,
                    _ => Self::GRIP_FINGER,
                } * grip
                    * bias;
                // The second segment carries no splay of its own: a finger
                // fans at the knuckle it grows out of and bends in a plane
                // after that, which is what makes a closing hand converge
                // rather than stay a fan all the way in.
                let splay = match self.limb {
                    Limb::Knuckle(_) => 0.0,
                    // Fanned OUT to cover a ball and drawn back IN as the
                    // hand closes: the two ends of the one gesture, and the
                    // reason the fan cannot simply be added to the rest
                    // splay.
                    _ => across * BodyParts::SPLAY * (1.0 + Self::FAN_WIDE * fan - grip),
                };
                Quat::from_rotation_z(splay) * Quat::from_rotation_x(curl)
            }
            // The thumb opposes, which is the whole of what a thumb is for:
            // out of the way of a ball coming into the palm, and across it
            // once the hand has closed on something.
            Limb::Thumb => {
                let grip = Self::grip(gait).max(Self::HAND_POINTING * pointing);
                let fan = Self::fan(gait) * (1.0 - pointing);
                Quat::from_rotation_z(
                    -self.side * (Self::THUMB_OUT + Self::THUMB_FAN * fan - Self::THUMB_IN * grip),
                ) * Quat::from_rotation_x(Self::THUMB_REST + Self::THUMB_CURL * grip)
            }
            Limb::Hip => {
                // **The stride carries the ground he covers.** The amplitude
                // is the larger of what he is working at and what the turf is
                // asking for — see [`Joint::stepping`] — and it is SIGNED by
                // the direction of travel, so a keeper dropping backwards
                // onto his line runs the cycle the other way round instead of
                // moonwalking down the pitch.
                let amplitude = Self::HIP_SWING.0 + Self::HIP_SWING.1 * Self::stepping(gait);
                // The lateral half of the same decomposition. A wide base
                // (both legs out, so the feet stay on their own sides of him)
                // and then each foot stepping across in turn — antiphase,
                // exactly like the stride, because at any instant one foot is
                // planted and sliding back under him while the other swings
                // on ahead of him.
                //
                // The near leg alone steps into a save; the far one is what
                // he is pushing off.
                // Whether THIS foot is the one off the ground, and how far
                // through its swing. The whole difference between a step and
                // a scissor: with the legs mirrored there was no swing to
                // pick up, and neither boot ever left the grass.
                let picking = Self::tread(leg).1 * Self::sidling(gait);
                let near_leg = (self.side * gait.save_aim.x).clamp(0.0, 1.0);
                // ⚠ Composed onto the WHOLE branch below rather than into the
                // run cycle. The base is a stance, not a stride: a keeper who
                // is set, or reaching, still has his feet where his last
                // side-step left them, and folding it into `running` lets the
                // set layer slerp it away underneath him.
                //
                // …and the whole leg turns onto the run first. See
                // [`Gait::open`]: a footballer travelling across himself
                // opens his hips and RUNS, and only what is left over after
                // that is a side-step. Outermost of the three, because it is
                // the frame the other two are expressed in — the splay is
                // across the line of the hips and the stride is along it,
                // and yawing the pair of them is exactly what opening up
                // does to a pair of legs.
                let across = Quat::from_rotation_y(Self::opened(gait))
                    * Quat::from_rotation_z(
                        Self::abduct(gait, self.side)
                            + Self::SAVE_STEP * gait.save * gait.save_aim.x * near_leg,
                    );
                // The run, plus what his legs do about a change of pace: feet
                // driving out behind him off the mark, planted out in front
                // under the brakes.
                let running = Quat::from_rotation_x(
                    // ⚠ [`Joint::striding`] and not the plain sinusoid, and
                    // the re-fit is applied to the foot's OFFSET rather than
                    // to the angle — `L·sin θ` is not linear in `θ`, so
                    // scaling the angle compresses the far end of every step
                    // and puts back most of the slip the shape just took
                    // out. Same trap as [`Joint::abduct`], which says so.
                    // `sin(amplitude)` IS the half-excursion as a share of a
                    // leg; the shape and the gain belong there, and the
                    // `asin` turns it back into a hip.
                    Self::swinging(gait, leg, amplitude) * gait.course.y
                        + Self::DRIVE_HIP * gait.drive
                        + Self::SHUFFLE_HIP_PICKUP * picking,
                );
                // …and under him when he is bent over them. ⚠ The knee
                // alone will not do it: the legs hang off the hips with
                // nothing beneath them, so bending a knee on its own swings
                // the shin backward and takes the boot off the grass, which
                // is what it did.
                let running = Self::held(
                    running,
                    Quat::from_rotation_x(Self::DOUBLED_HIP),
                    gait.doubled_over,
                );
                // ⚠ [`Joint::crouched`] and NOT `gait.set`. This is a slerp
                // onto a fixed angle, so whatever weight it is given is that
                // much of the stride gone — and a keeper walking with the
                // ball in his box used to arrive here at 0.83.
                let ready = Self::held(
                    running,
                    Quat::from_rotation_x(
                        Self::SET_HIP + Self::TOES_HIP * Self::on_his_toes(gait, self.side),
                    ),
                    Self::crouched(gait),
                );
                let ready = Self::held(
                    ready,
                    Quat::from_rotation_x(
                        Self::SET_HIP
                            + Self::SAVE_HIP * Self::stooping(gait)
                            // …and straight under him going the other way.
                            // See [`Joint::SAVE_RISE_HIP`].
                            + Self::SAVE_RISE_HIP * gait.save_aim.y.clamp(0.0, 1.0),
                    ),
                    gait.save,
                );
                let leaping = Self::held(ready, Quat::from_rotation_x(Self::JUMP_HIP), gait.jump);
                // In flight the legs trail — the near one straight behind
                // him because it is the one he pushed off, the far one
                // swinging up over it.
                let diving = Self::held(
                    leaping,
                    Quat::from_rotation_x(Self::DIVE_HIP + Self::DIVE_SCISSOR_HIP * leading),
                    // ⚠ …and NOT over a leap. `dive` means "off his feet"
                    // and stays 1 for a keeper who went straight up, so
                    // without this the trail slerps away the push the jump
                    // above just drew. See `PlayerActor::gait`.
                    gait.dive * gait.stretch * (1.0 - gait.jump),
                );
                let down = Self::held(diving, Quat::from_rotation_x(Self::DOWN_HIP), gait.grounded);
                // Knees drawn under him to push off, held at a constant
                // angle to the GRASS rather than to his own chest. See
                // [`Self::RISE_THIGH`].
                let down = Self::held(
                    down,
                    Quat::from_rotation_x(-(gait.over + Self::RISE_THIGH)),
                    Self::kneeling(gait),
                );
                // And the kick, on the striking leg only — the other one is
                // planted and keeps its stride.
                across
                    * Self::held(
                        down,
                        Quat::from_rotation_x(Self::through(Self::KICK_HIP, gait.swing)),
                        kicking * striking.max(0.0),
                    )
            }
            // Deepest as the leg folds through underneath the player, and all
            // but straight again by the time it reaches out to land. Squaring
            // the curve is what narrows the tuck to that one part of the
            // cycle; a plain cosine leaves the leading leg bent on touchdown,
            // which reads as a stumble rather than a stride.
            Limb::Knee => {
                let running = Quat::from_rotation_x(
                    // ⚠ [`Joint::tucking`], because the HIP has to solve
                    // against this same angle — a folded leg reaches less far
                    // forward than a straight one — and two copies of it do
                    // not stay the same angle.
                    Self::tucking(gait, leg)
                        // The swinging foot picks up out of a side-step, and
                        // a backpedalling man's knees come up in front of him
                        // — neither of which the sagittal cycle draws, since
                        // for a pure shuffle it has nowhere to swing to.
                        //
                        // Off the SWING rather than off the stride's own
                        // tuck: which foot is up is the thing a lateral gait
                        // is made of, and the tuck does not know.
                        + Self::shuffle_knee(gait, self.side)
                        + Self::SHUFFLE_PICKUP * Self::tread(leg).1 * Self::sidling(gait)
                        + Self::BACKPEDAL_KNEE
                            * Self::backing(gait)
                            * (0.5 + 0.5 * (leg - Self::TUCK_LEAD).cos()).powi(2)
                        // Sunk over the ball, the way a man carrying one runs.
                        + Self::CARRY_KNEE * gait.carrying,
                );
                // Weight off, knees soft. Paid for in height by
                // [`Self::SLUMP_DROP`].
                let running = Self::held(
                    running,
                    Quat::from_rotation_x(Self::SLUMP_KNEE),
                    gait.despair,
                );
                // …and bent under him, because that is what his hands are
                // resting on.
                let running = Self::held(
                    running,
                    Quat::from_rotation_x(Self::DOUBLED_KNEE),
                    gait.doubled_over,
                );
                let ready = Self::held(
                    running,
                    Quat::from_rotation_x(
                        Self::SET_KNEE + Self::TOES_KNEE * Self::on_his_toes(gait, self.side),
                    ),
                    Self::crouched(gait),
                );
                // Down under himself to a ball at his boots. Its own layer
                // above the set rather than a term inside the run cycle,
                // because a keeper making a save is nearly always set and the
                // set would slerp it away — see the note at [`Limb::Hip`].
                let saving = Self::held(
                    ready,
                    Quat::from_rotation_x(
                        Self::SET_KNEE
                            + Self::SAVE_KNEE * Self::stooping(gait)
                            + Self::SAVE_RISE_KNEE * gait.save_aim.y.clamp(0.0, 1.0),
                    ),
                    gait.save,
                );
                let leaping = Self::held(saving, Quat::from_rotation_x(Self::JUMP_KNEE), gait.jump);
                let diving = Self::held(
                    leaping,
                    Quat::from_rotation_x(Self::DIVE_KNEE + Self::DIVE_SCISSOR_KNEE * trailing),
                    gait.dive * gait.stretch * (1.0 - gait.jump),
                );
                let down = Self::held(
                    diving,
                    Quat::from_rotation_x(Self::DOWN_KNEE),
                    gait.grounded,
                );
                let down = Self::held(
                    down,
                    Quat::from_rotation_x(Self::RISE_KNEE),
                    Self::kneeling(gait),
                );
                // The kicking knee whips through the ball; the standing one
                // bends to take the weight while it does.
                let swung = Self::held(
                    down,
                    Quat::from_rotation_x(Self::through(Self::KICK_KNEE, gait.swing)),
                    kicking * striking.max(0.0),
                );
                let planted = Self::held(
                    swung,
                    Quat::from_rotation_x(Self::PLANT_KNEE),
                    kicking * (-striking).max(0.0),
                );
                // A throw-in is taken off both feet, and they give under it:
                // the knees bend into the arch and push back up through the
                // release. It is the only strike in football where the legs
                // do the same thing as each other.
                Self::held(
                    planted,
                    Quat::from_rotation_x(Self::through(Self::TOSS_KNEE, gait.swing)),
                    tossing,
                )
            }
            // The foot rolls through the stride: pulled up as the leg
            // reaches out to land, driven down and back off the toe at the
            // end of the push. See [`Self::ANKLE_PLANTAR`].
            //
            // Scaled by `run` like every other amplitude here, so a
            // standing player's foot is flat on the grass and none of the
            // standing poses — the set, the slump, the cradle — has to know
            // this joint exists.
            Limb::Ankle => {
                let middle = (Self::ANKLE_PLANTAR - Self::ANKLE_DORSI) * 0.5;
                let reach = (Self::ANKLE_PLANTAR + Self::ANKLE_DORSI) * 0.5;
                // Off `stepping` rather than `run` for the same reason the
                // hip is: the roll belongs to the step, and at a walk the
                // step is bigger than the effort. Signed with the stride, so
                // going backwards the foot rolls the other way — a man never
                // puts a heel down travelling backwards, which is what the
                // plantar term below is.
                // **Sideways, the pitch above has nothing to drive it.** It
                // is signed by `course.y`, so a boot travelling across the
                // body held ONE angle for the whole cycle — the welded foot
                // this joint exists to stop, back again on the other axis.
                //
                // A side-step turns the feet out toward the way he is going
                // and rolls them through the push, and the swinging one
                // points as it leaves the grass.
                let across = Self::sidling(gait);
                let (tread, lift) = Self::tread(leg);
                let rolling = Quat::from_rotation_y(
                    Self::TOE_OUT * gait.course.x * across
                        // **…and this man's own feet, whichever way he is
                        // going.** The term above is scaled by `course.x`,
                        // so it is a claim about a SIDE-STEP — and an
                        // outfielder is travelling forwards on 93% of the
                        // frames he is moving in, which left every boot on
                        // the pitch pointing dead along its own run for the
                        // whole match. Signed outward off `self.side`, so
                        // the two feet splay rather than both pointing one
                        // way. See [`Gait::toes`].
                        + self.side * gait.toes,
                ) * Quat::from_rotation_z(
                    Self::FOOT_ROLL * gait.course.x * across * tread,
                ) * Quat::from_rotation_x(
                    (middle - reach * swing * gait.course.y) * Self::stepping(gait)
                            + Self::BACKPEDAL_ANKLE * Self::backing(gait)
                            + Self::ANKLE_PLANTAR * 0.5 * lift * across
                            // …and up onto his toes for a ball above him,
                            // outside the `stepping` scaling above because
                            // a keeper making this save is standing still.
                            + Self::ANKLE_PLANTAR * Self::SAVE_RISE_TOE * Self::reaching(gait),
                );
                // Off his feet there is nothing to push against and the
                // toes fall into a point.
                let flying = Self::held(
                    rolling,
                    Quat::from_rotation_x(Self::DIVE_ANKLE),
                    gait.dive.max(gait.jump),
                );
                // …and the standing leg locks under a kick while the
                // striking foot points through the ball.
                Self::held(
                    flying,
                    Quat::from_rotation_x(Self::ANKLE_PLANTAR * 0.8),
                    kicking * striking.max(0.0),
                )
            }
        }
    }

    /// How much authority a swing has over the pose at this point in it.
    ///
    /// Full across the middle and blended to the run cycle at each end, so
    /// the kick arrives and leaves without a pop. NOT a taper across the whole
    /// swing: the keys put the backswing at −1 and the follow through at +1,
    /// so fading the authority out toward them cancels the two halves of a
    /// kick that are not the moment of contact — which is to say, all of it.
    fn taper(swing: f32) -> f32 {
        Actors::ease((1.0 - swing.abs()) / Self::KICK_BLEND)
    }

    /// **The opening as the rig applies it**: [`Gait::open`], less whatever
    /// a kick is taking off it.
    ///
    /// A footballer striking a ball plants and swings at the TARGET, not
    /// along the run he arrived on — both feet turn onto it, which is
    /// exactly what the follow-through facing (`Actors::facing`) already
    /// draws for the rest of him. Left in, the one moment his hips are most
    /// obviously open would be the one moment they are pointed somewhere
    /// else: the course during a follow-through is measured against the ball
    /// he has just played, so a man who strikes one across himself has a
    /// large opening precisely while his boot is going through it.
    ///
    /// Read by every place the yaw is applied — the sockets, the legs, the
    /// seat of the shorts and the chest that follows them — because they
    /// have to agree. Legs swung from sockets that turned by a different
    /// amount is not a pose, it is a hip out of its joint.
    fn opened(gait: Gait) -> f32 {
        gait.open * (1.0 - gait.power * Self::taper(gait.swing))
    }

    fn blend(range: (f32, f32), run: f32) -> f32 {
        range.0 + range.1 * run
    }

    /// **How far a running body is tipped forward**, in radians, and **how
    /// far it is rocked**, likewise.
    ///
    /// One function each because the HEAD has to cancel what the chest does
    /// with them — a runner's eyes stay on the play while his trunk works
    /// underneath him — and two copies of an expression do not stay the same
    /// expression. They had already parted: the chest's roll was moved onto
    /// [`Joint::cycling`] and its lean widened by [`Joint::LEAN_SPREAD`],
    /// while the head still cancelled the old `run`-scaled version. What
    /// that draws is a walker whose head tips side to side once a step,
    /// which is the one thing nobody's does and the exact fault the counter
    /// term was written for.
    ///
    /// Both carry [`Joint::upright`], so a keeper who is holding the ball,
    /// set, saving or off his feet stands out of the pair together and his
    /// head does not tip back to cancel a lean he no longer has.
    fn leaning(gait: Gait) -> f32 {
        Self::blend(Self::LEAN, gait.run)
            * (1.0 + Self::LEAN_SPREAD * gait.lean)
            * Self::upright(gait)
    }

    fn rocking(gait: Gait) -> f32 {
        Self::ROCK * Self::cycling(gait) * (gait.phase + FRAC_PI_2).sin() * Self::upright(gait)
    }

    /// **How much of the run cycle's carriage he is taking at all** — 1 for
    /// a man running and 0 for a goalkeeper who is doing something with his
    /// hands.
    ///
    /// He straightens up for all four, and the load-bearing reason is that
    /// the arm angles are measured off an upright chest: leave the lean and
    /// the rock in and a ball drifts a couple of centimetres out of his
    /// gloves every stride.
    fn upright(gait: Gait) -> f32 {
        (1.0 - gait.carry) * (1.0 - gait.dive) * (1.0 - Self::crouched(gait)) * (1.0 - gait.save)
    }

    /// How much of the trunk's forward lean the neck takes back out. Not all
    /// of it — a sprinter is looking a few metres in front of his own feet,
    /// not at the horizon.
    const HEAD_LEVEL: f32 = 0.75;

    /// **The shape of a stride**, −1 to +1: where this leg is fore and aft at
    /// this point in its own cycle.
    ///
    /// ⚠ **A sinusoid does not carry the ground.** A planted foot is
    /// stationary on the turf, so relative to a body travelling at `v` it
    /// travels at exactly `−v` for the whole of its stance — a straight
    /// line. A sinusoid does that at ONE INSTANT, mid-stance, which is the
    /// instant [`crate::players::actors::Actors::stride_of`] fits its amplitude
    /// to and the only instant `the_feet_carry_the_ground_he_covers` looks at.
    /// Either side of it the cosine falls away and then changes sign. Measured
    /// across every frame in which the boot is genuinely on the grass, the
    /// planted foot carried a **mean of 18–24% of the ground the body
    /// covered** at walking and jogging pace, and **−103% at the worst point
    /// of the stance** — travelling forward, at the body's own speed, while
    /// planted. `measure_stance` prints it.
    ///
    /// This is the same fault [`Joint::tread`] was written to take out of the
    /// side-step, still sitting in the forward run — which is **93% of the
    /// frames an outfielder moves in**.
    ///
    /// Blended toward a TRIANGLE, which travels at a constant rate and so
    /// matches the turf across the whole of its descent rather than at a
    /// point. Not all the way: the corners of a pure triangle are a foot
    /// reversing direction instantly, which is its own artefact, and the
    /// sinusoid rounds them off. `(2/π)·asin(sin θ)` is the triangle through
    /// the same zeros and peaks.
    fn striding(leg: f32) -> f32 {
        let sine = leg.sin();
        let triangle = sine.asin() * (2.0 / PI);
        sine + (triangle - sine) * Self::STRIDE_SHAPE
    }

    /// **How far this knee is folded through the FORWARD stride**, in
    /// radians — deepest as the leg folds through underneath him and all but
    /// straight again by the time it reaches out to land.
    ///
    /// Squaring the curve is what narrows the tuck to that one part of the
    /// cycle; a plain cosine leaves the leading leg bent on touchdown, which
    /// reads as a stumble rather than a stride.
    ///
    /// Its own function because [`Limb::Hip`] has to solve against exactly
    /// this angle: a folded leg does not reach as far forward as a straight
    /// one, so the hip that puts a straight foot on the mark puts a bent one
    /// somewhere else. See the two-link solve there.
    fn tucking(gait: Gait, leg: f32) -> f32 {
        Self::tucked(leg, Self::stepping(gait))
    }

    /// …with the stride's share handed in, so that the same expression at
    /// zero is the knee a man STANDING there holds. [`Joint::swinging`]
    /// needs both and they have to be one function.
    fn tucked(leg: f32, stepping: f32) -> f32 {
        let tuck = (0.5 + 0.5 * (leg - Self::TUCK_LEAD).cos()).powi(2);
        Self::KNEE_REST + (Self::KNEE_FLEX.0 + Self::KNEE_FLEX.1 * stepping) * tuck
    }
    /// Where in the cycle the fold is deepest, and how soft a knee is with no
    /// stride in it at all.
    const TUCK_LEAD: f32 = -0.2;
    const KNEE_REST: f32 = 0.07;

    /// **The two links of a leg**: hip to knee, and knee to the sole of the
    /// boot.
    ///
    /// The second carries the shin, the ankle's own drop and the boot,
    /// because as far as a stride is concerned they are one rigid piece
    /// hanging off the knee — and the pair sum to [`Physique::LEG`], which
    /// is what the stride model turns ground into an angle against.
    const THIGH_LINK: f32 = Physique::THIGH;
    const SHIN_LINK: f32 = Physique::LEG - Physique::THIGH;

    /// …and what that costs in amplitude.
    ///
    /// The amplitude is fitted so the foot's speed at mid-stance equals the
    /// body's, and a triangle's slope there is `2/π` of a sinusoid's — so
    /// the same excursion carries less ground and the swing has to be that
    /// much bigger. Exactly the bookkeeping [`Joint::TREAD_GAIN`] does for
    /// the lateral gait, and for the same reason.
    ///
    /// ⚠ **It is paid in FULL, and it used to be paid at 0.8.**
    ///
    /// The discount was there because [`Joint::stepping`]'s flight term —
    /// `run · spring`, unbounded and `max`ed against the ground — was paying
    /// for the same shortfall a second time, and the two together put a
    /// sprinter's thighs 111° apart, which is past what a body reaches.
    ///
    /// The flight term is now a bounded bonus ON the ground rather than a
    /// claim of its own, so it no longer pays for anything and the re-fit has
    /// to. Left at 0.8 the planted foot carried 99% of the ground at six
    /// metres a second against a floor of 105 — it skated, which is the
    /// failure this whole model exists to prevent and the reason the
    /// shortened stride could not simply be taken out of the swing.
    fn stride_gain() -> f32 {
        let full = 1.0 / (1.0 - Self::STRIDE_SHAPE + Self::STRIDE_SHAPE * 2.0 / PI);
        1.0 + (full - 1.0) * Self::STRIDE_GAIN
    }
    const STRIDE_GAIN: f32 = 1.0;
    const STRIDE_SHAPE: f32 = 0.55;

    /// **The hip angle that puts the FOOT where the stride wants it**, given
    /// the knee it is going to be drawn with.
    ///
    /// ⚠ **The knee moves the foot fore and aft, and the hip never knew.**
    /// A leg folded 57° sits its foot thirty degrees BEHIND the line of its
    /// own thigh, and the fold moves through the cycle — so a hip angle
    /// worked out as though the leg were a straight stick puts the foot
    /// somewhere different at every phase, and the difference is the knee
    /// fighting the hip. Measured, that is why the planted foot still skated
    /// through its touchdown after [`Joint::striding`] had made the intended
    /// profile right: the foot reached grass level while the swing was still
    /// carrying it forward, because it was not where the stride thought.
    ///
    /// Two links, closed form, no iteration. With the thigh `h` off vertical
    /// and the knee folded by `k`, the foot's offset is
    /// `T·sin h + S·sin(h + k)`, which is `R·sin(h + φ)` for
    /// `R = |T + S·e^{ik}|` and `φ = arg(T + S·e^{ik})` — the leg's effective
    /// length and the angle its foot sits off the thigh. So the hip is
    /// `asin(want / R) − φ`, and at `k = 0` it collapses to exactly the
    /// straight-stick expression this replaces.
    fn swinging(gait: Gait, leg: f32, amplitude: f32) -> f32 {
        let want = amplitude.sin() * Self::stride_gain() * Self::striding(leg);
        let knee = Self::tucking(gait, leg);
        let along = Self::THIGH_LINK + Self::SHIN_LINK * knee.cos();
        let out = Self::SHIN_LINK * knee.sin();
        // In units of a whole leg, because `want` is: `amplitude` is an angle
        // at the hip of a straight one.
        let reach = along.hypot(out) / Physique::LEG;
        // ⚠ **Faded in with the STRIDE, because at rest there is nothing to
        // correct and the standing pose is the reference every other test
        // measures against.** This rig stands its players with the knees
        // softly bent, so the solve's honest answer for a still figure is to
        // bring the thigh 7° forward and put the foot under the hip — which
        // is geometrically right, moves every standing man in the squad, and
        // took `a_save_keeps_his_boots_on_the_grass` with it. `stepping` is
        // 0.31 by a slow walk, so the correction is at full strength
        // everywhere it does any work.
        let settled = Actors::ease(Self::stepping(gait) / Self::STRIDE_SETTLE);
        (-want / reach).clamp(-0.95, 0.95).asin() - out.atan2(along) * settled
    }
    /// How much of a stride it takes for the two-link solve to be worth
    /// making. Below this he is standing about, not striding.
    const STRIDE_SETTLE: f32 = 0.15;

    /// **How far the shoulder swings**, in radians.
    ///
    /// Its own curve rather than [`Joint::strides`], and the cube is the
    /// whole of the point: a walker's arms HANG and swing along with him,
    /// and a sprinter DRIVES his. Those are not one movement scaled, and a
    /// straight line drawn through both gets one end wrong — this rig got
    /// the top one wrong. Measured, the hand travelled **30 cm through a
    /// walk and 36 cm through a flat sprint** while the stride went from
    /// 52 cm to 109 and the foot lift from 9 cm to 48: a squad whose arms
    /// did a brisk walk at every speed, which is most of why a sprint read
    /// as a walk played fast. See [`Joint::ELBOW_DRIVE`], which is the
    /// other half of the same picture.
    fn arming(gait: Gait) -> f32 {
        let driven = Self::cycling(gait) * gait.spring;
        Self::ARM_SWING.0 + Self::ARM_SWING.1 * driven + Self::ARM_DRIVE * driven * driven * driven
    }

    /// **How much of a locomotion cycle his body is drawing**, 0 standing
    /// and 1 flat out — whichever way he happens to be going.
    ///
    /// ⚠ **Every amplitude above the waist used to come off `run`, and
    /// `run` is `speed / SPRINT`: a claim about a sprint.** Measured over
    /// real recordings a goalkeeper spends nine tenths of his moving time
    /// under 1.5 m/s, where `run` is 0.08–0.17 — so while his boots swept
    /// 30 cm his gloves travelled 8 cm and his chest turned four degrees,
    /// against the 7.5 cm and 1.5° of a man standing still breathing. A
    /// trunk and two arms carried rigid on top of two working legs is most
    /// of what reads as an impaired gait, and it is why every report about
    /// this rig has been about the one man who does his moving at a walk.
    ///
    /// This is the fourth time in this crate a signal quoted as a share of a
    /// forward sprint has turned out to be nearly zero for the gait a
    /// goalkeeper actually uses — see [`Gait::carry_ground`], which is
    /// exactly this repair made to the legs, and the `course.y` traps in
    /// [`Joint::stepping`]. **The legs already know how big a step they are
    /// taking.** This is that step, as a share of the full run cycle, and
    /// the body above them takes its size from it.
    ///
    /// Floored by the run itself so that nothing at the top of the range
    /// moves: at a sprint the tuned cycle wins, exactly as it does in
    /// `stepping`. What changes is the bottom, and it changes for all
    /// twenty-two — a walking centre-half swings his arms too.
    fn cycling(gait: Gait) -> f32 {
        let carried = (gait.carry_ground / (Self::HIP_SWING.0 + Self::HIP_SWING.1)).clamp(0.0, 1.0);
        carried
            .max(gait.run)
            // …and a side-step, whose stride is deliberately a third of a
            // forward one and whose body works just as hard. Without this
            // the whole lateral gait inherits a zero from a term nobody
            // thought about, which is the shape of every one of the four.
            .max(Self::sidling(gait) * Self::LATERAL_BODY)
    }
    /// How much of a run cycle a body draws through a side-step. Short quick
    /// steps move a trunk and a pair of arms as much as long slow ones.
    const LATERAL_BODY: f32 = 0.45;

    /// **How much of the run cycle his legs are actually drawing** — the
    /// effort he is putting in, or the ground he is covering, whichever is
    /// asking for more.
    ///
    /// The amplitude used to come off `run` alone (`speed / SPRINT`) while
    /// the PHASE came off ground covered, and the two disagree badly at the
    /// bottom of the range: at half a metre a second the cadence was right
    /// and the legs moved a third of the distance the body did. That gap is
    /// the reported "glides across the field without any obvious foot
    /// movement", and it is worst in the band a goalkeeper spends 87% of a
    /// match in. See [`Gait::carry_ground`].
    fn stepping(gait: Gait) -> f32 {
        let ground = (gait.carry_ground - Self::HIP_SWING.0) / Self::HIP_SWING.1;
        // ⚠ The effort term is a claim about a FORWARD run, and only about
        // one: it is the tuned sprint, where the feet genuinely go back
        // faster than the ground because there is a flight phase to pay for
        // it. There is no flight phase in a side-step and none in a
        // backpedal, so out of the forward quadrant the ground is the whole
        // of the answer — otherwise a keeper shuffling at four metres a
        // second plants his feet a metre and a half apart.
        // ⚠ **…and it is a BONUS on the ground, not a claim of its own.**
        //
        // It used to be `run · spring` outright, and `max`ed against the
        // ground. That is an assertion about the amplitude in units of a
        // sprint cycle, and it does not know what a stride is: shorten
        // [`Actors::STRIDE`] and the ground demand falls while this does not,
        // so the legs go on swinging exactly as far as before and the whole
        // stride model is bypassed. Measured on the day the stride was
        // shortened, the boots still came 15% further apart than the ground
        // he was covering, at every pace above four metres a second.
        //
        // What a flight phase actually buys is a few per cent: his feet do go
        // back faster than the turf because for part of the cycle neither is
        // on it. So it is a percentage of the ground, and it cannot outrun
        // the thing it is compensating for.
        let flight = (gait.run * gait.spring * gait.course.y.max(0.0))
            .min(ground.clamp(0.0, 1.0) * (1.0 + Self::FLIGHT_BONUS));
        // ⚠ **And nothing at all for a side-step**, which is a claim about
        // the SAGITTAL cycle and is the whole of what this answers. Measured,
        // both terms above come out at 0.000 for a pure side-step at every
        // speed against 0.35 for the same speed forwards — and the obvious
        // repair, flooring it at some share of a run, is what put a running
        // knee into a gait that has no stride to fold it into. The flex here
        // reaches the knee through `tuck`, a curve keyed to where a leg is in
        // a STRIDE, so at 0.45 of a cycle the stance knee swung between 4°
        // and 53° every step: legs buckling under a man splayed across his
        // own base, which is exactly how it was reported. A side-step does
        // bend a knee, and it bends it by roughly a constant — see
        // [`Joint::SHUFFLE_KNEE`], which is where that lives now.
        flight.max(ground.clamp(0.0, 1.0))
    }

    /// How much of a running stride he is taking, given which way he is
    /// going. See [`Joint::SIDE_STEP`].
    pub fn shortening(course: Vec2) -> f32 {
        // Squared on the lateral axis, so it only really bites once he is
        // mostly travelling across himself. Linear, a man running diagonally
        // at a jog was being given a four-and-a-half-a-second cadence.
        (1.0 - (1.0 - Self::SIDE_STEP) * course.x * course.x
            - (1.0 - Self::BACK_STEP) * (-course.y).max(0.0))
        .clamp(Self::SIDE_STEP, 1.0)
    }

    /// How much of his travel is ACROSS his own body, 0..1 — the amount of
    /// the shuffle, without its direction.
    ///
    /// ⚠ **Not scaled by `run`.** A shuffle happens at a fifth of a sprint,
    /// so `speed / SPRINT` drew every side-step term at a fifth of its size:
    /// the pick-up came out at eight degrees of knee and **the feet never
    /// left the grass**, which was half of what made the first render read
    /// as a linkage. What a side-step's own terms scale with is how sideways
    /// he is going, not how fast — a man stepping across himself at a metre
    /// a second is fully stepping across himself.
    fn sidling(gait: Gait) -> f32 {
        Self::afoot(gait) * gait.course.x.abs()
    }

    /// …and how much of it is BACKWARDS, which is the other half of the same
    /// decomposition.
    fn backing(gait: Gait) -> f32 {
        Self::afoot(gait) * (-gait.course.y).max(0.0)
    }

    /// Whether he is going anywhere at all, 0..1 — the only thing the two
    /// above still need from his pace, and it saturates as soon as he is
    /// walking. `SIDLE_RAMP` of a sprint is 0.7 m/s.
    fn afoot(gait: Gait) -> f32 {
        Actors::ease(gait.run / Self::SIDLE_RAMP)
    }
    const SIDLE_RAMP: f32 = 0.12;

    /// How far below his own chest the save is, 0..1. A ball at his boots
    /// takes his whole body down with it; one at head height takes none of
    /// it, which is why the stoop cannot simply ride the save itself.
    fn stooping(gait: Gait) -> f32 {
        (-gait.save_aim.y).clamp(0.0, 1.0)
    }

    /// …and how far ABOVE it, 0..1 — the other half of the same axis, and
    /// the half that had nothing hanging off it. See [`Joint::SAVE_RISE_HIP`].
    ///
    /// Carries the save itself, unlike `stooping`, because it is read
    /// outside the save's own hold: the height he gains is a real one and is
    /// paid for at the hips, where there is no `save` weight to inherit.
    fn reaching(gait: Gait) -> f32 {
        gait.save * gait.save_aim.y.clamp(0.0, 1.0)
    }

    /// **How far his RIGHT leg is abducted through a side-step**, at this
    /// point in the cycle: the base he has planted, plus the step itself.
    ///
    /// The left leg is at exactly the opposite angle — the two are half a
    /// cycle apart, so as one foot slides back under him the other swings on
    /// ahead — which means one number describes the splay of both, and the
    /// height it costs can be paid for exactly rather than averaged over.
    /// **Where one foot is in its own step, and whether it is carrying him.**
    ///
    /// `.0` is −1..1: the foot's offset as a share of the step. It runs
    /// LINEARLY back across the stance, because a planted foot is stationary
    /// on the turf and therefore travels at exactly `−v` relative to a body
    /// moving at `v` — not at the varying rate of a sinusoid, which is
    /// stationary for one instant and skating either side of it. Then it
    /// sweeps forward across the swing, eased at both ends so the foot
    /// gathers and places rather than snapping.
    ///
    /// `.1` is 0..1 over the swing and zero through the stance: whether this
    /// is the foot that is off the ground.
    ///
    /// This is what makes the two legs stop being mirrors of each other. The
    /// stance is longer than the swing (see [`Joint::SHUFFLE_DUTY`]), so at
    /// any instant one foot is planted and drifting slowly while the other
    /// is up and travelling fast — which is what taking a step is, and what
    /// a pair of legs moving in perfect antisymmetry is not.
    fn tread(leg: f32) -> (f32, f32) {
        // Phase 0 of a foot's own cycle is the moment it is planted furthest
        // in the direction of travel, which is a quarter cycle ahead of the
        // sine the rest of the rig runs on.
        let step = ((leg - FRAC_PI_2) / TAU).rem_euclid(1.0);
        if step < Self::SHUFFLE_DUTY {
            (1.0 - 2.0 * step / Self::SHUFFLE_DUTY, 0.0)
        } else {
            let over = (step - Self::SHUFFLE_DUTY) / (1.0 - Self::SHUFFLE_DUTY);
            (-1.0 + 2.0 * Actors::ease(over), (over * PI).sin())
        }
    }

    /// How far his weight has shifted toward his right foot, −1..1: he is
    /// over whichever one is not in the air.
    ///
    /// The terms that belong to the BODY rather than to one leg come off
    /// this — the pelvis lists over the foot carrying him, the chest rides
    /// with it, the arms answer it. Without them the trunk holds a single
    /// pose for the whole cycle while the legs work underneath it, which is
    /// a mechanism with a body bolted on top and is exactly what rendered.
    fn carried(gait: Gait) -> f32 {
        Self::tread(gait.phase + PI).1 - Self::tread(gait.phase).1
    }

    /// How far one leg is abducted, in radians: the base he has planted plus
    /// where that foot is in its own step.
    ///
    /// ⚠ **Solved from the foot's POSITION, not written as an angle.** The
    /// tread is linear because a planted foot travels at a constant rate,
    /// and `L·sin θ` is not linear in `θ` — writing the tread straight into
    /// the angle compressed the far end of every step by a fifth and put the
    /// slip back that the whole thing exists to remove.
    fn abduct(gait: Gait, side: f32) -> f32 {
        let leg = if side < 0.0 {
            gait.phase + PI
        } else {
            gait.phase
        };
        let across = side * Self::SHUFFLE_STANCE * Self::side_step(gait) * gait.course.x.abs()
            + Self::side_step(gait) * gait.course.x * Self::tread(leg).0;
        // ⚠ Against the leg he is standing ON, not against a straight one:
        // the knee works through a side-step (see [`Joint::shuffle_knee`])
        // and `L·sin θ` only places the foot if `L` is the length that is
        // actually under him.
        (Self::reachable(across) / Self::folded(gait, side))
            .clamp(-0.95, 0.95)
            .asin()
    }

    /// **How far out to the side a foot will actually be planted**, in
    /// metres from the centreline.
    ///
    /// The backstop, and the one number in the lateral gait that is not derived
    /// from anything. Everything above it solves the side-step from the ground
    /// the body covers, which is right and is what stops a shuffle skating —
    /// but it is an equation, and an equation handed an impossible demand
    /// returns an impossible answer. The demand really is made: measured over a
    /// recording, 4.5% of the frames an outfielder is running in have him more
    /// than 100° off his own facing, because
    /// [`crate::players::actors::Actors::PIVOT_RATE`] will not let his heading
    /// come round any faster than a body can turn. Solved honestly, a man
    /// reversing at six and a half metres a second was drawn with his boots
    /// 1.88 m apart and his crown 69 cm below standing.
    ///
    /// [`crate::players::body::Gait::open`] is what removes nearly all of that,
    /// by turning his legs onto the run so there is no side-step left to solve.
    /// This is what catches the rest. A step his legs cannot reach is not a
    /// step, and what gives is the step rather than the man.
    const SIDE_PLANT: f32 = 0.45;

    /// …applied as a soft saturation rather than a clamp.
    ///
    /// Exact to within a percent below three-quarters of the reach, so every
    /// step a footballer really takes is still carried honestly by the foot
    /// that is on the grass — that property is the whole of `Joint::tread`
    /// and is what a clamp would have quietly broken at ordinary speeds.
    /// Above it the curve flattens onto the limit and stays there however
    /// impossible the demand gets.
    fn reachable(across: f32) -> f32 {
        across / (1.0 + (across / Self::SIDE_PLANT).powi(4)).powf(0.25)
    }

    /// How far one foot travels across him in a step, in metres — half of
    /// it either side of the base. The base is sized off this and off how
    /// sideways he is going, not off how hard he is working: it is there to
    /// keep his feet from crossing, and what they have to clear is each
    /// other.
    fn side_step(gait: Gait) -> f32 {
        // ⚠ The GROUND demand alone, not `stepping`. That one folds in the
        // forward run's flight allowance, which is a claim about a sprint —
        // feet outrunning the turf because there is a moment when neither is
        // on it — and a side-step has no flight phase to pay for it. Left in,
        // a diagonal jog drew a lateral step half again too big.
        Physique::LEG * gait.carry_ground.sin() * Self::TREAD_GAIN
    }

    /// **How hard this leg is pushing**, −1 taking his weight to +1 sending
    /// it back the other way, and 0 for a foot that is off the grass.
    ///
    /// A side-step is one leg receiving and the other driving, and the knee
    /// is where both of them happen: it folds to take the landing and
    /// extends through the push. [`Joint::SHUFFLE_KNEE`] held it at a
    /// constant — which was the right repair at the time, because the thing
    /// it replaced was a RUN's knee curve keyed to a stride a side-step does
    /// not have — but a constant is a strut, and a pair of struts swinging
    /// from the hips is what the render showed: straight splayed legs in a
    /// gait that is almost entirely legs.
    ///
    /// Only the planted foot pushes. Through the stance [`Joint::tread`]
    /// runs LINEARLY from +1, planted out ahead of his travel, to −1, behind
    /// him at the end of the push — so it already is how far through the
    /// push he is, and nothing new has to be integrated to know it.
    fn driving(leg: f32) -> f32 {
        let (tread, lift) = Self::tread(leg);
        -tread * (1.0 - lift.clamp(0.0, 1.0))
    }

    /// **How far this knee is bent through a side-step**, in radians: the
    /// soft carriage, the pick-up on the swing, and the push.
    ///
    /// One function because four things need the same answer and they
    /// cannot be allowed to disagree — the knee itself, the height the bend
    /// costs ([`Joint::stance_drop`]), the hip it lists
    /// ([`Joint::hip_list`]) and the leg length [`Joint::abduct`] solves the
    /// splay against. A knee written into the pose and forgotten by the
    /// other three puts a foot through the turf, which is the fault every
    /// one of those exists to prevent.
    /// ⚠ **The stance bend only — the swing leg's PICK-UP is not in here,
    /// and must not be.** Everything that reads this is levelling him
    /// against the GROUND, and a foot in the air is not on the ground: fold
    /// its knee into `leg_reach` and `hip_list` drops that hip to go and
    /// find turf it is deliberately clear of, which cancels most of the lift
    /// (measured: the swing foot came up 4.1 cm instead of 6.5). The pick-up
    /// is added at the knee itself, where it belongs.
    fn shuffle_knee(gait: Gait, side: f32) -> f32 {
        let leg = if side < 0.0 {
            gait.phase + PI
        } else {
            gait.phase
        };
        Self::sidling(gait) * Self::SHUFFLE_KNEE * (1.0 - Self::SHUFFLE_PUSH * Self::driving(leg))
    }

    /// **How far below the hip one foot reaches**, in metres — the leg's own
    /// length, folded by its knee and tilted by its splay.
    ///
    /// The single place the lateral gait's geometry is worked out. It used
    /// to be two: `splay_drop` knew about the abduction and `shuffle_drop`
    /// about a knee that was the same constant for both legs, and the moment
    /// the two knees stopped being equal that split stopped being able to
    /// express the answer.
    fn leg_reach(gait: Gait, side: f32) -> f32 {
        // Thigh and shin are the same length here, so the fold is isosceles
        // and the shortening is exact.
        Self::folded(gait, side) * Self::abduct(gait, side).cos()
    }

    /// …and the same leg before it is tilted, which is what
    /// [`Joint::abduct`] has to solve the splay against: `L·sin θ` is only
    /// the foot's offset if `L` is the leg he is actually standing on.
    fn folded(gait: Gait, side: f32) -> f32 {
        Physique::LEG * (Self::shuffle_knee(gait, side) * 0.5).cos()
    }

    /// …and the height his stance costs him, in metres.
    ///
    /// **A splayed leg is a SHORTER leg, and so is a bent one.** Both legs
    /// open and close through a shuffle and both knees work, so without
    /// paying for it his feet leave the grass at the wide point of every
    /// step and go through it at the narrow one — a keeper bouncing across
    /// his own six-yard box. It comes straight out of the triangle; there is
    /// nothing to tune.
    fn stance_drop(gait: Gait) -> f32 {
        // The mean of the two REACHES rather than of the angles, because it
        // is the reach the height is linear in — see [`Joint::hip_list`],
        // which then puts each hip where its own leg needs it.
        Physique::LEG - (Self::leg_reach(gait, 1.0) + Self::leg_reach(gait, -1.0)) * 0.5
    }

    /// **How much lower this hip sits than its partner, in metres.**
    ///
    /// Two legs of the same length, splayed by different angles, cannot both
    /// stand on level ground from level hips: the more splayed one is
    /// vertically shorter. Once the legs stopped being mirror images — which
    /// is the whole of the fix — that stopped being a hypothetical, and an
    /// averaged body height put the flatter foot four and a half centimetres
    /// under the turf.
    ///
    /// A person solves it by dropping the hip on the splayed side, and so
    /// does this. It is also, for free, the pelvic list that a side-step
    /// visibly has and that the rig had no other way to produce: the legs
    /// hang off the carriage rather than off the pelvis, so rotating
    /// [`Limb::Pelvis`] moves the seat of the shorts and nothing else.
    fn hip_list(gait: Gait, side: f32) -> f32 {
        (Self::leg_reach(gait, side) - Self::leg_reach(gait, -side)) * 0.5
    }

    /// …and the angle that amounts to, for the parts of him that ride on top
    /// of it.
    ///
    /// ⚠ **Sockets 176 mm apart turn four centimetres of list into thirteen
    /// degrees, and a spine does not pass that on.** Rendered raw onto the
    /// pelvis AND the chest it came to twenty-six degrees and read as a man
    /// falling over sideways rather than one taking a step. The seat of the
    /// shorts takes the geometry, because it sits ON the legs; the chest
    /// takes a quarter of it, because the lumbar spine is what absorbs the
    /// rest — which is also why a person's head and shoulders stay level
    /// while his hips work underneath him.
    fn pelvic_roll(gait: Gait) -> f32 {
        (Self::hip_list(gait, 1.0) - Self::hip_list(gait, -1.0)) / (2.0 * Physique::HIP_SPREAD)
    }
    const SPINE_ABSORBS: f32 = 0.25;

    /// How far this leg is through the little step a set keeper takes on the
    /// spot, 0..1 — see [`Joint::TOES_RATE`]. Half a cycle apart, so one boot
    /// is always down.
    ///
    /// Not while he is reaching for something: the save is what the dance is
    /// FOR, and a keeper going down to a ball at his feet is no longer
    /// deciding which way to go.
    /// **How ready his arms are**, 0..1 — the set posture, elbows bent and
    /// gloves up in front, rather than hanging at his sides.
    ///
    /// [`Gait::set`] alone was not enough: it is gated on the ball being
    /// near his goal, so a keeper stepping across with play further out did
    /// it with his arms swinging by his sides. **Straight, still arms beside
    /// a working pair of legs is most of what reads as an impaired gait.** A
    /// man who is moving sideways on purpose has his hands up, and nobody
    /// else on this pitch travels sideways at all.
    fn armed(gait: Gait) -> f32 {
        // ⚠ **…but not at a dead run.** [`Gait::set`] is the claim that the
        // ball is near his goal and it stays true while he covers ground —
        // which is right, and is why the legs stopped reading it — but a
        // keeper sprinting out to a through-ball with his forearms held up
        // in front of his chest is the same freeze one joint further up. The
        // band is [`Actors::SQUARE_UP`], where his shuffle ends and he is
        // genuinely running: one claim about one speed, and the same one his
        // hips already open on. A save has its own layer above this, so a
        // man arriving at the ball still puts his hands where it is.
        let running = Actors::ease(
            (gait.run * Actors::SPRINT - Actors::SQUARE_UP.0)
                / (Actors::SQUARE_UP.1 - Actors::SQUARE_UP.0),
        );
        gait.set
            .max(Self::sidling(gait) * Self::SIDLE_READY * gait.keeper)
            * (1.0 - running)
    }
    const SIDLE_READY: f32 = 0.60;

    /// **The set STANCE**, as against [`Gait::set`], which is the claim that
    /// the ball is near his goal.
    ///
    /// ⚠ **The two are not the same thing, and conflating them is what
    /// welded his legs on.** The hip and the knee take the set through
    /// [`Joint::held`], which SLERPS them onto a fixed angle at the weight
    /// it is given: a keeper who is 83% set has 17% of a stride left.
    /// Measured, a keeper walking at a metre a second with the ball inside
    /// his box swept his boot **0.9 cm** across a step he covered half a
    /// metre with — against 49.6 cm for the same walk with the set off. He
    /// was being slid across the grass with his legs held still, which is
    /// exactly the report: *"he moves without moving his legs"*. The old
    /// gate was `1 − speed/SPRINT`, which is 0.83 at a walk, and a keeper
    /// does nine tenths of his moving below a walk.
    ///
    /// A stance is something a man HOLDS. The instant he takes a step he is
    /// not in it any more — which is the gate [`Joint::on_his_toes`] has
    /// always had and the rest of the set should have had with it. What
    /// stays behind is his ARMS, which is [`Joint::armed`]: a keeper
    /// shuffling across his line has his gloves up and his legs working, and
    /// those are two different claims about him.
    fn crouched(gait: Gait) -> f32 {
        gait.set * (1.0 - Self::afoot(gait))
    }

    /// **How closed his hands are**, 0 spread flat and 1 a fist.
    ///
    /// One number for all ten fingers, layered in the same order the arm
    /// poses above are and for the same reason: a hand belongs to whatever
    /// the man is doing, and the last thing he started doing wins. It is a
    /// scalar rather than a quaternion, so the layers are a lerp instead of
    /// a slerp — but it is the same list, and it has to stay in the same
    /// order as the list at [`Limb::Shoulder`] or a keeper will catch a ball
    /// with the hands he was reaching with.
    fn grip(gait: Gait) -> f32 {
        let onto =
            |base: f32, value: f32, weight: f32| base + (value - base) * weight.clamp(0.0, 1.0);
        // Half curled standing, closing at a run — and a little different
        // per man, because two footballers do not carry their hands alike.
        let running = Self::HAND_REST
            + (Self::HAND_RUNNING - Self::HAND_REST) * gait.run
            + 0.07 * gait.signature;
        // Loose on his own head, or hanging.
        let slumped = onto(running, 0.36, gait.despair);
        // …curled over the crest of a hip, laid flat on a knee, open for a
        // clap. Each of them is a surface the hand is actually resting on,
        // and each wants a different one.
        let slumped = onto(slumped, Self::HAND_ON_HIP, gait.hands_on_hips);
        let slumped = onto(slumped, Self::HAND_FLAT, gait.doubled_over);
        let slumped = onto(slumped, Self::HAND_READY, gait.urging);
        let ready = onto(slumped, Self::HAND_READY, Self::armed(gait));
        // Behind the ball, or balled up behind a punch.
        let saving = onto(
            ready,
            Self::HAND_SAVING + (Self::HAND_FIST - Self::HAND_SAVING) * gait.parry,
            gait.save,
        );
        // Thrown out at full stretch — as open as they go, since the whole
        // object of the exercise is to be in two places at once.
        let out = onto(saving, Self::HAND_READY, gait.reach);
        // …and shut on a ball he has actually got, which beats both. This is
        // the layer the cradle is: [`Physique::CRADLE`] puts the ball in the
        // fork of his wrists and nothing until now closed a finger on it.
        let holding = onto(out, Self::HAND_HOLDING, gait.carry.max(gait.claimed));
        onto(
            holding,
            Self::HAND_GRASSED,
            gait.grounded * (1.0 - gait.carry.max(gait.claimed)),
        )
        .clamp(0.0, 1.0)
    }

    /// **And how far apart he has FANNED them**, 0 at rest and 1 wide.
    ///
    /// Held apart from the grip because they are not two ends of one axis: a
    /// hand can be open and neutral (walking back to his line) or open and
    /// spread (a shot coming at him), and only the second is a goalkeeper
    /// doing something. A `max` rather than a layered lerp — every one of
    /// these is the same gesture arrived at from a different direction, and
    /// the widest of them is the one his hands are in.
    fn fan(gait: Gait) -> f32 {
        Self::armed(gait)
            .max(gait.save * (1.0 - gait.parry))
            .max(gait.reach * (1.0 - gait.claimed))
            .clamp(0.0, 1.0)
    }

    /// **How far apart his gloves are through a clap**, 1 wide and 0
    /// together.
    ///
    /// Rectified rather than a plain sinusoid: a clap is a quick close and a
    /// return, and a pair of hands that spends half the cycle travelling
    /// evenly back out is a man conducting. [`Self::CLAP_RATE`] is a whole
    /// number on purpose — `idle` wraps at a full turn, and a fractional
    /// multiple of it steps at the wrap.
    fn clap(gait: Gait) -> f32 {
        1.0 - (gait.idle * Self::CLAP_RATE).sin().max(0.0)
    }

    /// **How far into the push off the floor he is**, 0 at both ends and 1
    /// in the middle. See [`Gait::kneeling`].
    fn kneeling(gait: Gait) -> f32 {
        gait.kneeling()
    }

    fn on_his_toes(gait: Gait, side: f32) -> f32 {
        // ⚠ And not while he is TAKING steps. The dance runs on the idle
        // clock and the stride runs on ground covered, so a keeper doing
        // both at once has two step rhythms in the same pair of legs — which
        // is not twice as alive, it is incoherent.
        let alive = Self::crouched(gait) * (1.0 - gait.save);
        if alive <= 1e-3 {
            return 0.0;
        }
        let offset = if side < 0.0 { PI } else { 0.0 };
        alive * (gait.idle * Self::TOES_RATE + offset).sin().max(0.0)
    }

    /// The angle a kicking joint holds at this point in the swing, from its
    /// three keys: top of the backswing, contact, end of the follow through.
    ///
    /// Piecewise rather than one curve because contact is not a smooth point —
    /// it is the instant a boot stops a moving ball, and the leg goes into it
    /// and out of it at quite different rates.
    fn through(keys: (f32, f32, f32), swing: f32) -> f32 {
        if swing < 0.0 {
            // Winding up: eased, so the leg gathers rather than snapping back.
            let back = Actors::ease(-swing);
            keys.1 + (keys.0 - keys.1) * back
        } else {
            keys.1 + (keys.2 - keys.1) * Actors::ease(swing)
        }
    }

    /// Fades a limb off the run cycle and onto the hold as a keeper gathers
    /// the ball. Short-circuited at zero because twenty-one players out of
    /// twenty-two are never holding anything, and a slerp per joint per frame
    /// for all of them is a cost with nothing to show for it.
    fn held(swinging: Quat, cradle: Quat, carry: f32) -> Quat {
        if carry <= 1e-3 {
            swinging
        } else {
            swinging.slerp(cradle, carry.min(1.0))
        }
    }
}

/// The whole figure, hung off the actor so it can leave the ground without
/// taking the actor's own marks with it.
///
/// Everything else in this rig moves a limb against a body that is standing on
/// the turf. A dive is the one thing that moves the body itself: the man goes
/// horizontal and airborne, and no arrangement of hips and knees expresses
/// that. Putting one node between the actor and the figure is what lets
/// [`crate::players::actors::Actors::animate`] topple and lift the lot in one
/// transform — while the contact shadow and the team ring, which stay
/// children of the actor, stay flat on the grass where they belong. A part of
/// one player worn in his own COMPLEXION: an ear, a forearm, a thigh.
/// Everything the shared skin ramp is picked for.
///
/// A marker rather than a lookup because these are repainted after the fact. A
/// real photograph of the man turns up while the match is running, and the tone
/// in it is the tone the rest of him should be — his neck has to be the
/// colour of his face. [`crate::players::portrait::Portraits::attach`] is what
/// asks.
#[derive(Component)]
pub struct Flesh {
    pub actor: Entity,
}

/// …and the cap of hair over the top of his head, for the same reason and by
/// the same route. A cap sits over the photograph, so a black one on a blond
/// man is the single most visible way a real face can still come out wrong.
#[derive(Component)]
pub struct Thatch {
    pub actor: Entity,
}

#[derive(Component)]
pub struct Carriage {
    /// The actor this figure belongs to; the dive is kept there.
    pub owner: Entity,
}

impl Carriage {
    /// Height of the point the body turns about, in metres — the hips.
    ///
    /// A diving keeper pivots around his middle: head one way, boots the
    /// other, weight staying over the spot the recording puts him on. Turning
    /// him about his feet instead would swing his whole body a metre sideways
    /// out of his own shadow.
    pub const PIVOT: f32 = Physique::HIP;

    /// And where those hips end up once he is all the way over, in metres.
    ///
    /// This is the number the dive was missing, and it is the difference
    /// between a save and a mannequin being spun on a pole. Rotating a body
    /// about a point fixed at [`Carriage::PIVOT`] means a keeper thrown flat
    /// is horizontal *at standing hip height*: shoulders at 1.2 m, boots
    /// half a metre in the air, and no part of him anywhere near the grass —
    /// for the whole 390–660 ms a recorded dive lasts, and then he lands
    /// still floating. A man lying on his side has his hips a hand's width
    /// off the turf, so the pivot has to travel down as the body goes over.
    ///
    /// Taken together with the recorded height this also gets the arc right
    /// for free. A big dive peaks at 0.5 m of recorded lift with the body
    /// most of the way over, so the hips land at `0.95 − 0.76·sin(tilt) + 0.5`
    /// ≈ 0.72 m — full stretch, most of a metre up, exactly the shape of the
    /// photograph — and the same expression walks him down to 0.19 m as the
    /// lift returns to zero, which is a keeper on the ground.
    ///
    /// ⚠ It is a HALF hip breadth, and it has to be measured as one. At 0.32
    /// — chosen while the body could only ever reach 79° and was propping its
    /// own head up on the residual 11° — a keeper lying on the turf floated a
    /// hand's width above it. The engine's `height` is the lift of the same
    /// hips (`MatchPlayer::leap` is documented in those terms), so this
    /// number is the one thing standing between a recorded 0 and the grass.
    const LYING: f32 = 0.19;

    /// **How far from upright this pair of angles leaves a figure**, as the
    /// SINE of the angle between its own up-axis and the world's.
    ///
    /// Off the composed rotation rather than off either Euler angle, so a dive
    /// that is half across the goal and half up the pitch settles as far as one
    /// that is all of either. Its own function because
    /// [`PlayerActor::lift`](crate::players::actors::PlayerActor) needs the
    /// same number to give the settle back as he gets up, and a formula written
    /// down in two places does not stay written down the same way.
    pub fn tilt(pitch: f32, roll: f32) -> f32 {
        let rotation = Quat::from_rotation_x(pitch) * Quat::from_rotation_z(roll);
        let upright = (rotation * Vec3::Y).y.clamp(-1.0, 1.0);
        (1.0 - upright * upright).max(0.0).sqrt()
    }

    /// …and how much of the drop that costs him, in metres: hips at
    /// [`Self::PIVOT`] standing and at [`Self::LYING`] flat out.
    pub const SETTLE: f32 = Self::PIVOT - Self::LYING;

    /// And where a man ON HIS KNEES carries those same hips, which is
    /// neither of the two.
    ///
    /// A thigh 20° off vertical ([`Joint::RISE_THIGH`]) puts the knee 0.43 m
    /// below the hips, and the knee is on the grass, so this is that plus
    /// the thickness of a leg. The settle cannot reach it from either end:
    /// at the angle a keeper kneels at it would have him a foot too low
    /// coming out of the sprawl and, once the recovery starts giving that
    /// back, a quarter of a metre too HIGH, with both boots hanging in
    /// mid-air. See [`PlayerActor::lift`](crate::players::actors::PlayerActor).
    pub const KNEELING: f32 = 0.50;

    /// The transform that tips a figure `pitch` radians over its toes and
    /// `roll` radians onto its side, `lift` metres off the turf, pivoting at
    /// the hips.
    ///
    /// Two axes because a keeper's dive mostly is not the poster one: across
    /// a recorded match, dives divide about evenly between those that travel
    /// further across the goal and those that travel further up the pitch —
    /// a man going down at a striker's feet rather than flying into a top
    /// corner. Roll alone drew every one of those side-on.
    ///
    /// A Bevy `Transform` is translate-rotate-scale, so the pivot cannot be
    /// expressed directly — it is folded into the translation instead, which
    /// is what the second term is: rotate about the origin, then put the hips
    /// back where they were.
    pub fn placed(pitch: f32, roll: f32, lift: f32) -> Transform {
        let rotation = Quat::from_rotation_x(pitch) * Quat::from_rotation_z(roll);
        let pivot = Vec3::Y * Self::PIVOT;
        let settle = Self::SETTLE * Self::tilt(pitch, roll);
        Transform::from_translation(pivot - rotation * pivot + Vec3::Y * (lift - settle))
            .with_rotation(rotation)
    }
}

/// Hangs one footballer's meshes off an actor entity.
pub struct Footballer;

impl Footballer {
    /// Builds the rig under `root`, which carries the player's position, facing
    /// and stride. Legs hang from the [`Carriage`] so the torso can lean without
    /// taking the feet with it; arms and head hang from the torso so that they
    /// do.
    pub fn assemble(
        commands: &mut Commands,
        root: Entity,
        parts: &BodyParts,
        outfit: &Outfit,
        keeper: bool,
    ) {
        let hips = Vec3::new(0.0, Physique::HIP, 0.0);
        let neck = Vec3::new(0.0, Physique::TORSO, 0.0);
        let elbow = Vec3::new(0.0, -Physique::UPPER_ARM, 0.0);
        let knee = Vec3::new(0.0, -Physique::THIGH, 0.0);
        let ankle = Physique::ANKLE;
        let wrist = Vec3::new(0.0, -Physique::FOREARM - Physique::WRIST_DROP, 0.0);

        let carriage = commands
            .spawn((
                Carriage { owner: root },
                Transform::default(),
                Visibility::default(),
            ))
            .id();
        commands.entity(root).add_child(carriage);

        commands.entity(carriage).with_children(|body| {
            body.spawn((
                Joint::new(root, Limb::Pelvis, 0.0, hips),
                Mesh3d(parts.pelvis.clone()),
                MeshMaterial3d(outfit.shorts.clone()),
                Transform::from_translation(hips),
            ));

            body.spawn((
                Joint::new(root, Limb::Torso, 0.0, hips),
                Mesh3d(parts.torso.clone()),
                MeshMaterial3d(outfit.shirt.clone()),
                Transform::from_translation(hips),
            ))
            .with_children(|torso| {
                // Both panels lie ON the shirt rather than in front of it, so
                // they take the torso's own transform and nothing else — see
                // [`Sculptor::decal`].
                if let Some(number) = outfit.number.clone() {
                    torso.spawn((
                        Mesh3d(parts.number.clone()),
                        MeshMaterial3d(number),
                        Transform::default(),
                    ));
                }
                if let Some(name) = outfit.name.clone() {
                    torso.spawn((
                        Mesh3d(parts.name.clone()),
                        MeshMaterial3d(name),
                        Transform::default(),
                    ));
                }
                torso.spawn((
                    Mesh3d(parts.collar.clone()),
                    MeshMaterial3d(outfit.trim.clone()),
                    Transform::default(),
                ));

                torso
                    .spawn((
                        Joint::new(root, Limb::Head, 0.0, neck),
                        Mesh3d(parts.head.clone()),
                        MeshMaterial3d(outfit.face.clone()),
                        Transform::from_translation(neck),
                    ))
                    .with_children(|head| {
                        if let Some(hair) = parts.hair[outfit.hair_style.index()].clone() {
                            head.spawn((
                                Mesh3d(hair),
                                MeshMaterial3d(outfit.hair.clone()),
                                Thatch { actor: root },
                                Transform::default(),
                            ));
                        }
                    });

                for side in [-1.0f32, 1.0] {
                    let shoulder =
                        Vec3::new(side * Physique::SHOULDER_SPREAD, Physique::SHOULDER, 0.0);
                    torso
                        .spawn((
                            Joint::new(root, Limb::Shoulder, side, shoulder),
                            Mesh3d(parts.upper_arm.clone()),
                            MeshMaterial3d(outfit.skin.clone()),
                            Flesh { actor: root },
                            Transform::from_translation(shoulder),
                        ))
                        .with_children(|arm| {
                            // A keeper wears long sleeves, which is half of
                            // what tells him apart from the twenty outfield
                            // players at any distance the strip does not.
                            arm.spawn((
                                Mesh3d(if keeper {
                                    parts.sleeve_long.clone()
                                } else {
                                    parts.sleeve.clone()
                                }),
                                MeshMaterial3d(outfit.shirt.clone()),
                                Transform::default(),
                            ));
                            if !keeper {
                                arm.spawn((
                                    Mesh3d(parts.cuff.clone()),
                                    MeshMaterial3d(outfit.trim.clone()),
                                    Transform::default(),
                                ));
                            }
                            arm.spawn((
                                Joint::new(root, Limb::Elbow, side, elbow),
                                Mesh3d(parts.forearm.clone()),
                                MeshMaterial3d(outfit.skin.clone()),
                                Flesh { actor: root },
                                Transform::from_translation(elbow),
                            ))
                            .with_children(|forearm| {
                                if keeper {
                                    forearm.spawn((
                                        Mesh3d(parts.sleeve_forearm.clone()),
                                        MeshMaterial3d(outfit.shirt.clone()),
                                        Transform::default(),
                                    ));
                                    forearm.spawn((
                                        Mesh3d(parts.cuff_forearm.clone()),
                                        MeshMaterial3d(outfit.trim.clone()),
                                        Transform::default(),
                                    ));
                                }
                                let mut wearing = forearm.spawn((
                                    Joint::new(root, Limb::Wrist, side, wrist),
                                    Mesh3d(if keeper {
                                        parts.glove.clone()
                                    } else {
                                        parts.hand[usize::from(side > 0.0)].clone()
                                    }),
                                    MeshMaterial3d(outfit.hands.clone()),
                                    Transform::from_translation(wrist),
                                ));
                                // A keeper's hands are GLOVES. Everything else
                                // marked here is repainted the colour of the
                                // man's own photograph, and a pair of gloves
                                // repainted flesh is not a pair of gloves.
                                if !keeper {
                                    wearing.insert(Flesh { actor: root });
                                }
                                wearing.with_children(|hand| {
                                    if !keeper {
                                        return;
                                    }
                                    // Every digit is a JOINT now, so the one
                                    // loop in `Actors::animate` that poses
                                    // the rig poses them too — the fingers
                                    // cost nothing here that the elbows do
                                    // not. The spawn transform carries the
                                    // rest position and the size; the angle
                                    // is [`Joint::pose`]'s.
                                    for (limb, at) in BodyParts::digits(side) {
                                        let thumb = matches!(limb, Limb::Thumb);
                                        let mut digit = hand.spawn((
                                            Joint::new(root, limb, side, at.translation),
                                            Mesh3d(if thumb {
                                                parts.thumb.clone()
                                            } else {
                                                parts.finger.clone()
                                            }),
                                            MeshMaterial3d(outfit.hands.clone()),
                                            at,
                                        ));
                                        let Limb::Finger(index) = limb else {
                                            continue;
                                        };
                                        digit.with_child((
                                            Joint::new(
                                                root,
                                                Limb::Knuckle(index),
                                                side,
                                                BodyParts::KNUCKLE_JOINT,
                                            ),
                                            Mesh3d(parts.fingertip.clone()),
                                            MeshMaterial3d(outfit.hands.clone()),
                                            Transform::from_translation(BodyParts::KNUCKLE_JOINT),
                                        ));
                                    }
                                });
                            });
                        });
                }
            });

            for side in [-1.0f32, 1.0] {
                let hip = Vec3::new(side * Physique::HIP_SPREAD, Physique::HIP, 0.0);
                body.spawn((
                    Joint::new(root, Limb::Hip, side, hip),
                    Mesh3d(parts.thigh.clone()),
                    MeshMaterial3d(outfit.skin.clone()),
                    Flesh { actor: root },
                    Transform::from_translation(hip),
                ))
                .with_children(|leg| {
                    leg.spawn((
                        Mesh3d(parts.shorts_leg.clone()),
                        MeshMaterial3d(outfit.shorts.clone()),
                        Transform::default(),
                    ));
                    leg.spawn((
                        Joint::new(root, Limb::Knee, side, knee),
                        Mesh3d(parts.shin.clone()),
                        MeshMaterial3d(outfit.socks.clone()),
                        Transform::from_translation(knee),
                    ))
                    .with_children(|shin| {
                        shin.spawn((
                            Mesh3d(parts.sock_top.clone()),
                            MeshMaterial3d(outfit.shorts.clone()),
                            Transform::default(),
                        ));
                        // The boot hangs off an ANKLE rather than off the
                        // shin: its mesh origin already sits where the
                        // ankle is (the sole is 38 mm below it), so the
                        // joint rotates the foot about the right point
                        // without moving the boot a millimetre at rest.
                        shin.spawn((
                            Joint::new(root, Limb::Ankle, side, ankle),
                            Mesh3d(parts.boot.clone()),
                            MeshMaterial3d(outfit.boots.clone()),
                            Transform::from_translation(ankle),
                        ));
                    });
                });
            }
        });
    }
}

/// Forward kinematics over the rig, so the poses above can be checked as
/// positions rather than as angles.
///
/// Every constant in a save is an angle at a joint, and an angle is impossible
/// to argue about — but "his boots are eleven centimetres under the turf" is
/// not. These walk the same offsets [`Footballer::assemble`] hangs the meshes
/// off and call the same [`Joint::pose`] the renderer calls, so a sign error
/// anywhere in the chain shows up here as a body part in the wrong place.
#[cfg(test)]
pub(crate) mod skeleton {
    use super::*;

    /// A player standing still, with nothing switched on — and a
    /// GOALKEEPER, since everything in this module that cares is his.
    ///
    /// The field list itself lives on `Gait::resting`, because the renderer
    /// needs one too: an actor carries the gait it was last posed with, and
    /// has to carry something before it has ever been posed.
    pub fn still() -> Gait {
        Gait {
            keeper: 1.0,
            ..Gait::resting()
        }
    }

    /// A keeper travelling across himself (`across` −1 to his left, +1 to
    /// his right) or backwards (`ahead` −1) at this much of a sprint, with
    /// the legs asked to carry `carry_ground` radians of hip swing.
    pub fn travelling(run: f32, across: f32, ahead: f32, carry_ground: f32) -> Gait {
        let mut gait = still();
        gait.run = run;
        gait.phase = 1.1;
        gait.course = Vec2::new(across, ahead).normalize_or(Vec2::Y);
        gait.carry_ground = carry_ground;
        gait
    }

    /// A save made on his feet, reaching `aim` (x across his body, y up and
    /// down, each −1..1).
    pub fn saving(aim: Vec2, parry: f32) -> Gait {
        let mut gait = still();
        gait.set = 1.0;
        gait.save = 1.0;
        gait.save_aim = aim;
        gait.parry = parry;
        gait
    }

    /// A man who has just conceded: `hands_to_head` 1 puts them on his
    /// head, 0 leaves his arms hanging.
    pub fn slumped(hands_to_head: f32) -> Gait {
        let mut gait = still();
        gait.despair = 1.0;
        gait.hands_to_head = hands_to_head;
        gait
    }

    /// …and the other two ways of taking it.
    pub fn on_his_hips() -> Gait {
        let mut gait = still();
        gait.despair = 1.0;
        gait.hands_on_hips = 1.0;
        gait
    }

    pub fn doubled_over() -> Gait {
        let mut gait = still();
        gait.despair = 1.0;
        gait.doubled_over = 1.0;
        gait
    }

    /// …and one who has just scored.
    pub fn cheering() -> Gait {
        let mut gait = still();
        gait.elation = 1.0;
        gait
    }

    /// A header at this point in the swing.
    pub fn nodding(swing: f32) -> Gait {
        let mut gait = still();
        gait.swing = swing;
        gait.header = 1.0;
        gait
    }

    /// And a throw-in.
    pub fn tossing(swing: f32) -> Gait {
        let mut gait = still();
        gait.swing = swing;
        gait.throw_in = 1.0;
        gait
    }

    /// Mid-stride at this much of a sprint.
    /// A man running at this share of a sprint.
    ///
    /// ⚠ **`carry_ground` is set from the same speed**, because the flight
    /// term is a bonus ON it now — see [`Joint::stepping`]. A fixture that
    /// left it at zero used to still produce a full stride, off the `run`
    /// claim alone; it now produces a man standing still with his legs
    /// straight, which is a fixture describing something that cannot happen
    /// rather than a bug in the rig.
    pub fn running(run: f32) -> Gait {
        let mut gait = still();
        gait.run = run;
        gait.phase = 1.1;
        gait.carry_ground = Actors::stride_of(7, run * Actors::SPRINT, Vec2::Y).1;
        gait
    }

    /// A right-footed kick at full power, at this point in the swing.
    pub fn kicking(swing: f32) -> Gait {
        let mut gait = still();
        gait.swing = swing;
        gait.power = 1.0;
        gait.foot = 1.0;
        gait
    }

    /// A keeper off his feet. `lead` is the side he went (−1 left, +1
    /// right, 0 straight forward at a striker's feet); `stretch` is how far
    /// through the extension he is; `reach` is the arms going out after it.
    pub fn diving(lead: f32, stretch: f32, reach: f32) -> Gait {
        let mut gait = still();
        gait.dive = 1.0;
        gait.stretch = stretch;
        gait.lead = lead;
        gait.reach = reach;
        gait
    }

    pub fn step(limb: Limb, side: f32, origin: Vec3, gait: Gait) -> Transform {
        let joint = Joint::new(Entity::PLACEHOLDER, limb, side, origin);
        Transform::from_translation(joint.place(gait)).with_rotation(joint.pose(gait))
    }

    /// The glove centre, in the figure's own space — that is, under the
    /// carriage, which is where [`Physique::CRADLE`] and [`Physique::CATCH`]
    /// are expressed too.
    pub fn glove(side: f32, gait: Gait) -> Vec3 {
        Physique::glove(side, gait)
    }

    /// The END of one digit, in the same space: `0`–`3` the fingers from the
    /// index outward, `4` the thumb.
    ///
    /// The whole question about a hand is where its fingertips are — spread
    /// wide across a shot, folded into the palm behind a punch, curled round
    /// a ball he has caught — and every one of those is a distance rather
    /// than an angle. Walks the same two joints [`Footballer::assemble`]
    /// hangs the meshes off, so a sign error in either shows up here as a
    /// fingertip in the wrong place.
    pub fn fingertip(side: f32, digit: usize, gait: Gait) -> Vec3 {
        /// The last ring of each mesh: where it actually ends.
        const TIP: Vec3 = Vec3::new(0.0, -0.042, 0.0);
        const THUMB: Vec3 = Vec3::new(0.0, -0.052, 0.0);
        let (limb, at) = BodyParts::digits(side)[digit];
        let hung = Physique::hand(side, gait)
            * step(limb, side, at.translation, gait).with_scale(at.scale);
        match limb {
            Limb::Finger(index) => (hung
                * step(Limb::Knuckle(index), side, BodyParts::KNUCKLE_JOINT, gait))
            .transform_point(TIP),
            _ => hung.transform_point(THUMB),
        }
    }

    /// Where the boot meets the grass, in the ANKLE.s own space — 38 mm
    /// below it, which is how the boot mesh is modelled. Lives here rather
    /// than on `Physique` because the renderer never needs it: it draws the
    /// boot, it does not ask where the bottom of it is.
    const SOLE: Vec3 = Vec3::new(0.0, -0.038, 0.0);

    /// **Every part of him that can end up under the turf**, named — the
    /// live [`Physique::underside`], which is the one the rig places him
    /// with, so a dump cannot report a burial the renderer does not have.
    pub fn landmarks(gait: Gait) -> Vec<(&'static str, Vec3)> {
        const NAMES: [&str; 9] = [
            "knee", "knee", "boot", "boot", "elbow", "elbow", "glove", "glove", "head",
        ];
        NAMES.into_iter().zip(Physique::underside(gait)).collect()
    }

    /// The sole of a boot, in the same space.
    pub fn boot(side: f32, gait: Gait) -> Vec3 {
        let hip = Vec3::new(side * Physique::HIP_SPREAD, Physique::HIP, 0.0);
        let knee = Vec3::new(0.0, -Physique::THIGH, 0.0);
        // Through the ANKLE — the foot rolls now, so a sole worked out by
        // adding a constant to the shin is the sole of a different figure.
        (step(Limb::Hip, side, hip, gait)
            * step(Limb::Knee, side, knee, gait)
            * step(Limb::Ankle, side, Physique::ANKLE, gait))
        .transform_point(SOLE)
    }

    /// And the crown of the head, off the skull's own last ring rather than
    /// off a number written down twice.
    pub fn crown(gait: Gait) -> Vec3 {
        let hips = Vec3::new(0.0, Physique::HIP, 0.0);
        let neck = Vec3::new(0.0, Physique::TORSO, 0.0);
        let top = BodyParts::SKULL[BodyParts::SKULL.len() - 1];
        (step(Limb::Torso, 0.0, hips, gait) * step(Limb::Head, 0.0, neck, gait))
            .transform_point(Vec3::new(0.0, top.y, top.offset))
    }
}

/// Draws the assembled figure to a buffer of pixels, with no GPU, no browser
/// and no camera to fight.
///
/// The skeleton tests above check the rig as POSITIONS, which is unarguable
/// and catches a limb through the turf — but a question like "does this still
/// read as a person" has no assertion, and the only tool for it was building
/// 25 MB of WebAssembly and driving a headless browser's camera with synthetic
/// wheel events. That loop takes ten minutes, needs the frame rate to
/// cooperate, and gets less reliable the more geometry there is to draw, which
/// is precisely backwards.
///
/// So: a scanline rasteriser over the same meshes the renderer gets, shaded
/// with the same interpolated normals and the same light. It is a few hundred
/// lines and it answers in a second.
#[cfg(test)]
pub(crate) mod preview {
    use super::skeleton;
    use super::*;
    use crate::scene::pitch::Pitch;
    use bevy::mesh::VertexAttributeValues;

    /// A frame buffer with a depth buffer behind it.
    pub struct Canvas {
        width: usize,
        height: usize,
        colour: Vec<Vec3>,
        depth: Vec<f32>,
    }

    impl Canvas {
        /// Background, chosen to be nothing a footballer is made of.
        const GROUND: Vec3 = Vec3::new(0.36, 0.52, 0.30);
        /// How much light reaches a surface facing away from the sun. The
        /// scene itself pairs one directional light with a generous ambient
        /// (see `Pitch::spawn`), and a preview that skipped it would draw
        /// every shaded side black and report a much worse model than ships.
        const AMBIENT: f32 = 0.45;

        pub fn new(width: usize, height: usize) -> Self {
            Canvas {
                width,
                height,
                colour: vec![Self::GROUND; width * height],
                depth: vec![f32::MAX; width * height],
            }
        }

        /// One triangle in a single colour — the flat-tinted case, which is
        /// every part of a footballer but his face.
        fn triangle(&mut self, corners: [Vec3; 3], shades: [f32; 3], tint: Vec3) {
            self.shaded(corners, shades, [tint; 3]);
        }

        /// The same, with the colour coming out of a texture: three corner
        /// `uv`s interpolated across the triangle and sampled per pixel.
        fn textured(
            &mut self,
            corners: [Vec3; 3],
            shades: [f32; 3],
            uvs: [[f32; 2]; 3],
            sample: &impl Fn([f32; 2]) -> Vec3,
        ) {
            self.scan(corners, shades, &|weights| {
                sample([
                    uvs[0][0] * weights.x + uvs[1][0] * weights.y + uvs[2][0] * weights.z,
                    uvs[0][1] * weights.x + uvs[1][1] * weights.y + uvs[2][1] * weights.z,
                ])
            });
        }

        /// One triangle, already in screen space: `x`/`y` in pixels, `z` into
        /// the screen, with a shade AND a colour per corner.
        fn shaded(&mut self, corners: [Vec3; 3], shades: [f32; 3], tints: [Vec3; 3]) {
            self.scan(corners, shades, &|weights: Vec3| {
                tints[0] * weights.x + tints[1] * weights.y + tints[2] * weights.z
            });
        }

        /// The scan itself, with whatever decides a pixel's colour handed in
        /// as the barycentric weights it is decided from.
        fn scan(&mut self, corners: [Vec3; 3], shades: [f32; 3], tint: &dyn Fn(Vec3) -> Vec3) {
            let area = (corners[1].x - corners[0].x) * (corners[2].y - corners[0].y)
                - (corners[2].x - corners[0].x) * (corners[1].y - corners[0].y);
            // Back faces are not drawn, exactly as the renderer does not draw
            // them — the inside of a torso is not part of the model.
            if area <= 1e-6 {
                return;
            }
            let low = |pick: fn(&Vec3) -> f32| {
                corners.iter().map(pick).fold(f32::MAX, f32::min).floor() as i32
            };
            let high = |pick: fn(&Vec3) -> f32| {
                corners.iter().map(pick).fold(f32::MIN, f32::max).ceil() as i32
            };
            let left = low(|corner| corner.x).max(0);
            let right = high(|corner| corner.x).min(self.width as i32 - 1);
            let top = low(|corner| corner.y).max(0);
            let bottom = high(|corner| corner.y).min(self.height as i32 - 1);

            for row in top..=bottom {
                for column in left..=right {
                    let at = Vec2::new(column as f32 + 0.5, row as f32 + 0.5);
                    let edge = |from: Vec3, to: Vec3| {
                        (to.x - from.x) * (at.y - from.y) - (at.x - from.x) * (to.y - from.y)
                    };
                    let weights = Vec3::new(
                        edge(corners[1], corners[2]),
                        edge(corners[2], corners[0]),
                        edge(corners[0], corners[1]),
                    ) / area;
                    if weights.min_element() < 0.0 {
                        continue;
                    }
                    let depth = weights.x * corners[0].z
                        + weights.y * corners[1].z
                        + weights.z * corners[2].z;
                    let index = row as usize * self.width + column as usize;
                    if depth >= self.depth[index] {
                        continue;
                    }
                    let shade =
                        weights.x * shades[0] + weights.y * shades[1] + weights.z * shades[2];
                    self.depth[index] = depth;
                    self.colour[index] = tint(weights) * shade;
                }
            }
        }

        pub fn pixels(&self) -> Vec<u8> {
            let mut out = Vec::with_capacity(self.width * self.height * 4);
            for colour in &self.colour {
                out.extend_from_slice(&[
                    (colour.x.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
                    (colour.y.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
                    (colour.z.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
                    255,
                ]);
            }
            out
        }
    }

    /// The camera: orthographic, turned `bearing` radians round the figure and
    /// framed on a metre band from the turf up.
    pub struct Lens {
        pub bearing: f32,
        pub bottom: f32,
        pub top: f32,
    }

    impl Lens {
        fn view(&self, canvas: &Canvas) -> Mat4 {
            let scale = canvas.height as f32 / (self.top - self.bottom);
            Mat4::from_translation(Vec3::new(
                canvas.width as f32 * 0.5,
                canvas.height as f32 + self.bottom * scale,
                0.0,
            )) * Mat4::from_scale(Vec3::new(scale, -scale, scale))
                * Mat4::from_rotation_y(self.bearing)
        }
    }

    /// A strip to preview in, chosen so every piece is a different value as
    /// well as a different hue — this is a shading test as much as a shape one.
    const SHIRT: Vec3 = Vec3::new(0.86, 0.78, 0.10);
    const SHORTS: Vec3 = Vec3::new(0.14, 0.15, 0.19);
    const TRIM: Vec3 = Vec3::new(0.20, 0.21, 0.26);
    const SKIN: Vec3 = Vec3::new(0.78, 0.60, 0.46);
    const HAIR: Vec3 = Vec3::new(0.22, 0.15, 0.10);
    const BOOTS: Vec3 = Vec3::new(0.92, 0.93, 0.95);

    /// One part with a PICTURE on it rather than a flat tint: the head
    /// wearing the face sheet that goes onto it.
    ///
    /// The only part of a footballer that is textured at all, and the one
    /// whose texture cannot be reviewed as a texture — a face sheet laid out
    /// flat is a smear that says nothing about where the eyes end up on a
    /// skull. See `portrait`'s dump, which is the caller.
    pub fn wearing(
        canvas: &mut Canvas,
        lens: &Lens,
        meshes: &Assets<Mesh>,
        parts: &BodyParts,
        at: Transform,
        sheet: (u32, u32, &[u8]),
        cap: bool,
    ) {
        // The skull, wearing the sheet…
        sheeted(canvas, lens, meshes, &parts.head, at, sheet);
        // …and the cap of hair over it, when he is wearing one. A player with
        // a real picture is not: his own hair is in the picture, so
        // `Portraits::attach` hides the cap and paints the crown instead.
        if let Some(hair) = parts.hair[2].clone().filter(|_| cap) {
            part(canvas, lens, meshes, &hair, at, HAIR);
        }
    }

    /// One mesh, drawn with a texture on it.
    fn sheeted(
        canvas: &mut Canvas,
        lens: &Lens,
        meshes: &Assets<Mesh>,
        handle: &Handle<Mesh>,
        at: Transform,
        sheet: (u32, u32, &[u8]),
    ) {
        let Some(mesh) = meshes.get(handle) else {
            return;
        };
        let (
            Some(VertexAttributeValues::Float32x3(positions)),
            Some(VertexAttributeValues::Float32x3(normals)),
            Some(VertexAttributeValues::Float32x2(uvs)),
            Some(indices),
        ) = (
            mesh.attribute(Mesh::ATTRIBUTE_POSITION),
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL),
            mesh.attribute(Mesh::ATTRIBUTE_UV_0),
            mesh.indices()
                .map(|values| values.iter().collect::<Vec<_>>()),
        )
        else {
            return;
        };

        let model = at.to_matrix();
        let view = lens.view(canvas) * model;
        let sun = -Pitch::SUN.normalize();
        let screen: Vec<Vec3> = positions
            .iter()
            .map(|point| view.transform_point3(Vec3::from(*point)))
            .collect();
        let shade: Vec<f32> = normals
            .iter()
            .map(|normal| {
                let world = model
                    .transform_vector3(Vec3::from(*normal))
                    .normalize_or_zero();
                Canvas::AMBIENT + (1.0 - Canvas::AMBIENT) * world.dot(sun).max(0.0)
            })
            .collect();

        let (width, height, pixels) = sheet;
        let sample = |uv: [f32; 2]| {
            let x = ((uv[0].rem_euclid(1.0)) * width as f32) as u32;
            let y = ((uv[1].clamp(0.0, 1.0)) * height as f32).min(height as f32 - 1.0) as u32;
            let at = ((y.min(height - 1) * width + x.min(width - 1)) * 4) as usize;
            Vec3::new(
                pixels[at] as f32,
                pixels[at + 1] as f32,
                pixels[at + 2] as f32,
            ) / 255.0
        };

        for triangle in indices.chunks_exact(3) {
            // Sampled per PIXEL rather than per corner. Sampling the three
            // corners and interpolating between them was cheaper and it lied:
            // it made the head as detailed as the mesh is dense, so a sheet
            // with four times the texels came out of the preview looking
            // exactly as soft as the one before it. What the renderer does is
            // this.
            canvas.textured(
                [
                    screen[triangle[0]],
                    screen[triangle[1]],
                    screen[triangle[2]],
                ],
                [shade[triangle[0]], shade[triangle[1]], shade[triangle[2]]],
                [uvs[triangle[0]], uvs[triangle[1]], uvs[triangle[2]]],
                &sample,
            );
        }
    }

    /// Draws every part of one footballer, posed by `gait`.
    ///
    /// Walks the same offsets [`Footballer::assemble`] hangs the meshes off.
    /// It is a second copy of that hierarchy and there is no way round it —
    /// `assemble` writes into an ECS a unit test has no world for — so a part
    /// added there and not here simply will not appear in a preview.
    pub fn figure(
        canvas: &mut Canvas,
        lens: &Lens,
        meshes: &Assets<Mesh>,
        parts: &BodyParts,
        gait: Gait,
    ) {
        posed(
            canvas,
            lens,
            meshes,
            parts,
            gait,
            Transform::IDENTITY,
            false,
        );
    }

    /// …and the same figure under a [`Carriage`], which is the only way to
    /// preview a goalkeeper.
    ///
    /// The topple and the lift are not joints — they are a transform on the
    /// whole body, applied in `Actors::carry_body` — so a dive drawn through
    /// [`figure`] is an upright man doing something odd with his arms. Every
    /// pose that puts a player on the floor has to come through here.
    pub fn posed(
        canvas: &mut Canvas,
        lens: &Lens,
        meshes: &Assets<Mesh>,
        parts: &BodyParts,
        gait: Gait,
        carriage: Transform,
        keeper: bool,
    ) {
        let hips = Vec3::new(0.0, Physique::HIP, 0.0);
        let neck = Vec3::new(0.0, Physique::TORSO, 0.0);
        let elbow = Vec3::new(0.0, -Physique::UPPER_ARM, 0.0);
        let knee = Vec3::new(0.0, -Physique::THIGH, 0.0);
        let wrist = Vec3::new(0.0, -Physique::FOREARM - Physique::WRIST_DROP, 0.0);
        let mut draw = |handle: &Handle<Mesh>, at: Transform, tint: Vec3| {
            part(canvas, lens, meshes, handle, carriage * at, tint);
        };

        let seat = skeleton::step(Limb::Pelvis, 0.0, hips, gait);
        draw(&parts.pelvis, seat, SHORTS);

        let torso = skeleton::step(Limb::Torso, 0.0, hips, gait);
        draw(&parts.torso, torso, SHIRT);
        draw(&parts.collar, torso, TRIM);

        let head = torso * skeleton::step(Limb::Head, 0.0, neck, gait);
        draw(&parts.head, head, SKIN);
        if let Some(hair) = parts.hair[2].clone() {
            draw(&hair, head, HAIR);
        }
        for side in [-1.0f32, 1.0] {
            let shoulder = Vec3::new(side * Physique::SHOULDER_SPREAD, Physique::SHOULDER, 0.0);
            let arm = torso * skeleton::step(Limb::Shoulder, side, shoulder, gait);
            draw(&parts.upper_arm, arm, SKIN);
            if keeper {
                draw(&parts.sleeve_long, arm, SHIRT);
            } else {
                draw(&parts.sleeve, arm, SHIRT);
                draw(&parts.cuff, arm, TRIM);
            }

            let fore = arm * skeleton::step(Limb::Elbow, side, elbow, gait);
            draw(&parts.forearm, fore, SKIN);
            if keeper {
                draw(&parts.sleeve_forearm, fore, SHIRT);
                draw(&parts.cuff_forearm, fore, TRIM);
            }
            let hand = fore * skeleton::step(Limb::Wrist, side, wrist, gait);
            draw(
                if keeper {
                    &parts.glove
                } else {
                    &parts.hand[usize::from(side > 0.0)]
                },
                hand,
                if keeper { TRIM } else { SKIN },
            );
            if keeper {
                for (limb, at) in BodyParts::digits(side) {
                    // The spawn transform carries the rest position and the
                    // size; the angle comes off the same `Joint::pose` the
                    // renderer calls. Scale is the one thing `step` cannot
                    // return, because the joint loop never writes it.
                    let digit = hand
                        * skeleton::step(limb, side, at.translation, gait).with_scale(at.scale);
                    let Limb::Finger(index) = limb else {
                        draw(&parts.thumb, digit, TRIM);
                        continue;
                    };
                    draw(&parts.finger, digit, TRIM);
                    draw(
                        &parts.fingertip,
                        digit
                            * skeleton::step(
                                Limb::Knuckle(index),
                                side,
                                BodyParts::KNUCKLE_JOINT,
                                gait,
                            ),
                        TRIM,
                    );
                }
            }

            let hip = Vec3::new(side * Physique::HIP_SPREAD, Physique::HIP, 0.0);
            let leg = skeleton::step(Limb::Hip, side, hip, gait);
            draw(&parts.thigh, leg, SKIN);
            draw(&parts.shorts_leg, leg, SHORTS);

            let lower = leg * skeleton::step(Limb::Knee, side, knee, gait);
            draw(&parts.shin, lower, SHORTS);
            draw(&parts.sock_top, lower, TRIM);
            let foot = lower * skeleton::step(Limb::Ankle, side, Physique::ANKLE, gait);
            draw(&parts.boot, foot, BOOTS);
        }
    }

    /// Shades and projects one mesh at one transform.
    fn part(
        canvas: &mut Canvas,
        lens: &Lens,
        meshes: &Assets<Mesh>,
        handle: &Handle<Mesh>,
        at: Transform,
        tint: Vec3,
    ) {
        let Some(mesh) = meshes.get(handle) else {
            return;
        };
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            return;
        };
        let Some(VertexAttributeValues::Float32x3(normals)) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            return;
        };
        let Some(indices) = mesh
            .indices()
            .map(|values| values.iter().collect::<Vec<_>>())
        else {
            return;
        };

        let model = at.to_matrix();
        let view = lens.view(canvas) * model;
        // The sun is in WORLD space and stays there however the preview's
        // camera is turned, which is the whole point of shading against it.
        let sun = -Pitch::SUN.normalize();

        let screen: Vec<Vec3> = positions
            .iter()
            .map(|point| view.transform_point3(Vec3::from(*point)))
            .collect();
        let shade: Vec<f32> = normals
            .iter()
            .map(|normal| {
                let world = model
                    .transform_vector3(Vec3::from(*normal))
                    .normalize_or_zero();
                Canvas::AMBIENT + (1.0 - Canvas::AMBIENT) * world.dot(sun).max(0.0)
            })
            .collect();

        for triangle in indices.chunks_exact(3) {
            canvas.triangle(
                [
                    screen[triangle[0]],
                    screen[triangle[1]],
                    screen[triangle[2]],
                ],
                [shade[triangle[0]], shade[triangle[1]], shade[triangle[2]]],
                tint,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::skeleton::step as step_of;
    use super::skeleton::*;
    use super::*;
    use crate::players::kit::Complexion;

    /// **A walking goalkeeper takes steps, however set he is.**
    ///
    /// The single largest thing wrong with this rig, and it hid for months
    /// behind a signal that reads correctly at both ends. `Joint::held`
    /// SLERPS the hip and the knee onto the set's fixed angles at whatever
    /// weight the set is given, and the set used to be gated on
    /// `1 − speed/SPRINT` — which is 0.83 at a metre a second. Measured, a
    /// keeper walking with the ball in his box swept his boot **0.9 cm**
    /// across a step he covered half a metre with, against 49.6 cm for the
    /// same walk with the set off. He was being slid across the grass with
    /// his legs held still, which is exactly the report: *"he often moves
    /// without moving his legs"*.
    ///
    /// A stance is something a man holds; the instant he takes a step he is
    /// not in it. See [`Joint::crouched`].
    #[test]
    fn the_set_does_not_weld_his_legs_on() {
        let sweep = |set: f32| {
            let (mut low, mut high) = (f32::MAX, f32::MIN);
            for step in 0..120 {
                let mut gait = travelling(1.0 / Actors::SPRINT, 0.0, 1.0, 0.24);
                gait.set = set;
                gait.phase = step as f32 * TAU / 120.0;
                let along = boot(1.0, gait).z;
                low = low.min(along);
                high = high.max(along);
            }
            high - low
        };
        let (loose, ready) = (sweep(0.0), sweep(1.0));
        assert!(
            ready > loose * 0.85,
            "set, his boot sweeps {ready:.3} m against {loose:.3} m with his arms down"
        );
    }

    /// **…and his gloves answer the step he is taking.**
    ///
    /// The same fault one line away: the set arms are reached through
    /// `Joint::held` too, so a keeper with his hands up had them welded
    /// there while his legs worked underneath him. Rendered eight phases of
    /// a side-step side by side, the entire upper body was pixel-identical
    /// in all eight — which is what an impaired gait looks like, and it is
    /// how it was reported.
    ///
    /// ⚠ Measured on ALL THREE axes. With his forearms up in front of him
    /// the arm points forward, so what a step does to a glove is mostly move
    /// it UP AND DOWN — the trap [`Joint::SAVE_ACROSS`] documents about
    /// rolling an arm held out in front, and reading one axis reported a
    /// moving hand as a still one.
    #[test]
    fn a_keeper_with_his_gloves_up_still_answers_the_step() {
        for (name, across, ahead) in [("a walk", 0.0, 1.0), ("a side-step", 1.0, 0.0)] {
            let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
            for step in 0..120 {
                let speed = 1.2;
                let course = Vec2::new(across, ahead);
                let open = Actors::opening(speed, course, true);
                let underfoot = Actors::underfoot(course, open);
                let mut gait = travelling(
                    speed / Actors::SPRINT,
                    underfoot.x,
                    underfoot.y,
                    Actors::stride_of(7, speed, underfoot).1,
                );
                gait.open = open;
                gait.set = 1.0;
                gait.phase = step as f32 * TAU / 120.0;
                let hand = glove(1.0, gait).to_array();
                for axis in 0..3 {
                    low[axis] = low[axis].min(hand[axis]);
                    high[axis] = high[axis].max(hand[axis]);
                }
            }
            let travelled = (0..3).map(|a| high[a] - low[a]).fold(0.0, f32::max);
            assert!(
                travelled > 0.05,
                "through {name} with his gloves up a hand moves {:.3} m",
                travelled
            );
        }
    }

    /// **A keeper going UP to a ball stands up to do it.**
    ///
    /// The save's only leg term was [`Joint::stooping`], which is zero for
    /// anything above his chest, so a keeper reaching over his own head was
    /// drawn holding the set crouch with his knees bent — the one posture a
    /// man extending upward is certainly not in, and most of *"he has none
    /// of the spring of a real goalkeeper"*. It matters more than the dive
    /// does: 84% of the balls that reach a keeper at pace reach one who
    /// never leaves the ground.
    #[test]
    fn a_high_save_stands_him_up() {
        let low = crown(saving(Vec2::new(0.0, -1.0), 0.0)).y;
        let level = crown(saving(Vec2::ZERO, 0.0)).y;
        let high = crown(saving(Vec2::new(0.0, 1.0), 0.0)).y;
        assert!(
            high > level + 0.07,
            "reaching over his head only gains him {:.3} m",
            high - level
        );
        assert!(
            level > low,
            "going down to his boots does not drop him ({low:.3} against {level:.3})"
        );
        // …and the gloves have to go with him, not just the head.
        let reach = glove(1.0, saving(Vec2::new(0.0, 1.0), 0.0)).y;
        assert!(
            reach > Physique::STATURE,
            "his gloves reach {reach:.2} m for a ball over the bar"
        );
    }

    /// **His feet point OUT, not in, and they do it whichever way he is
    /// going.**
    ///
    /// [`Joint::TOE_OUT`] is scaled by `course.x`, so it only ever applied
    /// to a side-step — and an outfielder travels forwards on 93% of the
    /// frames he moves in, which left every boot on the pitch aimed dead
    /// along its own run for a whole match. A yaw is invisible from the
    /// side, which is the only bearing the stride dumps render from, so it
    /// has to be a measurement.
    ///
    /// The sign is the half worth pinning: two feet splayed the same way is
    /// a man walking sideways.
    #[test]
    fn a_runner_toes_out() {
        /// A point out along the boot from the ankle, past the sole.
        const TOE: Vec3 = Vec3::new(0.0, -0.038, 0.12);
        let toed = |side: f32, toes: f32| {
            let mut gait = still();
            gait.keeper = 0.0;
            gait.toes = toes;
            gait.run = 0.8;
            gait.carry_ground = 0.5;
            gait.phase = FRAC_PI_2;
            let hip = Vec3::new(side * Physique::HIP_SPREAD, Physique::HIP, 0.0);
            let ankle = step(Limb::Hip, side, hip, gait)
                * step(
                    Limb::Knee,
                    side,
                    Vec3::new(0.0, -Physique::THIGH, 0.0),
                    gait,
                )
                * step(Limb::Ankle, side, Physique::ANKLE, gait);
            let toe = ankle.transform_point(TOE);
            let heel = ankle.transform_point(Vec3::new(0.0, -0.038, 0.0));
            // How far out of the sagittal plane the toe sits relative to the
            // heel, positive OUTWARD on either foot.
            (toe.x - heel.x) * side
        };
        for side in [-1.0f32, 1.0] {
            let straight = toed(side, 0.0);
            let splayed = toed(side, 0.30);
            assert!(
                straight.abs() < 0.01,
                "a man with no toe-out has his {} boot {straight:+.3} m off square",
                if side < 0.0 { "left" } else { "right" }
            );
            assert!(
                splayed > 0.02,
                "his {} boot points {splayed:+.3} m — inward, or not at all",
                if side < 0.0 { "left" } else { "right" }
            );
        }
    }

    /// **No two players run alike, and not along one axis either.**
    ///
    /// `no_two_players_run_alike` already asserts that two ids differ. This
    /// asserts the shape of the difference: the four traits a run cycle is
    /// built from have to be INDEPENDENT, or the squad is two kinds of
    /// runner drawn eleven times each rather than sixteen. Every one of
    /// these used to come off `Complexion::carriage`.
    #[test]
    fn a_squad_varies_along_more_than_one_axis() {
        let ids: Vec<u32> = (100..111).chain(200..211).collect();
        let traits: [(&str, fn(u32) -> f32); 4] = [
            ("carriage", Complexion::carriage),
            ("spring", Complexion::spring),
            ("elbows", Complexion::elbows),
            ("lean", Complexion::lean),
        ];
        for (first, (left, draw)) in traits.iter().enumerate() {
            for (right, other) in traits.iter().skip(first + 1) {
                let xs: Vec<f32> = ids.iter().map(|id| draw(*id)).collect();
                let ys: Vec<f32> = ids.iter().map(|id| other(*id)).collect();
                let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
                let (mx, my) = (mean(&xs), mean(&ys));
                let cov: f32 = xs
                    .iter()
                    .zip(&ys)
                    .map(|(x, y)| (x - mx) * (y - my))
                    .sum::<f32>();
                let spread = |v: &[f32], m: f32| v.iter().map(|x| (x - m) * (x - m)).sum::<f32>();
                let correlation = cov / (spread(&xs, mx) * spread(&ys, my)).sqrt();
                assert!(
                    correlation.abs() < 0.5,
                    "{left} and {right} move together across the squad ({correlation:+.2})"
                );
            }
        }
    }
    use crate::players::actors::Actors;
    /// **How much of him moves through one cycle of each gait he actually
    /// uses**, in centimetres and degrees.
    ///
    /// The dumps render a cycle and the eye picks out what is wrong with it;
    /// this puts a number on the same picture, which is the only way to tell
    /// "the arms barely move" from "the arms do not move at all" and the only
    /// way to know whether a change did anything. Every column is a
    /// peak-to-peak excursion across a whole cycle, so a figure that holds a
    /// pose reads zero however contorted the pose is.
    ///
    /// ```text
    /// cargo test --lib measure_locomotion -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "prints; run by hand when the gait changes"]
    fn measure_locomotion() {
        const STEPS: usize = 48;
        println!(
            "  {:<22} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
            "gait", "boot cm", "lift cm", "glove cm", "crown cm", "chest °", "hips °"
        );
        for (name, speed, course) in [
            ("walk forward 1.0", 1.0, Vec2::new(0.0, 1.0)),
            ("walk forward 1.4", 1.4, Vec2::new(0.0, 1.0)),
            ("jog forward 3.0", 3.0, Vec2::new(0.0, 1.0)),
            ("sprint 6.0", 6.0, Vec2::new(0.0, 1.0)),
            ("side-step 1.0", 1.0, Vec2::new(1.0, 0.0)),
            ("side-step 1.4", 1.4, Vec2::new(1.0, 0.0)),
            ("side-step 2.2", 2.2, Vec2::new(1.0, 0.0)),
            ("backpedal 1.4", 1.4, Vec2::new(0.0, -1.0)),
            ("backpedal 2.5", 2.5, Vec2::new(0.0, -1.0)),
        ] {
            // The course a keeper is actually left with at this speed: his
            // legs turn onto his run as he gets going, so a pure side-step at
            // four metres a second is a gait nobody has. See `Actors::opening`.
            let open = Actors::opening(speed, course, true);
            let underfoot = Actors::underfoot(course, open);
            let mut gait = travelling(
                (speed / Actors::SPRINT).clamp(0.0, 1.0),
                underfoot.x,
                underfoot.y,
                Actors::stride_of(7, speed, underfoot).1,
            );
            gait.open = open;
            gait.set = 1.0;
            let mut low = [f32::MAX; 11];
            let mut high = [f32::MIN; 11];
            for step in 0..STEPS {
                gait.phase = step as f32 * TAU / STEPS as f32;
                gait.idle = (gait.phase * 0.5).rem_euclid(TAU);
                let torso = step_of(Limb::Torso, 0.0, Vec3::new(0.0, Physique::HIP, 0.0), gait);
                let pelvis = step_of(Limb::Pelvis, 0.0, Vec3::new(0.0, Physique::HIP, 0.0), gait);
                let boot = boot(1.0, gait);
                // ⚠ The glove is measured on ALL THREE axes and not on the
                // one he is travelling along. With his forearms up in front
                // of him the arm points forward, so what the step does to a
                // glove is mostly move it UP AND DOWN — the same trap
                // [`Joint::SAVE_ACROSS`] documents about rolling an arm that
                // is held out in front, and reading `z` alone reported a
                // moving hand as a still one.
                let hand = glove(1.0, gait);
                let seen = [
                    // The boot, on whichever axis he is travelling along.
                    boot.z * underfoot.y.abs() + boot.x * underfoot.x.abs(),
                    boot.y,
                    hand.x,
                    hand.y,
                    hand.z,
                    crown(gait).y,
                    torso.rotation.to_euler(EulerRot::YXZ).0,
                    torso.rotation.to_euler(EulerRot::YXZ).1,
                    torso.rotation.to_euler(EulerRot::YXZ).2,
                    pelvis.rotation.to_euler(EulerRot::YXZ).0,
                    pelvis.rotation.to_euler(EulerRot::YXZ).2,
                ];
                for (index, value) in seen.into_iter().enumerate() {
                    low[index] = low[index].min(value);
                    high[index] = high[index].max(value);
                }
            }
            let span = |index: usize| high[index] - low[index];
            println!(
                "  {name:<22} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1}",
                span(0) * 100.0,
                span(1) * 100.0,
                span(2).max(span(3)).max(span(4)) * 100.0,
                span(5) * 100.0,
                span(6).max(span(7)).max(span(8)).to_degrees(),
                span(9).max(span(10)).to_degrees(),
            );
        }
    }

    /// A keeper who has gathered the ball holds it where the viewer draws it.
    ///
    /// [`Physique::CRADLE`] is not a free choice — it is worked down the arm
    /// from the two cradle angles, and the ball is drawn there rather than at
    /// its recorded position. If the arms and the constant ever part company,
    /// the gloves close on empty air.
    #[test]
    fn the_cradle_is_where_the_gloves_are() {
        let mut gait = still();
        gait.carry = 1.0;
        let between = (glove(-1.0, gait) + glove(1.0, gait)) * 0.5;
        assert!(
            between.distance(Physique::CRADLE) < 0.16,
            "ball drawn at {:?}, gloves meet at {between:?}",
            Physique::CRADLE
        );
    }

    /// And the same for a ball claimed at full stretch, which is drawn at
    /// [`Physique::catch`] instead — out at the end of the arms rather than
    /// against a chest that is nowhere near it.
    #[test]
    fn the_catch_is_where_the_gloves_are() {
        let mut gait = still();
        gait.reach = 1.0;
        gait.stretch = 1.0;
        gait.dive = 1.0;
        gait.claimed = 1.0;
        for lead in [-1.0f32, 0.0, 1.0] {
            gait.lead = lead;
            let between = (glove(-1.0, gait) + glove(1.0, gait)) * 0.5;
            let drawn = Physique::catch(lead);
            assert!(
                between.distance(drawn) < 0.13,
                "lead {lead}: ball drawn at {drawn:?}, gloves meet at {between:?}"
            );
            // Both hands are on it, not half a metre either side of it.
            assert!(
                glove(-1.0, gait).distance(glove(1.0, gait)) < 0.22,
                "lead {lead}: gloves never close"
            );
        }
        // And the two hold points are a long way apart, which is the entire
        // reason for having both.
        assert!(Physique::catch(0.0).distance(Physique::CRADLE) > 0.5);
    }

    /// Full stretch finishes above the crown of his head with the gloves
    /// properly apart, which is the whole point of leaving the ground: he is
    /// covering an area, not catching a pass.
    #[test]
    fn the_reach_goes_over_his_head() {
        let mut gait = still();
        gait.reach = 1.0;
        gait.stretch = 1.0;
        gait.dive = 1.0;
        let hands = glove(1.0, gait);
        let crown = crown(gait);
        assert!(
            hands.y > crown.y,
            "gloves at {hands:?} below crown {crown:?}"
        );
        assert!(
            glove(-1.0, gait).distance(glove(1.0, gait)) > 0.5,
            "gloves converging: {:?} and {hands:?}",
            glove(-1.0, gait)
        );
        // Outside the shoulders, not tucked in between them.
        assert!(hands.x.abs() > Physique::SHOULDER_SPREAD);
    }

    /// The two arms in a lateral dive do different things. Both at full
    /// stretch is the pose off the front of a cereal box.
    #[test]
    fn a_lateral_dive_has_a_leading_arm() {
        let mut gait = still();
        gait.reach = 1.0;
        gait.stretch = 1.0;
        gait.dive = 1.0;
        gait.lead = 1.0;
        let (trail, lead) = (glove(-1.0, gait), glove(1.0, gait));
        // Further along his own up-axis — which, once the carriage has rolled
        // him onto his side, is further across the goal: the top arm going
        // through the ball while the bottom one stays out for balance.
        assert!(
            lead.y - trail.y > 0.12,
            "arms level: lead {lead:?} trail {trail:?}"
        );
        let hips = Vec3::new(0.0, Physique::HIP, 0.0);
        assert!(
            hips.distance(lead) - hips.distance(trail) > 0.10,
            "arms equally extended: {} vs {}",
            hips.distance(lead),
            hips.distance(trail)
        );
        // A dive straight down the pitch keeps them square, because it is a
        // smother at a striker's feet and has no leading side.
        gait.lead = 0.0;
        assert!((glove(-1.0, gait).y - glove(1.0, gait).y).abs() < 1e-3);
    }

    /// The legs scissor with it: the leg he pushed off finishes straight and
    /// trailing, the far one folds up over it.
    #[test]
    fn a_lateral_dive_has_a_trailing_leg() {
        let mut gait = still();
        gait.dive = 1.0;
        gait.stretch = 1.0;
        gait.lead = 1.0;
        // Measured as how far each boot ends up from its own hip, because the
        // fold is a leg getting SHORTER: comparing heights only catches it
        // once the body is already over, and the body goes over on the
        // carriage rather than here.
        let hip = |side: f32| Vec3::new(side * Physique::HIP_SPREAD, Physique::HIP, 0.0);
        let folded = hip(-1.0).distance(boot(-1.0, gait));
        let straight = hip(1.0).distance(boot(1.0, gait));
        assert!(
            straight - folded > 0.15,
            "legs together: folded {folded} straight {straight}"
        );
        // The straight one is the one he pushed off, so it is as long as a
        // standing leg.
        assert!((straight - hip(1.0).distance(boot(1.0, still()))).abs() < 0.02);
    }

    /// A keeper who has gone over comes down with it.
    ///
    /// Rotating a body about a pivot fixed at standing hip height leaves him
    /// horizontal in mid-air with his boots half a metre off the grass, and
    /// then lands him there — which is most of what stopped a save reading as
    /// a save.
    #[test]
    fn going_over_brings_him_down() {
        assert!(Carriage::placed(0.0, 0.0, 0.0).translation.length() < 1e-5);

        // Flat across the goal at the moment of landing: no recorded lift
        // left to hold him up.
        let flat = Carriage::placed(0.0, -Actors::SPRAWL_ANGLE, 0.0);
        let hips = flat.transform_point(Vec3::new(0.0, Physique::HIP, 0.0));
        assert!(
            hips.y < 0.40,
            "hips still at {} m with the body flat",
            hips.y
        );
        assert!(hips.y > 0.15, "hips through the turf at {} m", hips.y);

        // And nothing on him ends up under the grass at any point of the
        // settle. Swept rather than sampled at the end, because the landing
        // is a quarter of a second long: the carriage walks from the angle he
        // arrived at to the angle of the ground while the limbs walk from
        // full stretch into the heap, and the two do NOT arrive together.
        // The carriage is stepped exactly as `PlayerActor::topple` steps it;
        // the LIMBS are held at full extension the whole way, which is the
        // harder case — the animator gives that back as he lands, so anything
        // clean here is clean there.
        let flying = Actors::SPRAWL_ANGLE;
        let committed = Actors::ease(
            (flying - Actors::GOES_OVER.0) / (Actors::GOES_OVER.1 - Actors::GOES_OVER.0),
        );
        for step in 0..=8 {
            let settling = step as f32 / 8.0;
            let over = flying + (FRAC_PI_2 - flying) * settling * committed;
            let carriage = Carriage::placed(0.0, -over, 0.0);
            let mut gait = still();
            gait.dive = 1.0;
            gait.stretch = 1.0;
            gait.grounded = settling;
            gait.lead = 1.0;
            // What `PlayerActor::gait` hands the arms across a landing: full
            // stretch on arrival, given back as he comes down on them. A dive
            // with the arms already at his sides is not a state the animator
            // can produce, and asserting against one measures nothing.
            gait.reach = 1.0 - settling;
            // Mid-landing a limb is allowed to be IN the turf. He arrives at
            // 79° with his arms spread across the goal, which puts the lead
            // glove BELOW his own body — correctly: it is the first thing to
            // touch the grass — and the recorded lift that was holding it up
            // snaps to zero the tick the engine's `fall` clamps it. Measured
            // across this sweep the worst of it is 0.10 m for about 0.13 s,
            // and the alternative is carrying the whole body high enough that
            // its extremes never reach the turf, which is the floating this
            // pass exists to remove. The pose he then HOLDS is the one that
            // has to be clean: a third of a second after a save, the better
            // part of four after a goal.
            let floor = if settling < 1.0 { -0.11 } else { -0.005 };
            for (what, part) in [
                ("left boot", boot(-1.0, gait)),
                ("right boot", boot(1.0, gait)),
                ("crown", crown(gait)),
                ("left glove", glove(-1.0, gait)),
                ("right glove", glove(1.0, gait)),
            ] {
                let world = carriage.transform_point(part);
                assert!(
                    world.y > floor,
                    "his {what} is {:.3} m under the grass at {:.0}% of the settle",
                    -world.y,
                    settling * 100.0
                );
            }
        }
    }

    /// Two axes of topple settle him as far as either one alone: a dive half
    /// across the goal and half up the pitch is just as horizontal.
    #[test]
    fn both_axes_of_topple_settle_him() {
        let hips = |pitch: f32, roll: f32| {
            Carriage::placed(pitch, roll, 0.0)
                .transform_point(Vec3::new(0.0, Physique::HIP, 0.0))
                .y
        };
        let sideways = hips(0.0, -1.4);
        let forwards = hips(1.4, 0.0);
        // Two axes that compose to the same tilt: cos(0.99)² ≈ cos(1.44).
        let diagonal = hips(0.99, -0.99);
        assert!((sideways - forwards).abs() < 1e-4);
        assert!(
            (diagonal - sideways).abs() < 0.05,
            "diagonal {diagonal} vs square {sideways}"
        );
        // And standing up costs him nothing.
        assert!((hips(0.0, 0.0) - Physique::HIP).abs() < 1e-5);
    }

    /// The set is a real crouch, so it costs real height — and the drop that
    /// pays for it has to leave his boots on the grass.
    #[test]
    fn the_set_keeps_his_boots_down() {
        let standing = boot(1.0, still());
        let mut gait = still();
        gait.set = 1.0;
        let crouched = boot(1.0, gait);
        assert!(
            (crouched.y - standing.y).abs() < 0.015,
            "boots move {} m into the set",
            crouched.y - standing.y
        );
        // And he really is lower: the crown comes down with the knees.
        assert!(crown(gait).y < crown(still()).y - 0.03);
    }

    /// **His hands go to the ball.**
    ///
    /// The save made on his feet is the one a keeper makes most often —
    /// measured, 84% of the balls arriving at him at pace — and the rig drew
    /// none of it: nothing moved until the ball was already in the hold band,
    /// which is to say until after it had stopped. Asserted as POSITIONS,
    /// like every other pose here: an angle at a shoulder is impossible to
    /// argue about, and "his gloves are by his knees for a ball over his
    /// head" is not.
    #[test]
    fn a_save_puts_his_gloves_on_the_ball() {
        let hands = |aim: Vec2| {
            let gait = saving(aim, 0.0);
            (glove(-1.0, gait) + glove(1.0, gait)) * 0.5
        };
        let low = hands(Vec2::new(0.0, -1.0));
        let chest = hands(Vec2::ZERO);
        let high = hands(Vec2::new(0.0, 1.0));
        assert!(
            low.y < 0.60,
            "he does not go down to one at his boots: gloves at {:.2} m",
            low.y
        );
        assert!(
            high.y > 1.85,
            "he does not go up to one over his head: gloves at {:.2} m",
            high.y
        );
        assert!(
            low.y < chest.y && chest.y < high.y,
            "the reach does not track the ball up: {:.2} / {:.2} / {:.2} m",
            low.y,
            chest.y,
            high.y
        );
        // And out in front of him rather than tucked against his chest —
        // a keeper meets the ball, he does not wait for it.
        assert!(
            chest.z > 0.25,
            "his hands are at his own chest, not in front of it: {:.2} m",
            chest.z
        );

        // Across him, and the NEAR arm leads: two arms travelling the same
        // distance is the superman, and rendered it is a man pointing at
        // something rather than a keeper saving anything.
        for wing in [-1.0_f32, 1.0] {
            let gait = saving(Vec2::new(wing, 0.0), 0.0);
            let near = glove(wing, gait).x * wing;
            let far = glove(-wing, gait).x * wing;
            assert!(
                near > 0.55,
                "the near glove does not go across him: {near:.2} m"
            );
            assert!(
                far < near - 0.30,
                "both arms travel the same way: near {near:.2} m, far {far:.2} m"
            );
        }
    }

    /// A parry is not a catch: the hands stay apart and the arms stay long,
    /// because the point of one is a surface and the point of the other is a
    /// pair of hands.
    #[test]
    fn a_parry_does_not_close_his_hands() {
        let spread = |parry: f32| {
            let gait = saving(Vec2::new(0.3, 0.2), parry);
            glove(1.0, gait).distance(glove(-1.0, gait))
        };
        assert!(
            spread(1.0) > spread(0.0) + 0.10,
            "a parry closes his hands as much as a catch does: {:.2} m against {:.2} m",
            spread(1.0),
            spread(0.0)
        );
    }

    /// **A keeper's set position is a BENT arm.**
    ///
    /// The one number behind *"he does nothing but stick them out"*: at
    /// −0.62 and −1.10 there was 135° between the two bones of his arm, so
    /// the ready position was a man reaching for something at the full
    /// length of his reach. A goalkeeper's is nearly a right angle, with his
    /// hands in front of his waist where they can go to either corner.
    ///
    /// Asserted as the geometry rather than as the constants: the angle at
    /// the elbow, and where that leaves the gloves against his own hips.
    #[test]
    fn the_set_bends_his_arms() {
        let mut gait = still();
        gait.set = 1.0;
        let shoulder = Vec3::new(Physique::SHOULDER_SPREAD, Physique::SHOULDER, 0.0);
        let arm = step(Limb::Torso, 0.0, Vec3::new(0.0, Physique::HIP, 0.0), gait)
            * step(Limb::Shoulder, 1.0, shoulder, gait);
        let elbow = arm
            * step(
                Limb::Elbow,
                1.0,
                Vec3::new(0.0, -Physique::UPPER_ARM, 0.0),
                gait,
            );
        let hand = glove(1.0, gait);

        let upper = (elbow.translation - arm.translation).normalize();
        let fore = (hand - elbow.translation).normalize();
        let bend = upper.dot(fore).acos().to_degrees();
        assert!(
            (70.0..115.0).contains(&bend),
            "his set arm is bent {bend:.0}°, which is not a goalkeeper's"
        );
        // Hands in front of his waist, not at arm's length — and outside his
        // own hips, where his weight is.
        assert!(
            (1.10..1.36).contains(&hand.y),
            "his gloves are at {:.2} m, which is not his waist",
            hand.y
        );
        assert!(
            (0.24..0.50).contains(&hand.z),
            "his gloves are {:.2} m in front of him",
            hand.z
        );
        assert!(hand.x > Physique::HIP_SPREAD, "his elbows are not out");
    }

    /// **A hand is open or it is closed, and until now it was neither.**
    ///
    /// The five digits were welded at a fixed splay, so a keeper spreading
    /// himself at a shot, punching a cross away and cradling a ball he had
    /// caught all had the identical hand on the end of the arm.
    ///
    /// Measured as how far the fingertip reaches DOWN THE HAND, in the
    /// wrist's own frame — the straight-line distance back to the knuckle
    /// will not do it, because a fully folded finger has curled round and is
    /// most of its own length away again, just in another direction.
    #[test]
    fn his_hands_open_and_shut() {
        let reach = |gait: Gait| {
            Physique::hand(1.0, gait)
                .to_matrix()
                .inverse()
                .transform_point3(fingertip(1.0, 1, gait))
                .y
        };
        let mut set = still();
        set.set = 1.0;
        let mut holding = still();
        holding.carry = 1.0;
        let open = reach(set);
        let fist = reach(saving(Vec2::new(0.3, 0.2), 1.0));
        let cradle = reach(holding);
        assert!(
            open < -0.165,
            "a set keeper's fingers are not out: they reach {open:.3} m down his hand"
        );
        assert!(
            fist > -0.075,
            "a punch is not a fist: the fingers still reach {fist:.3} m"
        );
        // …and a ball he has caught is held between the two, which is what
        // a hand round a ball is.
        assert!(
            (open + 0.020..fist - 0.020).contains(&cradle),
            "a cradled ball is not held in a curled hand: {cradle:.3} m against {fist:.3} and {open:.3}"
        );
        // Nobody stands about with a flat hand either.
        assert!(
            reach(still()) > open + 0.010,
            "his hand is flat open doing nothing: {:.3} m",
            reach(still())
        );
    }

    /// …and spreading them is a separate thing from opening them: a keeper
    /// covering his goal FANS his fingers, and a man walking back to his line
    /// with an open hand does not.
    #[test]
    fn a_keeper_spreads_his_fingers_at_a_shot() {
        let span = |gait: Gait| fingertip(1.0, 0, gait).distance(fingertip(1.0, 3, gait));
        let mut set = still();
        set.set = 1.0;
        assert!(
            span(set) > span(still()) + 0.02,
            "the set does not spread his hand: {:.3} m against {:.3} m",
            span(set),
            span(still())
        );
        assert!(
            span(diving(1.0, 1.0, 1.0)) > span(still()) + 0.02,
            "he goes full length with a hand he uses to carry shopping"
        );
        // A fist has no span at all — the fingers converge as they close,
        // which is the term that keeps a punch from being a spread paddle
        // folded over.
        let punch = saving(Vec2::new(0.3, 0.2), 1.0);
        assert!(
            span(punch) < span(set) * 0.75,
            "his fist is still fanned: {:.3} m against {:.3} m",
            span(punch),
            span(set)
        );
    }

    /// **Hands on the hips is a pose this rig could not reach**, and the
    /// note saying so is a year older than the yaw that fixed it.
    ///
    /// It was dropped in August 2026 because with the elbow out the forearm
    /// could only point forward and OUT — what it drew was a man holding an
    /// invisible tray. What was missing was a rotation about the arm's own
    /// long axis, which the standing save later added at the shoulder
    /// ([`Joint::SAVE_ACROSS`]); composed INNERMOST it turns the plane the
    /// elbow bends in without moving the upper arm at all.
    ///
    /// So the assertion is the one the note failed: the wrist has to arrive
    /// on the crest of his hip. Outside the shorts, at hip height, and
    /// beside him rather than in front.
    #[test]
    fn his_hands_reach_his_own_hips() {
        let gait = on_his_hips();
        let wrist = glove(1.0, gait);
        assert!(
            (0.85..1.06).contains(&wrist.y),
            "his hand is at {:.2} m, which is not his hip",
            wrist.y
        );
        assert!(
            (0.12..0.26).contains(&wrist.x),
            "his hand is {:.2} m across: it is not ON his hip",
            wrist.x
        );
        assert!(
            wrist.z.abs() < 0.16,
            "his hand is {:.2} m in front of him, which is the tray again",
            wrist.z
        );
        // …and the elbow is OUT, which is the whole silhouette of the pose.
        let shoulder = Vec3::new(Physique::SHOULDER_SPREAD, Physique::SHOULDER, 0.0);
        let elbow = (step(Limb::Torso, 0.0, Vec3::new(0.0, Physique::HIP, 0.0), gait)
            * step(Limb::Shoulder, 1.0, shoulder, gait))
        .transform_point(Vec3::new(0.0, -Physique::UPPER_ARM, 0.0));
        assert!(
            elbow.x > wrist.x + 0.08,
            "his elbows are not out: elbow {:.2}, wrist {:.2}",
            elbow.x,
            wrist.x
        );
    }

    /// **And bent double, his hands land on his knees** — with both boots
    /// still on the grass.
    ///
    /// ⚠ The knee is the half that goes wrong twice. Bending it alone swings
    /// the shin BACKWARD and lifts the boot off the turf, because the legs
    /// hang from the hips and there is nothing under them; so the hip has to
    /// flex with it, and then the pair of them shorten the leg and the body
    /// has to pay the difference. Same bookkeeping as [`Joint::SET_DROP`],
    /// four times the size.
    #[test]
    fn bent_double_he_puts_his_hands_on_his_knees() {
        let gait = doubled_over();
        let hip = Vec3::new(Physique::HIP_SPREAD, Physique::HIP, 0.0);
        let knee =
            step(Limb::Hip, 1.0, hip, gait).transform_point(Vec3::new(0.0, -Physique::THIGH, 0.0));
        let wrist = glove(1.0, gait);
        assert!(
            wrist.distance(knee) < 0.16,
            "his hand is {:.2} m from his knee: hand {wrist:?}, knee {knee:?}",
            wrist.distance(knee)
        );
        // He really is bent over — the crown comes forward, not just down.
        let crown = crown(gait);
        assert!(
            crown.z > 0.30 && crown.y < 1.35,
            "he is not bent double: crown at {crown:?}"
        );
        for side in [-1.0f32, 1.0] {
            let sole = boot(side, gait).y;
            assert!(
                (sole - boot(side, still()).y).abs() < 0.03,
                "his boot moves {:.3} m bending over",
                sole - boot(side, still()).y
            );
        }
    }

    /// **The four reactions are four pictures**, which is the whole reason
    /// they are picked rather than blended: the interpolation between a man
    /// with his hands on his head and one bent over his knees is a man doing
    /// neither.
    #[test]
    fn conceding_has_four_different_answers() {
        // The WRIST and the ELBOW together, because half of what separates
        // these is where the elbows are: a man with his hands on his hips
        // has his wrists barely a hand's width from where they would hang
        // anyway, and the picture is the two triangles either side of him.
        let arm = |gait: Gait| {
            let shoulder = Vec3::new(Physique::SHOULDER_SPREAD, Physique::SHOULDER, 0.0);
            let elbow = (step(Limb::Torso, 0.0, Vec3::new(0.0, Physique::HIP, 0.0), gait)
                * step(Limb::Shoulder, 1.0, shoulder, gait))
            .transform_point(Vec3::new(0.0, -Physique::UPPER_ARM, 0.0));
            (glove(1.0, gait), elbow)
        };
        let hands = [
            ("head", arm(slumped(1.0))),
            ("hips", arm(on_his_hips())),
            ("knees", arm(doubled_over())),
            ("hanging", arm(slumped(0.0))),
        ];
        for (i, (one, first)) in hands.iter().enumerate() {
            for (other, second) in &hands[i + 1..] {
                let apart = first.0.distance(second.0) + first.1.distance(second.1);
                assert!(
                    apart > 0.24,
                    "{one} and {other} are the same picture: {apart:.2} m of arm between them"
                );
            }
        }
    }

    /// **A keeper with nothing to do organises people**, and pointing is the
    /// one gesture in football where the four fingers do different things:
    /// the index goes straight and the rest shut.
    ///
    /// One arm, too. Which one is the SIGN of [`Gait::pointing`], so a
    /// signal that moved both would be a keeper hailing a taxi.
    #[test]
    fn he_points_with_one_arm_and_one_finger() {
        let mut gait = still();
        gait.pointing = 1.0;
        let out = glove(1.0, gait);
        let down = glove(-1.0, gait);
        assert!(
            out.y > down.y + 0.35,
            "both arms went: {:.2} m against {:.2} m",
            out.y,
            down.y
        );
        assert!(
            out.z > 0.25 && out.x > 0.30,
            "he is not pointing at anything: {out:?}"
        );
        // The index is out past the rest of the hand; the others are shut.
        let along = |digit: usize| {
            Physique::hand(1.0, gait)
                .to_matrix()
                .inverse()
                .transform_point3(fingertip(1.0, digit, gait))
                .y
        };
        assert!(
            along(0) < -0.16,
            "his index finger is not out: it reaches {:.3} m",
            along(0)
        );
        for shut in 1..4 {
            assert!(
                along(shut) > -0.09,
                "finger {shut} is still out at {:.3} m behind a pointed one",
                along(shut)
            );
        }
    }

    /// …and urging his defence up puts his gloves in front of his CHEST,
    /// where a clap happens, rather than in front of his face, where
    /// surrender does. They come together and apart on the idle clock.
    #[test]
    fn urging_claps_his_gloves_in_front_of_him() {
        let at = |idle: f32| {
            let mut gait = still();
            gait.urging = 1.0;
            gait.idle = idle;
            (glove(-1.0, gait), glove(1.0, gait))
        };
        let (left, right) = at(0.0);
        for hand in [left, right] {
            assert!(
                (1.15..1.45).contains(&hand.y),
                "his gloves are at {:.2} m: that is his face, not his chest",
                hand.y
            );
            assert!(
                hand.z > 0.20,
                "his gloves are not in front of him: {hand:?}"
            );
        }
        // Two beats: the widest and the narrowest of one clap.
        let mut apart: Vec<f32> = (0..24)
            .map(|step| {
                let (left, right) = at(step as f32 * TAU / 24.0);
                left.distance(right)
            })
            .collect();
        apart.sort_by(f32::total_cmp);
        assert!(
            apart[apart.len() - 1] - apart[0] > 0.10,
            "his hands never meet: {:.2}–{:.2} m apart",
            apart[0],
            apart[apart.len() - 1]
        );
        assert!(
            apart[0] < 0.20,
            "his gloves never actually clap: closest {:.2} m",
            apart[0]
        );
    }

    /// And whatever he is reaching for, his boots stay on the grass. Going
    /// down to a ball at his feet costs real height — the knees bend and the
    /// waist folds — and if the drop that pays for it and the pose that
    /// spends it ever part company he sinks into the turf.
    #[test]
    fn a_save_keeps_his_boots_on_the_grass() {
        let flat = boot(1.0, still()).y;
        for aim in [
            Vec2::new(0.0, -1.0),
            Vec2::new(1.0, -0.6),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, 1.0),
        ] {
            for side in [-1.0_f32, 1.0] {
                let sole = boot(side, saving(aim, 0.0)).y;
                assert!(
                    (sole - flat).abs() < 0.05,
                    "his {} boot moves {:.3} m saving one at ({:.1}, {:.1})",
                    if side < 0.0 { "left" } else { "right" },
                    sole - flat,
                    aim.x,
                    aim.y
                );
            }
        }
    }

    /// **A set keeper is never still, and he never has both feet in the
    /// air.**
    ///
    /// The little step on the spot is what makes him read as a man about to
    /// move rather than a man standing there — and because the two legs are
    /// half a cycle apart, it costs no height and nothing has to be paid for
    /// it. Both halves are asserted: that a foot really does come up, and
    /// that the other one is always down.
    #[test]
    fn a_set_keeper_dances_on_his_toes() {
        let flat = boot(1.0, still()).y;
        let mut lifted = 0.0_f32;
        for step in 0..64 {
            let mut gait = still();
            gait.set = 1.0;
            gait.idle = step as f32 * TAU / 64.0;
            let (left, right) = (boot(-1.0, gait).y, boot(1.0, gait).y);
            lifted = lifted.max((left - flat).max(right - flat));
            assert!(
                (left - flat).min(right - flat) < 0.012,
                "both boots are off the grass at idle {:.2}: {:.3} m and {:.3} m",
                gait.idle,
                left - flat,
                right - flat
            );
        }
        assert!(
            lifted > 0.02,
            "his feet never leave the grass — the set is a statue again ({lifted:.3} m)"
        );
    }

    /// The extension is a ramp and not a switch: a keeper halfway through a
    /// flight is halfway out of it. This is the whole difference between a
    /// dive and a photograph of one.
    #[test]
    fn the_extension_opens_out() {
        let mut gait = still();
        gait.dive = 1.0;
        gait.reach = 1.0;
        let mut reach_at = |stretch: f32| {
            gait.stretch = stretch;
            glove(1.0, gait).y
        };
        let (gathered, half, full) = (reach_at(0.0), reach_at(0.5), reach_at(1.0));
        assert!(
            full - gathered > 0.30,
            "no extension at all: {gathered} to {full}"
        );
        assert!(
            half > gathered + 0.08 && half < full - 0.08,
            "not a ramp: {gathered} / {half} / {full}"
        );
    }

    /// An outfielder leaving the ground is heading a ball, not saving one:
    /// he tucks his knees and keeps his arms out for balance, and he does not
    /// go over.
    #[test]
    fn a_header_is_not_a_dive() {
        let mut gait = still();
        gait.jump = 1.0;
        assert!(
            boot(1.0, gait).y > boot(1.0, still()).y + 0.15,
            "knees not tucked: {:?}",
            boot(1.0, gait)
        );
        let hands = glove(1.0, gait);
        assert!(hands.y < crown(gait).y, "outfielder reaching like a keeper");
        assert!(hands.x.abs() > glove(1.0, still()).x.abs(), "arms not out");
    }

    /// A jumping outfielder does not go over, whatever he does in the air —
    /// the sprawl is a keeper's alone, and the tip that drives it is never
    /// written for anybody else.
    #[test]
    fn only_a_keeper_sprawls() {
        let mut gait = still();
        gait.jump = 1.0;
        assert_eq!(gait.dive, 0.0);
        assert_eq!(gait.stretch, 0.0);
        assert_eq!(gait.reach, 0.0);
    }

    /// A kick is a leg going somewhere. The whole point of the swing is that
    /// the boot travels: back behind him, through the ball, and up the other
    /// side.
    #[test]
    fn the_boot_travels_through_the_ball() {
        // Sampled inside the swing rather than at its ends, where the pose
        // is deliberately handed back to the run cycle.
        let back = boot(1.0, kicking(-0.8));
        let contact = boot(1.0, kicking(0.0));
        let through = boot(1.0, kicking(0.8));

        // Behind him at the top of the backswing, in front of him at contact.
        assert!(back.z < -0.35, "no backswing: {back:?}");
        assert!(contact.z > 0.45, "no contact: {contact:?}");
        assert!(
            contact.z - back.z > 0.9,
            "the boot barely moves: {back:?} to {contact:?}"
        );
        // And still rising afterwards — a struck ball is hit through, not at.
        assert!(
            through.y > contact.y + 0.1,
            "no follow through: {contact:?} to {through:?}"
        );
        // It is a boot, not a foot through the turf.
        for stage in [-1.0f32, -0.8, -0.5, 0.0, 0.5, 0.8, 1.0] {
            let sole = boot(1.0, kicking(stage));
            assert!(sole.y > -0.01, "boot under the grass at {stage}: {sole:?}");
        }
    }

    /// **The foot rolls.** Reaching out to land it is pulled up; driving
    /// off the back of the stride it points. A boot welded in line with its
    /// shin is what "very stiff in their movements" looks like, and it is
    /// the joint an eye reads first because it is the one touching the
    /// ground.
    ///
    /// Asserted as the toe's position rather than as the angle, for the
    /// reason every pose test here is: an angle with the wrong sign is
    /// still a plausible-looking number.
    #[test]
    fn the_foot_rolls_through_the_stride() {
        // The toe, in the ankle's own space — forward of it and level.
        let toe = Vec3::new(0.0, -0.02, 0.10);
        let at = |phase: f32| {
            let mut gait = running(1.0);
            gait.phase = phase;
            let hip = Vec3::new(Physique::HIP_SPREAD, Physique::HIP, 0.0);
            let knee = Vec3::new(0.0, -Physique::THIGH, 0.0);
            (step(Limb::Hip, 1.0, hip, gait)
                * step(Limb::Knee, 1.0, knee, gait)
                * step(Limb::Ankle, 1.0, Physique::ANKLE, gait))
            .transform_point(toe)
        };
        // Half a cycle apart: leg reaching out, then leg trailing.
        let reaching = at(FRAC_PI_2);
        let driving = at(-FRAC_PI_2);
        // Off the back of the stride the toe is pointed — lower, relative
        // to the ankle it hangs from, than when it is pulled up to land.
        let ankle_of = |phase: f32| {
            let mut gait = running(1.0);
            gait.phase = phase;
            let hip = Vec3::new(Physique::HIP_SPREAD, Physique::HIP, 0.0);
            let knee = Vec3::new(0.0, -Physique::THIGH, 0.0);
            (step(Limb::Hip, 1.0, hip, gait) * step(Limb::Knee, 1.0, knee, gait))
                .transform_point(Physique::ANKLE)
        };
        let drop_driving = ankle_of(-FRAC_PI_2).y - driving.y;
        let drop_reaching = ankle_of(FRAC_PI_2).y - reaching.y;
        assert!(
            drop_driving > drop_reaching + 0.02,
            "the foot does not roll: toe {drop_driving:.3} m below the ankle driving \
             against {drop_reaching:.3} m reaching"
        );
    }

    /// Nothing in this rig does ground contact — the body rides at a fixed
    /// hip height and the legs swing under it — so the one thing a new leg
    /// joint can break is the turf line. The foot may not dig further into
    /// the grass than the old welded boot did.
    #[test]
    fn the_rolling_foot_does_not_dig_into_the_turf() {
        let lowest = |spring: f32| {
            let mut deepest = f32::MAX;
            for step_i in 0..64 {
                let mut gait = running(1.0);
                gait.phase = step_i as f32 * TAU / 64.0;
                gait.spring = spring;
                for side in [-1.0f32, 1.0] {
                    deepest = deepest.min(boot(side, gait).y);
                }
            }
            deepest
        };
        let standing = boot(1.0, still()).y;
        assert!(
            lowest(1.14) > standing - 0.30,
            "a running foot reaches {:.3} m below where a standing one sits",
            standing - lowest(1.14)
        );
    }

    /// …and it is flat when he is standing on it, which is what lets every
    /// standing pose in this rig — the set, the cradle, the slump — carry on
    /// knowing nothing about the joint.
    #[test]
    fn a_standing_foot_is_flat() {
        let flat = step(Limb::Ankle, 1.0, Physique::ANKLE, still());
        assert!(
            flat.rotation.angle_between(Quat::IDENTITY) < 1.0e-4,
            "a standing player is up on his toes: {:?}",
            flat.rotation
        );
    }

    /// **Twenty-two players, twenty-two runs.** The squad used to share one
    /// stride length, one cycle amplitude and one idle rate, so the whole
    /// pitch ran the same animation at different speeds — which is what
    /// "lacks variety" is. The three are cut from SEPARATE hashes on
    /// purpose: taken off one, a squad has two kinds of runner in it rather
    /// than eight.
    #[test]
    fn no_two_players_run_alike() {
        use crate::players::kit::Complexion;
        let ids: Vec<u32> = (100..140).collect();
        let strides: Vec<f32> = ids.iter().map(|&id| Complexion::stride(id)).collect();
        let springs: Vec<f32> = ids.iter().map(|&id| Complexion::spring(id)).collect();
        let tempos: Vec<f32> = ids.iter().map(|&id| Complexion::tempo(id)).collect();

        let spread = |v: &[f32]| {
            let lo = v.iter().copied().fold(f32::MAX, f32::min);
            let hi = v.iter().copied().fold(f32::MIN, f32::max);
            hi - lo
        };
        assert!(
            spread(&strides) > 0.18,
            "one cadence: {:?}",
            spread(&strides)
        );
        assert!(spread(&springs) > 0.15, "one amplitude");
        assert!(spread(&tempos) > 0.20, "everybody breathes together");

        // …and independent of each other. A long strider is not
        // automatically a high-kneed one.
        let correlation = |a: &[f32], b: &[f32]| {
            let n = a.len() as f32;
            let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
            let cov: f32 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
            let va: f32 = a.iter().map(|x| (x - ma) * (x - ma)).sum::<f32>().sqrt();
            let vb: f32 = b.iter().map(|y| (y - mb) * (y - mb)).sum::<f32>().sqrt();
            (cov / (va * vb)).abs()
        };
        assert!(
            correlation(&strides, &springs) < 0.5,
            "stride and amplitude are cut from the same bits: r={}",
            correlation(&strides, &springs)
        );
        assert!(
            correlation(&springs, &tempos) < 0.5,
            "amplitude and tempo are"
        );
    }

    /// Only one leg swings. The other one is planted, taking the whole body's
    /// weight while it does.
    #[test]
    fn the_other_leg_stays_planted() {
        let gait = kicking(0.0);
        let swinging = boot(1.0, gait);
        let planted = boot(-1.0, gait);
        assert!(
            swinging.z - planted.z > 0.5,
            "both legs swinging: {swinging:?} and {planted:?}"
        );
        // Bent under him rather than straight: he is standing on it.
        assert!(
            planted.y - boot(-1.0, still()).y < 0.015,
            "standing foot floating: {planted:?}"
        );
        // He sinks over it rather than standing tall on a bent leg.
        assert!(
            crown(gait).y < crown(still()).y - 0.03,
            "no sink into the plant"
        );
        // And a left-footed kick is the mirror of a right-footed one.
        let mut left = kicking(0.0);
        left.foot = -1.0;
        let mirrored = boot(-1.0, left);
        assert!((mirrored.z - swinging.z).abs() < 1e-3);
        assert!((mirrored.x + swinging.x).abs() < 1e-3);
    }

    /// Every rotation a footballer puts into a ball is paid for somewhere: the
    /// arm opposite the kicking leg comes across and up.
    #[test]
    fn the_arms_pay_for_the_kick() {
        let gait = kicking(0.0);
        let counter = glove(-1.0, gait);
        let same = glove(1.0, gait);
        assert!(
            counter.y - same.y > 0.15,
            "arms doing nothing: {counter:?} and {same:?}"
        );
        assert!(
            counter.y > glove(-1.0, still()).y + 0.15,
            "counter-arm never lifts: {counter:?}"
        );
    }

    /// **Hands on the head means hands ON the head.**
    ///
    /// The whole pose is four angles at two joints, and four angles are
    /// impossible to argue about — but "his gloves are level with his ears
    /// and eight centimetres apart" is not. Asserted as positions for the
    /// same reason every other pose here is: the keeper's slump was written
    /// blind, and a shoulder ten degrees short leaves a man holding his
    /// hands somewhere near his collarbone, which reads as nothing at all.
    #[test]
    fn a_beaten_keeper_puts_his_hands_on_his_head() {
        let gait = slumped(1.0);
        let head = crown(gait);
        for side in [-1.0f32, 1.0] {
            let hand = glove(side, gait);
            // Temple height, not crown height — the top of the skull is a
            // good fifteen centimetres above where a hand actually lands on
            // a head, and asking for the glove to be level with it asks for
            // a pose nobody adopts.
            assert!(
                hand.y > head.y - 0.25,
                "the {side} glove is not up at the head: {hand:?} against a crown at {head:?}"
            );
            assert!(
                hand.y > glove(side, still()).y + 0.50,
                "…it is still hanging by his side: {hand:?}"
            );
            assert!(
                (hand.x - head.x).abs() < 0.30,
                "…or it is out at arm's length rather than on it: {hand:?}"
            );
        }
        // Elbows OUT, not crossed over his own head — the sign trap
        // `REACH_SPREAD` documents, which puts the two wrists inside the
        // shoulders whenever the arms are raised.
        let (left, right) = (glove(-1.0, gait), glove(1.0, gait));
        assert!(
            (left.x - right.x).abs() > 0.16,
            "the two hands have crossed over: {left:?} and {right:?}"
        );
    }

    /// The other slump, and it has to be a genuinely different picture —
    /// two poses that read the same are one pose with extra constants.
    #[test]
    fn arms_hanging_is_not_hands_to_the_head() {
        let limp = slumped(0.0);
        let head = slumped(1.0);
        let lift = glove(1.0, head).y - glove(1.0, limp).y;
        assert!(
            lift > 0.45,
            "the two slumps put the hands in the same place: {lift:.2} m apart"
        );
        assert!(
            glove(1.0, limp).y < crown(limp).y - 0.75,
            "arms that are hanging hang: {:?}",
            glove(1.0, limp)
        );
    }

    /// And a man who has just scored is doing the opposite of both — up and
    /// open, not down and closed. If these two ever converge the aftermath
    /// draws one crowd of identical people, which is what it looked like
    /// before any of this existed.
    #[test]
    fn scoring_and_conceding_are_opposite_pictures() {
        let up = cheering();
        let down = slumped(1.0);
        // The head is the read at distance, and a fold at the hip carries
        // it FORWARD far more than it lowers it — a 20° stoop is a quarter
        // of a metre of travel and four centimetres of height. So the
        // discriminator is where the crown is over the boots, not how high
        // it is: one man is bent over his own toes and the other is leaning
        // back off his heels.
        assert!(
            crown(down).z - crown(up).z > 0.35,
            "the two reactions carry the head in the same place: {:?} vs {:?}",
            crown(up),
            crown(down)
        );
        assert!(
            crown(down).z > crown(still()).z + 0.20,
            "a man who has conceded is not standing up straight: {:?}",
            crown(down)
        );
        // Arms up in both — the difference is the head and the elbows, so
        // check the thing that actually separates them at distance.
        assert!(
            glove(1.0, up).x.abs() > glove(1.0, down).x.abs() + 0.08,
            "the celebration's arms are not opened out: {:?} vs {:?}",
            glove(1.0, up),
            glove(1.0, down)
        );
    }

    /// Anything in this rig that bends a leg has to pay for the height it
    /// costs, or the boots hang over the grass. Same rule `SET_DROP` and
    /// `CARRY_DROP` exist for; the slump bends a knee too.
    #[test]
    fn the_slump_keeps_his_boots_on_the_grass() {
        for hands in [0.0f32, 1.0] {
            let gait = slumped(hands);
            for side in [-1.0f32, 1.0] {
                let sole = boot(side, gait).y;
                let standing = boot(side, still()).y;
                assert!(
                    (sole - standing).abs() < 0.012,
                    "boot floats or sinks by {:.3} m at hands_to_head {hands}",
                    sole - standing
                );
            }
        }
    }

    /// Nothing happens to anybody who is not kicking, whatever the phase says.
    /// `swing` is zero for the whole squad every frame, so a term that ignored
    /// `power` would have twenty-two players permanently mid-kick.
    #[test]
    fn power_gates_the_whole_kick() {
        let mut idle = still();
        idle.swing = -0.5;
        idle.foot = 1.0;
        assert!((boot(1.0, idle) - boot(1.0, still())).length() < 1e-4);
        assert!((glove(-1.0, idle) - glove(-1.0, still())).length() < 1e-4);
        assert!((crown(idle) - crown(still())).length() < 1e-4);
    }

    /// A tap and a shot are the same movement at very different sizes.
    #[test]
    fn power_scales_the_swing() {
        let mut gentle = kicking(0.0);
        gentle.power = 0.15;
        let travel = |gait: Gait| boot(1.0, gait).distance(boot(1.0, still()));
        assert!(
            travel(kicking(0.0)) > travel(gentle) * 2.0,
            "a tap swings like a shot: {} vs {}",
            travel(gentle),
            travel(kicking(0.0))
        );
    }

    /// The hips lead a kick and the shoulders follow, which is what makes it
    /// look like it came from the ground up rather than from the knee.

    /// **The hips lead the opening and the chest follows it.**
    ///
    /// [`Gait::open`] is a rotation of the LOWER body: a footballer coming
    /// round onto a run turns his legs onto it first and his shoulders after,
    /// which is what the separation between the two reads as. Both have to
    /// go the same way — a chest that counter-rotated would draw a man
    /// wringing himself out — and the chest has to go less far, or there is
    /// no separation and the opening is just a man turned round.
    #[test]
    fn the_chest_follows_the_hips_round() {
        let turned = |limb: Limb, gait: Gait| {
            let joint = Joint::new(Entity::from_raw_u32(0).unwrap(), limb, 0.0, Vec3::ZERO);
            let point = joint.pose(gait) * Vec3::Z;
            point.x.atan2(point.z)
        };
        let mut gait = running(0.8);
        gait.open = 1.2;
        let hips = turned(Limb::Pelvis, gait);
        let chest = turned(Limb::Torso, gait);
        assert!(
            chest > 0.02 && chest < hips - 0.2,
            "with his legs {:.0} deg round, his hips are at {:.0} and his chest at {:.0}",
            gait.open.to_degrees(),
            hips.to_degrees(),
            chest.to_degrees()
        );
    }

    /// …and a kick takes the whole opening back off him: he plants and swings
    /// at the ball, not along the run he arrived on. See [`Joint::opened`].
    #[test]
    fn a_kick_squares_his_hips_onto_the_ball() {
        let mut gait = kicking(0.0);
        gait.open = 1.2;
        let socket = step(
            Limb::Hip,
            1.0,
            Vec3::new(Physique::HIP_SPREAD, Physique::HIP, 0.0),
            gait,
        );
        assert!(
            socket.translation.z.abs() < 0.01,
            "his hip socket is {:.3} m off the line of his chest at contact",
            socket.translation.z
        );
    }
    #[test]
    fn the_hips_lead_the_shoulders() {
        // Measured as how far each has turned by the moment of contact,
        // against where they are for a man standing still.
        let turned = |limb: Limb, gait: Gait| {
            let joint = Joint::new(Entity::from_raw_u32(0).unwrap(), limb, 0.0, Vec3::ZERO);
            let point = joint.pose(gait) * Vec3::Z;
            point.x.atan2(point.z)
        };
        // Going back, the chest coils further than the hips do.
        let (hips, chest) = (
            turned(Limb::Pelvis, kicking(-0.7)),
            turned(Limb::Torso, kicking(-0.7)),
        );
        assert!(
            hips * chest > 0.0,
            "coiling opposite ways: {hips} vs {chest}"
        );
        assert!(
            chest.abs() > hips.abs(),
            "hips coil further: {hips} vs {chest}"
        );
        // Coming through, the hips arrive first: at contact they have already
        // turned toward the target and the chest is still closed.
        for stage in [0.0f32, 0.5] {
            let hips = turned(Limb::Pelvis, kicking(stage));
            let chest = turned(Limb::Torso, kicking(stage));
            assert!(
                hips < chest - 0.05,
                "chest ahead of the hips at {stage}: {hips} vs {chest}"
            );
        }
        // And the turn reverses between the backswing and the follow through.
        assert!(turned(Limb::Pelvis, kicking(-0.8)) * turned(Limb::Pelvis, kicking(0.8)) < 0.0);
    }

    /// A keeper's throw is the same swing sent to his arm — and it must go
    /// over the top, not underarm.
    #[test]
    fn a_throw_goes_over_the_top() {
        let throwing = |swing: f32| {
            let mut gait = still();
            gait.swing = swing;
            gait.foot = 1.0;
            gait.throwing = 1.0;
            gait
        };
        let cocked = glove(1.0, throwing(-0.8));
        let release = glove(1.0, throwing(0.0));
        let after = glove(1.0, throwing(0.8));
        assert!(cocked.z < -0.1, "not cocked back: {cocked:?}");
        assert!(
            release.y > cocked.y,
            "released below the wind-up: {release:?}"
        );
        assert!(after.z > release.z, "no follow through: {after:?}");
        // Over the top: the hand never drops back down past the hip on the
        // way, which is what a linear interpolation between the two end
        // angles would have done.
        for step in 0..=32 {
            let swing = -0.8 + step as f32 / 20.0;
            let hand = glove(1.0, throwing(swing));
            assert!(hand.y > 1.05, "underarm at {swing}: {hand:?}");
        }
        // And his legs are not involved.
        assert!((boot(1.0, throwing(0.0)) - boot(1.0, still())).length() < 1e-4);
    }

    /// Driving off the mark and pulling up short both bend a player, and in
    /// opposite directions.
    #[test]
    fn a_change_of_pace_bends_him() {
        let lean = |drive: f32| {
            let mut gait = still();
            gait.drive = drive;
            crown(gait).z
        };
        assert!(lean(1.0) > lean(0.0) + 0.15, "no drive: {}", lean(1.0));
        assert!(lean(-1.0) < lean(0.0) - 0.12, "no brake: {}", lean(-1.0));
    }

    /// A header comes out of the spine, not out of a leg.
    ///
    /// Every ball met above head height used to be drawn as a kick, which at
    /// the back post is a man hooking his boot up past his own ear.
    #[test]
    fn a_header_comes_from_the_spine() {
        let back = crown(nodding(-0.8));
        let contact = crown(nodding(0.0));
        let through = crown(nodding(0.8));
        // The head travels forward through the ball, and keeps going.
        assert!(
            contact.z - back.z > 0.10,
            "no snap: {back:?} to {contact:?}"
        );
        assert!(through.z > contact.z, "no follow through: {through:?}");
        // Both arms go out to hold the space, and neither of them is a
        // keeper's reach.
        let arms = glove(1.0, nodding(0.0));
        assert!(
            arms.x.abs() > glove(1.0, still()).x.abs() + 0.05,
            "arms not out: {arms:?}"
        );
        assert!(arms.y < contact.y, "arms over his head like a save");
        // And his feet stay exactly where a run cycle would have left them:
        // this is one action, not two.
        for stage in [-0.8f32, 0.0, 0.8] {
            assert!(
                (boot(1.0, nodding(stage)) - boot(1.0, still())).length() < 1e-4,
                "the leg swings at a header, at {stage}"
            );
        }
    }

    /// A throw-in is taken with BOTH hands, over the head, off both feet —
    /// which is the whole reason it cannot borrow a goalkeeper's throw.
    #[test]
    fn a_throw_in_uses_both_hands() {
        // Shoulder height, which is the line the ball must never drop below on
        // its way round: a throw-in taken underarm is a foul throw, and this
        // rig would happily draw one, because these keys interpolate as
        // scalars and the short way round between "behind the head" and "out
        // in front" goes past his hip.
        let shoulders = Physique::HIP + Physique::SHOULDER;
        let hanging = glove(1.0, still()).y;
        // From the point the ball is behind his head onward. Earlier than
        // that he is still taking it back, and the deepest part of a long
        // throw's wind-up really does drop to about shoulder height with the
        // ball half a metre behind him.
        for stage in [-0.6f32, -0.3, 0.0, 0.3] {
            let gait = tossing(stage);
            let (left, right) = (glove(-1.0, gait), glove(1.0, gait));
            assert!(
                (left.x + right.x).abs() < 0.02 && (left.y - right.y).abs() < 0.02,
                "hands doing different things at {stage}: {left:?} and {right:?}"
            );
            assert!(
                left.y > shoulders,
                "the throw goes round below his shoulder at {stage}: {left:?}"
            );
        }
        // And across the whole sweep — including the blends into and out of
        // the run cycle at each end — the hands never drop below where they
        // hang anyway. Anything lower is an underarm bowl, which is what a
        // scalar interpolation taking the short way round produces.
        for step in 0..=40 {
            let stage = -1.0 + step as f32 / 20.0;
            let hands = glove(1.0, tossing(stage));
            assert!(
                hands.y > hanging - 0.01,
                "underarm at {stage}: {hands:?}, hanging at {hanging}"
            );
        }
        // Feet planted throughout: it is taken standing still, and both of
        // them stay on the grass.
        for stage in [-0.9f32, -0.3, 0.3, 0.9] {
            let gait = tossing(stage);
            for side in [-1.0f32, 1.0] {
                assert!(
                    boot(side, gait).y > -0.01,
                    "boot through the turf at {stage}"
                );
            }
        }
        // Over the top of him: taken back behind the head, up over it, and
        // well in front by the release.
        assert!(glove(1.0, tossing(-0.8)).z < -0.35, "not taken back");
        assert!(
            glove(1.0, tossing(-0.3)).y > crown(tossing(-0.3)).y,
            "the ball never gets above his head"
        );
        assert!(
            glove(1.0, tossing(0.6)).z > glove(1.0, tossing(-0.8)).z + 0.6,
            "the ball never leaves him"
        );
    }

    /// Neither of them touches anybody who is not making it, whatever the
    /// swing says — `swing` is written every frame for the whole squad, so a
    /// term that ignored its own amplitude would put twenty-two players
    /// permanently mid-header.
    #[test]
    fn the_amplitudes_gate_the_new_strikes() {
        let mut idle = still();
        idle.swing = -0.5;
        for part in [crown(idle), glove(1.0, idle), boot(1.0, idle)] {
            assert!((part - part).length() < 1e-6);
        }
        assert!((crown(idle) - crown(still())).length() < 1e-4);
        assert!((glove(1.0, idle) - glove(1.0, still())).length() < 1e-4);
    }

    /// The print lies ON the shirt.
    ///
    /// A flat rectangle wide enough to carry a number stands nearly four
    /// centimetres off a torso at its corners, which is what made the number
    /// read as a card pinned to a footballer. Measured as the ellipse radius
    /// each vertex lands at: 1.0 is the cloth itself.
    #[test]
    fn the_print_lies_on_the_shirt() {
        for panel in [
            Sculptor::decal(
                &BodyParts::shirt(),
                BodyParts::NUMBER_AT,
                BodyParts::NUMBER_HEIGHT,
                -FRAC_PI_2,
                BodyParts::NUMBER_ARC,
                BodyParts::PRINT_LIFT,
            ),
            Sculptor::decal(
                &BodyParts::shirt(),
                BodyParts::NAME_AT,
                BodyParts::NAME_HEIGHT,
                -FRAC_PI_2,
                BodyParts::NAME_ARC,
                BodyParts::PRINT_LIFT,
            ),
        ] {
            let positions = panel
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|values| values.as_float3())
                .expect("panel has positions")
                .to_vec();
            for point in &positions {
                let shirt = Sculptor::section(&BodyParts::shirt(), point[1]);
                // In the shirt's OWN section, which is not an ellipse — see
                // [`Ring::radius`].
                let radius = shirt.radius(point[0], point[2]);
                assert!(
                    (1.0..1.06).contains(&radius),
                    "{point:?} sits at {radius} of the shirt's own radius"
                );
                // On the BACK of it, which is where print goes.
                assert!(point[2] < shirt.offset, "print on the front: {point:?}");
            }
            // And it faces outward, or it is drawn from inside the player.
            let normals = panel
                .attribute(Mesh::ATTRIBUTE_NORMAL)
                .and_then(|values| values.as_float3())
                .expect("panel has normals");
            let outward = normals.iter().filter(|normal| normal[2] < -0.5).count();
            assert!(
                outward > normals.len() / 2,
                "the panel is wound inside out: {outward} of {} face backwards",
                normals.len()
            );
        }
    }

    /// **The shirt hangs over the shorts, and it has to keep doing it while
    /// he moves.**
    ///
    /// The shirt is hung off [`Limb::Torso`] and the seat off [`Limb::Pelvis`],
    /// which by design never turns — so the two swing against each other every
    /// time a player leans into a run, and a clearance that is comfortable on
    /// a man standing still is nothing of the kind on one running. Measured at
    /// the lean the run cycle actually uses, the seat came seven millimetres
    /// out through the back of the shirt and drew a dark crescent across the
    /// small of his back, on every player, for as long as he was moving.
    ///
    /// Checked as the shirt's own surface expressed in the seat's space, over
    /// the poses that lean hardest, because that is the quantity that has to
    /// stay above one and no amount of staring at the two profiles side by
    /// side will tell you whether it does.
    #[test]
    fn the_shirt_hangs_clear_of_the_shorts() {
        let hips = Vec3::new(0.0, Physique::HIP, 0.0);
        let shirt = BodyParts::shirt();
        let seat = BodyParts::seat();
        // The SEAT measured against the shirt, and not the other way round.
        // The failure is the seat coming out through the cloth, so what has to
        // stay inside is the seat — and at the hem's own edge the two are
        // deliberately close, which reversing the question would report as a
        // fault when it is the design.
        let hem = shirt[0].y;
        let clearance = |gait: Gait| {
            let torso = step_of(Limb::Torso, 0.0, hips, gait);
            let pelvis = step_of(Limb::Pelvis, 0.0, hips, gait);
            let into_shirt = torso.to_matrix().inverse();
            let mut tightest = f32::MAX;
            for ring in &seat {
                for step in 0..32 {
                    let (across, forward) = ring.at(TAU * step as f32 / 32.0);
                    let point = into_shirt.transform_point3(
                        pelvis.transform_point(Vec3::new(across, ring.y, forward)),
                    );
                    // Only where the shirt is actually over it. Below the hem
                    // the shorts are meant to be in the open.
                    if point.y < hem {
                        continue;
                    }
                    tightest =
                        tightest.min(Sculptor::section(&shirt, point.y).radius(point.x, point.z));
                }
            }
            tightest
        };

        // Standing, running and striking a ball: everything a player spends a
        // match doing, and the bar is a clear five per cent.
        for (what, gait) in [
            ("standing", still()),
            ("running", running(0.95)),
            ("striking", kicking(-0.4)),
            ("following through", kicking(0.6)),
            ("blowing", slumped(0.0)),
        ] {
            let out = clearance(gait);
            assert!(
                out < 0.92,
                "the seat reaches {out} of the way out through the shirt {what}"
            );
        }
        // Bent double over his own knees is a 57° stoop, and a shirt that is
        // one rigid surface cannot survive that with room to spare — real
        // cloth rides up the back and hangs off the front. Held to touching
        // rather than to clearance, which is what stops it drawing as a wedge
        // of shorts through the hip.
        let doubled = clearance(doubled_over());
        assert!(
            doubled < 1.0,
            "the seat comes {doubled} of the way through him bent double"
        );
    }

    /// **He is built like a man**, which is the one thing about this figure
    /// that cannot be judged from any single number in isolation.
    ///
    /// Two ratios say it, and both used to be a woman's. Shoulders over hips
    /// — measured where an eye measures them, across the sleeves and across
    /// the widest of the seat — runs about 1.5 on a male athlete and 1.15 on a
    /// woman; this was 1.09 on the shirt's own crest. And waist over hip as
    /// CIRCUMFERENCES is 0.85-0.95 on a man and 0.70 on a woman; this was
    /// 0.70 to two decimal places.
    ///
    /// Circumference has to be integrated rather than looked up, because the
    /// sections are not ellipses any more — see [`Ring::edge`].
    #[test]
    fn he_is_built_like_a_man() {
        let girth = |ring: Ring| {
            const STEPS: usize = 256;
            (0..STEPS)
                .map(|step| {
                    let at = |index: usize| {
                        let (across, forward) = ring.at(TAU * index as f32 / STEPS as f32);
                        Vec2::new(across, forward)
                    };
                    at(step).distance(at(step + 1))
                })
                .sum::<f32>()
        };
        let shirt = BodyParts::shirt();
        let seat = BodyParts::seat();
        // The widest of the seat, and the shoulder as the sleeve leaves it —
        // the crest of the shirt is INSIDE that and is not what anybody sees.
        let hip = seat.iter().fold(0.0f32, |widest, ring| widest.max(ring.x));
        let shoulder = Physique::SHOULDER_SPREAD + 0.0862;
        let across = shoulder / hip;
        assert!(
            (1.35..1.65).contains(&across),
            "his shoulders are {across} of his hips"
        );

        // …and the waist, which is the narrowest section of the trunk above
        // the hem, against the fullest of the seat.
        // Between the hem and the bottom of the ribs — above that the profile
        // is climbing toward the chest, and above THAT it is a neck.
        let waist = shirt
            .iter()
            .filter(|ring| (0.05..0.30).contains(&ring.y))
            .map(|ring| girth(*ring))
            .fold(f32::MAX, f32::min);
        let hips = seat.iter().map(|ring| girth(*ring)).fold(0.0, f32::max);
        let ratio = waist / hips;
        assert!(
            (0.82..0.98).contains(&ratio),
            "his waist is {waist} round and his hips {hips}, a ratio of {ratio}"
        );
    }

    /// Draws the figure from three sides, as raw RGBA, so it can be LOOKED at.
    ///
    /// See [`super::preview`] for why this exists at all. Off by default:
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_figure -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a file; run by hand when the figure changes"]
    fn dump_figure() {
        use super::preview::{Canvas, Lens, figure};

        const WIDE: usize = 260;
        const TALL: usize = 620;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);

        // Standing square on, from the side, and three-quarters on — plus a
        // stride and a kick, because a joint that is fine straight is not
        // necessarily fine bent.
        let poses: [(f32, Gait); 4] = [
            (0.0, still()),
            (FRAC_PI_2, still()),
            (0.7, running(0.9)),
            (0.7, kicking(-0.4)),
        ];
        let mut sheet = vec![0u8; WIDE * poses.len() * TALL * 4];
        for (column, (bearing, gait)) in poses.into_iter().enumerate() {
            let mut canvas = Canvas::new(WIDE, TALL);
            let lens = Lens {
                bearing,
                bottom: -0.02,
                top: 1.94,
            };
            figure(&mut canvas, &lens, &meshes, &parts, gait);
            let pixels = canvas.pixels();
            for row in 0..TALL {
                let from = row * WIDE * 4;
                let to = (row * WIDE * poses.len() + column * WIDE) * 4;
                sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
            }
        }

        let path = std::path::Path::new(&directory).join("figure.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", WIDE * 4, TALL, path.display());
    }

    /// **The TRUNK, close enough to see the cloth on it.**
    ///
    /// Every other dump here frames a whole man from head to boot, at which
    /// size a torso is sixty pixels tall and the only question it can answer
    /// is whether the pose is right. The complaints this one exists for are
    /// about the modelling rather than the rig — *"his torso looks like an
    /// articulated toy"*, *"the shirt and shorts do not look like clothes
    /// that fit a person"* — and none of them is visible at that scale.
    ///
    /// So: hips to crown, five bearings, at four times the pixels per metre
    /// the figure sheet has. What it is FOR is the three seams a kit has —
    /// the shoulder, the hem of the shirt and the hem of the shorts — plus
    /// the shoulders-against-hips ratio, which is the whole of what tells a
    /// man's silhouette from a woman's.
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_kit -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a file; run by hand when the shirt or the shorts change"]
    fn dump_kit() {
        use super::preview::{Canvas, Lens, figure};

        const WIDE: usize = 300;
        const TALL: usize = 620;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);

        // Front, three-quarter, side, back — the lathe is symmetric, so those
        // four are the whole of it — and then one running, because a hem that
        // hangs correctly on a man standing still can still ride up on him.
        let poses: [(f32, Gait); 5] = [
            (PI, still()),
            (PI * 0.72, still()),
            (FRAC_PI_2, still()),
            (0.0, still()),
            (PI * 0.78, running(0.9)),
        ];
        let mut sheet = vec![0u8; WIDE * poses.len() * TALL * 4];
        for (column, (bearing, gait)) in poses.into_iter().enumerate() {
            let mut canvas = Canvas::new(WIDE, TALL);
            let lens = Lens {
                bearing,
                bottom: 0.62,
                top: 1.86,
            };
            figure(&mut canvas, &lens, &meshes, &parts, gait);
            let pixels = canvas.pixels();
            for row in 0..TALL {
                let from = row * WIDE * 4;
                let to = (row * WIDE * poses.len() + column * WIDE) * 4;
                sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
            }
        }

        let path = std::path::Path::new(&directory).join("kit.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", WIDE * poses.len(), TALL, path.display());
    }

    /// A stride, frame by frame, plus the two ends of the amplitude spread.
    ///
    /// The run cycle is the one thing in this rig that cannot be judged
    /// from a single pose: "stiff" is a property of the sequence. Four
    /// phases of one player and then two players at the same phase — the
    /// first row answers whether the foot rolls and the trunk turns, the
    /// second whether a squad has more than one runner in it.
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_gait -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a file; run by hand when the run cycle changes"]
    fn dump_gait() {
        use super::preview::{Canvas, Lens, figure};

        const WIDE: usize = 260;
        const TALL: usize = 620;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);

        let at = |phase: f32, spring: f32| {
            let mut gait = running(0.95);
            gait.phase = phase;
            gait.spring = spring;
            gait
        };
        // Side on for the first four — a stride is a side-on picture.
        // …and a WALK, which the four columns above cannot show and which is
        // where the stride model does its work: 37% of a recorded match is
        // spent under 2.5 m/s, and until `carry_ground` existed the legs
        // covered a third of the ground the body did there. See
        // [`Gait::carry_ground`].
        let walking = |phase: f32| {
            let speed = 1.4;
            let mut gait = running((speed / Actors::SPRINT).clamp(0.0, 1.0));
            gait.phase = phase;
            gait.carry_ground = Actors::stride_of(7, speed, Vec2::Y).1;
            gait
        };
        let poses: [(f32, Gait); 8] = [
            (FRAC_PI_2, at(0.0, 1.0)),
            (FRAC_PI_2, at(FRAC_PI_2, 1.0)),
            (FRAC_PI_2, at(PI, 1.0)),
            (FRAC_PI_2, at(-FRAC_PI_2, 1.0)),
            (FRAC_PI_2, at(FRAC_PI_2, 0.86)),
            (FRAC_PI_2, at(FRAC_PI_2, 1.14)),
            (FRAC_PI_2, walking(FRAC_PI_2)),
            (FRAC_PI_2, walking(-FRAC_PI_2)),
        ];
        let mut sheet = vec![0u8; WIDE * poses.len() * TALL * 4];
        for (column, (bearing, gait)) in poses.into_iter().enumerate() {
            let mut canvas = Canvas::new(WIDE, TALL);
            let lens = Lens {
                bearing,
                bottom: -0.02,
                top: 1.94,
            };
            figure(&mut canvas, &lens, &meshes, &parts, gait);
            let pixels = canvas.pixels();
            for row in 0..TALL {
                let from = row * WIDE * 4;
                let to = (row * WIDE * poses.len() + column * WIDE) * 4;
                sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
            }
        }

        let path = std::path::Path::new(&directory).join("gait.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", WIDE * poses.len(), TALL, path.display());
    }

    /// **Eight men, one speed, one instant of the cycle.**
    ///
    /// Every other dump in this crate renders ONE player and asks whether he
    /// is drawn correctly. The complaint this answers is about the squad
    /// rather than the man — *"I want more variation in movements"* — and it
    /// is invisible in a picture of anybody. Everything except the player id
    /// is held fixed here, so the whole of what the sheet shows is how much
    /// difference `Complexion` makes.
    ///
    /// Two rows: a flat sprint, where the arms and the knees are the picture,
    /// and a walk, where the carriage is.
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_squad -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a file; run by hand when the run cycle changes"]
    fn dump_squad() {
        use super::preview::{Canvas, Lens, figure};

        const WIDE: usize = 250;
        const TALL: usize = 600;
        const MEN: usize = 8;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);

        let man = |id: u32, speed: f32| {
            let mut gait = running((speed / Actors::SPRINT).clamp(0.0, 1.0));
            gait.keeper = 0.0;
            gait.carry_ground = Actors::stride_of(id, speed, Vec2::Y).1;
            gait.spring = Complexion::spring(id);
            gait.signature = Complexion::carriage(id);
            gait.elbows = Complexion::elbows(id);
            gait.lean = Complexion::lean(id);
            gait.toes = Complexion::toes(id);
            // The SAME instant of the cycle for all of them — the phase
            // offset each player carries is a real difference, and it is not
            // the one this sheet is about.
            gait.phase = FRAC_PI_2;
            gait.idle = 0.8;
            gait
        };

        let mut sheet = vec![0u8; WIDE * MEN * TALL * 2 * 4];
        for row in 0..2 {
            // Side on for the sprint, three-quarters for the walk: a stride
            // is a sagittal picture and a carriage is not.
            let (bearing, speed) = if row == 0 {
                (FRAC_PI_2, 6.5)
            } else {
                (2.4, 1.4)
            };
            for (column, id) in (101..101 + MEN as u32).enumerate() {
                let mut canvas = Canvas::new(WIDE, TALL);
                let lens = Lens {
                    bearing,
                    bottom: -0.02,
                    top: 1.98,
                };
                figure(&mut canvas, &lens, &meshes, &parts, man(id, speed));
                let pixels = canvas.pixels();
                for line in 0..TALL {
                    let from = line * WIDE * 4;
                    let to = ((row * TALL + line) * WIDE * MEN + column * WIDE) * 4;
                    sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
                }
            }
        }

        let path = std::path::Path::new(&directory).join("squad.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", WIDE * MEN, TALL * 2, path.display());
    }
    /// The same, for the two reactions to a goal.
    ///
    /// They have their own dump because they are the one part of this rig
    /// that no assertion can finish the job on: `the_slump_keeps_his_boots_on
    /// _the_grass` and friends can prove the gloves are level with the ears
    /// and the feet are on the turf, and a man can still look like he is
    /// surrendering rather than like he has just conceded. Four angles at two
    /// joints, and the difference between the two readings is about fifteen
    /// degrees at the elbow.
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_reactions -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a file; run by hand when a reaction pose changes"]
    fn dump_reactions() {
        use super::preview::{Canvas, Lens, figure};

        const WIDE: usize = 260;
        const TALL: usize = 620;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);

        // Standing, for the comparison; then all four ways of taking a goal
        // — hands on the head square on and from the side, hands on the
        // hips, bent double over his knees, arms hanging — and the
        // celebration.
        //
        // `bearing` PI is the FRONT — bearing 0 looks at the back of a
        // player's head, which is the wrong side for reading what his hands
        // are doing. The two extra side views are there because hands on
        // the hips and hands on the knees are both poses about where the
        // hand lands, and neither reads square on.
        let poses: [(f32, Gait); 9] = [
            (PI, still()),
            (PI, slumped(1.0)),
            (FRAC_PI_2, slumped(1.0)),
            (PI, on_his_hips()),
            (FRAC_PI_2, on_his_hips()),
            (PI, doubled_over()),
            (FRAC_PI_2, doubled_over()),
            (PI, slumped(0.0)),
            (PI, cheering()),
        ];
        let mut sheet = vec![0u8; WIDE * poses.len() * TALL * 4];
        for (column, (bearing, gait)) in poses.into_iter().enumerate() {
            let mut canvas = Canvas::new(WIDE, TALL);
            let lens = Lens {
                bearing,
                bottom: -0.02,
                top: 1.94,
            };
            figure(&mut canvas, &lens, &meshes, &parts, gait);
            let pixels = canvas.pixels();
            for row in 0..TALL {
                let from = row * WIDE * 4;
                let to = (row * WIDE * poses.len() + column * WIDE) * 4;
                sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
            }
        }

        let path = std::path::Path::new(&directory).join("reactions.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", WIDE * poses.len(), TALL, path.display());
    }

    /// The dive, which is the pose the whole keeper rig exists for.
    ///
    /// Its own dump for the same reason the reactions have one: the skeleton
    /// tests can prove the gloves are outside the shoulders and no limb is
    /// under the turf, and the man can still read as falling over rather than
    /// as diving. The question *"can he get into the corner of the goal with
    /// his body horizontal"* is a question about a picture.
    ///
    /// The columns walk one dive: gathered at take-off, half extended, full
    /// stretch to his right, the same seen from behind the goal (which is the
    /// angle a viewer actually watches a save from), a top-corner dive with
    /// the lift a high ball earns him, and a forward smother at a striker's
    /// feet — the other axis of [`Carriage::placed`].
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_dive -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a file; run by hand when the dive changes"]
    fn dump_dive() {
        use super::preview::{Canvas, Lens, posed};

        const WIDE: usize = 420;
        const TALL: usize = 420;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);

        // `roll` is negated in `PlayerActor::topple` — going over onto his
        // own right is the NEGATIVE tip — so a dive to his right is a
        // negative roll here.
        let full = -Actors::SPRAWL_ANGLE;
        let poses: [(f32, Gait, Transform); 6] = [
            // Off the ground, not yet opened out.
            (
                PI,
                diving(0.9, 0.15, 0.1),
                Carriage::placed(0.0, full * 0.15, 0.18),
            ),
            // Half way through the extension.
            (
                PI,
                diving(0.9, 0.55, 0.6),
                Carriage::placed(0.0, full * 0.55, 0.30),
            ),
            // Full stretch, square on.
            (PI, diving(0.9, 1.0, 1.0), Carriage::placed(0.0, full, 0.27)),
            // …and three-quarters on, which is roughly the broadcast angle.
            (
                2.4,
                diving(0.9, 1.0, 1.0),
                Carriage::placed(0.0, full, 0.27),
            ),
            // A ball into the top corner: the engine's apex for a shot at
            // the crossbar, and the arms all the way out after it.
            (PI, diving(1.0, 1.0, 1.0), Carriage::placed(0.0, full, 0.62)),
            // Forward, at a striker's feet — the pitch axis of the topple.
            (
                PI / 2.0,
                diving(0.0, 1.0, 0.8),
                Carriage::placed(full, 0.0, 0.20),
            ),
        ];
        let mut sheet = vec![0u8; WIDE * poses.len() * TALL * 4];
        for (column, (bearing, gait, carriage)) in poses.into_iter().enumerate() {
            let mut canvas = Canvas::new(WIDE, TALL);
            let lens = Lens {
                bearing,
                bottom: -0.30,
                top: 2.50,
            };
            posed(&mut canvas, &lens, &meshes, &parts, gait, carriage, true);
            let pixels = canvas.pixels();
            for row in 0..TALL {
                let from = row * WIDE * 4;
                let to = (row * WIDE * poses.len() + column * WIDE) * 4;
                sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
            }
        }

        let path = std::path::Path::new(&directory).join("dive.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", WIDE * poses.len(), TALL, path.display());
    }

    /// **The keeper on his feet**, which is where he spends the match: the
    /// set stance, the three saves he makes standing, a parry, and the two
    /// gaits nobody else on the pitch uses.
    ///
    /// A save is half a second and a dive is rarer than that; the poses in
    /// here are the ones the camera actually holds on, and none of them had
    /// ever been rendered. Front-on (`bearing: PI`) except the shuffle,
    /// which only reads from the side — a man travelling across himself
    /// looks like a man standing still from in front.
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_keeper -- --ignored
    /// ```
    /// **One whole side-step, phase by phase.**
    ///
    /// A shuffle cannot be judged from one pose any more than a run can —
    /// what makes it read as a person rather than a linkage is what happens
    /// BETWEEN the poses, and the only way to see that is to lay the cycle
    /// out. Front-on, because a lateral gait is a frontal-plane picture, then
    /// the same cycle three-quarters on, which is the angle the broadcast
    /// camera actually watches a keeper from.
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_shuffle -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a file; run by hand when the shuffle changes"]
    fn dump_shuffle() {
        use super::preview::{Canvas, Lens, posed};

        const WIDE: usize = 300;
        const TALL: usize = 460;
        const STEPS: usize = 8;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);

        // The course a keeper actually has at each speed, through the real
        // opening band: dead abeam of the ball, so the whole of it is
        // lateral until `Actors::SQUARE_UP` starts turning his hips into the
        // run. Rendering a pure side-step at four metres a second — which
        // the first version of this did — draws a gait no human has.
        let at = |speed: f32, phase: f32| {
            let opening = Actors::ease(
                (speed - Actors::SQUARE_UP.0) / (Actors::SQUARE_UP.1 - Actors::SQUARE_UP.0),
            );
            let off = (1.0 - opening) * FRAC_PI_2;
            let course = Vec2::new(off.sin(), off.cos());
            let mut gait = travelling(
                (speed / Actors::SPRINT).clamp(0.0, 1.0),
                course.x,
                course.y,
                Actors::stride_of(7, speed, course).1,
            );
            gait.phase = phase;
            gait.idle = phase * 0.5;
            gait.set = 1.0;
            gait
        };
        let mut sheet = vec![0u8; WIDE * STEPS * TALL * 2 * 4];
        for row in 0..2 {
            let (bearing, speed) = if row == 0 { (PI, 1.3) } else { (2.5, 1.9) };
            for step in 0..STEPS {
                let gait = at(speed, step as f32 * TAU / STEPS as f32);
                let mut canvas = Canvas::new(WIDE, TALL);
                let lens = Lens {
                    bearing,
                    bottom: -0.08,
                    top: 2.05,
                };
                posed(
                    &mut canvas,
                    &lens,
                    &meshes,
                    &parts,
                    gait,
                    Transform::IDENTITY,
                    true,
                );
                let pixels = canvas.pixels();
                for line in 0..TALL {
                    let from = line * WIDE * 4;
                    let to = ((row * TALL + line) * WIDE * STEPS + step * WIDE) * 4;
                    sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
                }
            }
        }

        let path = std::path::Path::new(&directory).join("shuffle.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", WIDE * STEPS, TALL * 2, path.display());
    }

    /// **An OUTFIELDER travelling across himself**, which is a different
    /// question from [`dump_shuffle`] and a far more common one.
    ///
    /// A goalkeeper shuffles because he is square to the play and means to
    /// be. An outfielder is across his own body for one of two reasons — he
    /// is jockeying, at walking pace, or his heading has not finished coming
    /// round onto a run he is already on (`Actors::PIVOT_RATE`) — and the
    /// second is the overwhelming majority of it. Drawn as a keeper's
    /// shuffle, a man arcing round at five metres a second crouched a foot
    /// and a half with his feet a metre and a half apart at thirteen steps a
    /// second, which is how it was reported: *"they move sideways like
    /// invalids"*.
    ///
    /// Front-on, because a lateral gait is a frontal-plane picture, and then
    /// three-quarters on, which is where the broadcast camera sits and the
    /// only angle an opened hip shows from at all.
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_lateral -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a file; run by hand when the lateral gait changes"]
    fn dump_lateral() {
        use super::preview::{Canvas, Lens, posed};

        const WIDE: usize = 300;
        const TALL: usize = 460;
        const STEPS: usize = 8;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);
        // Speed, the way he is going in his own frame, and the angle to
        // watch it from. The first pair is a man jockeying; the rest are the
        // turn transient, which is what the camera actually spends its time
        // looking at. Front-on, then three-quarters — a lateral gait is a
        // frontal-plane picture, but three-quarters is where the broadcast
        // camera sits and is the only angle that shows an opened hip at all.
        let back = Vec2::new(120.0f32.to_radians().sin(), 120.0f32.to_radians().cos());
        let rows: [(f32, Vec2, f32); 8] = [
            (1.6, Vec2::X, PI),
            (1.6, Vec2::X, 2.5),
            (3.2, Vec2::X, PI),
            (5.0, Vec2::X, PI),
            (5.0, Vec2::X, 2.5),
            (5.5, Vec2::new(0.6, 0.8), 2.5),
            (5.5, back, PI),
            (5.5, back, 2.5),
        ];
        let mut sheet = vec![0u8; WIDE * STEPS * TALL * rows.len() * 4];
        for (row, (speed, course, bearing)) in rows.into_iter().enumerate() {
            for step in 0..STEPS {
                // Through the same two functions the renderer uses, so what
                // this draws is what the pitch draws.
                let open = Actors::opening(speed, course, false);
                let under = Actors::underfoot(course, open);
                let mut gait = travelling(
                    (speed / Actors::SPRINT).clamp(0.0, 1.0),
                    under.x,
                    under.y,
                    Actors::stride_of(7, speed, under).1,
                );
                gait.open = open;
                gait.phase = step as f32 * TAU / STEPS as f32;
                gait.idle = gait.phase * 0.5;
                // An outfielder: no set, no gloves, none of the keeper-only
                // pose. See [`Gait::keeper`].
                gait.keeper = 0.0;
                let mut canvas = Canvas::new(WIDE, TALL);
                let lens = Lens {
                    bearing,
                    bottom: -0.08,
                    top: 2.05,
                };
                posed(
                    &mut canvas,
                    &lens,
                    &meshes,
                    &parts,
                    gait,
                    Transform::IDENTITY,
                    false,
                );
                let pixels = canvas.pixels();
                for line in 0..TALL {
                    let from = line * WIDE * 4;
                    let to = ((row * TALL + line) * WIDE * STEPS + step * WIDE) * 4;
                    sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
                }
            }
        }

        let path = std::path::Path::new(&directory).join("lateral.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!(
            "{}x{} at {}",
            WIDE * STEPS,
            TALL * rows.len(),
            path.display()
        );
    }

    /// **The gloves, close enough to see.**
    ///
    /// Every other dump in this file frames a whole man, and at that size a
    /// hand is nine pixels across — which is exactly how five fingers welded
    /// at a fixed splay survived as long as they did. This one puts the lens
    /// on the glove itself: it asks the rig where the hand IS for each pose
    /// (`Physique::glove`) and frames a 32 cm band round it, so the camera
    /// follows the hand rather than the hand having to stay in shot.
    ///
    /// Two rows: from in FRONT of the player, which is where the palm and
    /// the spread are, and from his OUTSIDE, which is where the curl is. A
    /// grip cannot be read from one bearing any more than a shuffle can be
    /// read from one frame.
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_hands -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a file; run by hand when the hands change"]
    fn dump_hands() {
        use super::preview::{Canvas, Lens, posed};

        const WIDE: usize = 300;
        const TALL: usize = 300;
        /// How much of the world is in shot, in metres — a hand and a bit.
        const FRAME: f32 = 0.34;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);

        let mut set = still();
        set.set = 1.0;
        let mut carrying = still();
        carrying.carry = 1.0;
        let mut running = still();
        running.run = 0.8;
        running.phase = 1.1;
        let mut urging = still();
        urging.urging = 1.0;
        let mut pointing = still();
        pointing.pointing = 1.0;
        let poses: [Gait; 10] = [
            still(),
            running,
            set,
            saving(Vec2::new(0.35, 0.15), 0.0),
            saving(Vec2::new(0.85, 0.30), 1.0),
            carrying,
            diving(1.0, 1.0, 1.0),
            slumped(1.0),
            urging,
            pointing,
        ];
        // Front-on, and from the man's right — the hand the poses above are
        // asymmetric about. Then the same two for a BARE hand, which is the
        // one twenty men on the pitch are wearing and which has its own four
        // fingers and its own thumb (see [`BodyParts::hand`]); the poses are a
        // keeper's, but a hand is a hand.
        let bearings = [(PI, true), (PI / 2.0, true), (PI, false), (PI / 2.0, false)];
        let mut sheet = vec![0u8; WIDE * poses.len() * TALL * bearings.len() * 4];
        for (row, (bearing, keeper)) in bearings.into_iter().enumerate() {
            for (column, gait) in poses.into_iter().enumerate() {
                let hand = Physique::glove(1.0, gait);
                let mut canvas = Canvas::new(WIDE, TALL);
                let lens = Lens {
                    bearing,
                    bottom: hand.y - FRAME * 0.5,
                    top: hand.y + FRAME * 0.5,
                };
                posed(
                    &mut canvas,
                    &lens,
                    &meshes,
                    &parts,
                    gait,
                    // Slid so the hand is in the middle of the frame: the
                    // lens only aims up and down, and moving the figure to
                    // the origin centres it at every bearing at once.
                    Transform::from_translation(Vec3::new(-hand.x, 0.0, -hand.z)),
                    keeper,
                );
                let pixels = canvas.pixels();
                let stride = WIDE * poses.len();
                for line in 0..TALL {
                    let from = line * WIDE * 4;
                    let to = ((row * TALL + line) * stride + column * WIDE) * 4;
                    sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
                }
            }
        }

        let path = std::path::Path::new(&directory).join("hands.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!(
            "{}x{} at {}",
            WIDE * poses.len(),
            TALL * bearings.len(),
            path.display()
        );
    }

    #[test]
    #[ignore = "writes a file; run by hand when the keeper changes"]
    fn dump_keeper() {
        use super::preview::{Canvas, Lens, posed};

        const WIDE: usize = 420;
        const TALL: usize = 420;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);

        let mut set = still();
        set.set = 1.0;
        // Mid-cycle, so the legs are somewhere rather than square.
        let travel = |across: f32, ahead: f32| {
            let mut gait = travelling(
                0.45,
                across,
                ahead,
                Actors::stride_of(7, 2.6, Vec2::new(across, ahead)).1,
            );
            gait.phase = 1.9;
            gait
        };
        let mut alive = set;
        alive.idle = 0.35;
        // The two things he does with a match in which nothing is happening
        // to him, which is most of one. See [`Gait::urging`].
        let mut urging = still();
        urging.urging = 1.0;
        urging.idle = 0.9;
        let mut pointing = still();
        pointing.pointing = 1.0;
        let poses: [(f32, Gait); 11] = [
            (PI, set),
            (PI / 2.0, set),
            (PI, alive),
            (PI, saving(Vec2::new(-0.85, -0.75), 0.0)),
            (PI, saving(Vec2::new(0.1, 0.15), 0.0)),
            (PI, saving(Vec2::new(0.9, 0.85), 0.0)),
            (PI, saving(Vec2::new(0.9, 0.2), 1.0)),
            // The shuffle is a lateral pose and only reads from the FRONT;
            // the backpedal is a sagittal one and only reads from the side.
            (PI, travel(1.0, 0.0)),
            (PI / 2.0, travel(0.0, -1.0)),
            (PI, urging),
            (PI, pointing),
        ];
        let mut sheet = vec![0u8; WIDE * poses.len() * TALL * 4];
        for (column, (bearing, gait)) in poses.into_iter().enumerate() {
            let mut canvas = Canvas::new(WIDE, TALL);
            let lens = Lens {
                bearing,
                bottom: -0.10,
                top: 2.30,
            };
            posed(
                &mut canvas,
                &lens,
                &meshes,
                &parts,
                gait,
                Transform::IDENTITY,
                true,
            );
            let pixels = canvas.pixels();
            for row in 0..TALL {
                let from = row * WIDE * 4;
                let to = (row * WIDE * poses.len() + column * WIDE) * 4;
                sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
            }
        }

        let path = std::path::Path::new(&directory).join("keeper.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", WIDE * poses.len(), TALL, path.display());
    }

    /// **A keeper at full stretch is HORIZONTAL, and he is off the ground.**
    ///
    /// The user-visible claim, asserted as positions rather than as angles:
    /// at the top of a full-length dive to his right his shoulders are out
    /// past his own hips by most of a torso, both are within a hand's width
    /// of the same height, and nothing is under the grass. An angle can be
    /// right while the body it belongs to is still standing up — that is
    /// exactly the bug `Carriage::LYING` was added for.
    #[test]
    fn a_full_stretch_dive_lies_the_body_down() {
        // Apex of a real recorded dive at a low ball: 0.27 m.
        let carriage = Carriage::placed(0.0, -Actors::SPRAWL_ANGLE, 0.27);
        let gait = skeleton::diving(0.9, 1.0, 1.0);

        let hips = carriage.transform_point(Vec3::new(0.0, Physique::HIP, 0.0));
        let crown = carriage.transform_point(skeleton::crown(gait));
        let lead_glove = carriage.transform_point(skeleton::glove(1.0, gait));

        assert!(
            (crown.y - hips.y).abs() < 0.30,
            "he is not horizontal: crown at {:.2} m, hips at {:.2} m",
            crown.y,
            hips.y
        );
        assert!(
            (crown.x - hips.x).abs() > 0.35,
            "he has not gone over sideways: crown {:.2} vs hips {:.2} across",
            crown.x,
            hips.x
        );
        // The lead glove is the point of the whole exercise: it has to be
        // out past the shoulder, on the side he went, and still in the air.
        assert!(
            lead_glove.x > crown.x,
            "the lead glove is not leading: glove {:.2}, crown {:.2}",
            lead_glove.x,
            crown.x
        );
        for (what, point) in [("hips", hips), ("crown", crown), ("glove", lead_glove)] {
            assert!(
                point.y > 0.0,
                "{what} is under the turf at {:.2} m",
                point.y
            );
        }
        for side in [-1.0f32, 1.0] {
            let boot = carriage.transform_point(skeleton::boot(side, gait));
            assert!(
                boot.y > -0.02,
                "a boot is through the grass at {:.2} m",
                boot.y
            );
        }
    }

    /// The keeper AFTER the dive — the seconds he spends on the grass.
    ///
    /// The gap `dump_dive` left: every column of it is a man in the air, and
    /// a save is over in half a second while a beaten keeper is down for the
    /// better part of four (`Actors::BEATEN_HOLD`). The pose the camera holds
    /// on is the one nothing has ever drawn.
    ///
    /// The carriage is the REAL one — `PlayerActor::topple` scaled by the
    /// `flat` a recorded 12° launch earns (0.95), not the constant — because
    /// how far short of horizontal he settles is the question.
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_sprawl -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a file; run by hand when the landing changes"]
    fn dump_sprawl() {
        use super::preview::{Canvas, Lens, posed};

        const WIDE: usize = 440;
        const TALL: usize = 360;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);

        // What a real launch angle leaves of the sprawl, and what the ground
        // then does with it — the same two expressions `PlayerActor::topple`
        // composes, because the settle is the whole subject here.
        let flat = Actors::ease(1.0 - (12.0f32.to_radians() / FRAC_PI_2));
        let flying = Actors::SPRAWL_ANGLE * flat;
        let settled = |grounded: f32| {
            let committed = Actors::ease(
                (flying - Actors::GOES_OVER.0) / (Actors::GOES_OVER.1 - Actors::GOES_OVER.0),
            );
            -(flying + (FRAC_PI_2 - flying) * grounded * committed)
        };
        // …and the limbs alongside it: the extension given back as he lands,
        // the ground-out taking over. Both exactly as `PlayerActor::gait`
        // hands them over.
        let landed = |lead: f32, grounded: f32| {
            let mut gait = skeleton::diving(lead, 1.0 - grounded, 0.0);
            gait.grounded = grounded;
            gait
        };
        let poses: [(f32, Gait, Transform); 6] = [
            // Touchdown, arms still out and not yet settled.
            (
                2.4,
                skeleton::diving(0.9, 1.0, 0.6),
                Carriage::placed(0.0, settled(0.0), 0.05),
            ),
            // Halfway down.
            (
                2.4,
                landed(0.9, 0.5),
                Carriage::placed(0.0, settled(0.5), 0.0),
            ),
            // Lying there — the pose he holds. Three quarters on, square on,
            // and from his own head end.
            (
                2.4,
                landed(0.9, 1.0),
                Carriage::placed(0.0, settled(1.0), 0.0),
            ),
            (
                PI,
                landed(0.9, 1.0),
                Carriage::placed(0.0, settled(1.0), 0.0),
            ),
            (
                FRAC_PI_2,
                landed(0.9, 1.0),
                Carriage::placed(0.0, settled(1.0), 0.0),
            ),
            // And a forward smother, down on the pitch axis instead. POSITIVE
            // pitch: +X carries the head forward, and a keeper going down at a
            // striker's feet lands on his chest, not on his back.
            (
                2.4,
                landed(0.0, 1.0),
                Carriage::placed(-settled(1.0), 0.0, 0.0),
            ),
        ];
        let mut sheet = vec![0u8; WIDE * poses.len() * TALL * 4];
        for (column, (bearing, gait, carriage)) in poses.into_iter().enumerate() {
            let mut canvas = Canvas::new(WIDE, TALL);
            let lens = Lens {
                bearing,
                bottom: -0.15,
                top: 1.65,
            };
            posed(&mut canvas, &lens, &meshes, &parts, gait, carriage, true);
            let pixels = canvas.pixels();
            for row in 0..TALL {
                let from = row * WIDE * 4;
                let to = (row * WIDE * poses.len() + column * WIDE) * 4;
                sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
            }
        }

        let path = std::path::Path::new(&directory).join("sprawl.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", WIDE * poses.len(), TALL, path.display());
    }

    /// …and how far across the goal the top of that dive actually reaches,
    /// because "can he get to the corner" is a question with a number.
    ///
    /// The engine gives him the ground travel (see `GoalkeeperSpeedContext::
    /// Dive` — 2.0-2.9 m under the body); this is what the BODY adds on top
    /// of wherever it lands him, and the two together are what has to cover
    /// half a 7.32 m goal from a set position on the angle.
    #[test]
    fn the_dive_reaches_out_past_his_own_shoulder() {
        let carriage = Carriage::placed(0.0, -Actors::SPRAWL_ANGLE, 0.27);
        let gait = skeleton::diving(1.0, 1.0, 1.0);
        let glove = carriage.transform_point(skeleton::glove(1.0, gait));
        let hips = carriage.transform_point(Vec3::new(0.0, Physique::HIP, 0.0));
        let standing = skeleton::glove(1.0, skeleton::still());
        let across = glove.x - hips.x;
        assert!(
            across > 0.85,
            "the body is worth {across:.2} m across the goal, which is not a dive"
        );
        assert!(
            glove.x - standing.x > 0.60,
            "full stretch buys {:.2} m over standing there, which is not worth going for",
            glove.x - standing.x
        );
    }

    /// What one footballer costs, in triangles.
    ///
    /// A stated decision rather than whatever falls out. The figure was ~4,500
    /// triangles when the only camera was a gantry a hundred metres away; the
    /// camera can now be flown to arm's length, where sixteen flat panels
    /// round a torso and a hinge at every joint read — accurately — as a
    /// robot. This is the budget that bought a curve instead.
    ///
    /// Tripled again (35k → 104k) when the sections stopped being ellipses.
    /// A squared-off section carries nearly all of its curvature in four
    /// corners, so the facet count that read as smooth on a barrel chamfers
    /// visibly on a chest, and the corner is precisely where the shape is.
    /// [`Sculptor::SIDES`] and [`Sculptor::CURVE`] are the two knobs; both
    /// went up together, because a mesh that is fine round and coarse along
    /// looks worse than one that is evenly coarse. Then 104k → 134k for five
    /// digits on each bare hand — see [`BodyParts::hand`], and note that the
    /// hand was not in this count at all until then.
    ///
    /// It is affordable because of what it does NOT change: every mesh here is
    /// shared by all twenty-two players, so this is a few hundred thousand
    /// vertices in memory ONCE, and the ~400 draw calls a squad costs are set
    /// by the number of PARTS, not by their resolution — measured, the frame
    /// is per-entity bound and near enough resolution-insensitive. A GPU
    /// drawing twenty-two of these is putting through about 3 million
    /// triangles a frame.
    #[test]
    fn a_footballer_is_worth_his_triangles() {
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::new(&mut meshes);
        let count = |handle: &Handle<Mesh>| {
            meshes
                .get(handle)
                .and_then(|mesh| mesh.indices())
                .map_or(0, |indices| indices.len() / 3)
        };

        // Exactly what one player wears, counted once for the parts he has one
        // of and twice for the parts he has a pair of.
        let single = [&parts.torso, &parts.collar, &parts.pelvis, &parts.head]
            .iter()
            .map(|handle| count(handle))
            .sum::<usize>()
            + count(&parts.number)
            + count(&parts.name)
            // The fullest cap, since a squad wears a spread of them.
            + parts.hair.iter().flatten().map(count).max().unwrap_or(0);
        let paired = [
            &parts.upper_arm,
            &parts.sleeve,
            &parts.cuff,
            &parts.forearm,
            // The bare hand, which is what twenty of the twenty-two wear and
            // which this used to leave out of the count altogether.
            &parts.hand[1],
            &parts.glove,
            &parts.shorts_leg,
            &parts.thigh,
            &parts.shin,
            &parts.sock_top,
            &parts.boot,
        ]
        .iter()
        .map(|handle| count(handle))
        .sum::<usize>();

        let footballer = single + 2 * paired;
        assert!(
            (80_000..150_000).contains(&footballer),
            "a footballer is {footballer} triangles"
        );
        // And nothing in him is a hidden extravagance: no single part is worth
        // more than the head, which is the one anybody looks at.
        assert!(count(&parts.head) < footballer / 4);
    }

    /// The balls at the elbow and the knee are there for a limb that is BENT.
    /// Straight, they have to be invisible — a footballer standing still with
    /// a bead on each joint is a doll.
    #[test]
    fn the_joint_balls_hide_when_the_limb_is_straight() {
        // The knee ball against the two profiles that meet over it: the thigh
        // coming down and the sock going up.
        let ball = 0.048f32;
        for above in [0.010f32, 0.020, 0.030] {
            let across = ball * (1.0 - (above / ball).powi(2)).max(0.0).sqrt();
            let sock = Sculptor::section(&BodyParts::shin(), above).x;
            let thigh = 0.053 + (0.059 - 0.053) * (above / 0.035).min(1.0);
            assert!(
                across < sock.max(thigh),
                "the knee shows {across} at {above} above the joint, \
                 against a sock of {sock} and a thigh of {thigh}"
            );
        }
        // And it is big enough to be worth having: wider than the gap the two
        // tapers leave between them at the joint itself.
        assert!(ball > 0.045);
    }

    /// The face texture is laid out against the skull it wraps, and the front
    /// of that skull is a quarter of the way round the lathe.
    ///
    /// The single crossing between the mesh and the paint. Get it wrong and a
    /// squad takes the field with its eyes on the sides of its heads, which no
    /// amount of care inside the texture generator can catch.
    #[test]
    fn the_face_goes_on_the_front() {
        let head = Sculptor::part_at(&BodyParts::SKULL, Sculptor::HEAD_SIDES);
        let positions = head
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|values| values.as_float3())
            .expect("head has positions")
            .to_vec();
        let uvs = match head.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(VertexAttributeValues::Float32x2(values)) => values.clone(),
            _ => panic!("head has no texture coordinates"),
        };

        // The vertex at eye level that sticks out furthest FORWARD is the
        // middle of the face, and the texture puts the face at u = 0.25.
        let layout = BodyParts::face_layout();
        let mut front = (f32::MIN, 0.0f32);
        for (point, uv) in positions.iter().zip(&uvs) {
            if (point[1] - layout.eyes).abs() < 0.004 && point[2] > front.0 {
                front = (point[2], uv[0]);
            }
        }
        assert!(
            (front.1 - 0.25).abs() < 0.02,
            "the front of the head is at u = {}, not 0.25",
            front.1
        );

        // And `v` climbs with the model, which is why the texture is written
        // from the crown down.
        let mut lowest = (f32::MAX, 0.0f32);
        let mut highest = (f32::MIN, 0.0f32);
        for (point, uv) in positions.iter().zip(&uvs) {
            if point[1] < lowest.0 {
                lowest = (point[1], uv[1]);
            }
            if point[1] > highest.0 {
                highest = (point[1], uv[1]);
            }
        }
        assert!(lowest.1 < 0.01 && highest.1 > 0.99);
        assert!((lowest.0 - layout.foot).abs() < 1e-4);
        assert!((highest.0 - (layout.foot + layout.span)).abs() < 1e-4);
    }

    /// Every feature the texture draws is somewhere the skull actually is, in
    /// the order a face has them.
    #[test]
    fn the_features_are_in_the_right_order() {
        let layout = BodyParts::face_layout();
        assert!(layout.chin < layout.mouth);
        assert!(layout.mouth < layout.nostrils);
        assert!(layout.nostrils < layout.eyes);
        assert!(layout.eyes < layout.brow);
        assert!(layout.brow < layout.hairline);
        assert!(layout.hairline < layout.foot + layout.span);
        // The cheek half-width is what turns an angle into a distance across
        // the face, so it has to be the skull's, at eye level.
        assert!(
            (layout.cheek - Sculptor::section(&BodyParts::skull(), layout.eyes).x).abs() < 1e-6
        );
        // A face is about a fifth narrower than the head is deep, which is
        // what stops it reading as a barrel with eyes on it.
        let widest = BodyParts::SKULL
            .iter()
            .fold(0.0f32, |widest, ring| widest.max(ring.x));
        let deepest = BodyParts::SKULL
            .iter()
            .fold(0.0f32, |deepest, ring| deepest.max(ring.z));
        assert!(widest < deepest * 0.95, "{widest} across by {deepest} deep");
    }

    /// A cap of hair leaves a forehead, and the line it leaves it at is the
    /// one the face texture shades to meet.
    ///
    /// The set-back fades with height precisely so that this crossing lands
    /// somewhere a hairline belongs; a constant offset either buries the hair
    /// to the crown or sits it down over the eyebrows.
    #[test]
    fn the_hair_leaves_a_hairline() {
        let layout = BodyParts::face_layout();
        // The three caps, fullest last. Written out here rather than read back
        // off the meshes because what is being checked is the recipe.
        for (index, &(from, swell, recede)) in [
            (0.100f32, 0.005f32, 0.0498f32),
            (0.092, 0.009, 0.0586),
            (0.084, 0.017, 0.0790),
        ]
        .iter()
        .enumerate()
        {
            let rings = BodyParts::cap_rings(from, swell, recede);
            // Where the cap comes out through the skin, straight up the front
            // of the head. Searched from the brow, because a fringe below that
            // is hair over the eyes.
            let emerges = (0..300)
                .map(|step| layout.brow + step as f32 * 0.0005)
                .find(|&y| {
                    let hair = Sculptor::section(&rings, y);
                    let skull = Sculptor::section(&BodyParts::skull(), y);
                    hair.offset + hair.z > skull.offset + skull.z
                })
                .unwrap_or_else(|| panic!("cap {index} never comes out at the front"));
            // At or below the line the face texture shades to — never above
            // it, which would leave a band of lit forehead with the hair
            // starting somewhere else entirely. Below is free: the shading
            // simply ends up underneath the cap.
            assert!(
                emerges < layout.hairline + 0.008,
                "cap {index} starts at {emerges}, above the shaded line at {}",
                layout.hairline
            );
            assert!(
                emerges > layout.brow + 0.004,
                "cap {index} comes down over the eyebrows, at {emerges}"
            );
            // And it covers the sides from the eye line up, which is where
            // the top of an ear is and where hair grows — without hanging out
            // behind the skull, which is what setting the whole ring back
            // used to do.
            // Nowhere above the hairline does the skull come back out through
            // the cap. A ring of bare scalp at the crown — which a cap that
            // closed in one shallow cone left — is a tonsure, and it is
            // invisible in every test that only looks at the front.
            for step in 0..=60 {
                let top = layout.foot + layout.span - 0.0001;
                let y = emerges + (top - emerges) * step as f32 / 60.0;
                let hair = Sculptor::section(&rings, y);
                let skull = Sculptor::section(&BodyParts::skull(), y);
                for turn in 0..12 {
                    let angle = turn as f32 * PI / 6.0;
                    // Where the skull's surface sits inside the cap's ellipse:
                    // under 1 is covered, over 1 is hair growing out of a
                    // scalp that is showing through it.
                    let across = angle.cos() * skull.x / hair.x.max(1e-4);
                    let along =
                        (skull.offset + angle.sin() * skull.z - hair.offset) / hair.z.max(1e-4);
                    assert!(
                        across * across + along * along < 1.0,
                        "cap {index} leaves scalp showing at y {y}, {angle} rad round"
                    );
                }
            }

            let temple = Sculptor::section(&rings, layout.eyes);
            let skull = Sculptor::section(&BodyParts::skull(), layout.eyes);
            assert!(temple.x > skull.x, "cap {index} is bald at the temple");
            assert!(
                temple.offset - temple.z > skull.offset - skull.z - swell - 0.002,
                "cap {index} is a mullet: it reaches {} behind a skull that stops at {}",
                temple.offset - temple.z,
                skull.offset - skull.z
            );
        }
    }

    /// The man on the ball does not run like the twenty-one who have not got
    /// it: lower over it, arms wider, and genuinely shorter.
    #[test]
    fn the_carrier_runs_over_the_ball() {
        let mut gait = still();
        gait.carrying = 1.0;
        assert!(crown(gait).y < crown(still()).y - 0.02, "not lower");
        assert!(crown(gait).z > crown(still()).z + 0.05, "not over it");
        assert!(
            glove(1.0, gait).x.abs() > glove(1.0, still()).x.abs() + 0.02,
            "arms not out"
        );
        // But still a footballer running, not a man in a crouch.
        assert!(crown(gait).y > crown(still()).y - 0.10);
    }
}
