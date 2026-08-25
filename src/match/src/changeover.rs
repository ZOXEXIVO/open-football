//! **The shot a substitution gets**: a close-up of every man being replaced,
//! and then the touchline.
//!
//! The broadcast rig is eighteen metres up and eighty back, looking down at
//! the whole ground. That is the right camera for football and the wrong one
//! for a substitution: the two men crossing are twelve pixels tall, the
//! exchange happens at the very bottom edge of the frame, and the whole point
//! of playing it out is lost.
//!
//! So a change gets two shots.
//!
//! **One man at a time, where he is standing.** For every player coming off,
//! the camera cuts to a second behind his back — close enough to read the name
//! printed across his shoulders — and then swings round him to his face. Then
//! it cuts to the next man and does it again. Nobody has moved yet: the engine
//! holds the whole window still for exactly as long as this takes (see
//! `SubstitutionBreak::PORTRAIT_MS`, which is [`ChangeoverShot::PORTRAIT_MS`]
//! on the other side of the recording), because a man cannot be shown standing
//! on the pitch and be running off it at the same time.
//!
//! **And then the touchline.** The rig stands outside the line, level with the
//! halfway line, directly behind the men waiting at the fourth official's
//! gate, and points at the centre spot: their backs to the lens, their names
//! across their shoulders, and the whole pitch laid out in front of them. The
//! man being replaced runs the other way, towards the camera, and they cross
//! at the gate. It holds that until play resumes.
//!
//! It reads the substitutions straight off [`ViewerConfig`], including how
//! long each one stopped the match for, so the shot lasts exactly as long as
//! the change did and not a constant somebody guessed at.

use crate::actors::PlayerActor;
use crate::camera::{CameraFlight, CameraOrbit};
use crate::config::ViewerConfig;
use crate::field::Field;
use crate::focus::CameraSubject;
use crate::pitch::Pitch;
use crate::playback::Playback;
use bevy::prelude::*;
use std::f32::consts::PI;

/// One change, as the camera needs it: when, whose backs to get behind at the
/// touchline, and who to look at one at a time before that.
struct Change {
    from: f64,
    to: f64,
    coming_on: Vec<u32>,
    /// The men being replaced, in the order the shot works through them — one
    /// [`ChangeoverShot::PORTRAIT_MS`] each. Same length as `coming_on` unless
    /// the document was written before it carried who came off, in which case
    /// it is empty and the change opens at the touchline.
    coming_off: Vec<u32>,
}

/// Where the camera is and how it is lensed for one close-up, worked out
/// against the man's own body rather than against the pitch.
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
    /// The close-up this frame, if the shot is on one. `None` once it has
    /// worked through them and moved to the touchline.
    ///
    /// Carried on the resource rather than worked out in [`Self::blend`]
    /// because it is measured off the man's own transform, which only the
    /// system that queries the actors can see.
    portrait: Option<Portrait>,
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

    /// **How long the shot spends on each man being replaced**, in ms of match
    /// clock: a second on his back, the swing round him, and a second and a
    /// half on his face — 3.4 s a man.
    ///
    /// ⚠ **`SubstitutionBreak::PORTRAIT_MS` is this same figure and the two
    /// have to agree.** That is what stands the whole window still while this
    /// runs; this crate cannot depend on that one, so if either moves the other
    /// moves with it. Hold for less than the shot and the man walks out of his
    /// own close-up; hold for more and the picture is on the touchline while
    /// twenty-two men stand about on the pitch.
    const PORTRAIT_MS: f64 = 3_400.0;
    /// The share of it spent behind him. **This is the beat the whole shot
    /// exists for** — his name is printed across the back of his shirt and
    /// nowhere else, so it is the one angle that says who is coming off.
    const BACK_MS: f64 = 1_000.0;
    /// …and the swing from there round to his face. The remainder of
    /// [`Self::PORTRAIT_MS`] — a second and a half — is spent looking at it,
    /// which is long enough to take in a face rather than to register that one
    /// went past.
    const TURN_MS: f64 = 900.0;
    /// How far behind him the first half stands, in metres, and how high.
    ///
    /// Six metres at [`Self::BACK_LENS`] puts him about two thirds of the
    /// frame tall with his feet just inside the bottom edge, which is as much
    /// shirt as the frame can hold. The height is a man's own: this is a shot
    /// taken from where somebody standing behind him would see it, not from
    /// above.
    const BACK_OFF: (f32, f32) = (6.0, 1.8);
    /// And where the second half ends up: **closer**, and level with his eyes.
    ///
    /// Two and a half metres rather than six, because a face is a tenth of the
    /// size of a shirt and the point of coming round is to see it.
    const FACE_OFF: (f32, f32) = (2.8, 1.65);
    /// What the face half aims at — his eyeline rather than his chest, which
    /// is the difference between a portrait and a shot of a collar.
    const FACE_AT: f32 = 1.55;
    /// The two lenses. Both are held on a man a few metres away rather than
    /// eighty, so both are wider than the broadcast lens and neither is a
    /// zoom: the closeness is bought by DISTANCE.
    ///
    /// The face is the tighter of the two — at 2.5 m it frames from his chest
    /// to just over his head — and it tightens ACROSS the swing, so the man
    /// grows as the camera comes round him rather than jumping when it lands.
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
    /// **The box the lens may stand in**, as half-extents along and across
    /// the pitch, in metres: the run-off, pulled in by a metre so the
    /// advertising hoardings standing at the end of it are always behind the
    /// camera rather than across the shot.
    ///
    /// ⚠ The banks of seating start two metres beyond the boards and
    /// [`Bank::cull`](crate::pitch::Bank) hides whichever one the lens is
    /// inside — so a rig that wanders out here does not merely stand behind a
    /// wall, it makes a whole stand blink out of the picture as it crosses the
    /// line.
    const PERIMETER: f32 = 1.0;
    const INSIDE: (f32, f32) = (
        Field::HALF_LENGTH + Pitch::END_MARGIN - Self::PERIMETER,
        Field::HALF_WIDTH + Pitch::SIDE_MARGIN - Self::PERIMETER,
    );

    /// How long a change lasts when the recording does not say — a document
    /// written before the engine played substitutions out, or one made on the
    /// instant path. See `SubstitutionInfo::break_ms`.
    const ASSUMED_MS: f64 = 12_000.0;
    /// How long the shot lingers past the end of the change, in ms, before it
    /// walks back to the broadcast rig. Enough to see the substitute take up
    /// his position rather than cutting on the referee's arm.
    const LINGER_MS: f64 = 1_200.0;

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
        // A document written before the shot had a first beat carries no man
        // coming off. Nobody to look at is not the same as a player with id
        // zero, so he is left out rather than queued and skipped — which also
        // keeps the count of close-ups honest for the timings below.
        if off > 0 {
            change.coming_off.push(off);
        }
    }

    /// Whether the shot should be on this frame, how far it has closed, and
    /// which man it is looking at.
    ///
    /// Runs after the bodies have been placed, like [`CameraSubject::settle`],
    /// because it measures the close-ups off the men themselves: where a man
    /// is standing and **which way he is pointing** are properties of his
    /// transform, and a shot of somebody's back has to be worked out from the
    /// back he actually has rather than from where the ball is.
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
        // Which man the close-up is on, and how far through his beat we are.
        // Past the last of them the shot is at the touchline, which is where a
        // change with nobody to look at starts.
        let portrait = wanted.and_then(|change| {
            let beat = now - change.from;
            let index = (beat / Self::PORTRAIT_MS).floor().max(0.0) as usize;
            let man = *change.coming_off.get(index)?;
            let (_, transform, _) = actors.iter().find(|(actor, _, visibility)| {
                actor.id == man && **visibility != Visibility::Hidden
            })?;
            // Everybody else who is drawn, so the swing can duck in past
            // whoever is standing in its way — see `clear`. A stack array
            // rather than a collection: this runs every frame a close-up is on
            // and there are never more than a couple of dozen of them.
            let mut others = [Vec3::ZERO; 32];
            let mut count = 0;
            for (actor, at, visibility) in &actors {
                if actor.id != man && *visibility != Visibility::Hidden && count < others.len() {
                    others[count] = at.translation;
                    count += 1;
                }
            }
            Some(Self::close_up(
                transform,
                (beat - index as f64 * Self::PORTRAIT_MS) as f32,
                &others[..count],
            ))
        });

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
    /// goes behind his shoulders whichever way he happens to be facing, and
    /// swings round to his face from there. `others` is everybody else who is
    /// drawn, because the one thing this shot cannot do is put a body between
    /// the lens and its subject — see [`Self::clear`].
    fn close_up(transform: &Transform, into: f32, others: &[Vec3]) -> Portrait {
        let boots = transform.translation;
        let turn =
            Self::ease(((into - Self::BACK_MS as f32) / Self::TURN_MS as f32).clamp(0.0, 1.0));

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
        let bearing = Quat::from_rotation_y(turn * PI) * -Self::heading(transform);
        let wanted = Self::BACK_OFF.0 + (Self::FACE_OFF.0 - Self::BACK_OFF.0) * turn;
        let reach = Self::clear(boots, bearing, wanted, others);

        Portrait {
            stand: Vec3::new(
                boots.x + bearing.x * reach,
                boots.y + Self::BACK_OFF.1 + (Self::FACE_OFF.1 - Self::BACK_OFF.1) * turn,
                boots.z + bearing.z * reach,
            ),
            aim: boots
                .with_y(Self::CHEST)
                .lerp(boots.with_y(Self::FACE_AT), turn),
            // **The lens opens with every metre the shot gives up**, so a man
            // it had to duck in past does not change how big the subject
            // comes out — until [`Self::WIDEST`], where opening any further
            // would be a fisheye and the shot crops instead.
            lens: ((Self::BACK_LENS + (Self::FACE_LENS - Self::BACK_LENS) * turn) * reach / wanted)
                .max(Self::WIDEST),
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
    /// in metres.
    ///
    /// The box is the run-off — `Pitch::SIDE_MARGIN` across and
    /// `Pitch::END_MARGIN` behind the goals — pulled in by [`Self::PERIMETER`]
    /// so the hoardings standing at the end of it are always behind the lens
    /// rather than across it.
    fn inside_the_ground(boots: Vec3, bearing: Vec3) -> f32 {
        let wall = |at: f32, step: f32, limit: f32| {
            if step.abs() < 1e-4 {
                f32::MAX
            } else {
                (limit * step.signum() - at) / step
            }
        };
        wall(boots.x, bearing.x, Self::INSIDE.0).min(wall(boots.z, bearing.z, Self::INSIDE.1))
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
            // **Behind their backs, looking out at the middle of the ground.**
            //
            // The substitutes are standing at the fourth official's shoulder
            // on the halfway line and the camera is directly behind them with
            // the whole pitch in front of everybody. They run away into the
            // shot; the men they are replacing run towards it. Two or three
            // coming on at once enter within a couple of metres of each other,
            // so one position is behind all of them and the aim is between
            // them by construction.
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
    fn the_first_beat_stands_behind_his_shoulders() {
        // The name is printed across the back of the shirt and nowhere else,
        // so the whole shot is worthless from any other bearing.
        for bearing in [0.0, 1.1, PI, -2.4] {
            let man = standing(Vec3::new(12.0, 0.0, -7.0), bearing);
            let shot = ChangeoverShot::close_up(&man, 0.0, &[]);
            let to_camera = (shot.stand - man.translation).with_y(0.0).normalize();
            assert!(
                to_camera.dot(facing(bearing)) < -0.99,
                "the camera is not behind him at {bearing}: {to_camera:?}"
            );
            assert!((shot.stand.y - ChangeoverShot::BACK_OFF.1).abs() < 1e-4);
            assert_eq!(shot.aim, man.translation.with_y(ChangeoverShot::CHEST));
        }
    }

    #[test]
    fn and_the_last_beat_is_in_front_of_his_face() {
        let bearing = 0.8;
        let man = standing(Vec3::new(-3.0, 0.0, 20.0), bearing);
        let shot = ChangeoverShot::close_up(&man, ChangeoverShot::PORTRAIT_MS as f32, &[]);
        let to_camera = (shot.stand - man.translation).with_y(0.0).normalize();
        assert!(
            to_camera.dot(facing(bearing)) > 0.99,
            "he is not looking down the lens: {to_camera:?}"
        );
        assert_eq!(shot.aim, man.translation.with_y(ChangeoverShot::FACE_AT));
        // Closer, which is the whole reason for coming round.
        let back = ChangeoverShot::close_up(&man, 0.0, &[]);
        assert!(
            shot.stand.distance(man.translation) < back.stand.distance(man.translation),
            "the face shot is no nearer than the shirt was"
        );
        assert!(shot.lens > back.lens, "and no tighter");
    }

    #[test]
    fn the_swing_goes_round_him_rather_than_through_him() {
        // Dead behind to dead in front: the chord between them IS the man.
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
    #[test]
    fn the_lens_never_leaves_the_ground() {
        // ⚠ Six metres behind a full-back on his own touchline is three metres
        // BEHIND the hoardings, and five is inside a bank of seating — which
        // `Bank::cull` then hides, so a whole stand blinks out of the picture
        // as the swing crosses the line.
        let corners = [
            Vec3::new(0.0, 0.0, -Field::HALF_WIDTH),
            Vec3::new(0.0, 0.0, Field::HALF_WIDTH),
            Vec3::new(Field::HALF_LENGTH, 0.0, 0.0),
            Vec3::new(-Field::HALF_LENGTH, 0.0, Field::HALF_WIDTH),
        ];
        for boots in corners {
            for step in 0..16 {
                let man = standing(boots, step as f32 / 16.0 * TAU);
                for frame in 0..=100 {
                    let into = ChangeoverShot::PORTRAIT_MS as f32 * frame as f32 / 100.0;
                    let at = ChangeoverShot::close_up(&man, into, &[]).stand;
                    assert!(
                        at.x.abs() <= ChangeoverShot::INSIDE.0 + 1e-3
                            && at.z.abs() <= ChangeoverShot::INSIDE.1 + 1e-3,
                        "the lens walked out of the ground to {at:?} from {boots:?}"
                    );
                }
            }
        }
        // And what it gives up in distance it takes back in lens, so a man on
        // the touchline is framed like a man in the middle of the pitch.
        let edge = standing(Vec3::new(0.0, 0.0, -Field::HALF_WIDTH), 0.0);
        let middle = standing(Vec3::ZERO, 0.0);
        let size = |man: &Transform| {
            let shot = ChangeoverShot::close_up(man, 0.0, &[]);
            shot.lens / shot.stand.distance(man.translation)
        };
        assert!(
            (size(&edge) - size(&middle)).abs() < 0.02,
            "he changed size for standing near the line"
        );
    }

    #[test]
    fn the_lens_stops_short_of_anybody_standing_in_the_gap() {
        // ⚠ Rendered, the second close-up of a double change was the back of a
        // team-mate filling the frame with the subject somewhere behind him.
        // Nobody on the pitch will move until the change is over, so a man in
        // the way stays in the way for the whole beat.
        let man = standing(Vec3::ZERO, 0.0);
        let behind = -ChangeoverShot::heading(&man);
        let wanted = ChangeoverShot::BACK_OFF.0;

        assert_eq!(
            ChangeoverShot::clear(Vec3::ZERO, behind, wanted, &[]),
            wanted,
            "an empty lane costs the shot nothing"
        );

        // A man four metres behind him pulls the lens in front of him…
        let blocker = behind * 4.0;
        let reach = ChangeoverShot::clear(Vec3::ZERO, behind, wanted, &[blocker]);
        assert!(
            reach < 4.0 - 0.5 && reach > 2.0,
            "the lens came to {reach} m against a man at 4"
        );
        // …and the subject stays the same size in frame, because the lens
        // opens by exactly what the distance gave up.
        let blocked = ChangeoverShot::close_up(&man, 0.0, &[blocker]);
        let open = ChangeoverShot::close_up(&man, 0.0, &[]);
        assert!(
            (blocked.lens / blocked.stand.distance(man.translation)
                - open.lens / open.stand.distance(man.translation))
            .abs()
                < 0.02,
            "he changed size when the camera came forward"
        );

        // Somebody off to one side is not in the way, and neither is somebody
        // in front when the shot is looking at a back.
        for elsewhere in [behind * 4.0 + Vec3::new(3.0, 0.0, 3.0), -behind * 4.0] {
            assert_eq!(
                ChangeoverShot::clear(Vec3::ZERO, behind, wanted, &[elsewhere]),
                wanted,
                "the shot gave way to somebody at {elsewhere:?}"
            );
        }

        // ⚠ **And a man two metres away is ducked in front of as well**, all
        // the way to `CRAMPED`. He is the case this exists for: the real one
        // stood 2.15 m off the second man of a real change, and a shot that
        // stops politely short of him is a shot of his back.
        let close = ChangeoverShot::clear(Vec3::ZERO, behind, wanted, &[behind * 2.0]);
        assert!(
            close < 2.0 - 0.3,
            "the lens stayed at {close} m, behind a man at 2"
        );
        assert!(close >= ChangeoverShot::CRAMPED);

        // What gives then is the FRAMING, not the sight line: the lens stops
        // widening at `WIDEST` rather than going to a fisheye, so the shot
        // crops in instead.
        let driven = ChangeoverShot::close_up(&man, 0.0, &[behind * 2.0]);
        assert_eq!(driven.lens, ChangeoverShot::WIDEST);
        assert!(
            open.lens > driven.lens,
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
        assert_eq!(changes[0].to, 60_400.0 + 15_000.0 + 1_200.0);

        let mut apart = Vec::new();
        ChangeoverShot::stage(&mut apart, 60_000.0, 14_000.0, 7, 3);
        ChangeoverShot::stage(&mut apart, 74_000.0, 14_000.0, 9, 4);
        assert_eq!(apart.len(), 2);
    }

    #[test]
    fn a_document_that_does_not_say_who_came_off_opens_at_the_touchline() {
        // Zero is not a player. Queueing him would spend a beat of the shot
        // looking at nobody — and would push the touchline shot back past the
        // point the engine let everybody move again.
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
        // `SubstitutionBreak::PORTRAIT_MS` stands the whole window still for
        // this long per man. Spend more than it here and the man walks out of
        // his own shot.
        assert!(ChangeoverShot::BACK_MS + ChangeoverShot::TURN_MS < ChangeoverShot::PORTRAIT_MS);
        assert_eq!(
            ChangeoverShot::PORTRAIT_MS,
            3_400.0,
            "the engine holds 3400"
        );
    }
}
