//! Continuous garments deformed by the existing chest, pelvis and limb joints.
//!
//! Intersecting rigid sleeve caps leave a crease regardless of their size.
//! Smooth unions give the shirt and shorts shared vertices and normals;
//! skin weights let that surface follow the existing rig without reopening it.

use super::*;
use std::collections::HashMap;

pub(super) fn shoulder(side: f32) -> Mat4 {
    Mat4::from_rotation_translation(
        Quat::from_rotation_z(side * 0.75),
        Vec3::new(side * Physique::SHOULDER_SPREAD, Physique::SHOULDER, 0.0),
    )
}

pub(super) fn binds() -> Vec<Mat4> {
    vec![
        Mat4::IDENTITY,
        shoulder(-1.0).inverse(),
        shoulder(1.0).inverse(),
    ]
}

pub(super) fn shorts_binds() -> Vec<Mat4> {
    vec![
        Mat4::IDENTITY,
        Mat4::from_translation(Vec3::X * Physique::HIP_SPREAD),
        Mat4::from_translation(Vec3::NEG_X * Physique::HIP_SPREAD),
    ]
}

/// The same ownership field is used for surface construction and deformation.
struct Pattern {
    trunk: Vec<Ring>,
    sleeve: Vec<Ring>,
    arm: Mat4,
    shorts: bool,
}

impl Pattern {
    fn new(grain: Grain, keeper: bool) -> Self {
        let mut sleeve = if keeper {
            BodyParts::sleeve(&[
                Ring::oval(-0.018, 0.065, 0.061),
                Ring::oval(-0.050, 0.063, 0.059),
                Ring::oval(-0.090, 0.0592, 0.0556),
                Ring::oval(-0.155, 0.0546, 0.0516),
                Ring::oval(-0.228, 0.0518, 0.0496),
                Ring::oval(-0.300, 0.047, 0.047),
            ])
        } else {
            BodyParts::sleeve(&[
                Ring::oval(-0.018, 0.065, 0.061),
                Ring::oval(-0.050, 0.063, 0.059),
                Ring::oval(-0.085, 0.061, 0.057),
                Ring::oval(-0.112, 0.0592, 0.055),
                Ring::oval(-0.132, 0.056, 0.053),
            ])
        };
        if keeper {
            // The elbow closure belongs to the upper sleeve, as on the bare arm.
            for step in 1..=12 {
                let (down, radius) = (FRAC_PI_2 * step as f32 / 12.0).sin_cos();
                sleeve.push(Ring::oval(
                    -0.300 - 0.047 * down,
                    0.047 * radius.max(0.0),
                    0.047 * radius.max(0.0),
                ));
            }
        }
        sleeve.reverse();
        Self {
            trunk: BodyParts::shirt(grain),
            sleeve: Sculptor::curved(grain, &sleeve),
            arm: shoulder(1.0).inverse(),
            shorts: false,
        }
    }

    fn shorts(grain: Grain) -> Self {
        let mut leg = Sculptor::curved(grain, &BodyParts::SHORTS_LEG);
        leg.reverse();
        Self {
            trunk: BodyParts::seat(grain),
            sleeve: leg,
            arm: Mat4::from_translation(Vec3::new(-Physique::HIP_SPREAD, 0.0, 0.0)),
            shorts: true,
        }
    }

    fn distance(profile: &[Ring], p: Vec3, relief: Relief) -> f32 {
        // Binary search: this is sampled at every grid corner and normal.
        let end = profile.partition_point(|ring| ring.y < p.y);
        let ring = if end == 0 {
            profile[0]
        } else if end == profile.len() {
            profile[end - 1]
        } else {
            let a = profile[end - 1];
            let b = profile[end];
            a.lerp(b, (p.y - a.y) / (b.y - a.y).max(1e-6))
        };
        let r = Ring {
            x: ring.x.max(0.0001),
            z: ring.z.max(0.0001),
            ..ring
        };
        let mut radial = (r.radius(p.x, p.z) - 1.0) * r.x.min(r.z);
        if !relief.0.is_empty() {
            let x = p.x / r.x;
            let z = (p.z - r.offset) / r.z;
            let angle = (z.signum() * z.abs().powf(r.edge * 0.5))
                .atan2(x.signum() * x.abs().powf(r.edge * 0.5));
            let (x, z) = r.at(angle);
            let along = (p.y - profile[0].y) / (profile[profile.len() - 1].y - profile[0].y);
            radial -=
                relief.at(angle, p.y, along) * r.x.min(r.z) / x.hypot(z - r.offset).max(0.0001);
        }
        let cap = (profile[0].y - p.y).max(p.y - profile[profile.len() - 1].y);
        // A small rolled edge gives the hem a continuous normal. A sharp
        // plane/cylinder intersection produces alternating dark facets where
        // a coarse triangle straddles that corner.
        let roll = ((0.016 - (radial - cap).abs()) / 0.016).max(0.0);
        radial.max(cap) + roll * roll * 0.004
    }

    fn distances(&self, p: Vec3) -> (f32, f32, f32) {
        let local = self.arm.transform_point3(Vec3::new(p.x.abs(), p.y, p.z));
        // The cuff must follow its arm completely. Limit the shared cloth to
        // the shoulder and armpit, then narrow both blends before the hem.
        let blend = if self.shorts {
            0.045
        } else {
            0.038 * (1.0 - Relief::fade((-local.y - 0.030) / 0.065))
        };
        (
            Self::distance(
                &self.trunk,
                p,
                if self.shorts {
                    Relief::SMOOTH
                } else {
                    BodyParts::TRUNK
                },
            ),
            Self::distance(&self.sleeve, local, Relief::SMOOTH),
            blend.max(0.0001),
        )
    }

    fn field(&self, p: Vec3) -> f32 {
        let (chest, arm, width) = self.distances(p);
        let blend = ((width - (chest - arm).abs()) / width).max(0.0);
        chest.min(arm) - blend * blend * width * 0.25
    }

    fn normal(&self, p: Vec3) -> Vec3 {
        let e = 0.0005;
        Vec3::new(
            self.field(p + Vec3::X * e) - self.field(p - Vec3::X * e),
            self.field(p + Vec3::Y * e) - self.field(p - Vec3::Y * e),
            self.field(p + Vec3::Z * e) - self.field(p - Vec3::Z * e),
        )
        .normalize_or_zero()
    }

    fn weight(&self, p: Vec3) -> f32 {
        let (chest, arm, width) = self.distances(p);
        Relief::fade(0.5 + (chest - arm) / (width * 2.4))
    }
}

pub(super) fn shirt(grain: Grain, keeper: bool) -> Mesh {
    // Tests build the same pattern many times; production makes each once.
    #[cfg(test)]
    {
        use std::sync::OnceLock;
        static MESHES: [OnceLock<Mesh>; 4] = [const { OnceLock::new() }; 4];
        let index = usize::from(grain == Grain::SPARE) * 2 + usize::from(keeper);
        MESHES[index].get_or_init(|| cut(grain, keeper)).clone()
    }
    #[cfg(not(test))]
    cut(grain, keeper)
}

fn cut(grain: Grain, keeper: bool) -> Mesh {
    let pattern = Pattern::new(grain, keeper);
    let origin = Vec3::new(-0.39, -0.035, -0.155);
    let size = Vec3::new(0.78, 0.65, 0.31);
    surface(grain, &pattern, origin, size, !keeper)
}

pub(super) fn shorts(grain: Grain) -> Mesh {
    let make = || {
        surface(
            grain,
            &Pattern::shorts(grain),
            Vec3::new(-0.24, -0.35, -0.14),
            Vec3::new(0.48, 0.40, 0.28),
            false,
        )
    };
    #[cfg(test)]
    {
        use std::sync::OnceLock;
        static MESHES: [OnceLock<Mesh>; 2] = [const { OnceLock::new() }; 2];
        MESHES[usize::from(grain == Grain::SPARE)]
            .get_or_init(make)
            .clone()
    }
    #[cfg(not(test))]
    make()
}

fn surface(grain: Grain, pattern: &Pattern, origin: Vec3, size: Vec3, cuff: bool) -> Mesh {
    let spacing = if grain == Grain::SPARE { 0.020 } else { 0.010 };
    let counts = (size / spacing).ceil().as_uvec3() + UVec3::ONE;
    let at = |x: u32, y: u32, z: u32| (x + counts.x * (y + counts.y * z)) as usize;
    let mut points = Vec::new();
    let mut values = Vec::new();
    for z in 0..counts.z {
        for y in 0..counts.y {
            for x in 0..counts.x {
                let p = origin + Vec3::new(x as f32, y as f32, z as f32) * spacing;
                points.push(p);
                values.push(pattern.field(p));
            }
        }
    }
    let mut positions = Vec::<[f32; 3]>::new();
    let mut normals = Vec::<[f32; 3]>::new();
    let mut indices = Vec::new();
    let mut edges = HashMap::<(usize, usize), u32>::new();
    // The shared cube diagonal keeps adjacent tetrahedra conforming.
    const TETRA: [[usize; 4]; 6] = [
        [0, 1, 3, 7],
        [0, 3, 2, 7],
        [0, 2, 6, 7],
        [0, 6, 4, 7],
        [0, 4, 5, 7],
        [0, 5, 1, 7],
    ];
    for z in 0..counts.z - 1 {
        for y in 0..counts.y - 1 {
            for x in 0..counts.x - 1 {
                let cube = [
                    at(x, y, z),
                    at(x + 1, y, z),
                    at(x, y + 1, z),
                    at(x + 1, y + 1, z),
                    at(x, y, z + 1),
                    at(x + 1, y, z + 1),
                    at(x, y + 1, z + 1),
                    at(x + 1, y + 1, z + 1),
                ];
                if cube.iter().all(|&i| values[i] >= 0.0) || cube.iter().all(|&i| values[i] < 0.0) {
                    continue;
                }
                for tet in TETRA {
                    let mut face = Vec::with_capacity(4);
                    for [a, b] in [[0, 1], [0, 2], [0, 3], [1, 2], [1, 3], [2, 3]] {
                        let (a, b) = (cube[tet[a]], cube[tet[b]]);
                        if (values[a] < 0.0) == (values[b] < 0.0) {
                            continue;
                        }
                        let key = (a.min(b), a.max(b));
                        let index = *edges.entry(key).or_insert_with(|| {
                            let p = points[a].lerp(points[b], values[a] / (values[a] - values[b]));
                            let index = positions.len() as u32;
                            positions.push(p.to_array());
                            normals.push(pattern.normal(p).to_array());
                            index
                        });
                        face.push(index);
                    }
                    if face.len() < 3 {
                        continue;
                    }
                    let center = face
                        .iter()
                        .map(|&i| Vec3::from(positions[i as usize]))
                        .sum::<Vec3>()
                        / face.len() as f32;
                    let normal = pattern.normal(center);
                    let axis =
                        (Vec3::from(positions[face[0] as usize]) - center).normalize_or_zero();
                    let cross = normal.cross(axis);
                    face.sort_by(|&a, &b| {
                        let a = Vec3::from(positions[a as usize]) - center;
                        let b = Vec3::from(positions[b as usize]) - center;
                        a.dot(cross)
                            .atan2(a.dot(axis))
                            .total_cmp(&b.dot(cross).atan2(b.dot(axis)))
                    });
                    for n in 1..face.len() - 1 {
                        indices.extend([face[0], face[n], face[n + 1]]);
                    }
                }
            }
        }
    }
    let (positions, normals, uvs, indices) = if !cuff {
        let count = positions.len();
        let swatch = if pattern.shorts {
            Swatch::Shorts
        } else {
            Swatch::Shirt
        };
        (positions, normals, vec![swatch.uv(); count], indices)
    } else {
        trim(pattern, positions, normals, indices)
    };
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

type Surface = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>);

/// Split triangles at the cuff boundary. Each face samples one swatch, while
/// the two colours share exactly the same surface and deformation weights.
fn trim(
    pattern: &Pattern,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
) -> Surface {
    let distances: Vec<f32> = positions
        .iter()
        .map(|&p| {
            let p = Vec3::from(p);
            let local = pattern.arm.transform_point3(Vec3::new(p.x.abs(), p.y, p.z));
            let (chest, arm, _) = pattern.distances(p);
            (local.y + 0.116).max(arm - chest)
        })
        .collect();
    let mut out = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut vertices = HashMap::new();
    for tri in indices.as_chunks::<3>().0 {
        for cuff in [false, true] {
            let mut polygon = Vec::with_capacity(4);
            for edge in 0..3 {
                let a = tri[edge] as usize;
                let b = tri[(edge + 1) % 3] as usize;
                let inside_a = (distances[a] < 0.0) == cuff;
                let inside_b = (distances[b] < 0.0) == cuff;
                if inside_a {
                    polygon.push((a, a));
                }
                if inside_a != inside_b {
                    polygon.push((a.min(b), a.max(b)));
                }
            }
            let polygon: Vec<u32> = polygon
                .into_iter()
                .map(|(a, b)| {
                    *vertices.entry((a, b, cuff)).or_insert_with(|| {
                        let t = if a == b {
                            0.0
                        } else {
                            distances[a] / (distances[a] - distances[b])
                        };
                        let p = Vec3::from(positions[a]).lerp(Vec3::from(positions[b]), t);
                        let n = Vec3::from(normals[a])
                            .lerp(Vec3::from(normals[b]), t)
                            .normalize_or_zero();
                        let index = out.0.len() as u32;
                        out.0.push(p.to_array());
                        out.1.push(n.to_array());
                        out.2
                            .push(if cuff { Swatch::Trim } else { Swatch::Shirt }.uv());
                        index
                    })
                })
                .collect();
            for n in 1..polygon.len().saturating_sub(1) {
                out.3.extend([polygon[0], polygon[n], polygon[n + 1]]);
            }
        }
    }
    out
}

/// Added after merging the collar, so every attribute has the same length.
pub(super) fn skin(mesh: &mut Mesh, grain: Grain, keeper: bool) {
    let pattern = Pattern::new(grain, keeper);
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap();
    let mut joints = Vec::with_capacity(positions.len());
    let mut weights = Vec::with_capacity(positions.len());
    for &p in positions {
        let p = Vec3::from(p);
        let arm = pattern.weight(p);
        joints.push([0, if p.x < 0.0 { 1 } else { 2 }, 0, 0]);
        weights.push([1.0 - arm, arm, 0.0, 0.0]);
    }
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_INDEX,
        VertexAttributeValues::Uint16x4(joints),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    mesh.generate_skinned_mesh_bounds()
        .expect("shirt weights match its vertices");
}

pub(super) fn skin_shorts(mesh: &mut Mesh) {
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap();
    let mut weights = Vec::with_capacity(positions.len());
    for &p in positions {
        let leg = Relief::fade((-p[1] - 0.025) / 0.17);
        // Share the crotch between both hips. Below it, each opening follows
        // its own thigh so striding cannot pull the opposite hem across it.
        let width = 0.06 * (1.0 - Relief::fade((-p[1] - 0.14) / 0.07)) + 0.001;
        let right = Relief::fade(0.5 + p[0] / width);
        weights.push([1.0 - leg, leg * (1.0 - right), leg * right, 0.0]);
    }
    let count = weights.len();
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_INDEX,
        VertexAttributeValues::Uint16x4(vec![[0, 1, 2, 0]; count]),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    mesh.generate_skinned_mesh_bounds()
        .expect("shorts weights match their vertices");
}

#[cfg(test)]
pub(super) fn posed(mesh: &Mesh, grain: Grain, gait: Gait, keeper: bool) -> Mesh {
    let mut mesh = mesh.clone();
    skin(&mut mesh, grain, keeper);
    let mut transforms = vec![Mat4::IDENTITY];
    for side in [-1.0, 1.0] {
        let origin = Vec3::new(side * Physique::SHOULDER_SPREAD, Physique::SHOULDER, 0.0);
        transforms.push(
            skeleton::step(Limb::Shoulder, side, origin, gait).to_matrix()
                * shoulder(side).inverse(),
        );
    }
    deform(mesh, &transforms)
}

#[cfg(test)]
pub(super) fn posed_shorts(mesh: &Mesh, gait: Gait) -> Mesh {
    let mut mesh = mesh.clone();
    skin_shorts(&mut mesh);
    let hips = Vec3::Y * Physique::HIP;
    let bind = shorts_binds();
    let transforms = [
        skeleton::step(Limb::Pelvis, 0.0, hips, gait).to_matrix(),
        skeleton::step(Limb::Hip, -1.0, hips - Vec3::X * Physique::HIP_SPREAD, gait).to_matrix()
            * bind[1],
        skeleton::step(Limb::Hip, 1.0, hips + Vec3::X * Physique::HIP_SPREAD, gait).to_matrix()
            * bind[2],
    ];
    deform(mesh, &transforms)
}

#[cfg(test)]
fn deform(mut mesh: Mesh, transforms: &[Mat4]) -> Mesh {
    let Some(VertexAttributeValues::Uint16x4(joints)) = mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
    else {
        unreachable!()
    };
    let Some(VertexAttributeValues::Float32x4(weights)) =
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
    else {
        unreachable!()
    };
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap();
    let normals = mesh
        .attribute(Mesh::ATTRIBUTE_NORMAL)
        .unwrap()
        .as_float3()
        .unwrap();
    let mut moved = Vec::new();
    let mut turned = Vec::new();
    for index in 0..positions.len() {
        let mut matrix = Mat4::ZERO;
        for slot in 0..4 {
            matrix += transforms[joints[index][slot] as usize] * weights[index][slot];
        }
        moved.push(
            matrix
                .transform_point3(Vec3::from(positions[index]))
                .to_array(),
        );
        turned.push(
            matrix
                .transform_vector3(Vec3::from(normals[index]))
                .normalize_or_zero()
                .to_array(),
        );
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, moved);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, turned);
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembled_clothes_use_each_players_own_joints() {
        use crate::players::kit::HairStyle;
        let mut meshes = Assets::<Mesh>::default();
        let mut binds = Assets::<SkinnedMeshInverseBindposes>::default();
        let mut parts = BodyParts::new(&mut meshes, Grain::SPARE);
        parts.bind_clothes(&mut binds);
        let outfit = Outfit {
            kit: Handle::default(),
            limb: Handle::default(),
            strip: 0,
            boots: Handle::default(),
            skin: Handle::default(),
            face: Handle::default(),
            hands: Handle::default(),
            hair: Handle::default(),
            hair_style: HairStyle::Shaved,
            number: None,
            name: None,
            name_front: None,
        };
        let mut world = World::new();
        for keeper in [false, true] {
            let owner = world.spawn(Transform::default()).id();
            Footballer::assemble(&mut world.commands(), owner, &parts, &outfit, keeper);
            world.flush();
        }
        let mut count = 0;
        for (entity, shirt, mesh, chest) in world
            .query::<(Entity, &SkinnedMesh, &Mesh3d, &Joint)>()
            .iter(&world)
        {
            count += 1;
            assert_eq!(shirt.joints.len(), 3);
            assert_eq!(shirt.joints[0], entity);
            let shorts = matches!(chest.limb, Limb::Pelvis);
            assert!(shorts || matches!(chest.limb, Limb::Torso));
            assert_eq!(binds.get(&shirt.inverse_bindposes).unwrap().len(), 3);
            assert!(world.get::<DynamicSkinnedMeshBounds>(entity).is_some());
            if shorts {
                assert_eq!(mesh.0, parts.pelvis);
            } else {
                assert!(mesh.0 == parts.torso || mesh.0 == parts.torso_long);
            }
            for (index, side) in [(1, -1.0), (2, 1.0)] {
                let arm = world.get::<Joint>(shirt.joints[index]).unwrap();
                assert!(if shorts {
                    matches!(arm.limb, Limb::Hip)
                } else {
                    matches!(arm.limb, Limb::Shoulder)
                });
                assert_eq!(arm.side, side);
                assert_eq!(arm.owner, chest.owner);
            }
        }
        assert_eq!(count, 4);
        let heads = world
            .query::<(&Joint, &Mesh3d, Option<&SkinnedMesh>)>()
            .iter(&world)
            .filter(|(joint, _, _)| matches!(joint.limb, Limb::Head))
            .map(|(_, mesh, skin)| {
                assert_eq!(mesh.0, parts.head);
                assert!(skin.is_none());
            })
            .count();
        assert_eq!(heads, 2);
    }

    fn root(parents: &mut [usize], mut at: usize) -> usize {
        while parents[at] != at {
            parents[at] = parents[parents[at]];
            at = parents[at];
        }
        at
    }

    #[test]
    fn each_garment_is_one_connected_surface() {
        for grain in [Grain::FULL, Grain::SPARE] {
            for mesh in [shirt(grain, false), shirt(grain, true), shorts(grain)] {
                let positions = mesh
                    .attribute(Mesh::ATTRIBUTE_POSITION)
                    .unwrap()
                    .as_float3()
                    .unwrap();
                let normals = mesh
                    .attribute(Mesh::ATTRIBUTE_NORMAL)
                    .unwrap()
                    .as_float3()
                    .unwrap();
                let Some(VertexAttributeValues::Float32x2(uvs)) =
                    mesh.attribute(Mesh::ATTRIBUTE_UV_0)
                else {
                    panic!("shirt UVs")
                };
                let mut parents: Vec<_> = (0..positions.len()).collect();
                let mut seams = HashMap::new();
                // The only duplicate vertices are colour boundaries. Weld
                // those for topology; their positions and normals must match.
                for (index, &p) in positions.iter().enumerate() {
                    assert!(Vec3::from(p).is_finite());
                    assert!((Vec3::from(normals[index]).length() - 1.0).abs() < 0.001);
                    let key = p.map(|v| (v * 1_000_000.0).round() as i32);
                    if let Some(&other) = seams.get(&key) {
                        let parent = root(&mut parents, other);
                        parents[index] = parent;
                        assert!(Vec3::from(normals[index]).dot(Vec3::from(normals[other])) > 0.99);
                    } else {
                        seams.insert(key, index);
                    }
                }
                let indices: Vec<_> = mesh.indices().unwrap().iter().collect();
                for tri in indices.as_chunks::<3>().0 {
                    let first = root(&mut parents, tri[0]);
                    for &index in &tri[1..] {
                        let other = root(&mut parents, index);
                        parents[other] = first;
                        assert_eq!(
                            uvs[index], uvs[tri[0]],
                            "a face interpolates between swatches"
                        );
                    }
                }
                let first = root(&mut parents, indices[0]);
                for index in indices {
                    assert_eq!(
                        root(&mut parents, index),
                        first,
                        "a garment contains a separate shell"
                    );
                }
            }
        }
    }

    #[test]
    fn skin_weights_anchor_chest_and_cuffs_to_the_existing_rig() {
        for grain in [Grain::FULL, Grain::SPARE] {
            for keeper in [false, true] {
                let mut mesh = shirt(grain, keeper);
                skin(&mut mesh, grain, keeper);
                let positions = mesh
                    .attribute(Mesh::ATTRIBUTE_POSITION)
                    .unwrap()
                    .as_float3()
                    .unwrap();
                let Some(VertexAttributeValues::Uint16x4(joints)) =
                    mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
                else {
                    panic!("shirt joints")
                };
                let Some(VertexAttributeValues::Float32x4(weights)) =
                    mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
                else {
                    panic!("shirt weights")
                };
                let rest = [Mat4::IDENTITY, shoulder(-1.0), shoulder(1.0)];
                let binds = binds();
                let mut cuffs = 0;
                for (index, &p) in positions.iter().enumerate() {
                    let p = Vec3::from(p);
                    let w = weights[index];
                    assert!(w.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
                    assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
                    let mut reconstructed = Vec3::ZERO;
                    for slot in 0..4 {
                        let joint = joints[index][slot] as usize;
                        reconstructed += (rest[joint] * binds[joint]).transform_point3(p) * w[slot];
                    }
                    assert!(
                        p.distance(reconstructed) < 1e-6,
                        "bind pose shifts the cloth"
                    );
                    if p.x.abs() < 0.10 {
                        assert!(w[1] < 1e-6, "the arm moves the chest print");
                    }
                    let local = shoulder(p.x.signum()).inverse().transform_point3(p);
                    if local.y < -0.120 && local.x.abs() < 0.055 && local.z.abs() < 0.06 {
                        assert!(w[1] > 0.999, "the cuff slips against its arm");
                        cuffs += 1;
                    }
                }
                assert!(cuffs > 20, "the test did not sample the sleeves");
            }
        }
    }
}
