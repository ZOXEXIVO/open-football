use crate::actors::BallState;
use crate::field::Field;
use crate::playback::Playback;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;

/// The broadcast rig: one long-lens camera high on the halfway-line stand,
/// panning to keep the ball framed. It never leaves its gantry — only the pan,
/// tilt and a little lateral travel change — which is what makes footage read
/// as televised football rather than as a game camera.
#[derive(Component)]
pub struct TvCamera {
    /// Smoothed point of interest, in world space.
    focus: Vec3,
}

impl TvCamera {
    /// Height of the gantry above the turf.
    const HEIGHT: f32 = 34.0;
    /// How far back from the touchline the gantry sits. Together with the
    /// height this is what decides how much of the pitch the frame can hold:
    /// from here the near and far touchlines are ~23° apart, which the field of
    /// view below covers with room to spare.
    const SETBACK: f32 = 40.0;
    /// Fraction of the ball's travel along the pitch the rig itself tracks. A
    /// real main camera slides only a little; the rest is pan.
    const TRAVEL: f32 = 0.65;
    /// Vertical field of view. Long-lens rather than wide: a wide angle from
    /// this distance would make the pitch look like a table.
    const FOV: f32 = 0.50;
    /// How far the aim point follows the ball across the pitch, and how far it
    /// sits toward the near touchline. Chasing the ball's own width would swing
    /// the near third out of frame every time play went long.
    const AIM_ACROSS: f32 = 0.20;
    /// Pulls the aim point toward the near touchline. Aiming at the middle of
    /// the pitch tilts the rig up just far enough to push the near touchline
    /// off the bottom of the frame, which loses whoever is hugging it.
    const AIM_NEAR_BIAS: f32 = -9.0;
    /// Seconds for the framing to catch up to a ball that jumps across the
    /// pitch. Slow enough to look operated, fast enough not to lose the play.
    const RESPONSE: f32 = 0.42;

    pub fn spawn(mut commands: Commands) {
        let sideline = -(Field::HALF_WIDTH + Self::SETBACK);
        commands.spawn((
            TvCamera { focus: Vec3::ZERO },
            Camera3d::default(),
            Projection::from(PerspectiveProjection {
                fov: Self::FOV,
                ..default()
            }),
            AmbientLight {
                color: Color::srgb(0.80, 0.87, 1.0),
                brightness: 140.0,
                ..default()
            },
            // Haze over the far end of the ground. Without it the turf simply
            // stops at the edge of the surround and the pitch reads as a
            // floating rectangle; with it the far side falls away into the
            // background the way a long lens across a stadium does.
            DistanceFog {
                color: Color::srgb(0.05, 0.07, 0.10),
                falloff: FogFalloff::Linear {
                    start: 70.0,
                    end: 165.0,
                },
                ..default()
            },
            Transform::from_xyz(0.0, Self::HEIGHT, sideline).looking_at(Vec3::ZERO, Vec3::Y),
        ));
    }

    pub fn follow_play(
        ball: Res<BallState>,
        playback: Res<Playback>,
        time: Res<Time>,
        mut camera: Single<(&mut TvCamera, &mut Transform)>,
    ) {
        let (rig, transform) = &mut *camera;

        let target = if ball.on_pitch {
            Vec3::new(ball.position.x, 0.0, ball.position.z)
        } else {
            Vec3::ZERO
        };

        if playback.seeked {
            rig.focus = target;
        } else {
            // Exponential catch-up, framerate independent.
            let blend = 1.0 - (-time.delta_secs() / Self::RESPONSE).exp();
            rig.focus = rig.focus.lerp(target, blend.clamp(0.0, 1.0));
        }

        let travel =
            (rig.focus.x * Self::TRAVEL).clamp(-Field::HALF_LENGTH * 0.6, Field::HALF_LENGTH * 0.6);
        transform.translation =
            Vec3::new(travel, Self::HEIGHT, -(Field::HALF_WIDTH + Self::SETBACK));
        transform.look_at(
            Vec3::new(
                rig.focus.x,
                0.0,
                rig.focus.z * Self::AIM_ACROSS + Self::AIM_NEAR_BIAS,
            ),
            Vec3::Y,
        );
    }
}
