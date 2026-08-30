//! **The teams walked out**: both elevens standing in one line inside the near
//! touchline before a ball is kicked, and the camera that goes down the line.
//!
//! Football on television does not open on a kickoff. It opens on the two
//! sides lined up in front of the main stand: a held shot of one team, a held
//! shot of the other, and then a slow pass along the faces of the whole line.
//! Then it cuts to the gantry and the match starts.
//!
//! That is exactly what this is, and it has three beats:
//!
//! 1. **The home eleven, from behind, held still** for [`Lineup::TEAM_HOLD`].
//!    The frame holds that team and no more of the line, so eleven names are
//!    on the screen at once — the eleven printed across their shoulders. It is
//!    also what is up while the squad is being dressed — see
//!    [`Act::Assembling`], the one beat with no fixed length.
//! 2. **Cut, and the away eleven the same way.**
//! 3. **Cut round to the front, and one long pass along the whole line** at
//!    eye level — slow enough to look at each man as he goes by.
//!
//! ⚠ **No plate is drawn over anybody for any of it**, and that is not an
//! omission — it is where the names come from. The ceremony names its men off
//! the shirts they are standing in: the print across the shoulders in the two
//! shots, which are taken from BEHIND the line for exactly that reason, and a
//! print across the FRONT of the shirt — the same panel, the same lettering,
//! worn for the walk-out and nothing else — in the pass along the faces. The
//! name plate that follows a footballer through the match is held back until
//! the ceremony hands the pitch over. See [`Lineup::wear_the_name`] and
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
use crate::broadcast::camera::{CameraFlight, CameraOrbit, TvCamera};
use crate::broadcast::focus::CameraSubject;
use crate::players::actors::{PlayerActor, Undressed};
use crate::players::body::FrontPrint;
use crate::recording::loader::ChunkLoader;
use crate::recording::playback::Playback;
use crate::scene::field::Field;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::f32::consts::PI;

/// Where one man stands in the line, which way he is turned, and whose he is.
#[derive(Clone, Copy)]
struct Stand {
    id: u32,
    /// Which side of the line he belongs to. Carried here rather than looked
    /// up on [`ViewerConfig`] because the two static beats are ABOUT the
    /// split: one static beat frames a side, and the shot list is written in
    /// terms of it.
    home: bool,
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
    /// dressed a few men a frame. The opening wide is held over it, which is
    /// what turns "twenty-two bodies appearing three at a time" into "the
    /// teams coming out".
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
    /// bodies do not intersect on the wider builds.
    ///
    /// ⚠ **This is the ONLY thing that decides how big a printed name comes
    /// out**, which is not where anybody looks for it. A team shot has to hold
    /// eleven men across the frame, so each of them gets an eleventh of it
    /// wherever the camera stands and whatever lens it is on — moving in makes
    /// no difference at all, because [`Self::lens_for`] simply opens up to
    /// compensate. Shortening the LINE is what makes the frame narrower, and a
    /// narrower frame is the whole of it. Down from 1.25 (2026-08-26,
    /// maintainer: *"players should stand more compactly… names on shirts
    /// aren't always visible"*), which is 1.4x the print.
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

    /// **Where a team's own shot stands: behind the line, and how high.**
    ///
    /// Behind is the pitch side — they are facing the stand, so their backs
    /// are to the middle of the ground.
    ///
    /// ⚠ **The distance does not decide how big the men come out; the WIDTH
    /// the frame has to hold does.** Eleven men have to fit across it, so each
    /// of them gets an eleventh of it wherever the camera stands, and all this
    /// number chooses is how much perspective there is — how much bigger the
    /// man in the middle is than the man on the end. See [`Self::SPACING`],
    /// which is the knob that actually moves the print.
    ///
    /// Nine and a half metres (down from twelve, 2026-08-26: *"the camera
    /// behind them should be closer"*) puts the end men a tenth further off
    /// than the middle one and turned twenty-four degrees off square. Both of
    /// those are BETTER than they were at twelve metres despite the move in,
    /// because [`Self::SPACING`] shortened the block by more than the camera
    /// gave up — and both matter, since a name printed across a shoulder is
    /// foreshortened by however far round its owner is turned.
    ///
    /// The height is above a man's head on purpose, so the line is seen
    /// slightly down onto and the far end of it does not disappear behind the
    /// near end.
    const TEAM_BACK: (f32, f32) = (9.5, 3.0);
    /// How much clear ground the frame keeps outside the end men of a team, in
    /// metres. Small: every centimetre of it is width taken away from eleven
    /// names that are only a few dozen pixels wide as it is.
    const TEAM_MARGIN: f32 = 0.7;
    /// **How long each team is held for**, in seconds.
    const TEAM_HOLD: f32 = 3.0;
    /// The frame's shape when nothing better is known — see
    /// [`Self::lens_for`], which is handed the real one.
    const FRAME: f32 = 16.0 / 9.0;
    /// …and where the pass along the front stands: closer, and at eye level.
    ///
    /// A face is a tenth of the size of a shirt, so this is as close as the
    /// line allows — it still leaves the lens two and a half metres inside the
    /// touchline, which is the bound [`Self::INSET`] was chosen for.
    const FRONT: (f32, f32) = (4.3, 1.72);
    /// What each pass aims at, in metres up the man: the print across his
    /// shoulders on the way down, his eyes on the way back.
    const SHOULDERS: f32 = 1.38;
    const EYES: f32 = 1.60;
    /// **How far ahead of itself the lens looks as it travels**, as a fraction
    /// of how far it is standing off the line.
    ///
    /// A dolly whose aim is square to its own rail is a machine going past a
    /// row of objects. Leading it a little is what makes it read as an
    /// operator walking the line — and the second pass leads the other way,
    /// because it is walking the other way.
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
    /// is the better part of a second and a half of empty touchline.
    const RUN_ON: f32 = 1.8;
    /// The lens the pass is held on, as a multiple of the wheel's own factor —
    /// under one is wider than the broadcast lens, which every shot here is:
    /// each has its subject a few metres away instead of eighty, so what it
    /// has to buy with the lens is ANGLE and not magnification. See
    /// [`Self::magnification`].
    ///
    /// The team shots are not given one at all — theirs is worked out from the
    /// ground they have to cover. See [`Self::lens_for`].
    const FRONT_LENS: f32 = 0.66;
    /// **How fast the pass crosses the line, in metres a second.**
    ///
    /// The ceremony is wall-clock — the playhead is parked for all of it, so
    /// there is no match time to measure it in and the transport speed does
    /// not touch it — and this is the one number the length of it comes out
    /// of: twenty-three metres of travel at seven tenths of a metre a second
    /// is thirty-three seconds. `glide` runs the middle of that at about 0.88.
    ///
    /// Walked back four times on the maintainer's instruction, and every one
    /// of them the same instruction: to a THIRD of the six metres a second the
    /// pass opened at (2026-08-26) — a man and a half of frame per second,
    /// fast enough to read a name off a shirt and much too fast to look at
    /// anybody — then 30% slower, 30% slower again, and 30% slower once more
    /// (2026-08-29). The lens now takes a second and a quarter to travel from
    /// one man to the next, and the whole ceremony runs a little under forty
    /// seconds.
    ///
    /// ⚠ Which is long, and deliberately survivable: every way of asking for
    /// the football ends it on the frame it is asked — see [`Self::hold`].
    const PACE: f32 = 0.686;
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
                home: true,
                at: Vec3::new(at, 0.0, Self::ROW_Z),
                heading: Self::AT_THE_STAND,
            });
            at += Self::SPACING;
        }
        at += Self::DIVIDE - Self::SPACING;
        for player in away {
            row.push(Stand {
                id: player.id,
                home: false,
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
        windows: Query<&Window, With<PrimaryWindow>>,
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
        // The frame's own shape, because a team shot's lens is worked out from
        // it — see [`Self::lens_for`]. The replay is rendered into an image the
        // shape of the window, so the window IS the frame; sixteen by nine is
        // only the fallback for a window that has not sized itself yet.
        let frame = windows
            .single()
            .ok()
            .map(|window| window.width() / window.height().max(1.0))
            .filter(|shape| shape.is_finite() && *shape > 0.1)
            .unwrap_or(Self::FRAME);
        lineup.shot = Some(lineup.shot_at(into, frame));
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
    /// Which is what the two halves of the shot list were always for — the
    /// static beats are shot from BEHIND the line, where the back print is, and
    /// the pass comes down the front at four metres, where this one is.
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

    /// …and of one side's block of it, which is what a team shot has to hold.
    fn block(&self, home: bool) -> (f32, f32) {
        let mut from = f32::MAX;
        let mut to = f32::MIN;
        for stand in self.row.iter().filter(|stand| stand.home == home) {
            from = from.min(stand.at.x);
            to = to.max(stand.at.x);
        }
        if from > to { self.ends() } else { (from, to) }
    }

    /// How long the pass along the front takes, in seconds — the ground it has
    /// to cover at [`Self::PACE`].
    ///
    /// A method rather than a constant because the ground depends on how many
    /// men there are, and a document with a short team sheet in it must not
    /// leave the camera crawling past an empty touchline for the difference.
    fn walk_seconds(&self) -> f32 {
        let (first, last) = self.ends();
        ((last - first) + Self::RUN_ON * 2.0) / Self::PACE
    }

    /// The whole ceremony, in seconds.
    fn total(&self) -> f32 {
        Self::TEAM_HOLD * 2.0 + self.walk_seconds()
    }

    /// The camera, `into` seconds in, for a frame of this shape.
    ///
    /// ⚠ **Two cuts, and both are deliberate.** A held shot of one team, a cut
    /// to the other, and a cut round to the front for the pass — which is what
    /// television does with a line-up, and what a viewer needs if the point of
    /// the first two beats is to read a team sheet off them. The one thing
    /// that has to be continuous is the pass itself.
    fn shot_at(&self, into: f32, frame: f32) -> Shot {
        if into < Self::TEAM_HOLD {
            return self.team_shot(true, frame);
        }
        if into < Self::TEAM_HOLD * 2.0 {
            return self.team_shot(false, frame);
        }
        let (first, last) = self.ends();
        let start = first - Self::RUN_ON;
        let end = last + Self::RUN_ON;
        let along = Self::glide(
            ((into - Self::TEAM_HOLD * 2.0) / self.walk_seconds().max(1e-3)).clamp(0.0, 1.0),
        );
        Self::in_front(end + (start - end) * along)
    }

    /// **One team, from behind, with the whole of it in the frame.**
    ///
    /// Square on and dead centre: this is a team photograph, and anything
    /// oblique would make the far end of the line smaller than the near end
    /// for no reason. The lens is not a constant — it is whatever holds this
    /// side's own width from [`Self::TEAM_BACK`], so eleven men fit by
    /// construction rather than because somebody tuned a number until they
    /// did. See [`Self::lens_for`].
    fn team_shot(&self, home: bool, frame: f32) -> Shot {
        let (from, to) = self.block(home);
        let middle = (from + to) * 0.5;
        Shot {
            stand: Vec3::new(
                middle,
                Self::TEAM_BACK.1,
                Self::ROW_Z + Self::TEAM_BACK.0,
            ),
            aim: Vec3::new(middle, Self::SHOULDERS, Self::ROW_Z),
            lens: Self::lens_for(
                (to - from) + Self::TEAM_MARGIN * 2.0,
                Self::TEAM_BACK.0,
                frame,
            ),
        }
    }

    /// The pass: in front of them, at eye level, working back the other way —
    /// so the lead goes the other way with it.
    fn in_front(at: f32) -> Shot {
        Shot {
            stand: Vec3::new(at, Self::FRONT.1, Self::ROW_Z - Self::FRONT.0),
            aim: Vec3::new(at - Self::FRONT.0 * Self::LEAD, Self::EYES, Self::ROW_Z),
            lens: Self::FRONT_LENS,
        }
    }

    /// **The lens that holds `width` metres across a frame of shape `frame`
    /// from `range` metres away**, as a multiple of the wheel's own factor.
    ///
    /// ⚠ **The frame's SHAPE is an input and cannot be assumed.** The replay
    /// is drawn into an image the shape of the window and stretched onto it,
    /// so the horizontal angle a given lens covers depends on how wide the
    /// window is — and the one promise a team shot makes is that eleven men
    /// fit across it. Held at a constant tuned for sixteen by nine, the same
    /// shot on a narrower window cuts a full-back off each end.
    fn lens_for(width: f32, range: f32, frame: f32) -> f32 {
        let across = (width * 0.5 / range.max(0.1)).atan();
        let up = (across.tan() / frame.max(0.1)).atan();
        TvCamera::FOV / (2.0 * up).max(1e-3)
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
        let from = Lineup::TEAM_HOLD * 2.0;
        (0..=800)
            .map(|step| {
                lineup.shot_at(
                    from + lineup.walk_seconds() * step as f32 / 800.0,
                    Lineup::FRAME,
                )
            })
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

    /// Every man of one side is inside the frame a team shot holds, and nobody
    /// from the other side is.
    ///
    /// ⚠ **This is the promise the beat exists to make** — "so that the
    /// players of one team fit in the frame and I see player names" — and it
    /// is checked at three window shapes, because the lens that keeps it is
    /// worked out from the frame's own aspect. Held at a constant tuned for
    /// sixteen by nine, a narrower window cuts a full-back off each end.
    #[test]
    fn a_team_shot_holds_its_whole_eleven_and_only_its_eleven() {
        let lineup = walked_out();
        for frame in [16.0 / 9.0, 4.0 / 3.0, 21.0 / 9.0] {
            for home in [true, false] {
                let shot = lineup.team_shot(home, frame);
                // The frame's own half-angle across, out of the lens it chose.
                let up = TvCamera::FOV / shot.lens * 0.5;
                let across = (up.tan() * frame).atan();
                for stand in &lineup.row {
                    let to = stand.at - shot.stand;
                    let off = (to.x / to.z.abs().max(1e-3)).atan().abs();
                    if stand.home == home {
                        assert!(
                            off < across,
                            "at {frame:.2} one of his own is {off} out of a {across} frame"
                        );
                    } else {
                        assert!(
                            off > across,
                            "at {frame:.2} the other team is in shot at {off} of {across}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_team_shots_are_behind_them_and_the_pass_is_in_front() {
        let lineup = walked_out();
        // Each static beat stands behind the side it is looking at.
        //
        // ⚠ Measured against the MIDDLE of that side, because the camera
        // stands square on to the middle of it: a man on the end of a
        // twelve-metre block is twenty-seven degrees round from a lens twelve
        // metres back, which is the shot being wide rather than the shot being
        // in the wrong place. What is true of every one of them is the side of
        // the line the lens is on, and that is asserted separately.
        for (home, at) in [(true, 0.1), (false, Lineup::TEAM_HOLD + 0.1)] {
            let shot = lineup.shot_at(at, Lineup::FRAME);
            let (from, to) = lineup.block(home);
            let middle = lineup
                .row
                .iter()
                .filter(|stand| stand.home == home)
                .min_by(|left, right| {
                    (left.at.x - (from + to) * 0.5)
                        .abs()
                        .total_cmp(&(right.at.x - (from + to) * 0.5).abs())
                })
                .expect("a side has men in it");
            let to_lens = (shot.stand - middle.at).with_y(0.0).normalize();
            assert!(
                to_lens.dot(facing(middle)) < -0.9,
                "the home={home} team shot is not behind them: {to_lens:?}"
            );
            for stand in lineup.row.iter().filter(|stand| stand.home == home) {
                assert!(
                    shot.stand.z > stand.at.z,
                    "the lens is on the wrong side of the line for {}",
                    stand.id
                );
            }
        }
        // ⚠ **And the pass is on the other side of the line**, which is the
        // whole reason it is a beat of its own rather than more of the same.
        let man = lineup.row[7];
        let level = level_with(&lineup, &man);
        let to_lens = (level.stand - man.at).with_y(0.0).normalize();
        assert!(
            to_lens.dot(facing(&man)) > 0.9,
            "the pass is not in front of him: {to_lens:?}"
        );
        assert!(level.stand.y < Lineup::TEAM_BACK.1, "and it is no lower");
    }

    #[test]
    fn the_pass_along_the_front_never_cuts() {
        // The two team shots are cuts on purpose; the pass is the one stretch
        // that has to be continuous, sampled finely enough to catch a step at
        // either end of its own easing.
        let lineup = walked_out();
        let from = Lineup::TEAM_HOLD * 2.0;
        let total = lineup.total();
        let steps = 4_000;
        let mut previous: Option<Shot> = None;
        for step in 0..=steps {
            let at = from + (total - from) * step as f32 / steps as f32;
            let shot = lineup.shot_at(at.min(total - 1e-4), Lineup::FRAME);
            if let Some(previous) = previous {
                let jump = shot.stand.distance(previous.stand);
                assert!(jump < 0.02, "the camera jumped {jump} m at {at} s");
                let swing = shot.aim.distance(previous.aim);
                assert!(swing < 0.02, "the aim jumped {swing} m at {at} s");
                assert!((shot.lens - previous.lens).abs() < 1e-4);
            }
            previous = Some(shot);
        }
    }

    #[test]
    fn the_pass_crosses_the_line_at_the_pace_it_was_given() {
        // ⚠ **The length of the ceremony is a CONSEQUENCE of the pace**, not a
        // constant standing beside it: a short team sheet has less ground to
        // cover and must not leave the camera crawling past an empty touchline
        // for the difference.
        let lineup = walked_out();
        let (first, last) = lineup.ends();
        let ground = (last - first) + Lineup::RUN_ON * 2.0;
        assert!((lineup.walk_seconds() - ground / Lineup::PACE).abs() < 1e-3);
        assert!((lineup.total() - (Lineup::TEAM_HOLD * 2.0 + ground / Lineup::PACE)).abs() < 1e-3);
        // …and `glide` never runs the middle of it more than half as fast
        // again as that, which is the whole reason it is not a smoothstep.
        let from = Lineup::TEAM_HOLD * 2.0;
        let step = lineup.walk_seconds() / 600.0;
        let mut fastest: f32 = 0.0;
        for frame in 0..600 {
            let a = lineup.shot_at(from + step * frame as f32, Lineup::FRAME);
            let b = lineup.shot_at(from + step * (frame + 1) as f32, Lineup::FRAME);
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

    /// **Every man is shown a side of his shirt that has his name on it, and
    /// the pass goes by all of them.**
    ///
    /// ⚠ **This is the whole naming contract now that no plate is drawn.** The
    /// ceremony captions nobody — it names its men off the print they are
    /// wearing — so the promise the shot list has to keep is that each beat is
    /// on a side of the shirt that carries the name: the back in the two static
    /// shots, the front in the pass. Checked for every man rather than for the
    /// middle of the line, because it is the ends that a shot fails at.
    #[test]
    fn every_man_shows_the_camera_a_side_of_his_shirt_with_his_name_on_it() {
        let lineup = walked_out();
        let total = lineup.total();
        for stand in &lineup.row {
            // His own side's held shot, from behind — the back print.
            let at = if stand.home {
                Lineup::TEAM_HOLD * 0.5
            } else {
                Lineup::TEAM_HOLD * 1.5
            };
            let shot = lineup.shot_at(at, Lineup::FRAME);
            let to_lens = (shot.stand - stand.at).with_y(0.0).normalize();
            assert!(
                to_lens.dot(facing(stand)) < -0.85,
                "{} is not showing his back to his own team shot: {to_lens:?}",
                stand.id
            );

            // …and the pass, which comes down the front — the walk-out print.
            let mut passed = false;
            for step in 0..=2_000 {
                let shot = lineup.shot_at(total * step as f32 / 2_000.0, Lineup::FRAME);
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
            let shot = lineup.shot_at(total * step as f32 / 2_000.0, Lineup::FRAME);
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
