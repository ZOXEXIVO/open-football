use crate::actors::BallState;
use crate::changeover::ChangeoverShot;
use crate::config::ViewerConfig;
use crate::field::Field;
use crate::focus::CameraSubject;
use crate::playback::Playback;
use crate::quality::Quality;
use crate::stage::Stage;
use crate::touch::TouchDrive;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{
    AddEventListenerOptions, Element, FocusOptions, HtmlElement, KeyboardEvent, MouseEvent,
    WheelEvent,
};

/// The canvas the page handed the viewer, found by the CSS selector it named
/// in [`ViewerConfig`].
///
/// Three things below reach for it — the pointer claims, the key claims and
/// the focus call — and all three have to survive not finding it: a replay
/// with no camera controls is still a replay, where a panic here is a blank
/// page.
struct ViewerCanvas;

impl ViewerCanvas {
    fn find(selector: &str) -> Option<Element> {
        let found = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.query_selector(selector).ok().flatten());
        if found.is_none() {
            web_sys::console::warn_1(&format!("match viewer: no canvas at {selector}").into());
        }
        found
    }
}

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
        self.scale(Self::STEP.powf(notches));
    }

    /// Multiply the lens by a ratio.
    ///
    /// What a PINCH hands over: the fingers were this far apart and are now
    /// that far apart, and the lens should have changed by the same
    /// proportion. The wheel goes through here too — a notch is a ratio of
    /// [`Self::STEP`] — which is why a pinch and a wheel turn cannot drift
    /// apart, and why either can undo the other exactly.
    pub fn scale(&mut self, ratio: f32) {
        if !ratio.is_finite() || ratio <= 0.0 {
            return;
        }
        self.factor = (self.factor * ratio).clamp(Self::RANGE.0, Self::RANGE.1);
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
    ///
    /// The gesture itself lives in [`Self::drag`], because a finger performs
    /// exactly the same one — see [`crate::touch`]. All this system decides is
    /// when a mouse is doing it: a bare left button belongs to the transport
    /// bar, so the camera takes the two buttons the bar has no use for.
    pub fn handle_drag(
        mouse: Res<ButtonInput<MouseButton>>,
        mut moved: MessageReader<CursorMoved>,
        mut orbit: ResMut<CameraOrbit>,
        mut flight: ResMut<CameraFlight>,
    ) {
        // Read every frame, dragging or not. A reader left alone keeps the
        // last couple of frames of motion, which would otherwise all arrive
        // as one jump the moment a button went down.
        let drag = moved.read().fold(Vec2::ZERO, |total, event| {
            total + event.delta.unwrap_or(Vec2::ZERO)
        });

        if mouse.pressed(MouseButton::Right) || mouse.pressed(MouseButton::Middle) {
            orbit.drag(drag, &mut flight);
        }

        if mouse.just_released(MouseButton::Right) || mouse.just_released(MouseButton::Middle) {
            orbit.settle(&flight);
        }
    }

    /// One frame's worth of drag, whatever was doing the dragging.
    ///
    /// The same grip aims the free camera once the rig is airborne — see
    /// [`Airborne::aim`]. One gesture, two frames of reference: from the
    /// gantry it swings the ground round the centre spot, in flight it turns
    /// the head.
    ///
    /// Shared by the right-button drag above and the one-finger drag in
    /// [`crate::touch`], on purpose and not merely for the saving: the two are
    /// the same control, and the same sweep across the canvas has to leave the
    /// rig in the same place whether it was made with a mouse or a thumb. A
    /// second copy of these three lines is a second set of numbers waiting to
    /// drift.
    pub fn drag(&mut self, drag: Vec2, flight: &mut CameraFlight) {
        if drag == Vec2::ZERO {
            return;
        }
        if let Some(airborne) = flight.airborne.as_mut() {
            airborne.aim(drag);
            return;
        }
        // The ground follows the pointer rather than the lens: drag right and
        // the pitch turns right, which means the rig itself travels the other
        // way round. That is the gesture of spinning a model on a turntable,
        // and it is what a drag with the cursor still on screen reads as — the
        // opposite convention belongs to a mouselook, which needs the pointer
        // out of the way first.
        self.bearing = Self::wrap(self.bearing + drag.x * Self::RADIANS_PER_PIXEL);
        // Pulling the near side of the ground down toward you lifts the rig
        // over it, which is the same way round.
        self.elevation = (self.elevation + drag.y * Self::RADIANS_PER_PIXEL)
            .clamp(Self::ELEVATION.0, Self::ELEVATION.1);
    }

    /// The grip was let go. If it finished near enough to the gantry, the rig
    /// settles back onto it exactly.
    ///
    /// The detent belongs to the orbit: in flight the rig is nowhere near the
    /// gantry and the way home is the reset button.
    pub fn settle(&mut self, flight: &CameraFlight) {
        if !flight.airborne()
            && self.bearing.abs() < Self::DETENT
            && (self.elevation - TvCamera::rest_elevation()).abs() < Self::DETENT
        {
            *self = CameraOrbit::default();
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
        let Some(element) = ViewerCanvas::find(canvas) else {
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

/// The rig lifted off its orbit and flown by hand.
///
/// A second mode rather than a replacement. Until a movement key is pressed
/// the broadcast rig, the orbit drag and the wheel behave exactly as they
/// always did; the moment one is, the camera stops following the ball and
/// goes where it is pushed. [`CameraRig::reset`] — the button on the transport
/// bar — puts it back on the gantry.
///
/// What flight buys that the orbit cannot is everything at a distance other
/// than the gantry's: the orbit holds `TvCamera::rest_distance` from the
/// centre spot by design, so it can walk round the ground but never into it.
/// A look along the goal line, down onto the six-yard box, or out from the
/// back row of a stand all need the rig off that sphere.
#[derive(Resource, Default)]
pub struct CameraFlight {
    /// `None` while the broadcast rig has the camera. Seeded at the moment
    /// the first movement key goes down from wherever the rig was standing,
    /// so taking off continues the shot instead of cutting to a new one.
    airborne: Option<Airborne>,
}

/// Where the free camera is and which way it faces, held in the terms the
/// controls move it in: a place, a bearing and a tilt. No roll — a camera
/// flown round a stadium has to come back level or the football stops being
/// readable, and there is no gesture here that would ever want it.
struct Airborne {
    position: Vec3,
    yaw: f32,
    pitch: f32,
}

impl Airborne {
    /// Turn the head under a drag: the pointer leads and the camera follows
    /// it. Drag right and the view swings right, drag down and it tilts down
    /// onto the pitch.
    ///
    /// Deliberately NOT the turntable sense [`CameraOrbit::handle_drag`] uses.
    /// From the gantry the drag pushes the ground round a rig that is looking
    /// at it, so grabbing the scene is the honest reading of the gesture; in
    /// flight it turns a head that is standing still, and a head follows where
    /// it is pointed. Built the other way round first and it read as inverted,
    /// which is exactly what it was.
    fn aim(&mut self, drag: Vec2) {
        self.yaw = CameraOrbit::wrap(self.yaw - drag.x * CameraOrbit::RADIANS_PER_PIXEL);
        self.pitch = (self.pitch - drag.y * CameraOrbit::RADIANS_PER_PIXEL)
            .clamp(-CameraFlight::TILT, CameraFlight::TILT);
    }

    /// Which way the camera is pointed. Yaw then pitch, in that order, which
    /// is what leaves the horizon level at every tilt.
    fn facing(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
    }
}

impl CameraFlight {
    /// Metres a second. A stadium is about 120 m end to end, so this crosses
    /// it in five seconds — quick enough to get behind the far goal without
    /// waiting, slow enough to place a shot by hand. Flat: the lens does not
    /// scale it, see the note in [`CameraFlight::steer`].
    const SPEED: f32 = 26.0;
    /// Multiplier while a shift key is held.
    const BOOST: f32 = 3.2;
    /// Floor and ceiling. The floor is knee height rather than zero: below
    /// that the near-clip plane eats the turf and the shot is of nothing.
    const HEIGHT: (f32, f32) = (0.6, 130.0);
    /// How far outside the playing surface the rig may wander before it stops.
    /// Enough to sit behind the back wall of any stand and no further — past
    /// that there is only fog, and a viewer who has flown into it has no
    /// landmark left to fly back by.
    const RANGE: f32 = 150.0;
    /// Tilt stops just short of straight up and straight down, where a camera
    /// looking along its own up-vector has no way left to decide which way up
    /// the frame goes — the same reason the orbit's ceiling stops short.
    const TILT: f32 = FRAC_PI_2 - 0.02;

    pub fn airborne(&self) -> bool {
        self.airborne.is_some()
    }

    /// Back on the gantry, and the broadcast rig picks the shot up again from
    /// wherever the ball is — or from whichever player it has been asked to
    /// follow, which is the other caller: a free camera is written from the
    /// flight state every frame and cannot follow anybody.
    pub fn land(&mut self) {
        self.airborne = None;
    }

    /// WASD or the arrow keys fly the camera; Q and E take it down and up.
    ///
    /// A thumb on the on-screen stick arrives here as well, through
    /// [`TouchDrive`] — the keys and the stick are added together rather than
    /// chosen between, so neither can lock the other out and a laptop with
    /// both answers both.
    ///
    /// Runs after [`TvCamera::follow_play`], so on the frame the rig takes off
    /// it seeds itself from the broadcast position that system has just
    /// written and there is no jump. On every frame after that it simply
    /// overwrites what `follow_play` skipped.
    pub fn steer(
        keys: Res<ButtonInput<KeyCode>>,
        drive: Res<TouchDrive>,
        time: Res<Time>,
        mut flight: ResMut<CameraFlight>,
        mut camera: Single<&mut Transform, With<Camera3d>>,
    ) {
        let push = Self::push(&keys) + drive.push;
        // Nothing pressed and still on the gantry: the broadcast rig owns the
        // transform and this system must not touch it.
        if push == Vec3::ZERO && !flight.airborne() {
            return;
        }

        let airborne = flight.airborne.get_or_insert_with(|| {
            // `look_at` never rolls the camera, so the Y-then-X decomposition
            // recovers the bearing and tilt exactly.
            let (yaw, pitch, _) = camera.rotation.to_euler(EulerRot::YXZ);
            Airborne {
                position: camera.translation,
                yaw,
                pitch,
            }
        });

        let facing = airborne.facing();
        if push != Vec3::ZERO {
            // Bevy cameras look down their own −Z.
            let step = (facing * Vec3::NEG_Z * push.z + facing * Vec3::X * push.x)
                // Up is world up rather than the camera's: Q and E should
                // gain height, not slide along a tilted frame.
                + Vec3::Y * push.y;
            // Shift is the only thing that changes the pace. The wheel is the
            // lens and nothing else — it was briefly scaling this too, on the
            // reasoning that a tight lens magnifies its own motion, and the
            // result was one control quietly doing two jobs. The stick's own
            // shift is its rim: pushed that far out it asks for the same
            // multiplier, which is what keeps a thumb from being permanently
            // slower than a hand on a keyboard.
            let boost =
                if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) || drive.boost {
                    Self::BOOST
                } else {
                    1.0
                };
            // Clamped rather than normalised, which is what lets an ANALOGUE
            // control through. A key is on or off, so its push is a unit
            // vector and the two are the same thing — a diagonal comes out at
            // one either way. A stick held half over is a push of half the
            // length, and normalising it would have thrown that away and flown
            // at full pelt from the first millimetre.
            airborne.position +=
                step.clamp_length_max(1.0) * Self::SPEED * boost * time.delta_secs();

            let along = Field::HALF_LENGTH + Self::RANGE;
            let across = Field::HALF_WIDTH + Self::RANGE;
            airborne.position.x = airborne.position.x.clamp(-along, along);
            airborne.position.y = airborne.position.y.clamp(Self::HEIGHT.0, Self::HEIGHT.1);
            airborne.position.z = airborne.position.z.clamp(-across, across);
        }

        **camera = Transform::from_translation(airborne.position).with_rotation(facing);
    }

    /// The movement keys as one vector in the camera's own frame: x strafes,
    /// y climbs, z runs forward. WASD and the arrow keys are the same three
    /// axes twice over rather than two different controls — whichever hand
    /// is free gets the same camera.
    fn push(keys: &ButtonInput<KeyCode>) -> Vec3 {
        let axis = |plus: &[KeyCode], minus: &[KeyCode]| {
            f32::from(keys.any_pressed(plus.iter().copied()))
                - f32::from(keys.any_pressed(minus.iter().copied()))
        };
        Vec3::new(
            axis(
                &[KeyCode::KeyD, KeyCode::ArrowRight],
                &[KeyCode::KeyA, KeyCode::ArrowLeft],
            ),
            axis(&[KeyCode::KeyE], &[KeyCode::KeyQ]),
            axis(
                &[KeyCode::KeyW, KeyCode::ArrowUp],
                &[KeyCode::KeyS, KeyCode::ArrowDown],
            ),
        )
    }

    /// Takes the arrow keys and the space bar off the page.
    ///
    /// Same problem as the pointer buttons next door, and for the same reason
    /// — the page keeps the keyboard (see [`crate::MatchViewer::start`]), so
    /// winit's own suppression is off and the browser still answers an arrow
    /// with a scroll and a space with a page-down. Both would walk the article
    /// under the canvas while the camera flew.
    ///
    /// The listener goes on the canvas, not the document: keydown only reaches
    /// it while it has focus, so the rest of the page keeps its scrolling.
    /// WASD needs no such treatment — those keys have no default action.
    pub fn claim_flight_keys(canvas: &str) {
        let Some(element) = ViewerCanvas::find(canvas) else {
            return;
        };

        let press = Closure::<dyn FnMut(KeyboardEvent)>::new(|event: KeyboardEvent| {
            if matches!(
                event.key().as_str(),
                "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | " "
            ) {
                event.prevent_default();
            }
        });
        if element
            .add_event_listener_with_callback("keydown", press.as_ref().unchecked_ref())
            .is_err()
        {
            web_sys::console::warn_1(&"match viewer: keydown refused".into());
        }
        // Lives as long as the page, like the pointer listeners above.
        press.forget();
    }

    /// Puts the keyboard on the canvas as the replay opens, so the flight keys
    /// answer the first press.
    ///
    /// The counterpart to the note on [`CameraFlight::claim_flight_keys`]: a
    /// keydown only reaches the canvas while it has focus, which meant the
    /// arrow keys did nothing at all until the viewer had been clicked — and a
    /// control you have to discover by clicking first is one the legend above
    /// the stage promises and the page does not keep.
    ///
    /// A canvas is not focusable on its own, so `tabindex` comes first; it is
    /// set here rather than in the page's markup to keep it with the reason it
    /// exists. `preventScroll` because focus otherwise scrolls its element into
    /// view, and the stage is already exactly where the page put it.
    ///
    /// Runs at `Startup` rather than beside the claims in
    /// [`crate::MatchViewer::start`]: those only add listeners, but focus is
    /// state winit is free to move when it adopts the canvas, so this has to
    /// come after it has.
    pub fn focus_canvas(config: Res<ViewerConfig>) {
        let Some(element) = ViewerCanvas::find(&config.canvas) else {
            return;
        };
        let Some(element) = element.dyn_ref::<HtmlElement>() else {
            web_sys::console::warn_1(&"match viewer: canvas is not an HTML element".into());
            return;
        };

        element.set_tab_index(0);
        let options = FocusOptions::new();
        options.set_prevent_scroll(true);
        if element.focus_with_options(&options).is_err() {
            web_sys::console::warn_1(&"match viewer: canvas refused focus".into());
        }
    }
}

/// Everything the operator can move: where the rig stands, where it has been
/// flown, how tight the lens is, and which of the twenty-two it is following.
///
/// The four are separate resources because they are moved by separate
/// gestures, but there is only one shot, and one control that puts all of it
/// back — which is what this exists to hold.
pub struct CameraRig;

impl CameraRig {
    /// Back to the frame the replay opens on: on the gantry, at rest zoom,
    /// following the ball.
    pub fn reset(
        orbit: &mut CameraOrbit,
        zoom: &mut CameraZoom,
        flight: &mut CameraFlight,
        subject: &mut CameraSubject,
    ) {
        *orbit = CameraOrbit::default();
        *zoom = CameraZoom::default();
        flight.land();
        subject.release();
    }

    /// Has anything been moved off the opening shot? The reset button lights
    /// up on this, which is the only thing that says out loud that the camera
    /// is somewhere the replay did not put it.
    pub fn moved(
        orbit: &CameraOrbit,
        zoom: &CameraZoom,
        flight: &CameraFlight,
        subject: &CameraSubject,
    ) -> bool {
        flight.airborne()
            || subject.locked()
            || orbit.bearing.abs() > 1e-3
            || (orbit.elevation - TvCamera::rest_elevation()).abs() > 1e-3
            || (zoom.factor - 1.0).abs() > 1e-3
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
    ///
    /// Then 20% tighter (0.382 → 0.318), and 20% tighter again (0.318 → 0.265),
    /// which is where it stands: 1.06× TIGHTER than the original 0.28 rather
    /// than wider, so the 1.5× has now been walked all the way back and a step
    /// past. Each of these is done here rather than by resting the wheel at
    /// 1.2×, so that 1.00x keeps meaning "the shot the replay opens on" — the
    /// reset chip, the zoom readout and `CameraRig::moved` all read that
    /// number.
    ///
    /// What the tightening buys is turf. By the note above the pitch subtends
    /// only ~0.21 rad from a rig this low, so a 0.318 frame was about two
    /// thirds playing surface and one third stand and sky; at 0.265 it is about
    /// four fifths. The cost is at the other end of the wheel — the widest shot
    /// it can reach comes in by the same 20% — and that end was already
    /// spending itself on sky.
    const FOV: f32 = 0.265;
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

    /// What the four constants above become when the rig is following one
    /// player rather than the ball — see [`CameraSubject`], whose grip blends
    /// each of them in and back out again.
    ///
    /// They have to move together with the lens, and that is the whole reason
    /// they exist. Everything above frames the PLAY: the rig slides only four
    /// fifths of the way after the ball, the aim leads it across the pitch by
    /// less than a third, and a metre of near bias puts the touchline on the
    /// bottom edge. All three are the right answer for a wide shot and all
    /// three are wrong for a close one — at twice the magnification the
    /// horizontal frame is about 0.118 rad wide, and a ball out by the goal
    /// line sits 0.127 rad off the axis those constants aim down. Which is to
    /// say the player the camera had been asked to follow would be just off the
    /// edge of the frame.
    ///
    /// Followed, the rig tracks him one for one and looks straight at him: the
    /// slide, the lead and the bias all go, and the aim is lifted off the turf
    /// to his chest so he sits in the middle of the shot rather than along its
    /// bottom edge.
    const LOCK_TRAVEL: f32 = 1.0;
    const LOCK_TRAVEL_LIMIT: f32 = 0.95;
    const LOCK_AIM_ACROSS: f32 = 1.0;
    /// How far up him the shot is centred, in metres.
    const LOCK_RISE: f32 = 1.05;
    /// A close shot needs a quicker operator, which is the note on `RESPONSE`
    /// taken at its word: at twice the magnification the same lag puts twice
    /// as much of him off centre.
    const LOCK_RESPONSE: f32 = 0.18;

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

    pub fn spawn(mut commands: Commands, quality: Res<Quality>, stage: Res<Stage>) {
        let sideline = -Self::rest_reach();
        commands.spawn((
            TvCamera { focus: Vec3::ZERO },
            Camera3d::default(),
            // Not the canvas. The replay is drawn into an image that may be
            // smaller than the window, and a second camera puts that image on
            // the window with the transport bar over it — see `stage`, which
            // explains why the pixels have to come from here rather than from
            // the window's own scale factor, and why the bar is better off for
            // it.
            stage.target(),
            Projection::from(PerspectiveProjection {
                fov: Self::FOV,
                ..default()
            }),
            // How the edges in this scene are resolved, and the one setting
            // here that is decided by the machine rather than by taste. Bevy's
            // default is four samples and nothing ever said otherwise, which
            // on an integrated part is four times the attachment bandwidth of
            // the whole frame — see `quality`, which explains what replaces
            // them and why the swap is not a downgrade of the scene.
            quality.msaa(),
            quality.fxaa(),
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
            // The horizon stop of the sky gradient is this same colour, so the
            // far stands do not fade to one value and then meet a background of
            // another. See `Sky`, which used to be a near-black `ClearColor` and
            // is now the dome that colour was standing in for.
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
        flight: Res<CameraFlight>,
        subject: Res<CameraSubject>,
        changeover: Res<ChangeoverShot>,
        mut camera: Single<(&mut TvCamera, &mut Transform, &mut Projection)>,
    ) {
        let (rig, transform, projection) = &mut *camera;

        // How far the shot has closed onto one player, 0..1. Every framing
        // constant below that a follow changes is blended by it, so picking a
        // man out of the crowd is one continuous move of the camera rather
        // than a cut to a second one.
        let grip = subject.grip();
        let close = |wide: f32, tight: f32| wide + (tight - wide) * grip;

        // The lens. `FOV` is the framing at 1.0; zooming in narrows it. The
        // follow multiplies the wheel rather than moving it — see
        // [`CameraSubject::magnification`].
        if let Projection::Perspective(perspective) = projection.as_mut() {
            let wanted = Self::FOV
                / (zoom.factor * subject.magnification() * changeover.magnification()).max(0.01);
            if (perspective.fov - wanted).abs() > 1e-4 {
                perspective.fov = wanted;
            }
        }

        // A followed player outranks the ball, which is the whole point of
        // him: the shot he is in is the one the viewer asked for, wherever the
        // football has got to.
        let target = match (subject.target(), ball.on_pitch) {
            (Some(at), _) => Vec3::new(at.x, 0.0, at.z),
            (None, true) => Vec3::new(ball.position.x, 0.0, ball.position.z),
            (None, false) => Vec3::ZERO,
        };

        if playback.seeked {
            rig.focus = target;
        } else {
            // Exponential catch-up, framerate independent.
            let response = close(Self::RESPONSE, Self::LOCK_RESPONSE);
            let blend = 1.0 - (-time.delta_secs() / response).exp();
            rig.focus = rig.focus.lerp(target, blend.clamp(0.0, 1.0));
        }

        // In flight the transform belongs to `CameraFlight::steer`. The lens
        // and the focus above are still this system's — the wheel keeps
        // working in the air, and a focus kept warm means landing picks the
        // play up where it is rather than swinging across the ground to find
        // it.
        if flight.airborne() {
            return;
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
        let limit = along * close(Self::TRAVEL_LIMIT, Self::LOCK_TRAVEL_LIMIT);
        let travel =
            (rig.focus.dot(rail) * close(Self::TRAVEL, Self::LOCK_TRAVEL)).clamp(-limit, limit);

        let looking_at = rail * rig.focus.dot(rail)
            + depth
                * (rig.focus.dot(depth) * close(Self::AIM_ACROSS, Self::LOCK_AIM_ACROSS)
                    + close(Self::AIM_NEAR_BIAS, 0.0))
            + Vec3::Y * close(0.0, Self::LOCK_RISE);

        // …and last, the one shot that is not this rig at all. A substitution
        // is watched from beside the pitch rather than from the gantry — see
        // [`ChangeoverShot`], which mixes the two ends rather than cutting
        // between them, so the move out and the move back are one path.
        let (stand, aim) = changeover.blend(gantry + rail * travel, looking_at);
        transform.translation = stand;
        transform.look_at(aim, Vec3::Y);
    }
}
