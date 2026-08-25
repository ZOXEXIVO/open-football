//! The same camera, driven by fingers.
//!
//! Everything the replay's camera can do is reachable from a mouse and a
//! keyboard: the right button walks the rig round the ground, the wheel works
//! the lens, and WASD/QE fly it clear of the gantry altogether (see
//! [`crate::camera`]). None of the three exists on a phone, and a replay is
//! watched on phones.
//!
//! This module is the other half of those controls rather than a reduced
//! version of them. Nothing here decides what a gesture MEANS — a one-finger
//! drag hands its pixels to [`CameraOrbit::drag`], the same function the right
//! button calls, and a pinch hands its ratio to [`CameraZoom::scale`], the same
//! one a wheel notch goes through. Two code paths that both "turn the camera"
//! are two sets of numbers waiting to drift apart, and the whole point of the
//! touch controls is that they land in the places the mouse lands in.
//!
//! The one thing a finger genuinely cannot borrow is a held key. Flying is a
//! velocity — the camera moves for as long as W is down — and there is no
//! gesture that means "and keep doing that". So the flight keys get furniture
//! instead: a stick and a pair of altitude buttons, drawn only once the viewer
//! has actually seen a finger, writing into [`TouchDrive`] exactly what the
//! keyboard writes into its own push vector.

use crate::camera::{CameraFlight, CameraOrbit, CameraZoom};
use crate::textures::Textures;
use crate::timeline::Timeline;
use bevy::input::touch::{Touch, Touches};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// What the on-screen flight controls are asking of the free camera this
/// frame, in exactly the terms [`CameraFlight::steer`] already thinks in: a
/// push in the camera's own frame, and whether the pace is boosted.
///
/// Written from scratch every frame by [`FlightPad::handle_touch`], so there is
/// nothing to clear and no way for a finger that has come off to leave the
/// camera drifting. `steer` ADDS this to the keyboard's push rather than
/// choosing between them — a tablet with a keyboard attached answers both.
#[derive(Resource, Default)]
pub struct TouchDrive {
    pub push: Vec3,
    pub boost: bool,
}

/// Whether a finger has ever touched the canvas.
///
/// The flight furniture hangs on this, and it is a fact rather than a guess:
/// the page hides its mouse legend on `pointer: coarse`, but a media query is
/// about what the device HAS, and a laptop with a touchscreen has both. Waiting
/// for an actual touch puts the stick on the machines that are being touched
/// and keeps it out of the way on the ones that are not — including the
/// touchscreen laptop, right up until somebody uses the screen.
#[derive(Resource, Default)]
pub struct TouchDevice {
    seen: bool,
}

impl TouchDevice {
    /// Is the flight furniture on the canvas? Anything that has to decide
    /// whether a pointer landed on the replay or on a control has to know,
    /// because two of those controls are not drawn until a finger arrives.
    pub fn seen(&self) -> bool {
        self.seen
    }
}

/// The fingers currently on the open pitch, and where each was last frame.
///
/// Positions are tracked here rather than read from [`Touch::delta`], which
/// cannot be trusted for this: bevy only rolls `previous_position` forward on
/// frames that carried a touch event, so a finger that flicks and then holds
/// still keeps reporting the flick, and the camera spins on by itself.
///
/// Ordered, oldest first, because the pinch below reads the first two fingers
/// and [`Touches`] hands them over in a hash map's order — which is to say a
/// different pair on different frames.
#[derive(Resource, Default)]
pub struct TouchGesture {
    fingers: Vec<(u64, Vec2)>,
}

/// Where the viewer's touch furniture sits on the canvas.
///
/// One description, used for three things that have to agree exactly: laying
/// the nodes out, deciding which control a finger landed on, and deciding which
/// fingers belong to the camera instead. Measured in the logical pixels a touch
/// arrives in, which is the unit Bevy's UI lays out in as well.
struct Layout {
    size: Vec2,
}

impl Layout {
    /// Clear of the canvas edges, and of the transport bar below.
    const MARGIN: f32 = 14.0;
    /// The stick. Big enough to find with a thumb without looking, small
    /// enough that it is not what the eye lands on when the ball is in play —
    /// this sits over a live replay, and the replay has to stay the picture.
    const RADIUS: f32 = 44.0;
    const KNOB: f32 = 17.0;
    /// The two altitude buttons, stacked at the other corner. Square rather
    /// than round so they do not read as two more sticks, and smaller than the
    /// stick because they carry one axis between them where it carries two.
    const LIFT: f32 = 34.0;
    const LIFT_GAP: f32 = 8.0;

    fn of(window: &Window) -> Self {
        Layout {
            size: Vec2::new(window.width(), window.height()),
        }
    }

    /// Centre of the stick. Bottom left, which is where the hand holding a
    /// phone in landscape already is.
    fn stick(&self) -> Vec2 {
        Vec2::new(
            Self::MARGIN + Self::RADIUS,
            self.size.y - Timeline::BAR_HEIGHT - Self::MARGIN - Self::RADIUS,
        )
    }

    /// One of the altitude buttons, bottom right — the other thumb.
    fn lift(&self, up: bool) -> Rect {
        let floor = self.size.y - Timeline::BAR_HEIGHT - Self::MARGIN;
        let bottom = floor - if up { Self::LIFT + Self::LIFT_GAP } else { 0.0 };
        let right = self.size.x - Self::MARGIN;
        Rect::from_corners(
            Vec2::new(right - Self::LIFT, bottom - Self::LIFT),
            Vec2::new(right, bottom),
        )
    }

    /// Did this pointer land on the viewer's own furniture rather than on the
    /// open pitch?
    ///
    /// The camera gestures need this because touch has no second button to
    /// hide behind: the right button can drag the rig round without ever
    /// meaning anything to the transport bar, where one finger means both. So
    /// the bar, the stick and the two buttons are cut out of the canvas first,
    /// and what is left over turns the camera.
    ///
    /// The bar is a band across the bottom rather than a queried node, which is
    /// exactly what it is: [`Timeline::spawn`] gives it the full width and
    /// exactly [`Timeline::BAR_HEIGHT`] at the foot of a full-height column.
    ///
    /// `pad` says whether the flight furniture is on the canvas at all. It is
    /// hidden until a finger has been seen (see [`FlightPad::refresh`]), and a
    /// mouse — which asks this same question before picking a player out of the
    /// crowd — must not be blocked by two controls that are not being drawn.
    fn furniture(&self, at: Vec2, pad: bool) -> bool {
        if at.y >= self.size.y - Timeline::BAR_HEIGHT {
            return true;
        }
        pad && (at.distance(self.stick()) <= Self::RADIUS
            || self.lift(true).contains(at)
            || self.lift(false).contains(at))
    }
}

/// The camera gestures: what a finger does on the open pitch.
pub struct TouchControls;

impl TouchControls {
    /// How far apart two fingers have to be before the distance between them
    /// is worth dividing by. Two landing almost on top of each other report a
    /// separation of a pixel or two, and the ratio of one noisy pixel to the
    /// next is not a zoom, it is a lurch.
    const PINCH_FLOOR: f32 = 12.0;

    /// The first finger is what turns the flight furniture on.
    ///
    /// Runs ahead of everything that reads it, so the frame a touch device
    /// announces itself is the frame its controls exist — the alternative is a
    /// stick that can be grabbed before it can be seen.
    pub fn watch(touches: Res<Touches>, mut device: ResMut<TouchDevice>) {
        if !device.seen && touches.any_just_pressed() {
            device.seen = true;
        }
    }

    /// Did a pointer land on the viewer's own controls rather than on the open
    /// pitch?
    ///
    /// Shared with [`crate::focus`], which asks the same question of a mouse
    /// before it reads a click as "follow that player". One description of
    /// where the furniture is rather than two — a second copy is a second set
    /// of numbers waiting to drift, which is the note the one-finger drag
    /// already carries.
    pub fn on_furniture(window: &Window, at: Vec2, pad: bool) -> bool {
        Layout::of(window).furniture(at, pad)
    }

    /// One finger turns the camera; two work the lens.
    ///
    /// Deliberately only those two. Two fingers could also have been made to
    /// slide the rig about — every map does it — and it is left out for the
    /// same reason the wheel was never allowed to change the flight speed: a
    /// pinch and a two-finger slide are one grip, the centroid of a pinch
    /// always wanders, and the camera would creep every time the lens was
    /// touched. Flying is the stick's job and nothing else's.
    pub fn handle_gestures(
        touches: Res<Touches>,
        window: Single<&Window, With<PrimaryWindow>>,
        mut gesture: ResMut<TouchGesture>,
        mut orbit: ResMut<CameraOrbit>,
        mut zoom: ResMut<CameraZoom>,
        mut flight: ResMut<CameraFlight>,
    ) {
        let layout = Layout::of(&window);

        // Fingers that have come off. The last one to go ends the gesture,
        // which is the moment the gantry detent gets its chance — the same
        // moment releasing the right button gives it.
        let held = gesture.fingers.len();
        gesture
            .fingers
            .retain(|(id, _)| touches.get_pressed(*id).is_some());
        let let_go = held > 0 && gesture.fingers.is_empty();

        // ...and the ones that have just landed, in id order so that two
        // arriving on the same frame are always paired the same way round.
        //
        // A finger already being tracked is skipped rather than added again. It
        // sounds impossible and is not: a browser will happily report a second
        // press for a pointer that is already down when another one joins it,
        // and one finger listed twice reads here as a pinch between a point and
        // itself — no separation, no zoom, and the one-finger drag lost as well
        // because there are now two entries. Which is exactly how the pinch
        // failed the first time it was tried.
        let mut landed: Vec<&Touch> = touches.iter_just_pressed().collect();
        landed.sort_by_key(|touch| touch.id());
        for touch in landed {
            let known = gesture.fingers.iter().any(|(id, _)| *id == touch.id());
            // The pad is always cut out here: a finger on the glass is what
            // turns it on, and `TouchControls::watch` has already run this
            // frame — so by the time a touch reaches these gestures the stick
            // is on the canvas.
            if !known && !layout.furniture(touch.start_position(), true) {
                gesture.fingers.push((touch.id(), touch.position()));
            }
        }

        // Where each of them is now, and how far it has come since last frame.
        let mut moved: Vec<(Vec2, Vec2)> = Vec::with_capacity(gesture.fingers.len());
        for (id, last) in gesture.fingers.iter_mut() {
            let Some(touch) = touches.get_pressed(*id) else {
                continue;
            };
            let now = touch.position();
            moved.push((now, now - *last));
            *last = now;
        }

        match moved.as_slice() {
            // One finger IS the right-button drag — same pixels, same
            // constant, same turntable sense, and the same swap to a mouselook
            // once the rig is airborne.
            [(_, drag)] => orbit.drag(*drag, &mut flight),
            // Two is a pinch: the lens changes by the proportion the fingers
            // did, so spreading them to twice the span doubles the
            // magnification. Only the first two count — a third finger on a
            // phone is far more often the heel of a hand than an instruction.
            [(first, first_drag), (second, second_drag), ..] => {
                let now = first.distance(*second);
                let before = (*first - *first_drag).distance(*second - *second_drag);
                if now > Self::PINCH_FLOOR && before > Self::PINCH_FLOOR {
                    zoom.scale(now / before);
                }
            }
            [] => {}
        }

        if let_go {
            orbit.settle(&flight);
        }
    }
}

/// The stick, its knob, and the two altitude buttons.
///
/// State and furniture in one place because they are one control: the resource
/// is which finger has hold of what, and the components are what that looks
/// like.
#[derive(Resource, Default)]
pub struct FlightPad {
    /// The finger on the stick and where it is now.
    stick: Option<(u64, Vec2)>,
    /// The finger on an altitude button, and whether it is the up one.
    lift: Option<(u64, bool)>,
}

#[derive(Component)]
pub struct FlightStick;

#[derive(Component)]
pub struct FlightKnob;

#[derive(Component)]
pub struct FlightLift {
    up: bool,
}

impl FlightPad {
    /// How far the stick has to leave the middle before it asks for anything.
    /// A thumb resting on a control is not a request to fly, and without this
    /// the camera creeps for as long as the stick is touched at all.
    const DEAD: f32 = 0.14;
    /// And how far out it has to be pushed to ask for the shift key. Right on
    /// the rim, so it is somewhere a thumb arrives at deliberately.
    const RIM: f32 = 0.88;

    /// The same translucent slab the transport bar is drawn on, so the touch
    /// controls read as more of that furniture rather than as something the
    /// page has laid over the replay — and at the same weight, which is not a
    /// detail. This sits over whatever the camera happens to be pointed at:
    /// against the bright turf a light slab is legible and against a floodlit
    /// stand it disappears, so it has to be dark enough to hold either.
    const SLAB: Color = Color::srgba(0.035, 0.055, 0.085, 0.82);
    const EDGE: Color = Color::srgba(1.0, 1.0, 1.0, 0.20);
    const EDGE_LIVE: Color = Color::srgba(1.0, 1.0, 1.0, 0.40);
    const KNOB: Color = Color::srgba(1.0, 1.0, 1.0, 0.34);
    /// Blue for "this is what is moving" — the colour the bar already gives to
    /// the part of the match that has been played.
    const LIVE: Color = Color::srgba(0.29, 0.68, 0.98, 0.92);
    /// And the play button's green out at the rim, where the stick has taken
    /// the place of a shift key.
    const FAST: Color = Color::srgba(0.15, 0.72, 0.40, 0.95);

    /// Spawned at startup and hidden, rather than spawned when the first touch
    /// arrives: a control that has to be built mid-frame is a control that
    /// misses the very press that asked for it.
    pub fn spawn(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
        commands
            .spawn((
                FlightStick,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(Layout::MARGIN),
                    bottom: px(Timeline::BAR_HEIGHT + Layout::MARGIN),
                    width: px(Layout::RADIUS * 2.0),
                    height: px(Layout::RADIUS * 2.0),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(Layout::RADIUS)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Self::SLAB),
                BorderColor::all(Self::EDGE),
            ))
            .with_child((
                FlightKnob,
                Node {
                    position_type: PositionType::Absolute,
                    // Inside the ring's border, which is what an absolutely
                    // placed child is measured from.
                    left: px(Layout::RADIUS - Layout::KNOB - 1.0),
                    top: px(Layout::RADIUS - Layout::KNOB - 1.0),
                    width: px(Layout::KNOB * 2.0),
                    height: px(Layout::KNOB * 2.0),
                    border_radius: BorderRadius::all(px(Layout::KNOB)),
                    ..default()
                },
                BackgroundColor(Self::KNOB),
            ));

        for up in [true, false] {
            let stacked = if up {
                Layout::LIFT + Layout::LIFT_GAP
            } else {
                0.0
            };
            commands
                .spawn((
                    FlightLift { up },
                    Node {
                        position_type: PositionType::Absolute,
                        right: px(Layout::MARGIN),
                        bottom: px(Timeline::BAR_HEIGHT + Layout::MARGIN + stacked),
                        width: px(Layout::LIFT),
                        height: px(Layout::LIFT),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(8)),
                        display: Display::None,
                        ..default()
                    },
                    BackgroundColor(Self::SLAB),
                    BorderColor::all(Self::EDGE),
                ))
                .with_child((
                    ImageNode {
                        image: Textures::lift_icon(&mut images, up),
                        ..default()
                    },
                    Node {
                        width: px(13),
                        height: px(13),
                        ..default()
                    },
                ));
        }
    }

    /// Turns whatever is holding the furniture into the push vector the flight
    /// system already understands.
    ///
    /// Each control keeps its OWN finger, which is the whole reason this is
    /// written against [`Touches`] rather than against Bevy's `Interaction`:
    /// that reports the first press and no more, where climbing while flying
    /// forward is two thumbs at once and has to be.
    pub fn handle_touch(
        touches: Res<Touches>,
        window: Single<&Window, With<PrimaryWindow>>,
        mut pad: ResMut<FlightPad>,
        mut drive: ResMut<TouchDrive>,
    ) {
        let layout = Layout::of(&window);

        for touch in touches.iter_just_pressed() {
            let at = touch.start_position();
            if pad.stick.is_none() && at.distance(layout.stick()) <= Layout::RADIUS {
                pad.stick = Some((touch.id(), at));
            } else if pad.lift.is_none() {
                for up in [true, false] {
                    if layout.lift(up).contains(at) {
                        pad.lift = Some((touch.id(), up));
                    }
                }
            }
        }

        // Follow them, and let go of whatever has come off the glass.
        let stick = pad
            .stick
            .and_then(|(id, _)| touches.get_pressed(id).map(|touch| (id, touch.position())));
        let lift = pad
            .lift
            .filter(|(id, _)| touches.get_pressed(*id).is_some());
        pad.stick = stick;
        pad.lift = lift;

        let mut push = Vec3::ZERO;
        let mut boost = false;
        if let Some((_, at)) = pad.stick {
            let offset = (at - layout.stick()) / Layout::RADIUS;
            let reach = offset.length();
            // Rescaled past the dead zone rather than merely gated by it, so
            // the first millimetre of real travel is the slowest speed rather
            // than a seventh of full pelt.
            let throttle = ((reach - Self::DEAD) / (1.0 - Self::DEAD)).clamp(0.0, 1.0);
            let heading = offset.normalize_or_zero() * throttle;
            push.x = heading.x;
            // Screen y runs down the canvas; pushing the stick away from you
            // flies away from you.
            push.z = -heading.y;
            boost = reach >= Self::RIM;
        }
        if let Some((_, up)) = pad.lift {
            push.y = if up { 1.0 } else { -1.0 };
        }

        drive.push = push;
        drive.boost = boost;
    }

    /// Shows the furniture once there is a finger to use it, and paints what
    /// that finger is doing with it.
    ///
    /// The knob following the thumb is not decoration: the stick is drawn over
    /// a pitch that is itself moving, and without it there is nothing at all to
    /// say the control was hit rather than missed.
    pub fn refresh(
        device: Res<TouchDevice>,
        pad: Res<FlightPad>,
        window: Single<&Window, With<PrimaryWindow>>,
        mut ring: Query<
            (&mut Node, &mut BorderColor),
            (With<FlightStick>, Without<FlightKnob>, Without<FlightLift>),
        >,
        mut knob: Query<
            (&mut Node, &mut BackgroundColor),
            (With<FlightKnob>, Without<FlightStick>, Without<FlightLift>),
        >,
        mut lifts: Query<
            (&FlightLift, &mut Node, &mut BackgroundColor),
            (Without<FlightStick>, Without<FlightKnob>),
        >,
    ) {
        let shown = if device.seen {
            Display::Flex
        } else {
            Display::None
        };
        let layout = Layout::of(&window);

        // How far the thumb has taken the stick, as a fraction of the ring.
        // Clamped to the rim: a thumb that slides off the control keeps flying
        // in that direction, which is what a stick with a gate round it does.
        let offset = pad
            .stick
            .map(|(_, at)| ((at - layout.stick()) / Layout::RADIUS).clamp_length_max(1.0))
            .unwrap_or(Vec2::ZERO);
        let reach = offset.length();

        if let Ok((mut node, mut border)) = ring.single_mut() {
            node.display = shown;
            let wanted = if pad.stick.is_some() {
                Self::EDGE_LIVE
            } else {
                Self::EDGE
            };
            border.set_all(wanted);
        }
        if let Ok((mut node, mut fill)) = knob.single_mut() {
            let travel = offset * (Layout::RADIUS - Layout::KNOB);
            node.left = px(Layout::RADIUS - Layout::KNOB - 1.0 + travel.x);
            node.top = px(Layout::RADIUS - Layout::KNOB - 1.0 + travel.y);
            let wanted = match pad.stick {
                Some(_) if reach >= Self::RIM => Self::FAST,
                Some(_) if reach >= Self::DEAD => Self::LIVE,
                _ => Self::KNOB,
            };
            if fill.0 != wanted {
                fill.0 = wanted;
            }
        }
        for (lift, mut node, mut fill) in &mut lifts {
            node.display = shown;
            let held = pad.lift.is_some_and(|(_, up)| up == lift.up);
            let wanted = if held { Self::LIVE } else { Self::SLAB };
            if fill.0 != wanted {
                fill.0 = wanted;
            }
        }
    }
}
