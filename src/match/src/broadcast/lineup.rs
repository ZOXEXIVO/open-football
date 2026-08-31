//! **The teams walked out**: both elevens standing in one line inside the near
//! touchline before a ball is kicked, and the camera that goes down the line.
//!
//! Football on television does not open on a kickoff. It opens high over
//! the ground, finds the two sides lined up in front of the main stand,
//! comes down to them, and walks the line — one slow pass along the faces,
//! left to right across the frame. Then it cuts to the gantry and the match
//! starts.
//!
//! That is exactly what this is, and it has three beats — **one unbroken
//! camera move**, because there is no cut anywhere inside the ceremony:
//!
//! 1. **The approach.** It opens [`Lineup::OVERHEAD`] metres over the
//!    centre spot, the line a row of figures in front of the stand, and
//!    flies toward them from behind, sinking as it goes and arriving at eye
//!    level behind the end of the line. It is also what is up while the
//!    squad is being dressed — see [`Act::Assembling`], the one beat with
//!    no fixed length.
//! 2. **The corner.** Round the end of the line and onto the front of it,
//!    shedding speed as it comes round.
//! 3. **One long pass along the faces of the whole line** at eye level —
//!    slow enough to look at each man as he goes by, crossing the frame
//!    left to right. (A camera on the gantry side has world +x on the LEFT
//!    of its frame, so the dolly travels toward −x.) Past the last man
//!    comes the cut to the gantry and the kickoff.
//!
//! ⚠ **No plate is drawn over anybody for any of it**, and that is not an
//! omission — it is where the names come from. The ceremony names its men
//! off the shirt they are standing in: a print across the FRONT of the
//! shirt — the same panel, the same lettering as the back, worn for the
//! walk-out and nothing else — read by the pass along the faces, which is
//! the one beat close enough to read anything. The name plate that follows
//! a footballer through the match is held back until the ceremony hands the
//! pitch over. See [`Lineup::wear_the_name`] and
//! [`FrontPrint`](crate::players::body::FrontPrint).
//!
//! ## What it costs the replay, which is nothing
//!
//! **The ceremony is not in the recording and never could be.** The engine's
//! clock starts at the kickoff; there is no tick before it and nothing the
//! match knows about a line of men standing still. So this is staged entirely
//! on this side of the boundary, out of the team sheets the page already
//! sends — and the playhead is *parked* while it runs rather than advanced,
//! which is why a match that opens with fifteen seconds of ceremony still
//! opens at 0:00.
//!
//! ⚠ **Parking it at zero rather than leaving it where the loader put it** is
//! what makes the ceremony safe on a goals-only recording, which is what the
//! game actually writes. There the loader has already carried the playhead to
//! the first clip — the sixty-third minute, say, a second after a goal —
//! and every part of the viewer that reads the recording would be reading THAT:
//! eleven men celebrating, a keeper picking the ball out of his net, a mood on
//! [`Aftermath`](crate::players::aftermath::Aftermath) that would have half the
//! line standing to attention with their heads in their hands. At zero there
//! are no samples for anybody and all of it is quiet. [`Lineup::resume_at`]
//! holds where the replay is going back to.
//!
//! ## It is skippable, and everything counts as a skip
//!
//! Space, the play button, a drag on the seek rail, a click on a player, a
//! hand on the orbit — any of them ends the ceremony on the frame it happens.
//! The one thing that does not is the loader's own autoplay, which is the
//! signal the ceremony *starts* on: see [`Lineup::hold`], where the two are
//! told apart.

use crate::app::config::{PlayerInfo, ViewerConfig};
use crate::broadcast::camera::{CameraFlight, CameraOrbit, CameraZoom};
use crate::broadcast::focus::CameraSubject;
use crate::players::actors::{PlayerActor, Undressed};
use crate::players::body::FrontPrint;
use crate::recording::loader::ChunkLoader;
use crate::recording::playback::Playback;
use crate::scene::field::Field;
use bevy::prelude::*;
use std::f32::consts::PI;

/// Where one man stands in the line, and which way he is turned.
#[derive(Clone, Copy)]
struct Stand {
    id: u32,
    at: Vec3,
    heading: f32,
}

/// Where the camera is and how it is lensed for one frame of the ceremony.
#[derive(Clone, Copy)]
struct Shot {
    stand: Vec3,
    aim: Vec3,
    lens: f32,
}

/// Which beat the ceremony is on.
#[derive(Clone, Copy, Default, PartialEq)]
enum Act {
    /// Nothing has landed yet. The gantry has the picture and this is only
    /// watching for the recording.
    #[default]
    Waiting,
    /// The recording is in, the playhead is parked and the squad is being
    /// dressed a few men a frame. The opening aerial is held over it — and
    /// from [`Lineup::OVERHEAD`] metres over the centre spot, twenty-two
    /// bodies appearing three at a time is a line assembling in the
    /// distance rather than men popping into existence.
    Assembling,
    /// Running, seconds of REAL time into the walk down the line.
    Running(f32),
    /// The frame the ceremony hands everything back: the playhead is restored,
    /// the men are released and the picture returns to the gantry. One frame
    /// only — [`Lineup::pose`] needs a turn after [`Lineup::hold`] has decided
    /// it is over.
    Releasing,
    /// Over, and it never runs twice.
    Done,
}

/// The pre-match line, and the camera that walks it.
#[derive(Resource, Default)]
pub struct Lineup {
    act: Act,
    /// The line itself, worked out once from the team sheets: the home eleven
    /// first, then the away eleven, left to right along the pitch.
    row: Vec<Stand>,
    /// **Where the replay is going back to** — where the loader had left the
    /// playhead when the ceremony took the picture over. Zero on a full
    /// recording, the first clip's start on a goals-only one.
    resume_at: f64,
    /// Whether the loader was ready on the previous frame, so the frame it
    /// BECOMES ready can be told from every frame after it. See
    /// [`Self::hold`], where that distinction is the whole of the arming rule.
    saw_ready: bool,
    /// Real seconds spent waiting for the last man to be dressed. Bounded by
    /// [`Self::PATIENCE`]: a squad that never finishes assembling must not
    /// hold the football hostage.
    assembling: f32,
    /// This frame's camera, or `None` when the ceremony does not have the
    /// picture.
    shot: Option<Shot>,
}

impl Lineup {
    /// The most men a side puts in the line. A team sheet is eleven and the
    /// bench does not walk out.
    const ELEVEN: usize = 11;
    /// **How far inside the near touchline the line stands**, in metres.
    ///
    /// Far enough in that BOTH passes are on the grass. The faces pass stands
    /// [`Self::FRONT`] metres in front of the line, which at anything under
    /// four and a bit metres of inset would put it out in the run-off behind
    /// the advertising hoardings — the wall that pins
    /// [`ChangeoverShot::OUT`](crate::broadcast::changeover::ChangeoverShot)
    /// from below, and the one thing that would cut a row of men off at the
    /// knee.
    const INSET: f32 = 7.0;
    /// **Shoulder to shoulder, in metres** — and it is nearly literally that.
    ///
    /// A man is about 0.53 m across the arms (`Physique::SHOULDER_SPREAD` is
    /// 0.176 either side of his centreline, plus the arms hanging outside
    /// them), so this leaves about a foot of daylight between two of them:
    /// close enough to read as a line-up with arms linked, far enough that the
    /// bodies do not intersect on the wider builds. Down from 1.25
    /// (2026-08-26, maintainer: *"players should stand more compactly"*) —
    /// and every metre of line is also a metre and a half of pass, so a
    /// packed line keeps the ceremony moving.
    const SPACING: f32 = 0.85;
    /// And the gap between the two teams, which is what makes it read as two
    /// elevens rather than one crowd of twenty-two. Two and a half times the
    /// spacing, so it is unmistakably a break in the line.
    const DIVIDE: f32 = 2.2;
    /// Where the line stands, across the pitch.
    const ROW_Z: f32 = -(Field::HALF_WIDTH - Self::INSET);
    /// **Which way they are all turned**: at the main stand, which is the
    /// negative-z touchline the gantry sits behind.
    ///
    /// The model is built looking down +Z and the actor is rotated about Y by
    /// `atan2(x, z)`, so half a turn carries him onto −Z. It is the bearing a
    /// team faces for the anthems and the team photograph, and it is what puts
    /// the faces pass — which comes at them from the stand side — down their
    /// own eyeline.
    const AT_THE_STAND: f32 = PI;

    /// **Where the pass along the faces stands: how far off the line, and how
    /// high** — eye level, close enough that a man is a portrait.
    ///
    /// A face is a tenth of the size of a shirt, so this is as close as the
    /// line allows — it still leaves the lens two and a half metres inside the
    /// touchline, which is the bound [`Self::INSET`] was chosen for.
    ///
    /// ⚠ **The stand-off is also the RADIUS the corner swings on** — the
    /// pass and the corner are one circle's worth of geometry, which is part
    /// of what lets the whole ceremony be a single unbroken camera move.
    const FRONT: (f32, f32) = (4.3, 1.72);
    /// What the beats aim at, in metres up a man: his eyes on the pass along
    /// the faces, and the shoulder line of the row for the approach's locked
    /// look at it.
    const SHOULDERS: f32 = 1.38;
    const EYES: f32 = 1.60;
    /// **How far ahead of itself the lens looks as it travels**, as a fraction
    /// of how far it is standing off the line.
    ///
    /// A dolly whose aim is square to its own rail is a machine going past a
    /// row of objects. Leading it a little is what makes it read as an
    /// operator walking the line.
    ///
    /// ⚠ **A fraction rather than a distance, so the lead is the same ANGLE
    /// whatever the pass is standing off.** Fixed at a metre and a bit it was
    /// a sixth of the way across the frame from six and a half metres out and
    /// a quarter of it from four — and a quarter of the frame is 17 degrees,
    /// which on a portrait is the difference between a man looking down the
    /// lens and a man in three-quarter profile. Measured on a rendered pass:
    /// the framed man came out 12 degrees off face-on.
    const LEAD: f32 = 0.20;
    /// How far past the end man the pass runs at either end, in metres. Enough
    /// that the last man crosses the whole frame rather than stopping in the
    /// middle of it — and no more, because at [`Self::PACE`] every metre of it
    /// is the better part of a second and a half of empty touchline. The
    /// high end of it is also the post the corner pivots on: the swing onto
    /// the faces happens beyond the end man, so the pass opens already
    /// clear of him.
    const RUN_ON: f32 = 1.8;
    /// The lens the pass is held on, as a multiple of the wheel's own factor —
    /// under one is wider than the broadcast lens, which every shot here is:
    /// each has its subject a few metres away instead of eighty, so what it
    /// has to buy with the lens is ANGLE and not magnification. See
    /// [`Self::magnification`].
    ///
    /// **Three notches of the wheel wider than the 0.66 it was tuned at**
    /// (2026-08-30, maintainer: zoom the faces pass out by two clicks of the
    /// mouse wheel, then one more) — and written as clicks of the wheel's own
    /// step rather than as the number they come to, because this constant is
    /// QUOTED in the wheel's units: a click is a ratio of
    /// [`CameraZoom::STEP`], so "a click wider" is exact here and stays a
    /// click of the actual wheel if that step is ever retuned.
    ///
    /// Every beat of the ceremony is held on it — which is part of what keeps
    /// the move unbroken: nothing ever has to change lens.
    const FRONT_LENS: f32 =
        0.66 / (CameraZoom::STEP * CameraZoom::STEP * CameraZoom::STEP);
    /// **How fast the pass crosses the line, in metres a second.**
    ///
    /// The ceremony is wall-clock — the playhead is parked for all of it, so
    /// there is no match time to measure it in and the transport speed does
    /// not touch it — and this is the number most of its length comes out
    /// of: twenty-three metres of travel at seven tenths of a metre a second
    /// is thirty-three seconds. `glide` runs the middle of that at about 0.88.
    ///
    /// Walked back four times on the maintainer's instruction, and every one
    /// of them the same instruction: to a THIRD of the six metres a second the
    /// pass opened at (2026-08-26) — a man and a half of frame per second,
    /// fast enough to read a name off a shirt and much too fast to look at
    /// anybody — then 30% slower, 30% slower again, and 30% slower once more
    /// (2026-08-29). The lens now takes a second and a quarter to travel from
    /// one man to the next, and the whole ceremony — the pass, the corner and
    /// the fly home — runs a little over forty seconds.
    ///
    /// ⚠ Which is long, and deliberately survivable: every way of asking for
    /// the football ends it on the frame it is asked — see [`Self::hold`].
    const PACE: f32 = 0.686;
    /// **How fast the approach crosses the ground, in metres a second** —
    /// eight times the pass, because they are different sentences: the
    /// approach says *here is the ground and here are the teams*, and the
    /// pass says *now look at each of these men*. The approach holds this
    /// the whole way in, and the corner sheds it down to the pass's crawl.
    const FLY: f32 = 5.5;
    /// **How far out from the corner the approach straightens up**, in
    /// metres — the middle handle of its curve ([`Self::approach_point`]).
    /// The middle of the field is off to the side of the line's own axis,
    /// and the corner needs the camera arriving ALONG that axis; the handle
    /// is what turns one heading into the other without a kink at the seam.
    const HANDLE: f32 = 6.0;
    /// …and how high over the centre spot the ceremony OPENS, in metres.
    /// High enough that the first frame reads as the whole ground with a
    /// line of men in it rather than a patch of turf. The descent is EASED,
    /// so the opening hangs still for a beat and the arrival at the corner
    /// is dead level.
    const OVERHEAD: f32 = 13.0;
    /// How finely the approach's curve is walked when it is measured and
    /// travelled: enough legs that each is well under half a metre, so the
    /// polyline is within a millimetre or two of the true curve.
    const APPROACH_STEPS: usize = 64;
    /// **How long the wide will wait for the last man to be dressed**, in real
    /// seconds, before it starts anyway.
    ///
    /// [`Actors::take_the_field`](crate::players::actors::Actors::take_the_field)
    /// dresses three men a frame, so a full squad is a fifth of a second at
    /// sixty frames — but every one of the first few brings a shader the
    /// browser stops to link, and a machine that takes four seconds over that
    /// must not also be a machine where the football never starts.
    const PATIENCE: f32 = 6.0;
    /// How much of a full smoothstep a dolly leans into its move.
    ///
    /// ⚠ **Not all of it.** A smoothstep starts and ends at rest, and on a
    /// six-second pass that spends the first second not moving and runs the
    /// middle of it at half as fast again as the mean — which is precisely the
    /// stretch the pass exists for, and precisely the pace a name printed
    /// across a shirt cannot be read at. Leaning a little over half way there
    /// gives a move that comes off the mark at 45% of its own average and
    /// peaks at 128%, which is an operator rather than an animation.
    const LEAN: f32 = 0.55;

    /// Forms the line up, once, from the team sheets the page sent.
    ///
    /// `Startup` rather than per-frame for the reason
    /// [`ChangeoverShot::arm`](crate::broadcast::changeover::ChangeoverShot::arm)
    /// is: the answer cannot change. Who started is fixed by the time the page
    /// hands the document over.
    pub fn arm(mut commands: Commands, config: Res<ViewerConfig>) {
        let row = if config.lineup {
            Self::row_of(
                &Self::team_sheet(&config, true),
                &Self::team_sheet(&config, false),
            )
        } else {
            Vec::new()
        };
        commands.insert_resource(Lineup {
            // A document with nobody in it, or a page that asked for no
            // ceremony, is over before it starts.
            act: if row.len() < 2 { Act::Done } else { Act::Waiting },
            row,
            ..default()
        });
    }

    /// The eleven who started, on one side, in team-sheet order.
    ///
    /// ⚠ **The positional fallback is not a guess.** Both producers of this
    /// document — the web crate and the `.dev/match` harness — write a side's
    /// starters before its bench, so the first eleven of a side ARE the
    /// eleven, and a document written before the sheet said so still walks the
    /// right men out. `starting` is carried anyway, because relying on the
    /// order of a list is the kind of contract that holds until somebody sorts
    /// it.
    fn team_sheet<'a>(config: &'a ViewerConfig, home: bool) -> Vec<&'a PlayerInfo> {
        let named: Vec<&PlayerInfo> = config
            .players
            .iter()
            .filter(|player| player.is_home == home && player.starting)
            .take(Self::ELEVEN)
            .collect();
        if !named.is_empty() {
            return named;
        }
        config
            .players
            .iter()
            .filter(|player| player.is_home == home)
            .take(Self::ELEVEN)
            .collect()
    }

    /// Stands the two elevens up: one line, centred on the halfway line, with
    /// [`Self::DIVIDE`] between the sides.
    ///
    /// ⚠ **The home eleven stands at the LOW-x end, and that order is the shot
    /// list's direction.** The pass opens at the low end of the line, so the
    /// viewer is shown their own team's faces before the visitors' — and the
    /// corner turns round the away end, because that is where the pass runs
    /// out of men.
    fn row_of(home: &[&PlayerInfo], away: &[&PlayerInfo]) -> Vec<Stand> {
        if home.is_empty() || away.is_empty() {
            return Vec::new();
        }
        let span = |men: usize| (men.max(1) - 1) as f32 * Self::SPACING;
        let width = span(home.len()) + Self::DIVIDE + span(away.len());
        let mut row = Vec::with_capacity(home.len() + away.len());
        let mut at = -width * 0.5;
        for player in home {
            row.push(Stand {
                id: player.id,
                at: Vec3::new(at, 0.0, Self::ROW_Z),
                heading: Self::AT_THE_STAND,
            });
            at += Self::SPACING;
        }
        at += Self::DIVIDE - Self::SPACING;
        for player in away {
            row.push(Stand {
                id: player.id,
                at: Vec3::new(at, 0.0, Self::ROW_Z),
                heading: Self::AT_THE_STAND,
            });
            at += Self::SPACING;
        }
        row
    }

    /// Runs the ceremony's clock, holds the playhead still while it runs, and
    /// hands everything back when it is over.
    ///
    /// ⚠ **Registered between [`Playback::handle_keyboard`] and
    /// [`Playback::advance`], and both sides of that matter.** After the
    /// keyboard, the transport bar and the pick, so a viewer who asks for the
    /// football gets it on the frame he asks — every one of those turns
    /// `playing` on, and this is what reads that as a skip. Before `advance`,
    /// so the playhead this parks is the playhead that frame draws.
    pub fn hold(
        time: Res<Time<Real>>,
        loader: Res<ChunkLoader>,
        subject: Res<CameraSubject>,
        flight: Res<CameraFlight>,
        orbit: Res<CameraOrbit>,
        squad: Query<(&PlayerActor, Has<Undressed>)>,
        mut playback: ResMut<Playback>,
        mut lineup: ResMut<Lineup>,
    ) {
        // Somebody looking at something of their own. The same three the
        // substitution shot stands down for, and for the same reason: a man
        // being followed by hand, a rig in flight and a gantry that has been
        // walked round the ground are all a viewer who did not ask for this.
        let hands_off = subject.locked() || flight.airborne() || orbit.bearing.abs() > 1e-3;

        match lineup.act {
            Act::Done => return,
            // One frame behind [`Self::pose`], which needs a turn to let the
            // men go before the resource forgets it ever held them.
            Act::Releasing => {
                lineup.act = Act::Done;
                return;
            }
            Act::Waiting => {
                let ready = loader.ready;
                let arrived = ready && !lineup.saw_ready;
                lineup.saw_ready = ready;
                if !ready {
                    // ⚠ **`playing` while the recording is still in flight can
                    // only be the viewer**, because the loader's own autoplay
                    // is the thing that sets `ready` in the first place. That
                    // is the whole trick that lets the ceremony start on an
                    // autoplay and stand down for a keypress which looks
                    // exactly like one.
                    //
                    // ⚠⚠ **And `seeked` is deliberately NOT in that list**,
                    // though it is in the one below. The loader carries the
                    // playhead to the first clip on the frame the metadata
                    // lands — that is what opening a goals-only recording on
                    // the first goal rather than on an empty pitch IS — and it
                    // sets `seeked` doing it, one frame before the first chunk
                    // arms this. Read as a viewer scrubbing, it stood the
                    // ceremony down on every recording the game actually
                    // writes. A real drag out here costs nothing either:
                    // `resume_at` is taken when the ceremony arms, so wherever
                    // the rail was left is where the match starts.
                    if hands_off || playback.playing {
                        lineup.act = Act::Done;
                    }
                    return;
                }
                // A goalless clip recording keeps nothing. There is no match
                // behind the ceremony and no squad to dress for it.
                if !arrived || loader.nothing_to_play() || hands_off {
                    lineup.act = Act::Done;
                    return;
                }
                lineup.resume_at = playback.time_ms;
                playback.time_ms = 0.0;
                playback.playing = false;
                // The men are about to be stood somewhere the recording never
                // put them. Everything that reads a body off consecutive frames
                // has to be told this is a jump — see
                // [`Actors::animate`](crate::players::actors::Actors::animate),
                // whose whole notion of speed is one frame's displacement.
                playback.seeked = true;
                lineup.act = Act::Assembling;
                lineup.assembling = 0.0;
            }
            Act::Assembling | Act::Running(_) => {
                // `playing` is the ceremony's to hold from here, so anything
                // that turns it back on is somebody asking for the football.
                if hands_off || playback.playing || playback.seeked {
                    let scrubbed = playback.seeked;
                    lineup.release(&mut playback, !scrubbed);
                    return;
                }
            }
        }

        let delta = time.delta_secs();
        match lineup.act {
            Act::Assembling => {
                lineup.assembling += delta;
                let dressed = lineup.row.iter().all(|stand| {
                    squad
                        .iter()
                        .any(|(actor, undressed)| actor.id == stand.id && !undressed)
                });
                if dressed || lineup.assembling >= Self::PATIENCE {
                    lineup.act = Act::Running(0.0);
                }
            }
            Act::Running(into) => {
                let into = into + delta;
                if into >= lineup.total() {
                    lineup.release(&mut playback, true);
                    return;
                }
                lineup.act = Act::Running(into);
            }
            _ => {}
        }

        // Belt and braces: `advance` is switched off for the whole ceremony,
        // and the playhead being where the ceremony put it is load-bearing for
        // everything that reads the recording underneath it.
        playback.time_ms = 0.0;

        let into = match lineup.act {
            Act::Running(into) => into,
            _ => 0.0,
        };
        lineup.shot = Some(lineup.shot_at(into));
    }

    /// Hands the replay back.
    ///
    /// `restore` is false for exactly one route out — a drag on the seek rail,
    /// which has already said where the playhead belongs and must not be
    /// overruled by where the ceremony found it. Every other route (the end of
    /// the walk, the play button, the space bar, a click on a player) means
    /// "start the match", and the match starts where the loader had it.
    fn release(&mut self, playback: &mut Playback, restore: bool) {
        self.act = Act::Releasing;
        self.shot = None;
        if !restore {
            return;
        }
        playback.time_ms = self.resume_at;
        playback.playing = true;
        playback.seeked = true;
        // …and a CUT, not merely a seek. The frame before this is a line of
        // men standing on a touchline and the frame after it is a kickoff:
        // there is no continuity to preserve, so the replay fades the football
        // in through the vignette rather than snapping to it. See
        // [`crate::broadcast::cut`], which is careful about why a scrub does
        // not get the same treatment.
        playback.cut = true;
    }

    /// Stands the men in the line, and takes everybody else off the pitch.
    ///
    /// ⚠ **Registered after
    /// [`Actors::take_the_field`](crate::players::actors::Actors::take_the_field)
    /// and before
    /// [`Actors::animate`](crate::players::actors::Actors::animate).** The
    /// first is what puts a body on a man at all; the second poses whatever
    /// this leaves behind, and reading a frame-old position would have every
    /// man in the line sprinting on the spot.
    pub fn pose(
        lineup: Res<Lineup>,
        mut actors: Query<(
            &mut PlayerActor,
            &mut Transform,
            &mut Visibility,
            Has<Undressed>,
        )>,
    ) {
        match lineup.act {
            Act::Assembling | Act::Running(_) => {}
            // The one frame the ceremony gives everybody back.
            // `follow_playhead` has already put them where the recording says
            // they are; all this has to do is stop claiming them.
            Act::Releasing => {
                for (mut actor, _, _, _) in &mut actors {
                    actor.at_ease();
                }
                return;
            }
            _ => return,
        }

        for (mut actor, mut transform, mut visibility, undressed) in &mut actors {
            match lineup.stand_of(actor.id) {
                // ⚠ **Undressed is not the same as absent.** A man with no
                // body yet has no meshes to draw, but he does have a contact
                // shadow and a name plate and both of them hang off this
                // flag — so he waits in the wings until the frame he is
                // assembled on.
                Some(stand) if !undressed => {
                    transform.translation.x = stand.at.x;
                    transform.translation.z = stand.at.z;
                    actor.stand_to_attention(stand.heading);
                    if *visibility != Visibility::Inherited {
                        *visibility = Visibility::Inherited;
                    }
                }
                _ => {
                    if *visibility != Visibility::Hidden {
                        *visibility = Visibility::Hidden;
                    }
                    actor.at_ease();
                }
            }
        }
    }

    /// **Puts the name across the front of the shirt on for the walk-out, and
    /// takes it off again.**
    ///
    /// The print is the same one as the back of the shirt — same panel, same
    /// lettering, same material — half a turn round the body, and it is the
    /// only thing naming anybody during the ceremony: the plate that follows a
    /// footballer through the match is held back for all of it (see
    /// [`Actors::place_labels`](crate::players::actors::Actors::place_labels)).
    /// The pass comes down the FRONT at four metres, where this print is —
    /// the one beat of the ceremony close enough to read a name at all.
    ///
    /// Every print in the squad, not only the line's, and it does not have to
    /// be choosier than that: [`Self::pose`] hides every man who is not in the
    /// row outright, so a substitute wearing his name is a substitute nobody
    /// can see. `on()` is the whole test — the print is worn while the ceremony
    /// has the picture and at no other moment of the match.
    ///
    /// Written only on a change, like the contact shadows and the name plates:
    /// this is twenty-two comparisons a frame and, for all but two frames of a
    /// match, twenty-two writes it does not do.
    pub fn wear_the_name(
        lineup: Res<Lineup>,
        mut prints: Query<&mut Visibility, With<FrontPrint>>,
    ) {
        let wanted = if lineup.on() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        for mut visibility in &mut prints {
            if *visibility != wanted {
                *visibility = wanted;
            }
        }
    }

    /// Where this man stands, if he is in the line at all.
    fn stand_of(&self, id: u32) -> Option<&Stand> {
        self.row.iter().find(|stand| stand.id == id)
    }

    /// The x of the first and last man in the line.
    fn ends(&self) -> (f32, f32) {
        let first = self.row.first().map_or(0.0, |stand| stand.at.x);
        let last = self.row.last().map_or(0.0, |stand| stand.at.x);
        (first.min(last), first.max(last))
    }

    /// How long the pass along the faces takes, in seconds — the ground it
    /// has to cover at [`Self::PACE`].
    ///
    /// A method rather than a constant because the ground depends on how many
    /// men there are, and a document with a short team sheet in it must not
    /// leave the camera crawling past an empty touchline for the difference.
    fn walk_seconds(&self) -> f32 {
        let (first, last) = self.ends();
        ((last - first) + Self::RUN_ON * 2.0) / Self::PACE
    }

    /// The speed the pass opens and closes at: [`Self::glide`]'s slope is
    /// `1 − LEAN` at both of its ends, just under half the pass's own
    /// average. The corner sheds the approach's [`Self::FLY`] down to
    /// exactly here, so the hand-over onto the faces has no lurch in it.
    fn crawl() -> f32 {
        Self::PACE * (1.0 - Self::LEAN)
    }

    /// How long the corner round the end of the line takes, in seconds.
    ///
    /// ⚠ **A consequence of what it has to join, not a number of its own.**
    /// The corner covers half a circle of [`Self::FRONT`].0 while its speed
    /// sheds steadily from the [`Self::FLY`] the approach arrives at to the
    /// crawl the pass opens on — so neither seam has a lurch in it, and the
    /// duration is simply that distance over the average of the two speeds.
    fn swing_seconds(&self) -> f32 {
        PI * Self::FRONT.0 * 2.0 / (Self::crawl() + Self::FLY)
    }

    /// …and the approach: the ground its own curve covers, at [`Self::FLY`].
    fn approach_seconds(&self) -> f32 {
        self.approach_length() / Self::FLY
    }

    /// The whole ceremony, in seconds: the approach, the corner, the pass.
    fn total(&self) -> f32 {
        self.approach_seconds() + self.swing_seconds() + self.walk_seconds()
    }

    /// The camera, `into` seconds in.
    ///
    /// ⚠ **No cut anywhere.** One move: down from over the centre spot to
    /// behind the line, round the end man onto the faces, and left to right
    /// along the whole line — the three beats are one path, continuous in
    /// position AND in speed at both seams (see [`Self::approach_point`] and
    /// [`Self::round_the_end`]), and the only cut the ceremony has is the
    /// one at the very end, to the kickoff.
    fn shot_at(&self, into: f32) -> Shot {
        let (first, last) = self.ends();
        let start = first - Self::RUN_ON;
        let end = last + Self::RUN_ON;
        let approach = self.approach_seconds();
        if into < approach {
            let gone = (into * Self::FLY).min(self.approach_length());
            return self.approach(gone);
        }
        let swing = self.swing_seconds();
        if into < approach + swing {
            return self.round_the_end(end, into - approach);
        }
        let walk = self.walk_seconds();
        let along = Self::glide(((into - approach - swing) / walk.max(1e-3)).clamp(0.0, 1.0));
        Self::in_front(end + (start - end) * along)
    }

    /// The pass: in front of them at eye level, crossing the frame left to
    /// right — and leading ahead of itself, down the line.
    fn in_front(at: f32) -> Shot {
        Shot {
            stand: Vec3::new(at, Self::FRONT.1, Self::ROW_Z - Self::FRONT.0),
            aim: Vec3::new(at - Self::FRONT.0 * Self::LEAD, Self::EYES, Self::ROW_Z),
            lens: Self::FRONT_LENS,
        }
    }

    /// **The corner: half a circle round the end of the line**, `t` seconds
    /// into the swing, pivoting on the high end of the pass's run-on — from
    /// behind the line, round the end man, onto the faces.
    ///
    /// The approach's last position and the pass's first are the two ends of
    /// one semicircle of radius [`Self::FRONT`].0 centred on the line, so
    /// the path is continuous by construction. The SPEED is continuous by
    /// construction too: the camera decelerates steadily along the arc from
    /// [`Self::FLY`] to [`Self::crawl`] — an operator easing out of a run,
    /// not a machine changing gear.
    ///
    /// The height holds eye level for the whole turn — the descent belongs
    /// to the approach — and the look is carried from the middle of the row
    /// round to the first face, on a smoothstep of ground covered, so it is
    /// not still swinging at either seam.
    fn round_the_end(&self, corner: f32, t: f32) -> Shot {
        let swing = self.swing_seconds().max(1e-3);
        let gone = Self::FLY * t - (Self::FLY - Self::crawl()) * t * t / (2.0 * swing);
        let round = (gone / Self::FRONT.0).clamp(0.0, PI);
        let over = Self::ease((gone / (PI * Self::FRONT.0)).clamp(0.0, 1.0));
        Shot {
            stand: Vec3::new(
                corner + Self::FRONT.0 * round.sin(),
                Self::FRONT.1,
                Self::ROW_Z + Self::FRONT.0 * round.cos(),
            ),
            aim: Self::midline().lerp(Self::in_front(corner).aim, over),
            lens: Self::FRONT_LENS,
        }
    }

    /// Where the approach looks: the middle of the row, at shoulder height.
    /// Locked for the whole flight in — the ground rushes underneath while
    /// the subject holds still, which is what reads as an approach.
    fn midline() -> Vec3 {
        Vec3::new(0.0, Self::SHOULDERS, Self::ROW_Z)
    }

    /// **The approach's path**, `t` in 0..1: from [`Self::OVERHEAD`] metres
    /// over the centre spot down to behind the high end of the line.
    ///
    /// On the ground it is a quadratic curve whose middle handle sits
    /// [`Self::HANDLE`] metres short of the arrival, on the line's own axis,
    /// so the camera arrives travelling the way the corner opens and the
    /// seam has no kink in it; the first point is the centre spot — the
    /// origin, so its term vanishes. The descent is eased on top, which
    /// hangs the opening still for a beat and lands the arrival dead level.
    fn approach_point(&self, t: f32) -> Vec3 {
        let (_, last) = self.ends();
        let end = last + Self::RUN_ON;
        let back = Vec2::new(end, Self::ROW_Z + Self::FRONT.0);
        let handle = Vec2::new(end - Self::HANDLE, Self::ROW_Z + Self::FRONT.0);
        let one = 1.0 - t;
        let flat = handle * (2.0 * one * t) + back * (t * t);
        Vec3::new(
            flat.x,
            Self::FRONT.1 + (Self::OVERHEAD - Self::FRONT.1) * Self::ease(one),
            flat.y,
        )
    }

    /// How much ground the approach covers, in metres — measured along the
    /// path, descent included, so [`Self::FLY`] means what it says.
    fn approach_length(&self) -> f32 {
        let mut length = 0.0;
        let mut previous = self.approach_point(0.0);
        for step in 1..=Self::APPROACH_STEPS {
            let next = self.approach_point(step as f32 / Self::APPROACH_STEPS as f32);
            length += next.distance(previous);
            previous = next;
        }
        length
    }

    /// The approach, `gone` metres along its own path — walked by distance
    /// rather than by the curve's parameter, so the camera crosses the
    /// ground at [`Self::FLY`] the whole way and the corner receives it with
    /// no step in speed.
    fn approach(&self, gone: f32) -> Shot {
        let mut travelled = 0.0;
        let mut previous = self.approach_point(0.0);
        let mut stand = previous;
        for step in 1..=Self::APPROACH_STEPS {
            let next = self.approach_point(step as f32 / Self::APPROACH_STEPS as f32);
            let leg = next.distance(previous);
            if travelled + leg >= gone {
                stand = previous.lerp(next, (gone - travelled) / leg.max(1e-6));
                break;
            }
            travelled += leg;
            previous = next;
            stand = next;
        }
        Shot {
            stand,
            aim: Self::midline(),
            lens: Self::FRONT_LENS,
        }
    }

    /// One dolly's travel, 0..1, eased by [`Self::LEAN`].
    fn glide(along: f32) -> f32 {
        along + Self::LEAN * (Self::ease(along) - along)
    }

    /// Smoothstep.
    fn ease(along: f32) -> f32 {
        along * along * (3.0 - 2.0 * along)
    }

    /// True while the ceremony owns the picture.
    pub fn on(&self) -> bool {
        self.shot.is_some()
    }

    /// Where the camera stands and what it looks at, or `None` to leave the
    /// broadcast rig alone.
    pub fn framing(&self) -> Option<(Vec3, Vec3)> {
        self.shot.map(|shot| (shot.stand, shot.aim))
    }

    /// How the lens is held, as a multiple of the wheel's own factor. One when
    /// the ceremony is not on, which is the identity
    /// [`TvCamera::follow_play`](crate::broadcast::camera::TvCamera::follow_play)
    /// multiplies through.
    pub fn magnification(&self) -> f32 {
        self.shot.map_or(1.0, |shot| shot.lens)
    }

    /// Whether this man still has to be dressed for the ceremony, whatever the
    /// recording says about him.
    ///
    /// ⚠ **The recording is not the authority here and cannot be.** A man is
    /// normally built when the playhead comes within a few seconds of his
    /// first recorded sample; on a goals-only recording of a match he was
    /// substituted out of before the first goal, there is no such sample
    /// anywhere in the document — and he still started, and still walks out.
    pub fn wants(&self, id: u32) -> bool {
        match self.act {
            Act::Waiting | Act::Assembling | Act::Running(_) => {
                self.row.iter().any(|stand| stand.id == id)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broadcast::camera::TvCamera;

    fn sheet(ids: &[u32], home: bool) -> Vec<PlayerInfo> {
        ids.iter()
            .map(|id| PlayerInfo {
                id: *id,
                shirt_number: 1,
                last_name: "Okocha".to_string(),
                position: "MC".to_string(),
                is_home: home,
                starting: true,
                skin: 0,
                hair: 0,
                eyes: 0,
                photo: None,
                face: None,
            })
            .collect()
    }

    /// Both elevens, standing in the line they stand in.
    fn walked_out() -> Lineup {
        let home = sheet(&(1..=11).collect::<Vec<_>>(), true);
        let away = sheet(&(21..=31).collect::<Vec<_>>(), false);
        let home: Vec<&PlayerInfo> = home.iter().collect();
        let away: Vec<&PlayerInfo> = away.iter().collect();
        Lineup {
            row: Lineup::row_of(&home, &away),
            act: Act::Running(0.0),
            ..default()
        }
    }

    #[test]
    fn the_two_teams_stand_in_one_line_inside_the_near_touchline() {
        let lineup = walked_out();
        assert_eq!(lineup.row.len(), 22);
        for stand in &lineup.row {
            assert_eq!(stand.at.z, Lineup::ROW_Z);
            // On the grass, and far enough in that the faces pass — which
            // stands in front of them — is on it too.
            assert!(
                stand.at.z > -Field::HALF_WIDTH,
                "a man is standing off the pitch at {}",
                stand.at.z
            );
            assert!(
                stand.at.z - Lineup::FRONT.0 > -Field::HALF_WIDTH,
                "the faces pass would be out in the run-off"
            );
            assert!(stand.at.x.abs() < Field::HALF_LENGTH);
        }
        // Centred on the halfway line, and no two men on one coordinate.
        let (first, last) = lineup.ends();
        assert!((first + last).abs() < 1e-4, "the line is off centre");
        for pair in lineup.row.windows(2) {
            let gap = pair[1].at.x - pair[0].at.x;
            assert!(gap >= Lineup::SPACING - 1e-4, "two men at {gap} m");
        }
        // …and the two sides are further apart than any two team-mates, which
        // is what makes it read as two elevens.
        let divide = lineup.row[11].at.x - lineup.row[10].at.x;
        assert!(divide > Lineup::SPACING + 1.0, "the sides are not apart");
    }

    /// ⚠ **Packing the line is the only thing that makes a printed name
    /// legible** — a team shot has to hold eleven men across the frame however
    /// far back the camera stands, so the frame is as wide as the line is and
    /// no tighter. Which means the floor under [`Lineup::SPACING`] is anatomy
    /// rather than taste, and it is worth having a test say so: one more
    /// step towards compact and the men are standing inside each other.
    #[test]
    fn the_line_is_tight_without_the_men_standing_inside_one_another() {
        use crate::players::body::Physique;
        use crate::players::kit::Complexion;
        // Measured off the real build spread rather than assumed, so this
        // follows `Complexion::build` if its range ever moves.
        let widest = (0..20_000u32).map(Complexion::build).fold(0.0f32, f32::max);
        let shoulders = Physique::SHOULDER_SPREAD * 2.0 * widest;
        assert!(
            Lineup::SPACING > shoulders + 0.2,
            "{} m apart leaves two {shoulders} m shoulder spans only {} m of daylight",
            Lineup::SPACING,
            Lineup::SPACING - shoulders
        );
        // …and the break between the sides still reads as a break.
        assert!(Lineup::DIVIDE > Lineup::SPACING * 2.0);
    }

    #[test]
    fn every_man_is_turned_at_the_main_stand() {
        for stand in &walked_out().row {
            // The gantry is behind the negative-z touchline, and so is the
            // pass that comes round to their faces.
            let facing = facing(stand);
            assert!(facing.z < -0.99, "he is not facing the stand: {facing:?}");
        }
    }

    /// The frame of the pass at which the camera is level with `man`.
    fn level_with(lineup: &Lineup, man: &Stand) -> Shot {
        let from = lineup.approach_seconds() + lineup.swing_seconds();
        (0..=800)
            .map(|step| lineup.shot_at(from + lineup.walk_seconds() * step as f32 / 800.0))
            .min_by(|left, right| {
                (left.stand.x - man.at.x)
                    .abs()
                    .total_cmp(&(right.stand.x - man.at.x).abs())
            })
            .expect("a pass has frames in it")
    }

    /// Which way `man` is turned, flat.
    fn facing(man: &Stand) -> Vec3 {
        Vec3::new(man.heading.sin(), 0.0, man.heading.cos())
    }

    /// **The pass crosses the frame left to right.** The broadcast gantry is
    /// behind the −z touchline, and a camera there looking at the line has
    /// world +x on the LEFT of its frame — so "left to right", the way the
    /// viewer watches it, is a dolly travelling toward −x: opening past the
    /// high end of the line and handing over past the low one.
    #[test]
    fn the_pass_crosses_the_frame_left_to_right() {
        let lineup = walked_out();
        let from = lineup.approach_seconds() + lineup.swing_seconds();
        let opening = lineup.shot_at(from);
        assert!(
            opening.stand.x > lineup.row[21].at.x,
            "the pass does not open beyond the high end"
        );
        let handover = lineup.shot_at(lineup.total() - 1e-3);
        assert!(
            handover.stand.x < lineup.row[0].at.x,
            "the pass does not end past the last man"
        );
        // …and it never doubles back.
        let mut previous = opening.stand.x;
        for step in 1..=400 {
            let shot = lineup.shot_at(from + lineup.walk_seconds() * step as f32 / 400.0);
            assert!(shot.stand.x <= previous + 1e-4, "the pass doubled back");
            previous = shot.stand.x;
        }
    }

    /// The ceremony opens high over the middle of the field, and the
    /// approach only ever sinks and closes on the line — arriving at eye
    /// level BEHIND them, so the corner can bring the camera round onto the
    /// faces, which are down their own eyeline for the pass.
    #[test]
    fn the_approach_comes_down_from_the_middle_of_the_field_behind_them() {
        let lineup = walked_out();
        // The opening frame: high over the centre spot, looking at the row.
        let opening = lineup.shot_at(0.0);
        assert!(
            opening.stand.x.abs() < 0.5 && opening.stand.z.abs() < 0.5,
            "the ceremony does not open over the middle of the field: {:?}",
            opening.stand
        );
        assert!((opening.stand.y - Lineup::OVERHEAD).abs() < 0.1);
        let look = (opening.aim - opening.stand).normalize();
        assert!(
            look.z < 0.0 && look.y < 0.0,
            "the opening is not looking down at the line: {look:?}"
        );
        // The flight in only ever sinks and closes on the line…
        let mut previous = opening;
        for step in 1..=200 {
            let shot = lineup.shot_at(lineup.approach_seconds() * step as f32 / 200.0);
            assert!(
                shot.stand.z <= previous.stand.z + 1e-4,
                "it backed away from the line"
            );
            assert!(
                shot.stand.y <= previous.stand.y + 1e-4,
                "it climbed on the way in"
            );
            previous = shot;
        }
        // …arrives behind the line at eye level…
        let arrived = lineup.shot_at(lineup.approach_seconds());
        assert!(
            arrived.stand.z > Lineup::ROW_Z,
            "the approach does not arrive behind them"
        );
        assert!((arrived.stand.y - Lineup::FRONT.1).abs() < 0.05);
        // …and the corner comes out on the front of the line, still level.
        let round = lineup.shot_at(lineup.approach_seconds() + lineup.swing_seconds());
        assert!(
            round.stand.z < Lineup::ROW_Z,
            "the corner does not come out in front of them"
        );
        assert!((round.stand.y - Lineup::FRONT.1).abs() < 0.05);
        // The pass itself is down their own eyeline, at eye level.
        let man = lineup.row[7];
        let level = level_with(&lineup, &man);
        let to_lens = (level.stand - man.at).with_y(0.0).normalize();
        assert!(
            to_lens.dot(facing(&man)) > 0.9,
            "the pass is not in front of him: {to_lens:?}"
        );
        assert!(
            (level.stand.y - Lineup::FRONT.1).abs() < 1e-3,
            "the pass is not at eye level"
        );
    }

    /// **The whole ceremony is one unbroken camera move.** The pass, the
    /// corner and the fly are one continuous path — the only cut the ceremony
    /// has is the one at the very end, to the kickoff — so no frame-to-frame
    /// step anywhere may exceed what the fastest beat covers in a frame.
    #[test]
    fn the_ceremony_is_one_unbroken_camera_move() {
        let lineup = walked_out();
        let total = lineup.total();
        let steps = 4_000;
        let dt = total / steps as f32;
        let bound = Lineup::FLY * dt * 1.5 + 1e-3;
        let mut previous: Option<Shot> = None;
        for step in 0..=steps {
            let at = (dt * step as f32).min(total - 1e-4);
            let shot = lineup.shot_at(at);
            if let Some(previous) = previous {
                let jump = shot.stand.distance(previous.stand);
                assert!(jump < bound, "the camera jumped {jump} m at {at} s");
                let swing = shot.aim.distance(previous.aim);
                assert!(swing < bound, "the aim jumped {swing} m at {at} s");
                assert!((shot.lens - previous.lens).abs() < 1e-4);
            }
            previous = Some(shot);
        }
    }

    /// **The corner joins the approach to the pass with no lurch at either
    /// seam.** It receives the camera at the speed the approach flies at and
    /// hands it over at the crawl the pass opens on — continuity of SPEED,
    /// where the test above only proves continuity of position.
    #[test]
    fn the_corner_sheds_the_flight_down_to_the_crawl_of_the_pass() {
        let lineup = walked_out();
        let speed = |at: f32| {
            let dt = 0.004;
            lineup.shot_at(at + dt).stand.distance(lineup.shot_at(at).stand) / dt
        };
        let approach = lineup.approach_seconds();
        let swing = lineup.swing_seconds();
        assert!(
            (speed(approach - 0.01) - Lineup::FLY).abs() < Lineup::FLY * 0.05,
            "the approach arrives at {} m/s",
            speed(approach - 0.01)
        );
        assert!(
            (speed(approach + 0.005) - Lineup::FLY).abs() < Lineup::FLY * 0.1,
            "the corner receives it at {} m/s, not the approach's speed",
            speed(approach + 0.005)
        );
        assert!(
            (speed(approach + swing - 0.01) - Lineup::crawl()).abs() < Lineup::crawl() * 0.5,
            "the corner hands over at {} m/s, not the crawl",
            speed(approach + swing - 0.01)
        );
        assert!(
            (speed(approach + swing + 0.005) - Lineup::crawl()).abs() < Lineup::crawl() * 0.5,
            "the pass opens at {} m/s",
            speed(approach + swing + 0.005)
        );
    }

    #[test]
    fn the_pass_crosses_the_line_at_the_pace_it_was_given() {
        // ⚠ **The length of the ceremony is a CONSEQUENCE of its speeds**, not
        // a constant standing beside them: a short team sheet has less ground
        // to cover and must not leave the camera crawling past an empty
        // touchline for the difference.
        let lineup = walked_out();
        let (first, last) = lineup.ends();
        let ground = (last - first) + Lineup::RUN_ON * 2.0;
        assert!((lineup.walk_seconds() - ground / Lineup::PACE).abs() < 1e-3);
        let corner = PI * Lineup::FRONT.0 * 2.0 / (Lineup::crawl() + Lineup::FLY);
        assert!(
            (lineup.total()
                - (lineup.approach_length() / Lineup::FLY + corner + ground / Lineup::PACE))
                .abs()
                < 1e-2
        );
        // …and `glide` never runs the middle of the pass more than half as
        // fast again as its mean, which is the whole reason it is not a
        // smoothstep.
        let from = lineup.approach_seconds() + lineup.swing_seconds();
        let step = lineup.walk_seconds() / 600.0;
        let mut fastest: f32 = 0.0;
        for frame in 0..600 {
            let a = lineup.shot_at(from + step * frame as f32);
            let b = lineup.shot_at(from + step * (frame + 1) as f32);
            fastest = fastest.max(a.stand.distance(b.stand) / step);
        }
        assert!(
            fastest < Lineup::PACE * 1.5,
            "the middle of the pass runs at {fastest} m/s"
        );
    }

    #[test]
    fn the_pass_leads_its_subject_by_the_angle_it_was_given() {
        // ⚠ **The lead is what decides how face-on a portrait comes out**, and
        // it has to be an ANGLE rather than a distance. Measured on a rendered
        // pass before that fix, the framed man was 12 degrees off face-on.
        let lineup = walked_out();
        let man = lineup.row[7];
        let shot = level_with(&lineup, &man);
        let axis = (shot.aim - shot.stand).with_y(0.0).normalize();
        let to_man = (man.at - shot.stand).with_y(0.0).normalize();
        let off_centre = axis.dot(to_man).clamp(-1.0, 1.0).acos();
        let wanted = Lineup::LEAD.atan();
        assert!(
            (off_centre - wanted).abs() < 0.03,
            "the pass leads by {off_centre} rad, not {wanted}"
        );
    }

    /// **Every man is shown the side of his shirt with his walk-out name on
    /// it, and the pass goes by all of them.**
    ///
    /// ⚠ **This is the whole naming contract now that no plate is drawn.** The
    /// ceremony captions nobody — it names its men off the print they are
    /// wearing — and the pass along the faces is the one beat close enough to
    /// read anything, so it has to go by every man, front on. Checked for
    /// every man rather than for the middle of the line, because it is the
    /// ends that a shot fails at.
    #[test]
    fn every_man_shows_the_camera_a_side_of_his_shirt_with_his_name_on_it() {
        let lineup = walked_out();
        let from = lineup.approach_seconds() + lineup.swing_seconds();
        for stand in &lineup.row {
            let mut passed = false;
            for step in 0..=2_000 {
                let shot =
                    lineup.shot_at(from + lineup.walk_seconds() * step as f32 / 2_000.0);
                if shot.stand.z < Lineup::ROW_Z && (shot.aim.x - stand.at.x).abs() < 0.5 {
                    passed = true;
                }
            }
            assert!(passed, "{} never had the pass go by", stand.id);
            let level = level_with(&lineup, stand);
            let to_lens = (level.stand - stand.at).with_y(0.0).normalize();
            assert!(
                to_lens.dot(facing(stand)) > 0.9,
                "{} does not face the pass as it goes by: {to_lens:?}",
                stand.id
            );
        }
    }

    /// **The walk-out print has to be IN the frame of the pass, not merely
    /// facing it.**
    ///
    /// The pass is framed on a man's eyes and the print sits below them, so a
    /// lens tight enough to be a portrait would put his name off the bottom
    /// edge and the beat would name nobody at all. This is the bound that
    /// stops [`Lineup::FRONT_LENS`] being tightened past the thing the beat is
    /// for — and the one that has to be re-read whenever
    /// [`BodyParts::NAME_FRONT_AT`] moves.
    #[test]
    fn the_pass_holds_the_print_across_the_chest_in_frame() {
        use crate::players::body::{BodyParts, Physique};
        let lineup = walked_out();
        // Where the walk-out print actually is, in the world: its own height
        // up the torso, on a man of nominal build. The BOTTOM edge of it, not
        // the middle — it is a plate now, and half its height is a real
        // centimetre and a half of something the pass has to hold.
        let print = Physique::HIP + BodyParts::NAME_FRONT_AT - BodyParts::NAME_FRONT_HEIGHT * 0.5;
        for stand in &lineup.row {
            let shot = level_with(&lineup, stand);
            // The frame's half-angle up, out of the lens the pass is held on.
            let up = TvCamera::FOV / shot.lens * 0.5;
            let to_print = Vec3::new(stand.at.x, print, stand.at.z) - shot.stand;
            let axis = (shot.aim - shot.stand).normalize();
            let flat = to_print.with_y(0.0).length().max(1e-3);
            let below = ((shot.stand.y - print) / flat).atan();
            let aimed = (axis.y / axis.with_y(0.0).length().max(1e-3)).atan().abs();
            assert!(
                below + aimed < up,
                "{}'s name is {below} below a lens aimed {aimed} down, in a {up} frame",
                stand.id
            );
        }
    }

    #[test]
    fn the_camera_never_leaves_the_ground() {
        // Three walls are the run-off; the fourth is the pitch side, where
        // there is nothing to walk into for fifty metres.
        let lineup = walked_out();
        let total = lineup.total();
        for step in 0..=2_000 {
            let shot = lineup.shot_at(total * step as f32 / 2_000.0);
            assert!(
                shot.stand.z > -(Field::HALF_WIDTH + 1e-3),
                "the lens went out past the touchline to {:?}",
                shot.stand
            );
            assert!(
                shot.stand.x.abs() < Field::HALF_LENGTH,
                "the lens went behind a goal to {:?}",
                shot.stand
            );
            assert!(shot.stand.y > 0.5, "the lens went into the turf");
        }
    }

    #[test]
    fn a_document_with_no_team_sheet_flags_still_walks_the_first_eleven_out() {
        let mut players = sheet(&(1..=18).collect::<Vec<_>>(), true);
        players.extend(sheet(&(21..=38).collect::<Vec<_>>(), false));
        for player in &mut players {
            player.starting = false;
        }
        let config = ViewerConfig::of_players(players);
        let home = Lineup::team_sheet(&config, true);
        let away = Lineup::team_sheet(&config, false);
        assert_eq!(home.len(), 11);
        assert_eq!(away.len(), 11);
        assert_eq!(home[0].id, 1);
        assert_eq!(away[10].id, 31);
    }
}
