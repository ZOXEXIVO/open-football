//! What the ground stands under.
//!
//! There was nothing here before: past the back row of a stand the frame simply
//! ran out into `ClearColor`, a near-black that was defensible on its own terms
//! — a floodlit ground at night is a pale structure against a dark sky — but
//! read as a hole rather than as air. The banks deliberately have no back wall,
//! on the reasoning that "the rows now finish against the sky"; this is the sky
//! they were meant to finish against.
//!
//! It is a dome rather than a cube map: the whole thing is one vertical
//! gradient, and a gradient is a texture 4 texels wide, not six faces that have
//! to be generated, packed and kept in agreement at the corners.

use crate::textures::Textures;
use bevy::prelude::*;
use bevy::render::mesh::Mesh;
use std::f32::consts::FRAC_PI_2;

#[derive(Component)]
pub struct Sky;

impl Sky {
    /// How far off the lens the dome sits.
    ///
    /// It is not a distance to anything — the dome is carried by the camera, so
    /// this only has to clear the scene and stay inside the far plane. The
    /// stands are at most ~110 m from a lens that may itself be 200 m out, and
    /// the projection's far plane is 1000 m; 400 sits comfortably between.
    const RADIUS: f32 = 400.0;

    pub fn spawn(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
    ) {
        let gradient = Textures::sky(&mut images);
        let shell = materials.add(StandardMaterial {
            base_color_texture: Some(gradient),
            // Sky is not lit, it IS the light — running it through the
            // directional and the ambient would tint it with the floodlights
            // and put a shading terminator across the top of the frame.
            unlit: true,
            // And it is not in the haze either. The falloff ends at 215 m, so
            // a dome at 400 would be fogged to a flat wall of exactly the haze
            // colour — which would take the gradient back out again.
            fog_enabled: false,
            // The lens is inside the sphere, so what faces it is the back of
            // every triangle and the default cull would discard the lot.
            cull_mode: None,
            ..default()
        });

        commands.spawn((
            Sky,
            Mesh3d(meshes.add(Sphere::new(Self::RADIUS).mesh().uv(32, 24))),
            MeshMaterial3d(shell),
            // Bevy's UV sphere is built around the z axis — v runs from the
            // +Z pole to the −Z one — and the gradient is written zenith-first,
            // so the mesh is stood upright here rather than the texture being
            // authored sideways to compensate.
            Transform::from_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
        ));
    }

    /// Keeps the dome centred on the lens.
    ///
    /// A sky is at infinity: it may turn as the camera turns, but it must not
    /// slide as the camera moves, or flying down the touchline drags the
    /// horizon along like a painted backdrop on rails. Carrying it means the
    /// horizon also stays at eye level, which is where a horizon is.
    pub fn follow_lens(
        lens: Single<&Transform, (With<Camera3d>, Without<Sky>)>,
        mut dome: Single<&mut Transform, (With<Sky>, Without<Camera3d>)>,
    ) {
        // Only on a change. The dome is a 1,536-triangle shell and the write
        // is cheap, but the write is not what costs: `Transform` is
        // change-detected, so an unconditional one puts the sky through
        // transform propagation and re-uploads its uniform on every frame of
        // a replay watched from a camera that has not moved since kickoff.
        if dome.translation != lens.translation {
            dome.translation = lens.translation;
        }
    }
}
