//! The player: what the ball does, turned into what it sounds like.
//!
//! Nothing new is fetched and nothing new is recorded. Both signals come off
//! facts the viewer was already drawing from:
//!
//! - **the ball being struck** — [`BallState::impact`], which the animation
//!   rig already reads AHEAD of the playhead so a footballer can take a
//!   backswing. The same lookahead is what lets a contact be scheduled on the
//!   audio clock instead of fired on a frame.
//! - **the ball in the netting** — [`Netting::inside_a_goal`], the one place
//!   in the crate that owns what "in the goal" means, borrowed rather than
//!   re-derived.
//!
//! # Two detectors, and why the second one has to exist
//!
//! [`Actors::next_impact`](crate::players::actors::Actors) is built to decide
//! whether to swing a leg, and it is deliberately strict: the ball has to
//! leave above 4.5 m/s AND at 1.6 times the speed it arrived at. That is the
//! right test for an animation — a man does not wind up to stroke a ball five
//! metres sideways — and the wrong one for sound, because **that stroke is
//! most of what happens in a football match** and it is silent under those
//! gates.
//!
//! So [`Soundtrack::brushed`] runs alongside it with the physical test
//! instead: the ball's momentum changed while somebody was within reach of it.
//! Both feed one dedupe, so a contact loud enough for the rig to see is played
//! once and not twice.

use crate::players::actors::{Actors, BallState, Strike};
use crate::recording::playback::Playback;
use crate::scene::field::Field;
use crate::scene::net::Netting;
use crate::sound::mixer::Mixer;
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

/// What the soundtrack has already played, so that it does not play it again.
#[derive(Resource, Default)]
pub struct Soundtrack {
    /// Whether the viewer has asked for quiet. Owned here rather than in the
    /// transport bar because the bar is a view of it: the chip reads this and
    /// writes it, and everything else only reads.
    pub muted: bool,
    /// Match time of the contact already handed to the mixer, so a strike
    /// that stays visible in the lookahead for several frames is played once.
    /// `None` when nothing is armed.
    struck: Option<f64>,
    /// Whether the ball was inside a goal on the previous frame.
    netted: bool,
    /// What the ball was doing on the previous frame. The soft-touch detector
    /// is a test on how much this changed — see [`Soundtrack::brushed`].
    carried: Vec3,
}

impl Soundtrack {
    /// Master level while the replay is running and nobody has asked for
    /// quiet. **This is the volume knob**: every level in the mix is set
    /// relative to a shot, so turning the whole soundtrack up or down is this
    /// number and nothing else.
    const LEVEL: f32 = 1.0;

    /// How far apart in match time two contacts have to be to be two
    /// contacts.
    ///
    /// The lookahead re-derives the coming strike every frame off a 30 ms
    /// probe (see [`Actors::next_impact`](crate::players::actors::Actors)), so
    /// the instant it reports wobbles by up to a probe's width while the
    /// playhead closes on it. Anything inside this window is taken to be the
    /// same strike seen again — which is also what keeps the two detectors
    /// from playing one touch twice, and stops a scramble in a six-yard box
    /// from machine-gunning.
    const REARM_MS: f64 = 120.0;

    /// The smallest change in the ball's motion that is worth a sound, in
    /// metres per second.
    ///
    /// Low on purpose: this is the number that decides whether a five-metre
    /// square ball is audible, and that pass is the most common event in a
    /// football match. Set below the softest thing a player does to a ball on
    /// purpose and above the noise in a recorded path.
    const NUDGE: f32 = 1.6;

    /// …and the same floor expressed as an acceleration, for the frames where
    /// that is the tighter test.
    ///
    /// Gravity changes a ball by 9.81 m/s every second, so a single frame at
    /// 16x covers a quarter of that and would trip [`Self::NUDGE`] on a ball
    /// nobody has gone near. Two and a half g is comfortably above anything
    /// gravity and drag can do together and far below what a boot does.
    const GRAVITY_MARGIN: f32 = 24.5;

    /// How far a contact at the goal line is panned. A broadcast mix is
    /// nearly mono; this is enough to place a kick and not enough to notice
    /// it being placed.
    const SPREAD: f32 = 0.55;

    /// Once a frame: works out what the ball has just had done to it and tells
    /// the mixer.
    ///
    /// Runs behind [`Actors::follow_playhead`], which is what settles
    /// [`BallState`], and ahead of [`Playback::end_frame`], which clears the
    /// `seeked` flag this reads.
    pub fn follow_playhead(
        playback: Res<Playback>,
        ball: Res<BallState>,
        time: Res<Time>,
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
        // now forty minutes away, and the ball's velocity across the jump is
        // not a velocity at all.
        if playback.seeked {
            soundtrack.resync(&ball);
            return;
        }

        // Seconds of MATCH time this frame covered, which is the clock the
        // ball's own velocity is measured on.
        let step = (time.delta_secs() * playback.speed).max(1e-4);
        soundtrack.contact(&playback, &ball, step, mixer);
        soundtrack.netting(&ball, mixer);
        soundtrack.carried = ball.velocity;
    }

    /// Both detectors, into one dedupe.
    fn contact(&mut self, playback: &Playback, ball: &BallState, step: f32, mixer: &Mixer) {
        // The rig's own, read AHEAD of the playhead. `delay` is seconds of
        // MATCH time until contact and the audio clock runs in real seconds,
        // so it is divided by the playback speed on the way across: at 8x a
        // kick a tenth of a second away is twelve milliseconds away, and
        // handing the mixer the match figure would put every impact further
        // behind the picture the faster the replay ran.
        if let Some(impact) = ball.impact {
            let contact = impact.contact;
            let when = playback.time_ms + (contact.delay as f64) * 1000.0;
            if self.arm(when) {
                mixer.touch(
                    contact.kind,
                    Self::weight(contact.velocity.length()),
                    Self::across(contact.at.x),
                    contact.delay / playback.speed.max(0.01),
                );
            }
        }

        // …and the ones it is built to ignore, which happen now rather than
        // in a hundred milliseconds.
        if let Some((kind, change)) = self.brushed(ball, step)
            && self.arm(playback.time_ms)
        {
            mixer.touch(
                kind,
                Self::weight(change),
                Self::across(ball.position.x),
                0.0,
            );
        }
    }

    /// **A touch the rig's own detector is built to ignore.**
    ///
    /// The test is the physical one: the ball's momentum changed, and
    /// somebody was within reach of it when it did. Nothing else on a football
    /// pitch can change it that fast — gravity and drag are smooth, and the
    /// proximity gate is what keeps a bounce off the turf out of the mix.
    ///
    /// Returns what it was struck with and by how much the motion changed, in
    /// metres per second, which is the same currency
    /// [`Self::weight`] reads a recorded strike in.
    fn brushed(&self, ball: &BallState, step: f32) -> Option<(Strike, f32)> {
        // In the gloves. Whatever happens to the ball now is the keeper
        // carrying it, and `BallState` is displacing it to his hands anyway.
        if ball.held_by.is_some() || !ball.on_pitch {
            return None;
        }

        // ⚠ **The engine PUTS the ball down for a restart, a catch or a block
        // rather than moving it there**, and `Actors` blanks the velocity to
        // exactly zero when it sees that jump — see
        // [`Track::teleported`](crate::recording::replay::Track). Measured
        // across the blanking, a ball flying at twenty metres a second looks
        // like a twenty-metre-a-second contact, and there is a player standing
        // over the ball at most restarts. So the frame the velocity is thrown
        // away is not a frame anything can be read from.
        //
        // The cost is a ball trapped stone dead out of a fast pass, which
        // loses its touch. The rig misses that one too — it is a DROP in
        // speed, and `next_impact` only looks for a rise.
        if ball.velocity == Vec3::ZERO && self.carried.length() > Self::NUDGE {
            return None;
        }

        let (_, range) = ball.nearest?;
        if range > Actors::STRIKE_REACH {
            return None;
        }

        let change = (ball.velocity - self.carried).length();
        // Whichever of the two floors is higher wins, so this stays a test of
        // "something hit it" at every playback speed. See
        // [`Self::GRAVITY_MARGIN`].
        if change < Self::NUDGE.max(Self::GRAVITY_MARGIN * step) {
            return None;
        }

        // **A bounce off the turf is not a touch**, and it is otherwise the
        // biggest false positive here: a ball dropping onto the grass beside a
        // player reverses eight metres a second of vertical in one frame,
        // which sails past every test above.
        //
        // What separates the two is the ground plane. Turf takes the vertical
        // and hands back most of it; the two horizontal components come
        // through very nearly unaltered. Nothing a footballer does to a ball
        // leaves them alone — even a ball trapped dead straight down loses its
        // run. So a change that is ALL vertical, low enough to be the pitch,
        // is the pitch.
        let flat = Vec2::new(
            ball.velocity.x - self.carried.x,
            ball.velocity.z - self.carried.z,
        )
        .length();
        if ball.position.y < Actors::BALL_RADIUS * 2.0 && flat < Self::NUDGE * 0.5 {
            return None;
        }

        // Read off the same geometry the rig reads it off, rather than a
        // second opinion about what a header is: `before` is how fast the ball
        // was going into the contact, which is what separates a throw-in from
        // a pass down the line.
        Some((
            Actors::strike_kind(ball.position, self.carried.length()),
            change,
        ))
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
    /// so. The single gate both detectors go through.
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
        self.netted = ball.on_pitch && Netting::inside_a_goal(ball.position);
        self.carried = ball.velocity;
    }
}

/// The rules here that are about a football match rather than about audio:
/// what counts as a touch, and what counts as the same touch twice.
#[cfg(test)]
mod touches {
    use super::*;

    /// A ball on the deck doing `now`, with the nearest man `range` away.
    fn ball(now: Vec3, range: f32) -> BallState {
        BallState {
            position: Vec3::new(0.0, 0.2, 0.0),
            on_pitch: true,
            velocity: now,
            nearest: Some((7, range)),
            ..default()
        }
    }

    /// A soundtrack that watched the ball doing `was` on the previous frame —
    /// which is where the detector's other half lives.
    fn after(was: Vec3) -> Soundtrack {
        Soundtrack {
            carried: was,
            ..default()
        }
    }

    /// **The whole reason this detector exists.** A five-metre square ball off
    /// a ball that was already rolling fails both of the animation rig's gates
    /// — it leaves under 4.5 m/s and nowhere near 1.6x what it arrived at —
    /// and it is the most common event in a football match.
    #[test]
    fn a_small_pass_off_a_rolling_ball_is_a_touch() {
        // Rolling at 3 m/s, played square at 4 m/s across it.
        let was = Vec3::new(3.0, 0.0, 0.0);
        let now = Vec3::new(0.0, 0.0, 4.0);
        let soundtrack = after(was);
        let struck = soundtrack.brushed(&ball(now, 0.4), 1.0 / 60.0);
        assert!(
            struck.is_some(),
            "the commonest pass in football was silent"
        );
        // …and it is quiet, which is the other half of being right about it.
        let (kind, change) = struck.expect("a touch");
        assert!(
            matches!(kind, Strike::Boot),
            "a square ball is struck with a boot"
        );
        assert!(
            Soundtrack::weight(change) < 0.25,
            "a square ball is not a shot"
        );
    }

    /// A ball nobody is near is doing whatever the air and the turf are doing
    /// to it, and none of that is a contact.
    #[test]
    fn nothing_within_reach_is_nothing_at_all() {
        let was = Vec3::new(10.0, 2.0, 0.0);
        let now = Vec3::new(10.0, -6.0, 0.0);
        assert!(
            after(was)
                .brushed(&ball(now, Actors::STRIKE_REACH + 0.5), 1.0 / 60.0)
                .is_none(),
            "a bounce with nobody near it was played as a touch"
        );
    }

    /// Gravity must not become a touch at speed. A frame at 16x covers a
    /// quarter of a second, over which gravity alone changes the ball by
    /// 2.5 m/s — comfortably past the standing floor.
    #[test]
    fn gravity_is_never_a_touch_however_fast_the_replay_runs() {
        let step = 0.25; // one frame at 16x
        let was = Vec3::new(6.0, 4.0, 0.0);
        let now = was - Vec3::new(0.0, 9.81 * step, 0.0);
        assert!(
            after(was).brushed(&ball(now, 0.5), step).is_none(),
            "a falling ball was played as a touch at 16x"
        );
        // The same fall at 1x is far below the floor as well.
        let slow = 1.0 / 60.0;
        let dropped = was - Vec3::new(0.0, 9.81 * slow, 0.0);
        assert!(after(was).brushed(&ball(dropped, 0.5), slow).is_none());
    }

    /// …and a real boot at that same speed still is one.
    #[test]
    fn a_strike_still_lands_at_sixteen_times_speed() {
        let was = Vec3::new(2.0, 0.0, 0.0);
        let now = Vec3::new(-18.0, 3.0, 0.0);
        assert!(after(was).brushed(&ball(now, 0.3), 0.25).is_some());
    }

    /// The turf takes the vertical and hands most of it back, and leaves the
    /// run alone. A player standing over a bouncing ball is common, and this
    /// is otherwise the loudest thing in the mix that never happened.
    #[test]
    fn a_bounce_off_the_grass_is_not_a_touch() {
        let was = Vec3::new(4.0, -8.5, 1.0);
        let bounced = Vec3::new(4.0, 6.0, 1.0);
        let mut on_the_deck = ball(bounced, 0.9);
        on_the_deck.position.y = Actors::BALL_RADIUS;
        assert!(
            after(was).brushed(&on_the_deck, 1.0 / 60.0).is_none(),
            "the pitch was played as a pass"
        );

        // …but a volley met at the same instant is a contact, because a boot
        // takes the run off it as well as the drop.
        let struck = Vec3::new(-9.0, 6.0, 1.0);
        let mut met = ball(struck, 0.9);
        met.position.y = Actors::BALL_RADIUS;
        assert!(after(was).brushed(&met, 1.0 / 60.0).is_some());
    }

    /// …and the same bounce up in the air — off a head, off a knee — is a
    /// contact, because nothing up there is the pitch.
    #[test]
    fn a_ball_turned_over_above_the_grass_is_a_contact() {
        let was = Vec3::new(4.0, -8.5, 1.0);
        let now = Vec3::new(4.0, 6.0, 1.0);
        let mut airborne = ball(now, 0.9);
        airborne.position.y = 1.6;
        assert!(after(was).brushed(&airborne, 1.0 / 60.0).is_some());
    }

    /// ⚠ The engine PLACES the ball for a restart and the velocity is blanked
    /// to zero when it does. There is a player standing over the ball at most
    /// restarts, so read as a contact this is a goal kick that cracks like a
    /// volley on the frame the ball is put down.
    #[test]
    fn a_placement_is_not_a_contact() {
        let flying = Vec3::new(0.0, -14.0, 6.0);
        let placed = Vec3::ZERO;
        assert!(
            after(flying)
                .brushed(&ball(placed, 0.6), 1.0 / 60.0)
                .is_none(),
            "the engine putting the ball down was played as a strike"
        );
    }

    /// A ball in a goalkeeper's hands is being carried, not struck.
    #[test]
    fn a_ball_in_the_gloves_is_quiet() {
        let was = Vec3::new(9.0, 0.0, 0.0);
        let mut held = ball(Vec3::new(1.0, 0.0, 3.0), 0.2);
        held.held_by = Some(1);
        assert!(after(was).brushed(&held, 1.0 / 60.0).is_none());
    }

    /// The lookahead reports the same coming kick on every frame until the
    /// playhead reaches it, and the soft detector sees the same contact from
    /// the other side. It has to be played once.
    #[test]
    fn one_contact_is_one_sound_however_many_detectors_saw_it() {
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
        // The soft detector, firing at the moment the ball is actually hit,
        // is inside the same window and must not add a second.
        assert!(!soundtrack.arm(120.0), "both detectors played one touch");
        // The next touch in the move is its own sound.
        assert!(soundtrack.arm(520.0));
    }

    /// A scrub throws the bookkeeping away rather than firing it: the velocity
    /// across a jump is not a velocity, and the strike that was armed is now
    /// somewhere else in the match.
    #[test]
    fn a_seek_adopts_the_world_without_making_a_noise() {
        let mut soundtrack = Soundtrack {
            struck: Some(1_000.0),
            carried: Vec3::new(20.0, 0.0, 0.0),
            netted: true,
            ..default()
        };
        let landed = BallState {
            position: Vec3::new(0.0, 0.1, 0.0),
            on_pitch: true,
            velocity: Vec3::new(2.0, 0.0, 0.0),
            ..default()
        };
        soundtrack.resync(&landed);
        assert_eq!(soundtrack.struck, None);
        assert_eq!(soundtrack.carried, landed.velocity);
        assert!(
            !soundtrack.netted,
            "the ball is not in a goal at the centre spot"
        );
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
