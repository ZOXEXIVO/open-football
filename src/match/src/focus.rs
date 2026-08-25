//! Following one player: the click that picks him out of the crowd, the shot
//! that tightens onto him, and the ring on the grass that says which of the
//! twenty-two the camera is on.
//!
//! Three parts that only make sense together, which is why they share a file.
//! [`CameraSubject`] is the fact — who, how far the shot has closed on him,
//! and where he is standing this frame. [`crate::camera::TvCamera`] reads it
//! instead of the ball. [`FocusRing`] is the only thing on screen that says
//! any of it out loud: a replay that quietly stops framing the ball and gives
//! no reason for it reads as a broken camera.

use crate::actors::PlayerActor;
use crate::body::Physique;
use crate::camera::CameraFlight;
use crate::touch::{TouchControls, TouchDevice};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::f32::consts::{FRAC_PI_2, TAU};

/// The player the camera has been asked to follow.
///
/// Held as an entity rather than as a player id because that is what the pick
/// finds and what the ring tracks; the squad is spawned once and never
/// respawned, so an actor's entity is his for the session.
#[derive(Resource, Default)]
pub struct CameraSubject {
    /// Who — or `None` when the shot belongs to the ball again.
    locked: Option<Entity>,
    /// Where he is standing, refreshed by [`Self::settle`] once the bodies
    /// have been placed.
    ///
    /// Cached rather than looked up where it is wanted, because the camera
    /// system already holds the lens transform mutably and Bevy cannot prove a
    /// second `&Transform` query is disjoint from it. One `Vec3` copied per
    /// frame buys the camera a read that costs nothing and cannot conflict.
    at: Vec3,
    /// How far the shot has tightened onto him, 0..1.
    ///
    /// A ramp rather than a switch, and it is the whole difference between a
    /// broadcast camera picking a player up and a hard cut to a close-up. It
    /// runs both ways: letting go pulls back out over the same time rather
    /// than snapping to the wide shot.
    grip: f32,
}

impl CameraSubject {
    /// Seconds for the shot to close onto a player, and to give it back.
    const CLOSE_TIME: f32 = 0.55;
    /// How much tighter the lens goes once it is all the way on him. Enough
    /// that a man fills a useful share of the frame; not so much that the
    /// football around him — the pass he is making, the defender arriving —
    /// leaves it. The rest frame is about twenty-two metres tall out at the
    /// halfway line, so this leaves eleven: the man, and the ten metres of
    /// pitch his next few seconds happen in.
    const CLOSE: f32 = 2.0;
    /// How far a finger may slide and still count as a tap rather than as the
    /// one-finger camera drag it would otherwise be, in the logical pixels a
    /// touch arrives in.
    const TAP_SLOP: f32 = 12.0;
    /// How wide of a body a click may land and still find him, as a share of
    /// his drawn height. A footballer is roughly this fraction of his own
    /// height across the shoulders, so the target is about the size of the man
    /// — which is what makes a miss read as a miss rather than as a bug.
    const REACH: f32 = 0.42;
    /// …floored and capped, in screen pixels. The floor is what makes a player
    /// on the far touchline — twenty pixels tall on a lens this long —
    /// clickable at all; the cap stops a keeper filling half the frame from
    /// swallowing every click on his end of the ground.
    const REACH_LIMITS: (f32, f32) = (14.0, 72.0);
    /// Two bodies whose targets overlap by less than this share are both under
    /// the pointer, and the nearer one wins. Without it, a click on a man
    /// standing in front of another picks whichever the query reached first.
    const OVERLAP: f32 = 0.25;

    /// Where the camera should be pointed, or `None` while the ball has it.
    pub fn target(&self) -> Option<Vec3> {
        self.locked.map(|_| self.at)
    }

    /// How far the shot has closed, 0..1. Every framing constant the locked
    /// shot changes is blended by this.
    pub fn grip(&self) -> f32 {
        self.grip
    }

    /// How much tighter the lens is being held, as a multiple of the wheel's
    /// own factor. One at rest, [`Self::CLOSE`] once the shot is all the way
    /// on a player.
    ///
    /// Its own multiplier rather than a write into [`crate::camera::CameraZoom`]
    /// on purpose: the wheel belongs to the viewer, and a follow that moved it
    /// would either fight a hand on the wheel every frame or leave the zoom
    /// somewhere the viewer never put it once the follow ended.
    pub fn magnification(&self) -> f32 {
        1.0 + (Self::CLOSE - 1.0) * self.grip
    }

    /// Is a player being followed *now* — as opposed to the shot still pulling
    /// back out of one?
    pub fn locked(&self) -> bool {
        self.locked.is_some()
    }

    /// Back to the ball. The grip is deliberately left where it is: the shot
    /// widens over [`Self::CLOSE_TIME`] rather than cutting, and does it while
    /// the framing swings across to wherever the ball has got to.
    pub fn release(&mut self) {
        self.locked = None;
    }

    /// A click or a tap on the open pitch picks the player under it — or, when
    /// it lands on nobody, gives the shot back to the ball.
    ///
    /// Runs BEFORE the playhead advances, on purpose. What the pointer was
    /// aimed at is the frame that was on the screen when it went down, and
    /// that frame was drawn from last update's positions; testing against this
    /// update's would ask the viewer to lead a running player by however far he
    /// covers in a frame.
    pub fn handle_pick(
        mouse: Res<ButtonInput<MouseButton>>,
        keys: Res<ButtonInput<KeyCode>>,
        touches: Res<Touches>,
        device: Res<TouchDevice>,
        window: Single<&Window, With<PrimaryWindow>>,
        camera: Single<(&Camera, &Transform), With<Camera3d>>,
        actors: Query<(Entity, &Transform, &Visibility), With<PlayerActor>>,
        mut subject: ResMut<CameraSubject>,
        mut flight: ResMut<CameraFlight>,
    ) {
        // The way out that needs no aim. A viewer who has followed a player
        // into a corner and wants the whole pitch back should not have to find
        // a piece of empty grass to click on first.
        if keys.just_pressed(KeyCode::Escape) {
            subject.release();
            return;
        }

        let Some(at) = Self::pointer(&mouse, &touches, &window) else {
            return;
        };
        // The transport bar, the stick and the altitude buttons are not the
        // pitch. Touch has no second button to hide the camera behind, so
        // those cut-outs already exist for the one-finger drag — see
        // [`TouchControls::on_furniture`], which is that same test.
        if TouchControls::on_furniture(&window, at, device.seen()) {
            return;
        }

        match Self::under(at, &window, &camera, &actors) {
            Some(actor) => {
                subject.locked = Some(actor);
                // A free camera cannot follow anybody: its transform is written
                // from the flight state every frame and the broadcast rig's is
                // skipped entirely. Asking to follow a player is asking for the
                // rig back, so it lands — the same cut the RESET chip makes,
                // and for the same reason.
                flight.land();
            }
            // Grass. Which is the gesture for "stop following him": the click
            // that picks a player out of the crowd puts him back in it.
            None => subject.release(),
        }
    }

    /// Where the pointer went down this frame, if it did.
    ///
    /// A mouse press and a tap are the same gesture and answer here as one.
    /// The tap has to be told apart from the one-finger camera drag, which
    /// starts identically: a finger that has slid further than
    /// [`Self::TAP_SLOP`] was turning the ground, so touch is read on the
    /// RELEASE and only when it stayed put. The mouse has no such ambiguity —
    /// the orbit is on the right and wheel buttons — so it answers on the
    /// press, where a click belongs.
    fn pointer(
        mouse: &ButtonInput<MouseButton>,
        touches: &Touches,
        window: &Window,
    ) -> Option<Vec2> {
        if mouse.just_pressed(MouseButton::Left)
            && let Some(at) = window.cursor_position()
        {
            return Some(at);
        }
        touches
            .iter_just_released()
            .find(|touch| touch.distance().length() <= Self::TAP_SLOP)
            .map(|touch| touch.position())
    }

    /// Which player, if any, the pointer landed on.
    ///
    /// Screen-space rather than a ray cast into the scene, and not as a
    /// shortcut: a footballer here is a rig of fifty-odd small meshes with air
    /// between the arms and the body, and a ray through one of those gaps
    /// misses a man the viewer plainly clicked on. Measuring to the LINE from
    /// his boots to his crown treats him as the standing figure he reads as,
    /// and it costs twenty-two projections rather than a traversal of the
    /// scene.
    fn under(
        at: Vec2,
        window: &Window,
        camera: &(&Camera, &Transform),
        actors: &Query<(Entity, &Transform, &Visibility), With<PlayerActor>>,
    ) -> Option<Entity> {
        let (camera, camera_transform) = *camera;
        let lens = GlobalTransform::from(*camera_transform);

        // The replay is drawn into an image that may be smaller than the
        // window, so a projection lands in the target's pixels while the
        // pointer arrives in the window's logical ones. Exactly the correction
        // `Actors::place_labels` applies to put a name plate over a head, for
        // exactly the same reason — and read off the camera for the same one
        // again: on the frame a resize lands, the camera's own idea of its
        // target is what the projection used.
        let projected = camera.logical_viewport_size().unwrap_or(Vec2::ONE);
        let to_screen = Vec2::new(window.width(), window.height()) / projected.max(Vec2::ONE);

        // Every man the pointer is inside, with how central the hit was and how
        // far away he is standing.
        let mut hits: Vec<(Entity, f32, f32)> = Vec::new();
        for (entity, body, visibility) in actors {
            if *visibility == Visibility::Hidden {
                continue;
            }
            let boots = body.translation;
            let crown = boots + Vec3::Y * Physique::STATURE * body.scale.y;
            // Behind the lens, or off the far side of the projection: either
            // way there was nothing there to click on.
            let (Ok(head_of), Ok(foot_of)) = (
                camera.world_to_viewport(&lens, crown),
                camera.world_to_viewport(&lens, boots),
            ) else {
                continue;
            };
            let crown = head_of * to_screen;
            let boots = foot_of * to_screen;

            let stature = boots.distance(crown);
            let reach = (stature * Self::REACH).clamp(Self::REACH_LIMITS.0, Self::REACH_LIMITS.1);
            let miss = Self::distance_to_segment(at, boots, crown);
            if miss <= reach {
                let range = lens.translation().distance(body.translation);
                hits.push((entity, miss / reach, range));
            }
        }

        // The most central hit, and then anyone else the pointer is nearly as
        // deep inside — of whom the nearest to the camera wins, because that is
        // the one drawn in front and so the one that was clicked.
        let closest = hits
            .iter()
            .map(|(_, share, _)| *share)
            .fold(f32::INFINITY, f32::min);
        hits.into_iter()
            .filter(|(_, share, _)| *share <= closest + Self::OVERLAP)
            .min_by(|(_, _, near), (_, _, far)| near.total_cmp(far))
            .map(|(entity, _, _)| entity)
    }

    /// How far `point` is from the segment `from`–`to`.
    fn distance_to_segment(point: Vec2, from: Vec2, to: Vec2) -> f32 {
        let span = to - from;
        let length = span.length_squared();
        if length <= f32::EPSILON {
            return point.distance(from);
        }
        let along = ((point - from).dot(span) / length).clamp(0.0, 1.0);
        point.distance(from + span * along)
    }

    /// Keeps the lock honest and walks the grip toward it.
    ///
    /// Runs after the bodies have been placed and before anything reads the
    /// subject, so the position the camera frames and the position the ring is
    /// drawn at are this frame's rather than last frame's.
    pub fn settle(
        time: Res<Time>,
        actors: Query<(&Transform, &Visibility), With<PlayerActor>>,
        mut subject: ResMut<CameraSubject>,
    ) {
        if let Some(actor) = subject.locked {
            match actors.get(actor) {
                Ok((body, visibility)) if *visibility != Visibility::Hidden => {
                    subject.at = body.translation;
                }
                // Substituted off, sent off, or scrubbed past the end of his own
                // samples. A camera following a man who is no longer on the
                // pitch would sit staring at the spot he vanished from.
                _ => subject.release(),
            }
        }

        // Guarded, so the resource is not dirtied on every frame of the eighty
        // minutes nobody is being followed — the same rule the contact shadows
        // and the name plates keep about writes that change nothing.
        let wanted = if subject.locked() { 1.0 } else { 0.0 };
        if subject.grip != wanted {
            let step = time.delta_secs() / Self::CLOSE_TIME;
            subject.grip = if subject.grip < wanted {
                (subject.grip + step).min(wanted)
            } else {
                (subject.grip - step).max(wanted)
            };
        }
    }
}

/// The ring on the grass under the player being followed.
///
/// One entity, spawned hidden at startup and moved onto whoever is picked —
/// the same arrangement the contact shadows use, and for a stronger version of
/// the same reason: there is only ever one of these, and building and
/// despawning a mesh on every click is a pipeline's worth of work to say
/// something a transform already says.
#[derive(Component)]
pub struct FocusRing;

impl FocusRing {
    /// Outer radius, in metres. Just outside the contact shadow, which is about
    /// 1.13 m across — the ring has to read as drawn AROUND him rather than as
    /// a second shadow.
    const RADIUS: f32 = 0.82;
    /// …and how thick the band is. Seen from a rig 18 m up and 80 m back the
    /// circle is foreshortened to a sliver, so this is wider than it would need
    /// to be from overhead.
    const BAND: f32 = 0.16;
    /// Clear of the turf, of the paint at 0.012 and of the contact shadows at
    /// 0.018.
    const LIFT: f32 = 0.024;
    /// The viewer's own accent, lifted a little for the grass: this is
    /// furniture rather than football, and it should read as drawn ON the
    /// picture rather than as something lying in the stadium.
    const INK: Color = Color::srgba(0.36, 0.78, 1.0, 0.92);
    /// Seconds a breath takes, and how far it swells. Small on purpose — the
    /// pulse is there to say the ring is live rather than painted, and anything
    /// larger competes with the football for the eye.
    const BREATH: f32 = 1.7;
    const SWELL: f32 = 0.05;
    /// How much of the ring is drawn on the frame a player is picked, before
    /// the grip has opened it out. Not zero: the marker's job starts on that
    /// frame, and a ring that grows from nothing is a ring that is not there
    /// when the viewer looks for it.
    const SEED: f32 = 0.55;

    pub fn spawn(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
    ) {
        commands.spawn((
            FocusRing,
            Mesh3d(meshes.add(Annulus::new(Self::RADIUS - Self::BAND, Self::RADIUS).mesh())),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Self::INK,
                // Unlit, because it is not in the stadium: a marker that took
                // the floodlights would go dim in the shadow of a stand, which
                // is the one place a viewer most needs to find his man.
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                ..default()
            })),
            // The annulus is meshed in the XY plane facing +Z; this lays it on
            // the grass facing up.
            Transform::from_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
            Visibility::Hidden,
        ));
    }

    /// Puts the ring under whoever is being followed.
    ///
    /// It comes and goes with the LOCK rather than with the grip: the ring is
    /// the answer to "which player is this", and that answer is true from the
    /// frame he is picked. The grip only scales it, so it opens out with the
    /// shot instead of arriving at full size on a frame that is still wide.
    pub fn follow(
        time: Res<Time>,
        subject: Res<CameraSubject>,
        mut breath: Local<f32>,
        mut ring: Single<(&mut Transform, &mut Visibility), With<FocusRing>>,
    ) {
        let (transform, visibility) = &mut *ring;

        let Some(at) = subject.target() else {
            // Same rule as the contact shadows and the name plates: never write
            // a visibility that has not changed. Every write feeds the
            // propagation pass, and a hidden marker saying "still hidden" on
            // every frame of a match is an entity dirtied for nothing.
            if **visibility != Visibility::Hidden {
                **visibility = Visibility::Hidden;
            }
            return;
        };

        *breath = (*breath + time.delta_secs() / Self::BREATH).fract();
        let swell = 1.0 + Self::SWELL * (TAU * *breath).sin();
        let spread = (Self::SEED + (1.0 - Self::SEED) * subject.grip()) * swell;

        transform.translation = Vec3::new(at.x, Self::LIFT, at.z);
        transform.scale = Vec3::splat(spread);
        if **visibility != Visibility::Inherited {
            **visibility = Visibility::Inherited;
        }
    }
}
