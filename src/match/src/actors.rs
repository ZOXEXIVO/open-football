use crate::aftermath::Aftermath;
use crate::body::{BodyParts, Carriage, Footballer, Gait, Joint, Physique};
use crate::config::{PlayerInfo, ViewerConfig};
use crate::field::Field;
use crate::kit::{Complexion, Wardrobe};
use crate::loader::ChunkLoader;
use crate::pitch::Pitch;
use crate::playback::Playback;
use crate::portrait::Portraits;
use crate::replay::{ReplayTracks, Track};
use crate::textures::Textures;
use crate::timeline::DebugOverlay;
use crate::typeface::Faces;
use bevy::prelude::*;
use bevy::text::FontSource;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// A player on the pitch: the root of one footballer's rig, carrying where they
/// are, which way they are facing and where they have got to in their stride.
/// The name plate that tracks them is drawn as a UI node (see [`PlayerLabel`]).
#[derive(Component)]
pub struct PlayerActor {
    pub id: u32,
    /// Where this player stood last frame. How they are moving — heading,
    /// stride, effort — is read back out of the replay from this, which is what
    /// keeps the animation in step with the playhead however it is being
    /// driven: playing, scrubbed or run at 8x.
    previous: Option<Vec3>,
    /// Smoothed facing, in radians about the world Y axis.
    heading: f32,
    /// Position in the run cycle, in radians. Advanced by ground covered rather
    /// than by time, so the feet keep pace with the turf at any playback speed.
    phase: f32,
    /// Smoothed ground speed in metres per second of *match* time.
    speed: f32,
    /// …and smoothed ground VELOCITY, in the same units: which way he is
    /// going, as opposed to how fast.
    ///
    /// Its own field rather than a direction derived from `speed`, because
    /// the heading cannot be read off one frame's displacement: the
    /// recording's own quantisation is coarser than a frame of jogging, so
    /// the instantaneous direction is mostly rounding error. See the note
    /// at the site in [`Actors::animate`].
    travel: Vec3,
    /// Direction this player last struck the ball in, and how long he stays
    /// turned that way. Nobody passes or shoots across their own body without
    /// opening up to the ball first; before this a player kicking sideways
    /// while running stayed square to his RUN, so the ball left at an angle
    /// his body never acknowledged.
    strike: Option<(Vec3, f32)>,
    /// A slow clock-driven cycle for the movement a player makes when he is
    /// NOT running — breathing, shifting his weight, letting his arms drift.
    idle: f32,
    /// Smoothed turn rate, −1..1, so the body can bank into a change of
    /// direction instead of pivoting on the spot.
    turn: f32,
    /// Smoothed yaw from his facing to the ball: where he is looking.
    look: f32,
    /// Whether this is a goalkeeper. Only a keeper ever takes the ball in his
    /// hands or leaves his feet, so only a keeper is a candidate for the
    /// cradle, the dive and the leap.
    is_goalkeeper: bool,
    /// How far into the hold he is, 0..1. See [`Gait::carry`].
    carry: f32,
    /// How far into a dive, 0..1 — a body off its feet and committed rather
    /// than a body running.
    ///
    /// Driven by the recorded height, because that is the only honest signal
    /// there is: `PreparingForSave` shares the dive's speed band and reaches
    /// the same 12 m/s shuffling along the line, so no test on speed or
    /// direction can separate them. Measured against a real recording, a
    /// speed-and-lateral test caught 2 dives in 10 and fired for minutes on
    /// the shuffle. Leaving the ground is what only a dive does.
    dive: f32,
    /// How far through the extension, 0..1 — see [`Gait::stretch`].
    ///
    /// The one above is a switch and this is the ramp. A recorded dive lasts
    /// 390–660 ms; without a ramp the whole of it is a single frozen pose,
    /// which is what a save used to look like.
    stretch: f32,
    /// Seconds of MATCH time off the turf, and since the landing. Only one of
    /// the two ever runs.
    air: f32,
    down: f32,
    /// Fastest rise seen since he left the ground, in metres per second of
    /// match time.
    ///
    /// The angle a body leaves the ground at is the angle it travels at, so
    /// this against his ground speed is how flat the dive is — and it is the
    /// one measurement that separates a keeper thrown full length along the
    /// floor from one going straight up at a corner. Recorded dives launch
    /// at 10–15° above the horizontal; the standing leap at a cross launches
    /// at 45°.
    climb: f32,
    /// How flat that made the dive, 0..1 — latched at take-off and held, and
    /// the magnitude the tip below carries.
    flat: f32,
    /// Height last frame, so the rise above can be measured at all.
    previous_height: f32,
    /// Which way he is tipping, in his own frame: x onto his right, y over
    /// his toes, each −1..1.
    ///
    /// Both axes, because a keeper's dive is very often not the poster one:
    /// measured across a recorded match, dives split about evenly between
    /// those that travel further across the goal and those that travel
    /// further up the pitch — a man going down at a striker's feet rather
    /// than flying into a top corner.
    tip: Vec2,
    /// How far off the turf the RECORDING says he is, in metres.
    ///
    /// Not inferred, unlike the topple above: the engine gives a keeper a
    /// real vertical axis on the tick he commits to a jump or a dive
    /// (`MatchPlayer::leap`), so the height is measured and comes down the
    /// wire as the fourth element of his position sample. Everything on this
    /// pitch that can be recorded should be read rather than guessed.
    height: f32,
    /// The kick he is in the middle of, if any. See [`Kick`].
    kick: Option<Kick>,
    /// Smoothed acceleration along his own running line, −1..1: driving off
    /// the mark at +1, pulling up short at −1. See [`Gait::drive`].
    drive: f32,
    /// How much the ball is at his feet, 0..1. See [`Gait::carrying`].
    carrying: f32,
    /// Smoothed pitch from his eyeline to the ball, in radians.
    look_pitch: f32,
    /// How set he is, 0..1 — a keeper on his toes with a shot on. See
    /// [`Gait::set`].
    set: f32,
    /// Which end he defends. Only ever read to decide whether a goal was
    /// scored at his end or the other one — see [`Aftermath`].
    is_home: bool,
    /// How he took the last goal, 0..1 each, smoothed. Exactly one of them
    /// is ever non-zero, and for most of a match neither is. See
    /// [`Gait::despair`] and [`Gait::elation`].
    despair: f32,
    elation: f32,
    /// Which way he is going relative to his CHEST — see [`Gait::course`],
    /// which carries the same vector read against his legs instead. Its own
    /// field rather than a derivation at the point of use because the
    /// heading it is measured against is still turning, so it has to be
    /// taken after this frame's turn and before the pose is built.
    course: Vec2,
    /// How far his legs have turned off that chest onto the run, in radians
    /// — see [`Actors::opening`] and [`Gait::open`].
    open: f32,
    /// …and `course` read in the frame those legs are in, which is what the
    /// stride model and every lateral term in the pose actually want. See
    /// [`Actors::underfoot`].
    underfoot: Vec2,
    /// The hip amplitude the ground he is covering demands, in radians —
    /// see [`Gait::carry_ground`].
    carry_ground: f32,
    /// The ball that is about to arrive at him, if one is. Only ever a
    /// keeper's — see [`Save`].
    arrival: Option<Save>,
    /// How far into the reach he is, 0..1. Ratchets up as the ball closes
    /// and is given back by the follow-through, the same shape the dive's
    /// extension takes and for the same reason: a keeper does not fold his
    /// arms back up halfway through a save.
    reaction: f32,
    /// Where he is reaching, in his own frame — see [`Gait::save_aim`].
    aim: Vec2,
    /// …and whether it is a catch or a parry, smoothed so the hands are not
    /// deciding on the frame of contact.
    parry: f32,
    /// The match clock, in seconds, as the playhead has it.
    ///
    /// The only thing on this actor that is a time rather than a state, and
    /// it is here for the one behaviour that cannot be derived from the
    /// recording at all: **what a goalkeeper does with the eighty minutes of
    /// a match in which nothing is happening to him.** See
    /// [`Gait::urging`]. Reading the clock rather than integrating means a
    /// seek lands him wherever the clock says, which is right — a gesture is
    /// not a trajectory.
    clock: f32,
    /// **The gait this actor is being drawn in**, worked once at the end of
    /// its own update and read by everything downstream.
    ///
    /// [`Actors::animate`]'s second loop walks JOINTS, not players, and it
    /// built a whole `Gait` for each one — fifty-odd times per man, twelve
    /// hundred times a frame, every one of them the same answer. That was
    /// nearly free while a gait was a struct of copies; it stopped being
    /// free when the reaction, the idle gesture and the carriage angle
    /// joined it, which between them cost three hashes and a handful of
    /// trigonometry per call. Cached it is twenty-two.
    pose: Gait,
}

/// The name plate for one player, positioned each frame from the rig's
/// projected screen position.
#[derive(Component)]
pub struct PlayerLabel {
    pub actor: Entity,
}

/// The engine-state line under a name plate. Debug overlay only — this is the
/// whole reason the match harness has a viewer at all.
#[derive(Component)]
pub struct PlayerStateLabel {
    pub id: u32,
}

#[derive(Component)]
pub struct BallActor;

/// The contact patch under the ball. Without it a lofted ball is impossible to
/// place on the turf from a broadcast angle.
#[derive(Component)]
pub struct BallShadow;

/// One player's shadow on the turf.
///
/// A root entity that TRACKS a player rather than a child that hangs off him,
/// which it used to be, and the difference is the whole point of it. A shadow
/// belongs to the light and not to the body: it has to lie along the light's
/// bearing whichever way the player happens to be facing, and a child of an
/// actor is rotated by his heading and scaled by his build. Following him from
/// outside costs one transform a frame and buys a shadow that stays put while
/// he turns.
#[derive(Component)]
pub struct Silhouette {
    pub actor: Entity,
}

/// Where the ball is right now, in world space, so the camera does not have to
/// go looking for it.
#[derive(Resource, Default)]
pub struct BallState {
    pub position: Vec3,
    pub on_pitch: bool,
    /// Where the ball was on the previous frame, and how fast it is going in
    /// metres per second of MATCH time. The recording carries positions only,
    /// so — exactly as with the players — movement is read back out of it.
    ///
    /// Wanted by the strike detector in [`Actors::animate`]: a player who has
    /// just hit the ball has to turn and face where he hit it.
    pub previous: Option<Vec3>,
    pub velocity: Vec3,
    /// Angular velocity in radians per second of match time, and the rotation
    /// it has accumulated so far. See [`BallSpin`].
    pub spin: Vec3,
    pub rotation: Quat,
    /// What the ball was doing when it was last struck, so the bend it has put
    /// on since can be measured against it. `None` whenever it is not in
    /// flight.
    pub flight: Option<Flight>,
    /// The goalkeeper holding it, and how far his gloves are from where the
    /// recording says the ball is.
    ///
    /// An OFFSET rather than the glove position itself, and that is the whole
    /// design: `held_by` clears on the frame he lets go, but the ramp below
    /// takes a few more to run down, and an absolute point left behind by a
    /// keeper who has just thrown the ball twenty metres drags it visibly
    /// backwards out of his hands. A displacement applied to wherever the ball
    /// actually is now shrinks to nothing in the same few frames without ever
    /// pulling against its flight.
    pub held_by: Option<u32>,
    pub cradle_offset: Vec3,
    /// 0..1 ramp on the hold, shared by the ball's position and the keeper's
    /// arms so the two can never disagree about whether he has it.
    pub cradle: f32,
    /// The next kick, read out of the recording *before* it happens. `None`
    /// whenever nobody is about to hit it.
    pub impact: Option<Impact>,
    /// Whoever is nearest the ball this frame, and how far off he is in
    /// metres. The man on the ball, when that distance is short.
    pub nearest: Option<(u32, f32)>,
}

/// What a player hit the ball WITH.
///
/// The rig used to have exactly two answers — a boot, or a goalkeeper's hands
/// — and it inferred the second from whether he happened to be carrying it.
/// A footballer has four, and the other two are not rare: a ball arriving
/// above head height is met with a head, and a match contains forty-odd
/// throw-ins. Both used to be drawn as a leg swing, which at a corner is a man
/// hooking his boot up past his own ear.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Strike {
    Boot,
    Head,
    /// Out of a goalkeeper's hands, one-armed and overarm.
    Throw,
    /// And off the touchline, two-handed and over the head.
    ThrowIn,
}

/// One swing of a leg — the single most repeated thing a footballer does, and
/// the one this rig had nothing at all for.
///
/// Measured over a recorded match, the ball is struck **14.8 times a minute**:
/// a pass, a clearance or a shot every four seconds, every one of which used
/// to be drawn as a man running normally while the ball left him of its own
/// accord. A player turned to face what he had hit and never moved a boot.
#[derive(Clone, Copy)]
struct Kick {
    /// Where he is in the swing: −1 at the top of the backswing, 0 at
    /// contact, +1 at the end of the follow through.
    ///
    /// One signed number rather than a phase and a flag, because a kick is one
    /// continuous movement through the ball and the pose curve is easier to
    /// read written that way.
    swing: f32,
    /// How hard, 0..1, from the speed the ball leaves at. A five-yard pass and
    /// a shot from the edge of the box are not the same action.
    power: f32,
    /// Which boot: −1 left, +1 right.
    foot: f32,
    /// 0..1 ramp on the whole swing from the instant it was armed.
    ///
    /// The backswing is normally the length of the lookahead window, but only
    /// half the time: a ball already rolling masks the jump in speed until
    /// closer to contact, and measured over a real match one kick in ten is
    /// found with a single frame's warning. Without a ramp of its own those
    /// ones snap a leg into position in 16 ms. This is an animation blend and
    /// nothing else — it is not pretending to know anything the recording has
    /// not said.
    blend: f32,
    /// Where he hit it, flattened and normalised — he opens up to the ball
    /// before he strikes it, not after.
    direction: Vec3,
    /// And what with.
    kind: Strike,
}

/// The moment the ball is about to be struck, as the recording describes it —
/// before anybody has been blamed for it.
#[derive(Clone, Copy)]
pub struct Contact {
    /// Where the boot, head or hands will meet it.
    pub at: Vec3,
    /// How fast it will leave when they do.
    pub velocity: Vec3,
    /// Seconds of match time until then. Counts down as the playhead closes
    /// on it, and is what puts the swing in the right place: the pose is a
    /// function of this rather than of a clock the viewer starts, so it stays
    /// correct when the replay is scrubbed or run at 8x.
    pub delay: f32,
    /// What the geometry of the moment says it is. Read off the ball alone —
    /// how high it is being met and where on the pitch — because nothing in
    /// the recording says who is doing what, and the two answers that are not
    /// a kick are both unmistakable in the ball's own track.
    pub kind: Strike,
}

/// A kick that is about to happen, once it has an owner.
///
/// A footballer takes a backswing. By the time the ball is moving there is
/// nothing left to draw but the follow-through, so the strike that drives the
/// animation is read *ahead* of the playhead rather than off the current frame
/// — which the viewer can do for free, because the whole recording is already
/// in memory. See [`crate::replay::Track::position_ahead`].
#[derive(Clone, Copy)]
pub struct Impact {
    /// Who swings. The player nearest the ball at the moment it is struck,
    /// which measured over a real match is a median of 0.24 m away.
    pub by: u32,
    pub contact: Contact,
}

/// A ball about to arrive at a goalkeeper, and what he does about it.
///
/// The same idea as [`Impact`] and for the same reason: the recording is
/// already in memory, so the arrival can be read AHEAD of the playhead and the
/// hands can start going to the ball before it gets there — which is what
/// reacting to something means. Read off the ball's own track rather than off
/// the keeper's state, because the track is exact about both the moment and
/// the place, and the pose needs both.
///
/// This is the save that is not a dive, and it is most of them: measured over
/// a recorded match, **84% of the balls that arrive at a keeper at pace arrive
/// at one who never leaves the ground**, and every one of them used to be
/// drawn as a ball stopping dead at a man with his arms by his sides.
#[derive(Clone, Copy)]
pub struct Save {
    /// Seconds of match time until the ball is closest to him. Counts down
    /// as the playhead closes on it, exactly as [`Contact::delay`] does, so
    /// the reach arrives with the ball at any playback speed.
    pub delay: f32,
    /// Where it will be at that moment, in world space.
    pub at: Vec3,
    /// Whether it stays with him: gathered, rather than pushed away. Read a
    /// quarter of a second past the arrival, where the ball has either
    /// stopped on him or gone.
    pub held: bool,
}

/// The state of a ball in the air, kept so its rotation can be derived from
/// the whole flight rather than from one frame of it.
#[derive(Clone, Copy)]
pub struct Flight {
    /// Heading it was struck on, as `atan2(x, z)`.
    heading: f32,
    /// Seconds of match time since, and the smoothed sidespin read off the
    /// bend so far.
    elapsed: f32,
    sidespin: f32,
}

/// The rotation the ball is carrying, read back out of the path it takes.
///
/// The recording holds positions and nothing else — no spin, no owner — so the
/// ball's rotation is derived here the same way a player's stride is derived
/// from the ground he covers. Without it the ball is a painted sphere sliding
/// through the air on a frozen orientation: the one object on the pitch that
/// never looks alive, and the more so for being the one everybody is watching.
///
/// Three things put rotation on it, and all three are in the trajectory:
///
/// * **Rolling.** A ball on the deck turns at exactly `v / r` about the axis
///   across its travel. Nothing to estimate — this one is not a model.
/// * **Backspin.** A strike gets under the ball, so the rate is read off the
///   launch ANGLE: a driven pass carries little, a ball scooped up under the
///   laces carries a lot, and the same act produces both the loft and the
///   rotation.
/// * **Sidespin.** The engine curls a ball with a Magnus force,
///   `a = C·(ω × v)`, so the bend in the recorded path *is* the rotation that
///   caused it and can be inverted for it.
pub struct BallSpin;

impl BallSpin {
    /// Magnus coefficient in `a = C·(ω × v)`, SI. The engine's own — see
    /// `SpinModel::MAGNUS_COEFF` in the core crate. It is only used here to
    /// run the relation backwards, so the two have to be the same number.
    const MAGNUS: f32 = 0.0039;
    /// Ground speed, in metres per second, below which a ball is not really
    /// rolling — it is being nudged about at somebody's feet, and spinning it
    /// up reads as jitter rather than as motion.
    const CREEP: f32 = 0.35;
    /// Backspin a strike leaves on the ball, as a fraction of the rate it
    /// would be turning at if it were rolling at the same speed: floor for a
    /// ball hit flat, and how much more it picks up as the launch goes
    /// vertical.
    const BACKSPIN: (f32, f32) = (0.12, 0.55);
    /// Ceiling on any single axis, rad/s. 90 is about fourteen turns a second,
    /// past anything a human puts on a football, so it only ever catches an
    /// estimate that has run away.
    const MAX_RATE: f32 = 90.0;
    /// Rotation bleeds off slowly in flight — a struck ball is still turning
    /// when it arrives. Per SECOND of match time, matching the engine's own
    /// `SpinModel::DECAY_PER_TICK` of 0.9997 over its hundred ticks.
    const AIR_DECAY: f32 = 0.97;
    /// Seconds of flight before the bend is worth reading. The recording is
    /// quantised to 0.1 units horizontally and re-sampled every 30 ms, so the
    /// frame-to-frame turn is mostly noise; the total turn over a baseline
    /// this long is not.
    const BEND_WINDOW: f32 = 0.10;
    /// Seconds for the sidespin estimate to take up a new reading, and for a
    /// landing ball to swap flight rotation for rolling contact.
    const BEND_RESPONSE: f32 = 0.18;
    const GRIP_RESPONSE: f32 = 0.06;
    /// And for a ball that has been gathered or trapped to give its rotation
    /// up.
    const SETTLE_RESPONSE: f32 = 0.10;

    /// Rotation of a ball rolling on the turf: no slip, so the contact patch
    /// stands still and `ω = v / r` about the axis across the direction of
    /// travel.
    ///
    /// Against the DRAWN radius, not the regulation one. The viewer's ball is
    /// half again as big so it survives the broadcast distance
    /// ([`Actors::BALL_RADIUS`]), and what the eye checks is whether the
    /// surface it can see is keeping pace with the grass under it.
    fn rolling(velocity: Vec3) -> Vec3 {
        let flat = Vec3::new(velocity.x, 0.0, velocity.z);
        let speed = flat.length();
        if speed < Self::CREEP {
            return Vec3::ZERO;
        }
        match flat.try_normalize() {
            Some(heading) => Vec3::Y.cross(heading) * (speed / Actors::BALL_RADIUS),
            None => Vec3::ZERO,
        }
    }

    /// Rotation a strike leaves on the ball, from the velocity it left with.
    fn struck(velocity: Vec3) -> Vec3 {
        let speed = velocity.length();
        let Some(heading) = Vec3::new(velocity.x, 0.0, velocity.z).try_normalize() else {
            return Vec3::ZERO;
        };
        // Sine of the launch angle: 0 flat along the deck, 1 straight up.
        let climb = (velocity.y / speed.max(1e-3)).clamp(0.0, 1.0);
        let fraction = Self::BACKSPIN.0 + Self::BACKSPIN.1 * climb;
        // Backspin runs against the rolling sense: the top of the ball turns
        // back into the direction of travel, which is what holds it up.
        heading.cross(Vec3::Y) * (speed / Actors::BALL_RADIUS * fraction).min(Self::MAX_RATE)
    }

    /// Sidespin, inverted out of how far the flight has bent since it was
    /// struck.
    ///
    /// `a = C·ω·|v|` for a rotation about the vertical, and a path bending at
    /// `dθ/dt` at speed `|v|` has lateral acceleration `|v|·dθ/dt` — so the
    /// speed cancels and the whole estimate is the turn rate over the Magnus
    /// coefficient. Which is a division by 0.0039, so the baseline it is
    /// measured over has to be long enough to be signal.
    fn sidespin(turned: f32, elapsed: f32) -> f32 {
        if elapsed < Self::BEND_WINDOW {
            return 0.0;
        }
        (turned / elapsed / Self::MAGNUS).clamp(-Self::MAX_RATE, Self::MAX_RATE)
    }

    /// Exponential catch-up over `response` seconds, framerate independent.
    fn approach(response: f32, delta: f32) -> f32 {
        1.0 - (-delta / response).exp()
    }
}

pub struct Actors;

impl Actors {
    /// A match ball is 22 cm across. This one is half again as big: at the
    /// distance a broadcast camera sits, a regulation ball is four pixels.
    pub(crate) const BALL_RADIUS: f32 = 0.16;
    /// Width of the shadow on the turf, in metres.
    const FOOTPRINT: f32 = 1.32;
    /// How far up a standing body its shadow's middle lands, as a share of its
    /// height. Low, because the legs are the wider half of a footballer's
    /// outline and so carry most of the shadow with them.
    const SHADOW_CENTRE: f32 = 0.42;
    /// And how far the disc floats above the turf, in metres — clear of the
    /// grass and the paint.
    const SHADOW_LIFT: f32 = 0.018;
    /// Ground speed, in metres per second, that counts as flat out.
    pub(crate) const SPRINT: f32 = 6.0;
    /// Above this a player faces where they are going; below it they turn to
    /// watch the ball, which is what footballers standing still actually do.
    const MOVING: f32 = 1.1;
    /// No footballer covers ground this fast. Anything quicker is a seek, a
    /// substitution or a restart, and has to be cut to rather than run.
    const TELEPORT: f32 = 25.0;
    /// And the same for the ball, which is struck rather than run and needs a
    /// higher bar. Measured over a real match, the hardest strike in the
    /// recording is about 40 m/s and the next population up starts at 50 —
    /// see `Track::TELEPORT_SPEED`, which holds the same line for the drawn
    /// position.
    const BALL_TELEPORT: f32 = 45.0;
    /// How close the ball has to be to count as struck by this player — within
    /// a stride of him.
    const STRIKE_REACH: f32 = 1.7;
    /// And how fast it has to be leaving. Below this it is a touch, a trap or
    /// a ball rolling past, none of which a player opens his body up for.
    const STRUCK: f32 = 7.0;
    /// Seconds of match time a player stays turned toward what he just hit —
    /// the follow through, before he picks his running line back up.
    const STRIKE_HOLD: f32 = 0.45;
    /// And how fast a ball has to be leaving to count as having been KICKED
    /// rather than merely touched.
    ///
    /// Lower than [`Actors::STRUCK`], which decides whether he opens his body
    /// up to it, because the two questions are different. Turning to face a
    /// ball you have nudged two metres would be wrong; not moving your leg
    /// when you nudged it would be worse. Measured over a real match, a
    /// strike leaves at 8.6 m/s at the tenth percentile and 38 at the
    /// ninetieth, so most of the range this scales over is real football.
    const TOUCHED: f32 = 4.5;
    /// Ball speed, in metres per second, that counts as hit as hard as anybody
    /// hits one — the top of the swing. Measured p90 of a recorded match.
    const HAMMERED: f32 = 38.0;
    /// Height, in metres, above which a ball is being met with a head.
    ///
    /// A footballer is drawn 1.79 m to the crown with his eyes at 1.66, so a
    /// ball struck above this is above the shoulder of anybody standing under
    /// it — which leaves a header, and nothing else that happens often enough
    /// to draw. Under it sits the whole of the volley range, including the
    /// chest-high ones, and those really are kicks.
    const HEADED: f32 = 1.45;
    /// How close to a touchline a DEAD ball has to be struck from to be a
    /// throw-in, in metres, and how slowly it has to have been travelling
    /// first.
    ///
    /// The engine takes a throw-in from `AwaitedRestart::SPOT_INSET` inside
    /// the line — six game units, 75 cm — so this is that spot with room
    /// either side rather than a guess. Nothing else in football is played
    /// from a standing start out there.
    ///
    /// ⚠ **It was 0.45 m, derived from an inset of two units.** The inset
    /// moved to six when the ball started RUNNING OUT of play instead of
    /// being written onto its spot: the thrower now walks the ball back in
    /// from the run-off and has to finish inside the line, and `Arrive`'s
    /// own 3-unit deadzone is what sets how far inside that ends up being.
    /// At 0.45 m every throw-in was drawn as a boot instead.
    const TOUCHLINE_REACH: f32 = 1.3;
    const DEAD_BALL: f32 = 0.6;
    /// How far ahead of the playhead the viewer goes looking for the next
    /// kick, as a number of probes of the recording's own 30 ms sample step.
    ///
    /// Five of them, so the backswing gets 150 ms. A footballer's leg is
    /// behind him well before the ball moves, so this is the whole difference
    /// between a kick and a man standing still while the ball leaves him. It
    /// cannot be much longer: two players contesting a loose ball are both
    /// within reach of it, and the longer the warning the more often the wrong
    /// one starts swinging.
    const WINDUP_STEPS: u32 = 5;
    const PROBE: f64 = 30.0;
    /// Seconds of backswing, which is the same number in the units the pose
    /// wants it in.
    const WINDUP: f32 = Self::WINDUP_STEPS as f32 * Self::PROBE as f32 / 1000.0;
    /// How much faster the ball has to be going after the moment than before
    /// it for that moment to be a kick. A ball already in flight keeps its
    /// speed; a ball that has just been hit multiplies it.
    const IMPACT_RATIO: f32 = 1.6;
    /// Seconds of match time the follow through takes to play out, and how
    /// long a player then keeps any of it.
    const FOLLOW_THROUGH: f32 = 0.30;
    /// And how long the swing takes to take over from the run cycle once it is
    /// armed. See [`Kick::blend`].
    const KICK_ONSET: f32 = 0.06;
    /// Distance, in metres, inside which the ball counts as at a player's
    /// feet. Measured: somebody is within 1.4 m of a slow ball in 72% of
    /// frames, and the man who has it is a median of 6 cm from it.
    const AT_HIS_FEET: f32 = 1.15;
    /// Ball speed above which nobody is dribbling it — it has been played.
    const LOOSE: f32 = 9.0;
    /// Acceleration, in metres per second squared, that reads as a player
    /// going as hard as he can — driving off the mark or pulling up short.
    ///
    /// The recording is quantised to 1.25 cm and resampled every 30 ms, so a
    /// frame-to-frame acceleration is mostly noise (the raw p90 is 14 m/s²,
    /// which no human produces). It is smoothed over [`Actors::DRIVE_RESPONSE`]
    /// before being measured against this.
    const DRIVING: f32 = 4.5;
    const DRIVE_RESPONSE: f32 = 0.26;
    /// Radians per second of the standing-still cycle: a weight shift roughly
    /// every three and a half seconds.
    const IDLE_RATE: f32 = 1.8;
    /// Turn rate, in radians per second, that counts as changing direction as
    /// hard as a footballer can. Sets the scale for the lean into it.
    const HARD_TURN: f32 = 4.0;
    /// **The fastest a player may yaw, in radians per second, standing still
    /// and at a full sprint.**
    ///
    /// The rig had no limit at all: `TURN_RESPONSE` is a proportional
    /// catch-up, so a heading error of 180° was corrected at something like
    /// 1400 deg/s on the frame it appeared. That is the other half of "they
    /// spin around like toy tops" — the first half being the noisy bearing
    /// they were chasing (see [`Actors::travel`]) — and it is the half no
    /// amount of smoothing can fix, because it is a claim about bodies
    /// rather than about data: **a man moving at six metres a second cannot
    /// pivot, he has to arc.**
    ///
    /// Standing, a footballer turns on the spot in about a second — call it
    /// 400 deg/s at the peak of it. At a sprint he is bounded by the tightest
    /// arc he can hold: `ω = v / r`, and a turn radius under three and a half
    /// metres at 6 m/s is not something anybody runs. That is ~100 deg/s.
    /// Interpolated on speed, so easing off to change direction — which is
    /// what a real player does — buys back the agility.
    pub(crate) const PIVOT_RATE: (f32, f32) = (7.0, 1.8);
    /// **The band in which a footballer stops side-stepping and turns his
    /// legs onto his run**, in metres a second.
    ///
    /// A side-shuffle is a low-speed gait. It is what a defender jockeys in
    /// and what a goalkeeper covers his line with, it is comfortable at
    /// about a walk, and past two and a half metres a second nobody uses it
    /// — because a gait that never crosses its feet cannot lengthen its step
    /// without widening its base by the same amount, and the base is already
    /// as wide as the step. Above the band a footballer travelling across
    /// himself opens his hips and RUNS.
    ///
    /// The rig had no such band. Every frame in which travel disagreed with
    /// heading was drawn as a keeper's shuffle at full amplitude — and
    /// [`Actors::PIVOT_RATE`] guarantees that disagreement every time
    /// anybody changes direction at speed, because a body cannot pivot
    /// under itself, it has to arc. Measured off the pose: a man arcing
    /// round at five and a half metres a second was drawn with his feet
    /// 1.45 m apart, his knees folded to 53° and his crown 44 cm below
    /// standing, at thirteen steps a second. That is the report — *"they
    /// move sideways like invalids"* — and it was the common case, not an
    /// edge one.
    const OPEN_UP: (f32, f32) = (1.2, 2.6);
    /// …and how far round his legs will go, in radians.
    ///
    /// A crossover is a real rotation of a real pelvis, not a licence to
    /// draw the legs anywhere. Eighty degrees is past what a hip and a
    /// lumbar spine hold on their own — but the chest this is measured
    /// against is itself mid-turn, since the heading is already coming round
    /// onto the run, so what is drawn is the two meeting rather than the
    /// hips going the whole way alone. Whatever is left over after it is
    /// still a side-step, which is right: a crossover run is not a man
    /// running sideways, it is a man running at an angle and reaching the
    /// last of it across himself.
    const OPEN_LIMIT: f32 = 1.40;
    /// How the opening fades out as he starts travelling BACKWARDS, as a
    /// share of his course.
    ///
    /// Going backwards he does not open up, he backpedals — a gait this rig
    /// already has, and one that reverses through `course.y`. And the fade
    /// is what makes the bearing safe to read at all: a bearing wraps at
    /// ±π, so without it the opening would have to choose between turning
    /// him ninety degrees one way and ninety the other at the instant his
    /// course crossed dead astern. That is a coin toss, and a coin toss is
    /// where an animation tears.
    ///
    /// At 1.0 the fade is `ease(1 + cos θ)`: full while he is going
    /// anywhere forward of square, and reaching zero — flat, with no slope
    /// left in it — exactly at the wrap. Nothing downstream ever sees the
    /// discontinuity, and nothing has to be clamped to hide it.
    ///
    /// ⚠ **Not a narrower fade.** Cut off at 123°, as this first was, the
    /// frames past a right angle kept a residual side-step of 40° at a full
    /// stride — 0.4 m of lateral step, every bit as bad as no opening at
    /// all — and there are a lot of them: measured over a real recording,
    /// **4.5% of the frames an outfielder is running in have him more than
    /// 100° off his own facing**, a man reversing at a sprint with the
    /// heading still most of a second behind him.
    const OPEN_BACKING: f32 = 1.0;
    /// **How far off square he has to be going before a side-step stops
    /// being the gait**, in radians of bearing in his own frame.
    ///
    /// A shuffle is what a man does when he is square to something and
    /// means to stay that way — a defender jockeying a winger, a keeper on
    /// his line. Nothing else on the pitch keeps its feet from crossing. So
    /// below the first figure (29°) he is simply walking or running at an
    /// angle and his legs go with him whatever his pace; past the second
    /// (83°) he is genuinely across himself and the pace decides.
    const SQUARE_ON: (f32, f32) = (0.50, 1.45);
    /// Inside this range, in metres, he is watching the ball and nothing
    /// else; past `SCANNING` he is looking around him instead. The two
    /// leave a wide cross-over so a player drifting in and out of the play
    /// does not snap between the two.
    const WATCHING: f32 = 18.0;
    const SCANNING: f32 = 42.0;
    /// How far off his own facing that sweep carries his head, in radians.
    /// Small: this is a man checking his shoulder, not one looking away
    /// from the game.
    const SCAN_SWEEP: f32 = 0.5;
    /// How far a player will turn his head off his own facing before he has
    /// to turn his shoulders with it.
    const NECK: f32 = 1.05;
    /// Stride length: how far a player travels per step, walking and per extra
    /// metre per second of pace. A sprinter's stride tops out around 2 m.
    const STRIDE: (f32, f32, f32) = (0.75, 0.13, 2.10);
    /// The most steps a second anybody takes. See [`Actors::stride_of`].
    const TOP_CADENCE: f32 = 6.0;
    /// Seconds for a player to come round onto a new heading.
    const TURN_RESPONSE: f32 = 0.13;
    /// Seconds for the run cycle to take up a change of pace.
    const PACE_RESPONSE: f32 = 0.18;
    /// Seconds over which the direction of travel is smoothed.
    ///
    /// Has to outlast a sample boundary — the recorder emits every 30 ms
    /// and drops samples until the player has covered 3.75 cm, so a jog
    /// crosses one every 2-4 frames — while staying short enough that a
    /// real change of direction still reads as one. See `Actors::travel`.
    const TRAVEL_RESPONSE: f32 = 0.18;
    /// …and for a reaction to a goal to arrive. Slower than either of the
    /// two above on purpose: shoulders drop and arms go up over about half a
    /// second, and snapping either on the frame the ball crosses the line
    /// reads as a cut rather than as a man reacting.
    const MOOD_RESPONSE: f32 = 0.45;
    /// Gap between a player's boots and their name plate, as a fraction of how
    /// tall they are drawn. Measuring it against the player rather than in
    /// metres or in pixels is what keeps the plate clear of the boots at any
    /// distance and under any camera: a world-space offset shrinks to nothing
    /// as the rig flattens, and a pixel offset crowds whoever is nearest.
    const LABEL_GAP: f32 = 0.15;
    /// The band of heights, in metres, that means a ball is in a goalkeeper's
    /// gloves.
    ///
    /// Nothing in the recording says who owns the ball, let alone whether it
    /// has been picked up — but it does not have to. The engine carries a
    /// gathered ball at 1.15 m and every other ball at the height its own
    /// physics put it, so a ball sitting in this band ON TOP OF a keeper is a
    /// ball in his hands and nothing else can be.
    ///
    /// Wider than it needs to be on purpose. The exact carry height is one
    /// constant in the engine and this is a viewer reading its consequences
    /// from the far side of a recording; a band that only just fits it would
    /// break silently the day it moves.
    const GLOVE_HEIGHT: (f32, f32) = (0.85, 1.45);
    /// And how close to him, horizontally, in metres. The engine snaps a
    /// gathered ball to its owner's exact position, so this is really only
    /// tolerance for the moment of the claim; anything larger starts catching
    /// shots that pass him at chest height.
    const GLOVE_REACH: f32 = 0.55;
    /// And how close a ball has to be to an OUTFIELDER, in metres, before he
    /// counts as carrying it rather than as standing near it.
    ///
    /// A quarter of the keeper's reach, because it is answering a much
    /// narrower question. `GoalCelebration::move_ball` writes the carrier's
    /// own x and y into the ball, so the only gap between the two is the
    /// recorder's quantisation — 1.25 cm on each axis, plus whatever a frame
    /// of interpolation adds while he is walking. Anything looser starts
    /// picking up open play, where a ball at chest height inside half a metre
    /// of a man is on its way past him.
    const CARRIED_REACH: f32 = 0.14;
    /// Seconds of match time to take the ball up into the hold, and to let it
    /// go again. The release is quicker: he throws it.
    const CRADLE_RESPONSE: (f32, f32) = (0.14, 0.06);
    /// Above this, in metres, the ball is in the air rather than on the deck.
    /// The engine's own roll/fly split sits at 0.1 m; this is under it so a
    /// ball is spinning as it flies rather than as it lands.
    const AIRBORNE: f32 = 0.05;
    /// Height, in metres, at which a man counts as having left the turf.
    ///
    /// Two quanta of the recording, which rounds height to a centimetre.
    /// Low because it has to be: the engine's dive apex starts at 0.16 m — a
    /// save along the floor is still a man leaving the ground, just not by
    /// much — and the frame he goes is the frame the run cycle has to stop.
    const AIRBORNE_FEET: f32 = 0.02;
    /// Seconds of match time for the run cycle to give way once he leaves
    /// the ground. Barely a ramp at all: he is committed the moment his
    /// boots are off the grass, and a diving keeper still windmilling his
    /// legs is the single thing that would give the whole animation away.
    const SPRAWL_ATTACK: f32 = 0.05;
    /// Seconds of match time to open out from the gathered take-off to full
    /// stretch.
    ///
    /// The number the dive was missing. A recorded dive is airborne for
    /// 390–660 ms with a median of 450, and the extension is roughly the
    /// first half of it: he leaves the ground with his elbows in and is at
    /// full stretch around the apex. Everything after that is a man waiting
    /// to land, which is exactly how it should read.
    const EXTENSION: f32 = 0.22;
    /// Seconds of match time he stays down after the landing before he
    /// starts getting up, and how long the impact itself takes to settle
    /// him from full stretch into a heap.
    ///
    /// The hold is cut short by any real ground speed — measured, half of
    /// all landings are followed by the keeper covering five to seven metres
    /// inside a second, and dragging a sprawled body along behind that is
    /// worse than standing him up early.
    const SPRAWL_HOLD: f32 = 0.30;
    const GROUNDING: f32 = 0.16;
    /// Ground speed, in metres per second, at which the hold above is gone
    /// entirely. A jog: below it he is gathering himself where he landed and
    /// the hold stands in full, above it the recording has him up and going
    /// somewhere and no amount of choreography is worth dragging a sprawled
    /// body along behind him.
    const SPRAWL_URGENCY: f32 = 3.2;
    /// How much longer a BEATEN keeper stays on the grass, as a multiple of
    /// the hold above.
    ///
    /// A keeper who has dived and saved it is up almost at once — he has the
    /// ball and there is a game to restart. One who has dived and watched it
    /// go in does not get up: he lies there, and that is the single most
    /// recognisable image in the sport. The engine holds him still for four
    /// seconds after a goal (`GoalCelebration`), so nothing is fighting this;
    /// without it the sprawl expired 0.3 s after he landed and he stood
    /// calmly up out of the dive that had just cost his team a goal.
    const BEATEN_HOLD: f32 = 9.0;
    /// …and how much of that hold is this particular keeper's, either way.
    ///
    /// Two keepers do not take a goal for the same length of time, and a
    /// fixed hold is one more thing that makes the two men in the picture
    /// the same man. Off the same salted hash everything else about him
    /// comes from, so it is the same keeper's reaction every time.
    const BEATEN_SPREAD: f32 = 0.45;
    /// Seconds of match time to get back up. There is no attack time on the
    /// way out: the engine's own arc is the take-off, and going up is
    /// already as fast as gravity says. Landing is where a keeper takes his
    /// time.
    const SPRAWL_RECOVERY: f32 = 0.42;
    /// **Where a beaten keeper's recovery STOPS**, as a share of the dive
    /// still owed.
    ///
    /// He does not lie on the turf and then stand up. He comes up as far as
    /// his knees and stays there, head down, and gets to his feet when he
    /// has to go and fetch the ball — which is the picture, and which the
    /// rig drew as one smooth rotation from flat to vertical because the
    /// recovery was a single exponential with nothing in the middle of it.
    ///
    /// A floor rather than a pause: the decay simply runs to this instead of
    /// to zero, so it is one expression and nothing has to know it is in a
    /// second phase. Lifted by any ground he covers, so the moment the
    /// recording sets him off toward his own net he stands up on his own.
    const KNEELING: f32 = 0.42;
    /// **How far round onto his front he turns getting up**, as a share of
    /// the way there.
    ///
    /// Nobody stands up sideways. A body lying on its side rotates onto its
    /// front, gets a hand and a knee under itself and pushes — and the rig
    /// used to bring him straight back up about the axis he went down on,
    /// which is a plank on a hinge and no part of it is a movement a person
    /// can make. Not the whole way: he is on his front and his side at once
    /// for most of it, which is what a man propping himself on one arm is.
    ///
    /// Applied to the DIRECTION of the topple and not its size, so it costs
    /// nothing at either end: flat out he has not started and upright there
    /// is nothing left to turn.
    const ROLLS_OVER: f32 = 0.80;
    /// How much of the rise the roll takes: eased over the first two thirds
    /// of it, because he is on his front well before he is on his feet.
    const ROLL_EARLY: f32 = 1.6;
    /// **A goalkeeper's idle cycle**, in seconds of match clock: how long
    /// between gestures, how long each one holds, and the ramp either end.
    ///
    /// Offset per player off his own hash, so no two keepers are ever doing
    /// the same thing at the same moment — which is the whole point, and the
    /// trap that [`Complexion::carriage`] exists to avoid.
    const GESTURE_CYCLE: f32 = 15.0;
    const GESTURE_HOLD: f32 = 2.4;
    const GESTURE_STANCE: f32 = 4.2;
    const GESTURE_RAMP: f32 = 0.55;
    /// Seconds of match time over which the launch angle is read. Long
    /// enough to survive a noisy first step off the line, short enough that
    /// it is still the take-off being measured and not the flight.
    const LAUNCH_WINDOW: f32 = 0.10;
    /// How far the body goes over at full stretch, in radians.
    ///
    /// Not quite the full quarter turn, because a keeper lands on his
    /// shoulder and hip rather than flat on his back — but close, and much
    /// closer than it was. What gets scaled against it is how flat he left
    /// the ground: see [`PlayerActor::climb`]. A dive off the floor reaches
    /// about 79° of this and a running leap at a cross about 43°, which is
    /// the difference between the two saves and is measured rather than
    /// assumed.
    pub(crate) const SPRAWL_ANGLE: f32 = 1.38;
    /// The band, in radians, over which a landing stops being a landing and
    /// becomes a fall. See [`PlayerActor::topple`].
    ///
    /// Measured off the recording through the same `flat` latch the topple
    /// uses: a keeper going up for a cross launches at about 45° and reaches
    /// 43° of topple — he lands on his feet and gets straight up. A dive off
    /// the floor launches at 10–15° and reaches 64–79° — his feet are out at
    /// hip height with nothing under them, and there is no landing from that
    /// which is not a fall. The two figures are what the ends of this band
    /// are, and between them the settle eases rather than switching, because
    /// there is no angle at which a body abruptly starts falling over.
    pub(crate) const GOES_OVER: (f32, f32) = (0.75, 1.22);
    /// How far off the ground, in metres, counts as fully airborne for the
    /// arms. The engine's own leap apex runs 0.34 m for a poor jumper to
    /// 0.75 for a good one, so this reaches full stretch partway up rather
    /// than only at the very top of the best keeper's jump.
    const REACH_HEIGHT: f32 = 0.35;
    /// And for an outfielder, who is heading a ball rather than saving one.
    /// Measured: twelve outfield players leave the turf in a recorded match,
    /// up to 1.13 m.
    const JUMP_HEIGHT: f32 = 0.40;
    /// Distance from the ball, in metres, inside which a keeper is fully on
    /// his toes and beyond which he is simply standing.
    ///
    /// A goalkeeper is not a man waiting: he is set whenever the ball is
    /// anywhere near his goal and relaxed when it is not, and that alone is
    /// most of what separates one from an outfielder standing in a different
    /// coloured shirt. The near figure is roughly the top of the penalty
    /// area, the far one the edge of the middle third.
    const SET_RANGE: (f32, f32) = (16.5, 34.0);
    /// Eye height, in metres, for working out how far up or down he is
    /// looking. Crown is [`Physique::STATURE`]; eyes sit a little under it.
    const EYE: f32 = 1.66;
    /// Ground speed, in metres a second, above which a man is taking whole
    /// steps rather than shifting his weight.
    ///
    /// Deliberately low. The stride amplitude used to come off
    /// `speed / SPRINT`, which at a walking pace asks the legs for a
    /// quarter of the step the body is taking — a footballer strolling
    /// across his own box was being slid over the grass. Whatever he is
    /// doing above half a metre a second, he is doing it by putting one
    /// foot in front of the other.
    const STEPPING: f32 = 0.45;
    /// How long the gait takes to answer a change of direction, in seconds.
    /// A body does not swap the direction of its side-step inside a frame,
    /// and the smoothing is most of what turns the transition between
    /// running, shuffling and dropping back into one continuous movement
    /// rather than three animations with cuts between them.
    const COURSE_RESPONSE: f32 = 0.22;
    /// How fast a GOALKEEPER has to be going before he turns his back on
    /// the play and runs, in metres a second.
    ///
    /// Below it he stays square and shuffles or backpedals, which is what a
    /// keeper does with the overwhelming majority of the ground he covers:
    /// measured over a recorded match, 87% of his frames are under 2.5 m/s,
    /// and of the frames where he is moving with the ball inside 40 m,
    /// **47% are travelling backwards and 19% across himself**. Drawn as a
    /// man who turns to face wherever he is going, that is a goalkeeper who
    /// spends the build-up looking at his own posts.
    /// **He OPENS UP rather than switching.** Below the first figure he is
    /// dead square to the ball and side-steps; past the second he is running
    /// where he is going like anybody else; between them his hips turn a
    /// share of the way and his neck keeps his eyes on the ball, which is
    /// exactly what a keeper is coached to do and what the rig's existing
    /// `look` was already able to draw.
    ///
    /// ⚠ **A side-step is not available at any speed.** A two-legged
    /// alternating lateral gait moves each foot by the whole ground the body
    /// covers in a cycle, so at four metres a second his feet would be
    /// swinging through two metres of separation — which is not a shuffle,
    /// it is a bounding sideways run, and it is what the first render of
    /// this looked like. The band above is what keeps the lateral component
    /// inside the speeds a human can actually shuffle at, and it does it
    /// without a switch: the course rotates continuously, so there is no
    /// frame at which the gait changes.
    pub(crate) const SQUARE_UP: (f32, f32) = (1.1, 2.8);
    /// How far round from the ball a keeper will ever turn while it is
    /// live and near his goal, in radians. 1.75 = 100°.
    ///
    /// Past this he has turned his BACK on it, and a goalkeeper does not
    /// do that — not while recovering to his line, not while getting back
    /// for a ball played over him with the play still in front. He runs
    /// side-on and looks over his shoulder; [`Actors::NECK`] reaches 60° of
    /// the 100°, so at the limit the ball is at the very edge of his
    /// vision, which is what that looks like.
    ///
    /// It is the ceiling on the opening, not a replacement for it: a
    /// keeper whose run and ball are the same way never reaches it.
    ///
    /// ⚠ It is also what makes [`Actors::KEEPER_OPEN_UP`] necessary. Once
    /// his chest is held at 100° his course is nearly all lateral, and
    /// drawn as a side-step at four metres a second that is a man bounding
    /// sideways with his feet two metres apart. The legs have to open onto
    /// the run even though the chest does not — which is a crossover run,
    /// and is precisely how a keeper recovers.
    pub(crate) const SHOULDER: f32 = 1.75;
    /// Where a KEEPER's legs open onto his course, in metres a second —
    /// [`Actors::OPEN_UP`] for everybody else.
    ///
    /// It starts where his shuffle ends ([`Actors::SQUARE_UP`]), because
    /// that is the same claim about the same gait seen from two sides: up
    /// to 2.8 m/s a lateral step is a shuffle and his feet stay square; past
    /// it they cannot, and he crosses them. The width of the band is the
    /// outfielder's, since it is the same body.
    pub(crate) const KEEPER_OPEN_UP: (f32, f32) =
        (Self::SQUARE_UP.1, Self::SQUARE_UP.1 + (Self::OPEN_UP.1 - Self::OPEN_UP.0));
    /// How far ahead of the playhead a keeper's save is read, in seconds of
    /// match time, and at what interval.
    ///
    /// Measured off a real recording: a ball that ends up arriving at a
    /// keeper at pace passes 10 m from him a median of 900 ms earlier and
    /// 6 m from him 540 ms earlier. There is a great deal of time to fill,
    /// and until now none of it was drawn — the arms did not move until the
    /// ball was already in the 0.85–1.45 m hold band, i.e. until after it
    /// had stopped.
    const SAVE_WINDOW: f64 = 900.0;
    const SAVE_STEPS: u32 = 18;
    /// How far he will reach for one standing, in metres. Roughly a glove
    /// past his own fingertips either side.
    const SAVE_REACH: f32 = 2.45;
    /// …and how high. Above this he is leaving the ground for it, which is
    /// a leap and not a save on his feet.
    const SAVE_CEILING: f32 = 2.35;
    /// A ball slower than this is one he collects, not one he saves: the
    /// cradle already draws that, and reaching at every back-pass would be
    /// worse than reaching at none.
    const SAVE_STRUCK: f32 = 6.5;
    /// How long the reach takes to open out, in seconds of match time. A
    /// human reaction is about this, and the recording gives him five to
    /// nine times as much warning.
    const SAVE_ONSET: f32 = 0.30;
    /// …and how long the follow-through takes to give it back.
    const SAVE_RELEASE: f32 = 0.26;
    /// Where the aim reads 0: a ball at chest height is neither high nor
    /// low. Below `GATHER` and above `OVERHEAD` it saturates.
    const SAVE_GATHER: f32 = 0.05;
    const SAVE_OVERHEAD: f32 = 2.15;
    /// Ball speed under which the arrival counts as GATHERED rather than
    /// pushed away, measured a quarter of a second after contact.
    ///
    /// Generous, because it is not the discriminator — the HEIGHT is. A
    /// keeper who has claimed it is very often already moving with it (the
    /// engine sends him straight into his distribution), so a slow ball is
    /// a poor test of a catch; a ball still sitting in the
    /// [`Actors::GLOVE_HEIGHT`] band a quarter of a second after arriving at
    /// thirty metres a second is in his gloves and cannot be anything else.
    /// This only rules out one passing through the band in mid-flight.
    const SAVE_GATHERED: f32 = 7.0;
    /// …and the speed under which it has simply stopped, wherever it is.
    const SAVE_SMOTHERED: f32 = 2.0;

    pub fn spawn(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
        config: Res<ViewerConfig>,
        faces: Res<Faces>,
    ) {
        let parts = BodyParts::new(&mut meshes);
        let wardrobe = Wardrobe::new(
            &mut materials,
            &mut images,
            &config,
            &BodyParts::face_layout(),
        );
        // Every player takes the field wearing the face this crate painted
        // for him, and the real ones are sent for now: a photograph that
        // arrives in the third minute is repainted onto the head he is
        // already wearing. See [`crate::portrait`].
        commands.insert_resource(Portraits::fetch(&config, &wardrobe));

        let patch = meshes.add(Plane3d::default().mesh().size(1.0, 1.0));

        for player in &config.players {
            let actor = commands
                .spawn((
                    PlayerActor::new(player.id, player.is_goalkeeper(), player.is_home),
                    // Height and build are separate axes, so the squad is a
                    // spread of physiques rather than one model at twenty-two
                    // sizes. `splat` gave everybody an identical shape.
                    Transform::from_scale(Vec3::new(
                        Complexion::build(player.id),
                        Complexion::height(player.id),
                        Complexion::build(player.id),
                    )),
                    Visibility::Hidden,
                ))
                .id();

            // The contact shadow does not turn with him: see [`Silhouette`].
            commands.spawn((
                Silhouette { actor },
                Mesh3d(patch.clone()),
                MeshMaterial3d(wardrobe.shadow()),
                Transform::from_xyz(0.0, 0.018, 0.0),
                Visibility::Hidden,
            ));
            Footballer::assemble(
                &mut commands,
                actor,
                &parts,
                &wardrobe.outfit(player),
                player.is_goalkeeper(),
            );

            let mut plate = commands.spawn((
                PlayerLabel { actor },
                // The trailing newline is what puts the state on its own line
                // below the name when the debug span is attached.
                Text::new(Self::label_for(player, config.debug)),
                TextFont {
                    font: FontSource::Handle(faces.face_for(&player.last_name)),
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.96, 1.0)),
                // Tight enough to read as an outline rather than as a second
                // copy of the name: these plates sit over grass, over white
                // paint and over each other.
                TextShadow {
                    offset: Vec2::splat(1.0),
                    color: Color::srgba(0.0, 0.0, 0.0, 0.85),
                },
                TextLayout::justify(Justify::Center),
                // Width and alignment belong here rather than in
                // [`Self::place_labels`], which only ever wrote the same two
                // constants back over themselves — and a `Node` written on
                // every frame is a UI layout pass on every frame.
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(88.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Visibility::Hidden,
            ));

            if config.debug {
                plate.with_child((
                    PlayerStateLabel { id: player.id },
                    TextSpan::default(),
                    TextFont {
                        font_size: FontSize::Px(11.0),
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 0.93, 0.4)),
                ));
            }
        }

        let ball_material = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(Textures::football(&mut images)),
            // Trimmed from 0.25. The emissive is there to keep the ball
            // legible against the turf, but it is added uniformly — at 0.25
            // it lifted the black panels to grey and the ball was white
            // again, which defeats the point of painting it.
            emissive: LinearRgba::rgb(0.08, 0.08, 0.08),
            perceptual_roughness: 0.6,
            ..default()
        });
        commands.spawn((
            BallActor,
            Mesh3d(meshes.add(Sphere::new(Self::BALL_RADIUS).mesh().uv(16, 10))),
            MeshMaterial3d(ball_material),
            Transform::from_xyz(0.0, Self::BALL_RADIUS, 0.0),
            Visibility::Hidden,
        ));

        commands.spawn((
            BallShadow,
            Mesh3d(patch),
            MeshMaterial3d(wardrobe.shadow()),
            Transform::from_xyz(0.0, 0.02, 0.0),
            Visibility::Hidden,
        ));
    }

    /// Drives every player and the ball from the recording at the current
    /// playhead. Players with no samples around `now` were substituted off (or
    /// have not come on yet) and simply vanish.
    pub fn follow_playhead(
        playback: Res<Playback>,
        time: Res<Time>,
        loader: Res<ChunkLoader>,
        mut tracks: ResMut<ReplayTracks>,
        mut ball_state: ResMut<BallState>,
        mut players: Query<(&mut PlayerActor, &mut Transform, &mut Visibility)>,
        mut ball: Query<(&mut Transform, &mut Visibility), (With<BallActor>, Without<PlayerActor>)>,
        mut shadow: Query<
            (&mut Transform, &mut Visibility),
            (With<BallShadow>, Without<PlayerActor>, Without<BallActor>),
        >,
    ) {
        let now = playback.time_ms;
        // Seconds of MATCH time this frame covered. Everything derived from
        // the recording — velocity, rotation, the hold ramp — is measured
        // against this rather than against the wall clock, so it all means the
        // same thing at 1x and at 8x.
        let delta = (playback.speed.max(0.1) * time.delta_secs()).max(1e-4);
        // No samples here can mean two things. If the chunk covering `now` has
        // arrived, the entity really is off the pitch. If it has not, the data
        // is simply still in flight — freeze everyone where they stand rather
        // than blanking the pitch every time the viewer scrubs.
        let covered = loader.covers(now);

        let ball_position = tracks
            .ball
            .position_at(now)
            .map(|[x, y, z]| Field::to_world(x, y, z));

        // The next kick, found by walking the recording ahead of the playhead.
        // Blanked on a seek: there is no continuity to predict across one.
        let coming = if playback.seeked {
            None
        } else {
            Self::next_impact(&mut tracks.ball, now)
        };

        // Who has it in his gloves, where those gloves are, and who is nearest
        // it — both now, for the man on the ball, and at the moment it is
        // struck, for whoever is about to swing. All resolved in the same pass
        // that places the players, because every answer depends on where they
        // have just been put.
        let mut holder: Option<(u32, Vec3)> = None;
        let mut nearest: Option<(u32, f32)> = None;
        let mut striker: Option<(u32, f32)> = None;
        for (mut actor, mut transform, mut visibility) in &mut players {
            let position = tracks
                .players
                .get_mut(&actor.id)
                .and_then(|track| track.position_at(now));
            match position {
                Some([x, y, z]) => {
                    let world = Field::to_world(x, y, z);
                    transform.translation.x = world.x;
                    transform.translation.z = world.z;
                    // Height is kept OFF the actor transform on purpose. The
                    // contact shadow and the team ring hang off the actor and
                    // belong flat on the grass, so the rise is handed to the
                    // `Carriage` instead — which is also what stops it
                    // leaking into the ground speed `animate` reads out of
                    // consecutive positions.
                    actor.height = world.y;
                    *visibility = Visibility::Inherited;
                }
                None if covered => *visibility = Visibility::Hidden,
                None => {}
            }

            if *visibility == Visibility::Hidden {
                actor.arrival = None;
                continue;
            }
            // The save he is about to make on his feet, read ahead of the
            // playhead — see [`Save`]. Only ever a keeper's: nobody else on
            // the pitch puts his hands out at a ball, and the reach is the
            // one thing that would look absurd on an outfielder.
            actor.arrival = if actor.is_goalkeeper && !playback.seeked {
                Self::next_arrival(&mut tracks.ball, now, transform.translation)
            } else {
                None
            };
            let boots = Vec2::new(transform.translation.x, transform.translation.z);
            if let Some(ball) = ball_position {
                let range = boots.distance(Vec2::new(ball.x, ball.z));
                if nearest.is_none_or(|(_, best)| range < best) {
                    nearest = Some((actor.id, range));
                }
            }
            if let Some(contact) = coming {
                // Whoever will be standing over it. Not whoever is nearest the
                // ball NOW: over a hundred and fifty milliseconds a player
                // closing on a loose ball covers more than a metre, and the
                // man who ends up hitting it is often not the one closest to
                // it when the swing starts.
                let range = boots.distance(Vec2::new(contact.at.x, contact.at.z));
                if striker.is_none_or(|(_, best)| range < best) {
                    striker = Some((actor.id, range));
                }
            }

            let (Some(ball), None) = (ball_position, holder) else {
                continue;
            };
            // Two players on a pitch ever have the ball in their hands, and
            // they hold it in quite different places.
            let reach = Vec2::new(
                ball.x - transform.translation.x,
                ball.z - transform.translation.z,
            )
            .length();
            let hold = if actor.is_goalkeeper {
                let gathered = Self::in_his_hands(reach, ball.y, true);
                // In the throwing hand once he has started the throw, and
                // otherwise at his chest if he is on his feet or out at the
                // end of the stretch if he is not — the same blend the arms
                // themselves take, so the ball is wherever his gloves are
                // rather than wherever they would have been.
                //
                // The throw is the same omission as the throw-in's: the cradle
                // is where he holds a ball he has GATHERED, and a keeper
                // winding one up has taken it out of there and put it behind
                // his ear. Left at the chest it sat there while his arm went
                // over the top of it, and then the ball set off on its own.
                gathered.then(|| match actor.throwing_hand() {
                    Some(side) => Physique::palm(side, actor.gait()),
                    None => Physique::CRADLE.lerp(Physique::catch(actor.lead()), actor.extended()),
                })
            } else {
                // A throw-in is taken with the ball in BOTH HANDS, and the
                // recording cannot say so: the engine leaves it on the grass
                // where the throw is measured from (`Ball::carry_height` only
                // lifts a ball a keeper has gathered), so the thrower swept
                // both arms over his head while the ball sat by his boots and
                // then left of its own accord.
                //
                // Read off the rig rather than from a constant, because the
                // hold travels with the arms — see [`Physique::hands`]. It
                // lasts as long as the wind-up does and lets go at contact,
                // where the ramp on the hold walks the ball back onto its own
                // recorded flight inside a few frames.
                if actor.throwing_in() {
                    Some(Physique::hands(actor.gait()))
                } else {
                    // …and the OTHER outfielder who has it in his hands: the
                    // man carrying it back to the centre circle after a goal.
                    // `GoalCelebration::move_ball` parks it on his own
                    // coordinate at chest height and leaves it there for the
                    // whole walk back, so this was drawn as a ball floating
                    // inside his ribcage while he trudged along with his hands
                    // on his head. Reported as it looked.
                    //
                    // A far tighter radius than the keeper's glove reach, and
                    // that is the whole test: a ball played at somebody's
                    // chest in open play is PASSING THROUGH, and is never
                    // sitting still on his own centreline. See
                    // [`Actors::CARRIED_REACH`].
                    Self::in_his_hands(reach, ball.y, false).then_some(Physique::CRADLE)
                }
            };
            if let Some(hold) = hold {
                // Carried out through the CARRIAGE, which is where the topple
                // and the lift live, and only then out through the actor's own
                // height and build. Reading the hold point straight off the
                // actor — as this did — left the ball at the sternum of an
                // upright man while the keeper it belonged to was horizontal
                // the better part of a metre away, on every diving catch in
                // the match.
                //
                // A frame behind, because this frame's topple is not written
                // until `animate` has run: sixteen milliseconds of a
                // four-hundred-millisecond dive, and worth it for not making
                // the ball's position depend on system ordering.
                let (pitch, roll) = actor.topple();
                let gloves = Carriage::placed(pitch, roll, actor.lift()).transform_point(hold);
                holder = Some((actor.id, transform.transform_point(gloves)));
            }
        }

        ball_state.on_pitch = ball_position.is_some();
        ball_state.nearest = nearest;
        // Only if somebody is actually close enough to have hit it. A ball
        // that speeds up with nobody near it is a deflection, a restart or the
        // engine putting it back on the centre spot, and none of those is a
        // man swinging a leg.
        ball_state.impact = match (coming, striker) {
            (Some(contact), Some((by, range))) if range < Self::STRIKE_REACH => {
                Some(Impact { by, contact })
            }
            _ => None,
        };
        if let Some(world) = ball_position {
            // Ball velocity, in metres per second of match time, read off the
            // RAW recorded path — never off the drawn position, which is
            // displaced while a keeper has it. Blanked on a seek, where the
            // jump is the playhead moving and not the ball.
            // Nothing across a TELEPORT is a velocity either. The engine puts
            // the ball down for a restart, a catch or a block rather than
            // moving it there, and reading a speed off that hands `BallSpin`
            // a two-hundred-metre-a-second launch: it spins the ball up to its
            // own cap, opens a `Flight` on a heading that never existed, and
            // holds both for the next second of play. The recording is not
            // describing a ball that travelled — see `Track::teleported`.
            ball_state.velocity = match ball_state.previous {
                Some(previous) if !playback.seeked => {
                    let step = (world - previous) / delta;
                    if step.length() > Self::BALL_TELEPORT {
                        ball_state.spin = Vec3::ZERO;
                        ball_state.flight = None;
                        Vec3::ZERO
                    } else {
                        step
                    }
                }
                _ => Vec3::ZERO,
            };
            ball_state.previous = Some(world);
            ball_state.position = world;

            // Ramp the hold. One ramp, two consumers — the ball's position
            // here and the keeper's arms in `animate` — so the gloves can
            // never close a frame before or after the ball arrives in them.
            if let Some((keeper, gloves)) = holder {
                ball_state.held_by = Some(keeper);
                ball_state.cradle_offset = gloves - world;
            } else {
                ball_state.held_by = None;
            }
            let response = if holder.is_some() {
                Self::CRADLE_RESPONSE.0
            } else {
                Self::CRADLE_RESPONSE.1
            };
            let wanted = f32::from(holder.is_some());
            ball_state.cradle += (wanted - ball_state.cradle)
                * if playback.seeked {
                    1.0
                } else {
                    BallSpin::approach(response, delta)
                };

            let drawn = if ball_state.cradle > 1e-3 {
                world + ball_state.cradle_offset * ball_state.cradle
            } else {
                world
            };
            Self::turn_ball(&mut ball_state, delta, playback.seeked);

            if let Ok((mut transform, mut visibility)) = ball.single_mut() {
                transform.translation = drawn + Vec3::Y * Self::BALL_RADIUS;
                transform.rotation = ball_state.rotation;
                *visibility = Visibility::Inherited;
            }
            if let Ok((mut transform, mut visibility)) = shadow.single_mut() {
                // Thrown along the light like everything else's — see
                // [`Actors::cast_shadows`]. A ball twenty metres up over the
                // penalty spot does not put its shadow on the penalty spot.
                let offset = Self::sun_throw() * drawn.y.max(0.0);
                transform.translation = Vec3::new(drawn.x + offset.x, 0.02, drawn.z + offset.y);
                // Fade and spread the patch with height, the way a real one does.
                let spread = 0.62 * (1.0 + (drawn.y * 0.08).min(0.7));
                transform.scale = Vec3::new(spread, 1.0, spread);
                *visibility = Visibility::Inherited;
            }
        } else if covered {
            ball_state.previous = None;
            ball_state.velocity = Vec3::ZERO;
            ball_state.spin = Vec3::ZERO;
            ball_state.flight = None;
            ball_state.held_by = None;
            ball_state.cradle = 0.0;
            if let Ok((_, mut visibility)) = ball.single_mut() {
                *visibility = Visibility::Hidden;
            }
            if let Ok((_, mut visibility)) = shadow.single_mut() {
                *visibility = Visibility::Hidden;
            }
        }
    }

    /// Is the ball in this man's HANDS, given how far it is from him across
    /// the grass and how high off it?
    ///
    /// The recording says nothing about who is holding what, so both hands on
    /// a football are inferred from where it is sitting — and it is a strong
    /// inference, because a ball at chest height sitting still on top of a
    /// player is not doing anything else. Two radii, for the two ways it
    /// happens: `Ball::carry_height` parks a keeper's gather at 1.15 m and he
    /// holds it out at arm's length through a dive, while
    /// `GoalCelebration::move_ball` writes the carrier's own coordinate into
    /// it at 1.05 m and leaves it there for the whole walk back.
    ///
    /// Its own function so the rule can be tested rather than living inside a
    /// system that needs a world stood up around it.
    fn in_his_hands(reach: f32, height: f32, is_goalkeeper: bool) -> bool {
        let allowed = if is_goalkeeper {
            Self::GLOVE_REACH
        } else {
            Self::CARRIED_REACH
        };
        reach < allowed && height > Self::GLOVE_HEIGHT.0 && height < Self::GLOVE_HEIGHT.1
    }

    /// Finds the next moment the ball is struck, within the backswing window
    /// ahead of the playhead: where it will be hit, how fast it will leave,
    /// and how long until it happens.
    ///
    /// Walks the window a probe at a time rather than sampling its far end,
    /// for two reasons. The delay has to be a real countdown — the swing is a
    /// function of it, and a fixed one would leave every player's leg stuck at
    /// the top of the backswing until the ball had already gone. And a kick is
    /// a LOCAL jump in speed: comparing each step against the one before it
    /// catches a first-time volley, where the ball was already travelling
    /// quickly and simply changed direction and got faster.
    ///
    /// Stateless on purpose. Nothing is remembered between frames, so the
    /// swing is right wherever the playhead is put — scrubbed, reversed or
    /// running at 8x — instead of being right only if it was watched from the
    /// beginning.
    fn next_impact(ball: &mut Track, now: f64) -> Option<Contact> {
        let at = |ball: &mut Track, t: f64| {
            ball.position_ahead(t)
                .map(|[x, y, z]| Field::to_world(x, y, z))
        };
        // One probe BEHIND the playhead, so the first step in the window has
        // something to be a jump from.
        let mut previous = at(ball, now - Self::PROBE)?;
        let mut here = at(ball, now)?;
        let mut before = (here - previous).length() / (Self::PROBE as f32 / 1000.0);

        for step in 1..=Self::WINDUP_STEPS {
            let Some(next) = at(ball, now + step as f64 * Self::PROBE) else {
                return None;
            };
            let velocity = (next - here) / (Self::PROBE as f32 / 1000.0);
            let leaving = velocity.length();
            if leaving > Self::TOUCHED
                && leaving < Self::TELEPORT
                && leaving > before * Self::IMPACT_RATIO
            {
                // `here` is where the ball sat at the start of the step that
                // showed the jump, which is where the boot met it.
                let delay = (step - 1) as f32 * Self::PROBE as f32 / 1000.0;
                return Some(Contact {
                    at: here,
                    velocity,
                    delay,
                    kind: Self::strike_kind(here, before),
                });
            }
            previous = here;
            here = next;
            before = (here - previous).length() / (Self::PROBE as f32 / 1000.0);
        }
        None
    }

    /// **How long a step this player takes at this pace, and how far his hips
    /// have to swing to carry it.**
    ///
    /// One function, because the two answers have to agree and the whole of
    /// the reported gliding is what happened when they did not: the phase
    /// advanced by ground covered — one half-cycle per stride — while the
    /// amplitude came off `speed / SPRINT`, a completely unrelated quantity
    /// that at a walking pace asks the legs for a third of the distance the
    /// body is travelling.
    ///
    /// The amplitude is the angle at which a straight leg's foot sweeps
    /// `stride / PI` of ground either side of the hip, which is the amplitude
    /// at which a sinusoid's mid-stance speed equals the speed of the body
    /// over it. Faded in over the bottom half-metre a second so a man
    /// standing still is not frozen half way through a step.
    ///
    /// Both numbers carry [`Complexion::stride`], so cadence stays a property
    /// of the player and the amplitude now follows it — before, a long-strided
    /// man took long steps with the same short leg swing as everybody else.
    pub(crate) fn stride_of(id: u32, speed: f32, course: Vec2) -> (f32, f32) {
        let running = ((Self::STRIDE.0 + Self::STRIDE.1 * speed) * Complexion::stride(id))
            .clamp(Self::STRIDE.0 * 0.8, Self::STRIDE.2 * 1.2);
        // Nobody takes running strides across himself — see
        // [`Joint::shortening`], which the pose reads too so the two ends
        // cannot disagree about how big a step this is.
        let stride = running * Joint::shortening(course);
        // **…and nobody takes more than about six steps a second.**
        //
        // The shortening above is a RATIO of a running stride, which is the
        // right shape — a side-step is a fraction of a stride whatever pace
        // you are going at — and the wrong thing to leave unbounded. A third
        // of a stride at six metres a second is still 0.47 m, and 0.47 m
        // steps at six metres a second is thirteen a second: two thirds of a
        // step between one frame and the next, which does not read as quick
        // feet, it reads as a leg strobing. Elite sprinters peak at about
        // five.
        //
        // Only ever bites on a shortened stride. A man running FORWARDS is
        // under four a second at any pace a footballer reaches, so the cap
        // is invisible to the twenty-one players out of twenty-two this rig
        // spends its time on.
        let stride = stride.max(speed / Self::TOP_CADENCE);
        let demanded = (stride / PI / Physique::LEG).clamp(0.0, 0.95).asin();
        (stride, demanded * Self::ease(speed / Self::STEPPING))
    }

    /// **How far a player turns his legs off his chest onto the way he is
    /// going**, in radians, positive to his right.
    ///
    /// The whole of the outfielder's lateral gait, and the thing this rig
    /// had no representation of. See [`crate::body::Gait::open`] for what it
    /// is drawn as and [`Actors::OPEN_UP`] for why it exists; this is the
    /// decision.
    ///
    /// **He turns his legs all the way onto his course** — up to
    /// [`Actors::OPEN_LIMIT`] — **and what holds him back is being SQUARE.**
    /// Both halves matter, and the second is the one that is easy to get
    /// wrong. A part-opening does not help: the lateral step a shuffle takes
    /// is `duty × stride × course.x`, and the stride grows back as the
    /// course straightens, so the two cancel almost exactly. Measured across
    /// the range, a man walking at 1 m/s takes the same 0.25 m side-step at
    /// 30° off his facing as at 90° — which is why halving the residual
    /// bought nothing and a man strolling diagonally was drawn with his feet
    /// swinging through 0.8 m and his crown 11 cm down. The opening has to
    /// commit or not bother.
    ///
    /// So what is left is: when is a side-step the real gait? When he is
    /// SQUARE to something and slow. That is a defender jockeying and a
    /// keeper on his line, and it is genuinely how they move. A man walking
    /// thirty degrees off his facing is not doing that; he is walking, and
    /// he is one of the twenty who is looking at the ball rather than at his
    /// feet (an outfielder under [`Actors::MOVING`] faces the ball, which is
    /// where most of these courses come from at all).
    ///
    /// ⚠ **A goalkeeper opens up LATER, not never**, and the difference is
    /// the whole of how he recovers.
    ///
    /// This returned zero for him flat, on the reasoning that he stays
    /// square and shuffles because that is his job and
    /// [`Actors::SQUARE_UP`] does his opening by turning his whole HEADING.
    /// That was true while the heading went all the way round: at 2.8 m/s
    /// he was simply pointed at his run, his course was straight ahead, and
    /// there was nothing lateral left to draw.
    ///
    /// It stopped being true with [`Actors::SHOULDER`], which is there
    /// because turning his whole heading meant turning his BACK on the ball
    /// — the reported bug. His chest now holds at 100° off the run, so his
    /// course stays nearly all lateral at any speed, and a lateral course
    /// drawn with square feet at four metres a second is a man bounding
    /// sideways.
    ///
    /// A keeper recovering at pace does what anybody does: he crosses his
    /// legs and runs, and keeps his chest and his eyes on the ball. So the
    /// legs open exactly as everybody else's do, from
    /// [`Actors::KEEPER_OPEN_UP`] — which begins where his shuffle ends, so
    /// the keeper on his line and the jockeying defender both keep every
    /// lateral term untouched, and only the man who is genuinely running
    /// crosses over.
    pub(crate) fn opening(speed: f32, course: Vec2, keeper: bool) -> f32 {
        let length = course.length();
        if length < 1e-4 {
            return 0.0;
        }
        let bearing = course.x.atan2(course.y);
        // How far round he goes if he commits: onto the course, and no
        // further — `OPEN_LIMIT · sin θ` overshoots a bearing of its own
        // below about 75°, and turning his legs PAST the way he is going
        // would draw a side-step back in on the other side.
        let toward = (Self::OPEN_LIMIT * bearing.sin()).clamp(-bearing.abs(), bearing.abs());
        // …and what holds him back. Both terms have to be up for a shuffle:
        // square AND slow.
        let square = Self::ease(
            (bearing.abs() - Self::SQUARE_ON.0) / (Self::SQUARE_ON.1 - Self::SQUARE_ON.0),
        );
        let band = if keeper {
            Self::KEEPER_OPEN_UP
        } else {
            Self::OPEN_UP
        };
        let strolling = 1.0 - Self::ease((speed - band.0) / (band.1 - band.0));
        // Going backwards he backpedals instead — and this is also what
        // makes `bearing` safe to read at all, since it reaches zero, flat,
        // exactly where the bearing wraps. See [`Actors::OPEN_BACKING`].
        let forward = Self::ease((course.y / length + Self::OPEN_BACKING) / Self::OPEN_BACKING);
        toward * (1.0 - square * strolling) * forward
    }

    /// …and the course that is left once his legs have gone there: the same
    /// vector, read in the frame the legs are in rather than the frame the
    /// chest is.
    ///
    /// This is what makes the opening one continuous rotation rather than a
    /// second gait with a switch into it. Every lateral term in the rig is
    /// multiplied by `course.x` somewhere, so rotating the course toward
    /// straight-ahead collapses the side-step, the splay, the pelvic list
    /// and the shortened stride together and in proportion — and a jockeying
    /// defender at a walk, whose opening is zero, keeps every one of them
    /// untouched.
    ///
    /// Length is preserved, which matters: the course is deliberately NOT
    /// renormalised while a player is turning (see `Actors::animate`), and
    /// that shortening is what shrinks his step mid-turn.
    pub(crate) fn underfoot(course: Vec2, open: f32) -> Vec2 {
        let (sin, cos) = open.sin_cos();
        Vec2::new(
            course.x * cos - course.y * sin,
            course.x * sin + course.y * cos,
        )
    }

    /// The next ball to arrive within a keeper's reach at pace, read ahead of
    /// the playhead exactly as [`Actors::next_impact`] reads a kick — and for
    /// the same reason. A keeper who starts moving his hands when the ball
    /// reaches them is not making a save; he is being hit by a ball.
    ///
    /// Returns the CLOSEST APPROACH rather than the first probe inside his
    /// reach, and computes it on the segment rather than at the sample: at
    /// fifty milliseconds a probe a shot travels a metre and a half, so
    /// sampling alone would put his hands most of a metre from the ball. The
    /// path between two recorded samples is a straight line, so the closest
    /// point on it is exact.
    ///
    /// Stateless, like the kick: nothing is remembered between frames, so it
    /// is right wherever the playhead is put.
    fn next_arrival(ball: &mut Track, now: f64, keeper: Vec3) -> Option<Save> {
        let at = |ball: &mut Track, t: f64| {
            ball.position_ahead(t)
                .map(|[x, y, z]| Field::to_world(x, y, z))
        };
        let flat = |point: Vec3| Vec2::new(point.x - keeper.x, point.z - keeper.z).length();

        let step = Self::SAVE_WINDOW / Self::SAVE_STEPS as f64;
        let mut previous = at(ball, now)?;
        let opening = flat(previous);
        let mut best: Option<Save> = None;
        let mut closest = Self::SAVE_REACH;

        for probe in 1..=Self::SAVE_STEPS {
            let Some(here) = at(ball, now + probe as f64 * step) else {
                break;
            };
            let leg = here - previous;
            let travelling = leg.length() / (step as f32 / 1000.0);
            // Where on this leg he is nearest it. Taken across the grass,
            // so a ball dropping vertically past him does not read as
            // arriving early.
            let ground = Vec2::new(leg.x, leg.z);
            let along = if ground.length_squared() > 1e-6 {
                (Vec2::new(keeper.x - previous.x, keeper.z - previous.z).dot(ground)
                    / ground.length_squared())
                .clamp(0.0, 1.0)
            } else {
                0.0
            };
            let meeting = previous + leg * along;
            previous = here;

            // A ball he collects rather than saves — the cradle already
            // draws that — and one the recorder has picked up and put down
            // somewhere else, which is not a trajectory at all.
            if !(Self::SAVE_STRUCK..Self::BALL_TELEPORT).contains(&travelling) {
                continue;
            }
            // Over this and he is leaving the ground for it, which is a leap
            // and has its own pose.
            if meeting.y > Self::SAVE_CEILING {
                continue;
            }
            let range = flat(meeting);
            if range >= closest {
                continue;
            }
            closest = range;
            best = Some(Save {
                delay: (probe - 1) as f32 * step as f32 / 1000.0 + along * step as f32 / 1000.0,
                at: meeting,
                held: false,
            });
        }

        // **It has to be COMING TO HIM.** A keeper throwing the ball out
        // satisfies everything above — it is at his hands, it is quick, it
        // is under the bar — and the only thing that separates the two is
        // which way the gap is going. Without this he reached for every
        // delivery he made.
        let mut save = best.filter(|save| opening - flat(save.at) > Self::SAVE_REACH * 0.25)?;

        // Catch or parry, read off what the ball does next rather than
        // guessed: a quarter of a second on, it is either sitting in his
        // gloves or it has gone. `Ball::carry_height` parks a gathered ball
        // at 1.15 m and nothing else on a pitch is a football at chest
        // height on top of a man, which is the same inference
        // [`Actors::in_his_hands`] is built on.
        let settled = now + (save.delay * 1000.0) as f64 + 250.0;
        if let (Some(first), Some(second)) = (at(ball, settled), at(ball, settled + 150.0)) {
            let travelling = (second - first).length() / 0.15;
            let in_his_gloves = first.y > Self::GLOVE_HEIGHT.0 && first.y < Self::GLOVE_HEIGHT.1;
            // Either it is up at chest height on top of him, or it has simply
            // stopped. Both are balls he has, and neither is a ball he has
            // pushed away; requiring the first alone missed every gather he
            // made on the deck.
            save.held = travelling < Self::SAVE_SMOTHERED
                || (in_his_gloves && travelling < Self::SAVE_GATHERED);
        }
        Some(save)
    }

    /// What the ball's own track says it is about to be hit with.
    ///
    /// Nothing in the recording names the player, let alone the limb — but for
    /// the two strikes that are not kicks it does not have to, because both
    /// are unmistakable in the geometry of the moment. A ball met a metre and
    /// a half up is met with a head: nothing else of a footballer is up there.
    /// And a ball sitting STILL a hand's width inside a touchline and then
    /// leaving at pace is a throw-in — no other restart in football is taken
    /// from out there, and none of them is taken from a standstill on the
    /// paint.
    ///
    /// Deliberately conservative in both directions: everything that is not
    /// clearly one of the two falls back to a kick, which is what the
    /// overwhelming majority of the fourteen strikes a minute actually are.
    fn strike_kind(at: Vec3, before: f32) -> Strike {
        let dead = before < Self::DEAD_BALL;
        let touchline = Field::HALF_WIDTH - at.z.abs() < Self::TOUCHLINE_REACH;
        if dead && touchline && at.y < Self::AIRBORNE {
            Strike::ThrowIn
        } else if at.y > Self::HEADED {
            Strike::Head
        } else {
            Strike::Boot
        }
    }

    /// Advances the ball's rotation for this frame, from where its own path
    /// says it should be turning. See [`BallSpin`].
    fn turn_ball(ball: &mut BallState, delta: f32, seeked: bool) {
        if seeked {
            // The jump is the playhead's, not the ball's: there is no
            // trajectory across it to read a rotation from.
            ball.spin = Vec3::ZERO;
            ball.flight = None;
            return;
        }

        if ball.held_by.is_some() {
            // In the gloves. Whatever it was doing, it has stopped.
            //
            // Keyed off the holder and not off the ramp: the recorded ball
            // climbs a metre into his hands inside a single frame, which is a
            // launch by every test below, and waiting for the ramp to cross a
            // threshold lets it spin up hard for a tenth of a second first.
            ball.spin *= 1.0 - BallSpin::approach(BallSpin::SETTLE_RESPONSE, delta);
            ball.flight = None;
        } else if ball.position.y > Self::AIRBORNE {
            // A ball with no measurable heading — the top of a vertical lob,
            // or the first frame after a chunk landed — has no trajectory to
            // read. It keeps turning as it was; the estimate picks up again
            // the moment it is going somewhere.
            if let Some(travel) = Vec3::new(ball.velocity.x, 0.0, ball.velocity.z).try_normalize() {
                let heading = travel.x.atan2(travel.z);
                match &mut ball.flight {
                    // Already up. Hold the rotation it left with, less the
                    // little the air takes back, and keep refining the
                    // sidespin from how far it has bent since.
                    Some(flight) => {
                        flight.elapsed += delta;
                        let turned = (heading - flight.heading + PI).rem_euclid(TAU) - PI;
                        let reading = BallSpin::sidespin(turned, flight.elapsed);
                        flight.sidespin += (reading - flight.sidespin)
                            * BallSpin::approach(BallSpin::BEND_RESPONSE, delta);
                        ball.spin *= BallSpin::AIR_DECAY.powf(delta);
                        ball.spin.y = flight.sidespin;
                    }
                    // Just left the deck — or a boot, or a bounce. Whatever
                    // put it up there decided the rotation, and the launch
                    // velocity is the only record of it.
                    None => {
                        if ball.velocity.length() > BallSpin::CREEP {
                            ball.spin = BallSpin::struck(ball.velocity);
                        }
                        ball.flight = Some(Flight {
                            heading,
                            elapsed: 0.0,
                            sidespin: 0.0,
                        });
                    }
                }
            }
        } else {
            // On the grass. Rolling contact takes over within a few
            // hundredths of a second of touching down, which is what turns a
            // backspun ball round and checks it.
            ball.flight = None;
            let rolling = BallSpin::rolling(ball.velocity);
            ball.spin += (rolling - ball.spin) * BallSpin::approach(BallSpin::GRIP_RESPONSE, delta);
        }

        // Integrated in world space, so the ball keeps turning about a fixed
        // axis rather than about one that its own rotation drags round with
        // it — pre-multiplied for that reason. Renormalised every frame: this
        // runs a few hundred thousand times over a full replay.
        if ball.spin.length_squared() > 1e-6 {
            ball.rotation = (Quat::from_scaled_axis(ball.spin * delta) * ball.rotation).normalize();
        }
    }

    /// Turns each player's change of position into a heading and a stride, then
    /// poses their limbs from it.
    ///
    /// The recording holds positions and nothing else — no facing, no speed, no
    /// animation track — so everything a footballer's body does is derived here
    /// from the ground they cover. Driving the stride by distance rather than by
    /// time is what stops the feet from skating: however fast the playhead is
    /// running, a player still takes one step per stride length of turf.
    pub fn animate(
        playback: Res<Playback>,
        ball: Res<BallState>,
        aftermath: Res<Aftermath>,
        time: Res<Time>,
        mut actors: Query<(&mut PlayerActor, &mut Transform, &Visibility)>,
        mut joints: Query<(&Joint, &mut Transform), Without<PlayerActor>>,
    ) {
        let delta = time.delta_secs().max(1e-4);
        // Exponential catch-up, framerate independent.
        let turn = 1.0 - (-delta / Self::TURN_RESPONSE).exp();
        let pace = 1.0 - (-delta / Self::PACE_RESPONSE).exp();
        // Slower than either, because a man's shoulders drop over about half
        // a second and his arms come up over about the same. Snapping the
        // mood on the frame the ball crosses the line reads as a cut.
        let mood = 1.0 - (-delta / Self::MOOD_RESPONSE).exp();

        for (mut actor, mut transform, visibility) in &mut actors {
            if *visibility == Visibility::Hidden {
                actor.previous = None;
                continue;
            }

            let position = transform.translation;
            let step = match actor.previous {
                Some(previous) if !playback.seeked => position - previous,
                _ => Vec3::ZERO,
            };
            actor.previous = Some(position);

            // How he took the last goal. Read first, because most of what
            // follows has to defer to it: a man reacting to a goal is not
            // watching the ball, is not set for a shot, and is not dribbling
            // anything. See [`Aftermath`].
            let wanted_despair = aftermath.despair(actor.is_home);
            let wanted_elation = aftermath.elation(actor.is_home);
            let settle = if playback.seeked { 1.0 } else { mood };
            actor.despair += (wanted_despair - actor.despair) * settle;
            actor.elation += (wanted_elation - actor.elation) * settle;
            // Nobody in football watches the ball after a goal — and for the
            // conceding side it is behind them in their own net, so the
            // stand-still rule that faces a slow player at it turned the
            // whole eleven round to stare into the goal as they walked out
            // of it. Reported as players "spinning round".
            let heedless = actor.despair.max(actor.elation) > Aftermath::NOTHING;

            // Playback speed belongs to the viewer, not to the player: divide
            // it back out or everybody sprints at 8x.
            let ground = step.length();
            let observed = ground / (delta * playback.speed.max(0.1));
            let (ground, observed) = if observed > Self::TELEPORT {
                (0.0, actor.speed)
            } else {
                (ground, observed)
            };
            // Driving off the mark, or pulling up. A footballer accelerating
            // is bent forward over his own feet and one stopping dead has his
            // heels out in front of him, and both are constant — a player
            // changes pace far more often than he changes direction.
            //
            // Read off the smoothing itself, and ahead of the line that
            // advances it: an
            // exponential filter's output climbs at exactly (input − output)
            // over its response time, so the gap between what he is doing this
            // frame and what he has been doing IS his acceleration. Dividing
            // the same gap by the frame time instead — which is the obvious
            // way to write it — makes the answer eleven times too big at
            // 60 fps and a different number again at any other rate.
            let urge =
                ((observed - actor.speed) / Self::PACE_RESPONSE / Self::DRIVING).clamp(-1.0, 1.0);
            actor.drive += (urge - actor.drive)
                * if playback.seeked {
                    1.0
                } else {
                    1.0 - (-delta / Self::DRIVE_RESPONSE).exp()
                };
            actor.speed += (observed - actor.speed) * pace;

            // **WHICH WAY HE IS ACTUALLY GOING.**
            //
            // Not one frame's displacement, which is what the heading used
            // to be read from and is the reported "they spin around like
            // toy tops". The recording quantises to 0.1 game units (1.25 cm)
            // and drops a sample until the player has covered 0.3 (3.75 cm),
            // so at 60 fps a single frame's step is *below the resolution of
            // the data it is derived from*: the direction between two
            // interpolated points carries an angular error of order
            // `atan(1.25 / 3.75)` — eighteen degrees — and it re-rolls every
            // time the playhead crosses a sample boundary, which at a jog is
            // every second or third frame.
            //
            // Measured over a real chunk (`churn::measure_turning`): a mean
            // of **136 deg/s of yaw across every player-frame, with 12.5% of
            // frames turning faster than a full revolution per second**, and
            // 89% of it in this branch. The players were not turning; the
            // arithmetic was.
            //
            // A body's heading follows its MOMENTUM. Smoothing the travel
            // vector — rather than the heading it produces — is the honest
            // fix: it bridges the sample boundaries the noise lives on,
            // shrinks through a genuine change of direction (which is
            // physically right, a man slows to turn), and leaves the
            // heading integrator free to be responsive.
            let travelling = if playback.seeked || ground <= 0.0 {
                Vec3::ZERO
            } else {
                Vec3::new(step.x, 0.0, step.z) / (delta * playback.speed.max(0.1))
            };
            let settling = if playback.seeked {
                1.0
            } else {
                1.0 - (-delta / Self::TRAVEL_RESPONSE).exp()
            };
            let was = actor.travel;
            actor.travel = was + (travelling - was) * settling;

            // Did he just hit it? The ball has to be leaving him at pace and
            // from within reach. Requiring it to be moving AWAY is what tells
            // a strike from a reception — a player taking a ball in is just as
            // close to just as fast a ball, and without the test he would spin
            // to face the way it arrived.
            //
            // Never for the keeper who is gathering it: the ball climbing a
            // metre into his gloves inside one frame is a huge upward velocity
            // pointing straight away from his boots, which is a strike by
            // every test here and by none in reality.
            let gathering = ball.held_by == Some(actor.id) || actor.carry > 1e-3;
            if ball.on_pitch && !playback.seeked && !gathering {
                let from_him = ball.position - position;
                let reach = Vec3::new(from_him.x, 0.0, from_him.z).length();
                let departing = ball.velocity.dot(from_him) > 0.0;
                if reach < Self::STRIKE_REACH && departing && ball.velocity.length() > Self::STRUCK
                {
                    if let Some(direction) =
                        Vec3::new(ball.velocity.x, 0.0, ball.velocity.z).try_normalize()
                    {
                        actor.strike = Some((direction, Self::STRIKE_HOLD));
                    }
                }
            }
            // Tick the hold down in match time, so a strike does not hold for
            // eight times as long when the replay is run at 8x.
            if let Some((_, remaining)) = &mut actor.strike {
                *remaining -= delta * playback.speed.max(0.1);
                if *remaining <= 0.0 || playback.seeked {
                    actor.strike = None;
                }
            }

            // The swing itself, which the recording has already told us is
            // coming. See [`Kick`] and [`Actors::next_impact`].
            let mine = ball.impact.filter(|impact| impact.by == actor.id);
            actor.swing_leg(mine, delta * playback.speed.max(0.1), playback.seeked);

            // And whether the ball is at his feet. Measured, somebody is
            // within a stride of a slow ball in 72% of frames, so this is the
            // normal state of one player on the pitch rather than a rarity —
            // which is exactly why the man with it should not run identically
            // to the twenty-one without it.
            // …and nobody is dribbling anything in the seconds after a goal,
            // however close he happens to be standing to a ball that is
            // sitting in the netting behind him.
            let dribbling = ball.on_pitch
                && !gathering
                && !heedless
                && ball.velocity.length() < Self::LOOSE
                && ball
                    .nearest
                    .is_some_and(|(id, range)| id == actor.id && range < Self::AT_HIS_FEET);
            actor.carrying +=
                (f32::from(dribbling) - actor.carrying) * if playback.seeked { 1.0 } else { pace };

            // Off his feet, and therefore diving. Straight off the recorded
            // height — the engine takes a keeper off the ground on a real
            // ballistic arc — and only ever for a keeper: twelve outfield
            // players leave the turf in a recorded match to head a ball, and
            // every one of them used to be drawn toppling sideways with both
            // arms over his head.
            let match_delta = delta * playback.speed.max(0.1);
            // Against the instantaneous pace as well as the smoothed one: a
            // keeper who was set and has just gone is still being caught up
            // with by his own average.
            let launch = actor.speed.max(observed);
            let airborne = actor.track_flight(match_delta, launch, observed, playback.seeked);
            // And which way, recomputed every frame he is up there — for
            // exactly the reason it looks as though it should be latched.
            //
            // The tip is expressed in his OWN frame, and that frame is still
            // turning: a diving keeper faces the ball, and at the instant of
            // take-off his heading is still most of the way round to it off
            // his approach run. Latch the decomposition at take-off and it is
            // nailed to an axis that then rotates out from under it, so the
            // topple swings across the goal with his shoulders — and a save
            // to his right gets drawn as a man falling on his face. His
            // travel does not change in flight, so recomputing against the
            // current heading is what actually holds the dive still in world
            // space.
            if airborne {
                let forward = Vec3::new(actor.heading.sin(), 0.0, actor.heading.cos());
                let right = Vec3::new(actor.heading.cos(), 0.0, -actor.heading.sin());
                if let Some(going) = Vec3::new(step.x, 0.0, step.z).try_normalize() {
                    actor.tip = Vec2::new(going.dot(right), going.dot(forward)) * actor.flat;
                }
            }

            let facing = Self::facing(&actor, &ball, position, step, gathering || heedless);
            let mut turn_signal = 0.0_f32;
            if let Some(facing) = Vec3::new(facing.x, 0.0, facing.z).try_normalize() {
                // Rotating about Y by `atan2(x, z)` carries +Z onto the facing,
                // and the model is built looking down +Z.
                let wanted = facing.x.atan2(facing.z);
                let swing = (wanted - actor.heading + PI).rem_euclid(TAU) - PI;
                let mut applied = swing * if playback.seeked { 1.0 } else { turn };
                // …but no faster than a body can. See `PIVOT_RATE`: the
                // catch-up above is proportional and therefore unbounded,
                // and the bound is the whole difference between a man
                // arcing round and a man rotating on the spot. Measured in
                // MATCH time so the cap is the same at 1x and at 8x.
                if !playback.seeked {
                    let match_delta = delta * playback.speed.max(0.1);
                    let eased = (actor.speed / Self::SPRINT).clamp(0.0, 1.0);
                    let ceiling = (Self::PIVOT_RATE.0
                        + (Self::PIVOT_RATE.1 - Self::PIVOT_RATE.0) * eased)
                        * match_delta;
                    applied = applied.clamp(-ceiling, ceiling);
                }
                actor.heading += applied;
                // In radians per second of match time, normalised against a
                // hard change of direction, so the lean is the same at any
                // frame rate or playback speed.
                let rate = applied / (delta * playback.speed.max(0.1));
                turn_signal = (rate / Self::HARD_TURN).clamp(-1.0, 1.0);
            }
            actor.turn += (turn_signal - actor.turn) * pace;
            transform.rotation = Quat::from_rotation_y(actor.heading);

            // **Which way he is going, relative to which way he is pointed.**
            //
            // For everybody who turns to face his run this is straight ahead
            // and every term it drives collapses to the plain forward cycle.
            // It exists for the man it does not collapse for: a goalkeeper
            // stays square to the play, and measured over a recorded match
            // 47% of the frames in which he is moving with the ball inside
            // forty metres are travelling BACKWARDS and 19% ACROSS himself.
            // Drawn as a forward run cycle, that is a man being slid about on
            // the grass — which is exactly how it was reported.
            //
            // Taken after this frame's turn, because the frame it is measured
            // in is the one the pose is built in.
            let forward = Vec3::new(actor.heading.sin(), 0.0, actor.heading.cos());
            let sideways = Vec3::new(actor.heading.cos(), 0.0, -actor.heading.sin());
            let going = Vec3::new(actor.travel.x, 0.0, actor.travel.z);
            let wanted_course = match going.try_normalize() {
                // Below a shuffle there is no course to speak of and the
                // amplitudes it drives are all but zero anyway; pointing it
                // forward keeps a standing player out of a half-step.
                Some(way) if actor.speed > Self::STEPPING * 0.5 => {
                    Vec2::new(way.dot(sideways), way.dot(forward))
                }
                _ => Vec2::Y,
            };
            // Smoothed, and deliberately NOT renormalised afterwards. A
            // change of direction takes a body time, and while it is
            // happening the course is genuinely shorter than one — which
            // shrinks the step, which is what a man turning does. Left
            // unsmoothed, a keeper reversing across his line swapped the
            // direction of his side-step inside a frame.
            let settle = if playback.seeked {
                1.0
            } else {
                1.0 - (-delta / Self::COURSE_RESPONSE).exp()
            };
            let was = actor.course;
            actor.course = (was + (wanted_course - was) * settle).clamp_length_max(1.0);

            // **And how much of that he takes with his legs turned rather
            // than with a side-step.**
            //
            // Taken off the SMOOTHED course, so it inherits the smoothing
            // above and needs none of its own — and taken here rather than
            // at the pose, because the stride model has to see the same
            // answer the legs are drawn at or the cadence and the amplitude
            // stop agreeing. See [`Actors::opening`].
            actor.open = Self::opening(actor.speed, actor.course, actor.is_goalkeeper);
            actor.underfoot = Self::underfoot(actor.course, actor.open);

            // Idle cycle runs on the clock, not on ground covered, because it
            // exists precisely for the player who is covering none.
            actor.idle = (actor.idle
                + delta * playback.speed.max(0.1) * Self::IDLE_RATE * Complexion::tempo(actor.id))
            .rem_euclid(TAU);
            // …and the MATCH clock, read rather than integrated, for the one
            // thing that is a function of when rather than of what: what a
            // goalkeeper does with the eighty minutes in which nothing
            // happens to him. See [`PlayerActor::gesturing`].
            actor.clock = (playback.time_ms * 1e-3) as f32;

            // Where he is looking. Clamped to what a neck can do — past that a
            // real player turns his whole body, which he is already doing.
            //
            // Not at a ball he is holding, for the same reason he does not
            // turn toward one: it is inside his own chest, so the bearing is
            // rounding error and the pitch of it is a man staring at his own
            // boots. He looks where he is facing, which is up the pitch he is
            // about to throw it to.
            let wanted_look = if ball.on_pitch && !gathering && !heedless {
                let to_ball = ball.position - position;
                let range = Vec3::new(to_ball.x, 0.0, to_ball.z).length();
                match Vec3::new(to_ball.x, 0.0, to_ball.z).try_normalize() {
                    Some(bearing) => {
                        let angle = bearing.x.atan2(bearing.z);
                        let at_the_ball = (((angle - actor.heading + PI).rem_euclid(TAU)) - PI)
                            .clamp(-Self::NECK, Self::NECK);
                        // **He does not stare at it from sixty metres.**
                        //
                        // Every head on the pitch was welded to the ball at
                        // every range, so twenty-two necks swivelled in
                        // perfect unison all match — which is a large part
                        // of what reads as mechanical, and it is worst
                        // exactly where most of the players are: away from
                        // the play, where a real footballer is looking
                        // around him. Past `SCANNING` the look crosses over
                        // to a slow sweep on his own idle clock, so the men
                        // off the ball are each doing something slightly
                        // different and the ones near it are still locked on.
                        let watching = (1.0
                            - (range - Self::WATCHING) / (Self::SCANNING - Self::WATCHING))
                            .clamp(0.0, 1.0);
                        let scan = (actor.idle * 0.5).sin() * Self::SCAN_SWEEP;
                        at_the_ball * watching + scan * (1.0 - watching)
                    }
                    None => 0.0,
                }
            } else {
                0.0
            };
            actor.look += (wanted_look - actor.look) * if playback.seeked { 1.0 } else { turn };

            // And how far up or down. A cross comes in above head height and a
            // shot along the floor arrives below the knee; a player who tracks
            // both of them with his chin level is watching neither. Clamped
            // harder downward than upward, which is what a neck does.
            let wanted_pitch = if ball.on_pitch && !gathering && !heedless {
                let to_ball = ball.position - position;
                let range = Vec3::new(to_ball.x, 0.0, to_ball.z).length().max(0.4);
                ((ball.position.y - actor.height - Self::EYE) / range)
                    .atan()
                    .clamp(-0.45, 0.80)
            } else {
                0.0
            };
            actor.look_pitch +=
                (wanted_pitch - actor.look_pitch) * if playback.seeked { 1.0 } else { turn };

            // Set, or simply standing. A keeper drops onto his toes as the
            // ball comes into range of his goal and stands out of it again
            // when it goes away — which is the posture every save comes out
            // of, and the reason a dive used to arrive from nowhere.
            let wanted_set = if actor.is_goalkeeper && ball.on_pitch && !heedless {
                let to_ball = ball.position - position;
                let range = Vec3::new(to_ball.x, 0.0, to_ball.z).length();
                ((Self::SET_RANGE.1 - range) / (Self::SET_RANGE.1 - Self::SET_RANGE.0))
                    .clamp(0.0, 1.0)
            } else {
                0.0
            };
            actor.set += (wanted_set - actor.set) * if playback.seeked { 1.0 } else { pace };

            // **The save he makes on his feet**, which is 84% of the balls
            // that arrive at him at pace. The reach opens out as the ball
            // closes and is given back by the follow-through: a ratchet, like
            // the dive's extension and for the same reason — nobody folds his
            // arms back up halfway through a save.
            //
            // [`Save::delay`] is a real countdown read off the recording, so
            // the hands arrive with the ball at 1x, at 8x and wherever the
            // playhead is dropped, exactly as a kick's backswing does.
            let arriving = actor.arrival.filter(|_| !heedless);
            let wanted_reaction =
                arriving.map_or(0.0, |save| Self::ease(1.0 - save.delay / Self::SAVE_ONSET));
            if playback.seeked || wanted_reaction > actor.reaction {
                actor.reaction = wanted_reaction;
            } else {
                actor.reaction -=
                    actor.reaction * (1.0 - (-match_delta / Self::SAVE_RELEASE).exp());
            }
            // Where the hands go, and whether they close on it. Both are
            // left where the last live prediction put them once the ball has
            // gone past, so the follow-through finishes where the ball
            // actually was instead of swinging back to the middle as the
            // reach runs out.
            if let Some(save) = arriving {
                let to_ball = save.at - position;
                let across = Vec3::new(to_ball.x, 0.0, to_ball.z).dot(sideways) / Self::SAVE_REACH;
                let up = (save.at.y - Self::SAVE_GATHER)
                    / (Self::SAVE_OVERHEAD - Self::SAVE_GATHER)
                    * 2.0
                    - 1.0;
                let wanted_aim = Vec2::new(across.clamp(-1.0, 1.0), up.clamp(-1.0, 1.0));
                let settle = if playback.seeked { 1.0 } else { pace };
                let aim = actor.aim;
                actor.aim = aim + (wanted_aim - aim) * settle;
                let parry = actor.parry;
                actor.parry = parry + (f32::from(!save.held) - parry) * settle;
            }

            // The one man with the ball in his gloves takes the ramp that the
            // ball itself is riding; everybody else lets whatever they were
            // holding fall away. Following a ramp with a second one is
            // deliberate — it puts the arms a fraction behind the ball, so a
            // keeper throwing it out has a follow through rather than
            // snapping back to his sides on the frame it leaves him.
            //
            // Not while he is THROWING it, though. A player taking a throw-in
            // is `held_by` too, and the cradle — elbows in, ball against the
            // chest — is not what he is doing with it; his arms belong to the
            // throw. Everything else that ends up in a man's hands is a
            // cradle: a keeper who has gathered it, and the one outfielder
            // walking it back to the centre circle, who was drawn trudging
            // along with his hands on his head while the ball he was
            // supposedly carrying floated at his waist.
            let wanted = if ball.held_by == Some(actor.id) && !actor.throwing_in() {
                ball.cradle
            } else {
                0.0
            };
            actor.carry += (wanted - actor.carry) * if playback.seeked { 1.0 } else { pace };

            // …and how long a step HE takes. See `Complexion::stride`: this
            // was a global, so twenty-two players covering the same ground
            // at the same pace all took the same number of steps to do it,
            // which is most of what "lacks variety" means. Cadence is the
            // thing an eye picks a runner out by.
            let (stride, carry_ground) = Self::stride_of(actor.id, actor.speed, actor.underfoot);
            // Half a cycle per step: the other leg takes the next one.
            actor.phase = (actor.phase + ground * PI / stride).rem_euclid(TAU);
            actor.carry_ground = carry_ground;

            // **Last**, once every field it reads has been written: the pose
            // this actor is in. Everything downstream — fifty-odd joints
            // here, the carriage in [`Self::carry_body`] — reads it rather
            // than rebuilding it. See [`PlayerActor::pose`].
            actor.pose = actor.gait();
        }

        // **Where the rig actually gets posed**, and the one loop in the
        // viewer whose cost is the SQUAD size rather than the eleven on the
        // pitch. The page hands over both team sheets — starters and
        // substitutes, thirty-six men — and every one of them is assembled at
        // startup, so this walks some six hundred joints where at most three
        // hundred and seventy belong to anybody currently on the field.
        //
        // Two guards, and they are not the same guard:
        //
        // **The first** skips a man who is not on the pitch. His gait was left
        // untouched by the loop above (which returns early on a hidden actor),
        // so posing him writes last-known angles over identical last-known
        // angles — some two hundred and fifty entities dirtied for a picture
        // nobody is being shown. It is also wrong in the small way that matters here:
        // `Transform` is change-detected, and a write feeds transform
        // propagation whether or not the value moved.
        //
        // **The second** is the same point made about a man who IS on the
        // pitch and is standing still. `Joint::pose` is a pure function of the
        // gait, so an unchanged gait produces bit-identical angles — and a
        // paused replay, a stopped player, half time and the seconds before
        // kickoff are all cases where the whole squad's joints reduce to a
        // comparison. Against a `Vec3` and a `Quat` that is seven floats
        // against the propagation pass, the `GlobalTransform` write and the
        // per-entity uniform this frame would otherwise re-upload for each of
        // them.
        for (joint, mut transform) in &mut joints {
            let Ok((actor, _, visibility)) = actors.get(joint.owner) else {
                continue;
            };
            if *visibility == Visibility::Hidden {
                continue;
            }
            let rotation = joint.pose(actor.pose);
            let translation = joint.place(actor.pose);
            if transform.rotation != rotation {
                transform.rotation = rotation;
            }
            if transform.translation != translation {
                transform.translation = translation;
            }
        }
    }

    /// Takes the whole figure off its feet: the topple that puts a diving
    /// keeper horizontal and the lift that gets him off the grass.
    ///
    /// Its own system rather than a second loop inside [`Actors::animate`]
    /// because the carriage and the joints both want `&mut Transform`, and
    /// two mutable transform queries in one system have to be proved
    /// disjoint by filters that say nothing about why they are there.
    ///
    /// The transform lands on the [`Carriage`] and not on the actor, so the
    /// contact shadow and the team ring — which hang off the actor itself —
    /// stay flat on the turf underneath him.
    pub fn carry_body(
        actors: Query<&PlayerActor>,
        mut carriages: Query<(&Carriage, &mut Transform)>,
    ) {
        for (carriage, mut transform) in &mut carriages {
            let Ok(actor) = actors.get(carriage.owner) else {
                continue;
            };
            let (pitch, roll) = actor.topple();
            // Written only on a change, for the reason the joint loop in
            // [`Self::animate`] gives at length: a carriage is the root of a man's whole body,
            // so dirtying one dirties the twenty-odd meshes hanging off it —
            // and for all but a handful of players on any given frame the
            // value being written is the value already there.
            let placed = Carriage::placed(pitch, roll, actor.lift());
            if *transform != placed {
                *transform = placed;
            }
        }
    }

    /// Which way a player should be turned this frame, as an unnormalised
    /// direction — or [`Vec3::ZERO`] to leave him facing where he already was.
    ///
    /// Its own function because it is a priority list with five branches and
    /// every one of them is a claim about football that can be got wrong: a
    /// man sets his body before he strikes rather than after, a keeper going
    /// over stays square to the shot, a follow-through outranks a running
    /// line. Buried inside `animate` none of it could be checked.
    fn facing(
        actor: &PlayerActor,
        ball: &BallState,
        position: Vec3,
        step: Vec3,
        unwatched: bool,
    ) -> Vec3 {
        if let Some(kick) = actor.kick.filter(|kick| kick.swing < 0.0) {
            // Opening up to what he is ABOUT to hit. A footballer sets his
            // body before he strikes the ball, not after it has gone — which
            // the viewer can honour because it knows the kick is coming.
            // Outranks everything below, including the dive: this is the same
            // man being turned by the same intention.
            kick.direction
        } else if actor.grounded() > 1e-3 {
            // **On the floor, he keeps the heading he landed on.**
            //
            // The topple is expressed in his OWN frame — see
            // [`PlayerActor::topple`] — so yawing the actor swings the whole
            // sprawled body round on the grass like a compass needle, and the
            // direction he went over in world space is whatever the last turn
            // left it as. Nothing else in the rig has that property, which is
            // why this branch has to come before the one below rather than
            // relying on `unwatched`: after a goal the ball a beaten keeper
            // would be turning to watch is a metre behind him IN THE NET,
            // where the bearing to it is rounding error and swings through a
            // right angle frame to frame. He is also, being on the floor,
            // not turning to look at anything.
            Vec3::ZERO
        } else if actor.dive > 1e-3 && ball.on_pitch {
            // A keeper off his feet stays square to the shot and lets his body
            // go over; he does not turn to face the corner he is diving into.
            // Outranks the run for the same reason the strike does — this is a
            // man travelling one way and pointed another, and it is also what
            // makes `tip` decomposable onto his own right and forward.
            ball.position - position
        } else if let Some((direction, _)) = actor.strike {
            // Opened up to where he played it, for as long as the follow
            // through lasts. Outranks the run: this is the one moment a
            // footballer is not facing where he is going.
            direction
        } else if actor.is_goalkeeper
            && ball.on_pitch
            && !unwatched
            && position.distance(ball.position) < Self::SET_RANGE.1
        {
            // **A goalkeeper stays square to the play, and opens up as he
            // gets going — but he never turns his back on the ball.**
            //
            // Everybody else turns to face his run, and should: an
            // outfielder covering ground is going somewhere. A keeper
            // covering ground is watching something — he shuffles across his
            // line and drops back onto it without taking his eyes off the
            // ball, and as he has to move faster he turns his hips into the
            // run while his neck holds the ball, which is what `look`
            // already draws.
            //
            // Bounded by the same range that puts him on his toes, so it is
            // one claim rather than two: while the ball is near enough to be
            // his problem he never turns away from it, and a keeper
            // strolling out to the edge of his area with play at the other
            // end walks where he is going like anybody else.
            //
            // Above the run branch rather than below it, because the run
            // branch is exactly what it is overruling. Measured, this is 87%
            // of his frames. See [`Actors::SQUARE_UP`].
            //
            // ⚠ **THIS BRANCH USED TO SWITCH OFF ABOVE `SQUARE_UP.1`** —
            // 2.8 m/s — and everything past it fell through to "face your
            // run". So a keeper recovering to his line at anything above a
            // jog was drawn with his back to the attacking team, which is
            // the reported bug, and it is worst in exactly the situation it
            // matters: the ball in front of him and play coming on.
            //
            // A real keeper does not do that, at any speed, while the ball
            // is live in front of him. He goes side-on and cross-steps, head
            // over his shoulder. So the opening does not stop at a speed, it
            // stops at an ANGLE: he turns as far onto his run as
            // [`Actors::SHOULDER`] and no further. Where the run and the
            // ball are the same way — a keeper sprinting out to a
            // through-ball — the clamp does not bind and nothing changes.
            let watching = ball.position - position;
            let travel = Vec3::new(actor.travel.x, 0.0, actor.travel.z);
            match (
                Vec3::new(watching.x, 0.0, watching.z).try_normalize(),
                travel.try_normalize(),
            ) {
                (Some(watching), Some(going)) => {
                    // Rotated toward his run rather than lerped toward it:
                    // two nearly opposite directions average to nothing, and
                    // "nothing" is where a heading goes wild.
                    let opening = Self::ease(
                        (actor.speed - Self::SQUARE_UP.0) / (Self::SQUARE_UP.1 - Self::SQUARE_UP.0),
                    );
                    let square = watching.x.atan2(watching.z);
                    let along = going.x.atan2(going.z);
                    let swing = ((along - square + PI).rem_euclid(TAU)) - PI;
                    let turned =
                        square + (swing * opening).clamp(-Self::SHOULDER, Self::SHOULDER);
                    Vec3::new(turned.sin(), 0.0, turned.cos())
                }
                _ => watching,
            }
        } else if actor.speed > Self::MOVING {
            // His momentum, not this frame's step — see `Actors::travel`.
            // Falls back to the raw step only when the filter has nothing
            // in it yet (the first frame after a seek).
            let travel = Vec3::new(actor.travel.x, 0.0, actor.travel.z);
            if travel.length_squared() > 1e-6 {
                travel
            } else {
                Vec3::new(step.x, 0.0, step.z)
            }
        } else if ball.on_pitch && !unwatched {
            ball.position - position
        } else {
            // He keeps the heading he had. A man standing still with no ball
            // to watch has no reason to turn, and one who has it IN HIS HANDS
            // has less than none — which is the whole reason for that second
            // test.
            //
            // `unwatched` covers a second case with the same shape: the
            // seconds after a goal, when nobody is watching the ball because
            // it is in the net and the match has stopped. For the conceding
            // side the ball is BEHIND them and a metre or two away, rattling
            // in the mesh, so the bearing to it swings wildly and they were
            // drawn pivoting to track it out of their own goal. See
            // [`Aftermath`].
            //
            // The engine snaps a held ball to the middle of the man holding
            // it, so `ball.position - position` is not a bearing: it is the
            // difference between two independently ROUNDED positions.
            // Measured over a real match it comes out at 1.25 cm at the 90th
            // percentile — exactly one quantisation step — and non-zero often
            // enough that a heading really was written from it three times in
            // ten. The heading it wrote swung a median of 45° from one frame
            // to the next and past a right angle one frame in nine. Since the
            // ball is then DRAWN a third of a metre in front of whatever that
            // came out as, a keeper standing with the ball had it orbiting
            // him, and half the time it sat at his back.
            Vec3::ZERO
        }
    }

    /// How far a point one metre above the turf throws its shadow across it,
    /// and which way. Straight off the light — see [`Pitch::SUN`].
    fn sun_throw() -> Vec2 {
        Vec2::new(Pitch::SUN.x, Pitch::SUN.z) / -Pitch::SUN.y
    }

    /// Lays each player's shadow on the grass where the stadium light puts it.
    ///
    /// The scene has no shadow maps — they are the single most expensive thing
    /// it could ask a WebGL2 context for — so every shadow on this pitch is a
    /// painted disc that somebody has to place. Placed symmetrically under the
    /// boots, as these were, twenty-two footballers stand on twenty-two neat
    /// pools of their own and the whole squad reads as pasted onto the turf:
    /// the light comes from 28° off the vertical, so a real one lands the best
    /// part of half a metre to the side and is stretched along that bearing.
    ///
    /// Its own system, after the body has been moved, and reading the actor's
    /// world transform rather than hanging off it — see [`Silhouette`].
    pub fn cast_shadows(
        actors: Query<(&PlayerActor, &Transform, &Visibility)>,
        mut shadows: Query<(&Silhouette, &mut Transform, &mut Visibility), Without<PlayerActor>>,
    ) {
        let throw = Self::sun_throw();
        let bearing = throw.x.atan2(throw.y);
        // A disc seen from the light's angle covers this much more ground
        // along the bearing than across it.
        let stretch = (1.0 + throw.length_squared()).sqrt();

        // Written only on a change, all three times below. `Visibility` is
        // change-detected and every write feeds the propagation pass, so
        // twenty-two shadows saying "still visible" on every frame is
        // twenty-two entities dirtied for nothing — the same trap
        // [`Bank::cull`] documents.
        let settle = |visibility: &mut Visibility, wanted: Visibility| {
            if *visibility != wanted {
                *visibility = wanted;
            }
        };

        for (mark, mut transform, mut visibility) in &mut shadows {
            let Ok((actor, body, shown)) = actors.get(mark.actor) else {
                settle(&mut visibility, Visibility::Hidden);
                continue;
            };
            if *shown == Visibility::Hidden {
                settle(&mut visibility, Visibility::Hidden);
                continue;
            }
            // A body's shadow is the smear between the shadow of its boots and
            // the shadow of its head, so it is centred on neither — and the
            // legs are the wider half of a footballer's silhouette, which is
            // why the middle sits low.
            let centre = Physique::STATURE * body.scale.y * Self::SHADOW_CENTRE + actor.lift();
            let offset = throw * centre;
            transform.translation = Vec3::new(
                body.translation.x + offset.x,
                Self::SHADOW_LIFT,
                body.translation.z + offset.y,
            );
            transform.rotation = Quat::from_rotation_y(bearing);
            // It spreads as its caster leaves the ground. The material is
            // shared by the whole squad and so cannot be faded per player, but
            // spreading the same painted disc over more grass thins it, which
            // is most of what a keeper half a metre up needs from it.
            let spread = Self::FOOTPRINT * 0.86 * (1.0 + 0.30 * actor.lift().min(1.2));
            transform.scale = Vec3::new(spread, 1.0, spread * stretch);
            settle(&mut visibility, Visibility::Inherited);
        }
    }

    /// Smoothstep on 0..1 — a ramp that leaves and arrives at rest.
    ///
    /// Every one of the dive's ramps runs through this rather than being
    /// linear. A body accelerating out of a push-off and settling into full
    /// extension has no corners in it, and a linear ramp puts one at each
    /// end of every limb's travel.
    pub(crate) fn ease(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Writes each player's current engine state under their name. Debug
    /// overlay only, and only for recordings that carry state tracking.
    pub fn follow_states(
        playback: Res<Playback>,
        overlay: Res<DebugOverlay>,
        mut tracks: ResMut<ReplayTracks>,
        mut labels: Query<(&PlayerStateLabel, &mut TextSpan)>,
    ) {
        let now = playback.time_ms;
        for (label, mut span) in &mut labels {
            let wanted = if overlay.states {
                tracks
                    .states
                    .get_mut(&label.id)
                    .and_then(|track| track.name_at(now))
                    .unwrap_or_default()
            } else {
                ""
            };
            if span.as_str() != wanted {
                **span = wanted.to_string();
            }
        }
    }

    /// Projects each visible player to screen space and parks their name plate
    /// just below their feet.
    ///
    /// Both the camera and the players are root entities, so their local
    /// transforms are their world transforms — projecting from those rather
    /// than from `GlobalTransform` keeps the plates locked to the players
    /// instead of trailing a frame behind the pan.
    pub fn place_labels(
        camera: Single<(&Camera, &Transform), With<Camera3d>>,
        actors: Query<(&Transform, &Visibility), With<PlayerActor>>,
        mut labels: Query<(&PlayerLabel, &mut Node, &mut Visibility), Without<PlayerActor>>,
    ) {
        let (camera, camera_transform) = *camera;
        let camera_transform = GlobalTransform::from(*camera_transform);

        // Same rule as the contact shadows: touch neither the visibility nor
        // the node unless the value actually moved. A `Node` write reruns the
        // layout pass over the WHOLE UI tree, so a plate that has not moved
        // must not write itself.
        let settle = |visibility: &mut Visibility, wanted: Visibility| {
            if *visibility != wanted {
                *visibility = wanted;
            }
        };

        for (label, mut node, mut visibility) in &mut labels {
            let Ok((actor_transform, actor_visibility)) = actors.get(label.actor) else {
                settle(&mut visibility, Visibility::Hidden);
                continue;
            };
            if *actor_visibility == Visibility::Hidden {
                settle(&mut visibility, Visibility::Hidden);
                continue;
            }
            // Project the player twice — once at the boots, once at the crown —
            // and hang the plate below the boots by a share of the height
            // between them.
            let boots = actor_transform.translation;
            let crown = boots + Vec3::Y * Physique::STATURE * actor_transform.scale.y;
            let (Ok(boots), Ok(crown)) = (
                camera.world_to_viewport(&camera_transform, boots),
                camera.world_to_viewport(&camera_transform, crown),
            ) else {
                settle(&mut visibility, Visibility::Hidden);
                continue;
            };

            // The plate is centred by hand: UI nodes are positioned by their
            // top-left corner and the text width is not known here.
            //
            // Rounded to the pixel it will be drawn at, so a player standing
            // still — before kickoff, at a restart, through half time — stops
            // writing his plate's node at all. Sub-pixel positions on a UI
            // node buy nothing: `bevy_ui` rounds them for the draw anyway, and
            // the only thing the extra precision was doing was guaranteeing a
            // relayout of the bar and twenty-two plates on every frame.
            //
            // Width and alignment are set once, where the plate is spawned:
            // they are constants, and writing a constant into a
            // change-detected component every frame is the same relayout by
            // another route.
            let stature = (boots.y - crown.y).abs().max(6.0);
            let left = Val::Px((boots.x - 44.0).round());
            let top = Val::Px((boots.y + stature * Self::LABEL_GAP).round());
            if node.left != left || node.top != top {
                node.left = left;
                node.top = top;
            }
            settle(&mut visibility, Visibility::Inherited);
        }
    }

    /// The name plate. Just the surname: the shirt number is on the player's
    /// back, where a viewer reads it from, and repeating it in front of every
    /// name only crowds the pitch.
    fn label_for(player: &PlayerInfo, debug: bool) -> String {
        if debug {
            format!("{}\n", player.last_name)
        } else {
            player.last_name.clone()
        }
    }
}

impl PlayerActor {
    fn new(id: u32, is_goalkeeper: bool, is_home: bool) -> Self {
        PlayerActor {
            id,
            is_goalkeeper,
            is_home,
            despair: 0.0,
            elation: 0.0,
            carry: 0.0,
            kick: None,
            drive: 0.0,
            carrying: 0.0,
            dive: 0.0,
            stretch: 0.0,
            air: 0.0,
            down: 0.0,
            climb: 0.0,
            flat: 0.0,
            previous_height: 0.0,
            tip: Vec2::ZERO,
            height: 0.0,
            look_pitch: 0.0,
            set: 0.0,
            previous: None,
            heading: 0.0,
            // Start everyone at a different point in the run cycle. The
            // phase advances with ground covered, so two players moving at
            // the same speed from the same start stay in step for the whole
            // match — which is why the squad used to move as one organism.
            phase: Complexion::carriage(id) * std::f32::consts::PI,
            speed: 0.0,
            travel: Vec3::ZERO,
            strike: None,
            // Offset so twenty-two players are not all breathing in unison,
            // which would be its own kind of robot.
            idle: Complexion::carriage(id) * std::f32::consts::PI,
            turn: 0.0,
            look: 0.0,
            course: Vec2::Y,
            open: 0.0,
            underfoot: Vec2::Y,
            carry_ground: 0.0,
            arrival: None,
            reaction: 0.0,
            aim: Vec2::ZERO,
            parry: 0.0,
            clock: 0.0,
            pose: Gait::resting(),
        }
    }

    /// How far off the turf he is, in metres, straight off the recording —
    /// plus the one thing the recording cannot say.
    ///
    /// Nothing is added to the FLIGHT. The engine launches a keeper on a
    /// ballistic arc out of his `jumping` attribute and lets gravity bring
    /// him down, so the rise and the landing are already a physical
    /// trajectory rather than an animation curve, and anything this end
    /// could contribute would be fighting it.
    ///
    /// **The push off the floor is the exception**, and the recorded height
    /// is exactly zero for the whole of it, because the engine has no notion
    /// of a man on his hands and knees.
    ///
    /// [`Carriage::placed`] drops a figure by how far from upright it is,
    /// which is the right rule for LYING DOWN — a body on its side has its
    /// hips a half hip-breadth off the grass and nothing else is holding it
    /// up — and quite wrong for a body at the same angle with its weight on
    /// two knees and two palms. Measured through a real recovery without
    /// this: his hips sit 0.55 m up with the trunk only 31° off vertical,
    /// which is nowhere near enough for a leg to reach the turf, and **his
    /// boots spend the whole get-up 15–20 cm UNDER the pitch.** That was
    /// true before any of the get-up was drawn; nothing had ever looked.
    ///
    /// So the settle is given back in step with `rising`, which is exactly
    /// the share of his own weight he has taken off the ground — and then
    /// pulled toward [`Carriage::KNEELING`] by how far into the kneel he is,
    /// because a man on his knees is neither lying nor standing and the
    /// settle has no third answer. Without that second half a beaten keeper
    /// holding the kneel floats with both boots a quarter of a metre off
    /// the grass, which is the same fault the other way up.
    fn lift(&self) -> f32 {
        let (pitch, roll) = self.topple();
        let settled = Physique::HIP - Carriage::SETTLE * Carriage::tilt(pitch, roll);
        let standing = settled + (Physique::HIP - settled) * self.rising();
        // Its OWN gait rather than the cached [`Self::pose`]: this is called
        // from three systems, one of which runs before the frame's update,
        // and it is a handful of calls a frame rather than the fifty-odd
        // per player the cache exists for.
        let hips = standing + (Carriage::KNEELING - standing) * self.gait().kneeling();
        self.height + hips - settled
    }

    /// Advances the whole of a save: off his feet, through the extension, and
    /// back up off the grass. Returns whether he is airborne this frame.
    ///
    /// Its own function rather than forty lines inside [`Actors::animate`]
    /// because it is the one part of this rig that is a state machine over
    /// TIME rather than a pose, and because the recorded height series it
    /// consumes can then be replayed straight out of a real match and checked.
    ///
    /// Everything it reads is already on `self` — the recorded height, the
    /// height it had last frame — bar what only the caller knows: how much
    /// match time has passed, and how fast he is covering ground both
    /// smoothed (`pace`, which survives a noisy frame) and as measured this
    /// frame (`ground`, which does not lag).
    fn track_flight(&mut self, match_delta: f32, pace: f32, ground: f32, seeked: bool) -> bool {
        let climb = if seeked || match_delta <= 0.0 {
            0.0
        } else {
            (self.height - self.previous_height) / match_delta
        };
        self.previous_height = self.height;
        // Only ever a keeper: twelve outfield players leave the turf in a
        // recorded match to head a ball, and every one of them used to be
        // drawn toppling sideways with both arms over his head.
        let airborne = self.is_goalkeeper && self.height > Actors::AIRBORNE_FEET;

        if seeked {
            // Nothing across a jump of the playhead is a trajectory. Land on
            // whatever the frame itself says and start again.
            self.air = 0.0;
            self.down = 0.0;
            self.climb = 0.0;
            self.flat = 1.0;
            self.dive = f32::from(airborne);
            self.stretch = self.dive;
        } else if airborne {
            self.air += match_delta;
            self.down = 0.0;
            self.climb = self.climb.max(climb);
            self.dive += (1.0 - self.dive) * (1.0 - (-match_delta / Actors::SPRAWL_ATTACK).exp());
            // The extension ratchets: it opens out across the flight and is
            // given back only by the recovery, because nobody folds himself
            // up again in mid-air.
            self.stretch = self
                .stretch
                .max(Actors::ease((self.air / Actors::EXTENSION).clamp(0.0, 1.0)));
            // How far he goes over is the angle he left the ground at,
            // because a body in the air travels along the vector it launched
            // on. Latched over the first moments off the turf and then held:
            // the rise is bleeding off to nothing by the apex anyway.
            if self.air <= Actors::LAUNCH_WINDOW {
                let launch = self.climb.max(0.0).atan2(pace.max(0.1));
                self.flat = Actors::ease(1.0 - (launch / FRAC_PI_2).clamp(0.0, 1.0));
            }
        } else if self.dive > 1e-3 {
            self.down += match_delta;
            self.air = 0.0;
            self.climb = 0.0;
            // He lies there for a beat and then gets up — unless the
            // recording already has him moving, in which case standing him up
            // early is the lesser of the two lies. Measured: half of all
            // landings are followed by the keeper covering five to seven
            // metres inside a second, and half by him covering under one.
            //
            // Against the ground he is covering THIS frame and not his
            // smoothed pace, which is the whole reason that argument exists:
            // the smoothing takes about a fifth of a second to catch up, and
            // for that fifth of a second every keeper still reads as
            // travelling at the ten metres a second he dived at. Judged on
            // that, the hold expired instantly on every save in the match and
            // nobody ever stayed down.
            let hurry = (ground / Actors::SPRAWL_URGENCY).clamp(0.0, 1.0);
            // …and a beaten keeper stays down, for a length of time that is
            // his own. See [`Actors::BEATEN_HOLD`].
            let beaten = self.despair
                * (1.0 + Actors::BEATEN_SPREAD * Complexion::carriage(self.id))
                * (Actors::BEATEN_HOLD - 1.0);
            let hold = Actors::SPRAWL_HOLD * (1.0 - hurry) * (1.0 + beaten);
            if self.down > hold {
                let release = 1.0 - (-match_delta / Actors::SPRAWL_RECOVERY).exp();
                // **He comes up as far as his knees and stops there.**
                //
                // The recovery used to run to zero in one exponential, which
                // is a body rotating from flat to upright at a constant rate
                // with every limb frozen — reported, exactly, as getting up
                // like a robot. The floor is what puts a beat in the middle
                // of it: he kneels, and stays kneeling until the recording
                // gives him somewhere to be. See [`Actors::KNEELING`].
                let kneel = Actors::KNEELING * self.despair * (1.0 - hurry);
                self.dive -= (self.dive - kneel).max(0.0) * release;
                // The EXTENSION goes all the way back regardless: a man on
                // his knees is not still at full stretch, whatever else he
                // is doing.
                self.stretch -= self.stretch * release;
            }
        } else {
            self.air = 0.0;
            self.down = 0.0;
            self.climb = 0.0;
            self.dive = 0.0;
            self.stretch = 0.0;
        }
        airborne
    }

    /// Takes the swing forward one frame: arms it from a kick the recording
    /// says is coming, then carries it through the ball and out the other
    /// side.
    ///
    /// The backswing is driven by the countdown to contact and the follow
    /// through by the clock, and the join between them is at `swing = 0`,
    /// which is the moment the boot meets the ball. Everything before contact
    /// is therefore locked to the recording — the leg arrives exactly when the
    /// ball leaves, at any playback speed and wherever the playhead is put —
    /// and everything after it is free, because by then there is nothing left
    /// to be in step with.
    fn swing_leg(&mut self, coming: Option<Impact>, match_delta: f32, seeked: bool) {
        if seeked {
            self.kick = None;
            return;
        }

        if let Some(impact) = coming {
            let contact = impact.contact;
            // Square-rooted: the amplitude is a distance a boot travels and
            // the number driving it is a speed, and a linear map between them
            // leaves the median pass — 17 m/s, a firm ball over twenty yards —
            // as a third of a swing.
            let power = ((contact.velocity.length() - Actors::TOUCHED)
                / (Actors::HAMMERED - Actors::TOUCHED))
                .clamp(0.0, 1.0)
                .sqrt();
            let direction = Vec3::new(contact.velocity.x, 0.0, contact.velocity.z)
                .try_normalize()
                .unwrap_or(Vec3::Z);
            // Which boot. Whichever leg is trailing as the swing starts: a
            // kick is the continuation of a stride rather than an
            // interruption of one, so taking the leg that was coming through
            // anyway is both what a footballer does and what blends. A man
            // standing still has no stride to continue, and uses the foot he
            // favours.
            let foot = self.kick.map_or_else(
                || {
                    if self.speed > Actors::MOVING {
                        if self.phase.sin() < 0.0 { 1.0 } else { -1.0 }
                    } else {
                        Complexion::footedness(self.id)
                    }
                },
                |kick| kick.foot,
            );
            let blend = self.kick.map_or(0.0, |kick| kick.blend);
            self.kick = Some(Kick {
                // −1 at the far end of the window, 0 at contact.
                swing: -(contact.delay / Actors::WINDUP).clamp(0.0, 1.0),
                power,
                foot,
                blend: (blend + match_delta / Actors::KICK_ONSET).min(1.0),
                direction,
                // The ball's own track has already said what the moment looks
                // like; the one thing it cannot see is whose hands are on it.
                // A keeper who has gathered it and is about to send it upfield
                // is throwing it however the geometry reads, and drawing him
                // volleying it out of his own gloves would be worse than
                // drawing nothing at all.
                kind: if self.carry > 0.5 {
                    Strike::Throw
                } else {
                    contact.kind
                },
            });
        } else if let Some(kick) = &mut self.kick {
            // Contact has passed out of the window ahead. The rest is the
            // follow through, which nothing in the recording constrains.
            kick.blend = (kick.blend + match_delta / Actors::KICK_ONSET).min(1.0);
            kick.swing = (kick.swing.max(0.0) + match_delta / Actors::FOLLOW_THROUGH).min(1.0);
            if kick.swing >= 1.0 {
                self.kick = None;
            }
        }
    }

    /// How far the whole figure has gone over, as the pitch and roll the
    /// [`Carriage`] takes.
    ///
    /// Rotating about +Z carries the head toward −X, so going over onto his
    /// own right is the NEGATIVE of the sideways tip — the same sign
    /// convention as the bank into a turn in `Joint::pose`. About +X carries
    /// the head forward, which is a keeper going down at a striker's feet and
    /// needs no flip.
    ///
    /// Scaled by the extension rather than by the dive, so the body goes over
    /// across the flight instead of snapping flat on the frame he leaves the
    /// ground. One function because two callers need the answer — the
    /// carriage itself, and the ball he may be holding — and they cannot be
    /// allowed to disagree.
    ///
    /// **And then the ground has the last word.** [`Actors::SPRAWL_ANGLE`] is
    /// the angle of a body in FLIGHT, and a dive off a real launch angle
    /// reaches 64–79° of it — so a keeper who landed kept his 11–26° and lay
    /// there on an invisible slope with his hips a third of a metre up and his
    /// head held clear of the grass, which is what "he doesn't look real on
    /// the floor" is. A body on the ground is at the angle of the ground; a
    /// body that got its feet under it is upright. See [`Self::committed`],
    /// which is which.
    fn topple(&self) -> (f32, f32) {
        let Some(way) = self.tip.try_normalize() else {
            return (0.0, 0.0);
        };
        // **He rolls onto his front before he pushes.** See
        // [`Actors::ROLLS_OVER`] — the axis he went down on is not the axis
        // he comes up about, and turning the direction rather than the
        // magnitude means the change costs nothing at either end of the
        // recovery.
        let onto_his_front =
            Actors::ease((self.rising() * Actors::ROLL_EARLY).min(1.0)) * Actors::ROLLS_OVER;
        let way = Vec2::from_angle(way.angle_to(Vec2::Y) * onto_his_front).rotate(way);
        let flying = Actors::SPRAWL_ANGLE * self.tip.length() * self.stretch;
        let landed = FRAC_PI_2 * self.committed();
        let tip = way * (flying + (landed - flying) * self.settling());
        (tip.y, -tip.x)
    }

    /// How far past the point of no return he was when he arrived, 0..1 — the
    /// difference between a landing and a fall.
    ///
    /// A keeper who went up for a cross is barely leaning and gets his feet
    /// under him; one who is already three quarters of the way over cannot,
    /// and gravity takes him the rest of the way. Without it the whole
    /// on-the-floor pose — the curl, the legs coming up, the arms folding in —
    /// was applied to every landing in the match, so a keeper who caught a
    /// corner spent the next half second in a deep squat leaning thirty
    /// degrees with his boots through the turf. See [`Actors::GOES_OVER`].
    ///
    /// Off the tip alone and NOT the extension, unlike the angle itself: how
    /// far over he ended up is a fact about the take-off, and it does not
    /// stop being true while he is getting back up. Reading a decaying
    /// `stretch` through this instead has him rediscover halfway through the
    /// recovery that he was never really down, and pop upright in a quarter
    /// of a second.
    fn committed(&self) -> f32 {
        let over = Actors::SPRAWL_ANGLE * self.tip.length();
        Actors::ease((over - Actors::GOES_OVER.0) / (Actors::GOES_OVER.1 - Actors::GOES_OVER.0))
    }

    /// How far through the landing he is, 0..1, whatever kind of landing it
    /// turned out to be. What ends the flight: the extension gives way, the
    /// arms come down, the ball comes in off the gloves and onto the chest.
    fn settling(&self) -> f32 {
        (self.down / Actors::GROUNDING).clamp(0.0, 1.0) * self.dive
    }

    /// …and how far into the ground-out he is: the landing and the commitment
    /// together, which is the only combination that means he is DOWN THERE.
    ///
    /// Faded by the dive itself, so it lets go as he gets back to his feet
    /// rather than pinning him to the turf.
    fn grounded(&self) -> f32 {
        self.settling() * self.committed()
    }

    /// **How far off the floor he has pushed**, 0 flat out and 1 back on his
    /// feet.
    ///
    /// The complement of the dive, gated on his actually having been down
    /// there — so it is zero all the way through a flight, zero for a keeper
    /// who landed on his feet at a cross, and climbs to one across the
    /// recovery. Everything the get-up draws is this against
    /// [`Self::grounded`], which is its opposite: the product of the two
    /// peaks in the MIDDLE of the movement, which is where a man getting off
    /// the floor is on one knee with a hand on the turf, and falls to
    /// nothing at both ends without either of them having to know about the
    /// other.
    fn rising(&self) -> f32 {
        let landed = (self.down / Actors::GROUNDING).clamp(0.0, 1.0);
        landed * self.committed() * (1.0 - self.dive)
    }

    /// How far out at the end of a stretch he is — off his feet and not yet
    /// gathered up. What decides whether a ball he has claimed is in his
    /// gloves or against his chest, so it ends with the LANDING and not with
    /// the ground-out: a keeper who takes a cross at the top of a leap and
    /// lands on his feet brings it in to his chest like anybody else.
    fn extended(&self) -> f32 {
        self.dive * (1.0 - self.settling())
    }

    /// He has the ball in both hands for a throw-in: from the moment the
    /// swing is armed up to and INCLUDING the instant it leaves him.
    ///
    /// Contact is `swing == 0`, and the ball is in his hands right up to it —
    /// letting go a frame early puts it back on the grass for one frame
    /// before it flies, which is the sort of thing nobody sees and everybody
    /// notices. Everything after contact is the follow through, where his
    /// hands are empty.
    fn throwing_in(&self) -> bool {
        self.kick
            .is_some_and(|kick| kick.kind == Strike::ThrowIn && kick.swing <= 0.0)
    }

    /// And which hand a KEEPER is about to throw it out of, if he is: the
    /// same side the swing is routed to, since only that arm moves.
    fn throwing_hand(&self) -> Option<f32> {
        self.kick
            .filter(|kick| kick.kind == Strike::Throw && kick.swing <= 0.0)
            .map(|kick| if kick.foot < 0.0 { -1.0 } else { 1.0 })
    }

    /// **How this man takes a goal**, as the three weights [`Gait`] carries:
    /// hands on his head, hands on his hips, bent double over his knees.
    /// What is left over is his arms hanging, which is the fourth.
    ///
    /// One draw per player, held for the match, off its own salt — see
    /// [`Complexion::reaction`]. It was `carriage > 0`, which is to say a
    /// coin flip between two poses with every goalkeeper on the pitch taking
    /// the same one; a conceding eleven that splits four ways is the
    /// difference between a reaction and a formation.
    ///
    /// Keepers still lean heavily toward the head, and should: he is the man
    /// the camera cuts to, and hands on the head is the picture.
    fn taking_it(&self) -> (f32, f32, f32) {
        match (self.is_goalkeeper, Complexion::reaction(self.id)) {
            (true, 0..46) | (false, 0..28) => (1.0, 0.0, 0.0),
            (true, 46..76) | (false, 28..54) => (0.0, 1.0, 0.0),
            (true, 76..91) | (false, 54..76) => (0.0, 0.0, 1.0),
            // …and the rest simply hang their arms, which is what all three
            // being zero means.
            _ => (0.0, 0.0, 0.0),
        }
    }

    /// **What a goalkeeper with nothing to do is doing**: urging his back
    /// four up, pointing somebody into position, or standing with his hands
    /// on his hips. See [`Gait::urging`].
    ///
    /// Everything else this rig draws is read off the recording, and this
    /// cannot be — but it also does not need to be. It is gated on his
    /// having nothing else to do at all: the ball is not near his goal, he
    /// is not moving, he has not got it, nothing has just happened. Whatever
    /// he does inside that window is unfalsifiable by the recording, and a
    /// man standing to attention for eighty minutes is the one option that
    /// is definitely wrong.
    fn gesturing(&self) -> (f32, f32, f32) {
        let spare = f32::from(self.is_goalkeeper)
            * (1.0 - self.set)
            * (1.0 - (self.speed / Actors::MOVING).clamp(0.0, 1.0))
            * (1.0 - self.carry)
            * (1.0 - self.dive)
            * (1.0 - self.reaction)
            * (1.0 - self.despair.max(self.elation));
        if spare <= 1e-3 {
            return (0.0, 0.0, 0.0);
        }
        // His own place in the cycle. Read off the clock rather than
        // integrated, so a seek lands him wherever the match is rather than
        // resuming a gesture nobody saw begin.
        let phase = (self.clock + Complexion::carriage(self.id) * Actors::GESTURE_CYCLE)
            .rem_euclid(Actors::GESTURE_CYCLE);
        let window = |from: f32, hold: f32| {
            let since = phase - from;
            if !(0.0..hold).contains(&since) {
                return 0.0;
            }
            let ramp = Actors::GESTURE_RAMP;
            Actors::ease((since / ramp).min((hold - since) / ramp).clamp(0.0, 1.0)) * spare
        };
        // Which arm he points with is his, like everything else about him.
        let hand = if Complexion::carriage(self.id) < 0.0 {
            -1.0
        } else {
            1.0
        };
        (
            window(1.2, Actors::GESTURE_HOLD),
            window(6.0, Actors::GESTURE_HOLD) * hand,
            window(9.8, Actors::GESTURE_STANCE),
        )
    }

    /// Which side of his body he committed to, −1..1. See [`Gait::lead`].
    fn lead(&self) -> f32 {
        let travel = self.tip.length();
        if travel > 1e-3 {
            (self.tip.x / travel).clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }

    fn gait(&self) -> Gait {
        // An outfielder leaving the ground is heading a ball, not saving one.
        let jump = if self.is_goalkeeper {
            0.0
        } else {
            (self.height / Actors::JUMP_HEIGHT).clamp(0.0, 1.0)
        };
        let grounded = self.grounded();
        let extended = self.extended();
        // A boot, a head, a keeper's throw and a throw-in are one swing routed
        // to four different sets of limbs, so they are four amplitudes off one
        // phase. At most one of them is ever non-zero.
        let kicking = self.kick.filter(|kick| kick.kind == Strike::Boot);
        let nodding = self.kick.filter(|kick| kick.kind == Strike::Head);
        let throwing = self.kick.filter(|kick| kick.kind == Strike::Throw);
        let tossing = self.kick.filter(|kick| kick.kind == Strike::ThrowIn);
        // How he took the goal — the weight, and then which of the four
        // reactions is his. Worked once here rather than four times below,
        // because the four are one draw and have to stay exclusive.
        let taking = self.despair
            * (1.0 - self.carry)
            * (1.0 - self.dive)
            // …and not while he is still on the grass, where `beaten` has
            // him. Every one of the four is a pose for a man standing up,
            // and half a standing slump composed onto a body that is
            // kneeling folds it through another twenty degrees on top of a
            // fold it has already been given twice.
            * (1.0 - grounded)
            * (1.0 - (self.speed / Actors::SPRINT).clamp(0.0, 1.0));
        let (on_head, on_hips, doubled) = self.taking_it();
        // …and what he is doing with a match in which nothing has happened
        // to him, which is most of one.
        let (urging, pointing, standing) = self.gesturing();
        Gait {
            // A man in the air is not running, whatever the ground he is
            // covering says. Fading the run out through this one number
            // stops the stride, the bob, the arm swing and the lean all at
            // once — a diving keeper windmilling his legs is the single
            // thing that would give the whole animation away.
            run: (self.speed / Actors::SPRINT).clamp(0.0, 1.0) * (1.0 - self.dive) * (1.0 - jump),
            phase: self.phase,
            signature: Complexion::carriage(self.id),
            idle: self.idle,
            turn: self.turn,
            look: self.look,
            look_pitch: self.look_pitch,
            // The chest hold, which is only the hold once he is back under
            // his own weight. A ball claimed at full stretch stays in the
            // gloves that claimed it until he has landed on it.
            carry: self.carry * (1.0 - extended),
            dive: self.dive,
            // **The flight pose belongs to the flight.** Given back as he
            // lands rather than held until the recovery lets go of the dive:
            // a keeper who leapt at a cross and caught it is standing on the
            // grass, and holding the extension there left him standing with
            // one leg still scissored up behind him a third of a metre off
            // the turf. What replaces it is the ground-out below, and for a
            // landing that was not a fall, nothing — which is right, because
            // a man who got his feet under him is just standing there.
            stretch: self.stretch * (1.0 - self.settling()),
            grounded,
            lead: self.lead(),
            // Both hands to the ball: he has it, and he is still up there.
            claimed: self.carry * extended,
            // Phase and side come off the swing whichever limb it drives;
            // which limb that is, is the two amplitudes below.
            swing: self.kick.map_or(0.0, |kick| kick.swing),
            foot: self.kick.map_or(0.0, |kick| kick.foot),
            spring: Complexion::spring(self.id),
            power: kicking.map_or(0.0, |kick| kick.power * kick.blend),
            // A keeper rolling the ball out and one hurling it to the halfway
            // line are the same movement; the gentlest of them still has to
            // read as a throw, hence the floor. Same again for the other two.
            throwing: throwing.map_or(0.0, |kick| kick.power.max(0.45) * kick.blend),
            // Not while he is off his feet: a keeper punching a cross clear
            // reads as a high contact, and he is already drawn at full stretch
            // with both arms up, which is the right picture. Nodding at it on
            // the way past is not.
            header: nodding.map_or(0.0, |kick| kick.power.max(0.45) * kick.blend)
                * (1.0 - self.dive),
            throw_in: tossing.map_or(0.0, |kick| kick.power.max(0.55) * kick.blend),
            drive: self.drive,
            // Nobody dribbles the ball off his feet, and nobody dribbles it
            // while he is swinging at it either.
            carrying: self.carrying * (1.0 - self.dive) * (1.0 - jump),
            jump,
            // A keeper is set whenever the ball is near his goal — but not
            // while he is running, off his feet, or holding it: all three are
            // things a man in the set position is by definition not doing.
            set: self.set
                * (1.0 - (self.speed / Actors::SPRINT).clamp(0.0, 1.0))
                * (1.0 - self.dive)
                * (1.0 - self.carry),
            // Both arms go out for the dive and for the leap — but not once
            // the ball is settled in his gloves, where the cradle takes them
            // back in, and not once he is down. A man off the ground is
            // reaching for something; there is no other reason for a
            // footballer to be up there.
            reach: if self.is_goalkeeper {
                self.dive
                    .max((self.height / Actors::REACH_HEIGHT).clamp(0.0, 1.0))
                    * (1.0 - self.carry * (1.0 - extended))
                    * (1.0 - self.settling())
            } else {
                0.0
            },
            // How he took the goal — and only while he is not doing
            // anything else. A man with his hands on his head is not
            // running flat out, is not holding a ball and is not lying on
            // the floor, so each of those takes the slump back off him
            // rather than fighting it. Notably the beaten keeper: he ends
            // the reaction by walking into his own net for the ball, and
            // the cradle has to win from the moment he picks it up.
            despair: taking,
            // The celebration, by contrast, is mostly done at a sprint, so
            // it deliberately survives the run — a man wheeling away with
            // his arms up is the picture.
            elation: self.elation * (1.0 - self.carry) * (1.0 - self.dive),
            // Which of the four reactions is his. Whole weights, mood
            // included — see [`Gait::hands_to_head`].
            hands_to_head: taking * on_head,
            // Hands on the hips are two things at once: how some men take a
            // goal, and what a goalkeeper does standing about. They cannot
            // both be on — `gesturing` is gated on nothing having happened —
            // so the two channels simply add.
            hands_on_hips: (taking * on_hips + standing).clamp(0.0, 1.0),
            doubled_over: taking * doubled,
            urging,
            pointing,
            rising: self.rising(),
            // How far the carriage has him over, as an ANGLE — the one
            // thing the pose has never known about the transform it is
            // drawn under. See [`Gait::over`].
            over: {
                let (pitch, roll) = self.topple();
                Carriage::tilt(pitch, roll).clamp(0.0, 1.0).asin()
            },
            // Down there AND beaten, which is the four seconds after a goal
            // that had no reaction in them at all.
            beaten: self.despair * grounded,
            course: self.underfoot,
            open: self.open,
            // A man off his feet is not taking steps, whatever ground he is
            // covering — the same gate `run` carries, and it has to be here
            // too because this one deliberately bypasses `run`.
            carry_ground: self.carry_ground * (1.0 - self.dive) * (1.0 - jump),
            // The save on his feet, and therefore not while he is off them,
            // not once the ball is in his gloves, and not while he is
            // reacting to a goal.
            save: self.reaction
                * (1.0 - self.dive)
                * (1.0 - jump)
                * (1.0 - self.carry)
                * (1.0 - self.despair.max(self.elation)),
            save_aim: self.aim,
            parry: self.parry,
            keeper: f32::from(self.is_goalkeeper),
        }
    }
}

#[cfg(test)]
mod flight {
    use super::*;

    /// Recorded heights, in metres, sampled every 30 ms through one real
    /// goalkeeper dive.
    ///
    /// Lifted verbatim out of `.dev/match/match_results/dev`: keeper 100 at
    /// 1 720 080 ms, a full-length save with a 0.51 m apex over 630 ms in the
    /// air. Every constant in the flight is titrated against a series like
    /// this rather than against a feeling about how long a dive takes.
    const FULL_LENGTH: [f32; 22] = [
        0.03, 0.12, 0.20, 0.27, 0.33, 0.38, 0.42, 0.46, 0.48, 0.50, 0.51, 0.51, 0.50, 0.48, 0.45,
        0.42, 0.37, 0.32, 0.25, 0.18, 0.10, 0.01,
    ];
    /// And one along the floor: keeper 100 at 292 560 ms, 0.24 m of apex over
    /// 420 ms. The engine's low dives barely leave the grass, and they still
    /// have to read as a man going full length rather than as a hop.
    const ALONG_THE_FLOOR: [f32; 14] = [
        0.04, 0.10, 0.14, 0.18, 0.21, 0.23, 0.24, 0.23, 0.22, 0.19, 0.16, 0.12, 0.07, 0.01,
    ];

    /// Runs a recorded height series through the flight tracker at the
    /// recording's own 30 ms sample rate, and hands back what the rig would
    /// have been drawing at each step.
    fn fly(heights: &[f32], pace: f32, after: f32, tail: usize) -> Vec<(f32, f32, f32)> {
        let mut actor = PlayerActor::new(1, true, true);
        let mut frames = Vec::new();
        let step = 0.03;
        for height in heights.iter().chain(std::iter::repeat_n(&0.0, tail)) {
            actor.height = *height;
            actor.speed = pace;
            // What the recording has him doing once he is down is a separate
            // question from how fast he dived, and the two together are what
            // decide whether he stays there.
            let ground = if *height > Actors::AIRBORNE_FEET {
                pace
            } else {
                after
            };
            actor.track_flight(step, pace, ground, false);
            frames.push((actor.dive, actor.stretch, actor.flat));
        }
        frames
    }

    /// The same flight, carried through to the CARRIAGE it leaves him in.
    ///
    /// `fly` above reports the three scalars; this one hands back the actor
    /// itself, with the tip [`Actors::animate`] would have written while he
    /// was up there — which is the only way to ask where his hips actually
    /// ended up.
    fn land(heights: &[f32], pace: f32, after: f32, tail: usize, way: Vec2) -> PlayerActor {
        let mut actor = PlayerActor::new(1, true, true);
        let step = 0.03;
        for height in heights.iter().chain(std::iter::repeat_n(&0.0, tail)) {
            actor.height = *height;
            actor.speed = pace;
            let ground = if *height > Actors::AIRBORNE_FEET {
                pace
            } else {
                after
            };
            if actor.track_flight(step, pace, ground, false) {
                actor.tip = way.normalize() * actor.flat;
            }
        }
        actor
    }

    #[test]
    #[ignore = "prints; run by hand"]
    fn measure_rising() {
        use crate::body::skeleton;
        for despair in [0.0f32, 1.0] {
            let mut actor = PlayerActor::new(1, true, true);
            actor.despair = despair;
            println!("--- despair {despair}");
            for (frame, height) in FULL_LENGTH
                .iter()
                .chain(std::iter::repeat_n(&0.0, 200))
                .enumerate()
            {
                actor.height = *height;
                let airborne = *height > Actors::AIRBORNE_FEET;
                actor.speed = if airborne { 9.0 } else { 0.0 };
                let ground = if airborne { 9.0 } else { 0.0 };
                if actor.track_flight(0.03, 9.0, ground, false) {
                    actor.tip = Vec2::X * actor.flat;
                }
                let since = frame as i64 - FULL_LENGTH.len() as i64;
                if since < 0 || since % 6 != 0 {
                    continue;
                }
                let (pitch, roll) = actor.topple();
                let carriage = Carriage::placed(pitch, roll, actor.lift());
                let gait = actor.gait();
                let at = |p: Vec3| carriage.transform_point(p).y;
                let low = at(skeleton::glove(-1.0, gait))
                    .min(at(skeleton::glove(1.0, gait)))
                    .min(at(skeleton::boot(-1.0, gait)))
                    .min(at(skeleton::boot(1.0, gait)))
                    .min(at(skeleton::crown(gait)));
                println!(
                    "{:5.2}s dive {:.2} rise {:.2} ground {:.2} hips {:.2} crown {:.2} gloveR {:.2} bootR {:.2} lowest {:.3}",
                    since as f32 * 0.03,
                    actor.dive,
                    actor.rising(),
                    gait.grounded,
                    at(Vec3::new(0.0, Physique::HIP, 0.0)),
                    at(skeleton::crown(gait)),
                    at(skeleton::glove(1.0, gait)),
                    at(skeleton::boot(1.0, gait)),
                    low
                );
            }
        }
    }

    /// **The whole of getting up, drawn frame by frame off a real dive.**
    ///
    /// `dump_sprawl` in `body.rs` renders the pose he LANDS in and the pose
    /// he LIES in; both are stills, and the complaint was never about a
    /// still. *"He falls over and gets up like a robot"* is a complaint
    /// about the seconds in between, which no dump had ever drawn — and
    /// which cannot be drawn from a hand-written `Gait`, because what makes
    /// it a movement is that the carriage, the topple axis, the knees and
    /// the arms are all functions of the same clock.
    ///
    /// So this one replays the recorded height series through the real
    /// `track_flight` and renders whatever the rig would have been showing,
    /// every twelfth frame from the landing on. Two rows: a keeper who SAVED
    /// it, who is up almost at once, and one who was BEATEN, who rolls onto
    /// his front, comes up as far as his knees and stays there.
    ///
    /// ```text
    /// MATCH_FIGURE_DUMP=<dir> cargo test --lib dump_rising -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a file; run by hand when the recovery changes"]
    fn dump_rising() {
        use crate::body::preview::{Canvas, Lens, posed};
        use bevy::asset::Assets;
        use bevy::mesh::Mesh;

        const WIDE: usize = 300;
        const TALL: usize = 300;
        const STEPS: usize = 9;

        let Ok(directory) = std::env::var("MATCH_FIGURE_DUMP") else {
            panic!("set MATCH_FIGURE_DUMP to a directory");
        };
        let mut meshes = Assets::<Mesh>::default();
        let parts = crate::body::BodyParts::new(&mut meshes);

        let mut sheet = vec![0u8; WIDE * STEPS * TALL * 2 * 4];
        // Frames of match time between columns, at the recording's own 30 ms.
        // The beaten row is spaced wider because it is a longer movement —
        // he holds the floor nine times as long (`BEATEN_HOLD`), and at the
        // saved row's spacing the whole sheet is the hold.
        for (row, (despair, every)) in [(0.0f32, 12usize), (1.0, 26)].into_iter().enumerate() {
            let mut actor = PlayerActor::new(1, true, true);
            actor.despair = despair;
            let mut column = 0;
            for (frame, height) in FULL_LENGTH
                .iter()
                .chain(std::iter::repeat_n(&0.0, STEPS * every))
                .enumerate()
            {
                actor.height = *height;
                let airborne = *height > Actors::AIRBORNE_FEET;
                // Nine metres a second going over, and standing still once
                // he is down: the recording is not pulling him anywhere, so
                // nothing is cutting the hold short.
                actor.speed = if airborne { 9.0 } else { 0.0 };
                let ground = if airborne { 9.0 } else { 0.0 };
                if actor.track_flight(0.03, 9.0, ground, false) {
                    actor.tip = Vec2::X * actor.flat;
                }
                // From the landing on, which is the half nothing has drawn.
                let since = frame as i64 - FULL_LENGTH.len() as i64;
                if since < 0 || since as usize % every != 0 || column >= STEPS {
                    continue;
                }
                let (pitch, roll) = actor.topple();
                let mut canvas = Canvas::new(WIDE, TALL);
                let lens = Lens {
                    bearing: 2.4,
                    bottom: -0.15,
                    top: 1.85,
                };
                posed(
                    &mut canvas,
                    &lens,
                    &meshes,
                    &parts,
                    actor.gait(),
                    Carriage::placed(pitch, roll, actor.lift()),
                    true,
                );
                let pixels = canvas.pixels();
                let stride = WIDE * STEPS;
                for line in 0..TALL {
                    let from = line * WIDE * 4;
                    let to = ((row * TALL + line) * stride + column * WIDE) * 4;
                    sheet[to..to + WIDE * 4].copy_from_slice(&pixels[from..from + WIDE * 4]);
                }
                column += 1;
            }
        }

        let path = std::path::Path::new(&directory).join("rising.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", WIDE * STEPS, TALL * 2, path.display());
    }

    /// **A keeper on the grass is lying ON it.**
    ///
    /// The pose the camera holds on for four seconds after a goal
    /// (`BEATEN_HOLD`), and the one nothing checked. [`Actors::SPRAWL_ANGLE`]
    /// is the angle of a body in flight and a real launch leaves him 11–26°
    /// short of it, so a landed keeper lay on an invisible slope with his
    /// hips a third of a metre up and his head held clear of the turf —
    /// reported, correctly, as not looking real. Asserted as positions,
    /// because an angle can be right while the body it belongs to is still
    /// propping itself up.
    #[test]
    fn a_keeper_on_the_grass_is_lying_on_it() {
        use crate::body::skeleton;

        let actor = land(&FULL_LENGTH, 9.0, 0.0, 8, Vec2::X);
        let (pitch, roll) = actor.topple();
        let carriage = Carriage::placed(pitch, roll, actor.lift());
        let gait = actor.gait();
        let at = |part: Vec3| carriage.transform_point(part);

        let hips = at(Vec3::new(0.0, Physique::HIP, 0.0));
        assert!(
            (0.14..0.26).contains(&hips.y),
            "his hips are at {:.2} m, which is not a man on the ground",
            hips.y
        );
        // A body on the ground is at the angle of the ground: head, hips and
        // boots within a body's own thickness of each other.
        let crown = at(skeleton::crown(gait));
        assert!(
            (crown.y - hips.y).abs() < 0.16,
            "his head is {:.2} m above his hips: he is propped up, not lying down",
            crown.y - hips.y
        );
        for side in [-1.0f32, 1.0] {
            let boot = at(skeleton::boot(side, gait));
            assert!(
                (boot.y - hips.y).abs() < 0.20,
                "a boot is {:.2} m off his own hips",
                boot.y - hips.y
            );
        }
        // And none of him is under the turf.
        for (what, part) in [
            ("crown", skeleton::crown(gait)),
            ("left boot", skeleton::boot(-1.0, gait)),
            ("right boot", skeleton::boot(1.0, gait)),
            ("left glove", skeleton::glove(-1.0, gait)),
            ("right glove", skeleton::glove(1.0, gait)),
        ] {
            assert!(
                at(part).y > 0.0,
                "his {what} is {:.3} m under the grass",
                -at(part).y
            );
        }
    }

    /// And he gets off the floor the way a man does: pushing himself up.
    ///
    /// The trap the commitment sets. Read through a decaying `stretch` it
    /// falls back through its own band partway through the recovery, the
    /// landing angle it is aiming at collapses to nothing with it, and a
    /// keeper who was flat on the turf is upright a quarter of a second
    /// later — which is a man being deleted and redrawn standing, not one
    /// getting up.
    ///
    /// ⚠ Measured as the COMPOSED tilt and not as the roll, which is what it
    /// used to read. He rolls onto his front on the way up
    /// ([`Actors::ROLLS_OVER`]), so most of the way through the recovery a
    /// keeper who dived flat across his goal has hardly any roll left and is
    /// still nowhere near standing — the number went to sixteen degrees
    /// while the man was still face down on the turf. The angle between his
    /// own up-axis and the world's is the quantity the claim was always
    /// about, and it is the one [`Carriage::placed`] settles him by.
    #[test]
    fn he_comes_up_off_the_floor_in_his_own_time() {
        let mut actor = PlayerActor::new(1, true, true);
        let mut over = Vec::new();
        for height in FULL_LENGTH.iter().chain(std::iter::repeat_n(&0.0, 90)) {
            actor.height = *height;
            actor.speed = 9.0;
            let ground = if *height > Actors::AIRBORNE_FEET {
                9.0
            } else {
                0.0
            };
            if actor.track_flight(0.03, 9.0, ground, false) {
                actor.tip = Vec2::X * actor.flat;
            }
            let (pitch, roll) = actor.topple();
            let upright = (Quat::from_rotation_x(pitch) * Quat::from_rotation_z(roll) * Vec3::Y).y;
            over.push(upright.clamp(-1.0, 1.0).acos());
        }

        let landed = FULL_LENGTH.len();
        assert!(
            over[landed + 8] > 1.5,
            "not flat through the hold: {:.0}°",
            over[landed + 8].to_degrees()
        );
        // A third of a second into the recovery he is on his way up and a
        // long way from standing.
        let rising = over[landed + 21];
        assert!(
            (0.7..1.4).contains(&rising),
            "off the floor in one movement: {:.0}° a third of a second in",
            rising.to_degrees()
        );
        assert!(
            over[landed + 80] < 0.15,
            "never gets up: {:?}",
            over[landed + 80]
        );
        // …and every step of it downward, once it has turned.
        for pair in over[landed + 12..].windows(2) {
            assert!(pair[1] <= pair[0] + 1e-4, "he goes back down: {pair:?}");
        }
    }

    /// **He gets up off the grass, and every part of him stays out of it.**
    ///
    /// The claim behind the whole get-up, and the one nothing had ever
    /// measured: sweeping a real recovery frame by frame, **his boots spent
    /// it 15–20 cm UNDER the pitch**, because [`Carriage::placed`] settles a
    /// figure by how far from upright it is — which is right for a body
    /// lying on its side and hopeless for one halfway to its feet — and the
    /// legs were drawn hanging from hips that were still on the floor. It
    /// was true before any of this was drawn, and it was invisible because
    /// nobody looked at the frames between the two poses that had tests.
    ///
    /// The second half is the positive claim: somewhere in the middle of it
    /// there is a frame where his hands are ON the turf and his head is
    /// well off it. That is a man pushing himself up rather than a plank
    /// rotating about its hips, and it is the difference the report was
    /// about.
    #[test]
    fn he_gets_up_off_the_grass_rather_than_swinging_upright() {
        use crate::body::skeleton;

        let mut actor = PlayerActor::new(1, true, true);
        let mut pushing = false;
        for (frame, height) in FULL_LENGTH
            .iter()
            .chain(std::iter::repeat_n(&0.0, 120))
            .enumerate()
        {
            actor.height = *height;
            let airborne = *height > Actors::AIRBORNE_FEET;
            actor.speed = if airborne { 9.0 } else { 0.0 };
            let ground = if airborne { 9.0 } else { 0.0 };
            if actor.track_flight(0.03, 9.0, ground, false) {
                actor.tip = Vec2::X * actor.flat;
            }
            // From the moment he has SETTLED, not from touchdown. The
            // landing itself is a scissored body arriving at the angle it
            // flew at, it is over in [`Actors::GROUNDING`], and it is
            // `a_keeper_on_the_grass_is_lying_on_it`'s subject; this one is
            // about the seconds after it.
            if (frame as f32 - FULL_LENGTH.len() as f32) * 0.03 < Actors::GROUNDING {
                continue;
            }
            let (pitch, roll) = actor.topple();
            let carriage = Carriage::placed(pitch, roll, actor.lift());
            let gait = actor.gait();
            let at = |part: Vec3| carriage.transform_point(part).y;
            let crown = at(skeleton::crown(gait));
            for (what, part) in [
                ("crown", skeleton::crown(gait)),
                ("left boot", skeleton::boot(-1.0, gait)),
                ("right boot", skeleton::boot(1.0, gait)),
                ("left glove", skeleton::glove(-1.0, gait)),
                ("right glove", skeleton::glove(1.0, gait)),
            ] {
                assert!(
                    at(part) > -0.05,
                    "his {what} is {:.3} m under the pitch getting up, \
                     {:.2} s after landing",
                    -at(part),
                    (frame - FULL_LENGTH.len()) as f32 * 0.03
                );
            }
            // Hands down, head up: he is pushing.
            let hands = at(skeleton::glove(-1.0, gait)).min(at(skeleton::glove(1.0, gait)));
            if hands < 0.22 && crown > 0.55 {
                pushing = true;
            }
        }
        assert!(
            pushing,
            "he never gets a hand on the turf: the recovery is still a rotation"
        );
    }

    /// **And he rolls onto his front to do it.**
    ///
    /// A body lying on its side does not stand up sideways, and the topple
    /// used to come back up about exactly the axis it went down on. Measured
    /// as the share of the tilt that is PITCH rather than roll: a keeper who
    /// dived flat across his goal is all roll on the floor and mostly pitch
    /// by the time he is on his knees.
    #[test]
    fn he_rolls_onto_his_front_before_he_gets_up() {
        let flat = land(&FULL_LENGTH, 9.0, 0.0, 8, Vec2::X);
        let (pitch, roll) = flat.topple();
        assert!(
            pitch.abs() < 0.15 && roll.abs() > 1.4,
            "he did not land on his side: pitch {pitch:.2}, roll {roll:.2}"
        );

        let rising = land(&FULL_LENGTH, 9.0, 0.0, 24, Vec2::X);
        let (pitch, roll) = rising.topple();
        assert!(
            pitch.abs() > roll.abs(),
            "he comes up about the axis he went down on: pitch {pitch:.2}, roll {roll:.2}"
        );
    }

    /// **A beaten keeper does not lie down and then stand up.** He comes as
    /// far as his knees and stops there, which is the picture, and which the
    /// recovery — one exponential running to zero — had no way to draw.
    ///
    /// Asserted as the hips: on the floor they are at a half hip-breadth,
    /// kneeling they are at half a metre, standing at 0.95. He holds the
    /// middle one, and holds it for seconds rather than passing through it.
    #[test]
    fn a_beaten_keeper_stops_on_his_knees() {
        let hips = |tail: usize, despair: f32| {
            let mut actor = PlayerActor::new(1, true, true);
            actor.despair = despair;
            for height in FULL_LENGTH.iter().chain(std::iter::repeat_n(&0.0, tail)) {
                actor.height = *height;
                let airborne = *height > Actors::AIRBORNE_FEET;
                actor.speed = if airborne { 9.0 } else { 0.0 };
                let ground = if airborne { 9.0 } else { 0.0 };
                if actor.track_flight(0.03, 9.0, ground, false) {
                    actor.tip = Vec2::X * actor.flat;
                }
            }
            let (pitch, roll) = actor.topple();
            Carriage::placed(pitch, roll, actor.lift())
                .transform_point(Vec3::new(0.0, Physique::HIP, 0.0))
                .y
        };
        // Two seconds down — still flat out, where a keeper who saved it is
        // already standing.
        assert!(
            hips(60, 1.0) < 0.30,
            "he did not stay down: {:.2}",
            hips(60, 1.0)
        );
        assert!(hips(60, 0.0) > 0.85, "a keeper who saved it is still down");
        // Then on his knees, and there four seconds later.
        for tail in [100, 160, 220] {
            let kneeling = hips(tail, 1.0);
            assert!(
                (0.40..0.65).contains(&kneeling),
                "he is not on his knees {:.1} s after landing: hips at {kneeling:.2} m",
                (tail - FULL_LENGTH.len()) as f32 * 0.03
            );
        }
    }

    /// …but a keeper who leapt at a cross and landed on his feet never
    /// kneels at all. The same rule [`PlayerActor::committed`] carries for
    /// the sprawl, and the same failure it exists to stop: a pose for a man
    /// on the floor applied to one who is standing on the grass.
    #[test]
    fn a_standing_leap_never_kneels() {
        for tail in [4, 12, 24, 48] {
            let leapt = land(&FULL_LENGTH, 3.0, 0.0, tail, Vec2::X);
            let gait = leapt.gait();
            assert!(
                gait.kneeling() < 0.12 && gait.propping() < 0.12,
                "he drops to his knees catching a corner: kneel {:.2}, prop {:.2}",
                gait.kneeling(),
                gait.propping()
            );
        }
    }

    /// …but a man who went up for a cross lands on his FEET.
    ///
    /// The other half of the same rule, and the reason the landing is scaled
    /// by how far over he already was rather than simply applied: a keeper
    /// leaping at a corner is barely leaning, and taking every landing to a
    /// right angle would lay him out flat every time he caught one.
    ///
    /// The boots are the assertion that matters. Everything on the floor —
    /// the curl, the tucked legs, the arms folding in — used to be applied to
    /// his landing too, which for an upright body is a deep squat: he stood
    /// there for half a second a foot into the turf, 5.5 times a match.
    #[test]
    fn a_standing_leap_is_not_a_fall() {
        use crate::body::skeleton;

        let leapt = land(&FULL_LENGTH, 3.0, 0.0, 8, Vec2::X);
        let (_, roll) = leapt.topple();
        let dived = land(&ALONG_THE_FLOOR, 9.0, 0.0, 8, Vec2::X);
        let (_, flat) = dived.topple();
        assert!(
            roll.abs() < 0.20,
            "a standing leap left him leaning at {:.0}°",
            roll.abs().to_degrees()
        );
        assert!(
            flat.abs() > 1.45,
            "a floor dive left him propped up at {:.0}° off flat",
            90.0 - flat.abs().to_degrees()
        );

        let carriage = Carriage::placed(0.0, roll, leapt.lift());
        let gait = leapt.gait();
        for side in [-1.0f32, 1.0] {
            let boot = carriage.transform_point(skeleton::boot(side, gait));
            assert!(
                boot.y.abs() < 0.03,
                "he landed from a leap with a boot at {:.2} m",
                boot.y
            );
        }
    }

    /// The whole point of the rewrite: a dive is not one pose held for half a
    /// second. Over a recorded flight the extension has to still be opening
    /// out well after take-off, and it has to be finished by the apex.
    #[test]
    fn the_flight_is_not_one_frame() {
        let frames = fly(&FULL_LENGTH, 9.0, 9.0, 0);
        // Committed within a couple of frames — the run cycle stops dead.
        assert!(frames[1].0 > 0.4, "still running: {:?}", frames[1]);
        // But nowhere near extended.
        assert!(frames[1].1 < 0.35, "snapped flat: {:?}", frames[1]);
        // A third of the way up, halfway out.
        assert!(
            (0.35..0.85).contains(&frames[4].1),
            "not opening out: {:?}",
            frames[4]
        );
        // Full stretch by the apex, which the recording puts at frame 10.
        assert!(frames[10].1 > 0.97, "never extends: {:?}", frames[10]);
        // And having got there it stays there for the rest of the flight: a
        // keeper does not fold himself back up in mid-air.
        assert!(frames[20].1 > 0.97);
    }

    /// He stays down after he lands, and then gets up. Not a bounce, and not
    /// a body left lying on the grass either.
    #[test]
    fn he_lies_there_and_then_gets_up() {
        // Landed, and standing still: nothing in the recording is pulling him
        // anywhere, so he takes his time.
        let frames = fly(&FULL_LENGTH, 9.0, 0.0, 60);
        let landed = FULL_LENGTH.len();
        assert!(
            frames[landed + 4].0 > 0.95,
            "up too soon: {:?}",
            frames[landed + 4]
        );
        assert!(
            frames[landed + 20].0 < 0.55,
            "still on the grass: {:?}",
            frames[landed + 20]
        );
        assert!(
            frames[landed + 59].0 < 0.1,
            "never gets up: {:?}",
            frames[landed + 59]
        );

        // And a keeper the recording has running again is not dragged along
        // the turf on his side waiting for the hold to expire.
        let hurried = fly(&FULL_LENGTH, 9.0, 6.0, 60);
        assert!(
            hurried[landed + 8].0 < frames[landed + 8].0 - 0.2,
            "hold not cut by the run: {:?} vs {:?}",
            hurried[landed + 8],
            frames[landed + 8]
        );
    }

    /// How far he goes over is read off the angle he left the ground at, and
    /// that is what separates the two saves a keeper makes.
    #[test]
    fn the_launch_angle_decides_the_topple() {
        // Thrown along the floor at 9 m/s: 2 m/s of rise against that is
        // about 12°, which is a body travelling all but horizontally.
        let flat = fly(&ALONG_THE_FLOOR, 9.0, 9.0, 0)[6].2;
        assert!(flat > 0.85, "a floor dive drawn upright: {flat}");

        // The same keeper going straight up at a corner — 3 m/s of rise
        // against 3 m/s of ground speed, a 45° take-off — leans, and does not
        // topple.
        let leap = fly(&FULL_LENGTH, 3.0, 3.0, 0)[6].2;
        assert!(
            (0.25..0.75).contains(&leap),
            "a standing leap drawn as a sprawl: {leap}"
        );
        assert!(flat > leap + 0.2);
    }

    /// Scrubbing the playhead is not a trajectory, so nothing may be
    /// integrated across it.
    #[test]
    fn a_seek_starts_the_flight_again() {
        let mut actor = PlayerActor::new(1, true, true);
        for height in FULL_LENGTH.iter().take(12) {
            actor.height = *height;
            actor.track_flight(0.03, 9.0, 9.0, false);
        }
        assert!(actor.air > 0.3 && actor.stretch > 0.97);
        actor.height = 0.0;
        actor.track_flight(0.03, 9.0, 9.0, true);
        assert_eq!(
            (actor.air, actor.down, actor.dive, actor.stretch),
            (0.0, 0.0, 0.0, 0.0)
        );
    }

    /// **Only a goalkeeper with nothing to do gestures**, and that gate is
    /// the whole licence for the one behaviour in this crate that is not
    /// derived from the recording.
    ///
    /// Inside the window — ball at the other end, standing still, nothing in
    /// his hands, no goal just scored — the recording says only that he is
    /// standing there, and a man standing to attention for eighty minutes is
    /// the one reading of that which is definitely wrong. Outside it, the
    /// recording is saying something and this must not argue with it.
    #[test]
    fn only_an_idle_keeper_organises_anybody() {
        let idle = |set: &dyn Fn(&mut PlayerActor)| {
            let mut actor = PlayerActor::new(4, true, true);
            actor.clock = 2.0;
            set(&mut actor);
            let (urging, pointing, hips) = actor.gesturing();
            urging + pointing.abs() + hips
        };
        // Somewhere in his own cycle he is doing something.
        let busy: f32 = (0..60)
            .map(|step| {
                idle(&|actor: &mut PlayerActor| {
                    actor.clock = step as f32 * Actors::GESTURE_CYCLE / 60.0;
                })
            })
            .sum();
        assert!(busy > 1.0, "a keeper with a whole match off never moves");

        let gates: [(&str, &dyn Fn(&mut PlayerActor)); 6] = [
            ("the ball is at his goal", &|a: &mut PlayerActor| {
                a.set = 1.0
            }),
            ("he is running", &|a: &mut PlayerActor| a.speed = 4.0),
            ("he has the ball", &|a: &mut PlayerActor| a.carry = 1.0),
            ("he is on the floor", &|a: &mut PlayerActor| a.dive = 1.0),
            ("he has just conceded", &|a: &mut PlayerActor| {
                a.despair = 1.0
            }),
            ("a shot is coming", &|a: &mut PlayerActor| a.reaction = 1.0),
        ];
        for (what, gate) in gates {
            for step in 0..60 {
                let moving = idle(&|actor: &mut PlayerActor| {
                    actor.clock = step as f32 * Actors::GESTURE_CYCLE / 60.0;
                    gate(actor);
                });
                assert!(
                    moving < 1e-3,
                    "he is waving his arms about while {what}: {moving:.2}"
                );
            }
        }

        // And nobody else on the pitch does it at all.
        let mut outfielder = PlayerActor::new(4, false, true);
        for step in 0..60 {
            outfielder.clock = step as f32 * Actors::GESTURE_CYCLE / 60.0;
            let (urging, pointing, hips) = outfielder.gesturing();
            assert_eq!((urging, pointing, hips), (0.0, 0.0, 0.0));
        }
    }

    /// **The two men in the picture must not take the goal the same way.**
    ///
    /// The reaction was a coin flip off `Complexion::carriage`, which every
    /// goalkeeper overrode anyway — so both keepers on the pitch, and half
    /// the outfielders, did the identical thing. Four reactions off their
    /// own salt, and a squad splits across all of them.
    #[test]
    fn a_conceding_eleven_reacts_four_ways() {
        let mut seen = [0u32; 4];
        for id in 1..40u32 {
            let mut actor = PlayerActor::new(id, id % 12 == 0, true);
            actor.despair = 1.0;
            let (head, hips, doubled) = actor.taking_it();
            let which = match (head, hips, doubled) {
                (1.0, _, _) => 0,
                (_, 1.0, _) => 1,
                (_, _, 1.0) => 2,
                _ => 3,
            };
            seen[which] += 1;
            // Exactly one of them, always: the four are picked, not blended.
            assert!(
                (head + hips + doubled - 1.0).abs() < 1e-6 || head + hips + doubled == 0.0,
                "player {id} is doing two things at once: {head} {hips} {doubled}"
            );
        }
        for (which, count) in seen.iter().enumerate() {
            assert!(*count > 0, "nobody in the squad takes it reaction {which}");
        }
    }

    /// An outfielder heading a ball is a metre off the ground and is still
    /// not diving.
    #[test]
    fn an_outfielder_never_dives() {
        let mut actor = PlayerActor::new(2, false, true);
        for height in [0.2, 0.6, 1.0, 1.1, 0.9, 0.4, 0.0] {
            actor.height = height;
            assert!(!actor.track_flight(0.03, 5.0, 5.0, false));
            assert_eq!((actor.dive, actor.stretch), (0.0, 0.0));
        }
    }
}

#[cfg(test)]
mod kicks {
    use super::*;
    use crate::replay::{Sample, Track};

    /// A real pass out of `.dev/match/match_results/dev`, in engine units at
    /// the recording's own 30 ms step: the ball drifting at about two metres a
    /// second for a fifth of a second, then struck at 15.2 m/s.
    ///
    /// Contact is the sample at t = 180. Everything the detector claims is
    /// measured against that.
    const A_PASS: [(u32, f32, f32); 14] = [
        (0, 8.0, 269.5),
        (30, 7.6, 269.6),
        (60, 7.1, 269.8),
        (90, 6.6, 270.0),
        (120, 6.1, 270.2),
        (150, 5.7, 270.4),
        (180, 5.2, 270.5),
        (210, 2.3, 272.7),
        (240, 2.0, 272.3),
        (270, 2.9, 270.3),
        (300, 3.9, 268.3),
        (330, 4.8, 266.3),
        (360, 5.7, 264.3),
        (390, 6.6, 262.3),
    ];

    fn track(rows: &[(u32, f32, f32)]) -> Track {
        let mut track = Track::default();
        track.merge(
            rows.iter()
                .map(|&(t, x, y)| Sample { t, x, y, z: 0.0 })
                .collect(),
        );
        track
    }

    /// A 20 m/s strike off the deck in the middle of the pitch, `delay`
    /// seconds away.
    fn coming(delay: f32) -> Option<Impact> {
        Some(Impact {
            by: 7,
            contact: Contact {
                at: Vec3::ZERO,
                velocity: Vec3::new(0.0, 0.0, 20.0),
                delay,
                kind: Strike::Boot,
            },
        })
    }

    /// The kick is found before it happens, and the countdown to it is a
    /// countdown — which is the whole reason for reading ahead at all.
    #[test]
    fn a_kick_is_seen_coming() {
        let mut ball = track(&A_PASS);
        // Too early: contact is 150 ms off and the window only reaches 120.
        assert!(Actors::next_impact(&mut ball, 30.0).is_none());

        // From here on it is in view, and the wait shortens by exactly the
        // time that passes.
        for (now, expected) in [(60.0, 0.12), (90.0, 0.09), (120.0, 0.06), (180.0, 0.0)] {
            let contact =
                Actors::next_impact(&mut ball, now).unwrap_or_else(|| panic!("missed at {now}"));
            assert!(
                (contact.delay - expected).abs() < 1e-3,
                "at {now} ms the wait is {}, not {expected}",
                contact.delay
            );
            assert!(
                (contact.velocity.length() - 15.2).abs() < 1.0,
                "struck at {} m/s",
                contact.velocity.length()
            );
            // A rolling ball in the middle of the pitch, met on the deck: a
            // kick and nothing more exotic.
            assert!(contact.kind == Strike::Boot);
        }

        // And once it has gone there is nothing left ahead to find.
        assert!(Actors::next_impact(&mut ball, 260.0).is_none());
    }

    /// A ball that is merely rolling is not a kick, however long it rolls for.
    #[test]
    fn rolling_is_not_kicking() {
        let drift: Vec<(u32, f32, f32)> = (0..14)
            .map(|i| (i * 30, 8.0 - i as f32 * 0.45, 269.5 + i as f32 * 0.1))
            .collect();
        let mut ball = track(&drift);
        for step in 0..8 {
            assert!(Actors::next_impact(&mut ball, step as f64 * 30.0).is_none());
        }
    }

    /// The leg arrives when the ball leaves. Everything before contact is
    /// locked to the recording; everything after it runs on the clock, because
    /// by then there is nothing left to be in step with.
    #[test]
    fn the_swing_arrives_with_the_ball() {
        let mut actor = PlayerActor::new(7, false, true);

        // Wound up at the far end of the window and through the ball at zero.
        actor.swing_leg(coming(0.12), 0.03, false);
        assert!(actor.kick.unwrap().swing < -0.7, "not wound up");
        for delay in [0.09, 0.06, 0.03, 0.0] {
            actor.swing_leg(coming(delay), 0.03, false);
        }
        let contact = actor.kick.unwrap();
        assert!(contact.swing.abs() < 1e-3, "boot late: {}", contact.swing);
        assert!(contact.blend > 0.99, "swing never took over");
        assert!(contact.power > 0.5, "a 20 m/s strike is not a tap");

        // Then the follow through, on the clock, and gone at the end of it.
        actor.swing_leg(None, 0.15, false);
        assert!(actor.kick.unwrap().swing > 0.45);
        actor.swing_leg(None, 0.15, false);
        assert!(actor.kick.is_none(), "the swing never finishes");
    }

    /// A kick armed with almost no warning still has to arrive rather than
    /// appear: a leg that snaps into position inside one frame is a glitch,
    /// not a backswing.
    #[test]
    fn a_late_kick_still_eases_in() {
        let mut actor = PlayerActor::new(7, false, true);
        actor.swing_leg(coming(0.0), 1.0 / 60.0, false);
        let kick = actor.kick.unwrap();
        assert!(kick.blend < 0.4, "no onset ramp: {}", kick.blend);
        assert!(kick.swing.abs() < 1e-3);
    }

    /// Scrubbing the playhead cancels the swing rather than leaving a leg in
    /// the air.
    #[test]
    fn a_seek_cancels_the_swing() {
        let mut actor = PlayerActor::new(7, false, true);
        actor.swing_leg(coming(0.06), 0.03, false);
        assert!(actor.kick.is_some());
        actor.swing_leg(None, 0.03, true);
        assert!(actor.kick.is_none());
    }

    /// The three things that are not a kick, told apart from a kick by the
    /// ball's own track and nothing else.
    ///
    /// Deliberately checked from BOTH sides: the two special cases have to
    /// fire on the geometry that produces them, and — much more importantly —
    /// they have to stay off everything else, because there are fourteen real
    /// strikes a minute and any one of them drawn as a throw-in is worse than
    /// all of them drawn as kicks.
    #[test]
    fn the_ball_says_what_hit_it() {
        let middle = Field::HALF_WIDTH - 20.0;

        // A ball met above the shoulder is a header.
        assert!(
            Actors::strike_kind(Vec3::new(0.0, 1.9, middle), 4.0) == Strike::Head,
            "a ball two metres up is not being kicked"
        );
        // A chest-high volley is not.
        assert!(Actors::strike_kind(Vec3::new(0.0, 1.30, middle), 4.0) == Strike::Boot);
        assert!(Actors::strike_kind(Vec3::new(0.0, 0.0, middle), 0.0) == Strike::Boot);

        // A dead ball on the touchline is a throw-in. The engine sets one down
        // a quarter of a metre inside the line, which is what this is.
        let touchline = Field::HALF_WIDTH - 0.25;
        for side in [-1.0f32, 1.0] {
            assert!(
                Actors::strike_kind(Vec3::new(12.0, 0.0, side * touchline), 0.0) == Strike::ThrowIn,
                "no throw-in at z = {}",
                side * touchline
            );
        }
        // But a ball played along the touchline at pace is a pass, and a dead
        // ball anywhere else is a free kick — which is taken with a boot.
        assert!(
            Actors::strike_kind(Vec3::new(12.0, 0.0, touchline), 6.0) == Strike::Boot,
            "a ball already travelling is nobody's throw-in"
        );
        assert!(Actors::strike_kind(Vec3::new(12.0, 0.0, middle), 0.0) == Strike::Boot);
        // And a cross whipped in from the touchline is not one either: it is
        // the height that decides, and a throw-in is taken off the floor.
        assert!(Actors::strike_kind(Vec3::new(12.0, 1.8, touchline), 0.0) == Strike::Head);
    }

    /// A man with the ball in his hands does not turn to look at it.
    ///
    /// The engine snaps a held ball to the middle of the man holding it, so
    /// the bearing to it is the difference between two independently rounded
    /// positions. Measured over a real recording that gap is one quantisation
    /// step — 1.25 cm — at the 90th percentile, and the heading it implies
    /// swings a median of 45° per frame. The ball is then drawn a third of a
    /// metre in front of that, so it orbited him and spent half its time at
    /// his back.
    #[test]
    fn a_held_ball_does_not_turn_the_man_holding_it() {
        let mut ball = BallState {
            on_pitch: true,
            // A quantisation step off his own position, at carry height: what
            // a real recording of a keeper with the ball in his gloves looks
            // like.
            position: Vec3::new(0.0125, 1.15, 0.0),
            ..Default::default()
        };
        let keeper = PlayerActor::new(1, true, true);

        assert_eq!(
            Actors::facing(&keeper, &ball, Vec3::ZERO, Vec3::ZERO, true),
            Vec3::ZERO,
            "a rounding error turned a man holding the ball"
        );
        // And the same frame WITHOUT the hold is the bug this replaced: a
        // bearing does get written, off a centimetre of rounding.
        let loose = Actors::facing(&keeper, &ball, Vec3::ZERO, Vec3::ZERO, false);
        assert!(Vec3::new(loose.x, 0.0, loose.z).length() > 0.0);

        // Holding it does not freeze him, though: a keeper running the ball
        // out still faces where he is going.
        let mut running = PlayerActor::new(1, true, true);
        running.speed = 4.0;
        let travel = Actors::facing(&running, &ball, Vec3::ZERO, Vec3::new(0.0, 0.0, 0.6), true);
        assert!(travel.z > 0.0 && travel.x.abs() < 1e-6);

        // Nor does it stop him opening up to the throw he is about to make.
        let mut throwing = PlayerActor::new(1, true, true);
        throwing.swing_leg(coming(0.09), 0.03, false);
        let aimed = Actors::facing(&throwing, &ball, Vec3::ZERO, Vec3::ZERO, true);
        assert!(aimed.length() > 0.5, "he never turns to throw: {aimed:?}");

        // A ball that is NOT in anybody's hands is still watched.
        ball.position = Vec3::new(9.0, 0.0, 4.0);
        let watching = Actors::facing(&keeper, &ball, Vec3::ZERO, Vec3::ZERO, false);
        assert!((watching.x - 9.0).abs() < 1e-6 && (watching.z - 4.0).abs() < 1e-6);
    }

    /// A throw-in is taken with the ball in both hands, and it lets go of it.
    #[test]
    fn a_thrower_holds_the_ball_until_he_lets_go() {
        let mut actor = PlayerActor::new(7, false, true);
        let throw_in = |delay: f32| {
            Some(Impact {
                by: 7,
                contact: Contact {
                    at: Vec3::new(12.0, 0.0, Field::HALF_WIDTH - 0.25),
                    velocity: Vec3::new(0.0, 3.0, -11.0),
                    delay,
                    kind: Strike::ThrowIn,
                },
            })
        };

        actor.swing_leg(throw_in(0.12), 0.03, false);
        assert!(actor.throwing_in(), "the ball is not in his hands");
        actor.swing_leg(throw_in(0.0), 0.03, false);
        assert!(actor.throwing_in(), "he drops it before he throws it");
        // Contact, and it has gone.
        actor.swing_leg(None, 0.05, false);
        assert!(!actor.throwing_in(), "the ball never leaves him");

        // Nobody kicking one is holding it.
        let mut booting = PlayerActor::new(7, false, true);
        booting.swing_leg(coming(0.06), 0.03, false);
        assert!(!booting.throwing_in());
        assert!(booting.throwing_hand().is_none());

        // And a keeper winding up a throw carries the ball in the hand that
        // is doing the throwing, not against the chest it has left.
        let mut keeper = PlayerActor::new(1, true, true);
        keeper.carry = 1.0;
        keeper.swing_leg(coming(0.09), 0.03, false);
        let side = keeper.throwing_hand().expect("no throwing hand");
        assert!(!keeper.throwing_in(), "a keeper's throw is not a throw-in");
        let ball = Physique::palm(side, keeper.gait());
        let chest = Physique::CRADLE;
        assert!(
            ball.distance(chest) > 0.25,
            "the ball never leaves his chest: {ball:?}"
        );
        assert!(ball.y > chest.y, "it never gets above the cradle: {ball:?}");
    }

    /// **The man walking the ball back to the centre circle is carrying it.**
    ///
    /// The other outfielder with it in his hands, and the one nothing knew
    /// about: the ball was drawn at its recorded position, which
    /// `GoalCelebration::move_ball` puts on his own centreline — so it floated
    /// inside his ribcage and stuck out of his back as he walked — while the
    /// slump, which nothing was taking off him, had his hands on his head. A
    /// man carrying a football does not have his hands on his head.
    #[test]
    fn the_man_walking_it_back_is_carrying_it() {
        // A quantisation step off his own position at the height the
        // celebration parks it: the whole walk to the halfway line.
        assert!(Actors::in_his_hands(0.0125, 1.05, false));
        // A ball played at somebody's chest in open play is passing him.
        assert!(!Actors::in_his_hands(0.42, 1.05, false));
        // Nothing on the floor is in anybody's hands, however close it is.
        assert!(!Actors::in_his_hands(0.0, 0.11, false));
        // And a keeper's gather still reaches out to arm's length, because
        // he holds it there through a dive.
        assert!(Actors::in_his_hands(0.42, 1.15, true));

        // Then the pose: the hold has to beat the reaction to the goal.
        let mut actor = PlayerActor::new(7, false, true);
        actor.despair = 1.0;
        assert!(
            actor.gait().despair > 0.9,
            "he has just conceded and is not showing it"
        );
        actor.carry = 1.0;
        let carrying = actor.gait();
        assert_eq!(
            carrying.despair, 0.0,
            "he is walking the ball back with his hands on his head"
        );
        assert!(carrying.carry > 0.9, "and not holding it either");
    }

    /// The hold travels with the arms rather than sitting at one point.
    ///
    /// [`Physique::hands`] is asked of the rig instead of written down as a
    /// constant precisely because a throw-in has no single hold point — and
    /// this is that claim, as positions.
    #[test]
    fn the_throw_carries_the_ball_over_his_head() {
        let posed = |swing: f32| Physique::hands(crate::body::skeleton::tossing(swing));

        let cocked = posed(-0.6);
        let over = posed(-0.2);
        let released = posed(0.2);
        // Behind him, up over his head, then out in front — and on the centre
        // line the whole way, because it is held in both hands.
        assert!(cocked.z < 0.0, "not taken back: {cocked:?}");
        assert!(over.y > cocked.y, "never comes up: {over:?}");
        assert!(
            released.z > cocked.z + 0.5,
            "never comes through: {released:?}"
        );
        for hold in [cocked, over, released] {
            assert!(hold.x.abs() < 0.02, "one-handed: {hold:?}");
        }
    }

    /// A keeper about to send the ball upfield throws it, whatever the
    /// geometry of the moment says — he has it in his hands.
    #[test]
    fn a_keeper_with_the_ball_throws_it() {
        let mut actor = PlayerActor::new(1, true, true);
        actor.carry = 1.0;
        actor.swing_leg(coming(0.06), 0.03, false);
        assert!(actor.kick.unwrap().kind == Strike::Throw);

        // And once it has left him it is his boot again.
        let mut kicking = PlayerActor::new(1, true, true);
        kicking.swing_leg(coming(0.06), 0.03, false);
        assert!(kicking.kick.unwrap().kind == Strike::Boot);
    }
}

/// **The save he makes on his feet**, read off the ball's own track ahead of
/// the playhead — see [`Save`].
///
/// Measured over a recorded match, 84% of the balls that arrive at a keeper
/// at pace arrive at one who never leaves the ground, and the rig drew none
/// of it: `dive` comes from recorded HEIGHT and `carry` only once the ball is
/// already sitting in the 0.85-1.45 m hold band, so what was drawn was a ball
/// stopping dead at a man with his arms by his sides.
#[cfg(test)]
mod saves {
    use super::*;
    use crate::replay::{Sample, Track};

    /// The keeper, on his line in the middle of the goal at the left-hand
    /// end. Engine units are 0.125 m and the field is 840 × 545.
    const KEEPER: Vec3 = Vec3::new(-51.0, 0.0, 0.0);

    /// A ball track, in engine units at the recording's own 30 ms step.
    fn track(rows: &[(u32, f32, f32, f32)]) -> Track {
        let mut track = Track::default();
        track.merge(
            rows.iter()
                .map(|&(t, x, y, z)| Sample { t, x, y, z })
                .collect(),
        );
        track
    }

    /// A shot struck from `out` units and arriving `across` units off the
    /// keeper, at 20 m/s, sampled every 30 ms.
    fn shot(out: f32, across: f32, height: f32, stops: bool) -> Track {
        // 20 m/s = 160 u/s = 4.8 u per 30 ms step.
        let steps = (out / 4.8).ceil() as u32;
        let mut rows = Vec::new();
        for step in 0..=steps + 12 {
            let travelled = (step as f32 * 4.8).min(out);
            let done = travelled >= out;
            let past = if done && !stops {
                (step as f32 * 4.8 - out).min(60.0)
            } else {
                0.0
            };
            rows.push((
                step * 30,
                20.0 + out - travelled - past,
                272.5 + across * (travelled / out),
                if done {
                    height
                } else {
                    height * travelled / out
                },
            ));
        }
        track(&rows)
    }

    /// **He sees it coming.** The countdown is a real one — the same shape
    /// the kick's backswing runs on — so the hands arrive with the ball
    /// wherever the playhead is dropped and at any playback speed.
    #[test]
    fn a_shot_at_him_is_seen_coming() {
        let mut ball = shot(140.0, 6.0, 0.9, true);
        let mut previous = f32::MAX;
        let mut seen = 0;
        for step in 0..24 {
            let now = step as f64 * 30.0;
            let Some(save) = Actors::next_arrival(&mut ball, now, KEEPER) else {
                continue;
            };
            seen += 1;
            assert!(
                save.delay < previous + 1e-3,
                "the countdown went backwards: {previous:.3} s then {:.3} s",
                save.delay
            );
            previous = save.delay;
        }
        assert!(
            seen >= 8,
            "only {seen} frames of warning — there is nothing to draw a reaction over"
        );
    }

    /// …and he knows WHERE. The aim is the closest approach on the segment,
    /// not the nearest sample: at fifty milliseconds a probe a shot travels
    /// a metre and a half, so sampling alone puts his hands most of a metre
    /// from the ball.
    #[test]
    fn the_reach_goes_where_the_ball_will_be() {
        let mut ball = shot(140.0, 12.0, 1.4, true);
        let save = Actors::next_arrival(&mut ball, 0.0, KEEPER).expect("a shot at him");
        let arrival = Field::to_world(20.0, 272.5 + 12.0, 1.4);
        assert!(
            save.at.distance(arrival) < 0.20,
            "his hands go to {:?} for a ball arriving at {arrival:?}",
            save.at
        );
    }

    /// **A keeper throwing the ball out is not saving it.** It leaves his
    /// hands at pace, from inside his own reach, under the bar — everything
    /// an arrival is except the one thing that matters, which is which way
    /// the gap is going. Without the test he reached for every delivery he
    /// made.
    #[test]
    fn a_ball_leaving_him_is_not_a_save() {
        let mut rows = Vec::new();
        for step in 0..30 {
            rows.push((step * 30, 20.0 + step as f32 * 4.8, 272.5, 1.2));
        }
        let mut ball = track(&rows);
        assert!(
            Actors::next_arrival(&mut ball, 0.0, KEEPER).is_none(),
            "he reaches for his own throw"
        );
    }

    /// A ball going wide of him is not his either — the reach is a reach,
    /// not a wish.
    #[test]
    fn a_ball_past_the_post_is_not_a_save() {
        let mut ball = shot(140.0, 40.0, 0.9, true);
        assert!(
            Actors::next_arrival(&mut ball, 0.0, KEEPER).is_none(),
            "he reaches at a ball five metres wide of him"
        );
    }

    /// And whether he kept hold of it, which is the difference between two
    /// quite different pictures. Read off what the ball does next rather
    /// than guessed: a quarter of a second on, it has either stopped on him
    /// or gone.
    #[test]
    fn he_knows_whether_he_caught_it() {
        let mut caught = shot(140.0, 4.0, 1.1, true);
        let mut parried = shot(140.0, 4.0, 1.1, false);
        assert!(
            Actors::next_arrival(&mut caught, 0.0, KEEPER)
                .expect("a shot at him")
                .held,
            "a ball that stops dead on him is a catch"
        );
        assert!(
            !Actors::next_arrival(&mut parried, 0.0, KEEPER)
                .expect("a shot at him")
                .held,
            "a ball that carries on past him is a parry"
        );
    }
}

/// **Do his feet carry the ground he covers?**
///
/// The one question the rig's other test modules cannot ask. `flight` replays
/// recorded heights, `skeleton` asserts poses as positions and `churn`
/// measures the heading; a foot that slides is none of those — it is the
/// stance foot's speed relative to the body against the body's own speed over
/// the turf, and it needs the stride model and the forward kinematics in the
/// same place.
///
/// Reported as *"the goalkeeper glides across the field without any obvious
/// foot movement"*, and it was true of everybody: the phase advanced by ground
/// covered while the amplitude came off `speed / SPRINT`, so at a walk the
/// cadence was right and the legs moved a third of the distance the body did.
#[cfg(test)]
mod ground {
    use super::*;
    use crate::body::skeleton::{boot, crown, running, still, travelling};

    /// Somebody with an ordinary stride, so the numbers below are about the
    /// model rather than about one player's cadence.
    const WALKER: u32 = 7;

    /// A gait at this pace, at this point in the cycle, going this way.
    fn moving(speed: f32, phase: f32, course: Vec2) -> Gait {
        let (_, carry_ground) = Actors::stride_of(WALKER, speed, course);
        let mut gait = travelling(
            (speed / Actors::SPRINT).clamp(0.0, 1.0),
            course.x,
            course.y,
            carry_ground,
        );
        gait.phase = phase;
        gait
    }

    /// How fast the RIGHT boot travels backwards relative to the body, in
    /// metres a second, at MID-STANCE.
    ///
    /// Mid-stance is where the leg passes under the hip, which for this rig
    /// is `phase = PI` on the right leg: the sagittal amplitude is
    /// `sin(leg)`, so the foot is level with the hip and travelling at its
    /// fastest relative to him. It is also the quantity the amplitude is
    /// derived from — a leg swinging through a sinusoid can only match the
    /// turf at one point of its stance, and this is the point, because it is
    /// the one the eye reads.
    ///
    /// A fixed phase rather than a search for the lowest boot: the height
    /// profile is shallow (a centimetre covers a third of the cycle) and the
    /// knee tuck puts a second dip beside the first, so a search lands
    /// wherever the noise does. Positive means "backwards", which is what a
    /// planted foot does while the body travels forwards over it — so a
    /// backpedal reads NEGATIVE here, and that is the whole of the
    /// reversal test.
    fn stance_slip(speed: f32, course: Vec2) -> f32 {
        let (stride, _) = Actors::stride_of(WALKER, speed, course);
        // Radians of cycle a second: half a cycle per stride of ground.
        let rate = PI * speed / stride;
        let nudge = 0.5;
        let ahead = boot(1.0, moving(speed, PI + nudge, course)).z;
        let behind = boot(1.0, moving(speed, PI - nudge, course)).z;
        -(ahead - behind) / (2.0 * nudge) * rate
    }

    /// **The feet have to carry the ground.** A foot on the turf travels
    /// backwards relative to the body at exactly the speed the body travels
    /// forwards over it; anything less is the body sliding out from under it,
    /// and that is what a glide is.
    ///
    /// Held to a quarter, and only up the walking and jogging band: a runner
    /// has a flight phase and his feet genuinely do go back faster than the
    /// ground, which is what the sprint end of `HIP_SWING` draws and is left
    /// alone.
    #[test]
    fn the_feet_carry_the_ground_he_covers() {
        for speed in [0.8_f32, 1.4, 2.0, 3.0] {
            let slip = stance_slip(speed, Vec2::Y);
            assert!(
                (slip - speed).abs() < speed * 0.25,
                "at {speed:.1} m/s the stance foot travels {slip:.2} m/s: \
                 {:.0}% of the ground he is covering",
                100.0 * slip / speed
            );
        }
    }

    /// And the old model, restated, so the fix cannot quietly be undone: the
    /// run cycle's own amplitude alone is not enough to carry a walk.
    #[test]
    fn the_run_cycle_alone_cannot_carry_a_walk() {
        let speed = 1.4_f32;
        let (stride, _) = Actors::stride_of(WALKER, speed, Vec2::Y);
        let rate = PI * speed / stride;
        let mut gait = running((speed / Actors::SPRINT).clamp(0.0, 1.0));
        let mut planted = (0.0, f32::MAX);
        for step in 0..240 {
            gait.phase = step as f32 * TAU / 240.0;
            let sole = boot(1.0, gait);
            if sole.y < planted.1 {
                planted = (gait.phase, sole.y);
            }
        }
        let nudge = 0.02;
        gait.phase = planted.0 + nudge;
        let ahead = boot(1.0, gait);
        gait.phase = planted.0 - nudge;
        let behind = boot(1.0, gait);
        let slip = -(ahead.z - behind.z) / (2.0 * nudge) * rate;
        assert!(
            slip < speed * 0.8,
            "the run cycle alone already carries {slip:.2} m/s of {speed:.1} — \
             `carry_ground` has nothing left to fix and can go"
        );
    }

    /// A keeper going backwards runs the cycle the other way round. Without
    /// it he moonwalks: the legs stride up the pitch while the body travels
    /// down it, which is the single most obvious way an animation gives
    /// itself away.
    #[test]
    fn he_backpedals_instead_of_moonwalking() {
        let forwards = stance_slip(2.0, Vec2::Y);
        let backwards = stance_slip(2.0, -Vec2::Y);
        assert!(
            forwards > 0.0 && backwards < 0.0,
            "the stride does not reverse: {forwards:.2} m/s going forwards \
             against {backwards:.2} m/s going backwards"
        );
    }

    /// **A shuffling keeper never crosses his feet.** The two legs are half a
    /// cycle apart, so their lateral offsets are equal and opposite and the
    /// only thing keeping the swinging foot from passing through the planted
    /// one is the base he sets between them — see [`Joint::SHUFFLE_STANCE`].
    #[test]
    fn a_side_step_never_crosses_his_feet() {
        for phase in 0..120 {
            let gait = moving(2.5, phase as f32 * TAU / 120.0, Vec2::X);
            let right = boot(1.0, gait);
            let left = boot(-1.0, gait);
            assert!(
                right.x > left.x + 0.04,
                "his feet cross at phase {}: right boot at {:.3} m, left at {:.3} m",
                phase,
                right.x,
                left.x
            );
        }
    }

    /// …and it is a real step across, not a lean. Both feet have to move
    /// sideways relative to him or the shuffle is a man being slid across the
    /// six-yard box with his legs held still, which is the report.
    #[test]
    fn a_side_step_moves_his_feet_across_him() {
        let sweep = |side: f32| {
            let mut low = f32::MAX;
            let mut high = f32::MIN;
            for phase in 0..120 {
                let across = boot(side, moving(2.5, phase as f32 * TAU / 120.0, Vec2::X)).x;
                low = low.min(across);
                high = high.max(across);
            }
            high - low
        };
        for side in [-1.0_f32, 1.0] {
            let travel = sweep(side);
            assert!(
                travel > 0.18,
                "the {} boot only travels {travel:.3} m across him in a side-step",
                if side < 0.0 { "left" } else { "right" }
            );
        }
    }

    /// **He takes one step at a time.**
    ///
    /// The whole of what made the first side-step read as a cyborg, and the
    /// hardest thing to see in a still: the two legs were antisymmetric, so
    /// they splayed together and closed together like a pair of dividers,
    /// and — a sinusoid being stationary at exactly one INSTANT of its
    /// stance — neither foot was ever really planted. Rendered eight phases
    /// side by side, both boots sat on the grass in all eight and the legs
    /// scissored between them.
    ///
    /// Both halves are pinned: at every phase one boot is DOWN, and at some
    /// phase the other is genuinely UP. See [`Joint::tread`].
    #[test]
    fn a_side_step_takes_one_foot_at_a_time() {
        let flat = boot(1.0, still()).y;
        let mut highest = 0.0_f32;
        for phase in 0..120 {
            let gait = moving(1.4, phase as f32 * TAU / 120.0, Vec2::X);
            let (left, right) = (boot(-1.0, gait).y - flat, boot(1.0, gait).y - flat);
            assert!(
                left.min(right) < 0.020,
                "both feet are off the grass at phase {phase}: {left:.3} m and {right:.3} m"
            );
            highest = highest.max(left.max(right));
        }
        assert!(
            highest > 0.035,
            "neither foot ever leaves the grass — the step is a slide ({highest:.3} m)"
        );
    }

    /// …and the foot he is standing on carries the ground ACROSS him, the
    /// same claim [`the_feet_carry_the_ground_he_covers`] makes about a run.
    ///
    /// Measured at mid-stance, where `Joint::tread` puts the planted foot
    /// half way through its travel. The tread is linear there on purpose: a
    /// planted foot is stationary on the turf, so relative to the body it
    /// moves at exactly `−v` for the whole stance rather than at the varying
    /// rate of a sinusoid.
    #[test]
    fn the_planted_foot_carries_the_ground_across_him() {
        let speed = 1.4_f32;
        let (stride, _) = Actors::stride_of(WALKER, speed, Vec2::X);
        let rate = PI * speed / stride;
        // Mid-stance of the right foot, in its own cycle.
        let planted = FRAC_PI_2 + TAU * Joint::SHUFFLE_DUTY * 0.5;
        let nudge = 0.25;
        let ahead = boot(1.0, moving(speed, planted + nudge, Vec2::X)).x;
        let behind = boot(1.0, moving(speed, planted - nudge, Vec2::X)).x;
        let slip = -(ahead - behind) / (2.0 * nudge) * rate;
        assert!(
            (slip - speed).abs() < speed * 0.25,
            "the planted foot travels {slip:.2} m/s across him against {speed:.1} of ground: \
             {:.0}% of it",
            100.0 * slip / speed
        );
    }

    /// Every one of the above has to leave the boots on the grass. A wide
    /// base is a LOW one and the drop that pays for it is a real loss of
    /// height, so getting the two out of step buries him or floats him.
    #[test]
    fn a_shuffle_keeps_his_boots_on_the_grass() {
        let flat = boot(1.0, still()).y;
        for course in [Vec2::X, -Vec2::X, -Vec2::Y, Vec2::new(0.7, -0.7)] {
            for phase in 0..120 {
                let sole = boot(1.0, moving(2.5, phase as f32 * TAU / 120.0, course)).y;
                assert!(
                    sole > flat - 0.045,
                    "his boot is {:.3} m into the turf going ({:.1}, {:.1})",
                    flat - sole,
                    course.x,
                    course.y
                );
            }
        }
    }

    /// A gait as the RENDERER would build it at this pace on this course —
    /// through the opening, which `moving` above deliberately skips because
    /// its job is to exercise the side-step itself.
    fn drawn(speed: f32, phase: f32, course: Vec2) -> Gait {
        let open = Actors::opening(speed, course, false);
        let under = Actors::underfoot(course, open);
        let (_, carry_ground) = Actors::stride_of(WALKER, speed, under);
        let mut gait = travelling(
            (speed / Actors::SPRINT).clamp(0.0, 1.0),
            under.x,
            under.y,
            carry_ground,
        );
        gait.open = open;
        gait.keeper = 0.0;
        gait.phase = phase;
        gait
    }

    /// **The report, as an assertion.**
    ///
    /// *"They move sideways like invalids — crouching and buckling their
    /// legs."* Both halves of that were literally true and both were
    /// measurable: a man arcing round at five and a half metres a second was
    /// drawn 44 cm below his own standing height, and at the worst corner of
    /// the range 62 cm — a footballer squatting to two thirds of his height
    /// while sprinting.
    ///
    /// A crouch of a few centimetres is real and belongs to the gait: a wide
    /// base IS a low one, which is the whole of [`Joint::splay_drop`], and a
    /// jockeying defender does sit down into it. A foot and a half is not a
    /// gait, it is an equation being handed a demand no body could meet.
    /// Swept over every course and every pace, because the worst corner was
    /// nowhere near the obvious one — it was not the pure side-step at all,
    /// it was a man travelling backwards and across at a sprint.
    #[test]
    fn nobody_crouches_across_himself() {
        let tall = crown(still()).y;
        let mut worst = (0.0f32, 0.0f32, 0.0f32);
        for step in 0..72 {
            let bearing = step as f32 * TAU / 72.0;
            let course = Vec2::new(bearing.sin(), bearing.cos());
            for tick in 0..24 {
                let speed = 0.4 + tick as f32 * 0.3;
                for phase in 0..24 {
                    let gait = drawn(speed, phase as f32 * TAU / 24.0, course);
                    let drop = tall - crown(gait).y;
                    if drop > worst.0 {
                        worst = (drop, speed, bearing.to_degrees());
                    }
                }
            }
        }
        assert!(
            worst.0 < 0.17,
            "he sinks {:.3} m at {:.1} m/s going {:.0} deg off his own facing",
            worst.0,
            worst.1,
            worst.2
        );
    }

    /// **And no legs strobe.**
    ///
    /// The other half of the same failure, and the one a still frame cannot
    /// show. [`Joint::shortening`] is a RATIO — a side-step is a fraction of
    /// a running stride, which is the right shape — so at a sprint a third
    /// of a stride is still half a metre and asks for thirteen steps a
    /// second. At sixty frames that is two thirds of a step between one
    /// frame and the next: not quick feet, a leg aliasing.
    #[test]
    fn nobody_takes_more_steps_than_a_sprinter() {
        for step in 0..72 {
            let bearing = step as f32 * TAU / 72.0;
            let course = Vec2::new(bearing.sin(), bearing.cos());
            for tick in 0..24 {
                let speed = 0.4 + tick as f32 * 0.3;
                let open = Actors::opening(speed, course, false);
                let under = Actors::underfoot(course, open);
                let (stride, _) = Actors::stride_of(WALKER, speed, under);
                let cadence = speed / stride;
                assert!(
                    cadence <= Actors::TOP_CADENCE + 1e-3,
                    "{cadence:.1} steps a second at {speed:.1} m/s going {:.0} deg \
                     off his own facing",
                    bearing.to_degrees()
                );
            }
        }
    }

    /// **A man running at an angle is RUNNING.**
    ///
    /// The claim [`crate::body::Gait::open`] exists to make. Forty degrees
    /// off his own facing at a sprint is not a gait of its own — it is a
    /// footballer coming round onto a run while his shoulders catch up — and
    /// what it has to be drawn as is the run, with his legs where his feet
    /// are going. Held to the stride and the height of the same man going
    /// dead ahead, because once those two agree there is nothing left to
    /// differ.
    #[test]
    fn a_runner_turns_his_legs_onto_his_run() {
        let angle = 40.0f32.to_radians();
        let course = Vec2::new(angle.sin(), angle.cos());
        let straight = Actors::stride_of(WALKER, 5.0, Vec2::Y).0;
        let open = Actors::opening(5.0, course, false);
        let angled = Actors::stride_of(WALKER, 5.0, Actors::underfoot(course, open)).0;
        assert!(
            (angled - straight).abs() < straight * 0.02,
            "his stride goes from {straight:.2} m straight ahead to {angled:.2} m \
             at 40 deg, so he is not running it"
        );
        let tall = crown(still()).y;
        for phase in 0..48 {
            let at = phase as f32 * TAU / 48.0;
            let across = tall - crown(drawn(5.0, at, course)).y;
            let ahead = tall - crown(drawn(5.0, at, Vec2::Y)).y;
            assert!(
                (across - ahead).abs() < 0.012,
                "at phase {phase} he rides {across:.3} m down going at 40 deg \
                 against {ahead:.3} m going straight"
            );
        }
    }

    /// …but a defender jockeying still side-steps, and so does a goalkeeper
    /// on his line. Both are the point: the opening is not "stop drawing the
    /// shuffle", it is "stop drawing it for the twenty men who are not doing
    /// it".
    ///
    /// The keeper half of this used to read "at any speed at all", because
    /// [`Actors::SQUARE_UP`] turned his whole heading onto his run and left
    /// him nothing lateral to draw. [`Actors::SHOULDER`] ended that — he now
    /// holds his chest on the ball rather than turning his back on it, so
    /// his course stays lateral while he runs, and a lateral course drawn
    /// with square feet at four metres a second is a man bounding sideways.
    /// He crosses over instead, from where his shuffle ends.
    #[test]
    fn square_and_slow_is_still_a_side_step() {
        assert_eq!(
            Actors::opening(1.0, Vec2::X, false),
            0.0,
            "a defender jockeying at a walk has opened his hips up"
        );
        // A keeper set, and a keeper shuffling across his line: square feet.
        for speed in [0.5_f32, 1.0, 2.0, Actors::SQUARE_UP.1] {
            assert_eq!(
                Actors::opening(speed, Vec2::X, true),
                0.0,
                "a goalkeeper has stopped shuffling at {speed:.1} m/s"
            );
        }
        // …and a keeper genuinely running across himself crosses over, as
        // far as anybody else does.
        let running = Actors::opening(6.0, Vec2::X, true);
        assert!(
            (running - Actors::opening(6.0, Vec2::X, false)).abs() < 1e-4,
            "a keeper at a sprint is drawn with a different gait from every \
             other body on the pitch: {running:.3}"
        );
        assert!(
            running > 1.0,
            "he sprints across himself with his feet still square: {running:.3}"
        );
        // Continuous through the band, and monotone: no frame at which the
        // gait changes.
        let mut previous = 0.0;
        for step in 0..=20 {
            let speed = Actors::SQUARE_UP.1
                + (Actors::KEEPER_OPEN_UP.1 - Actors::SQUARE_UP.1) * step as f32 / 20.0;
            let open = Actors::opening(speed, Vec2::X, true);
            assert!(
                open >= previous - 1e-6 && open - previous < 0.25,
                "his legs jump from {previous:.3} to {open:.3} at {speed:.2} m/s"
            );
            previous = open;
        }
    }

    /// **And he never turns his back on a live ball near his goal.**
    ///
    /// The reported bug: a keeper recovering to his line at anything above
    /// a jog fell through the square-up branch entirely and was drawn
    /// facing his run — which, retreating, is directly away from the play.
    #[test]
    fn a_keeper_recovering_keeps_the_ball_in_front_of_him() {
        let ball = BallState {
            on_pitch: true,
            position: Vec3::new(0.0, 0.0, 12.0),
            ..Default::default()
        };
        // Him on his line at the origin, the ball 12 m in front, and his
        // run straight backwards into his own goal.
        for speed in [1.5_f32, 2.9, 4.0, 6.0] {
            let mut keeper = PlayerActor::new(1, true, true);
            keeper.speed = speed;
            keeper.travel = Vec3::new(0.0, 0.0, -speed);
            let facing = Actors::facing(&keeper, &ball, Vec3::ZERO, Vec3::ZERO, false);
            let flat = Vec3::new(facing.x, 0.0, facing.z)
                .try_normalize()
                .expect("a heading");
            let to_ball = Vec3::new(0.0, 0.0, 1.0);
            let off = flat.dot(to_ball).clamp(-1.0, 1.0).acos();
            assert!(
                off <= Actors::SHOULDER + 1e-3,
                "retreating at {speed:.1} m/s he is {:.0}deg off the ball — his back \
                 is to the play",
                off.to_degrees()
            );
        }
        // …and coming the other way, at a ball he is running AT, nothing
        // is clamped: he faces it, which is also his run.
        let mut charging = PlayerActor::new(1, true, true);
        charging.speed = 6.0;
        charging.travel = Vec3::new(0.0, 0.0, 6.0);
        let facing = Actors::facing(&charging, &ball, Vec3::ZERO, Vec3::ZERO, false);
        let flat = Vec3::new(facing.x, 0.0, facing.z)
            .try_normalize()
            .expect("a heading");
        assert!(
            flat.z > 0.999,
            "he is not looking at the ball he is sprinting out to: {flat:?}"
        );
    }

    /// …and going backwards he backpedals rather than turning round, which
    /// is a gait this rig already has.
    #[test]
    fn he_backpedals_rather_than_opening_up() {
        for speed in [1.0_f32, 3.0, 6.0] {
            assert!(
                Actors::opening(speed, -Vec2::Y, false).abs() < 1e-4,
                "he turns his legs round to drop back at {speed:.1} m/s"
            );
        }
    }

    /// **The opening never tears.**
    ///
    /// It is a rotation applied to a live body every frame, so a step in it
    /// is a leg jumping. There are two places it could: the bearing it is
    /// taken from wraps at ±π, and the fade that keeps a backpedal a
    /// backpedal has to reach zero before it gets there. Swept round the
    /// whole circle at every pace, an eighth of a degree at a time.
    #[test]
    fn the_opening_never_tears() {
        let steps = 2880;
        for tick in 0..30 {
            let speed = 0.2 + tick as f32 * 0.25;
            let mut last = None;
            for step in 0..=steps {
                let bearing = step as f32 * TAU / steps as f32;
                let course = Vec2::new(bearing.sin(), bearing.cos());
                let open = Actors::opening(speed, course, false);
                if let Some(was) = last {
                    let jumped: f32 = open - was;
                    assert!(
                        jumped.abs() < 0.02,
                        "the opening jumps {:.1} deg over an eighth of a degree \
                         of course at {speed:.1} m/s, near {:.0} deg",
                        jumped.to_degrees(),
                        bearing.to_degrees()
                    );
                }
                last = Some(open);
            }
        }
    }

    /// …and it never turns his legs PAST the way he is going, which would
    /// draw the side-step straight back in on the other side of him.
    #[test]
    fn the_opening_never_overshoots_his_run() {
        for step in 0..144 {
            let bearing = (step as f32 - 72.0) * PI / 72.0;
            let course = Vec2::new(bearing.sin(), bearing.cos());
            for tick in 0..24 {
                let speed = 0.4 + tick as f32 * 0.3;
                let open = Actors::opening(speed, course, false);
                assert!(
                    open * bearing >= -1e-6 && open.abs() <= bearing.abs() + 1e-4,
                    "at {speed:.1} m/s going {:.0} deg his legs go to {:.0} deg",
                    bearing.to_degrees(),
                    open.to_degrees()
                );
            }
        }
    }
}

/// **What does the viewer actually draw a goalkeeper doing?**
///
/// The end-to-end check on the two things the pose tests cannot see, because
/// neither is a pose: which way he is TRAVELLING relative to which way he is
/// POINTED — the decomposition the shuffle and the backpedal come off — and
/// how often the ball is arriving at him at all.
///
/// Same harness as [`churn`]: point `MATCH_REPLAY` at a decompressed chunk
/// and it walks a real recording at 60 fps through the real [`Actors::facing`]
/// and the real [`Actors::next_arrival`].
#[cfg(test)]
mod keeper {
    use super::*;
    use crate::replay::{ChunkPayload, ReplayTracks};

    fn load() -> Option<ReplayTracks> {
        let path = std::env::var("MATCH_REPLAY").ok()?;
        let body = std::fs::read_to_string(path).expect("readable chunk");
        let chunk: ChunkPayload = serde_json::from_str(&body).expect("a chunk");
        let mut tracks = ReplayTracks::default();
        tracks.absorb(chunk);
        Some(tracks)
    }

    /// Who the keepers are, inferred from the one thing that is true of them
    /// and of nobody else: they spend the match on a goal line. The recording
    /// carries states but the group prefix is stripped on the way in, so
    /// "Goalkeeper: Standing" arrives as "Standing" and cannot be asked.
    fn keepers(tracks: &mut ReplayTracks, start: f64) -> Vec<u32> {
        let ids: Vec<u32> = tracks.players.keys().copied().collect();
        let mut depth: Vec<(u32, f32)> = ids
            .into_iter()
            .filter_map(|id| {
                let track = tracks.players.get_mut(&id)?;
                let mut worst: f32 = 0.0;
                for step in 0..60 {
                    let at = start + step as f64 * 1000.0;
                    if let Some(p) = track.position_at(at) {
                        worst = worst.max((p[0] - 420.0).abs());
                    }
                }
                Some((id, worst))
            })
            .collect();
        depth.sort_by(|a, b| b.1.total_cmp(&a.1));
        depth.truncate(2);
        depth.into_iter().map(|(id, _)| id).collect()
    }

    #[test]
    #[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
    fn measure_keeper() {
        let Some(mut tracks) = load() else {
            panic!("set MATCH_REPLAY to a decompressed chunk");
        };
        let (start, until) = tracks.ball.span().expect("a recorded chunk");
        let ids = keepers(&mut tracks, start);
        let frame = 1.0f32 / 60.0;
        let frames = ((until - start) / (frame as f64 * 1000.0)) as u32;

        let mut moving = 0u64;
        // forward / across / backward, in his own frame
        let mut course = [0u64; 3];
        let mut still = 0u64;
        let mut samples = 0u64;
        let mut reaching = 0u64;
        let mut arrivals = 0u64;
        let mut leads: Vec<f32> = Vec::new();
        let mut held = 0u64;

        for id in ids {
            let mut actor = PlayerActor::new(id, true, true);
            let mut previous: Option<Vec3> = None;
            let mut seen: Option<f32> = None;
            for f in 0..frames {
                let now = start + f as f64 * frame as f64 * 1000.0;
                let Some(p) = tracks.players.get_mut(&id).and_then(|t| t.position_at(now)) else {
                    previous = None;
                    continue;
                };
                let position = Field::to_world(p[0], p[1], p[2]);
                let ball = tracks
                    .ball
                    .position_at(now)
                    .map(|b| Field::to_world(b[0], b[1], b[2]));
                let step = match previous {
                    Some(prev) => position - prev,
                    None => Vec3::ZERO,
                };
                previous = Some(position);
                if f == 0 {
                    continue;
                }

                let observed = step.length() / frame;
                actor.speed +=
                    (observed - actor.speed) * (1.0 - (-frame / Actors::PACE_RESPONSE).exp());
                let travelling = Vec3::new(step.x, 0.0, step.z) / frame;
                let was = actor.travel;
                actor.travel =
                    was + (travelling - was) * (1.0 - (-frame / Actors::TRAVEL_RESPONSE).exp());

                let mut state = BallState::default();
                if let Some(b) = ball {
                    state.on_pitch = true;
                    state.position = b;
                }
                let want = Actors::facing(&actor, &state, position, step, false);
                if let Some(want) = Vec3::new(want.x, 0.0, want.z).try_normalize() {
                    let wanted = want.x.atan2(want.z);
                    let swing = (wanted - actor.heading + PI).rem_euclid(TAU) - PI;
                    let eased = (actor.speed / Actors::SPRINT).clamp(0.0, 1.0);
                    let ceiling = (Actors::PIVOT_RATE.0
                        + (Actors::PIVOT_RATE.1 - Actors::PIVOT_RATE.0) * eased)
                        * frame;
                    actor.heading += (swing * (1.0 - (-frame / Actors::TURN_RESPONSE).exp()))
                        .clamp(-ceiling, ceiling);
                }

                samples += 1;
                if actor.speed <= Actors::STEPPING * 0.5 {
                    still += 1;
                } else {
                    moving += 1;
                    let forward = Vec3::new(actor.heading.sin(), 0.0, actor.heading.cos());
                    let sideways = Vec3::new(actor.heading.cos(), 0.0, -actor.heading.sin());
                    if let Some(way) =
                        Vec3::new(actor.travel.x, 0.0, actor.travel.z).try_normalize()
                    {
                        let along = way.dot(forward);
                        let across = way.dot(sideways).abs();
                        let bucket = if across > along.abs() {
                            1
                        } else if along > 0.0 {
                            0
                        } else {
                            2
                        };
                        course[bucket] += 1;
                    }
                }

                // …and the save he is about to make. Counted as EPISODES —
                // one arrival is visible for many frames, which is the whole
                // point of it.
                match Actors::next_arrival(&mut tracks.ball, now, position) {
                    Some(save) => {
                        reaching += 1;
                        if seen.is_none() {
                            arrivals += 1;
                            leads.push(save.delay);
                            if save.held {
                                held += 1;
                            }
                        }
                        seen = Some(save.delay);
                    }
                    None => seen = None,
                }
            }
        }

        let share = |n: u64, of: u64| 100.0 * n as f64 / of.max(1) as f64;
        let median = |mut v: Vec<f32>| {
            v.sort_by(f32::total_cmp);
            v.get(v.len() / 2).copied().unwrap_or(f32::NAN)
        };
        println!(
            "KEEPER MOTION over {samples} frames: still {:.0}%, moving {:.0}%",
            share(still, samples),
            share(moving, samples)
        );
        println!(
            "  of the moving frames: forward {:.0}%, ACROSS himself {:.0}%, BACKWARD {:.0}%",
            share(course[0], moving),
            share(course[1], moving),
            share(course[2], moving)
        );
        println!(
            "  saves on his feet: {arrivals} episodes, reach live on {:.1}% of frames, \
             first seen {:.2} s out, held {held} of them",
            share(reaching, samples),
            median(leads)
        );
    }
}

/// **How much do the twenty-two actually turn?**
///
/// The rig's other test modules replay recorded HEIGHTS
/// ([`flight`]) and assert poses as positions ([`crate::body::skeleton`]).
/// Neither can see the thing reported as "they spin around like toy tops",
/// because that is not a pose and not a height — it is the heading, frame
/// over frame, and the only honest way to measure it is to run a real
/// recording through the real decision.
///
/// So: point `MATCH_REPLAY` at a decompressed chunk
/// (`.dev/match/match_results/dev/*.json.gz`) and this walks it at 60 fps
/// through [`Actors::facing`] and the same exponential heading integrator
/// `animate` uses, reporting degrees of yaw per second and how often a
/// player swings past a right angle between two frames.
#[cfg(test)]
mod churn {
    use super::*;
    use crate::replay::{ChunkPayload, ReplayTracks};

    /// A player turning faster than this is not turning, he is spinning.
    /// 360°/s is a full revolution a second — nothing a footballer does.
    const SPIN: f32 = 360.0;

    fn load() -> Option<ReplayTracks> {
        let path = std::env::var("MATCH_REPLAY").ok()?;
        let body = std::fs::read_to_string(path).expect("readable chunk");
        let chunk: ChunkPayload = serde_json::from_str(&body).expect("a chunk");
        let mut tracks = ReplayTracks::default();
        tracks.absorb(chunk);
        Some(tracks)
    }

    #[test]
    #[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
    fn measure_turning() {
        let Some(mut tracks) = load() else {
            panic!("set MATCH_REPLAY to a decompressed chunk");
        };
        let ids: Vec<u32> = tracks.players.keys().copied().collect();
        let start = 900_000.0f64;
        let frame = 1.0f32 / 60.0;
        let frames = 60 * 60; // one minute

        let mut samples = 0u64;
        let mut spun = 0u64;
        let mut past_right_angle = 0u64;
        let mut yaw_sum = 0.0f64;
        // …and the same restricted to the man ON the ball, which is where
        // the bearing is most degenerate.
        let mut on_ball = 0u64;
        let mut on_ball_yaw = 0.0f64;
        // [running, watching the ball, holding his heading]
        let mut by_branch = [(0u64, 0.0f64); 3];

        for id in ids {
            let mut actor = PlayerActor::new(id, false, true);
            let mut previous: Option<Vec3> = None;
            let mut last_heading = 0.0f32;
            for f in 0..frames {
                let now = start + f as f64 * frame as f64 * 1000.0;
                let Some(p) = tracks.players.get_mut(&id).and_then(|t| t.position_at(now)) else {
                    previous = None;
                    continue;
                };
                let position = Field::to_world(p[0], p[1], p[2]);
                let ball = tracks
                    .ball
                    .position_at(now)
                    .map(|b| Field::to_world(b[0], b[1], b[2]));
                let step = match previous {
                    Some(prev) => position - prev,
                    None => Vec3::ZERO,
                };
                previous = Some(position);

                let observed = step.length() / frame;
                actor.speed +=
                    (observed - actor.speed) * (1.0 - (-frame / Actors::PACE_RESPONSE).exp());
                let travelling = Vec3::new(step.x, 0.0, step.z) / frame;
                let was = actor.travel;
                actor.travel =
                    was + (travelling - was) * (1.0 - (-frame / Actors::TRAVEL_RESPONSE).exp());

                let mut state = BallState::default();
                if let Some(b) = ball {
                    state.on_pitch = true;
                    state.position = b;
                    let range = Vec3::new(b.x - position.x, 0.0, b.z - position.z).length();
                    state.nearest = Some((id, range));
                }
                // Which branch produced it — the whole question is WHERE
                // the churn comes from, and `facing` does not say.
                let branch = if actor.speed > Actors::MOVING {
                    0
                } else if state.on_pitch {
                    1
                } else {
                    2
                };
                let want = Actors::facing(&actor, &state, position, step, false);
                if let Some(want) = Vec3::new(want.x, 0.0, want.z).try_normalize() {
                    let wanted = want.x.atan2(want.z);
                    let swing = (wanted - actor.heading + PI).rem_euclid(TAU) - PI;
                    let mut applied = swing * (1.0 - (-frame / Actors::TURN_RESPONSE).exp());
                    let eased = (actor.speed / Actors::SPRINT).clamp(0.0, 1.0);
                    let ceiling = (Actors::PIVOT_RATE.0
                        + (Actors::PIVOT_RATE.1 - Actors::PIVOT_RATE.0) * eased)
                        * frame;
                    applied = applied.clamp(-ceiling, ceiling);
                    actor.heading += applied;
                }
                let turned = ((actor.heading - last_heading + PI).rem_euclid(TAU) - PI).abs();
                last_heading = actor.heading;
                if f == 0 {
                    continue;
                }
                let rate = turned.to_degrees() / frame;
                samples += 1;
                yaw_sum += rate as f64;
                if rate > SPIN {
                    spun += 1;
                }
                if turned.to_degrees() > 90.0 {
                    past_right_angle += 1;
                }
                if state.nearest.is_some_and(|(_, r)| r < Actors::AT_HIS_FEET) {
                    on_ball += 1;
                    on_ball_yaw += rate as f64;
                }
                by_branch[branch].0 += 1;
                by_branch[branch].1 += rate as f64;
            }
        }

        println!(
            "TURN CHURN over {samples} player-frames: mean {:.0} deg/s, \
             spinning (>{SPIN:.0} deg/s) {:.1}%, past a right angle in one frame {:.2}%",
            yaw_sum / samples as f64,
            spun as f64 * 100.0 / samples as f64,
            past_right_angle as f64 * 100.0 / samples as f64
        );
        for (name, (n, sum)) in ["running", "watching the ball", "holding"]
            .iter()
            .zip(by_branch)
        {
            if n > 0 {
                println!(
                    "  {name}: {n} frames ({:.0}%), mean {:.0} deg/s",
                    n as f64 * 100.0 / samples as f64,
                    sum / n as f64
                );
            }
        }
        if on_ball > 0 {
            println!(
                "  …the man ON the ball ({on_ball} frames): mean {:.0} deg/s",
                on_ball_yaw / on_ball as f64
            );
        }
    }

    /// **How far off his own facing does an outfielder actually travel?**
    ///
    /// The question [`crate::body::Gait::open`] turns on, and one that could
    /// only be guessed at until this existed. [`super::keeper::measure_keeper`] asks
    /// it of the two men who are square to the play on purpose; this asks it
    /// of the other twenty, for whom every degree of it is the
    /// heading integrator still catching up with a run already under way.
    ///
    /// Same harness as [`measure_turning`], carried one step further — the
    /// course, the opening, and the pose those two produce — so what it
    /// reports is what the pitch draws rather than what the model would draw
    /// if the recording asked for it.
    ///
    /// ```text
    /// MATCH_REPLAY=<chunk.json> cargo test --lib measure_lateral -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs MATCH_REPLAY pointed at a decompressed recording chunk"]
    fn measure_lateral() {
        use crate::body::skeleton::{boot, crown, still, travelling};

        let Some(mut tracks) = load() else {
            panic!("set MATCH_REPLAY to a decompressed chunk");
        };
        let ids: Vec<u32> = tracks.players.keys().copied().collect();
        // Off the chunk own span rather than a wall-clock figure: a chunk is
        // a few minutes somewhere in a match, and which few is not knowable
        // from here.
        let (start, until) = tracks.ball.span().expect("a recorded chunk");
        let frame = 1.0f32 / 60.0;
        let frames = (((until - start) / (frame as f64 * 1000.0)) as u32).min(60 * 90);
        let tall = crown(still()).y;

        let mut moving = 0u64;
        // |bearing| in his own frame: under 30, 30-60, 60-100, past 100.
        let mut band = [0u64; 4];
        // ...and the same again over the frames he is running in, which is
        // where a lateral gait costs anything.
        let mut running = 0u64;
        let mut running_band = [0u64; 4];
        let mut worst = (0.0f32, 0.0f32, 0.0f32);
        let mut widest = (0.0f32, 0.0f32, 0.0f32);

        for id in ids {
            let mut actor = PlayerActor::new(id, false, true);
            let mut previous: Option<Vec3> = None;
            for f in 0..frames {
                let now = start + f as f64 * frame as f64 * 1000.0;
                let Some(p) = tracks.players.get_mut(&id).and_then(|t| t.position_at(now)) else {
                    previous = None;
                    continue;
                };
                let position = Field::to_world(p[0], p[1], p[2]);
                let ball = tracks
                    .ball
                    .position_at(now)
                    .map(|b| Field::to_world(b[0], b[1], b[2]));
                let step = match previous {
                    Some(prev) => position - prev,
                    None => Vec3::ZERO,
                };
                previous = Some(position);

                let observed = step.length() / frame;
                actor.speed +=
                    (observed - actor.speed) * (1.0 - (-frame / Actors::PACE_RESPONSE).exp());
                let travelling_at = Vec3::new(step.x, 0.0, step.z) / frame;
                let was = actor.travel;
                actor.travel =
                    was + (travelling_at - was) * (1.0 - (-frame / Actors::TRAVEL_RESPONSE).exp());

                let mut state = BallState::default();
                if let Some(b) = ball {
                    state.on_pitch = true;
                    state.position = b;
                }
                let want = Actors::facing(&actor, &state, position, step, false);
                if let Some(want) = Vec3::new(want.x, 0.0, want.z).try_normalize() {
                    let wanted = want.x.atan2(want.z);
                    let swing = (wanted - actor.heading + PI).rem_euclid(TAU) - PI;
                    let mut applied = swing * (1.0 - (-frame / Actors::TURN_RESPONSE).exp());
                    let eased = (actor.speed / Actors::SPRINT).clamp(0.0, 1.0);
                    let ceiling = (Actors::PIVOT_RATE.0
                        + (Actors::PIVOT_RATE.1 - Actors::PIVOT_RATE.0) * eased)
                        * frame;
                    applied = applied.clamp(-ceiling, ceiling);
                    actor.heading += applied;
                }

                let forward = Vec3::new(actor.heading.sin(), 0.0, actor.heading.cos());
                let sideways = Vec3::new(actor.heading.cos(), 0.0, -actor.heading.sin());
                let going = Vec3::new(actor.travel.x, 0.0, actor.travel.z);
                let wanted_course = match going.try_normalize() {
                    Some(way) if actor.speed > Actors::STEPPING * 0.5 => {
                        Vec2::new(way.dot(sideways), way.dot(forward))
                    }
                    _ => Vec2::Y,
                };
                let settle = 1.0 - (-frame / Actors::COURSE_RESPONSE).exp();
                actor.course =
                    (actor.course + (wanted_course - actor.course) * settle).clamp_length_max(1.0);
                if f == 0 || actor.speed <= Actors::STEPPING * 0.5 {
                    continue;
                }

                let open = Actors::opening(actor.speed, actor.course, false);
                let under = Actors::underfoot(actor.course, open);
                let bearing = actor.course.x.atan2(actor.course.y).abs().to_degrees();
                let slot = if bearing < 30.0 {
                    0
                } else if bearing < 60.0 {
                    1
                } else if bearing < 100.0 {
                    2
                } else {
                    3
                };
                moving += 1;
                band[slot] += 1;
                if actor.speed > 3.0 {
                    running += 1;
                    running_band[slot] += 1;
                }

                // ...and what that draws. A pose is a whole cycle, so it is
                // walked rather than sampled at whatever phase this frame
                // happens to be at: the crouch and the base both peak inside
                // a single step.
                let mut gait = travelling(
                    (actor.speed / Actors::SPRINT).clamp(0.0, 1.0),
                    under.x,
                    under.y,
                    Actors::stride_of(id, actor.speed, under).1,
                );
                gait.open = open;
                gait.keeper = 0.0;
                for phase in 0..24 {
                    gait.phase = phase as f32 * TAU / 24.0;
                    let drop = tall - crown(gait).y;
                    if drop > worst.0 {
                        worst = (drop, actor.speed, bearing);
                    }
                    let apart = (boot(1.0, gait) - boot(-1.0, gait)).length();
                    if apart > widest.0 {
                        widest = (apart, actor.speed, bearing);
                    }
                }
            }
        }

        let share = |n: u64, of: u64| 100.0 * n as f64 / of.max(1) as f64;
        println!(
            "OUTFIELD COURSE over {moving} moving player-frames: \
             under 30 deg {:.1}%, 30-60 {:.1}%, 60-100 {:.1}%, past 100 {:.1}%",
            share(band[0], moving),
            share(band[1], moving),
            share(band[2], moving),
            share(band[3], moving)
        );
        println!(
            "  ...of the {running} frames above 3 m/s: {:.1}% / {:.1}% / {:.1}% / {:.1}%",
            share(running_band[0], running),
            share(running_band[1], running),
            share(running_band[2], running),
            share(running_band[3], running)
        );
        println!(
            "  worst crouch drawn: {:.3} m at {:.1} m/s, {:.0} deg off his facing",
            worst.0, worst.1, worst.2
        );
        println!(
            "  widest base drawn: {:.2} m at {:.1} m/s, {:.0} deg off his facing",
            widest.0, widest.1, widest.2
        );
    }
}
