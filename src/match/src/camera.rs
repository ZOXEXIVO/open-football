use crate::actors::BallState;
use crate::field::Field;
use crate::playback::Playback;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;

/// How far the lens is pulled in, as a multiple of the default framing.
///
/// Above 1 is tighter, below 1 is wider. Driven by the two chips on the
/// transport bar; the camera turns it into a field of view every frame, so
/// nothing else in the rig has to know about it.
#[derive(Resource)]
pub struct CameraZoom {
    pub factor: f32,
}

impl Default for CameraZoom {
    fn default() -> Self {
        CameraZoom { factor: 1.0 }
    }
}

impl CameraZoom {
    /// One press of a chip. Geometric rather than additive, so a step out
    /// undoes a step in exactly and the control feels the same at both ends
    /// of its range.
    const STEP: f32 = 1.10;
    const RANGE: (f32, f32) = (0.45, 3.0);

    pub fn step(&mut self, direction: i32) {
        let scale = if direction > 0 {
            Self::STEP
        } else {
            1.0 / Self::STEP
        };
        self.factor = (self.factor * scale).clamp(Self::RANGE.0, Self::RANGE.1);
    }
}

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
    /// Barely above the players' own eyeline, which is what makes the pitch
    /// read as ground they are standing on rather than a plan of one.
    ///
    /// Dropping this far changes the framing problem rather than tightening it.
    /// Seen from up on a gantry the two touchlines are a wide angle apart and
    /// the lens has to spend itself covering them; seen from near pitch level
    /// the whole width collapses into a thin band — here about 0.17 rad against
    /// a 0.34 rad frame — so the full width comes back into shot for free and
    /// the aim barely has to track across it any more. What the height buys
    /// instead is depth: a near player is drawn nearly three times the size of
    /// a far one, which is the compression that reads as televised football.
    const HEIGHT: f32 = 18.0;
    /// How far back from the touchline the gantry sits.
    ///
    /// Height and setback pull the pitch band in opposite directions and the
    /// setback wins: backing off compresses the two touchlines together faster
    /// than the extra height spreads them apart. Both moved here, and the band
    /// came out at 0.205 rad — near enough unchanged, which is why the lens
    /// did not have to move with them.
    const SETBACK: f32 = 48.0;
    /// Fraction of the ball's travel along the pitch the rig itself tracks. A
    /// real main camera slides only a little; the rest is pan. On a lens this
    /// long the slide has to do more of the work, or a break down the wing
    /// leaves the frame before the pan catches it.
    const TRAVEL: f32 = 0.80;
    /// How far along the pitch the rig is allowed to run, as a fraction of the
    /// half-length.
    const TRAVEL_LIMIT: f32 = 0.70;
    /// Vertical field of view. Long-lens rather than wide: this is the number
    /// that decides how close the footage feels.
    ///
    /// It is also what stops a low rig wasting its frame. The pitch only
    /// subtends ~0.21 rad from down here, so a wider lens spends the rest of
    /// the shot on the turf behind the near hoarding and on the sky above the
    /// far stand. At 0.28 the playing surface fills about three quarters of the
    /// frame and the background takes the remaining quarter, which is where a
    /// televised angle puts it.
    ///
    /// Widened 1.5× (0.28 → 0.42) on request: everything is drawn two thirds
    /// the size and correspondingly more of the ground is in shot. Worth
    /// knowing what the extra frame is spent on — by the note above, the pitch
    /// subtends only ~0.21 rad from a rig this low, so where 0.28 put about
    /// three quarters of the frame on grass, 0.42 puts about half and the rest
    /// goes to stand and sky. If the extra width should land on turf instead,
    /// `HEIGHT` is the knob: lifting the gantry spreads the two touchlines
    /// apart so the wider lens has more pitch to cover.
    ///
    /// Then pulled 10% back in (0.42 → 0.382): the 1.5× was a shade far, and
    /// this is the fine adjustment on top of it. Net against the original
    /// 0.28 it is 1.36× wider.
    const FOV: f32 = 0.382;
    /// How far the aim point follows the ball across the pitch. Low down the
    /// whole width is in shot anyway, so this is back to a gentle lead rather
    /// than a chase — and it has to stay gentle, because tilting up from here
    /// runs the top of the frame past the horizon.
    const AIM_ACROSS: f32 = 0.30;
    /// Pulls the aim point toward the near touchline. Set so the near touchline
    /// lands on the bottom edge of the frame — the waste has to go somewhere,
    /// and a quarter-frame of stand above the far touchline is atmosphere,
    /// where the same quarter-frame of empty turf below the near one is not.
    const AIM_NEAR_BIAS: f32 = -1.0;
    /// Seconds for the framing to catch up to a ball that jumps across the
    /// pitch. Slow enough to look operated, fast enough not to lose the play;
    /// a tighter frame needs a quicker operator.
    const RESPONSE: f32 = 0.30;

    pub fn spawn(mut commands: Commands) {
        let sideline = -(Field::HALF_WIDTH + Self::SETBACK);
        commands.spawn((
            TvCamera { focus: Vec3::ZERO },
            Camera3d::default(),
            Projection::from(PerspectiveProjection {
                fov: Self::FOV,
                ..default()
            }),
            // Carries the fill for the whole scene — see the note on the
            // directional light in `Pitch::spawn`.
            AmbientLight {
                color: Color::srgb(0.84, 0.89, 1.0),
                brightness: 900.0,
                ..default()
            },
            // Haze over the far end of the ground. Without it the turf simply
            // stops at the edge of the surround and the pitch reads as a
            // floating rectangle; with it the far side falls away into the
            // background the way a long lens across a stadium does.
            // Pushed out from where a gantry wanted it: down here every player
            // is further from the lens, and the far side was fading into the
            // haze along with the background it was meant to separate from.
            // Haze colour has to move with the stands. Distant geometry tends
            // toward it, so against the old near-black a pale stand simply
            // faded to black at range and all the colour put into it was
            // thrown away over the far side of the ground. A mid grey-blue
            // reads as floodlit air — and is still dark enough that the far
            // half of the turf does not wash out.
            //
            // The SKY stays near-black (`ClearColor` in `lib.rs`), which is
            // correct rather than inconsistent: a floodlit ground at night is
            // exactly a pale structure against a black sky.
            DistanceFog {
                color: Color::srgb(0.200, 0.230, 0.280),
                falloff: FogFalloff::Linear {
                    start: 100.0,
                    end: 215.0,
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
        zoom: Res<CameraZoom>,
        mut camera: Single<(&mut TvCamera, &mut Transform, &mut Projection)>,
    ) {
        let (rig, transform, projection) = &mut *camera;

        // The lens. `FOV` is the framing at 1.0; zooming in narrows it.
        if let Projection::Perspective(perspective) = projection.as_mut() {
            let wanted = Self::FOV / zoom.factor.max(0.01);
            if (perspective.fov - wanted).abs() > 1e-4 {
                perspective.fov = wanted;
            }
        }

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
