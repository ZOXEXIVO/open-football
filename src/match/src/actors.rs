use crate::body::{BodyParts, Carriage, Footballer, Gait, Joint, Physique};
use crate::config::{PlayerInfo, ViewerConfig};
use crate::field::Field;
use crate::kit::{Complexion, Wardrobe};
use crate::loader::ChunkLoader;
use crate::playback::Playback;
use crate::replay::{ReplayTracks, Track};
use crate::textures::Textures;
use crate::timeline::DebugOverlay;
use bevy::prelude::*;
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
    /// Out of his hands rather than off his boot, which is a keeper throwing
    /// the ball out.
    thrown: bool,
}

/// A kick that is about to happen.
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
    /// How fast it will leave when he does.
    pub velocity: Vec3,
    /// Seconds of match time until contact. Counts down as the playhead
    /// closes on it, and is what puts the swing in the right place: the pose
    /// is a function of this rather than of a clock the viewer starts, so it
    /// stays correct when the replay is scrubbed or run at 8x.
    pub delay: f32,
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
    /// Width of the shadow and the team ring on the turf, in metres.
    const FOOTPRINT: f32 = 1.32;
    /// Ground speed, in metres per second, that counts as flat out.
    const SPRINT: f32 = 6.0;
    /// Above this a player faces where they are going; below it they turn to
    /// watch the ball, which is what footballers standing still actually do.
    const MOVING: f32 = 1.1;
    /// No footballer covers ground this fast. Anything quicker is a seek, a
    /// substitution or a restart, and has to be cut to rather than run.
    const TELEPORT: f32 = 25.0;
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
    /// How far a player will turn his head off his own facing before he has
    /// to turn his shoulders with it.
    const NECK: f32 = 1.05;
    /// Stride length: how far a player travels per step, walking and per extra
    /// metre per second of pace. A sprinter's stride tops out around 2 m.
    const STRIDE: (f32, f32, f32) = (0.75, 0.13, 2.10);
    /// Seconds for a player to come round onto a new heading.
    const TURN_RESPONSE: f32 = 0.13;
    /// Seconds for the run cycle to take up a change of pace.
    const PACE_RESPONSE: f32 = 0.18;
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
    /// Seconds of match time to get back up. There is no attack time on the
    /// way out: the engine's own arc is the take-off, and going up is
    /// already as fast as gravity says. Landing is where a keeper takes his
    /// time.
    const SPRAWL_RECOVERY: f32 = 0.42;
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

    pub fn spawn(
        mut commands: Commands,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
        config: Res<ViewerConfig>,
    ) {
        let parts = BodyParts::new(&mut meshes);
        let wardrobe = Wardrobe::new(&mut materials, &mut images, &config);
        let patch = meshes.add(Plane3d::default().mesh().size(1.0, 1.0));

        for player in &config.players {
            let actor = commands
                .spawn((
                    PlayerActor::new(player.id, player.is_goalkeeper()),
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

            // Drawn under the boots, in this order: the shadow that roots the
            // player on the turf, then their team's ring around it.
            commands.entity(actor).with_children(|marks| {
                marks.spawn((
                    Mesh3d(patch.clone()),
                    MeshMaterial3d(wardrobe.shadow()),
                    Transform::from_xyz(0.0, 0.018, 0.0)
                        .with_scale(Vec3::splat(Self::FOOTPRINT * 0.86)),
                ));
                marks.spawn((
                    Mesh3d(patch.clone()),
                    MeshMaterial3d(wardrobe.marker(player.is_home)),
                    Transform::from_xyz(0.0, 0.022, 0.0).with_scale(Vec3::splat(Self::FOOTPRINT)),
                ));
            });
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
                Node {
                    position_type: PositionType::Absolute,
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
                continue;
            }
            let boots = Vec2::new(transform.translation.x, transform.translation.z);
            if let Some(ball) = ball_position {
                let range = boots.distance(Vec2::new(ball.x, ball.z));
                if nearest.is_none_or(|(_, best)| range < best) {
                    nearest = Some((actor.id, range));
                }
            }
            if let Some((at, _, _)) = coming {
                // Whoever will be standing over it. Not whoever is nearest the
                // ball NOW: over a hundred and fifty milliseconds a player
                // closing on a loose ball covers more than a metre, and the
                // man who ends up hitting it is often not the one closest to
                // it when the swing starts.
                let range = boots.distance(Vec2::new(at.x, at.z));
                if striker.is_none_or(|(_, best)| range < best) {
                    striker = Some((actor.id, range));
                }
            }

            if !actor.is_goalkeeper || holder.is_some() {
                continue;
            }
            if let Some(ball) = ball_position {
                let reach = Vec2::new(
                    ball.x - transform.translation.x,
                    ball.z - transform.translation.z,
                )
                .length();
                if reach < Self::GLOVE_REACH
                    && ball.y > Self::GLOVE_HEIGHT.0
                    && ball.y < Self::GLOVE_HEIGHT.1
                {
                    // At his chest if he is on his feet, out at the end of the
                    // stretch if he is not — the same blend the arms
                    // themselves take, so the ball is wherever his gloves are
                    // rather than wherever they would have been.
                    let hold =
                        Physique::CRADLE.lerp(Physique::catch(actor.lead()), actor.extended());
                    // Carried out through the CARRIAGE, which is where the
                    // topple and the lift live, and only then out through the
                    // actor's own height and build. Reading the hold point
                    // straight off the actor — as this did — left the ball at
                    // the sternum of an upright man while the keeper it
                    // belonged to was horizontal the better part of a metre
                    // away, on every diving catch in the match.
                    //
                    // A frame behind, because this frame's topple is not
                    // written until `animate` has run: sixteen milliseconds of
                    // a four-hundred-millisecond dive, and worth it for not
                    // making the ball's position depend on system ordering.
                    let (pitch, roll) = actor.topple();
                    let gloves = Carriage::placed(pitch, roll, actor.lift()).transform_point(hold);
                    holder = Some((actor.id, transform.transform_point(gloves)));
                }
            }
        }

        ball_state.on_pitch = ball_position.is_some();
        ball_state.nearest = nearest;
        // Only if somebody is actually close enough to have hit it. A ball
        // that speeds up with nobody near it is a deflection, a restart or the
        // engine putting it back on the centre spot, and none of those is a
        // man swinging a leg.
        ball_state.impact = match (coming, striker) {
            (Some((_, velocity, delay)), Some((by, range))) if range < Self::STRIKE_REACH => {
                Some(Impact {
                    by,
                    velocity,
                    delay,
                })
            }
            _ => None,
        };
        if let Some(world) = ball_position {
            // Ball velocity, in metres per second of match time, read off the
            // RAW recorded path — never off the drawn position, which is
            // displaced while a keeper has it. Blanked on a seek, where the
            // jump is the playhead moving and not the ball.
            ball_state.velocity = match ball_state.previous {
                Some(previous) if !playback.seeked => (world - previous) / delta,
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
                transform.translation = Vec3::new(drawn.x, 0.02, drawn.z);
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
    fn next_impact(ball: &mut Track, now: f64) -> Option<(Vec3, Vec3, f32)> {
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
                return Some((here, velocity, delay));
            }
            previous = here;
            here = next;
            before = (here - previous).length() / (Self::PROBE as f32 / 1000.0);
        }
        None
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
        time: Res<Time>,
        mut actors: Query<(&mut PlayerActor, &mut Transform, &Visibility)>,
        mut joints: Query<(&Joint, &mut Transform), Without<PlayerActor>>,
    ) {
        let delta = time.delta_secs().max(1e-4);
        // Exponential catch-up, framerate independent.
        let turn = 1.0 - (-delta / Self::TURN_RESPONSE).exp();
        let pace = 1.0 - (-delta / Self::PACE_RESPONSE).exp();

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
            let dribbling = ball.on_pitch
                && !gathering
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

            let facing = if let Some(kick) = actor.kick.filter(|kick| kick.swing < 0.0) {
                // Opening up to what he is ABOUT to hit. A footballer sets his
                // body before he strikes the ball, not after it has gone —
                // which the viewer can honour because it knows the kick is
                // coming. Outranks everything below, including the dive: this
                // is the same man being turned by the same intention.
                kick.direction
            } else if actor.dive > 1e-3 && ball.on_pitch {
                // A keeper off his feet stays square to the shot and lets his
                // body go over; he does not turn to face the corner he is
                // diving into. Outranks the run for the same reason the strike
                // does — this is a man travelling one way and pointed
                // another, and it is also what makes `tip` decomposable onto
                // his own right and forward.
                ball.position - position
            } else if let Some((direction, _)) = actor.strike {
                // Opened up to where he played it, for as long as the follow
                // through lasts. Outranks the run: this is the one moment a
                // footballer is not facing where he is going.
                direction
            } else if actor.speed > Self::MOVING {
                Vec3::new(step.x, 0.0, step.z)
            } else if ball.on_pitch {
                ball.position - position
            } else {
                Vec3::ZERO
            };
            let mut turn_signal = 0.0_f32;
            if let Some(facing) = Vec3::new(facing.x, 0.0, facing.z).try_normalize() {
                // Rotating about Y by `atan2(x, z)` carries +Z onto the facing,
                // and the model is built looking down +Z.
                let wanted = facing.x.atan2(facing.z);
                let swing = (wanted - actor.heading + PI).rem_euclid(TAU) - PI;
                let applied = swing * if playback.seeked { 1.0 } else { turn };
                actor.heading += applied;
                // In radians per second of match time, normalised against a
                // hard change of direction, so the lean is the same at any
                // frame rate or playback speed.
                let rate = applied / (delta * playback.speed.max(0.1));
                turn_signal = (rate / Self::HARD_TURN).clamp(-1.0, 1.0);
            }
            actor.turn += (turn_signal - actor.turn) * pace;
            transform.rotation = Quat::from_rotation_y(actor.heading);

            // Idle cycle runs on the clock, not on ground covered, because it
            // exists precisely for the player who is covering none.
            actor.idle =
                (actor.idle + delta * playback.speed.max(0.1) * Self::IDLE_RATE).rem_euclid(TAU);

            // Where he is looking. Clamped to what a neck can do — past that a
            // real player turns his whole body, which he is already doing.
            let wanted_look = if ball.on_pitch {
                let to_ball = ball.position - position;
                match Vec3::new(to_ball.x, 0.0, to_ball.z).try_normalize() {
                    Some(bearing) => {
                        let angle = bearing.x.atan2(bearing.z);
                        (((angle - actor.heading + PI).rem_euclid(TAU)) - PI)
                            .clamp(-Self::NECK, Self::NECK)
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
            let wanted_pitch = if ball.on_pitch {
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
            let wanted_set = if actor.is_goalkeeper && ball.on_pitch {
                let to_ball = ball.position - position;
                let range = Vec3::new(to_ball.x, 0.0, to_ball.z).length();
                ((Self::SET_RANGE.1 - range) / (Self::SET_RANGE.1 - Self::SET_RANGE.0))
                    .clamp(0.0, 1.0)
            } else {
                0.0
            };
            actor.set += (wanted_set - actor.set) * if playback.seeked { 1.0 } else { pace };

            // The one man with the ball in his gloves takes the ramp that the
            // ball itself is riding; everybody else lets whatever they were
            // holding fall away. Following a ramp with a second one is
            // deliberate — it puts the arms a fraction behind the ball, so a
            // keeper throwing it out has a follow through rather than
            // snapping back to his sides on the frame it leaves him.
            let wanted = if ball.held_by == Some(actor.id) {
                ball.cradle
            } else {
                0.0
            };
            actor.carry += (wanted - actor.carry) * if playback.seeked { 1.0 } else { pace };

            let stride = (Self::STRIDE.0 + Self::STRIDE.1 * actor.speed)
                .clamp(Self::STRIDE.0, Self::STRIDE.2);
            // Half a cycle per step: the other leg takes the next one.
            actor.phase = (actor.phase + ground * PI / stride).rem_euclid(TAU);
        }

        for (joint, mut transform) in &mut joints {
            let Ok((actor, _, _)) = actors.get(joint.owner) else {
                continue;
            };
            let gait = actor.gait();
            transform.rotation = joint.pose(gait);
            transform.translation = joint.place(gait);
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
            *transform = Carriage::placed(pitch, roll, actor.lift());
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

        for (label, mut node, mut visibility) in &mut labels {
            let Ok((actor_transform, actor_visibility)) = actors.get(label.actor) else {
                *visibility = Visibility::Hidden;
                continue;
            };
            if *actor_visibility == Visibility::Hidden {
                *visibility = Visibility::Hidden;
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
                *visibility = Visibility::Hidden;
                continue;
            };

            // The plate is centred by hand: UI nodes are positioned by their
            // top-left corner and the text width is not known here.
            let stature = (boots.y - crown.y).abs().max(6.0);
            node.left = Val::Px(boots.x - 44.0);
            node.top = Val::Px(boots.y + stature * Self::LABEL_GAP);
            node.width = Val::Px(88.0);
            node.justify_content = JustifyContent::Center;
            *visibility = Visibility::Inherited;
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
    fn new(id: u32, is_goalkeeper: bool) -> Self {
        PlayerActor {
            id,
            is_goalkeeper,
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
            strike: None,
            // Offset so twenty-two players are not all breathing in unison,
            // which would be its own kind of robot.
            idle: Complexion::carriage(id) * std::f32::consts::PI,
            turn: 0.0,
            look: 0.0,
        }
    }

    /// How far off the turf he is, in metres, straight off the recording.
    ///
    /// Nothing is added to it here. The engine launches a keeper on a
    /// ballistic arc out of his `jumping` attribute and lets gravity bring
    /// him down, so the rise and the landing are already a physical
    /// trajectory rather than an animation curve — anything this end could
    /// contribute would be fighting it.
    fn lift(&self) -> f32 {
        self.height
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
            if self.down > Actors::SPRAWL_HOLD * (1.0 - hurry) {
                let release = 1.0 - (-match_delta / Actors::SPRAWL_RECOVERY).exp();
                self.dive -= self.dive * release;
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
            // Square-rooted: the amplitude is a distance a boot travels and
            // the number driving it is a speed, and a linear map between them
            // leaves the median pass — 17 m/s, a firm ball over twenty yards —
            // as a third of a swing.
            let power = ((impact.velocity.length() - Actors::TOUCHED)
                / (Actors::HAMMERED - Actors::TOUCHED))
                .clamp(0.0, 1.0)
                .sqrt();
            let direction = Vec3::new(impact.velocity.x, 0.0, impact.velocity.z)
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
                swing: -(impact.delay / Actors::WINDUP).clamp(0.0, 1.0),
                power,
                foot,
                blend: (blend + match_delta / Actors::KICK_ONSET).min(1.0),
                direction,
                // Out of his hands, not off his boot. A keeper who has just
                // gathered the ball and is about to send it upfield is
                // throwing it, and drawing him volleying it out of his own
                // gloves would be worse than drawing nothing.
                thrown: self.carry > 0.5,
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
    fn topple(&self) -> (f32, f32) {
        let tip = self.tip * Actors::SPRAWL_ANGLE * self.stretch;
        (tip.y, -tip.x)
    }

    /// How far into the ground-out after a landing, 0..1.
    ///
    /// Faded by the dive itself, so it lets go as he gets back to his feet
    /// rather than pinning him to the turf.
    fn grounded(&self) -> f32 {
        (self.down / Actors::GROUNDING).clamp(0.0, 1.0) * self.dive
    }

    /// How far out at the end of a stretch he is — off his feet and not yet
    /// gathered up on the grass. What decides whether a ball he has claimed
    /// is in his gloves or against his chest.
    fn extended(&self) -> f32 {
        self.dive * (1.0 - self.grounded())
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
        // A kick off the boot and a kick out of the hands are the same swing
        // routed to different limbs, so they are two fields off one source.
        let kicking = self.kick.filter(|kick| !kick.thrown);
        let throwing = self.kick.filter(|kick| kick.thrown);
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
            stretch: self.stretch,
            grounded,
            lead: self.lead(),
            // Both hands to the ball: he has it, and he is still up there.
            claimed: self.carry * extended,
            // Phase and side come off the swing whichever limb it drives;
            // which limb that is, is the two amplitudes below.
            swing: self.kick.map_or(0.0, |kick| kick.swing),
            foot: self.kick.map_or(0.0, |kick| kick.foot),
            power: kicking.map_or(0.0, |kick| kick.power * kick.blend),
            // A keeper rolling the ball out and one hurling it to the halfway
            // line are the same movement; the gentlest of them still has to
            // read as a throw, hence the floor.
            throwing: throwing.map_or(0.0, |kick| kick.power.max(0.45) * kick.blend),
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
                    * (1.0 - grounded)
            } else {
                0.0
            },
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
        let mut actor = PlayerActor::new(1, true);
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
        let mut actor = PlayerActor::new(1, true);
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

    /// An outfielder heading a ball is a metre off the ground and is still
    /// not diving.
    #[test]
    fn an_outfielder_never_dives() {
        let mut actor = PlayerActor::new(2, false);
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
            let (_, velocity, delay) =
                Actors::next_impact(&mut ball, now).unwrap_or_else(|| panic!("missed at {now}"));
            assert!(
                (delay - expected).abs() < 1e-3,
                "at {now} ms the wait is {delay}, not {expected}"
            );
            assert!(
                (velocity.length() - 15.2).abs() < 1.0,
                "struck at {} m/s",
                velocity.length()
            );
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
        let mut actor = PlayerActor::new(7, false);
        let coming = |delay: f32| {
            Some(Impact {
                by: 7,
                velocity: Vec3::new(0.0, 0.0, 20.0),
                delay,
            })
        };

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
        let mut actor = PlayerActor::new(7, false);
        actor.swing_leg(
            Some(Impact {
                by: 7,
                velocity: Vec3::new(0.0, 0.0, 20.0),
                delay: 0.0,
            }),
            1.0 / 60.0,
            false,
        );
        let kick = actor.kick.unwrap();
        assert!(kick.blend < 0.4, "no onset ramp: {}", kick.blend);
        assert!(kick.swing.abs() < 1e-3);
    }

    /// Scrubbing the playhead cancels the swing rather than leaving a leg in
    /// the air.
    #[test]
    fn a_seek_cancels_the_swing() {
        let mut actor = PlayerActor::new(7, false);
        actor.swing_leg(
            Some(Impact {
                by: 7,
                velocity: Vec3::new(0.0, 0.0, 20.0),
                delay: 0.06,
            }),
            0.03,
            false,
        );
        assert!(actor.kick.is_some());
        actor.swing_leg(None, 0.03, true);
        assert!(actor.kick.is_none());
    }
}
