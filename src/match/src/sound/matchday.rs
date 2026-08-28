//! The player: what the ball does, turned into what it sounds like.
//!
//! # Only the two ends of a pass
//!
//! A footballer touches the ball about fourteen times a minute and almost all
//! of those touches are him carrying it — the knocks that take a dribble up
//! the pitch. Played, they are a rattle under the whole match, and they were
//! reported as exactly that. **A move has two moments worth hearing: the ball
//! being sent, and the ball arriving.** Everything in between is a man
//! running with it, and it is silent here.
//!
//! # How the two are told apart
//!
//! Not by how hard the ball was hit — a dribble knocked into space and a short
//! pass leave the boot at the same speed, and no threshold separates them. By
//! **who ends up with it**, which is a different question and one the viewer
//! can simply look up: the whole recording is already in memory, so where the
//! ball and a given player will be in three quarters of a second is a read
//! rather than a guess. That is [`Soundtrack::keeps_it`], and it answers both
//! halves at once:
//!
//! - a contact after which the striker **still has the ball** is a dribble
//!   touch — silent;
//! - a contact after which he **does not** is the ball being sent — a pass
//!   out;
//! - a player gaining the ball and **still having it** a moment later has
//!   received it — a pass in. One who does not had it roll past him.
//!
//! It is the same trick the animation rig already runs on this recording to
//! give a footballer a backswing — see
//! [`Track::position_ahead`](crate::recording::replay::Track).

use crate::players::actors::{Actors, BallState};
use crate::recording::playback::Playback;
use crate::recording::replay::ReplayTracks;
use crate::scene::field::Field;
use crate::scene::net::Netting;
use crate::sound::mixer::{Meeting, Mixer};
use bevy::prelude::*;

/// The browser's audio engine, once it has been persuaded to hand one over.
///
/// A non-send resource because everything inside it is a JavaScript handle,
/// and lazily filled because a page that has never been touched is not allowed
/// to open one: a viewer who never presses play never creates a context, and
/// so never gets the console warning that comes with creating one nobody
/// asked for.
///
/// `None` after [`Self::amplify`] has run means the browser refused, and the
/// replay is silent for the rest of the session. That is a whole failure
/// mode: it is checked once and never retried, because a browser that says no
/// to Web Audio does not change its mind.
#[derive(Default)]
pub struct Speakers {
    mixer: Option<Mixer>,
    asked: bool,
}

impl Speakers {
    /// The mixer, opening the audio engine on the first call.
    ///
    /// Only ever called on a frame that actually wants a noise — see
    /// [`Soundtrack::follow_playhead`], which returns before this while the
    /// replay is paused or muted.
    fn amplify(&mut self) -> Option<&Mixer> {
        if !self.asked {
            self.asked = true;
            self.mixer = Mixer::open();
        }
        self.mixer.as_ref()
    }

    /// The mixer only if there already is one — never opening it. What the
    /// muted and paused paths use, so that asking for silence does not start
    /// an audio engine to be silent with.
    fn live(&self) -> Option<&Mixer> {
        self.mixer.as_ref()
    }
}

/// A sound the match has earned, worked out and then handed to the mixer.
///
/// Returned rather than played on the spot so the two rules that decide it —
/// which are about football, not about audio — can be checked without a
/// browser to make a noise in.
struct Cue {
    meeting: Meeting,
    /// How hard, 0..1.
    weight: f32,
    /// Where across the picture, -1..1.
    pan: f32,
    /// Seconds of REAL time from now. Non-zero only on the path that reads
    /// the strike before it happens.
    delay: f32,
}

/// Who has the ball, and what has already been played about it.
#[derive(Resource, Default)]
pub struct Soundtrack {
    /// Whether the viewer has asked for quiet. Owned here rather than in the
    /// transport bar because the bar is a view of it: the chip reads this and
    /// writes it, and everything else only reads.
    pub muted: bool,
    /// **The last man to have had the ball**, which is not the same as the man
    /// within reach of it now — see [`Soundtrack::possession`] for why it has
    /// to be sticky. `None` only before anybody has touched it.
    holder: Option<u32>,
    /// Whether the ball leaving the current holder has already been heard.
    ///
    /// The pass out is normally played off the animation rig's lookahead, on
    /// time and a tenth of a second before the ball is anywhere; the fallback
    /// below fires on possession actually being lost, which is later. This is
    /// what stops the two from both playing.
    spoken: bool,
    /// Match time of the contact already handed to the mixer, so a strike
    /// that stays visible in the lookahead for several frames is played once.
    struck: Option<f64>,
    /// Whether the ball was inside a goal on the previous frame.
    netted: bool,
    /// What the ball was doing on the previous frame, so the fallback knows
    /// how hard it was sent.
    carried: Vec3,
}

impl Soundtrack {
    /// Master level while the replay is running and nobody has asked for
    /// quiet. **This is the volume knob**: every level in the mix is set
    /// relative to a shot, so turning the whole soundtrack up or down is this
    /// number and nothing else.
    const LEVEL: f32 = 1.0;

    /// How long after a contact the recording is asked who has the ball, in
    /// milliseconds of match time.
    ///
    /// Long enough that a pass has unmistakably left its man — even a gentle
    /// five-metre ball is most of the way to its target by now — and short
    /// enough that a dribbler has not yet caught up with and re-touched a
    /// knock, which would make his own dribble look like a pass he received
    /// back.
    const SETTLE_MS: f64 = 750.0;

    /// …and how far the ball may be from him at that moment and still be his,
    /// in metres.
    ///
    /// A man running with the ball knocks it a stride or two ahead and stays
    /// with it; over three quarters of a second even a long knock leaves him
    /// no further behind than this. A ball he has actually passed is six
    /// metres away by then at the gentlest weight anybody passes at.
    const STILL_HIS: f32 = 3.5;

    /// **How high the ball may be and still be AT a man rather than over
    /// him**, in metres.
    ///
    /// ⚠ **Without a ceiling, possession is a shadow on the grass.** The
    /// range in `BallState::nearest` is measured across the ground — the rig
    /// has no use for the other axis, because a shadow is where a man stands
    /// over the ball — so a cross six metres up is owned by every player it
    /// flies OVER. Each one takes the ball off whoever actually played it and
    /// clears `spoken` over somebody who never touched it, the backstop fires
    /// a strike in mid-flight, and the arrival is either heard while the ball
    /// is still in the air above the man or, once he is already the holder
    /// when it lands, never heard at all. A cross, a clearance, a goal kick
    /// and a ball over the top are all that shape, which is how the long
    /// game came to be the silent half of the match.
    ///
    /// The number is the engine's, not one of this module's own: a player is
    /// 1.8 m to the head and reaches 2.2 m with both feet down, a jump takes
    /// that to between 2.5 m and 3.1 m depending on how well he leaps, and
    /// the engine stops letting him CLAIM a ball at 2.8 m — which is this
    /// same question asked where possession is actually decided. Anything
    /// higher is in flight, and a ball in flight belongs to nobody until it
    /// comes down.
    const OVERHEAD: f32 = 2.8;

    /// How far apart in match time two contacts have to be to be two
    /// contacts.
    ///
    /// The lookahead re-derives the coming strike every frame off a 30 ms
    /// probe (see [`Actors::next_impact`](crate::players::actors::Actors)), so
    /// the instant it reports wobbles by up to a probe's width while the
    /// playhead closes on it. Anything inside this window is the same strike
    /// seen again.
    const REARM_MS: f64 = 120.0;

    /// The slowest a ball can leave somebody and count as having been sent,
    /// in metres per second.
    ///
    /// Only the fallback needs it — the lookahead path knows the answer
    /// outright. Set below the gentlest thing anybody passes at, because the
    /// possession test has already done the work of deciding this was a pass;
    /// all this rules out is a ball trickling out of play off a shin.
    const SENT: f32 = 2.5;

    /// How far a contact at the goal line is panned. A broadcast mix is
    /// nearly mono; this is enough to place a kick and not enough to notice
    /// it being placed.
    const SPREAD: f32 = 0.55;

    /// Once a frame: works out what has become of the ball and tells the
    /// mixer.
    ///
    /// Runs behind [`Actors::follow_playhead`], which is what settles
    /// [`BallState`], and ahead of [`Playback::end_frame`], which clears the
    /// `seeked` flag this reads.
    pub fn follow_playhead(
        playback: Res<Playback>,
        ball: Res<BallState>,
        mut tracks: ResMut<ReplayTracks>,
        mut soundtrack: ResMut<Soundtrack>,
        mut speakers: NonSendMut<Speakers>,
    ) {
        // A paused replay is a still picture, and nothing is being kicked in
        // a still picture. Muted is the same instruction arriving from the bar
        // instead of from the transport.
        if soundtrack.muted || !playback.playing {
            if let Some(mixer) = speakers.live() {
                mixer.listen(0.0);
            }
            soundtrack.resync(&ball);
            return;
        }

        let Some(mixer) = speakers.amplify() else {
            return;
        };
        // Cheap on a running context, and the only thing that gets a viewer
        // who pressed play before the browser trusted the page from silence
        // to sound.
        mixer.wake();
        mixer.listen(Self::LEVEL);

        // A seek — or a cut from the end of one clip to the start of the next
        // — lands the playhead somewhere it has not been playing towards.
        // Everything this resource remembers is about the stretch of match it
        // just left: the strike that was armed for the frame after this one is
        // now forty minutes away, and whoever had the ball no longer does.
        if playback.seeked {
            soundtrack.resync(&ball);
            return;
        }

        let sent = soundtrack.sent(&playback, &ball, &mut tracks);
        let changed = soundtrack.possession(playback.time_ms, &ball, &mut tracks);
        for cue in [sent, changed].into_iter().flatten() {
            mixer.touch(cue.meeting, cue.weight, cue.pan, cue.delay);
        }
        soundtrack.netting(&ball, mixer);
        soundtrack.carried = ball.velocity;
    }

    /// **The ball being sent**, off the animation rig's own lookahead.
    ///
    /// This is the path that lands on time: the rig reads the coming strike
    /// ahead of the playhead so a footballer can take a backswing, and the
    /// same reading lets the sound be SCHEDULED on the audio clock rather than
    /// fired on the frame it happens to be noticed. `delay` is seconds of
    /// MATCH time and the audio clock runs in real seconds, so it is divided
    /// by the playback speed on the way across.
    ///
    /// A contact the striker still owns three quarters of a second later is a
    /// man running with the ball. It is armed — so the fallback and the
    /// lookahead do not both count it — and then not played.
    fn sent(
        &mut self,
        playback: &Playback,
        ball: &BallState,
        tracks: &mut ReplayTracks,
    ) -> Option<Cue> {
        let impact = ball.impact?;
        let contact = impact.contact;
        let when = playback.time_ms + (contact.delay as f64) * 1000.0;
        if !self.arm(when) {
            return None;
        }
        if Self::keeps_it(tracks, impact.by, when).unwrap_or(false) {
            // His own ball, still. The dribble is the thing this crate is
            // quiet about.
            return None;
        }
        self.spoken = true;
        Some(Cue {
            meeting: Meeting::Struck(contact.kind),
            weight: Self::weight(contact.velocity.length()),
            pan: Self::across(contact.at.x),
            delay: contact.delay / playback.speed.max(0.01),
        })
    }

    /// **The ball arriving**, and the backstop for it leaving.
    ///
    /// Runs off possession changing hands rather than off a contact, which is
    /// what makes it exact about the two moments that matter and blind to
    /// everything between them.
    ///
    /// ⚠ **The holder is sticky, and has to be.** He is the last man to have
    /// had the ball, not the man within reach of it right now — a dribbler
    /// knocks it three metres ahead and is out of reach of his own ball for
    /// most of every stride. Clearing the holder each time that happens turns
    /// one carried ball into a pass out and a pass in per touch, which is the
    /// rattle this whole file exists to remove.
    fn possession(&mut self, now: f64, ball: &BallState, tracks: &mut ReplayTracks) -> Option<Cue> {
        match Self::owner(ball) {
            // Nobody within reach: travelling, loose, or knocked ahead of the
            // man running with it.
            None => {
                let holder = self.holder?;
                // Normally the going has already been heard — [`Self::sent`]
                // fires a tenth of a second earlier off the lookahead — so
                // this speaks only for the passes the rig's own gates are too
                // strict to see. And only if he really has given it up.
                if self.spoken
                    || self.carried.length() <= Self::SENT
                    || Self::keeps_it(tracks, holder, now).unwrap_or(false)
                    || !self.arm(now)
                {
                    return None;
                }
                self.spoken = true;
                Some(Cue {
                    meeting: Meeting::Struck(Actors::strike_kind(
                        ball.position,
                        self.carried.length(),
                    )),
                    weight: Self::weight(self.carried.length()),
                    pan: Self::across(ball.position.x),
                    delay: 0.0,
                })
            }
            // The man who already had it, back on his own ball between
            // touches. Nothing has changed hands and nothing is played.
            Some(taker) if self.holder == Some(taker) => None,
            // Somebody else has it — but only if he still has it in a moment.
            // A ball rolling past a man's feet puts him nearest for three
            // frames and is not a touch at all.
            Some(taker) => {
                self.holder = Some(taker);
                self.spoken = false;
                Self::keeps_it(tracks, taker, now)
                    .unwrap_or(false)
                    .then(|| Cue {
                        meeting: Meeting::Received,
                        weight: Self::weight(self.carried.length()),
                        pan: Self::across(ball.position.x),
                        delay: 0.0,
                    })
            }
        }
    }

    /// **Does `id` still have the ball a moment after `when`?**
    ///
    /// The one question this module turns on, and the recording can simply be
    /// asked it — see the note at the top of the file. `None` when either
    /// track has nothing to say that far ahead, which is the honest answer at
    /// the very end of a clip; both callers read that as "he does not", so a
    /// contact at the edge of a recording is played rather than swallowed.
    fn keeps_it(tracks: &mut ReplayTracks, id: u32, when: f64) -> Option<bool> {
        let settled = when + Self::SETTLE_MS;
        let ball = tracks.ball.position_ahead(settled)?;
        let man = tracks.players.get_mut(&id)?.position_ahead(settled)?;
        let ball = Field::to_world(ball[0], ball[1], ball[2]);
        let man = Field::to_world(man[0], man[1], man[2]);
        // Across the ground only: a ball over his head is still his.
        Some(Vec2::new(ball.x - man.x, ball.z - man.z).length() <= Self::STILL_HIS)
    }

    /// Who is carrying the ball, if anybody.
    ///
    /// [`Actors::STRIKE_REACH`] rather than a radius of this module's own:
    /// "close enough to have played it" is one question with one answer, and
    /// the rig already owns it. It owns one AXIS of it, though — the reach is
    /// a distance across the grass — so the height the rig has no use for is
    /// added here, and it is the difference between a man on the ball and a
    /// man a ball is flying over. See [`Self::OVERHEAD`].
    fn owner(ball: &BallState) -> Option<u32> {
        if !ball.on_pitch {
            return None;
        }
        // In the gloves he is holding it whatever the distance says — the
        // drawn ball is displaced to his hands and the recorded one is not.
        if let Some(keeper) = ball.held_by {
            return Some(keeper);
        }
        if ball.position.y > Self::OVERHEAD {
            return None;
        }
        ball.nearest
            .filter(|(_, range)| *range <= Actors::STRIKE_REACH)
            .map(|(id, _)| id)
    }

    /// The ball crossing into the netting, on the frame it does.
    fn netting(&mut self, ball: &BallState, mixer: &Mixer) {
        let netted = ball.on_pitch && Netting::inside_a_goal(ball.position);
        if netted && !self.netted {
            mixer.net(Self::across(ball.position.x));
        }
        self.netted = netted;
    }

    /// Whether a contact at this match time is a new one, and remembers it if
    /// so. The single gate both paths go through.
    fn arm(&mut self, when: f64) -> bool {
        if self
            .struck
            .is_some_and(|last| when <= last + Self::REARM_MS)
        {
            return false;
        }
        self.struck = Some(when);
        true
    }

    /// How hard it was hit, 0..1, off the speed involved.
    ///
    /// Against [`Actors::HAMMERED`] — the crate's existing p90 of a recorded
    /// match's strikes — rather than a number of this module's own. A second
    /// figure for "hit as hard as anybody hits one" is how the picture and the
    /// sound come to disagree about which of them was a shot.
    fn weight(speed: f32) -> f32 {
        (speed / Actors::HAMMERED).clamp(0.0, 1.0)
    }

    /// Where a point on the pitch sits across the picture, -1..1.
    ///
    /// The pitch's LENGTH is what runs left to right on screen: the main
    /// camera sits on a touchline looking across, so the two goals are the two
    /// sides of the frame. See [`Mixer::aim`](crate::sound::mixer::Mixer).
    fn across(x: f32) -> f32 {
        (x / Field::HALF_LENGTH).clamp(-1.0, 1.0) * Self::SPREAD
    }

    /// Forgets everything about the stretch of match just left, and adopts
    /// wherever the playhead now is as the state of the world without making
    /// a noise about any of it.
    fn resync(&mut self, ball: &BallState) {
        self.struck = None;
        self.spoken = false;
        self.holder = Self::owner(ball);
        self.netted = ball.on_pitch && Netting::inside_a_goal(ball.position);
        self.carried = ball.velocity;
    }
}

/// The rules here that are about a football match rather than about audio:
/// who has the ball, and which of the things done to it are worth hearing.
#[cfg(test)]
mod possession {
    use super::*;
    use crate::players::actors::Strike;
    use crate::recording::replay::Sample;

    /// A ball on the deck, `range` metres from player 7.
    fn ball(range: f32) -> BallState {
        BallState {
            position: Vec3::new(0.0, 0.2, 0.0),
            on_pitch: true,
            nearest: Some((7, range)),
            ..default()
        }
    }

    /// Engine units for a distance in metres along the pitch.
    fn units(metres: f32) -> f32 {
        metres / Field::METERS_PER_UNIT
    }

    /// A straight run from one point to another over a second, sampled at the
    /// recorder's own 30 ms step.
    ///
    /// ⚠ **The step matters.** A track whose samples are further apart than
    /// [`INTERPOLATION_GAP_MS`](crate::recording::replay) is HELD rather than
    /// interpolated — the recorder drops samples that repeat a position, so a
    /// wide gap means the thing was standing still. A fixture written with two
    /// samples a second apart therefore describes nobody moving at all, which
    /// is how this file's first pass "proved" that a twelve-metre pass never
    /// left its man.
    ///
    /// Metres in, engine units out: `x` and `y` are the 0.125 m grid.
    fn run(from: (f32, f32), to: (f32, f32), height: f32) -> Vec<Sample> {
        const STEP_MS: u32 = 30;
        const OVER_MS: u32 = 1_000;
        (0..=OVER_MS / STEP_MS)
            .map(|step| {
                let t = step * STEP_MS;
                let f = (t as f32 / OVER_MS as f32).min(1.0);
                Sample {
                    t,
                    x: units(from.0 + (to.0 - from.0) * f),
                    y: units(from.1 + (to.1 - from.1) * f),
                    // Height is already metric — the one axis that is not the
                    // grid. See `Field::to_world`.
                    z: height * f,
                }
            })
            .collect()
    }

    /// A recording of one ball and player 7, each running a straight line.
    fn recording(ball: Vec<Sample>, man: Vec<Sample>) -> ReplayTracks {
        let mut tracks = ReplayTracks::default();
        tracks.ball.merge(ball);
        tracks.players.entry(7).or_default().merge(man);
        tracks
    }

    /// **The whole point.** A man carrying the ball knocks it ahead and runs
    /// after it, so three quarters of a second later it is still at his feet —
    /// and that touch must not be played.
    #[test]
    fn a_dribbler_still_has_the_ball_and_so_makes_no_sound() {
        // Ball knocked from 100 to 104 m along the pitch; he follows it from
        // 99 to 103.2. Under a metre apart when the question is asked.
        let mut tracks = recording(
            run((100.0, 30.0), (104.0, 30.0), 0.0),
            run((99.0, 30.0), (103.2, 30.0), 0.0),
        );
        assert_eq!(
            Soundtrack::keeps_it(&mut tracks, 7, 0.0),
            Some(true),
            "his own knock was heard as a pass"
        );
    }

    /// …and the same knock, once he has actually passed it, is not his any
    /// more. Even a gentle five-metre ball is well clear of him by then.
    #[test]
    fn a_pass_has_left_its_man() {
        let mut tracks = recording(
            run((100.0, 30.0), (112.0, 30.0), 0.0),
            // He plays it and stops.
            run((99.5, 30.0), (100.5, 30.0), 0.0),
        );
        assert_eq!(Soundtrack::keeps_it(&mut tracks, 7, 0.0), Some(false));
    }

    /// …and the shortest thing anybody would call a pass is still a pass. A
    /// five-metre ball is clear of the man who played it well before the
    /// question is asked, which is what lets the whole test work without a
    /// threshold on how hard the ball was hit.
    #[test]
    fn even_a_five_metre_ball_has_left_him() {
        let mut tracks = recording(
            run((100.0, 30.0), (105.0, 30.0), 0.0),
            run((99.6, 30.0), (99.9, 30.0), 0.0),
        );
        assert_eq!(Soundtrack::keeps_it(&mut tracks, 7, 0.0), Some(false));
    }

    /// A ball over a man's head is still a ball he is under. The test is
    /// across the ground, not through the air.
    #[test]
    fn height_does_not_take_the_ball_off_him() {
        let mut tracks = recording(
            // Straight up off his own boot and hanging over him.
            run((100.0, 30.0), (100.5, 30.0), 6.0),
            run((100.0, 30.0), (100.2, 30.0), 0.0),
        );
        assert_eq!(Soundtrack::keeps_it(&mut tracks, 7, 0.0), Some(true));
    }

    /// Nothing streamed yet is not "he lost it" — but both callers read it as
    /// a sound worth playing, because a contact at the very edge of a clip is
    /// better heard than swallowed.
    #[test]
    fn a_track_that_says_nothing_is_not_a_dribble() {
        let mut nothing = ReplayTracks::default();
        assert_eq!(Soundtrack::keeps_it(&mut nothing, 7, 0.0), None);
        assert!(!Soundtrack::keeps_it(&mut nothing, 7, 0.0).unwrap_or(false));
    }

    /// Possession is the rig's own reach, and a ball out of play belongs to
    /// nobody however close somebody is standing.
    #[test]
    fn who_has_the_ball() {
        assert_eq!(Soundtrack::owner(&ball(0.5)), Some(7));
        assert_eq!(Soundtrack::owner(&ball(Actors::STRIKE_REACH + 0.1)), None);

        let mut gone = ball(0.5);
        gone.on_pitch = false;
        assert_eq!(Soundtrack::owner(&gone), None);

        // A keeper holding it owns it whatever the recorded distance says:
        // the drawn ball is displaced into his gloves and the recorded one is
        // not, so the range can read as anything.
        let mut gathered = ball(9.0);
        gathered.held_by = Some(1);
        assert_eq!(Soundtrack::owner(&gathered), Some(1));
    }

    /// ⚠ **A ball in flight belongs to nobody it passes over.** The reach is
    /// a distance across the grass, so a cross six metres up reads as being
    /// at the feet of every man under it. See [`Soundtrack::OVERHEAD`].
    #[test]
    fn a_ball_in_flight_belongs_to_nobody_it_passes_over() {
        let mut over = ball(0.4);
        over.position.y = 6.0;
        assert_eq!(Soundtrack::owner(&over), None, "it is over him, not at him");

        // Dropping onto him it is his again, a shade before it lands — which
        // is when a man plays a dropping ball.
        over.position.y = Soundtrack::OVERHEAD - 0.4;
        assert_eq!(Soundtrack::owner(&over), Some(7));

        // And a ball met with the head is a ball he reached.
        over.position.y = 1.9;
        assert_eq!(Soundtrack::owner(&over), Some(7), "he can head that");
    }

    /// **The long game was the silent half of the match.** A cross flies over
    /// a man on its way to the far post: taken across the grass alone he owns
    /// it while it is six metres above him, which costs the man who crossed
    /// it the ball and plays the arrival into thin air — and then leaves the
    /// real arrival silent, because by the time it lands he is already the
    /// holder and nothing has changed hands.
    #[test]
    fn a_cross_is_heard_arriving_and_not_before() {
        let mut taken = recording(
            run((112.0, 30.0), (113.0, 30.0), 0.0),
            run((112.4, 30.0), (113.2, 30.0), 0.0),
        );
        let mut soundtrack = Soundtrack {
            holder: Some(3),
            // The ball leaving him has already been heard off the lookahead.
            spoken: true,
            carried: Vec3::new(18.0, 0.0, 0.0),
            ..default()
        };

        let mut over = ball(0.4);
        over.position.y = 6.0;
        assert!(
            soundtrack.possession(0.0, &over, &mut taken).is_none(),
            "the ball was heard arriving while it was still in the air"
        );
        assert_eq!(
            soundtrack.holder,
            Some(3),
            "a man it flew over took it off the man who crossed it"
        );

        // …and now it comes down on him.
        let arrival = soundtrack
            .possession(0.0, &ball(0.4), &mut taken)
            .expect("the cross arriving is a sound");
        assert!(matches!(arrival.meeting, Meeting::Received));
        assert_eq!(soundtrack.holder, Some(7));
    }

    /// **An aerial contact is heard on its way out.** The lookahead never
    /// sees a header — the ball does not multiply its speed, it turns round —
    /// so the backstop is the whole of it: he is under the dropping ball, he
    /// does not keep it, and the moment it climbs off his head it has left
    /// him. Before the ceiling above, it never left him at all: it was still
    /// within a stride of him across the grass while it was three metres up.
    #[test]
    fn a_header_is_heard_when_it_leaves_him() {
        // Headed twenty metres back the way it came, climbing as it goes.
        let mut cleared = recording(
            run((100.0, 30.0), (80.0, 30.0), 4.0),
            run((100.0, 30.0), (100.5, 30.0), 0.0),
        );
        let mut soundtrack = Soundtrack {
            holder: Some(7),
            carried: Vec3::new(-14.0, 6.0, 0.0),
            ..default()
        };

        let climbing = BallState {
            position: Vec3::new(0.0, 3.0, 0.0),
            on_pitch: true,
            nearest: Some((7, 0.8)),
            ..default()
        };
        let out = soundtrack
            .possession(0.0, &climbing, &mut cleared)
            .expect("the header leaving him is a sound");
        assert!(
            matches!(out.meeting, Meeting::Struck(Strike::Head)),
            "a ball off the top of his head was played as a boot"
        );
        assert!(soundtrack.spoken, "and it is not said twice");
    }

    /// ⚠ **The bug this design nearly shipped with.** A dribbler knocks the
    /// ball three metres ahead, so for most of every stride it is out of his
    /// reach and possession reads as nobody's. Taken at face value that is a
    /// pass out and then a pass in, per touch, for the whole run — the exact
    /// rattle the file exists to remove. The holder has to be sticky.
    #[test]
    fn a_knock_ahead_of_himself_is_not_a_pass_and_back() {
        // He is running with it and stays with it, as `keeps_it` sees.
        let mut tracks = recording(
            run((100.0, 30.0), (104.0, 30.0), 0.0),
            run((99.0, 30.0), (103.2, 30.0), 0.0),
        );
        let mut soundtrack = Soundtrack {
            holder: Some(7),
            carried: Vec3::new(5.0, 0.0, 0.0),
            ..default()
        };

        // Mid-knock: the ball is three metres in front of him, so nobody is
        // within reach of it.
        let loose = BallState {
            position: Vec3::new(0.0, 0.1, 0.0),
            on_pitch: true,
            nearest: Some((7, 3.0)),
            ..default()
        };
        assert!(
            soundtrack.possession(0.0, &loose, &mut tracks).is_none(),
            "his own knock was played as a pass out"
        );
        assert_eq!(soundtrack.holder, Some(7), "it is still his ball");

        // …and catching up with it is not receiving a pass.
        assert!(
            soundtrack
                .possession(0.0, &ball(0.5), &mut tracks)
                .is_none(),
            "catching his own knock was played as a reception"
        );
    }

    /// …while the ball actually leaving him, to a man who keeps it, is the
    /// pair of sounds a move is supposed to make.
    #[test]
    fn a_pass_is_heard_going_and_arriving() {
        let mut gone = recording(
            run((100.0, 30.0), (112.0, 30.0), 0.0),
            run((99.5, 30.0), (100.5, 30.0), 0.0),
        );
        let mut soundtrack = Soundtrack {
            holder: Some(7),
            carried: Vec3::new(12.0, 0.0, 0.0),
            ..default()
        };

        let away = BallState {
            position: Vec3::new(0.0, 0.1, 0.0),
            on_pitch: true,
            nearest: Some((7, 4.0)),
            ..default()
        };
        let out = soundtrack
            .possession(0.0, &away, &mut gone)
            .expect("the ball leaving him is a sound");
        assert!(matches!(out.meeting, Meeting::Struck(_)));
        assert!(soundtrack.spoken, "and it is not said twice");

        // Now a different man gets it and keeps it.
        let mut taken = recording(
            run((112.0, 30.0), (113.0, 30.0), 0.0),
            run((112.4, 30.0), (113.2, 30.0), 0.0),
        );
        let mut receiving = Soundtrack {
            holder: Some(3),
            carried: Vec3::new(9.0, 0.0, 0.0),
            ..default()
        };
        let arrival = receiving
            .possession(0.0, &ball(0.4), &mut taken)
            .expect("the ball arriving is a sound");
        assert!(matches!(arrival.meeting, Meeting::Received));
        assert_eq!(receiving.holder, Some(7));
    }

    /// A ball that merely rolls past a man's boots put him nearest for three
    /// frames. He never had it, so nothing arrived.
    #[test]
    fn a_ball_rolling_past_a_man_is_not_a_reception() {
        let mut through = recording(
            run((100.0, 30.0), (112.0, 30.0), 0.0),
            run((100.2, 30.0), (100.6, 30.0), 0.0),
        );
        let mut soundtrack = Soundtrack {
            holder: Some(3),
            carried: Vec3::new(12.0, 0.0, 0.0),
            ..default()
        };
        assert!(
            soundtrack
                .possession(0.0, &ball(0.6), &mut through)
                .is_none()
        );
    }

    /// The lookahead reports the same coming kick on every frame until the
    /// playhead reaches it, and the possession backstop sees the same pass
    /// from the other side. It has to be played once.
    #[test]
    fn one_pass_is_one_sound_however_many_paths_saw_it() {
        let mut soundtrack = Soundtrack::default();
        // The frames a strike 120 ms away is visible on, as the playhead
        // closes: the instant it lands on stays put, give or take a probe.
        let sightings = [(0.0, 120.0), (30.0, 90.0), (60.0, 62.0), (90.0, 30.0)];
        let mut played = 0;
        for (elapsed, delay) in sightings {
            if soundtrack.arm(elapsed + delay) {
                played += 1;
            }
        }
        assert_eq!(played, 1, "the same strike was played {played} times");
        // The backstop, firing when the ball actually clears him, is inside
        // the same window and must not add a second.
        assert!(!soundtrack.arm(120.0));
        // The next pass in the move is its own sound.
        assert!(soundtrack.arm(520.0));
    }

    /// A scrub throws the bookkeeping away rather than firing it, and adopts
    /// whoever has the ball where it landed — so the first frame after a jump
    /// is not heard as a reception.
    #[test]
    fn a_seek_adopts_the_world_without_making_a_noise() {
        let mut soundtrack = Soundtrack {
            struck: Some(1_000.0),
            spoken: true,
            holder: Some(3),
            netted: true,
            carried: Vec3::new(20.0, 0.0, 0.0),
            ..default()
        };
        soundtrack.resync(&ball(0.5));
        assert_eq!(soundtrack.struck, None);
        assert!(!soundtrack.spoken);
        assert_eq!(
            soundtrack.holder,
            Some(7),
            "whoever has it where we landed already has it"
        );
        assert!(!soundtrack.netted);
    }

    /// The pitch's length runs across the picture, and both ends have to land
    /// on opposite sides of it without ever reaching hard left or right.
    #[test]
    fn the_two_goals_are_the_two_sides_of_the_frame() {
        let left = Soundtrack::across(-Field::HALF_LENGTH);
        let right = Soundtrack::across(Field::HALF_LENGTH);
        assert!(left < 0.0 && right > 0.0);
        assert_eq!(left, -right);
        assert!(right < 0.75, "a broadcast mix is nearly mono");
        assert_eq!(Soundtrack::across(0.0), 0.0, "the centre spot is centred");
    }
}
