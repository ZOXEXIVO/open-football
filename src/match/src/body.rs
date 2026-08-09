use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use std::f32::consts::{PI, TAU};

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
    /// Sides around each ring. Twelve is round at the size a broadcast camera
    /// draws a player and still leaves a full squad at a few thousand triangles.
    const SIDES: usize = 12;
    /// Rings from pole to pole on a sphere.
    const STACKS: usize = 8;

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
        self.positions
            .push([offset.x, offset.y + ring.y, offset.z]);
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
    pub const SHOULDER: f32 = 0.505;
    pub const SHOULDER_SPREAD: f32 = 0.196;
    pub const UPPER_ARM: f32 = 0.30;
    pub const FOREARM: f32 = 0.26;
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
    boot: Handle<Mesh>,
}

impl BodyParts {
    pub fn new(meshes: &mut Assets<Mesh>) -> Self {
        BodyParts {
            // Hips, waist, chest, shoulders — the shirt.
            torso: meshes.add(Sculptor::part(&[
                Ring::oval(0.000, 0.140, 0.096),
                Ring::oval(0.090, 0.132, 0.090),
                Ring::oval(0.200, 0.145, 0.098),
                Ring::oval(0.320, 0.174, 0.107),
                Ring::oval(0.420, 0.196, 0.110),
                Ring::oval(0.500, 0.204, 0.104),
                Ring::oval(0.555, 0.158, 0.090),
                Ring::oval(0.580, 0.090, 0.072),
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
                Ring::oval(0.000, 0.052, 0.056),
                Ring::oval(0.025, 0.076, 0.080),
                Ring::oval(0.065, 0.094, 0.101),
                Ring::oval(0.115, 0.099, 0.108),
                Ring::oval(0.165, 0.091, 0.099),
                Ring::oval(0.205, 0.072, 0.078),
                Ring::oval(0.235, 0.040, 0.044),
                Ring::oval(0.252, 0.000, 0.000),
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
            upper_arm: meshes.add(Sculptor::part(&[
                Ring::round(0.020, 0.052),
                Ring::round(0.000, 0.055),
                Ring::round(-0.140, 0.048),
                Ring::round(-0.280, 0.042),
                Ring::round(-0.300, 0.038),
            ])),
            // A short sleeve, riding on the shoulder joint so it swings with it.
            sleeve: meshes.add(Sculptor::part(&[
                Ring::oval(0.055, 0.072, 0.070),
                Ring::oval(0.000, 0.078, 0.075),
                Ring::oval(-0.090, 0.070, 0.068),
                Ring::oval(-0.135, 0.062, 0.060),
            ])),
            forearm: meshes.add(Sculptor::part(&[
                Ring::round(0.000, 0.044),
                Ring::round(-0.120, 0.039),
                Ring::round(-0.240, 0.033),
                Ring::round(-0.260, 0.028),
            ])),
            hand: meshes.add(Sculptor::ellipsoid(Vec3::new(0.035, 0.050, 0.028))),
            // The leg of the shorts: it belongs to the thigh, not to the hips.
            shorts_leg: meshes.add(Sculptor::part(&[
                Ring::oval(0.040, 0.114, 0.104),
                Ring::oval(-0.040, 0.120, 0.109),
                Ring::oval(-0.130, 0.116, 0.105),
                Ring::oval(-0.205, 0.106, 0.097),
            ])),
            thigh: meshes.add(Sculptor::part(&[
                Ring::round(-0.100, 0.086),
                Ring::round(-0.200, 0.079),
                Ring::round(-0.340, 0.068),
                Ring::round(-0.440, 0.058),
                Ring::round(-0.455, 0.052),
            ])),
            // Shin and sock in one: socks cover a footballer's leg to the knee.
            shin: meshes.add(Sculptor::part(&[
                Ring::round(0.020, 0.060),
                Ring::round(-0.080, 0.058),
                Ring::round(-0.220, 0.049),
                Ring::round(-0.360, 0.040),
                Ring::oval(-0.440, 0.036, 0.038),
                Ring::oval(-0.455, 0.034, 0.036),
            ])),
            boot: meshes.add(Sculptor::ellipsoid(Vec3::new(0.046, 0.040, 0.095))),
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
                self.origin + Vec3::Y * bob
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

        match self.limb {
            Limb::Pelvis => Quat::IDENTITY,
            Limb::Torso => {
                Quat::from_rotation_x(lean)
                    * Quat::from_rotation_y(-0.14 * gait.run * gait.phase.sin())
            }
            // Kept level: a runner leans from the hips and looks up the pitch,
            // not at their own boots.
            Limb::Head => Quat::from_rotation_x(-lean * 0.75),
            Limb::Shoulder => {
                // Arms swing against the leg on the same side, and are carried
                // wider the harder the player is running.
                Quat::from_rotation_z(self.side * (0.15 + 0.07 * gait.run))
                    * Quat::from_rotation_x(Self::blend(Self::ARM_SWING, gait.run) * swing)
            }
            Limb::Elbow => Quat::from_rotation_x(
                -Self::blend(Self::ELBOW_FLEX, gait.run) - 0.18 * gait.run * swing,
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
                    .with_child((
                        Mesh3d(parts.boot.clone()),
                        MeshMaterial3d(outfit.boots.clone()),
                        Transform::from_xyz(0.0, -Physique::SHIN + 0.005, 0.035),
                    ));
                });
            }
        });
    }
}
