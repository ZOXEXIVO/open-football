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

use crate::art::textures::Textures;
use crate::players::actors::{Actors, BallState};
use crate::scene::field::Field;
use crate::scene::pitch::Pitch;
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

    /// Extra spread per metre of push — see [`Self::weight`]. A little over
    /// one, so a shot that bags the net half a metre drags getting on for a
    /// metre of mesh with it either side of the contact.
    const SPREAD_PER_DEPTH: f32 = 1.15;

    /// Frequency and decay of the wobble after the ball settles. A goal net
    /// rings at a few hertz and is dead inside a second — it is mostly
    /// damping, held between the frame and the ball lying in the bottom of it.
    const WOBBLE_HZ: f32 = 3.2;
    const WOBBLE_HALF_LIFE: f32 = 0.28;

    /// Below this the net is at rest and the mesh is left alone — which is
    /// also what stops ten panels rewriting their vertex buffers every frame
    /// for the eighty-nine minutes of a match in which nothing goes in.
    const NEGLIGIBLE: f32 = 0.004;

    /// How far past a panel a ball can be and still be IN it: the engine's
    /// largest slack, `GoalNet::GIVE_BACK` = 8 game units.
    ///
    /// # This is what stopped the net moving when a shot MISSED
    ///
    /// The contact test used to be "is the ball past this panel's plane,
    /// anywhere within its extent" — with no bound on HOW FAR past. A side
    /// panel's plane is the post, extended sideways to the ends of the
    /// world, and its `across` axis is the goal's own 1.9 m depth. So a
    /// shot crossing the goal-line plane wide of the post satisfied both:
    /// it was inside the panel's depth (the plane is 24 cm thick either
    /// side once [`Self::MARGIN`] is allowed) and it was "past" the panel by
    /// however far outside the post it happened to be.
    ///
    /// `amplitude` is then that distance, and it feeds `shape` directly —
    /// so a shot that missed by three metres pushed the side netting three
    /// metres out over the touchline, and one that missed by ten pushed it
    /// ten. `weight`'s Gaussian widens with the push as well
    /// ([`Self::SPREAD_PER_DEPTH`]), so at that size the falloff is flat and
    /// the WHOLE panel goes with it. That is the reported "when the ball
    /// misses the goal, the net moves".
    ///
    /// A membrane tied along its edges cannot be pushed further than its
    /// slack, so anything beyond it is a ball that is somewhere else.
    /// [`Netting::inside_a_goal`] is the other half and the stricter one;
    /// this bound is what keeps the panel honest on its own.
    const MAX_GIVE: f32 = Netting::GIVE_BACK;

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
                // The mesh-square count is baked into the UVs rather than
                // set per material with `uv_transform`, for two reasons: one
                // material can then serve all ten panels, and — because the
                // side panels are TRAPEZOIDS — the repeat has to follow the
                // local edge length or the squares shear as the panel
                // narrows. A net's mesh does not stretch.
                uvs.push([
                    t * (half_across * 2.0) / Netting::MESH_SQUARE,
                    v * edge / Netting::MESH_SQUARE,
                ]);
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

    /// Half a ball's grace either side of a panel's own extent, so a shot
    /// into the very top corner still takes the netting with it.
    const MARGIN: f32 = 0.25;

    /// Take the ball's push on this panel for one frame.
    ///
    /// `ball` is `None` when the ball is not inside a goal at all — see
    /// [`Netting::inside_a_goal`] — in which case the panel only rings down
    /// whatever it was already carrying.
    ///
    /// Returns `true` if the mesh needs rewriting — because the ball is in
    /// it, because it is still ringing, or because it has just come to rest
    /// and needs one last frame to lie flat.
    fn absorb(&mut self, ball: Option<Vec3>, ball_radius: f32, delta: f32) -> bool {
        // How far past this panel the ball is pressing, or `None` if it is
        // not pressing on this one. A ball further out than the netting can
        // stretch was never in it: see [`Self::MAX_GIVE`].
        let push = ball.and_then(|ball| {
            let offset = ball - self.origin;
            let out = offset.dot(self.normal) + ball_radius;
            let u = offset.dot(self.across) / self.half_across.max(1e-3);
            let v = offset.dot(self.up) / self.edge_at(u).max(1e-3);
            let within =
                u.abs() <= 1.0 + Self::MARGIN && (-Self::MARGIN..=1.0 + Self::MARGIN).contains(&v);
            (within && out > 0.0 && out <= Self::MAX_GIVE + ball_radius).then_some((offset, out, u))
        });

        if let Some((offset, out, u)) = push {
            // The mesh is wherever the ball is: project the ball onto the
            // panel and push that point out by however far past it the ball
            // has travelled.
            //
            // CLAMPED into the panel's own extent, and that is load-bearing.
            // `within` allows half a ball of grace past each edge so a shot
            // into the top corner still takes the netting with it — but the
            // Gaussian in `weight` is measured from this point, so a contact
            // left outside the panel puts its own peak where there are no
            // vertices and the visible bulge collapses to the tail of the
            // curve. A ball resting against a post is exactly that case, and
            // it is the commonest place for one to finish.
            let flat = offset - self.normal * offset.dot(self.normal);
            let along = flat
                .dot(self.across)
                .clamp(-self.half_across, self.half_across);
            let up = flat.dot(self.up).clamp(0.0, self.edge_at(u));
            self.contact = self.across * along + self.up * up;
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
    ///
    /// The spread WIDENS with the push. A membrane does not have a fixed
    /// influence radius: leaning on it a centimetre moves a hand's breadth
    /// of mesh, and putting half a metre of it into the stanchion drags the
    /// whole panel in. With `SPREAD` alone the deep bulges came out as a
    /// narrow spike rather than a bag, which is neither what a net does nor
    /// what makes the movement readable.
    fn weight(&self, rest: Vec3, param: Vec2) -> f32 {
        let spread = Self::SPREAD + self.amplitude.abs() * Self::SPREAD_PER_DEPTH;
        let reach = (rest - self.contact).length() / spread;
        let bulge = (-reach * reach).exp();
        // Pinned at the two edges the panel is TIED to. The exponent decides
        // how much of the panel the pin steals: at the 4th power the
        // constraint is felt only in the last fifth, which is what a cord
        // lashed to a bar actually does.
        let pin_across = 1.0 - param.x.abs().powi(4);
        let pin_up = 1.0 - (param.y * 2.0 - 1.0).abs().powi(4);
        bulge * pin_across.max(0.0) * pin_up.max(0.0)
    }
}

/// Both goals: the frame, and the netting hung on it.
pub struct Netting;

impl Netting {
    /// Matching `GoalFrame::POST_RADIUS` in the engine — see
    /// [`Field::POST_RADIUS`] for why the two have to agree now that the
    /// physics rebounds the ball off the woodwork.
    const POST_RADIUS: f32 = Field::POST_RADIUS;

    /// Grid resolution per panel. Fine enough that the bulge reads as a
    /// curve rather than a tent, coarse enough that rewriting one is a few
    /// hundred floats.
    const BACK_STEPS: UVec2 = UVec2::new(18, 8);
    const SIDE_STEPS: UVec2 = UVec2::new(10, 8);
    const ROOF_STEPS: UVec2 = UVec2::new(18, 6);

    /// Side of one mesh square, in metres. A goal net is knotted at about
    /// this pitch, and it is what turns the panel's real size into a texture
    /// repeat count — see [`Textures::netting`].
    const MESH_SQUARE: f32 = 0.12;

    /// How far the netting lets the ball past each panel, matching the
    /// engine's `GoalNet::GIVE_BACK` (8 game units) and `GIVE_SIDE` (4).
    /// The back net is hung slack and bags; the sides and roof are pulled
    /// tighter, which is why a ball driven into the side netting stops
    /// against it rather than in it.
    ///
    /// They differ, and using the largest for all three axes is what let a
    /// ball SAILING OVER THE BAR count as being in the goal — 3.4 m is
    /// inside 2.44 + 1.0 and outside 2.44 + 0.5. See
    /// [`Self::inside_a_goal`].
    const GIVE_BACK: f32 = 8.0 * Field::METERS_PER_UNIT;
    const GIVE_SIDE: f32 = 4.0 * Field::METERS_PER_UNIT;

    pub fn spawn(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
    ) {
        let frame = materials.add(StandardMaterial {
            base_color: Color::srgb(0.97, 0.97, 0.97),
            perceptual_roughness: 0.5,
            ..default()
        });
        // The cords come from the texture's alpha, so the base colour carries
        // only their brightness — tinting it as well would darken the twine
        // twice. `cull_mode: None` because a goal is looked into from the
        // front and out of from behind.
        //
        // **Unlit, and that is a statement about the geometry rather than a
        // shortcut.** A panel's normal is a fiction here: the surface being
        // drawn is not a sheet facing one way, it is a mesh of cylinders
        // facing every way at once, so from any angle some part of every cord
        // has the light on it. Shaded as a sheet, the side panels — whose
        // normals point at the touchlines — came out a dark grey mesh against
        // the grass, and a goal net is white from wherever you stand. The
        // stadium is lit from four corners (`Pitch::SUN` and the fills), so
        // there is no shadow here for the diffuse term to be earning either.
        let cords = Textures::netting(images);
        let netting = materials.add(StandardMaterial {
            base_color: Color::srgb(0.88, 0.90, 0.92),
            base_color_texture: Some(cords),
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            double_sided: true,
            unlit: true,
            ..default()
        });

        // **The frame is built on the INNER faces, which is where the Laws
        // measure a goal and where the physics keeps its posts.** Each
        // member's axis therefore sits one radius OUTSIDE the nominal
        // dimension: the posts wide of the mouth and the bar above it. Put
        // the axes on the nominal lines instead — which is what this used to
        // do — and half of every member hangs inside the goal, so a shot the
        // engine scores is drawn passing through the woodwork and a shot the
        // engine rebounds is drawn missing it.
        let half_goal = Field::PHYSICS_GOAL_HALF_WIDTH;
        let post_axis = half_goal + Self::POST_RADIUS;
        let bar_axis = Field::PHYSICS_GOAL_HEIGHT + Self::POST_RADIUS;
        // The upright runs from the grass to the underside of the bar.
        let post = Pitch::stock(meshes, Cylinder::new(Self::POST_RADIUS, bar_axis).into());
        let bar = Pitch::stock(
            meshes,
            Cylinder::new(Self::POST_RADIUS, (post_axis + Self::POST_RADIUS) * 2.0).into(),
        );
        let back_height = Field::NET_BACK_HEIGHT;

        for side in [-1.0f32, 1.0] {
            let goal_line = side * Field::HALF_LENGTH;
            // Out of the goal, for this end, is away from the pitch.
            let out = Vec3::new(side, 0.0, 0.0);

            for post_side in [-1.0f32, 1.0] {
                commands.spawn((
                    Mesh3d(post.clone()),
                    MeshMaterial3d(frame.clone()),
                    Transform::from_xyz(goal_line, bar_axis * 0.5, post_side * post_axis),
                ));
            }

            commands.spawn((
                Mesh3d(bar.clone()),
                MeshMaterial3d(frame.clone()),
                Transform::from_xyz(goal_line, bar_axis, 0.0)
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
                    Field::PHYSICS_GOAL_HEIGHT,
                    back_height,
                    Self::SIDE_STEPS,
                );
            }

            // Roof netting, sloping from the crossbar down to the back bar.
            // Its own "up" runs along the slope and its normal is the
            // perpendicular of that, tilted back over the goal.
            let slope = Vec3::new(
                side * Field::NET_DEPTH,
                back_height - Field::PHYSICS_GOAL_HEIGHT,
                0.0,
            );
            let length = slope.length();
            let along = slope / length;
            let above = Vec3::new(-along.y * side, along.x * side, 0.0).normalize();
            Self::panel(
                commands,
                meshes,
                &netting,
                Vec3::new(goal_line, Field::PHYSICS_GOAL_HEIGHT, 0.0),
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
            Mesh3d(Pitch::stock(meshes, mesh)),
            MeshMaterial3d(material.clone()),
            // The vertices already carry the panel's world orientation, so
            // the entity only has to be moved to its origin.
            Transform::from_translation(origin),
            panel,
        ));
    }

    /// The back panel of the right-hand goal, built exactly as `spawn`
    /// builds it. Exists so the deformation can be interrogated without a
    /// GPU — see the tests below.
    #[cfg(test)]
    fn back_panel() -> (NetPanel, Mesh) {
        NetPanel::build(
            Vec3::new(Field::HALF_LENGTH + Field::NET_DEPTH, 0.0, 0.0),
            Vec3::Z,
            Vec3::Y,
            Vec3::X,
            Field::PHYSICS_GOAL_HALF_WIDTH,
            Field::NET_BACK_HEIGHT,
            Field::NET_BACK_HEIGHT,
            Self::BACK_STEPS,
        )
    }

    /// The near-touchline side panel of the right-hand goal, built exactly
    /// as `spawn` builds it. This is the panel a shot that misses wide
    /// passes closest to, so it is the one the miss test needs.
    #[cfg(test)]
    fn side_panel() -> (NetPanel, Mesh) {
        NetPanel::build(
            Vec3::new(
                Field::HALF_LENGTH + Field::NET_DEPTH * 0.5,
                0.0,
                Field::PHYSICS_GOAL_HALF_WIDTH,
            ),
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            Field::NET_DEPTH * 0.5,
            Field::PHYSICS_GOAL_HEIGHT,
            Field::NET_BACK_HEIGHT,
            Self::SIDE_STEPS,
        )
    }

    /// Is the ball inside one of the two goals?
    ///
    /// **A net bags because the ball is IN it.** Nothing else moves one: a
    /// ball passing outside a post is beside the netting, not against it,
    /// and one thumped down the touchline is not near a goal at all.
    ///
    /// Each panel used to answer that for itself, from its own plane and
    /// extent, and the side panels cannot — their plane is the post, which
    /// divides the whole world into "inside the goal" and "the rest of the
    /// pitch", and their `across` axis is the goal's 1.9 m depth, which a
    /// shot crossing the goal line satisfies exactly. So every ball that
    /// missed WIDE was read as pressing on the side netting from the
    /// inside, by however far wide it had missed. See [`NetPanel::MAX_GIVE`]
    /// for what that then did to the mesh.
    ///
    /// One test for the whole net, in the one place that knows where the
    /// goals are. The bounds are the goal's own volume plus the netting's
    /// slack, since the engine settles the ball INSIDE the mesh
    /// ([`Self::GIVE_BACK`] / [`Self::GIVE_SIDE`]) and those balls must
    /// still count.
    ///
    /// ⚠ **The goal line gets no grace at all, and that is the load-bearing
    /// half.** Every other bound is a metre or less from a genuinely wide
    /// shot, so the only thing that separates a ball bagging the side
    /// netting from one flashing past the post is whether it is BEHIND THE
    /// LINE. Allowing even a ball's radius of slack there lets the miss
    /// back in: a shot that goes wide crosses the plane of the goal line
    /// exactly, which is the frame the artefact was drawn on.
    /// `pub(crate)` so the soundtrack can ask the SAME question rather than
    /// asking its own: the paragraphs above are there because "in the goal"
    /// has exactly one answer in this crate, and a second copy of the rule is
    /// how the net comes to rustle for a ball the mix says went wide.
    pub(crate) fn inside_a_goal(ball: Vec3) -> bool {
        // Distance past the nearer goal line — negative out on the pitch.
        let past_line = ball.x.abs() - Field::HALF_LENGTH;
        past_line > 0.0
            // ⚠ **No lateral grace, for the same reason the goal line gets
            // none.** `GIVE_SIDE` is the slack the engine settles a ball
            // INTO the side netting by, and it is measured inward from the
            // panel; adding it to the half-width instead put a 50 cm band
            // just OUTSIDE each post inside the goal. That was nearly
            // unreachable while a ball wide of the post was snapped back
            // onto the pitch the instant it crossed the line. It is not any
            // more — a miss now runs on behind the goal (`core::RunOff`)
            // and passes straight through that band on its way to the
            // hoardings, and every one of them would ripple the side net as
            // if it had gone in. The post's own radius is the honest
            // boundary: a ball touching the outside of the post is not in
            // the goal.
            && past_line < Field::NET_DEPTH + Self::GIVE_BACK
            && ball.z.abs() < Field::PHYSICS_GOAL_HALF_WIDTH + Field::POST_RADIUS
            && ball.y < Self::roof_ceiling(past_line)
    }

    /// **How high the roof netting reaches, `past_line` metres behind the
    /// goal line.**
    ///
    /// The lid used to be flat at the crossbar plus the side give — 2.94 m
    /// across the whole 2.9 m of depth. A goal's roof net does not do
    /// that. It is a membrane pinned along two bars: the 2.44 m crossbar
    /// at the front and [`Field::NET_BACK_HEIGHT`] at the back, which is
    /// why a ball driven in under the bar dips as it goes, and it is the
    /// shape the roof PANEL is already built to ([`Netting::roof`] takes
    /// an `edge_near` and an `edge_far`).
    ///
    /// **And the give goes to zero at both ends**, for the reason
    /// `GoalNet::slack_at` gives in the engine: a membrane tied along an
    /// edge cannot be pushed AT that edge. Full slack in the middle of the
    /// panel, none at the bar. Carried here because the front pin is the
    /// crossbar itself, so without it a ball that has just cleared the bar
    /// is half a metre inside the volume at the exact moment it goes over.
    ///
    /// A flat lid is only ever wrong for a ball ABOVE the goal, and until
    /// a miss started running on behind the byline (`core::RunOff`) there
    /// was hardly ever one there. There is now: every ball over the bar
    /// flies the length of the goal at two and a half metres or more, and
    /// each one rippled the netting on its way over as though it had gone
    /// in. Reported 2026-08-21.
    ///
    /// Past the back bar the mesh has stopped sloping — it is the back
    /// panel from there on, and that panel's top IS the back height.
    #[inline]
    fn roof_ceiling(past_line: f32) -> f32 {
        let t = (past_line / Field::NET_DEPTH).clamp(0.0, 1.0);
        let mesh =
            Field::PHYSICS_GOAL_HEIGHT + (Field::NET_BACK_HEIGHT - Field::PHYSICS_GOAL_HEIGHT) * t;
        mesh + Self::GIVE_SIDE * 4.0 * t * (1.0 - t)
    }

    /// Push the netting around with the ball, once per frame.
    pub fn ripple(
        ball: Res<BallState>,
        time: Res<Time>,
        mut meshes: ResMut<Assets<Mesh>>,
        mut panels: Query<(&mut NetPanel, &Mesh3d)>,
    ) {
        let delta = time.delta_secs().clamp(1e-4, 0.1);
        // Resolved once for the whole net rather than per panel: it is a
        // question about the BALL, and asking it ten times invites ten
        // different answers — which is precisely how the side panels came
        // to disagree with the back one about what "in the goal" means.
        let contact = Self::inside_a_goal(ball.position).then_some(ball.position);
        for (mut panel, handle) in &mut panels {
            if !panel.absorb(contact, Actors::BALL_RADIUS, delta) {
                continue;
            }
            if let Some(mut mesh) = meshes.get_mut(&handle.0) {
                panel.shape(&mut mesh);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read the mesh's positions back out.
    fn positions(mesh: &Mesh) -> Vec<Vec3> {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(values)) => {
                values.iter().map(|p| Vec3::from(*p)).collect()
            }
            _ => panic!("the panel has no positions"),
        }
    }

    /// Furthest any vertex has been pushed along the panel's outward normal.
    fn deepest(panel: &NetPanel, mesh: &Mesh) -> f32 {
        positions(mesh)
            .iter()
            .zip(panel.rest.iter())
            .map(|(now, rest)| (*now - *rest).dot(panel.normal))
            .fold(f32::MIN, f32::max)
    }

    /// A goal net's whole job. The engine bags the mesh — it settles the ball
    /// inside the netting's give — and this end has to put the mesh THERE,
    /// or the ball is drawn hanging in space behind a flat sheet.
    #[test]
    fn the_mesh_reaches_the_ball_that_is_lying_in_it() {
        let (mut panel, mut mesh) = Netting::back_panel();
        // Half a metre into the back netting, at the height it is slackest,
        // dead centre of the goal.
        let ball = Vec3::new(
            Field::HALF_LENGTH + Field::NET_DEPTH + 0.5,
            Field::NET_BACK_HEIGHT * 0.5,
            0.0,
        );
        assert!(panel.absorb(Some(ball), Actors::BALL_RADIUS, 1.0 / 60.0));
        panel.shape(&mut mesh);
        let reach = deepest(&panel, &mesh);
        assert!(
            reach > 0.45,
            "the mesh has to follow the ball half a metre in, reached {reach:.3} m"
        );
        assert!(reach < 0.75, "…and not overshoot it, reached {reach:.3} m");
    }

    /// The commonest place for a ball to finish is wedged against a post,
    /// and the Gaussian in `weight` is measured from the contact point — so
    /// a contact left OUTSIDE the panel's own extent puts its peak where
    /// there are no vertices and the bulge collapses. That is the netting
    /// looking stationary for exactly the shot that most deserves it.
    #[test]
    fn a_ball_against_a_post_still_takes_the_netting_with_it() {
        let (mut panel, mut mesh) = Netting::back_panel();
        // Past the post, which the engine's side netting permits.
        let ball = Vec3::new(
            Field::HALF_LENGTH + Field::NET_DEPTH + 0.35,
            0.25,
            Field::PHYSICS_GOAL_HALF_WIDTH + 0.2,
        );
        assert!(panel.absorb(Some(ball), Actors::BALL_RADIUS, 1.0 / 60.0));
        panel.shape(&mut mesh);
        let reach = deepest(&panel, &mesh);
        assert!(
            reach > 0.15,
            "a ball in the corner of the goal must still bag the net, reached {reach:.3} m"
        );
    }

    /// Nothing in the goal, nothing moving. The mesh is rewritten from
    /// `rest` every frame rather than accumulated, so a paused or seeked
    /// replay must not be able to leave it permanently dented.
    #[test]
    fn an_empty_goal_has_a_flat_net() {
        let (mut panel, mut mesh) = Netting::back_panel();
        let ball = Vec3::new(0.0, 0.3, 0.0); // centre spot
        // First call reports nothing to do…
        assert!(!panel.absorb(Some(ball), Actors::BALL_RADIUS, 1.0 / 60.0));
        // …and if something does write the mesh, it writes it flat.
        panel.shape(&mut mesh);
        assert!(deepest(&panel, &mesh).abs() < 1.0e-6);
    }

    /// The wobble has to die. A net that rings for the rest of the match is
    /// worse than one that never moves.
    #[test]
    fn the_net_rings_and_then_settles() {
        let (mut panel, mut mesh) = Netting::back_panel();
        let ball = Vec3::new(
            Field::HALF_LENGTH + Field::NET_DEPTH + 0.5,
            Field::NET_BACK_HEIGHT * 0.5,
            0.0,
        );
        panel.absorb(Some(ball), Actors::BALL_RADIUS, 1.0 / 60.0);
        panel.shape(&mut mesh);
        assert!(deepest(&panel, &mesh) > 0.4, "it starts bagged");
        // Ball gone. Four seconds of frames, sampling what the ring has left
        // at one second — a net is visually dead well inside that, even
        // though the buffer keeps being rewritten down to `NEGLIGIBLE`.
        let mut ringing = 0;
        let mut after_a_second = f32::NAN;
        for frame in 0..240 {
            if panel.absorb(None, Actors::BALL_RADIUS, 1.0 / 60.0) {
                ringing += 1;
                panel.shape(&mut mesh);
            }
            if frame == 59 {
                after_a_second = deepest(&panel, &mesh).abs();
            }
        }
        assert!(ringing > 10, "it must actually ring, only {ringing} frames");
        assert!(
            after_a_second < 0.08,
            "a net is dead inside a second; {after_a_second:.3} m still moving"
        );
        assert!(
            ringing < 240,
            "and it must stop rewriting the mesh, still going after {ringing} frames"
        );
        assert!(
            deepest(&panel, &mesh).abs() < 1.0e-6,
            "and finish exactly flat, left at {:.5} m",
            deepest(&panel, &mesh)
        );
    }

    /// **A shot that misses does not move the net.** Reported from the
    /// viewer: *"when ball miss goals, net moving"*.
    ///
    /// The side panel's plane is the post, and a plane divides the whole
    /// world — so every ball outside the post read as pressing on the
    /// netting from the inside, by however far outside it was. The
    /// amplitude IS that distance, so a shot missing by two metres bagged
    /// the side net two metres over the touchline.
    #[test]
    fn a_shot_that_misses_wide_leaves_the_side_netting_alone() {
        let (mut panel, mut mesh) = Netting::side_panel();
        for wide in [0.4f32, 2.0, 8.0] {
            // Crossing the goal-line plane at chest height, `wide` metres
            // outside the near post — the commonest miss in football.
            let ball = Vec3::new(
                Field::HALF_LENGTH,
                1.2,
                Field::PHYSICS_GOAL_HALF_WIDTH + wide,
            );
            assert!(
                !Netting::inside_a_goal(ball),
                "a ball {wide} m wide of the post is not in the goal"
            );
            let contact = Netting::inside_a_goal(ball).then_some(ball);
            panel.absorb(contact, Actors::BALL_RADIUS, 1.0 / 60.0);
            panel.shape(&mut mesh);
            let reach = deepest(&panel, &mesh).abs();
            assert!(
                reach < 1.0e-6,
                "missing by {wide} m moved the side netting {reach:.3} m"
            );
        }
    }

    /// …and the panel refuses it on its own, without the volume test in
    /// front of it. Both halves have to hold: `inside_a_goal` is what keeps
    /// the ball out, `MAX_GIVE` is what keeps the panel honest if anything
    /// ever offers it one anyway.
    #[test]
    fn a_panel_refuses_a_ball_further_out_than_it_can_stretch() {
        let (mut panel, _) = Netting::side_panel();
        // Inside the goal's depth and height, so only the distance past the
        // post decides it.
        let just_in = Vec3::new(
            Field::HALF_LENGTH + Field::NET_DEPTH * 0.5,
            1.2,
            Field::PHYSICS_GOAL_HALF_WIDTH + 0.2,
        );
        assert!(
            panel.absorb(Some(just_in), Actors::BALL_RADIUS, 1.0 / 60.0),
            "a ball 20 cm into the side netting is in it"
        );
        let (mut panel, _) = Netting::side_panel();
        let far_out = Vec3::new(
            Field::HALF_LENGTH + Field::NET_DEPTH * 0.5,
            1.2,
            Field::PHYSICS_GOAL_HALF_WIDTH + 3.0,
        );
        assert!(
            !panel.absorb(Some(far_out), Actors::BALL_RADIUS, 1.0 / 60.0),
            "3 m past the post is not a contact — the netting cannot stretch that far"
        );
    }

    /// The volume test has to keep saying yes to the balls that DO belong
    /// in the net, including the ones the engine has settled inside the
    /// mesh, or the fix above trades one stationary net for another.
    #[test]
    fn a_ball_in_the_goal_is_still_in_the_goal() {
        // Crossing the line dead centre, under the bar.
        assert!(Netting::inside_a_goal(Vec3::new(
            Field::HALF_LENGTH + 0.1,
            1.5,
            0.0
        )));
        // Bagged in the back netting, where the engine settles it.
        assert!(Netting::inside_a_goal(Vec3::new(
            Field::HALF_LENGTH + Field::NET_DEPTH + 0.5,
            0.4,
            0.0
        )));
        // Wedged against the inside of a post. The lateral bound is the
        // POST, not the side netting's give — see `inside_a_goal`: the
        // give is measured inward from the panel, and adding it to the
        // half-width put a 50 cm band OUTSIDE each post inside the goal,
        // which is the band every ball that misses now runs through.
        assert!(Netting::inside_a_goal(Vec3::new(
            Field::HALF_LENGTH + 0.6,
            0.2,
            Field::PHYSICS_GOAL_HALF_WIDTH - 0.05
        )));
        assert!(
            !Netting::inside_a_goal(Vec3::new(
                Field::HALF_LENGTH + 0.6,
                0.2,
                Field::PHYSICS_GOAL_HALF_WIDTH + 0.4
            )),
            "40 cm outside the post is beside the goal, not in it"
        );
        // …and no to the places a ball actually spends the match. The
        // corner flag is the one that matters: it is a quarter of a metre
        // in front of the goal line, which is inside the side panel's own
        // depth, and thirty metres outside the post.
        assert!(!Netting::inside_a_goal(Vec3::new(0.0, 0.2, 0.0)));
        assert!(!Netting::inside_a_goal(Vec3::new(
            Field::HALF_LENGTH - 0.25,
            0.11,
            Field::HALF_WIDTH - 0.25
        )));
        // Over the bar, on its way out of the ground.
        assert!(!Netting::inside_a_goal(Vec3::new(
            Field::HALF_LENGTH + 0.5,
            3.4,
            0.0
        )));
    }

    /// **The lid slopes, because the roof netting does.**
    ///
    /// A goal's roof net runs from the 2.44 m crossbar down to
    /// [`Field::NET_BACK_HEIGHT`], and the volume test used to hold the
    /// crossbar's height across the whole 2.9 m of depth. Nothing was ever
    /// up there to notice until a miss started running on behind the goal
    /// (`core::RunOff`): now every ball over the bar flies the length of
    /// the goal at two and a half metres or more, and a flat lid ripples
    /// the netting for each one as though it had gone in.
    #[test]
    fn a_ball_over_the_goal_never_reaches_the_netting() {
        // Just under the bar at the line is in; the same height at the
        // back bar is over the roof net, which has sloped away beneath it.
        assert!(Netting::inside_a_goal(Vec3::new(
            Field::HALF_LENGTH + 0.1,
            2.3,
            0.0
        )));
        assert!(
            !Netting::inside_a_goal(Vec3::new(Field::HALF_LENGTH + Field::NET_DEPTH, 2.3, 0.0)),
            "the roof net is {:.2} m up at the back bar — a ball at 2.30 m \
             is over the top of it",
            Field::NET_BACK_HEIGHT
        );
        // The whole flight of a ball skied over the bar, from the line to
        // the back of the netting. None of it is in the goal.
        let mut depth = 0.05;
        while depth < Field::NET_DEPTH + Netting::GIVE_BACK {
            assert!(
                !Netting::inside_a_goal(Vec3::new(Field::HALF_LENGTH + depth, 2.6, 0.0)),
                "a ball 2.60 m up — above the 2.44 m bar — registered inside \
                 the goal {depth:.2} m past the line"
            );
            depth += 0.05;
        }
    }

    /// The netting has to be netting. An untextured sheet is why the net
    /// read as stationary in the first place: there is nothing on a
    /// featureless surface for the eye to track.
    #[test]
    fn the_netting_texture_is_cords_and_holes() {
        let mut images = Assets::<Image>::default();
        let handle = Textures::netting(&mut images);
        let image = images.get(&handle).expect("texture was added");
        let data = image.data.as_ref().expect("texture has pixels");
        // A repeating net texture MUST carry a mip chain — see
        // `Textures::mipped_netting`. Without one the cords sample to dotted
        // lines at any distance, which was the state the net was screenshotted
        // in.
        let levels = image.texture_descriptor.mip_level_count;
        assert!(
            levels > 1,
            "the netting has to be pre-filtered, got {levels} mip level(s)"
        );
        let size = image.texture_descriptor.size.width as usize;
        assert_eq!(
            1 << (levels - 1),
            size,
            "the chain has to halve all the way to 1x1 from {size}"
        );
        // Level 0 only: the mips are averages and would dilute the counts.
        let alphas: Vec<u8> = data[..size * size * 4].chunks(4).map(|px| px[3]).collect();
        // A cord at full strength, not a uniform wash. The ceiling is
        // `OPACITY`, deliberately short of 1 so the ball stays visible
        // through the back of the net.
        let cord = alphas.iter().filter(|a| **a > 150).count();
        let hole = alphas.iter().filter(|a| **a < 40).count();
        assert!(
            cord > 0 && hole > 0,
            "a net is cord AND hole: {cord} cord texels, {hole} clear ones out of {}",
            alphas.len()
        );
        // Mostly hole, or it is a grille rather than netting — and at range
        // the whole panel would integrate to a solid sheet. But not so sparse
        // that the mip-flattened haze is invisible, which is the state the
        // untextured sheet was in: the density the eye gets at broadcast
        // distance is this coverage times the cord's own opacity.
        let covered = cord as f32 / alphas.len() as f32;
        assert!(
            (0.15..0.35).contains(&covered),
            "cord should cover about a quarter of the panel, covers {:.0}%",
            covered * 100.0
        );
    }
}
