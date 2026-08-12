use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::kit::Outfit;

/// One cross-section of a body part: an ellipse of half-widths `x` (across the
/// body) and `z` (front to back) at height `y`, in the part's own space.
#[derive(Clone, Copy)]
struct Ring {
    y: f32,
    x: f32,
    z: f32,
}

impl Ring {
    const fn round(y: f32, radius: f32) -> Self {
        Ring {
            y,
            x: radius,
            z: radius,
        }
    }

    const fn oval(y: f32, x: f32, z: f32) -> Self {
        Ring { y, x, z }
    }
}

/// Lathes stacked ellipses into a closed, smooth-shaded mesh.
///
/// Every part of a footballer — a thigh, the torso, a skull — is a tube through
/// a handful of cross-sections, so one lathe covers the whole model and the
/// shapes stay editable as a list of numbers rather than as geometry. Nothing
/// here is loaded from disk: a glTF character would cost more to ship than the
/// entire viewer.
#[derive(Default)]
struct Sculptor {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Sculptor {
    /// Sides around each ring. Sixteen holds a round silhouette at the range
    /// the broadcast camera now sits at, and still leaves a full squad at a few
    /// thousand triangles — the meshes are shared by all twenty-two.
    const SIDES: usize = 16;
    /// Rings from pole to pole on a sphere.
    const STACKS: usize = 10;

    /// A part described by its profile, from either end.
    fn part(rings: &[Ring]) -> Mesh {
        let mut sculptor = Sculptor::default();
        sculptor.loft(rings, Vec3::ZERO);
        sculptor.build()
    }

    /// A rounded lump — a hand, a boot — sized on each axis.
    fn ellipsoid(radii: Vec3) -> Mesh {
        let mut sculptor = Sculptor::default();
        sculptor.loft(&Self::sphere(radii), Vec3::ZERO);
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
                }
            })
            .collect()
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
        for ring in &profile {
            for side in 0..Self::SIDES {
                let angle = TAU * side as f32 / Self::SIDES as f32;
                self.positions.push([
                    offset.x + angle.cos() * ring.x,
                    offset.y + ring.y,
                    offset.z + angle.sin() * ring.z,
                ]);
                self.uvs
                    .push([side as f32 / Self::SIDES as f32, (ring.y - foot) / span]);
            }
        }

        let sides = Self::SIDES as u32;
        for step in 0..profile.len() as u32 - 1 {
            for side in 0..sides {
                let next = (side + 1) % sides;
                let a = base + step * sides + side;
                let b = base + step * sides + next;
                let c = base + (step + 1) * sides + next;
                let d = base + (step + 1) * sides + side;
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
        self.positions.push([offset.x, offset.y + ring.y, offset.z]);
        self.uvs.push([0.5, 0.5]);
        for side in 0..Self::SIDES {
            let angle = TAU * side as f32 / Self::SIDES as f32;
            self.positions.push([
                offset.x + angle.cos() * ring.x,
                offset.y + ring.y,
                offset.z + angle.sin() * ring.z,
            ]);
            self.uvs
                .push([0.5 + angle.cos() * 0.5, 0.5 + angle.sin() * 0.5]);
        }

        let sides = Self::SIDES as u32;
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
}

/// Every mesh a footballer is made of, built once and shared by all
/// twenty-two: only the materials differ from player to player.
pub struct BodyParts {
    torso: Handle<Mesh>,
    pelvis: Handle<Mesh>,
    head: Handle<Mesh>,
    hair: Handle<Mesh>,
    upper_arm: Handle<Mesh>,
    sleeve: Handle<Mesh>,
    forearm: Handle<Mesh>,
    hand: Handle<Mesh>,
    shorts_leg: Handle<Mesh>,
    thigh: Handle<Mesh>,
    shin: Handle<Mesh>,
    sock_top: Handle<Mesh>,
    boot: Handle<Mesh>,
    /// The flat panel the shirt number is printed on.
    number: Handle<Mesh>,
}

impl BodyParts {
    pub fn new(meshes: &mut Assets<Mesh>) -> Self {
        BodyParts {
            // Hips, waist, chest, shoulders — the shirt. A footballer's torso
            // is a V: the waist is the narrowest point and the shoulders carry
            // most of the width, which is most of what reads as "athlete" in a
            // silhouette this size.
            // The shoulder line is the whole problem with a figure this size.
            // It used to go 0.196 wide at y=0.530 and 0.150 at y=0.558 — a
            // 23% collapse across 28 mm — which is a coat-hanger, not a
            // shoulder: a flat shelf ending in a cliff, with the arm hung off
            // the edge of it. Real shoulders are a dome. The trapezius leaves
            // the neck and falls away over a good ten centimetres, and the
            // widest point is the acromion, out at the very end of the
            // collarbone, with a rounded crest over it.
            //
            // So the crest is now broad and slightly domed (0.482-0.535 all
            // within 4% of each other) and the run into the neck takes three
            // rings instead of one.
            torso: meshes.add(Sculptor::part(&[
                Ring::oval(0.000, 0.142, 0.098),
                Ring::oval(0.070, 0.134, 0.092),
                Ring::oval(0.140, 0.131, 0.089),
                Ring::oval(0.220, 0.148, 0.100),
                Ring::oval(0.300, 0.168, 0.107),
                Ring::oval(0.370, 0.185, 0.111),
                Ring::oval(0.440, 0.198, 0.111),
                Ring::oval(0.482, 0.207, 0.108),
                Ring::oval(0.512, 0.209, 0.104),
                Ring::oval(0.535, 0.201, 0.099),
                Ring::oval(0.552, 0.176, 0.093),
                Ring::oval(0.568, 0.134, 0.082),
                Ring::oval(0.580, 0.088, 0.070),
            ])),
            // The seat of the shorts, which stays put while the legs swing.
            pelvis: meshes.add(Sculptor::part(&[
                Ring::oval(0.100, 0.140, 0.098),
                Ring::oval(0.020, 0.152, 0.104),
                Ring::oval(-0.060, 0.160, 0.108),
                Ring::oval(-0.130, 0.154, 0.104),
            ])),
            // Neck, jaw and skull, hung off the base of the neck.
            head: meshes.add(Sculptor::part(&[
                Ring::oval(-0.060, 0.048, 0.052),
                Ring::oval(0.000, 0.053, 0.057),
                Ring::oval(0.022, 0.070, 0.074),
                Ring::oval(0.045, 0.086, 0.092),
                Ring::oval(0.075, 0.096, 0.103),
                Ring::oval(0.110, 0.100, 0.109),
                Ring::oval(0.145, 0.096, 0.104),
                Ring::oval(0.180, 0.086, 0.093),
                Ring::oval(0.212, 0.068, 0.074),
                Ring::oval(0.238, 0.040, 0.044),
                Ring::oval(0.253, 0.000, 0.000),
            ])),
            // A cap over the skull, set back a little so a hairline reads.
            hair: meshes.add({
                let mut sculptor = Sculptor::default();
                sculptor.loft(
                    &[
                        Ring::oval(0.085, 0.101, 0.110),
                        Ring::oval(0.125, 0.102, 0.111),
                        Ring::oval(0.170, 0.095, 0.103),
                        Ring::oval(0.210, 0.078, 0.084),
                        Ring::oval(0.240, 0.049, 0.053),
                        Ring::oval(0.262, 0.000, 0.000),
                    ],
                    Vec3::new(0.0, 0.0, -0.008),
                );
                sculptor.build()
            }),
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
            sleeve: meshes.add(Sculptor::part(&[
                Ring::oval(0.030, 0.064, 0.062),
                Ring::oval(-0.015, 0.074, 0.071),
                Ring::oval(-0.080, 0.070, 0.067),
                Ring::oval(-0.130, 0.062, 0.060),
            ])),
            forearm: meshes.add(Sculptor::part(&[
                Ring::round(0.000, 0.046),
                Ring::round(-0.060, 0.043),
                Ring::round(-0.140, 0.037),
                Ring::round(-0.215, 0.031),
                Ring::round(-0.260, 0.027),
            ])),
            hand: meshes.add(Sculptor::ellipsoid(Vec3::new(0.035, 0.050, 0.028))),
            // The leg of the shorts: it belongs to the thigh, not to the hips.
            shorts_leg: meshes.add(Sculptor::part(&[
                Ring::oval(0.040, 0.114, 0.104),
                Ring::oval(-0.040, 0.120, 0.109),
                Ring::oval(-0.130, 0.116, 0.105),
                Ring::oval(-0.205, 0.106, 0.097),
            ])),
            // Quadriceps high on the thigh, narrowing into the knee.
            thigh: meshes.add(Sculptor::part(&[
                Ring::oval(-0.090, 0.089, 0.086),
                Ring::oval(-0.160, 0.086, 0.083),
                Ring::oval(-0.250, 0.078, 0.075),
                Ring::oval(-0.340, 0.068, 0.066),
                Ring::oval(-0.420, 0.059, 0.058),
                Ring::oval(-0.455, 0.053, 0.053),
            ])),
            // Shin and sock in one: socks cover a footballer's leg to the knee.
            // The calf sits high and at the back, which is what stops the lower
            // leg reading as a broom handle when a stride opens out.
            shin: meshes.add(Sculptor::part(&[
                Ring::oval(0.020, 0.060, 0.060),
                Ring::oval(-0.040, 0.062, 0.063),
                Ring::oval(-0.110, 0.059, 0.061),
                Ring::oval(-0.200, 0.050, 0.052),
                Ring::oval(-0.300, 0.042, 0.043),
                Ring::oval(-0.390, 0.036, 0.037),
                Ring::oval(-0.440, 0.034, 0.036),
                Ring::oval(-0.455, 0.033, 0.035),
            ])),
            // The turnover at the top of the sock, in the shorts colour — the
            // one piece of kit detail that survives at this distance.
            sock_top: meshes.add(Sculptor::part(&[
                Ring::oval(0.012, 0.0620, 0.0625),
                Ring::oval(-0.030, 0.0645, 0.0655),
                Ring::oval(-0.078, 0.0620, 0.0635),
            ])),
            boot: meshes.add(Sculptor::ellipsoid(Vec3::new(0.048, 0.038, 0.105))),
            // A real shirt number covers most of the upper back.
            number: meshes.add(Rectangle::new(0.250, 0.215).mesh().build()),
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
    Hip,
    Knee,
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
    /// 0 for everybody all match; ramps to 1 for the one goalkeeper who has
    /// the ball in his gloves.
    ///
    /// A keeper who had gathered the ball was posed exactly like one who had
    /// not: arms hanging at his sides, swinging with the run cycle, while the
    /// ball hung at chest height inside his own torso. This is the signal that
    /// brings the forearms up and settles the chest so it can sit in his
    /// hands instead.
    pub carry: f32,
}

impl Joint {
    /// The angle the leading leg swings through, standing and at a sprint.
    const HIP_SWING: (f32, f32) = (0.10, 0.62);
    const KNEE_FLEX: (f32, f32) = (0.16, 1.55);
    const ARM_SWING: (f32, f32) = (0.10, 0.75);
    const ELBOW_FLEX: (f32, f32) = (0.25, 1.05);
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
    const HIP_TWIST: f32 = 0.065;
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
            Limb::Pelvis | Limb::Torso | Limb::Hip => {
                let bob = Self::BOB * gait.run * (0.5 + 0.5 * (gait.phase * 2.0).cos());
                // Breathing, for a player who is not running. Fades out as he
                // does, where the stride bob takes over.
                let breathe = Self::BREATHE * (1.0 - gait.run) * (0.5 + 0.5 * gait.idle.sin());
                self.origin + Vec3::Y * (bob + breathe)
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

        match self.limb {
            // Hips counter-rotate against the shoulders — the thing that
            // makes a run read as a person rather than a mechanism — and
            // carry the weight shift when he is standing.
            Limb::Pelvis => {
                Quat::from_rotation_y(Self::HIP_TWIST * gait.run * gait.phase.sin())
                    * Quat::from_rotation_z(Self::WEIGHT_SHIFT * weight)
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
                let settle = 1.0 - gait.carry;
                Quat::from_rotation_x(lean * (1.0 + 0.16 * gait.signature) * settle)
                    * Quat::from_rotation_y(-0.14 * gait.run * gait.phase.sin() * settle)
                    * Quat::from_rotation_z(roll * settle)
            }
            // He watches the ball. The head hangs off the torso, so this yaw
            // is already relative to his chest — turning it is the single
            // cheapest thing in the whole rig that reads as attention, and
            // without it twenty-two players stare rigidly down their own
            // running line all match.
            //
            // Still kept level in pitch: a runner leans from the hips and
            // looks up the pitch, not at his own boots.
            Limb::Head => Quat::from_rotation_y(gait.look) * Quat::from_rotation_x(-lean * 0.75),
            Limb::Shoulder => {
                // Arms swing against the leg on the same side, and are carried
                // wider the harder the player is running — and wider again, or
                // tighter, depending on the man.
                let carriage = 0.15 + 0.07 * gait.run + 0.055 * gait.signature;
                // Nobody's two arms do the same thing. Tied to the signature
                // so it is this player's asymmetry, the same one every time.
                let asymmetry = 1.0 + 0.11 * gait.signature * self.side;
                // Standing, they drift instead of locking. Offset by side so
                // the two do not move as a pair.
                let drift = 0.055 * standing * (gait.idle + self.side).sin();
                let arm = Self::blend(Self::ARM_SWING, gait.run) * swing * asymmetry + drift;
                let swinging =
                    Quat::from_rotation_z(self.side * carriage) * Quat::from_rotation_x(arm);
                Self::held(
                    swinging,
                    Quat::from_rotation_z(self.side * Self::CRADLE_SPREAD)
                        * Quat::from_rotation_x(Self::CRADLE_SHOULDER),
                    gait.carry,
                )
            }
            // Elbow carriage is the most individual thing about a runner:
            // some hold them almost straight, some at a right angle.
            Limb::Elbow => Self::held(
                Quat::from_rotation_x(
                    -Self::blend(Self::ELBOW_FLEX, gait.run) * (1.0 + 0.24 * gait.signature)
                        - 0.18 * gait.run * swing,
                ),
                Quat::from_rotation_x(Self::CRADLE_ELBOW),
                gait.carry,
            ),
            Limb::Hip => Quat::from_rotation_x(-Self::blend(Self::HIP_SWING, gait.run) * swing),
            // Deepest as the leg folds through underneath the player, and all
            // but straight again by the time it reaches out to land. Squaring
            // the curve is what narrows the tuck to that one part of the
            // cycle; a plain cosine leaves the leading leg bent on touchdown,
            // which reads as a stumble rather than a stride.
            Limb::Knee => Quat::from_rotation_x(
                0.07 + Self::blend(Self::KNEE_FLEX, gait.run)
                    * (0.5 + 0.5 * (leg - 0.5).cos()).powi(2),
            ),
        }
    }

    fn blend(range: (f32, f32), run: f32) -> f32 {
        range.0 + range.1 * run
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

/// Hangs one footballer's meshes off an actor entity.
pub struct Footballer;

impl Footballer {
    /// Builds the rig under `root`, which carries the player's position, facing
    /// and stride. Legs hang from the root so the torso can lean without taking
    /// the feet with it; arms and head hang from the torso so that they do.
    pub fn assemble(commands: &mut Commands, root: Entity, parts: &BodyParts, outfit: &Outfit) {
        let hips = Vec3::new(0.0, Physique::HIP, 0.0);
        let neck = Vec3::new(0.0, Physique::TORSO, 0.0);
        let elbow = Vec3::new(0.0, -Physique::UPPER_ARM, 0.0);
        let knee = Vec3::new(0.0, -Physique::THIGH, 0.0);

        commands.entity(root).with_children(|body| {
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
                // Printed across the shoulder blades, standing a couple of
                // millimetres off the shirt so it never fights it for depth.
                if let Some(number) = outfit.number.clone() {
                    torso.spawn((
                        Mesh3d(parts.number.clone()),
                        MeshMaterial3d(number),
                        Transform::from_xyz(0.0, 0.330, -0.115)
                            .with_rotation(Quat::from_rotation_y(PI)),
                    ));
                }

                torso
                    .spawn((
                        Joint::new(root, Limb::Head, 0.0, neck),
                        Mesh3d(parts.head.clone()),
                        MeshMaterial3d(outfit.skin.clone()),
                        Transform::from_translation(neck),
                    ))
                    .with_child((
                        Mesh3d(parts.hair.clone()),
                        MeshMaterial3d(outfit.hair.clone()),
                        Transform::default(),
                    ));

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
                            arm.spawn((
                                Mesh3d(parts.sleeve.clone()),
                                MeshMaterial3d(outfit.shirt.clone()),
                                Transform::default(),
                            ));
                            arm.spawn((
                                Joint::new(root, Limb::Elbow, side, elbow),
                                Mesh3d(parts.forearm.clone()),
                                MeshMaterial3d(outfit.skin.clone()),
                                Transform::from_translation(elbow),
                            ))
                            .with_child((
                                Mesh3d(parts.hand.clone()),
                                MeshMaterial3d(outfit.hands.clone()),
                                Transform::from_xyz(0.0, -Physique::FOREARM - 0.03, 0.0),
                            ));
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
                            Mesh3d(parts.sock_top.clone()),
                            MeshMaterial3d(outfit.shorts.clone()),
                            Transform::default(),
                        ));
                        shin.spawn((
                            Mesh3d(parts.boot.clone()),
                            MeshMaterial3d(outfit.boots.clone()),
                            Transform::from_xyz(0.0, -Physique::SHIN + 0.005, 0.035),
                        ));
                    });
                });
            }
        });
    }
}
