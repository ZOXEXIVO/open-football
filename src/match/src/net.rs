//! The goal netting, and what it does when the ball goes in.
//!
//! The engine now plays a goal out rather than teleporting the ball to the
//! centre spot the instant it crosses the line (see `ball/net.rs` in the match
//! engine). That means the recording carries the ball travelling into the
//! goal, stretching the mesh, and settling — and a flat, rigid sheet of
//! netting throws all of that away. The one moment in a football match that
//! everybody watches twice deserves a net that moves.
//!
//! # The ball's own position is the deflection
//!
//! No cloth simulation is needed and none would be honest: the engine already
//! solves how far past each panel the netting lets the ball travel, and that
//! distance IS the bulge. This module reads it straight off the ball —
//! wherever the ball is behind a panel, the mesh is there too — and invents
//! only what the recording cannot carry: the wobble that runs on after the
//! ball has stopped pushing.
//!
//! # Shape of the deformation
//!
//! Two factors on top of the depth:
//!
//! * A **Gaussian around the contact point**, measured in metres, because a
//!   net is a membrane and pushing one point of it drags its neighbours in.
//! * An **edge pin**, because the mesh is tied to the frame. Without it a
//!   ball hit into the top corner peels the netting off the crossbar.

use crate::actors::{Actors, BallState};
use crate::field::Field;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use std::f32::consts::{FRAC_PI_2, TAU};

/// One deformable face of a goal net, and the state of the wave running
/// through it.
///
/// The panel is a trapezoid in its own frame: `across` spans it, `up` runs
/// from its bottom edge, and the up-extent may differ at the two ends — which
/// is what lets the side netting drop from the crossbar to the back bar
/// rather than standing as a rectangle. `normal` points out of the goal,
/// which is the way the ball pushes.
#[derive(Component)]
pub struct NetPanel {
    /// Undisturbed vertex positions, relative to the entity's translation.
    /// The mesh is rewritten from these every frame rather than accumulated,
    /// so a paused or seeked replay cannot leave the net permanently dented.
    rest: Vec<Vec3>,
    /// Parameter coordinates of each vertex, index for index with `rest`:
    /// `.x` runs -1..1 across the panel, `.y` runs 0..1 up it. Used only for
    /// the edge pin, which is a property of the frame the net is tied to
    /// rather than of any distance in metres.
    param: Vec<Vec2>,
    /// World-space frame. `origin` is the mid-point of the panel's bottom
    /// edge — the entity's translation, so `rest` is relative to it.
    origin: Vec3,
    across: Vec3,
    up: Vec3,
    normal: Vec3,
    half_across: f32,
    /// Up-extent at the `across = -half` and `across = +half` ends.
    edge_near: f32,
    edge_far: f32,
    /// How far the mesh is currently pushed out, in metres, and where — as a
    /// point on the panel, relative to `origin`.
    amplitude: f32,
    contact: Vec3,
    /// Phase of the wobble that runs on once the ball stops pushing. Held at
    /// zero while the ball is in the mesh, so contact reads as a pure bulge.
    phase: f32,
}

impl NetPanel {
    /// How far from the contact point the netting still moves. A football is
    /// 22 cm across and the mesh it drags with it is about three times that.
    const SPREAD: f32 = 0.62;

    /// Frequency and decay of the wobble after the ball settles. A goal net
    /// rings at a few hertz and is dead inside a second — it is mostly
    /// damping, held between the frame and the ball lying in the bottom of it.
    const WOBBLE_HZ: f32 = 3.2;
    const WOBBLE_HALF_LIFE: f32 = 0.28;

    /// Below this the net is at rest and the mesh is left alone — which is
    /// also what stops ten panels rewriting their vertex buffers every frame
    /// for the eighty-nine minutes of a match in which nothing goes in.
    const NEGLIGIBLE: f32 = 0.004;

    /// Up-extent at `u`, which runs -1..1 across the panel.
    #[inline]
    fn edge_at(&self, u: f32) -> f32 {
        let t = (u * 0.5 + 0.5).clamp(0.0, 1.0);
        self.edge_near + (self.edge_far - self.edge_near) * t
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        origin: Vec3,
        across: Vec3,
        up: Vec3,
        normal: Vec3,
        half_across: f32,
        edge_near: f32,
        edge_far: f32,
        steps: UVec2,
    ) -> (Self, Mesh) {
        let (nx, ny) = (steps.x.max(1), steps.y.max(1));
        let capacity = ((nx + 1) * (ny + 1)) as usize;
        let mut rest = Vec::with_capacity(capacity);
        let mut param = Vec::with_capacity(capacity);
        let mut uvs = Vec::with_capacity(capacity);

        for row in 0..=ny {
            let v = row as f32 / ny as f32;
            for column in 0..=nx {
                let t = column as f32 / nx as f32;
                let u = t * 2.0 - 1.0;
                let edge = edge_near + (edge_far - edge_near) * t;
                param.push(Vec2::new(u, v));
                rest.push(across * (u * half_across) + up * (v * edge));
                uvs.push([t, v]);
            }
        }

        let mut indices = Vec::with_capacity((nx * ny * 6) as usize);
        for row in 0..ny {
            for column in 0..nx {
                let a = row * (nx + 1) + column;
                let b = a + 1;
                let c = a + nx + 1;
                let d = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }

        let positions: Vec<[f32; 3]> = rest.iter().map(|p| [p.x, p.y, p.z]).collect();
        let normals = vec![[normal.x, normal.y, normal.z]; positions.len()];
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            // Read back and rewritten whenever the ball is in the goal, so
            // the vertex data has to stay in the main world.
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices));

        (
            NetPanel {
                rest,
                param,
                origin,
                across,
                up,
                normal,
                half_across,
                edge_near,
                edge_far,
                amplitude: 0.0,
                contact: Vec3::ZERO,
                phase: 0.0,
            },
            mesh,
        )
    }

    /// Take the ball's push on this panel for one frame.
    ///
    /// Returns `true` if the mesh needs rewriting — because the ball is in
    /// it, because it is still ringing, or because it has just come to rest
    /// and needs one last frame to lie flat.
    fn absorb(&mut self, ball: Vec3, ball_radius: f32, delta: f32) -> bool {
        let offset = ball - self.origin;
        let out = offset.dot(self.normal) + ball_radius;
        let u = offset.dot(self.across) / self.half_across.max(1e-3);
        let v = offset.dot(self.up) / self.edge_at(u).max(1e-3);

        // Half a ball's grace either side, so a shot into the very top
        // corner still takes the netting with it.
        const MARGIN: f32 = 0.25;
        let within = u.abs() <= 1.0 + MARGIN && (-MARGIN..=1.0 + MARGIN).contains(&v);

        if within && out > 0.0 {
            // The mesh is wherever the ball is: project the ball onto the
            // panel and push that point out by however far past it the ball
            // has travelled.
            self.contact = offset - self.normal * offset.dot(self.normal);
            self.amplitude = out;
            self.phase = 0.0;
            return true;
        }

        if self.amplitude <= Self::NEGLIGIBLE {
            let was_moving = self.amplitude != 0.0;
            self.amplitude = 0.0;
            return was_moving;
        }

        self.phase += TAU * Self::WOBBLE_HZ * delta;
        self.amplitude *= 0.5f32.powf(delta / Self::WOBBLE_HALF_LIFE);
        true
    }

    /// Write the current deformation into `mesh`.
    fn shape(&self, mesh: &mut Mesh) {
        let swing = self.amplitude * self.phase.cos();
        let positions: Vec<[f32; 3]> = self
            .rest
            .iter()
            .zip(self.param.iter())
            .map(|(rest, param)| {
                let point = rest + self.normal * (swing * self.weight(*rest, *param));
                [point.x, point.y, point.z]
            })
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    }

    /// How much of the deflection this vertex takes: a Gaussian around the
    /// contact — in metres, because that is where the membrane lives — pinned
    /// to zero at the frame.
    fn weight(&self, rest: Vec3, param: Vec2) -> f32 {
        let reach = (rest - self.contact).length() / Self::SPREAD;
        let bulge = (-reach * reach).exp();
        let pin_across = 1.0 - param.x.abs().powi(4);
        let pin_up = 1.0 - (param.y * 2.0 - 1.0).abs().powi(4);
        bulge * pin_across.max(0.0) * pin_up.max(0.0)
    }
}

/// Both goals: the frame, and the netting hung on it.
pub struct Netting;

impl Netting {
    const POST_RADIUS: f32 = 0.07;

    /// Grid resolution per panel. Fine enough that the bulge reads as a
    /// curve rather than a tent, coarse enough that rewriting one is a few
    /// hundred floats.
    const BACK_STEPS: UVec2 = UVec2::new(18, 8);
    const SIDE_STEPS: UVec2 = UVec2::new(10, 8);
    const ROOF_STEPS: UVec2 = UVec2::new(18, 6);

    pub fn spawn(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
    ) {
        let frame = materials.add(StandardMaterial {
            base_color: Color::srgb(0.97, 0.97, 0.97),
            perceptual_roughness: 0.5,
            ..default()
        });
        let netting = materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 1.0, 1.0, 0.11),
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            double_sided: true,
            perceptual_roughness: 1.0,
            ..default()
        });

        let post = meshes.add(Cylinder::new(Self::POST_RADIUS, Field::GOAL_HEIGHT));
        let bar = meshes.add(Cylinder::new(
            Self::POST_RADIUS,
            Field::GOAL_WIDTH + Self::POST_RADIUS * 2.0,
        ));
        let half_goal = Field::GOAL_WIDTH * 0.5;
        let back_height = Field::NET_BACK_HEIGHT;

        for side in [-1.0f32, 1.0] {
            let goal_line = side * Field::HALF_LENGTH;
            // Out of the goal, for this end, is away from the pitch.
            let out = Vec3::new(side, 0.0, 0.0);

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

            // Back panel — the one that bags when a shot goes in.
            Self::panel(
                commands,
                meshes,
                &netting,
                Vec3::new(goal_line + side * Field::NET_DEPTH, 0.0, 0.0),
                Vec3::Z,
                Vec3::Y,
                out,
                half_goal,
                back_height,
                back_height,
                Self::BACK_STEPS,
            );

            // Side panels: a trapezoid each, full crossbar height at the goal
            // line falling to the back bar's height at the back.
            for post_side in [-1.0f32, 1.0] {
                Self::panel(
                    commands,
                    meshes,
                    &netting,
                    Vec3::new(
                        goal_line + side * Field::NET_DEPTH * 0.5,
                        0.0,
                        post_side * half_goal,
                    ),
                    out,
                    Vec3::Y,
                    Vec3::new(0.0, 0.0, post_side),
                    Field::NET_DEPTH * 0.5,
                    Field::GOAL_HEIGHT,
                    back_height,
                    Self::SIDE_STEPS,
                );
            }

            // Roof netting, sloping from the crossbar down to the back bar.
            // Its own "up" runs along the slope and its normal is the
            // perpendicular of that, tilted back over the goal.
            let slope = Vec3::new(side * Field::NET_DEPTH, back_height - Field::GOAL_HEIGHT, 0.0);
            let length = slope.length();
            let along = slope / length;
            let above = Vec3::new(-along.y * side, along.x * side, 0.0).normalize();
            Self::panel(
                commands,
                meshes,
                &netting,
                Vec3::new(goal_line, Field::GOAL_HEIGHT, 0.0),
                Vec3::Z,
                along,
                above,
                half_goal,
                length,
                length,
                Self::ROOF_STEPS,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn panel(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        material: &Handle<StandardMaterial>,
        origin: Vec3,
        across: Vec3,
        up: Vec3,
        normal: Vec3,
        half_across: f32,
        edge_near: f32,
        edge_far: f32,
        steps: UVec2,
    ) {
        let (panel, mesh) = NetPanel::build(
            origin,
            across,
            up,
            normal,
            half_across,
            edge_near,
            edge_far,
            steps,
        );
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material.clone()),
            // The vertices already carry the panel's world orientation, so
            // the entity only has to be moved to its origin.
            Transform::from_translation(origin),
            panel,
        ));
    }

    /// Push the netting around with the ball, once per frame.
    pub fn ripple(
        ball: Res<BallState>,
        time: Res<Time>,
        mut meshes: ResMut<Assets<Mesh>>,
        mut panels: Query<(&mut NetPanel, &Mesh3d)>,
    ) {
        let delta = time.delta_secs().clamp(1e-4, 0.1);
        for (mut panel, handle) in &mut panels {
            if !panel.absorb(ball.position, Actors::BALL_RADIUS, delta) {
                continue;
            }
            if let Some(mut mesh) = meshes.get_mut(&handle.0) {
                panel.shape(&mut mesh);
            }
        }
    }
}
