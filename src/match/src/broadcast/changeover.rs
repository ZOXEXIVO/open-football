//! **The shot a substitution gets**: every man coming on, one at a time —
//! his face, his name, and then his run onto the field.
//!
//! The broadcast rig is eighteen metres up and eighty back, looking down at
//! the whole ground. That is the right camera for football and the wrong one
//! for a substitution: the two men crossing are twelve pixels tall, the
//! exchange happens at the very bottom edge of the frame, and the whole point
//! of playing it out is lost.
//!
//! So each man coming on gets a beat of his own, and it has three parts.
//!
//! 1. **His face**, from a couple of metres in front of him on the grass. He
//!    is standing at the fourth official's shoulder waiting to go on, square
//!    to the line, looking out at the football.
//! 2. **Round to his back**, where the name is printed and nowhere else. The
//!    camera does not cut — it swings round him, rising and drawing back as it
//!    goes, and comes to rest outside the touchline with the whole pitch in
//!    front of him.
//! 3. **And he goes.** The rig stays exactly where it stopped and he runs away
//!    from it onto the field. Then it cuts to the next man and starts again.
//!
//! Nobody moves during the first two parts: the engine holds the window still
//! for exactly as long as they take (see `SubstitutionBreak::PORTRAIT_MS`,
//! which is [`ChangeoverShot::PORTRAIT_MS`] on the other side of the
//! recording), because a man cannot be shown standing at the gate and be
//! running onto the pitch at the same time. And it holds the men after him for
//! his run as well — [`ChangeoverShot::RUN_MS`] — so a triple change is three
//! complete arrivals rather than one portrait each and then a scramble.
//!
//! **The men going off are not watched.** One of them leaves on the same tick
//! as each substitute and they cross at the gate, which is a picture the shot
//! catches in passing; none of them is ever the subject of it.
//!
//! **And then the touchline**, once the last man has gone: the rig stands
//! outside the line level with the halfway line and points at the centre spot,
//! holding the whole ground until play resumes.
//!
//! It reads the substitutions straight off [`ViewerConfig`], including how
//! long each one stopped the match for, so the shot lasts exactly as long as
//! the change did and not a constant somebody guessed at.

use crate::app::config::ViewerConfig;
use crate::broadcast::camera::{CameraFlight, CameraOrbit};
use crate::broadcast::focus::CameraSubject;
use crate::players::actors::PlayerActor;
use crate::recording::playback::Playback;
use crate::scene::field::Field;
use crate::scene::pitch::Pitch;
use bevy::prelude::*;
use std::f32::consts::PI;

/// One change, as the camera needs it: when, and which men to work through
/// one at a time.
struct Change {
    from: f64,
    to: f64,
    /// The men coming on, in the order the shot works through them — one
    /// [`ChangeoverShot::BEAT_MS`] each.
    coming_on: Vec<u32>,
    /// And the men they are replacing. **Nothing is ever pointed at them**;
    /// they are here so the sight-line test can leave them out — see
    /// [`ChangeoverShot::clear`]. Empty on a document written before it
    /// carried who came off, which costs the shot nothing.
    coming_off: Vec<u32>,
}

/// Where the camera is and how it is lensed for one beat, worked out against
/// the man's own body rather than against the pitch.
struct Portrait {
    stand: Vec3,
    aim: Vec3,
    lens: f32,
}

/// The camera a substitution is watched from.
#[derive(Resource, Default)]
pub struct ChangeoverShot {
    changes: Vec<Change>,
    /// How far the shot has left the gantry, 0..1.
    ///
    /// **It cuts IN and ramps OUT**, which is not the asymmetry it looks like.
    /// The first thing a change shows is one second on one man: ramp into that
    /// over [`Self::CLOSE_TIME`] and seven tenths of the beat is a camera
    /// still on its way. The walk back at the end has all the time it needs,
    /// so it takes it — the same ramp [`CameraSubject`]'s grip uses, for the
    /// same reason.
    grip: f32,
    /// The beat this frame, if the shot is on one. `None` once it has worked
    /// through them and moved to the touchline.
    ///
    /// Carried on the resource rather than worked out in [`Self::blend`]
    /// because it is measured off the man's own transform, which only the
    /// system that queries the actors can see.
    portrait: Option<Portrait>,
    /// **Where the rig planted itself behind the man it is watching**, and
    /// which of the change's men that was.
    ///
    /// ⚠ Everything else here is re-derived from the subject's own body every
    /// frame, and for the run it cannot be: he is moving. A camera worked out
    /// from a running man travels with him at eight metres a second, which is
    /// a drone shot of the back of his head and not a touchline one. So the
    /// position is taken once, at the end of his close-up, and held for as
    /// long as he is the subject.
    plant: Option<(usize, Vec3, f32)>,
}

impl ChangeoverShot {
    /// Seconds to walk back out of the shot at the end of a change.
    const CLOSE_TIME: f32 = 0.7;
    /// How high the touchline rig stands, in metres. Just clear of the height
    /// the hoardings demand at [`Self::OUT`] (2.6 m), and no higher: the aim is
    /// the centre of the field forty-two metres away, so this is under three
    /// degrees off level — a camera looking OUT at the pitch rather than down
    /// on it.
    const HEIGHT: f32 = 3.2;
    /// What it aims at: the centre spot, at chest height rather than on the
    /// grass, which is the last degree of tilt out of the shot. Also what the
    /// first half of a close-up aims at — the print on a shirt is at chest
    /// height on the other side of him.
    const CHEST: f32 = 1.15;
    /// **How far outside the touchline the rig stands, in metres.**
    ///
    /// Eight, and it is as close as the ground allows.
    ///
    /// **Close is what the shot is FOR** — close enough to read the name
    /// across the back of the shirt of the man about to come on, which is a
    /// strip a few centimetres tall on a body seven metres away. Everything
    /// below is the argument for why it cannot be closer still.
    ///
    /// **The advertising hoardings are the floor.** They are 3.4 m out and a
    /// metre high and the substitute is 0.75 m out, so anything behind them
    /// is behind a wall 2.65 m in front of its subject: seeing his feet over
    /// it takes a height of 0.95 x distance / 2.65, which at 7.25 m is
    /// 2.6 m. Rendered at 22 m out and 3 m up, the boards took the bottom
    /// forty per cent of the frame and cut both men off at the knee.
    ///
    /// **And the frame is the ceiling**, because height and depression grow
    /// together and the foreground man has to fit under an aim that is on the
    /// centre spot. At 12 m his feet sat 19 degrees below a centre spot 5
    /// below level against a 17-degree half-frame and he was cut off at the
    /// shins; at 8 m the separation is 21, which is what [`Self::LENS`] is
    /// opened to hold.
    const OUT: f32 = 8.0;
    /// The lens at the touchline. See [`Self::magnification`] — it OPENS,
    /// because the shot has to hold a man seven metres away and the centre of
    /// the field forty-two metres away in the same frame.
    const LENS: f32 = 0.36;

    /// **How long each man coming on stands still in front of the camera**,
    /// in ms of match clock: a second and a half on his face, the swing round
    /// him, and a second on his back — 3.4 s a man.
    ///
    /// ⚠ **`SubstitutionBreak::PORTRAIT_MS` is this same figure and the two
    /// have to agree.** That is what stands the whole window still while this
    /// runs; this crate cannot depend on that one, so if either moves the other
    /// moves with it. Hold for less than the shot and he sets off in the middle
    /// of his own close-up; hold for more and the picture is on a man who has
    /// already arrived while twenty-one others stand about.
    const PORTRAIT_MS: f64 = Self::FACE_MS + Self::TURN_MS + Self::BACK_MS;
    /// The share of it spent looking at his face, which is where the beat
    /// opens. He is square to the line watching the football, so this is a
    /// three-quarter rather than a stare.
    const FACE_MS: f64 = 1_400.0;
    /// …and the swing from there round to his back.
    ///
    /// ⚠ **It is a longer move than it looks and it is sized by how fast the
    /// camera may travel, not by how long a viewer needs.** The arc is half a
    /// circle round him AND a pull from under three metres out to eight, which
    /// is seventeen metres of ground; eased, the middle of it runs at half as
    /// fast again as the mean. At 900 ms — what the old swing, on a shorter
    /// arc, was given — the lens covers half a metre between one frame and the
    /// next, which is a whip rather than a move.
    const TURN_MS: f64 = 1_200.0;
    /// Then the rest of [`Self::PORTRAIT_MS`] is spent standing behind him.
    ///
    /// **This is the angle the whole shot exists for** — his name is printed
    /// across the back of his shirt and nowhere else — and it is short because
    /// it does not end here: [`Self::RUN_MS`] is spent on the same bearing
    /// from the same spot, so the print is in frame for well over three
    /// seconds. What this buys is a beat of him standing before he goes.
    const BACK_MS: f64 = 800.0;
    /// **And then he runs on, with the rig standing where it stopped.**
    ///
    /// Two seconds at the engine's `ON` speed is sixteen metres, which is far
    /// enough onto the field to read as a man arriving. He keeps going
    /// afterwards — the camera does not, it cuts to the next man, or away.
    ///
    /// ⚠ **`SubstitutionBreak::RUN_MS` is this same figure**, on the same
    /// terms as [`Self::PORTRAIT_MS`]: it is what holds the man after him
    /// still while this runs — and, since 2026-08-26, what ENDS the window.
    /// The engine no longer waits for either walker to arrive, so the last
    /// man's run is the last thing in the change and the shot stops with it.
    const RUN_MS: f64 = 2_000.0;
    /// One man's whole turn in front of the camera. The next man's face is at
    /// the end of it.
    const BEAT_MS: f64 = Self::PORTRAIT_MS + Self::RUN_MS;
    /// **Where the beat opens**, in metres in front of him and how high: close,
    /// and level with his eyes.
    ///
    /// Under three metres, because a face is a tenth of the size of a shirt
    /// and half a beat is spent on it.
    const FACE_OFF: (f32, f32) = (2.8, 1.65);
    /// And where the swing ends up: **behind him, outside the line, and up.**
    ///
    /// Eight metres and 3.2 m are the touchline rig's own numbers — see
    /// [`Self::OUT`] and [`Self::HEIGHT`], where they are argued from the
    /// hoardings and the frame — and this is where the shot comes to rest, so
    /// they had better be. A man standing at the gate is three quarters of a
    /// metre outside the line, so [`Self::inside_the_ground`] stops the lens
    /// a little short of eight and the framing holds anyway: what the shot
    /// gives up in distance it takes back in lens.
    ///
    /// At that distance he stands a little under half the frame tall with his
    /// feet well inside the bottom edge and the pitch he is about to run onto
    /// laid out above him — which is as much shirt as a shot he then runs away
    /// down can afford to spend.
    const BACK_OFF: (f32, f32) = (Self::OUT, Self::HEIGHT);
    /// What the face half aims at — his eyeline rather than his chest, which
    /// is the difference between a portrait and a shot of a collar.
    const FACE_AT: f32 = 1.55;
    /// The two lenses. Both are held on a man a few metres away rather than
    /// eighty, so both are wider than the broadcast lens and neither is a
    /// zoom: the closeness is bought by DISTANCE.
    ///
    /// The face is the tighter of the two — at 2.8 m it frames from his chest
    /// to just over his head — and it OPENS across the swing as the camera
    /// draws back, so the man shrinks into his own picture rather than
    /// jumping when it lands.
    const BACK_LENS: f32 = 0.60;
    const FACE_LENS: f32 = 0.72;
    /// How near the sight line another man has to be before he counts as
    /// standing in the way, in metres. A body is half a metre across and the
    /// close-up frames about three, so anybody inside this is not merely in
    /// shot — he is across it. See [`Self::clear`].
    const LANE: f32 = 1.0;
    /// And how far in front of him the lens then stops. Far enough that he is
    /// behind the camera rather than filling the bottom of the frame.
    const GAP: f32 = 0.9;
    /// The closest the shot will ever be driven, in metres, and the most of
    /// its distance it will give up to get there.
    ///
    /// Both floors on the same thing: a shot that ducks in past everybody who
    /// is standing about ends up with the lens on somebody's shoulder. Past
    /// this the man in the way is left in shot instead — a team-mate in the
    /// background is a picture, a team-mate at arm's length is not.
    const CRAMPED: f32 = 1.4;
    /// **The widest the lens may open**, as a multiple of the wheel's own
    /// factor — a floor on the magnification, which is a ceiling on the angle.
    ///
    /// The lens opens by whatever the shot gives up in distance so the man
    /// stays the same size in frame, and that is right down to about three
    /// metres. Below it the arithmetic asks for a fisheye: driven in to
    /// [`Self::CRAMPED`] the back beat wants 0.14, which is 108 degrees and a
    /// man with a head the size of his shoulders.
    ///
    /// So the widening stops here and the framing gives instead — the closer
    /// the shot is forced, the tighter it crops. That is the right way round:
    /// what the first beat is for is the print across his shoulders, and a
    /// shot cropped at his waist still has all of it.
    const WIDEST: f32 = 0.30;
    /// **The box the lens may stand in**, in metres — [`Self::ALONG`] the
    /// pitch and [`Self::ACROSS`] it.
    ///
    /// Three of its four walls are the run-off, pulled in by this so the
    /// advertising hoardings standing at the end of it are always behind the
    /// camera rather than across the shot.
    const PERIMETER: f32 = 1.0;
    /// End to end: the run-off behind each goal. Nothing this shot does comes
    /// within forty metres of either — the men it looks at are all standing
    /// within a few metres of the halfway line — so this is a guard rather
    /// than a working bound.
    const ALONG: (f32, f32) = (
        -(Field::HALF_LENGTH + Pitch::END_MARGIN - Self::PERIMETER),
        Field::HALF_LENGTH + Pitch::END_MARGIN - Self::PERIMETER,
    );
    /// And across it, which is **not symmetric.**
    ///
    /// The far side is the run-off like everything else. The near side is the
    /// bench touchline, and the whole shot lives out there: the swing ends
    /// [`Self::OUT`] metres beyond the line, past the boards and two metres
    /// into the near bank of seating.
    ///
    /// ⚠ That works because [`Bank::cull`](crate::scene::pitch::Bank) hides
    /// whichever stand the lens is inside — and it is also why the wall is
    /// here at all. A rig that wanders further does not merely stand behind a
    /// wall; it makes a whole stand blink out of the picture as it crosses the
    /// front row.
    const ACROSS: (f32, f32) = (
        -(Field::HALF_WIDTH + Self::OUT),
        Field::HALF_WIDTH + Pitch::SIDE_MARGIN - Self::PERIMETER,
    );

    /// How long a change lasts when the recording does not say — a document
    /// written before the engine played substitutions out, or one made on the
    /// instant path. One man's beat, which is what a window with one change in
    /// it now costs exactly. See `SubstitutionInfo::break_ms`.
    const ASSUMED_MS: f64 = Self::BEAT_MS;
    /// How long the shot lingers past the end of the change, in ms, before it
    /// walks back to the broadcast rig.
    ///
    /// **Nothing.** It used to hold the touchline wide for a beat, to see the
    /// substitute take up his position rather than cutting on the referee's
    /// arm — but the engine no longer waits for him to get there (see
    /// `SubstitutionBreak::beats_ms`), so there is no arrival left to wait
    /// for: the change ends two seconds into his run and the shot ends with
    /// it. [`Self::CLOSE_TIME`] still ramps the rig home, so it is a move
    /// rather than a cut.
    const LINGER_MS: f64 = 0.0;

    /// Group the config's substitutions into one shot per stoppage.
    ///
    /// A double change is one moment and gets one shot: the marks share a
    /// timestamp, so anything within a second of another belongs with it — and
    /// the camera then works through their men one at a time, which is why the
    /// grouping has to match the window the engine opened.
    pub fn arm(mut commands: Commands, config: Res<ViewerConfig>) {
        let mut changes: Vec<Change> = Vec::new();
        for change in &config.substitutions {
            let hold = if change.break_ms > 0 {
                change.break_ms as f64
            } else {
                Self::ASSUMED_MS
            };
            Self::stage(
                &mut changes,
                change.time,
                hold,
                change.player_in_id,
                change.player_out_id,
            );
        }
        commands.insert_resource(ChangeoverShot {
            changes,
            ..default()
        });
    }

    /// Add one change to the list, folding it into the stoppage it shares a
    /// whistle with if there is one.
    fn stage(changes: &mut Vec<Change>, from: f64, hold: f64, on: u32, off: u32) {
        let to = from + hold + Self::LINGER_MS;
        let open = changes
            .iter_mut()
            .find(|open| (open.from - from).abs() < 1_000.0);
        let change = match open {
            Some(open) => {
                open.to = open.to.max(to);
                open
            }
            None => {
                changes.push(Change {
                    from,
                    to,
                    coming_on: Vec::with_capacity(2),
                    coming_off: Vec::with_capacity(2),
                });
                changes.last_mut().expect("just pushed")
            }
        };
        change.coming_on.push(on);
        // A document written before the recording carried who came off says
        // zero, and zero is not a player. He is left out rather than pushed —
        // the list is a set of ids to keep out of the sight-line test and a
        // phantom in it would be a body standing at the origin that the swing
        // politely declines to duck past.
        if off > 0 {
            change.coming_off.push(off);
        }
    }

    /// Whether the shot should be on this frame, how far it has closed, and
    /// which man it is looking at.
    ///
    /// Runs after the bodies have been placed, like [`CameraSubject::settle`],
    /// because it measures the beats off the men themselves: where a man is
    /// standing and **which way he is pointing** are properties of his
    /// transform, and a shot that comes round to somebody's back has to be
    /// worked out from the back he actually has rather than from where the
    /// ball is.
    pub fn settle(
        time: Res<Time>,
        playback: Res<Playback>,
        orbit: Res<CameraOrbit>,
        flight: Res<CameraFlight>,
        subject: Res<CameraSubject>,
        actors: Query<(&PlayerActor, &Transform, &Visibility)>,
        mut shot: ResMut<ChangeoverShot>,
    ) {
        // The viewer's own camera outranks this one. A man being followed by
        // hand, a rig in flight, or a gantry the viewer has walked round the
        // ground are all somebody asking to look at something else.
        let hands_off = flight.airborne() || subject.locked() || orbit.bearing.abs() > 1e-3;
        let now = playback.time_ms;
        let wanted = if hands_off {
            None
        } else {
            shot.changes
                .iter()
                .find(|change| now >= change.from && now <= change.to)
        };

        // Nobody drawn means the chunk holding him has not landed yet, or the
        // playhead was scrubbed past him. Either way there is nothing to
        // watch, and the shot stays where it is.
        let anybody = wanted.is_some_and(|change| {
            actors.iter().any(|(actor, _, visibility)| {
                *visibility != Visibility::Hidden && change.coming_on.contains(&actor.id)
            })
        });
        // Which man the shot is on, and how far through his beat we are. Past
        // the last of them it is at the touchline, holding the ground until
        // play resumes.
        let beat = wanted.and_then(|change| {
            let elapsed = now - change.from;
            let index = (elapsed / Self::BEAT_MS).floor().max(0.0) as usize;
            let man = *change.coming_on.get(index)?;
            let (_, transform, _) = actors.iter().find(|(actor, _, visibility)| {
                actor.id == man && **visibility != Visibility::Hidden
            })?;
            let into = (elapsed - index as f64 * Self::BEAT_MS) as f32;

            // **He is running — the rig stays where the swing left it.**
            if into >= Self::PORTRAIT_MS as f32 {
                let planted = shot
                    .plant
                    .filter(|(on, _, _)| *on == index)
                    .map(|(_, stand, lens)| (stand, lens));
                return Some(match planted {
                    Some(planted) => (index, Self::watching(transform.translation, planted)),
                    // Nothing to stand on: the playhead was scrubbed into the
                    // middle of his run and the beat that would have planted
                    // the camera never played. Take the shot off his back
                    // where he is, which follows him rather than watching him
                    // go — a scrub cuts anyway.
                    None => (index, Self::close_up(transform, Self::PORTRAIT_MS as f32, &[])),
                });
            }

            // Everybody else who is drawn, so the swing can duck in past
            // whoever is standing in its way — see `clear`. A stack array
            // rather than a collection: this runs every frame a close-up is on
            // and there are never more than a couple of dozen of them.
            //
            // ⚠ **The men in the change are left out of it.** They are the
            // only bodies on the ground that move while the window is open —
            // one substitute is already running on and one man is walking off
            // — and a sight line that gave way to them would yank the lens
            // about for the length of somebody else's close-up. They are also
            // standing in a row 1.75 m apart at the gate, which the swing is
            // meant to sweep past rather than dive in front of.
            let mut others = [Vec3::ZERO; 32];
            let mut count = 0;
            for (actor, at, visibility) in &actors {
                let in_the_change =
                    change.coming_on.contains(&actor.id) || change.coming_off.contains(&actor.id);
                if !in_the_change && *visibility != Visibility::Hidden && count < others.len() {
                    others[count] = at.translation;
                    count += 1;
                }
            }
            Some((
                index,
                Self::close_up(transform, into, &others[..count]),
            ))
        });

        // **The rig plants at the end of the swing and holds.** Taken here,
        // on the last frame of the close-up, because that is the only frame
        // that has both the man standing still and the camera at rest behind
        // him — see [`Self::plant`].
        let plant = match &beat {
            Some((index, portrait)) => Some((*index, portrait.stand, portrait.lens)),
            None => None,
        };
        let portrait = beat.map(|(_, portrait)| portrait);

        // **A close-up cuts.** It is on screen for a second and a ramp would
        // spend most of that arriving; the walk home at the end still ramps.
        let grip = if portrait.is_some() {
            1.0
        } else {
            Self::stepped(shot.grip, f32::from(anybody), Self::CLOSE_TIME, &time)
        };

        // Same rule the name plates and the contact shadows keep: write
        // nothing that has not moved, so the resource is not dirtied through
        // the eighty minutes with no change in them.
        if shot.grip != grip {
            shot.grip = grip;
        }
        if shot.portrait.is_some() || portrait.is_some() {
            shot.portrait = portrait;
        }
        if shot.plant.is_some() || plant.is_some() {
            shot.plant = plant;
        }
    }

    /// One frame of a ramp from `from` towards `to`, `seconds` long end to end.
    fn stepped(from: f32, to: f32, seconds: f32, time: &Time) -> f32 {
        if from == to {
            return to;
        }
        let step = time.delta_secs() / seconds;
        if from < to {
            (from + step).min(to)
        } else {
            (from - step).max(to)
        }
    }

    /// The shot on one man, `into` ms into his own beat.
    ///
    /// Everything here is in HIS frame rather than the pitch's: the camera
    /// stands in front of his face whichever way he happens to be facing, and
    /// swings round to his shoulders from there. `others` is everybody else
    /// who is drawn, because the one thing this shot cannot do is put a body
    /// between the lens and its subject — see [`Self::clear`].
    ///
    /// Past [`Self::PORTRAIT_MS`] he is running and this is not called: the
    /// rig is planted where the swing left it — see [`Self::watching`].
    fn close_up(transform: &Transform, into: f32, others: &[Vec3]) -> Portrait {
        let boots = transform.translation;
        let turn =
            Self::ease(((into - Self::FACE_MS as f32) / Self::TURN_MS as f32).clamp(0.0, 1.0));

        // ⚠ **The bearing is worked out for THIS frame, and so is everything
        // hung off it.**
        //
        // The obvious build takes the two end positions, clears each of them
        // once, and swings between them — and it puts the camera through
        // people. The ends are dead behind him and dead in front; the metre of
        // ground the lens actually travels over is neither, and it is where
        // the man standing two metres to his left is. Measured on the first
        // change of a real match: a team-mate 2.15 m away, the swing passing
        // within a foot of him, his body across the whole frame and then
        // through the near plane.
        //
        // So the arc is a bearing that is re-cleared every frame. What comes
        // out is not a circle any more — the lens ducks in where somebody is
        // standing and comes back out after him — which is exactly what
        // somebody carrying a camera round a man would do.
        //
        // It starts on his FACING and turns half a circle onto his back: the
        // beat opens looking him in the face and ends on the name across his
        // shoulders, because the second of those is the angle he then runs
        // away down.
        let bearing = Quat::from_rotation_y(turn * PI) * Self::heading(transform);
        let wanted = Self::FACE_OFF.0 + (Self::BACK_OFF.0 - Self::FACE_OFF.0) * turn;
        let reach = Self::clear(boots, bearing, wanted, others);

        Portrait {
            stand: Vec3::new(
                boots.x + bearing.x * reach,
                boots.y + Self::FACE_OFF.1 + (Self::BACK_OFF.1 - Self::FACE_OFF.1) * turn,
                boots.z + bearing.z * reach,
            ),
            aim: boots
                .with_y(Self::FACE_AT)
                .lerp(boots.with_y(Self::CHEST), turn),
            // **The lens opens with every metre the shot gives up**, so a man
            // it had to duck in past — or one the ground made it stop short of
            // — does not change how big the subject comes out. Until
            // [`Self::WIDEST`], where opening any further would be a fisheye
            // and the shot crops instead.
            lens: ((Self::FACE_LENS + (Self::BACK_LENS - Self::FACE_LENS) * turn) * reach / wanted)
                .max(Self::WIDEST),
        }
    }

    /// The shot while he runs on: **the rig does not move.**
    ///
    /// `planted` is where the swing left it and how it was lensed, taken on
    /// the last frame of his close-up and held. All that changes is the aim,
    /// which stays on him, so he runs away down the middle of the frame with
    /// the pitch opening out around him and the name still legible for the
    /// first stride or two of it.
    ///
    /// Nothing is cleared here. Twenty-one men were standing still when the
    /// camera planted and the two who are not are the pair this beat is
    /// about — a lens that ducked out of the way of a man crossing the shot at
    /// eight metres a second would be the only thing in it that lurched.
    fn watching(boots: Vec3, planted: (Vec3, f32)) -> Portrait {
        Portrait {
            stand: planted.0,
            aim: boots.with_y(Self::CHEST),
            lens: planted.1,
        }
    }

    /// Which way he is facing, flat.
    ///
    /// The model is built looking down +Z — see `Actors`, which rotates it
    /// about Y by `atan2(x, z)` to carry +Z onto the way he is facing. Reading
    /// it off the transform is the only way to be sure: `Actors::facing` turns
    /// a stationary man toward the ball, and toward nothing at all when the
    /// ball is off the pitch, which is exactly the case at the stoppage every
    /// substitution is made at.
    fn heading(transform: &Transform) -> Vec3 {
        (transform.rotation * Vec3::Z)
            .with_y(0.0)
            .normalize_or_zero()
    }

    /// **How far the lens may stand off him along `bearing`**, in metres:
    /// as far as it wants, unless the ground runs out or somebody is standing
    /// in the gap.
    ///
    /// ⚠ **A close-up is the only shot in the replay with something between
    /// the camera and its subject as a matter of course.** Twenty-one other
    /// men are stood still all over the pitch, none of them will move until
    /// the change is over, and one of them is sooner or later a couple of
    /// metres from the man being looked at. Rendered, the second close-up of a
    /// double change was the back of a team-mate filling the frame.
    ///
    /// ⚠ **And the ground is not infinite.** Six metres behind a full-back
    /// standing on his own touchline is three metres BEHIND the advertising
    /// hoardings — a wall across the shot — and five is inside a bank of
    /// seating, which `Bank::cull` then takes out of the picture altogether:
    /// a whole stand blinking off and on as the lens swings past the line.
    ///
    /// So the shot gives up distance rather than the subject, all the way in
    /// to [`Self::CRAMPED`] — it would rather be a foot behind his shoulder
    /// than have somebody else's back across the frame. What gives with it is
    /// the FRAMING, not the sight line: see [`Self::WIDEST`], which is where
    /// the lens stops opening and the shot starts cropping instead.
    fn clear(boots: Vec3, bearing: Vec3, wanted: f32, others: &[Vec3]) -> f32 {
        let mut reach = wanted.min(Self::inside_the_ground(boots, bearing));
        for other in others {
            let to = (*other - boots).with_y(0.0);
            let along = to.dot(bearing);
            if along <= 0.0 || (to - bearing * along).length() >= Self::LANE {
                continue;
            }
            reach = reach.min(along - Self::GAP);
        }
        reach.clamp(Self::CRAMPED, wanted)
    }

    /// How far the lens can go along `bearing` before it is out of the ground,
    /// in metres. See [`Self::ALONG`] and [`Self::ACROSS`] for the box.
    fn inside_the_ground(boots: Vec3, bearing: Vec3) -> f32 {
        let wall = |at: f32, step: f32, (low, high): (f32, f32)| {
            if step.abs() < 1e-4 {
                f32::MAX
            } else if step > 0.0 {
                (high - at) / step
            } else {
                (low - at) / step
            }
        };
        wall(boots.x, bearing.x, Self::ALONG).min(wall(boots.z, bearing.z, Self::ACROSS))
    }

    /// Smoothstep. Every beat of the move starts and stops on it, which is
    /// what keeps a camera that is only ever given a linear ramp reading as
    /// operated rather than as animated.
    fn ease(t: f32) -> f32 {
        t * t * (3.0 - 2.0 * t)
    }

    /// How the lens is held while the shot is on, as a multiple of the
    /// wheel's own factor.
    ///
    /// **Below one — it opens up.** Every one of these shots has its subject a
    /// few metres in front of the lens instead of eighty-two, which is all the
    /// magnification anybody needs; what they have to buy with the lens is the
    /// ANGLE. At the touchline that is the angle between the man in the
    /// foreground (twenty-four degrees down) and the centre of the field
    /// (three): twenty-one degrees of separation needs a half-frame at least
    /// that tall, and the broadcast lens is 7.6 degrees, so held there the
    /// substitute is below the bottom edge of the shot he is the subject of.
    ///
    /// Net of the two he stands about a third of the frame tall — which is
    /// what it takes to read a name printed across a shirt. A close-up buys
    /// the rest of it by standing six metres away instead of eighty.
    pub fn magnification(&self) -> f32 {
        let lens = match &self.portrait {
            Some(portrait) => portrait.lens,
            None => Self::LENS,
        };
        1.0 + (lens - 1.0) * self.grip
    }

    /// The broadcast rig's position and aim, blended towards whichever shot
    /// the change is on by [`Self::grip`].
    ///
    /// Both ends are computed every frame and mixed, rather than switching
    /// between them, so the walk home at the end is one continuous path and
    /// cannot cut.
    pub fn blend(&self, gantry: Vec3, looking_at: Vec3) -> (Vec3, Vec3) {
        if self.grip <= 0.0 {
            return (gantry, looking_at);
        }
        let (stand, aim) = match &self.portrait {
            Some(portrait) => (portrait.stand, portrait.aim),
            // **And then the wide, once the last man has gone.**
            //
            // The same place every beat ended, put back on the halfway line
            // and pointed at the middle of the ground: the gate they all came
            // through in the foreground, whoever is still walking off crossing
            // it, and the whole pitch in front. It holds that until play
            // resumes and then walks home to the gantry.
            None => (
                Vec3::new(0.0, Self::HEIGHT, -(Field::HALF_WIDTH + Self::OUT)),
                Vec3::new(0.0, Self::CHEST, 0.0),
            ),
        };
        (
            gantry.lerp(stand, self.grip),
            looking_at.lerp(aim, self.grip),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    /// A man standing at `at`, facing `bearing` radians round from +Z.
    fn standing(at: Vec3, bearing: f32) -> Transform {
        Transform::from_translation(at).with_rotation(Quat::from_rotation_y(bearing))
    }

    /// Which way a man on `bearing` is facing, worked out from the angle the
    /// test built him on rather than from [`ChangeoverShot::heading`] — which
    /// is the thing under test, and a shot checked against its own arithmetic
    /// is checked against nothing.
    fn facing(bearing: f32) -> Vec3 {
        Vec3::new(bearing.sin(), 0.0, bearing.cos())
    }

    /// How big the subject comes out, near enough: the lens over the ground
    /// the lens is standing off him.
    ///
    /// **Horizontal**, because the height of the rig is a property of the beat
    /// and not of how far the shot was driven in — so it is exactly this that
    /// [`ChangeoverShot::close_up`] holds constant when the ground or a body in
    /// the way costs it some of its distance.
    fn in_frame(shot: &Portrait, man: Vec3) -> f32 {
        shot.lens / (shot.stand - man).with_y(0.0).length()
    }

    /// Is `other` standing on the line between the lens and the man it is
    /// pointed at — near enough to it, and far enough from both ends to be a
    /// body across the frame rather than part of the picture?
    fn across_the_shot(lens: Vec3, man: Vec3, other: Vec3) -> bool {
        let to_man = (man - lens).with_y(0.0);
        let range = to_man.length();
        let sight = to_man / range;
        let to_other = (other - lens).with_y(0.0);
        let along = to_other.dot(sight);
        along > 0.3 && along < range - 0.3 && (to_other - sight * along).length() < 0.55
    }

    #[test]
    fn the_beat_opens_on_his_face() {
        for bearing in [0.0, 1.1, PI, -2.4] {
            let man = standing(Vec3::new(12.0, 0.0, -7.0), bearing);
            let shot = ChangeoverShot::close_up(&man, 0.0, &[]);
            let to_camera = (shot.stand - man.translation).with_y(0.0).normalize();
            assert!(
                to_camera.dot(facing(bearing)) > 0.99,
                "he is not looking down the lens at {bearing}: {to_camera:?}"
            );
            assert!((shot.stand.y - ChangeoverShot::FACE_OFF.1).abs() < 1e-4);
            assert_eq!(shot.aim, man.translation.with_y(ChangeoverShot::FACE_AT));
        }
    }

    #[test]
    fn and_ends_behind_his_shoulders() {
        // The name is printed across the back of the shirt and nowhere else,
        // so the second half of the beat is worthless from any other bearing —
        // and it is the bearing he then runs away down.
        let bearing = 0.8;
        let man = standing(Vec3::new(-3.0, 0.0, 20.0), bearing);
        let shot = ChangeoverShot::close_up(&man, ChangeoverShot::PORTRAIT_MS as f32, &[]);
        let to_camera = (shot.stand - man.translation).with_y(0.0).normalize();
        assert!(
            to_camera.dot(facing(bearing)) < -0.99,
            "the camera is not behind him: {to_camera:?}"
        );
        assert_eq!(shot.aim, man.translation.with_y(ChangeoverShot::CHEST));
        // Further back and higher, which is what puts the pitch he is about to
        // run onto in the same frame as him.
        let face = ChangeoverShot::close_up(&man, 0.0, &[]);
        assert!(
            shot.stand.distance(man.translation) > face.stand.distance(man.translation),
            "the shot on his back is no further off than the one on his face"
        );
        assert!(shot.stand.y > face.stand.y, "and no higher");
        assert!(shot.lens < face.lens, "and no wider");
    }

    #[test]
    fn the_swing_goes_round_him_rather_than_through_him() {
        // Dead in front to dead behind: the chord between them IS the man.
        let man = standing(Vec3::new(4.0, 0.0, 4.0), -1.9);
        let mut previous: Option<Vec3> = None;
        let mut nearest = f32::MAX;
        for step in 0..=200 {
            let into = ChangeoverShot::PORTRAIT_MS as f32 * step as f32 / 200.0;
            let at = ChangeoverShot::close_up(&man, into, &[]).stand;
            let reach = (at - man.translation).with_y(0.0).length();
            nearest = nearest.min(reach);
            if let Some(previous) = previous {
                let jump = at.distance(previous);
                assert!(jump < 0.5, "the shot jumped {jump} m in one step");
            }
            previous = Some(at);
        }
        assert!(
            nearest > ChangeoverShot::FACE_OFF.0 - 0.01,
            "the camera closed to {nearest} m — through him"
        );
    }

    #[test]
    fn and_then_the_rig_stands_still_while_he_runs() {
        // ⚠ **The one thing the run beat must not do is travel with him.**
        // Everything else in this shot is re-derived from the subject's own
        // body every frame; do that to a man running at eight metres a second
        // and the camera goes with him.
        let man = standing(Vec3::new(2.0, 0.0, -35.0), 0.1);
        let planted = ChangeoverShot::close_up(&man, ChangeoverShot::PORTRAIT_MS as f32, &[]);
        let plant = (planted.stand, planted.lens);

        let mut away = man.translation;
        for _ in 0..20 {
            away += Vec3::new(0.0, 0.0, 1.1);
            let shot = ChangeoverShot::watching(away, plant);
            assert_eq!(shot.stand, planted.stand, "the rig followed him");
            assert_eq!(shot.lens, planted.lens, "…and re-lensed on the way");
            // The aim is the only thing that moves, and it is on him.
            assert_eq!(shot.aim, away.with_y(ChangeoverShot::CHEST));
        }
        // Which means he shrinks: he has run twenty-two metres away from a
        // camera that started eight off him.
        assert!(ChangeoverShot::watching(away, plant).stand.distance(away) > 25.0);
    }

    #[test]
    fn nobody_else_ever_stands_between_the_lens_and_the_man() {
        // ⚠ **This is the one the arc got wrong.** Clearing only the two ends
        // of the swing leaves the whole middle of it — which is exactly where
        // the man standing a couple of metres to his left is. Measured on the
        // first change of a real match: a team-mate 2.15 m away, the lens
        // sweeping past four metres out, and his back across the frame with
        // the subject somewhere behind him.
        let man = standing(Vec3::new(-8.5, 0.0, -29.2), 0.4);
        // Every bearing round him, one at a time, at the distance the real
        // one stood at and at a comfortable one.
        for reach in [2.15f32, 3.6] {
            for step in 0..24 {
                let angle = step as f32 / 24.0 * TAU;
                let beside = man.translation + Vec3::new(angle.sin(), 0.0, angle.cos()) * reach;
                for frame in 0..=200 {
                    let into = ChangeoverShot::PORTRAIT_MS as f32 * frame as f32 / 200.0;
                    let at = ChangeoverShot::close_up(&man, into, &[beside]).stand;
                    assert!(
                        !across_the_shot(at, man.translation, beside),
                        "a man at {reach} m on bearing {step}/24 is across the shot from {at:?}"
                    );
                }
            }
        }
    }
    /// Where a substitute actually stands: at the fourth official's shoulder,
    /// three quarters of a metre beyond the bench touchline, square to the
    /// line and facing the pitch. `Bench::entry_gate` and `Actors::facing` on
    /// the other two sides of the recording, restated because this crate
    /// cannot reach either.
    fn at_the_gate(along: f32) -> Transform {
        standing(Vec3::new(along, 0.0, -(Field::HALF_WIDTH + 0.75)), 0.0)
    }

    #[test]
    fn the_lens_never_leaves_the_ground() {
        // ⚠ Behind a man on his own touchline is BEHIND the hoardings, and
        // further still is inside a bank of seating — which `Bank::cull` then
        // hides, so a whole stand blinks out of the picture as the swing
        // crosses the line. The bench side is the one place the shot is
        // allowed out there, because that is where it lives and the near bank
        // is culled for the whole of it.
        let edges = [
            Vec3::new(0.0, 0.0, -Field::HALF_WIDTH),
            Vec3::new(0.0, 0.0, Field::HALF_WIDTH),
            Vec3::new(Field::HALF_LENGTH, 0.0, 0.0),
            Vec3::new(-Field::HALF_LENGTH, 0.0, Field::HALF_WIDTH),
            at_the_gate(4.5).translation,
        ];
        for boots in edges {
            for step in 0..16 {
                let man = standing(boots, step as f32 / 16.0 * TAU);
                for frame in 0..=100 {
                    let into = ChangeoverShot::PORTRAIT_MS as f32 * frame as f32 / 100.0;
                    let at = ChangeoverShot::close_up(&man, into, &[]).stand;
                    assert!(
                        at.x >= ChangeoverShot::ALONG.0 - 1e-3
                            && at.x <= ChangeoverShot::ALONG.1 + 1e-3
                            && at.z >= ChangeoverShot::ACROSS.0 - 1e-3
                            && at.z <= ChangeoverShot::ACROSS.1 + 1e-3,
                        "the lens walked out of the ground to {at:?} from {boots:?}"
                    );
                }
            }
        }

        // And the real one comes to rest exactly where the touchline rig
        // stands: `OUT` metres beyond the line, however far out the gate the
        // engine put him on happens to be.
        let man = at_the_gate(1.0);
        let rest = ChangeoverShot::close_up(&man, ChangeoverShot::PORTRAIT_MS as f32, &[]).stand;
        assert!(
            (rest.z + Field::HALF_WIDTH + ChangeoverShot::OUT).abs() < 1e-3,
            "the shot on his back came to rest at {rest:?}"
        );
        assert!((rest.y - ChangeoverShot::HEIGHT).abs() < 1e-4);

        // And what it gives up in distance it takes back in lens, so the
        // three quarters of a metre the gate costs the shot does not make him
        // smaller than a man stood in the middle of the pitch would be.
        let middle = standing(Vec3::ZERO, 0.0);
        let size = |man: &Transform| {
            let shot = ChangeoverShot::close_up(man, ChangeoverShot::PORTRAIT_MS as f32, &[]);
            in_frame(&shot, man.translation)
        };
        assert!(
            (size(&man) - size(&middle)).abs() < 0.01,
            "he changed size for standing beyond the line"
        );
    }

    #[test]
    fn the_lens_stops_short_of_anybody_standing_in_the_gap() {
        // ⚠ Rendered, the second close-up of a double change was the back of a
        // team-mate filling the frame with the subject somewhere behind him.
        // Nobody outside the change moves until it is over, so a man in the
        // way stays in the way for the whole beat.
        let man = standing(Vec3::ZERO, 0.0);
        let ahead = ChangeoverShot::heading(&man);
        let behind = -ahead;

        assert_eq!(
            ChangeoverShot::clear(Vec3::ZERO, behind, ChangeoverShot::BACK_OFF.0, &[]),
            ChangeoverShot::BACK_OFF.0,
            "an empty lane costs the shot nothing"
        );

        // A man in the gap in front pulls the lens in front of him…
        let wanted = ChangeoverShot::FACE_OFF.0;
        let blocker = ahead * 2.6;
        let reach = ChangeoverShot::clear(Vec3::ZERO, ahead, wanted, &[blocker]);
        assert!(
            reach < 2.6 - 0.5 && reach > ChangeoverShot::CRAMPED,
            "the lens came to {reach} m against a man at 2.6"
        );
        // …and the subject stays the same size in frame, because the lens
        // opens by exactly what the distance gave up.
        let blocked = ChangeoverShot::close_up(&man, 0.0, &[blocker]);
        let open = ChangeoverShot::close_up(&man, 0.0, &[]);
        assert!(
            (in_frame(&blocked, man.translation) - in_frame(&open, man.translation)).abs() < 0.01,
            "he changed size when the camera came forward"
        );

        // Somebody off to one side is not in the way, and neither is somebody
        // behind when the shot is on his face.
        for elsewhere in [ahead * 2.6 + Vec3::new(3.0, 0.0, 3.0), behind * 2.6] {
            assert_eq!(
                ChangeoverShot::clear(Vec3::ZERO, ahead, wanted, &[elsewhere]),
                wanted,
                "the shot gave way to somebody at {elsewhere:?}"
            );
        }

        // ⚠ **And a man two metres away is ducked in front of as well**, all
        // the way to `CRAMPED`. He is the case this exists for: the real one
        // stood 2.15 m off the second man of a real change, and a shot that
        // stops politely short of him is a shot of his back.
        let close = ChangeoverShot::clear(Vec3::ZERO, ahead, wanted, &[ahead * 2.0]);
        assert!(
            close < 2.0 - 0.3,
            "the lens stayed at {close} m, behind a man at 2"
        );
        assert!(close >= ChangeoverShot::CRAMPED);

        // What gives then is the FRAMING, not the sight line: the lens stops
        // widening at `WIDEST` rather than going to a fisheye, so the shot
        // crops in instead.
        let back = ChangeoverShot::PORTRAIT_MS as f32;
        let driven = ChangeoverShot::close_up(&man, back, &[behind * 4.0]);
        assert_eq!(driven.lens, ChangeoverShot::WIDEST);
        assert!(
            ChangeoverShot::close_up(&man, back, &[]).lens > driven.lens,
            "a shot with room is no tighter than one without"
        );
    }

    #[test]
    fn a_stoppage_is_one_shot_and_works_through_its_men_in_turn() {
        let mut changes = Vec::new();
        ChangeoverShot::stage(&mut changes, 60_000.0, 14_000.0, 7, 3);
        ChangeoverShot::stage(&mut changes, 60_400.0, 15_000.0, 9, 4);
        assert_eq!(changes.len(), 1, "one whistle is one shot");
        assert_eq!(changes[0].coming_on, vec![7, 9]);
        assert_eq!(changes[0].coming_off, vec![3, 4]);
        // The later of the two whistles decides when the shot ends, and there
        // is no linger past it any more — see [`ChangeoverShot::LINGER_MS`].
        assert_eq!(
            changes[0].to,
            60_400.0 + 15_000.0 + ChangeoverShot::LINGER_MS
        );

        let mut apart = Vec::new();
        ChangeoverShot::stage(&mut apart, 60_000.0, 14_000.0, 7, 3);
        ChangeoverShot::stage(&mut apart, 74_000.0, 14_000.0, 9, 4);
        assert_eq!(apart.len(), 2);
    }

    #[test]
    fn a_document_that_does_not_say_who_came_off_still_gets_its_close_ups() {
        // Zero is not a player, and the shot does not need him: what it looks
        // at is the man coming ON, which every document has carried since
        // there were substitutions in one. All he would have been is one more
        // id kept out of the sight-line test.
        let mut changes = Vec::new();
        ChangeoverShot::stage(&mut changes, 60_000.0, 14_000.0, 7, 0);
        assert_eq!(changes[0].coming_on, vec![7]);
        assert!(changes[0].coming_off.is_empty());
    }

    #[test]
    fn the_touchline_shot_is_outside_the_line_and_the_lens_opens() {
        let shot = ChangeoverShot {
            changes: Vec::new(),
            grip: 1.0,
            portrait: None,
            plant: None,
        };
        let (stand, aim) = shot.blend(Vec3::ZERO, Vec3::ZERO);
        assert!(
            stand.z < -Field::HALF_WIDTH,
            "the men come on between the camera and the pitch: {stand:?}"
        );
        assert_eq!(aim, Vec3::new(0.0, ChangeoverShot::CHEST, 0.0));
        assert_eq!(shot.magnification(), ChangeoverShot::LENS);

        let mut off = shot;
        off.grip = 0.0;
        assert_eq!(
            off.magnification(),
            1.0,
            "a shot that is off touches nothing"
        );
        assert_eq!(
            off.blend(Vec3::X, Vec3::Y),
            (Vec3::X, Vec3::Y),
            "…and leaves the gantry exactly where it was"
        );
    }

    #[test]
    fn the_close_ups_fit_inside_the_hold_the_engine_gives_them() {
        // `SubstitutionBreak::PORTRAIT_MS` stands the man still for exactly
        // this long, and `SubstitutionBreak::RUN_MS` holds the men after him
        // for the rest of the beat. Neither number can be reached from this
        // crate, so both are restated and asserted.
        //
        // ⚠ **And `BEAT_MS` is now the whole length of the window** — the
        // engine stops on its beats rather than waiting for either walker to
        // arrive (`SubstitutionBreak::beats_ms`), so a shot sized off these
        // figures is a shot sized off the footage that exists.
        assert_eq!(
            ChangeoverShot::PORTRAIT_MS,
            3_400.0,
            "the engine holds him for 3400"
        );
        assert_eq!(
            ChangeoverShot::RUN_MS,
            2_000.0,
            "the engine holds the next man for 2000"
        );
        assert_eq!(ChangeoverShot::BEAT_MS, 5_400.0);
    }
}
