use crate::actors::BallState;
use crate::field::Field;
use crate::playback::Playback;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use std::f32::consts::{PI, TAU};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{AddEventListenerOptions, MouseEvent, WheelEvent};

/// How far the lens is pulled in, as a multiple of the default framing.
///
/// Above 1 is tighter, below 1 is wider. Driven by the mouse wheel; the
/// camera turns it into a field of view every frame, so nothing else in the
/// rig has to know about it.
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
    /// One notch of the wheel. Geometric rather than additive, so a turn
    /// back out undoes a turn in exactly and the control feels the same at
    /// both ends of its range.
    const STEP: f32 = 1.10;
    const RANGE: (f32, f32) = (0.45, 3.0);

    /// How much of a notch one pixel of a smooth-scrolling wheel is worth.
    /// Trackpads and high-resolution wheels report pixels rather than
    /// notches, and a pixel per notch would make them uncontrollable.
    const PIXELS_PER_NOTCH: f32 = 50.0;

    /// Zoom by a fractional number of notches, so a wheel that reports
    /// pixels moves the lens as smoothly as it is turned.
    fn turn(&mut self, notches: f32) {
        self.factor = (self.factor * Self::STEP.powf(notches)).clamp(Self::RANGE.0, Self::RANGE.1);
    }

    /// The wheel drives the lens.
    ///
    /// Scrolling up pulls the play closer — the direction every map and
    /// every 3D tool uses, and the one that matches the chips this
    /// replaced. Both report units are handled: a notched wheel sends
    /// lines, a trackpad sends pixels.
    pub fn handle_wheel(mut wheel: MessageReader<MouseWheel>, mut zoom: ResMut<CameraZoom>) {
        let notches: f32 = wheel
            .read()
            .map(|scroll| match scroll.unit {
                MouseScrollUnit::Line => scroll.y,
                MouseScrollUnit::Pixel => scroll.y / Self::PIXELS_PER_NOTCH,
            })
            .sum();
        if notches != 0.0 {
            zoom.turn(notches);
        }
    }
}

/// Where the operator has walked the rig to, held as a pair of angles rather
/// than as a place: a bearing round the centre spot and a height above the
/// turf, both measured from the gantry the shot starts on.
///
/// The distance from the centre spot does not change with them. With the lens
/// fixed, distance is the only thing that decides how large a footballer comes
/// out, so holding it is what makes the move read as one camera walking round
/// the ground rather than as a cut between several.
#[derive(Resource)]
pub struct CameraOrbit {
    /// Rotation about the centre spot, zero at the gantry, kept in (−π, π].
    pub bearing: f32,
    /// Angle up from the turf to the rig, measured at the centre spot.
    pub elevation: f32,
}

impl Default for CameraOrbit {
    fn default() -> Self {
        CameraOrbit {
            bearing: 0.0,
            elevation: TvCamera::rest_elevation(),
        }
    }
}

impl CameraOrbit {
    /// How far a pixel of drag turns the rig. A sweep across a full-width
    /// canvas comes to most of a turn, so the far side of the ground is one
    /// gesture away rather than several.
    const RADIANS_PER_PIXEL: f32 = 0.006;
    /// How low and how high the rig can be walked. The floor is about head
    /// height out by the hoardings; the ceiling stops short of straight down,
    /// where a camera looking along its own up-vector has no way left to
    /// decide which way up the frame goes.
    const ELEVATION: (f32, f32) = (0.035, 1.40);
    /// How near the gantry a drag has to finish for the rig to settle back
    /// onto it exactly. Five degrees — too small to see it click, wide enough
    /// to find by hand, and the only way back to the broadcast shot once the
    /// rig has been walked off it.
    const DETENT: f32 = 0.09;

    /// Right button or wheel button held, and the ground turns under the
    /// pointer.
    pub fn handle_drag(
        mouse: Res<ButtonInput<MouseButton>>,
        mut moved: MessageReader<CursorMoved>,
        mut orbit: ResMut<CameraOrbit>,
    ) {
        // Read every frame, dragging or not. A reader left alone keeps the
        // last couple of frames of motion, which would otherwise all arrive
        // as one jump the moment a button went down.
        let drag = moved.read().fold(Vec2::ZERO, |total, event| {
            total + event.delta.unwrap_or(Vec2::ZERO)
        });

        let held = mouse.pressed(MouseButton::Right) || mouse.pressed(MouseButton::Middle);
        if held && drag != Vec2::ZERO {
            // The ground follows the pointer rather than the lens: drag right
            // and the pitch turns right, which means the rig itself travels
            // the other way round. That is the gesture of spinning a model on
            // a turntable, and it is what a drag with the cursor still on
            // screen reads as — the opposite convention belongs to a
            // mouselook, which needs the pointer out of the way first.
            orbit.bearing = Self::wrap(orbit.bearing + drag.x * Self::RADIANS_PER_PIXEL);
            // Pulling the near side of the ground down toward you lifts the
            // rig over it, which is the same way round.
            orbit.elevation = (orbit.elevation + drag.y * Self::RADIANS_PER_PIXEL)
                .clamp(Self::ELEVATION.0, Self::ELEVATION.1);
        }

        let let_go =
            mouse.just_released(MouseButton::Right) || mouse.just_released(MouseButton::Middle);
        if let_go
            && orbit.bearing.abs() < Self::DETENT
            && (orbit.elevation - TvCamera::rest_elevation()).abs() < Self::DETENT
        {
            *orbit = CameraOrbit::default();
        }
    }

    /// Keeps the bearing in (−π, π], so the detent has one home rather than
    /// one per lap.
    fn wrap(angle: f32) -> f32 {
        (angle + PI).rem_euclid(TAU) - PI
    }

    /// Takes the right button, the wheel button and the wheel itself off
    /// the page.
    ///
    /// The browser answers a right-press with a context menu, a
    /// wheel-press with autoscroll, and a wheel turn by scrolling the
    /// document — the first two would land on top of the drag, and the
    /// third would walk the page down while the lens zoomed. Winit will
    /// suppress all three, but only through the same switch that swallows
    /// the keyboard — and the page keeps the keyboard, F5 included (see
    /// [`crate::MatchViewer::start`]) — so the listeners go on by hand
    /// instead.
    pub fn claim_pointer_buttons(canvas: &str) {
        let Some(element) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.query_selector(canvas).ok().flatten())
        else {
            web_sys::console::warn_1(&format!("match viewer: no canvas at {canvas}").into());
            return;
        };

        let menu = Closure::<dyn FnMut(MouseEvent)>::new(|event: MouseEvent| {
            event.prevent_default();
        });
        let press = Closure::<dyn FnMut(MouseEvent)>::new(|event: MouseEvent| {
            // 1 is the wheel, 2 the right button. The left one is left alone:
            // it works the transport bar, and taking its default away would
            // take the canvas's focus with it.
            if event.button() == 1 || event.button() == 2 {
                event.prevent_default();
            }
        });

        // The wheel drives the zoom, so the document must not move with it.
        // Registered non-passive — a passive listener is not allowed to
        // call `prevent_default`, and browsers default wheel listeners on
        // scrollable elements to passive.
        let wheel = Closure::<dyn FnMut(WheelEvent)>::new(|event: WheelEvent| {
            event.prevent_default();
        });
        let active = AddEventListenerOptions::new();
        active.set_passive(false);
        if element
            .add_event_listener_with_callback_and_add_event_listener_options(
                "wheel",
                wheel.as_ref().unchecked_ref(),
                &active,
            )
            .is_err()
        {
            web_sys::console::warn_1(&"match viewer: wheel refused".into());
        }

        for (name, listener) in [("contextmenu", &menu), ("mousedown", &press)] {
            if element
                .add_event_listener_with_callback(name, listener.as_ref().unchecked_ref())
                .is_err()
            {
                web_sys::console::warn_1(&format!("match viewer: {name} refused").into());
            }
        }

        // All three live as long as the page does. There is nothing to hang
        // them on and nothing to reclaim them at teardown, which is what
        // `forget` says out loud.
        menu.forget();
        press.forget();
        wheel.forget();
    }
}

/// The broadcast rig: one long-lens camera high on the halfway-line stand,
/// panning to keep the ball framed. Left alone it never leaves its gantry —
/// only the pan, tilt and a little lateral travel change — which is what makes
/// footage read as televised football rather than as a game camera. A drag on
/// the right or wheel button walks it round the ground from there; everything
/// below is written in the gantry's terms and turns with it.
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

    /// How far out from the centre spot the gantry stands, along the ground.
    fn rest_reach() -> f32 {
        Field::HALF_WIDTH + Self::SETBACK
    }

    /// The angle up from the centre spot to the gantry: where the orbit
    /// starts, and where its detent puts the rig back.
    fn rest_elevation() -> f32 {
        Self::HEIGHT.atan2(Self::rest_reach())
    }

    /// Centre spot to lens. Held constant as the rig orbits, so a shot from
    /// behind a goal draws a footballer exactly as large as the one from the
    /// main stand.
    fn rest_distance() -> f32 {
        Self::HEIGHT.hypot(Self::rest_reach())
    }

    pub fn spawn(mut commands: Commands) {
        let sideline = -Self::rest_reach();
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
        orbit: Res<CameraOrbit>,
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

        // Where the gantry stands for this bearing and height. At rest the
        // three numbers below come back out as the constants they were
        // written as: `(0, HEIGHT, -(HALF_WIDTH + SETBACK))`.
        let distance = Self::rest_distance();
        let reach = distance * orbit.elevation.cos();
        let gantry = Vec3::new(
            orbit.bearing.sin() * reach,
            distance * orbit.elevation.sin(),
            -orbit.bearing.cos() * reach,
        );
        // The rail the rig slides along and the axis it looks down, both
        // turned with it. From the gantry they are the pitch's own length and
        // width, which is the sense in which the framing constants above are
        // written; from behind a goal they have swapped over.
        let rail = Vec3::new(orbit.bearing.cos(), 0.0, orbit.bearing.sin());
        let depth = Vec3::new(-orbit.bearing.sin(), 0.0, orbit.bearing.cos());

        // How much pitch there is to run along: the full half-length from the
        // main stand, the half-width from behind a goal, and the two mixed
        // anywhere in between.
        let along = rail.x.abs() * Field::HALF_LENGTH + rail.z.abs() * Field::HALF_WIDTH;
        let limit = along * Self::TRAVEL_LIMIT;
        let travel = (rig.focus.dot(rail) * Self::TRAVEL).clamp(-limit, limit);

        transform.translation = gantry + rail * travel;
        transform.look_at(
            rail * rig.focus.dot(rail)
                + depth * (rig.focus.dot(depth) * Self::AIM_ACROSS + Self::AIM_NEAR_BIAS),
            Vec3::Y,
        );
    }
}
