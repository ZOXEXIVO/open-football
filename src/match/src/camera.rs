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
    ///
    /// Height and setback together decide nothing about how large the players
    /// come out, which is worth stating because it is the opposite of the
    /// intuition. Holding both touchlines in frame forces the field of view to
    /// cover the angle between them, and that angle *widens* as the rig comes
    /// in — by exactly as much as the shorter distance magnifies. Every
    /// (height, setback) pair that frames the full width draws a footballer the
    /// same size. Getting closer therefore means giving up some of the pitch,
    /// which is what the numbers below do: they frame the play rather than the
    /// ground, and the aim tracks the ball across the width to keep the part
    /// that matters inside the cut.
    const HEIGHT: f32 = 27.0;
    /// How far back from the touchline the gantry sits.
    const SETBACK: f32 = 29.0;
    /// Fraction of the ball's travel along the pitch the rig itself tracks. A
    /// real main camera slides only a little; the rest is pan. On a lens this
    /// long the slide has to do more of the work, or a break down the wing
    /// leaves the frame before the pan catches it.
    const TRAVEL: f32 = 0.80;
    /// How far along the pitch the rig is allowed to run, as a fraction of the
    /// half-length.
    const TRAVEL_LIMIT: f32 = 0.70;
    /// Vertical field of view. Long-lens rather than wide: this is the number
    /// that decides how close the footage feels, and 0.40 rad holds a little
    /// over half the pitch length across a 16:9 frame — about what a broadcast
    /// main camera carries during open play.
    const FOV: f32 = 0.40;
    /// How far the aim point follows the ball across the pitch. The frame no
    /// longer spans both touchlines at once, so this has to be high enough that
    /// whichever touchline gets cut is always the one play is furthest from —
    /// a winger on the near touchline has to be in shot when the ball is with
    /// him, and is expendable when it is on the far side.
    const AIM_ACROSS: f32 = 0.65;
    /// Pulls the aim point toward the near touchline. Aiming at the middle of
    /// the pitch tilts the rig up far enough to push the near third off the
    /// bottom of the frame, which loses whoever is hugging it — and on this
    /// lens there is no slack left to absorb that.
    const AIM_NEAR_BIAS: f32 = -12.0;
    /// Seconds for the framing to catch up to a ball that jumps across the
    /// pitch. Slow enough to look operated, fast enough not to lose the play;
    /// a tighter frame needs a quicker operator.
    const RESPONSE: f32 = 0.34;

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

        let limit = Field::HALF_LENGTH * Self::TRAVEL_LIMIT;
        let travel = (rig.focus.x * Self::TRAVEL).clamp(-limit, limit);
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
