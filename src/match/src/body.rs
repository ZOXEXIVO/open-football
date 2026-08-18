use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
#[cfg(test)]
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::actors::Actors;
use crate::kit::Outfit;
use crate::textures::FaceLayout;

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
}

impl Ring {
    const fn round(y: f32, radius: f32) -> Self {
        Ring {
            y,
            x: radius,
            z: radius,
            offset: 0.0,
        }
    }

    const fn oval(y: f32, x: f32, z: f32) -> Self {
        Ring {
            y,
            x,
            z,
            offset: 0.0,
        }
    }

    /// An ellipse carried `offset` metres in front of the axis.
    const fn set(y: f32, x: f32, z: f32, offset: f32) -> Self {
        Ring { y, x, z, offset }
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
        }
    }

    /// The section `t` of the way from this one to `other`.
    fn lerp(self, other: Ring, t: f32) -> Ring {
        Ring {
            y: self.y + (other.y - self.y) * t,
            x: self.x + (other.x - self.x) * t,
            z: self.z + (other.z - self.z) * t,
            offset: self.offset + (other.offset - self.offset) * t,
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
                let angle = TAU * step as f32 / 16.0;
                let across = angle.cos() * self.x / cap.x;
                let along = (self.offset + angle.sin() * self.z - cap.offset) / cap.z;
                worst = worst.max(across.hypot(along));
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
    const SIDES: usize = 32;
    /// And for the head, which carries a face.
    ///
    /// At sixteen the front of a skull was four vertices wide, so an eye
    /// landed between two of them and the texture sheared across the facets.
    /// A head is also the one part of a footballer anybody looks AT.
    const HEAD_SIDES: usize = 48;
    /// And for the small round parts — see [`Sculptor::ellipsoid`].
    const BLOB_SIDES: usize = 24;
    /// Rings from pole to pole on a sphere.
    const STACKS: usize = 14;
    /// How many rings are lathed between each pair a profile is written with.
    ///
    /// The control points are the shape as it is AUTHORED — a dozen numbers a
    /// human can read and edit — and the mesh is the smooth curve through
    /// them rather than the polyline between them. That distinction is most of
    /// what separates a limb from a stack of truncated cones: the silhouette
    /// error of a straight span is second order and invisible, but the CREASE
    /// where two spans meet is first order and is exactly what the eye reads
    /// as "assembled out of parts".
    const CURVE: usize = 3;

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

    /// A rounded lump — a hand, a boot, the ball of a joint — sized on each
    /// axis.
    ///
    /// Coarser than the lathed parts on purpose: there are ten of these on a
    /// footballer and none of them is more than six centimetres across, where
    /// twenty-four sides put the silhouette error under half a millimetre.
    fn ellipsoid(radii: Vec3) -> Mesh {
        Self::lathe(&Self::sphere(radii), Self::BLOB_SIDES)
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
        const ROWS: usize = 5;
        const COLUMNS: usize = 12;

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
                sculptor.positions.push([
                    angle.cos() * (section.x + lift),
                    y,
                    section.offset + angle.sin() * (section.z + lift),
                ]);
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
                self.positions.push([
                    offset.x + angle.cos() * ring.x,
                    offset.y + ring.y,
                    offset.z + ring.offset + angle.sin() * ring.z,
                ]);
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
            self.positions.push([
                offset.x + angle.cos() * ring.x,
                offset.y + ring.y,
                offset.z + ring.offset + angle.sin() * ring.z,
            ]);
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
    /// It is what turns ground covered into an angle at the hip and back
    /// again, so it is the crossing between the stride model in
    /// [`crate::actors::Actors::stride_of`] and the geometry here — and,
    /// like [`Self::CRADLE`], a number that has to be one number.
    pub const LEG: f32 = Self::THIGH + Self::SHIN - 0.005 + 0.038;
    /// Hip to the base of the neck.
    pub const TORSO: f32 = 0.58;
    /// Shoulder joints, in the torso's own space.
    ///
    /// Deliberately sunk BENEATH and INSIDE the torso's shoulder crest
    /// (which peaks 0.209 wide at y=0.512). An arm socket level with the
    /// crest, as this was, leaves the top cap of the upper arm standing
    /// proud of the shirt as a separate rounded lump — the single thing
    /// that made the figures read as assembled out of parts. Buried, the
    /// only bit of arm you see above the armpit is the deltoid, which is
    /// how a shoulder actually looks.
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
    /// 32 cm across — see [`crate::actors::Actors::BALL_RADIUS`] — so a centre
    /// on the wrist swallows the whole glove and half the forearm with it.
    pub fn palm(side: f32, gait: Gait) -> Vec3 {
        Self::hand(side, gait).transform_point(Vec3::new(0.0, -Self::PALM, 0.0))
    }

    /// And where a ball held in BOTH is: between them.
    pub fn hands(gait: Gait) -> Vec3 {
        (Self::palm(-1.0, gait) + Self::palm(1.0, gait)) * 0.5
    }

    /// One hand, walked forward down the arm.
    fn hand(side: f32, gait: Gait) -> Transform {
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
pub struct BodyParts {
    torso: Handle<Mesh>,
    /// The neck of the shirt, in the club's trim colour. Derived from the
    /// torso rather than written out again — see [`Sculptor::band`].
    collar: Handle<Mesh>,
    pelvis: Handle<Mesh>,
    head: Handle<Mesh>,
    /// A nose and a pair of ears. Tiny, and between them most of what makes a
    /// head read as a head from the side rather than as an egg with a face
    /// painted on the front of it.
    nose: Handle<Mesh>,
    ear: Handle<Mesh>,
    /// One cap per hair style; `None` for the shaved head, which is the scalp
    /// itself with the stubble drawn onto the face texture.
    hair: [Option<Handle<Mesh>>; 4],
    upper_arm: Handle<Mesh>,
    /// The two joints that bend far enough to open a gap between the tapers
    /// either side of them.
    elbow: Handle<Mesh>,
    knee: Handle<Mesh>,
    sleeve: Handle<Mesh>,
    /// The band round the end of a sleeve, in the trim colour.
    cuff: Handle<Mesh>,
    forearm: Handle<Mesh>,
    hand: Handle<Mesh>,
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
    /// One finger of it, instanced four times across the knuckles, and the
    /// thumb, which is shorter and set forward and in.
    finger: Handle<Mesh>,
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
    /// The shoulder line is the whole problem with a figure this size. It used
    /// to go 0.196 wide at y=0.530 and 0.150 at y=0.558 — a 23% collapse
    /// across 28 mm — which is a coat-hanger, not a shoulder: a flat shelf
    /// ending in a cliff, with the arm hung off the edge of it. Real shoulders
    /// are a dome. The trapezius leaves the neck and falls away over a good
    /// ten centimetres, and the widest point is the acromion, out at the very
    /// end of the collarbone, with a rounded crest over it. So the crest is
    /// broad and slightly domed (0.482-0.535 all within 4% of each other) and
    /// the run into the neck takes three rings instead of one.
    ///
    /// The offsets are the second half of the same argument. A chest is not
    /// centred on the spine: the pectorals stand a good centimetre in front of
    /// the axis and the shoulder blades hang behind it, so the deepest part of
    /// the back is UP at the blades and not down at the waist. Lathed
    /// symmetrically — as this was — the same numbers describe a barrel.
    ///
    /// A constant rather than an argument to one call, because four other
    /// things are derived from it: the collar, the two printed panels, and
    /// nothing may be allowed to drift out of step with the cloth it sits on.
    const SHIRT: [Ring; 13] = [
        Ring::set(0.050, 0.138, 0.094, 0.005),
        Ring::set(0.070, 0.134, 0.092, 0.006),
        Ring::set(0.140, 0.131, 0.089, 0.006),
        Ring::set(0.220, 0.148, 0.100, 0.004),
        Ring::set(0.300, 0.168, 0.109, 0.006),
        Ring::set(0.370, 0.185, 0.114, 0.008),
        Ring::set(0.440, 0.198, 0.113, 0.004),
        Ring::set(0.482, 0.207, 0.109, -0.002),
        Ring::set(0.512, 0.209, 0.104, -0.004),
        Ring::set(0.535, 0.201, 0.099, -0.004),
        Ring::set(0.552, 0.176, 0.093, -0.004),
        Ring::set(0.568, 0.134, 0.082, -0.006),
        Ring::set(0.580, 0.088, 0.070, -0.008),
    ];

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
    const SKULL: [Ring; 13] = [
        Ring::set(-0.075, 0.052, 0.056, -0.004),
        Ring::set(-0.030, 0.050, 0.054, -0.006),
        Ring::set(0.005, 0.052, 0.058, -0.008),
        Ring::set(0.028, 0.062, 0.070, 0.000),
        Ring::set(0.048, 0.072, 0.082, 0.004),
        Ring::set(0.072, 0.082, 0.090, 0.004),
        Ring::set(0.100, 0.088, 0.096, 0.000),
        Ring::set(0.130, 0.090, 0.099, -0.002),
        Ring::set(0.158, 0.089, 0.098, -0.004),
        Ring::set(0.185, 0.084, 0.094, -0.006),
        Ring::set(0.212, 0.073, 0.083, -0.008),
        Ring::set(0.238, 0.052, 0.060, -0.010),
        Ring::set(0.255, 0.000, 0.000, -0.012),
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
    const HAIRLINE: f32 = 0.198;
    /// Where the nose is hung, and the ears.
    const NOSE_AT: Vec3 = Vec3::new(0.0, 0.128, 0.078);
    const EAR_AT: Vec3 = Vec3::new(0.086, 0.106, -0.012);

    /// Where the print goes on the back of the shirt: the name across the
    /// shoulders and the number under it, both in the torso's own space.
    ///
    /// The arcs decide the panels' widths, since the ink follows the cloth
    /// round the body — 1.45 radians at the number's height is a chord of
    /// 23 cm and an arc of 23.3, which is a real shirt number. Whoever draws
    /// the textures has to match that aspect or the glyphs come out stretched;
    /// see [`crate::textures::Textures::number`].
    const NAME_AT: f32 = 0.464;
    const NAME_HEIGHT: f32 = 0.058;
    const NAME_ARC: f32 = 1.34;
    const NUMBER_AT: f32 = 0.316;
    const NUMBER_HEIGHT: f32 = 0.190;
    const NUMBER_ARC: f32 = 1.45;
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
            // Derived from the shirt it sits on, as the sock turnover is —
            // but starting at 0.576 rather than a comfortable-looking way
            // down, because the torso is still 25 cm across at 0.570 and a
            // band taken from there is not a collar, it is a yoke across the
            // shoulders. The whole taper from the shoulder crest to the neck
            // happens inside seven centimetres, so the collar has to live in
            // the top two of them.
            collar: meshes.add(Sculptor::part(&[
                Ring::set(0.576, 0.108, 0.078, -0.0075),
                Ring::set(0.583, 0.095, 0.073, -0.0080),
                Ring::set(0.591, 0.087, 0.068, -0.0080),
                Ring::set(0.599, 0.081, 0.064, -0.0080),
            ])),
            // The seat of the shorts, which stays put while the legs swing.
            //
            // Wide enough to CONTAIN the tops of the two legs of the shorts,
            // which it was not. The hips are 88 mm apart and a leg of cloth
            // round a thigh is a hundred wide, so anything narrower than about
            // 0.19 has the two tubes cutting out through its sides — and two
            // nearly tangent surfaces crossing draw as a hard rectangular
            // notch, which is what sat on the front of every pair of shorts on
            // the pitch. Widened, the legs emerge under the seat's own hem
            // instead, which is where a leg of a pair of shorts emerges.
            pelvis: meshes.add(Sculptor::part(&[
                Ring::set(0.100, 0.146, 0.100, 0.004),
                Ring::set(0.030, 0.172, 0.113, 0.000),
                Ring::set(-0.050, 0.192, 0.122, -0.006),
                Ring::set(-0.115, 0.186, 0.117, -0.006),
                Ring::set(-0.152, 0.163, 0.104, -0.004),
            ])),
            head: meshes.add(Sculptor::part_at(&Self::SKULL, Sculptor::HEAD_SIDES)),
            // Bridge, ball, nostrils. Two centimetres of geometry, and the
            // difference between a face and a mask: a nose is the one feature
            // that survives being seen from the side, which is where a
            // footballer spends most of a match relative to the camera.
            nose: meshes.add(Sculptor::part_at(
                &[
                    Ring::set(0.030, 0.009, 0.011, 0.000),
                    Ring::set(0.012, 0.012, 0.014, 0.006),
                    Ring::set(-0.008, 0.016, 0.018, 0.013),
                    Ring::set(-0.022, 0.019, 0.019, 0.015),
                    Ring::set(-0.033, 0.020, 0.015, 0.010),
                    Ring::set(-0.040, 0.015, 0.008, 0.002),
                ],
                12,
            )),
            ear: meshes.add(Sculptor::ellipsoid(Vec3::new(0.009, 0.030, 0.020))),
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
                Some(meshes.add(Self::cap(0.098, 0.005, 0.0426))),
                Some(meshes.add(Self::cap(0.090, 0.009, 0.0518))),
                Some(meshes.add(Self::cap(0.082, 0.017, 0.0586))),
            ],
            // Deltoid, bicep, taper to the elbow.
            //
            // The top ring is now narrow (0.030) and sits high, so it ends up
            // inside the torso and its cap is never seen; the deltoid swells
            // 20 mm BELOW the socket, which is where a shoulder muscle
            // actually sits. Previously the widest ring was 5 mm below the
            // top, so the arm's broadest point was level with its own
            // socket and the join showed as a seam.
            upper_arm: meshes.add(Sculptor::part(&[
                Ring::round(0.055, 0.030),
                Ring::round(0.020, 0.050),
                Ring::round(-0.020, 0.057),
                Ring::round(-0.075, 0.054),
                Ring::round(-0.140, 0.049),
                Ring::round(-0.215, 0.044),
                Ring::round(-0.280, 0.040),
                Ring::round(-0.300, 0.037),
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
            sleeve: meshes.add(Sculptor::part(&[
                Ring::oval(0.056, 0.026, 0.026),
                Ring::oval(0.042, 0.050, 0.049),
                Ring::oval(0.020, 0.068, 0.066),
                Ring::oval(-0.015, 0.078, 0.075),
                Ring::oval(-0.080, 0.074, 0.071),
                Ring::oval(-0.126, 0.067, 0.065),
            ])),
            // And the band round the end of it. The same trim as the collar,
            // and the pair of them together are what say "kit" rather than
            // "coloured shape" at any distance a face is legible from. Rolled
            // under at the hem, the way a sewn edge is, so it does not end in
            // a flat washer hanging round the arm.
            cuff: meshes.add(Sculptor::part(&[
                Ring::oval(-0.098, 0.0735, 0.0712),
                Ring::oval(-0.126, 0.0705, 0.0684),
                Ring::oval(-0.140, 0.0672, 0.0652),
                Ring::oval(-0.147, 0.0600, 0.0582),
            ])),
            // The elbow is the NARROW point of an arm and the forearm's belly
            // sits below it. Started at its widest, as this did, the forearm
            // was wider than the arm it hangs off and the join showed as a
            // step right round the elbow — a hinge, on every player, at the
            // one place a limb is supposed to look continuous.
            forearm: meshes.add(Sculptor::part(&[
                Ring::round(0.014, 0.033),
                Ring::round(-0.014, 0.042),
                Ring::round(-0.055, 0.046),
                Ring::round(-0.140, 0.038),
                Ring::round(-0.215, 0.031),
                Ring::round(-0.245, 0.028),
                Ring::round(-0.262, 0.024),
            ])),
            // A ball at each of the two joints that bend, filling the gap the
            // two tapers leave between them when they do. Without it an arm
            // bent through a right angle opens a wedge at the outside of the
            // elbow, and a knee lifted into a stride opens the same wedge in
            // front of the kneecap.
            // Sized to sit just inside the two tapers when the limb is
            // straight — the thigh is 0.056 across two centimetres above the
            // knee and the sock 0.060 — so a standing player never shows one,
            // and a bent one shows nothing else.
            elbow: meshes.add(Sculptor::ellipsoid(Vec3::splat(0.038))),
            knee: meshes.add(Sculptor::ellipsoid(Vec3::splat(0.048))),
            hand: meshes.add(Sculptor::ellipsoid(Vec3::new(0.035, 0.050, 0.028))),
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
            // One finger: padded at the base, rounded at the tip. Modelled
            // about its own root so the splay below is a rotation and not a
            // second set of numbers.
            finger: meshes.add(Sculptor::part_at(
                &[
                    Ring::oval(0.006, 0.015, 0.013),
                    Ring::oval(-0.014, 0.016, 0.014),
                    Ring::oval(-0.044, 0.015, 0.013),
                    Ring::oval(-0.064, 0.013, 0.011),
                    Ring::oval(-0.074, 0.009, 0.008),
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
            sleeve_long: meshes.add(Sculptor::part(&[
                Ring::oval(0.056, 0.026, 0.026),
                Ring::oval(0.042, 0.050, 0.049),
                Ring::oval(0.020, 0.068, 0.066),
                Ring::oval(-0.015, 0.078, 0.075),
                Ring::oval(-0.080, 0.062, 0.060),
                Ring::oval(-0.150, 0.055, 0.053),
                Ring::oval(-0.225, 0.050, 0.048),
                Ring::oval(-0.290, 0.045, 0.044),
            ])),
            sleeve_forearm: meshes.add(Sculptor::part(&[
                Ring::oval(0.050, 0.044, 0.043),
                Ring::oval(0.010, 0.050, 0.049),
                Ring::oval(-0.060, 0.052, 0.051),
                Ring::oval(-0.150, 0.045, 0.044),
                Ring::oval(-0.225, 0.038, 0.037),
                Ring::oval(-0.252, 0.035, 0.034),
                Ring::oval(-0.266, 0.029, 0.028),
            ])),
            cuff_forearm: meshes.add(Sculptor::part(&[
                Ring::oval(-0.212, 0.0405, 0.0395),
                Ring::oval(-0.240, 0.0375, 0.0365),
                Ring::oval(-0.256, 0.0345, 0.0335),
                Ring::oval(-0.266, 0.0290, 0.0282),
            ])),
            // The leg of the shorts: it belongs to the thigh, not to the hips.
            //
            // Its top ring stops UNDER the shirt's hem. Four centimetres
            // higher, as it was, it stood up inside the shirt and the two
            // surfaces crossed each other somewhere round the hip — two nearly
            // tangent surfaces interpenetrating, which is the one arrangement
            // that no amount of depth precision draws cleanly. The waist came
            // out as a ragged blue-and-white sawtooth at any range close
            // enough to see it. A footballer tucks his shirt in, and now so
            // does this one: the hem ends at 0.050 and the shorts start above
            // it, so nothing crosses anything.
            // Rolled under at the hem for the same reason the cuff is: a leg
            // of cloth that simply stops is a tube with a hole in the end of
            // it, and from below that hole is what you see.
            shorts_leg: meshes.add(Sculptor::part(&[
                Ring::oval(0.012, 0.097, 0.093),
                Ring::oval(-0.045, 0.110, 0.102),
                Ring::oval(-0.130, 0.116, 0.106),
                Ring::oval(-0.196, 0.110, 0.100),
                Ring::oval(-0.216, 0.099, 0.090),
            ])),
            // Quadriceps high on the thigh, narrowing into the knee — and the
            // whole muscle carried a few millimetres forward of the bone,
            // which is where it is.
            thigh: meshes.add(Sculptor::part(&[
                Ring::set(-0.090, 0.089, 0.088, 0.000),
                Ring::set(-0.160, 0.086, 0.086, 0.002),
                Ring::set(-0.250, 0.078, 0.078, 0.003),
                Ring::set(-0.340, 0.068, 0.068, 0.002),
                Ring::set(-0.420, 0.059, 0.059, 0.000),
                Ring::set(-0.455, 0.053, 0.053, 0.000),
            ])),
            shin: meshes.add(Sculptor::part(&Self::SHIN)),
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
        /// crown, which stands a little above the bare one.
        const SHOULDER: f32 = 0.246;

        let crown = Self::SKULL[Self::SKULL.len() - 1];
        // Sampled finely, and it has to be. Both profiles are lofted through
        // curves rather than in straight lines, but a cap that took only a
        // handful of samples of the head it covers would still cut inside it
        // between them — and where it does, a ring of bare scalp comes through
        // the hair. Every player took the field wearing a tonsure once
        // already; see `the_hair_leaves_a_hairline`.
        let skull = Self::skull();
        let span = SHOULDER - from;
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
        // Two more rings over the top of the skull before the cap closes.
        //
        // Without them the last band runs straight from a three-centimetre
        // ring to a point in twelve millimetres — a cone so shallow it is
        // effectively a lid, and it leaves a ring of bare scalp showing
        // between itself and the rest of the cap, because a straight-sided
        // lid cannot follow a dome. Every player with hair took the field
        // wearing a tonsure.
        for over in [0.250f32, 0.2535] {
            rings.push(Sculptor::section(&skull, over).capped(swell, -swell));
        }
        rings.push(Ring::set(crown.y + swell * 0.9, 0.0, 0.0, crown.offset));
        // And one BELOW the cap, tucked inside the head all the way round.
        //
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
    /// space: four across the knuckle line and a thumb off the inside edge,
    /// each splayed a little so the hand reads as open rather than as a
    /// paddle with grooves cut in it.
    ///
    /// One place, because [`Footballer::assemble`] and the software preview
    /// both walk this hierarchy — a part added to one and not the other
    /// simply does not appear in the pictures the tests draw.
    ///
    /// `true` means the thumb, which is the other mesh.
    pub fn digits(side: f32) -> [(bool, Transform); 5] {
        let knuckle = |across: f32| {
            (
                false,
                Transform::from_translation(Vec3::new(across, Self::KNUCKLES, 0.004))
                    .with_rotation(Quat::from_rotation_z(across * Self::SPLAY)),
            )
        };
        [
            knuckle(-0.042),
            knuckle(-0.014),
            knuckle(0.014),
            knuckle(0.042),
            (
                true,
                Transform::from_translation(Vec3::new(-side * 0.046, -0.026, 0.016)).with_rotation(
                    Quat::from_rotation_z(-side * 0.95) * Quat::from_rotation_x(-0.30),
                ),
            ),
        ]
    }

    /// The knuckle line, in the wrist's own space — where the glove ends and
    /// the fingers start — and how far a finger splays per metre it sits off
    /// the middle of the hand.
    const KNUCKLES: f32 = -0.086;
    const SPLAY: f32 = 3.4;

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
    /// goalkeeper — see [`crate::actors::Actors::animate`] for how a dive is
    /// told from a run.
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
    /// The only thing in this rig that is not derived from the position
    /// track, and it cannot be: eleven men standing still because they are
    /// sick and eleven standing still because they are waiting look
    /// identical from above. See [`Aftermath`](crate::aftermath::Aftermath)
    /// for where the signal comes from.
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
    /// Which slump: 0 hands on the hips, 1 hands on the head.
    ///
    /// Kept as a hard 0 or 1 rather than a blend, because the two poses are
    /// far apart and the interpolation between them is a man holding his
    /// arms out sideways, which is neither. Every keeper takes the second
    /// one — it is the picture of a beaten goalkeeper, and he is the man the
    /// camera is on.
    pub hands_to_head: f32,
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
    /// lateral movement, and the rig had no representation of it at all.
    /// A keeper covering his line is square to the play and MEANS to be: he
    /// side-steps, feet never crossing, and every constant in
    /// [`Joint::SHUFFLE_STANCE`] and below is about him. Nobody else on the
    /// pitch is doing that. An outfielder is across his own body for one of
    /// two reasons — he is jockeying, at walking pace, or his heading has
    /// not finished coming round onto a run he is already on, which
    /// [`crate::actors::Actors::PIVOT_RATE`] guarantees will happen every
    /// time anyone changes direction at speed — and the second is the
    /// overwhelming majority of it. **A man arcing round a turn is running,
    /// not shuffling.** Drawn as a keeper's shuffle he crouched a foot and a
    /// half with his feet a metre and a half apart at thirteen steps a
    /// second, which is how it was reported: *"they move sideways like
    /// invalids"*.
    ///
    /// So the legs turn onto the course and the chest does not, which is
    /// what "opening the hips" means and what the hips are for. It is not a
    /// second gait with a switch between them: the course is rotated by this
    /// same angle before anything reads it (see
    /// [`crate::actors::Actors::underfoot`]), so at a full opening the
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

impl Joint {
    /// The angle the leading leg swings through, standing and at a sprint.
    const HIP_SWING: (f32, f32) = (0.10, 0.62);
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
    const ELBOW_FLEX: (f32, f32) = (0.25, 1.25);
    const LEAN: (f32, f32) = (0.045, 0.20);
    /// How far a running player's whole body rises as the stride closes up,
    /// in metres.
    const BOB: f32 = 0.06;
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
    const SET_SHOULDER: f32 = -0.62;
    const SET_SPREAD: f32 = 0.40;
    const SET_ELBOW: f32 = -1.10;
    const SET_WRIST: f32 = -0.40;
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
    /// radians, and it is a constant rather than a curve.
    ///
    /// The one thing a lateral gait genuinely does ask the knees for.
    /// Everything else in this rig bends a knee through `tuck`, which is
    /// keyed to where the leg is in a STRIDE — right for a run, where the
    /// knee folds through the swing and straightens onto the plant, and
    /// wrong for a side-step, where there is no swing through the sagittal
    /// plane to fold into. Driven off `tuck` at the share of a run cycle a
    /// side-step used to be given, the stance knee swung between 4° and 53°
    /// on every step, which is a man's legs buckling under him. A man
    /// shuffling holds them softly bent and keeps them there.
    ///
    /// Paid for in height by [`Joint::shuffle_drop`], like every other
    /// bend in this rig that is a real loss rather than a pose.
    const SHUFFLE_KNEE: f32 = 0.30;
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
    /// **The slump.** A man who has just conceded, in two variants.
    ///
    /// Both fold the trunk forward and drop the chin; what differs is what
    /// the arms do, which is the whole read at broadcast distance.
    ///
    /// **Hands to the head** is the keeper's, and the one everybody
    /// pictures: upper arms up past the ears, forearms folded back so the
    /// gloves come onto the crown. **Limp** is the other half of a conceding
    /// eleven: arms simply hanging, shoulders rolled in, head down, walking.
    ///
    /// Hands on the HIPS was tried and dropped, and the reason is worth
    /// keeping: this rig has no roll about the arm's own long axis, so with
    /// the elbow out to the side the forearm can only point forward and
    /// OUT. It cannot bring the hand in to the waist, and what it draws
    /// instead is a man holding an invisible tray. A pose the skeleton
    /// cannot reach is not a pose.
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
                let bob =
                    Self::BOB * gait.run * gait.spring * (0.5 + 0.5 * (gait.phase * 2.0).cos());
                // Breathing, for a player who is not running. Fades out as he
                // does, where the stride bob takes over.
                let breathe = Self::BREATHE * (1.0 - gait.run) * (0.5 + 0.5 * gait.idle.sin());
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
                            - Self::SET_DROP * gait.set
                            - Self::CARRY_DROP * gait.carrying
                            - Self::SLUMP_DROP * gait.despair
                            // A wide base is a low one, and a keeper going
                            // down to a ball at his boots loses most of a
                            // hand's width of height doing it. Both are real
                            // losses rather than poses, so they are paid for
                            // here — the same reason `SET_DROP` is.
                            - Self::splay_drop(gait)
                            - Self::shuffle_drop(gait)
                            - Self::SAVE_DROP * gait.save * Self::stooping(gait)
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
        let lean = Self::blend(Self::LEAN, gait.run);

        // Weight going from one foot to the other, and back. Half the idle
        // rate, because a shift is a whole cycle where a breath is half of
        // one. Fades out the moment he starts running.
        let standing = 1.0 - gait.run;
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
        // The two halves of the slump, split rather than blended — see
        // [`Gait::hands_to_head`]. Both are zero for every player for all but
        // the few seconds after a goal, so every layer they drive
        // short-circuits inside [`Self::held`].
        let limp = gait.despair * (1.0 - gait.hands_to_head);
        let on_head = gait.despair * gait.hands_to_head;

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
                    Self::HIP_TWIST * gait.run * gait.spring * gait.phase.sin() * gait.course.y
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
                let roll = Self::ROCK * gait.run * (gait.phase + FRAC_PI_2).sin()
                    + Self::WEIGHT_SHIFT * 0.8 * weight
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
                let settle =
                    (1.0 - gait.carry) * (1.0 - gait.dive) * (1.0 - gait.set) * (1.0 - gait.save);
                let running = Quat::from_rotation_x(lean * (1.0 + 0.16 * gait.signature) * settle)
                    * Quat::from_rotation_y(
                        -Self::CHEST_TWIST
                            * gait.run
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
                        Self::SET_LEAN * gait.set * (1.0 - gait.save * gait.save_aim.y.max(0.0)),
                    )
                    * Quat::from_rotation_x(
                        Self::DIVE_ARCH * gait.stretch * (1.0 - gait.grounded),
                    )
                    * Quat::from_rotation_y(
                        Self::DIVE_TWIST * gait.lead * gait.stretch * (1.0 - 0.5 * gait.grounded),
                    )
                    * Quat::from_rotation_x(Self::DOWN_CURL * gait.grounded)
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
                    * Quat::from_rotation_x(
                        Self::SLUMP_STOOP * gait.despair + Self::CHEER_ARCH * gait.elation,
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
                let steady = -Self::ROCK * gait.run * (gait.phase + FRAC_PI_2).sin();
                Quat::from_rotation_y(
                    gait.look
                        - Self::DIVE_TWIST * gait.lead * gait.stretch * (1.0 - 0.5 * gait.grounded),
                ) * Quat::from_rotation_x(-lean * 0.75 - gait.look_pitch)
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
                        Self::SLUMP_HEAD_DOWN * gait.despair
                            + Self::CHEER_HEAD_UP * gait.elation,
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
                let arm = Self::strides(Self::ARM_SWING, gait) * swing * gait.course.y * asymmetry
                    + drift
                    + counter;
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
                let cheering = Self::held(
                    slumped,
                    Quat::from_rotation_z(-self.side * Self::CHEER_SPREAD)
                        * Quat::from_rotation_x(Self::CHEER_SHOULDER),
                    gait.elation,
                );
                let ready = Self::held(
                    cheering,
                    Quat::from_rotation_z(self.side * Self::SET_SPREAD)
                        * Quat::from_rotation_x(Self::SET_SHOULDER),
                    Self::armed(gait),
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
                    -Self::blend(Self::ELBOW_FLEX, gait.run) * (1.0 + 0.24 * gait.signature)
                        - 0.18 * gait.run * swing
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
                let running = Self::held(
                    slumped,
                    Quat::from_rotation_x(Self::CHEER_ELBOW),
                    gait.elation,
                );
                let ready = Self::held(
                    running,
                    Quat::from_rotation_x(Self::SET_ELBOW),
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
                Self::held(out, Quat::from_rotation_x(Self::CRADLE_WRIST), bracing)
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
                    -amplitude * swing * gait.course.y
                        + Self::DRIVE_HIP * gait.drive
                        + Self::SHUFFLE_HIP_PICKUP * picking,
                );
                let ready = Self::held(
                    running,
                    Quat::from_rotation_x(
                        Self::SET_HIP + Self::TOES_HIP * Self::on_his_toes(gait, self.side),
                    ),
                    gait.set,
                );
                let ready = Self::held(
                    ready,
                    Quat::from_rotation_x(Self::SET_HIP + Self::SAVE_HIP * Self::stooping(gait)),
                    gait.save,
                );
                let leaping = Self::held(ready, Quat::from_rotation_x(Self::JUMP_HIP), gait.jump);
                // In flight the legs trail — the near one straight behind
                // him because it is the one he pushed off, the far one
                // swinging up over it.
                let diving = Self::held(
                    leaping,
                    Quat::from_rotation_x(Self::DIVE_HIP + Self::DIVE_SCISSOR_HIP * leading),
                    gait.dive * gait.stretch,
                );
                let down = Self::held(diving, Quat::from_rotation_x(Self::DOWN_HIP), gait.grounded);
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
                let tuck = (0.5 + 0.5 * (leg - 0.5).cos()).powi(2);
                let running = Quat::from_rotation_x(
                    0.07
                        // Off the same amplitude the hip is using, so a leg
                        // taking a full walking step folds through the middle
                        // of it instead of goose-stepping.
                        + (Self::KNEE_FLEX.0 + Self::KNEE_FLEX.1 * Self::stepping(gait)) * tuck
                        // The swinging foot picks up out of a side-step, and
                        // a backpedalling man's knees come up in front of him
                        // — neither of which the sagittal cycle draws, since
                        // for a pure shuffle it has nowhere to swing to.
                        //
                        // Off the SWING rather than off the stride's own
                        // tuck: which foot is up is the thing a lateral gait
                        // is made of, and the tuck does not know.
                        + Self::SHUFFLE_PICKUP * Self::tread(leg).1 * Self::sidling(gait)
                        // …and BOTH knees stay softly bent while he is
                        // travelling across himself, whichever of them is
                        // carrying him. A constant, not a curve — see
                        // [`Joint::SHUFFLE_KNEE`], and `Joint::stepping` for
                        // what a curve did.
                        + Self::SHUFFLE_KNEE * Self::sidling(gait)
                        + Self::BACKPEDAL_KNEE * Self::backing(gait) * tuck
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
                let ready = Self::held(
                    running,
                    Quat::from_rotation_x(
                        Self::SET_KNEE + Self::TOES_KNEE * Self::on_his_toes(gait, self.side),
                    ),
                    gait.set,
                );
                // Down under himself to a ball at his boots. Its own layer
                // above the set rather than a term inside the run cycle,
                // because a keeper making a save is nearly always set and the
                // set would slerp it away — see the note at [`Limb::Hip`].
                let saving = Self::held(
                    ready,
                    Quat::from_rotation_x(Self::SET_KNEE + Self::SAVE_KNEE * Self::stooping(gait)),
                    gait.save,
                );
                let leaping = Self::held(saving, Quat::from_rotation_x(Self::JUMP_KNEE), gait.jump);
                let diving = Self::held(
                    leaping,
                    Quat::from_rotation_x(Self::DIVE_KNEE + Self::DIVE_SCISSOR_KNEE * trailing),
                    gait.dive * gait.stretch,
                );
                let down = Self::held(
                    diving,
                    Quat::from_rotation_x(Self::DOWN_KNEE),
                    gait.grounded,
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
                let rolling = Quat::from_rotation_y(Self::TOE_OUT * gait.course.x * across)
                    * Quat::from_rotation_z(Self::FOOT_ROLL * gait.course.x * across * tread)
                    * Quat::from_rotation_x(
                        (middle - reach * swing * gait.course.y) * Self::stepping(gait)
                            + Self::BACKPEDAL_ANKLE * Self::backing(gait)
                            + Self::ANKLE_PLANTAR * 0.5 * lift * across,
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

    /// The same, sized to this particular player. Every amplitude in the
    /// run cycle goes through here rather than through `blend` so that the
    /// squad is a spread of runners rather than one runner drawn twenty-two
    /// times — see `Gait::spring`. The standing end of each range is left
    /// alone: how a man carries himself at rest is `signature`.
    fn strides(range: (f32, f32), gait: Gait) -> f32 {
        range.0 + range.1 * gait.run * gait.spring
    }

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
        let flight = gait.run * gait.spring * gait.course.y.max(0.0);
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
        (Self::reachable(across) / Physique::LEG)
            .clamp(-0.95, 0.95)
            .asin()
    }

    /// **How far out to the side a foot will actually be planted**, in
    /// metres from the centreline.
    ///
    /// The backstop, and the one number in the lateral gait that is not
    /// derived from anything. Everything above it solves the side-step from
    /// the ground the body covers, which is right and is what stops a
    /// shuffle skating — but it is an equation, and an equation handed an
    /// impossible demand returns an impossible answer. The demand really is
    /// made: measured over a recording, 4.5% of the frames an outfielder is
    /// running in have him more than 100° off his own facing, because
    /// [`crate::actors::Actors::PIVOT_RATE`] will not let his heading come
    /// round any faster than a body can turn. Solved honestly, a man
    /// reversing at six and a half metres a second was drawn with his boots
    /// 1.88 m apart and his crown 69 cm below standing.
    ///
    /// [`crate::body::Gait::open`] is what removes nearly all of that, by
    /// turning his legs onto the run so there is no side-step left to solve.
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

    /// …and the height that splay costs him, in metres.
    ///
    /// **A splayed leg is a SHORTER leg.** Both of them open and close
    /// together through a shuffle, so without paying for it his feet leave
    /// the grass at the wide point of every step and go through it at the
    /// narrow one — a keeper bouncing across his own six-yard box. It comes
    /// straight out of the triangle; there is nothing to tune.
    fn splay_drop(gait: Gait) -> f32 {
        // The mean of the two, and of the COSINES rather than of the angles,
        // because it is the cosines the height is linear in — see
        // [`Joint::hip_list`], which then puts each hip where its own leg
        // needs it.
        let reach = (Self::abduct(gait, 1.0).cos() + Self::abduct(gait, -1.0).cos()) * 0.5;
        Physique::LEG * (1.0 - reach)
    }

    /// …and the height the soft knee of a side-step costs him, in metres.
    ///
    /// A bent knee is a shorter leg for the same reason a splayed one is,
    /// and [`Joint::SHUFFLE_KNEE`] bends BOTH of them by the same constant
    /// through the whole cycle — so unpaid for it lifts the whole figure a
    /// centimetre off the turf for as long as he is travelling across
    /// himself. Thigh and shin are the same length here, so the fold is
    /// isosceles and the drop is exact.
    fn shuffle_drop(gait: Gait) -> f32 {
        let fold = Self::SHUFFLE_KNEE * Self::sidling(gait) * 0.5;
        (Physique::THIGH + Physique::SHIN) * (1.0 - fold.cos())
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
        let mine = Self::abduct(gait, side).cos();
        let other = Self::abduct(gait, -side).cos();
        Physique::LEG * (mine - other) * 0.5
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
        gait.set
            .max(Self::sidling(gait) * Self::SIDLE_READY * gait.keeper)
    }
    const SIDLE_READY: f32 = 0.60;

    fn on_his_toes(gait: Gait, side: f32) -> f32 {
        // ⚠ And not while he is TAKING steps. The dance runs on the idle
        // clock and the stride runs on ground covered, so a keeper doing
        // both at once has two step rhythms in the same pair of legs — which
        // is not twice as alive, it is incoherent.
        let alive = gait.set * (1.0 - gait.save) * (1.0 - Self::afoot(gait));
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
/// Everything else in this rig moves a limb against a body that is standing
/// on the turf. A dive is the one thing that moves the body itself: the man
/// goes horizontal and airborne, and no arrangement of hips and knees
/// expresses that. Putting one node between the actor and the figure is what
/// lets [`crate::actors::Actors::animate`] topple and lift the lot in one
/// transform — while the contact shadow and the team ring, which stay
/// children of the actor, stay flat on the grass where they belong.
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
        // How far from upright the figure has ended up, as the sine of the
        // angle between its own up-axis and the world's. Off the composed
        // rotation rather than off either Euler angle, so a dive that is
        // half across the goal and half up the pitch settles as far as one
        // that is all of either.
        let upright = (rotation * Vec3::Y).y.clamp(-1.0, 1.0);
        let tilt = (1.0 - upright * upright).max(0.0).sqrt();
        let settle = (Self::PIVOT - Self::LYING) * tilt;
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
                                Transform::default(),
                            ));
                        }
                        head.spawn((
                            Mesh3d(parts.nose.clone()),
                            MeshMaterial3d(outfit.skin.clone()),
                            Transform::from_translation(BodyParts::NOSE_AT),
                        ));
                        for side in [-1.0f32, 1.0] {
                            head.spawn((
                                Mesh3d(parts.ear.clone()),
                                MeshMaterial3d(outfit.skin.clone()),
                                Transform::from_translation(
                                    BodyParts::EAR_AT * Vec3::new(side, 1.0, 1.0),
                                ),
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
                                Transform::from_translation(elbow),
                            ))
                            .with_child((
                                // The ball of the joint, filling what the two
                                // tapers leave open when the arm bends.
                                Mesh3d(parts.elbow.clone()),
                                MeshMaterial3d(outfit.skin.clone()),
                                Transform::default(),
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
                                forearm
                                    .spawn((
                                        Joint::new(root, Limb::Wrist, side, wrist),
                                        Mesh3d(if keeper {
                                            parts.glove.clone()
                                        } else {
                                            parts.hand.clone()
                                        }),
                                        MeshMaterial3d(outfit.hands.clone()),
                                        Transform::from_translation(wrist),
                                    ))
                                    .with_children(|hand| {
                                        if !keeper {
                                            return;
                                        }
                                        for (thumb, at) in BodyParts::digits(side) {
                                            hand.spawn((
                                                Mesh3d(if thumb {
                                                    parts.thumb.clone()
                                                } else {
                                                    parts.finger.clone()
                                                }),
                                                MeshMaterial3d(outfit.hands.clone()),
                                                at,
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
                            Mesh3d(parts.knee.clone()),
                            MeshMaterial3d(outfit.socks.clone()),
                            Transform::default(),
                        ));
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

    /// A player standing still, with nothing switched on.
    pub fn still() -> Gait {
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
            throwing: 0.0,
            header: 0.0,
            throw_in: 0.0,
            drive: 0.0,
            carrying: 0.0,
            despair: 0.0,
            elation: 0.0,
            hands_to_head: 0.0,
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
            keeper: 1.0,
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
    pub fn running(run: f32) -> Gait {
        let mut gait = still();
        gait.run = run;
        gait.phase = 1.1;
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

    /// Where the boot meets the grass, in the ANKLE.s own space — 38 mm
    /// below it, which is how the boot mesh is modelled. Lives here rather
    /// than on `Physique` because the renderer never needs it: it draws the
    /// boot, it does not ask where the bottom of it is.
    const SOLE: Vec3 = Vec3::new(0.0, -0.038, 0.0);

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
    use crate::pitch::Pitch;
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

        /// One triangle, already in screen space: `x`/`y` in pixels, `z` into
        /// the screen, with a shade per corner.
        fn triangle(&mut self, corners: [Vec3; 3], shades: [f32; 3], tint: Vec3) {
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
                    self.colour[index] = tint * shade;
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
        draw(
            &parts.nose,
            head * Transform::from_translation(BodyParts::NOSE_AT),
            SKIN,
        );

        for side in [-1.0f32, 1.0] {
            draw(
                &parts.ear,
                head * Transform::from_translation(BodyParts::EAR_AT * Vec3::new(side, 1.0, 1.0)),
                SKIN,
            );

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
            draw(&parts.elbow, fore, SKIN);
            if keeper {
                draw(&parts.sleeve_forearm, fore, SHIRT);
                draw(&parts.cuff_forearm, fore, TRIM);
            }
            let hand = fore * skeleton::step(Limb::Wrist, side, wrist, gait);
            draw(
                if keeper { &parts.glove } else { &parts.hand },
                hand,
                if keeper { TRIM } else { SKIN },
            );
            if keeper {
                for (thumb, at) in BodyParts::digits(side) {
                    draw(
                        if thumb { &parts.thumb } else { &parts.finger },
                        hand * at,
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
            draw(&parts.knee, lower, SHORTS);
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
    use super::skeleton::*;
    use super::*;
    use crate::actors::Actors;

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
        use crate::kit::Complexion;
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
                let radius =
                    Vec2::new(point[0] / shirt.x, (point[2] - shirt.offset) / shirt.z).length();
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

        // Standing, for the comparison; then the keeper's slump square on and
        // from the side, the outfielder's, and the celebration.
        //
        // `bearing` PI is the FRONT — bearing 0 looks at the back of a
        // player's head, which is the wrong side for reading what his hands
        // are doing.
        let poses: [(f32, Gait); 5] = [
            (PI, still()),
            (PI, slumped(1.0)),
            (FRAC_PI_2, slumped(1.0)),
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
        let poses: [(f32, Gait); 9] = [
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
    /// It is affordable because of what it does NOT change: every mesh here is
    /// shared by all twenty-two players, so this is a few tens of thousands of
    /// vertices in memory ONCE, and the ~400 draw calls a squad costs are set
    /// by the number of PARTS, not by their resolution. A GPU that can draw
    /// twenty-two of these is drawing about three quarters of a million
    /// triangles a frame, which is a fraction of what a decade-old integrated
    /// one manages.
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
            + count(&parts.nose)
            + count(&parts.number)
            + count(&parts.name)
            // The fullest cap, since a squad wears a spread of them.
            + parts.hair.iter().flatten().map(count).max().unwrap_or(0);
        let paired = [
            &parts.ear,
            &parts.upper_arm,
            &parts.elbow,
            &parts.knee,
            &parts.sleeve,
            &parts.cuff,
            &parts.forearm,
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
            (22_000..46_000).contains(&footballer),
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

    /// The nose stands off the face and the ears off the sides, and neither of
    /// them floats clear of the head it belongs to.
    #[test]
    fn the_nose_and_ears_stand_proud() {
        let tip = BodyParts::NOSE_AT + Vec3::new(0.0, -0.022, 0.015 + 0.019);
        let face = Sculptor::section(&BodyParts::skull(), tip.y);
        assert!(
            tip.z - (face.offset + face.z) > 0.012,
            "the nose is inside the face: {} vs {}",
            tip.z,
            face.offset + face.z
        );
        // Its root is buried, or there is a hole where it joins.
        let root = BodyParts::NOSE_AT + Vec3::new(0.0, 0.030, -0.011);
        let bridge = Sculptor::section(&BodyParts::skull(), root.y);
        assert!(root.z < bridge.offset + bridge.z, "the nose floats off");

        let ear = Sculptor::section(&BodyParts::skull(), BodyParts::EAR_AT.y);
        let out = BodyParts::EAR_AT.x + 0.009;
        assert!(out > ear.x + 0.004, "ears flush with the skull");
        assert!(BodyParts::EAR_AT.x < ear.x, "ears hanging in mid-air");
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
            (0.098f32, 0.005f32, 0.0426f32),
            (0.090, 0.009, 0.0518),
            (0.082, 0.017, 0.0586),
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
            // And it covers the sides from the top of the ear up, which is
            // where hair grows — without hanging out behind the skull, which
            // is what setting the whole ring back used to do.
            // Nowhere above the hairline does the skull come back out through
            // the cap. A ring of bare scalp at the crown — which a cap that
            // closed in one shallow cone left — is a tonsure, and it is
            // invisible in every test that only looks at the front.
            for step in 0..=60 {
                let y = emerges + (0.2549 - emerges) * step as f32 / 60.0;
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

            let temple = Sculptor::section(&rings, BodyParts::EAR_AT.y + 0.030);
            let skull = Sculptor::section(&BodyParts::skull(), BodyParts::EAR_AT.y + 0.030);
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
