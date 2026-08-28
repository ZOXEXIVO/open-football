//! The instrument: an audio graph that can make the sound of a struck ball.
//!
//! Everything is synthesised — see [`crate::sound`] for why nothing is
//! fetched. The whole soundtrack comes out of one buffer of pink noise and a
//! handful of the browser's own filters, which is more than enough for the one
//! thing it has to make: pink noise IS the slap of a boot on a leather panel,
//! and a sine dropped an octave in fifty milliseconds is the ball's own mass
//! moving.

use crate::players::actors::Strike;
use bevy::prelude::warn;
use std::cell::Cell;
use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState, AudioScheduledSourceNode,
    BiquadFilterType, GainNode, OscillatorType, StereoPannerNode,
};

/// What just happened to the ball, as the instrument cares about it.
///
/// The two halves of a pass, and the only two things this crate makes a noise
/// for. Everything a player does to the ball while it stays his — the touches
/// that carry it up the pitch — is deliberately silent; see
/// [`Soundtrack`](super::matchday::Soundtrack), which is what decides.
#[derive(Clone, Copy)]
pub enum Meeting {
    /// **Sent away**: a pass, a clearance, a shot. What it was struck with
    /// carries the colour.
    Struck(Strike),
    /// **Taken under control**: a first touch, a trap, a chest-down. The ball
    /// is being STOPPED rather than sent, so there is no crack in it — a
    /// cushioned contact is mostly the low body of the ball dying, with just
    /// enough edge left to say a boot was involved. It has to be audibly the
    /// quieter of the two: over a match the ear should hear a pass go and a
    /// pass arrive, in that order, and never mistake which was which.
    Received,
}

/// What one contact with the ball sounds like.
///
/// Two layers, because a real one has two: a low **body** — the ball's own
/// mass moving, which is what you feel from the back of a stand — and a bright
/// **edge**, the slap of the striking surface on the panel, which is what
/// tells a shot from a pass. Held as data rather than as four near-identical
/// methods so the one interesting claim in here — that a shot is louder,
/// brighter and longer than a touch — is a table somebody can read and a test
/// can pin.
struct Knock {
    /// The body: a sine swept from `from` down to `to` hertz over `fall`
    /// seconds. A ball being hit is a pitch DROP, not a note; the drop is
    /// most of what makes it read as an impact rather than a beep.
    from: f32,
    to: f32,
    fall: f32,
    /// The edge: noise through a bandpass at `edge` hertz, gone in `snap`
    /// seconds. A wider `q` for the duller surfaces — a head is not a crack.
    edge: f32,
    q: f32,
    snap: f32,
    /// How loud the whole thing is.
    level: f32,
}

impl Knock {
    /// The four surfaces a footballer meets a ball with, as
    /// [`Strike`](crate::players::actors::Strike) already tells them apart.
    ///
    /// `weight` is 0..1 — how hard it was hit, taken off the speed the ball
    /// leaves at. It moves three things at once and that is the point: a
    /// harder strike is not simply a louder one, it is brighter and it cracks
    /// rather than thumps. Scaling gain alone gives a quiet shot and a loud
    /// shot that are obviously the same sound twice.
    ///
    /// **A shot has to arrive harder than a pass, and the levels alone will
    /// not do it.** Ramped straight off `weight`, the hardest strike in
    /// football came out about three decibels over an ordinary pass — and
    /// then the limiter, whose threshold both of them cleared, pulled that
    /// down to almost nothing. So two things carry it: the ramp below gets a
    /// second term that only opens above the pace a ball is struck AT GOAL
    /// with, and the whole table is scaled so that even the hardest one peaks
    /// just under the limiter and is never touched by it.
    ///
    /// Six decibels between a pass and a shot. Plainly the same sound —
    /// nothing about the timbre moves with it — and plainly harder.
    fn of(meeting: Meeting, weight: f32) -> Knock {
        // How much of this was a strike at goal rather than a ball played
        // somewhere, 0..1.
        let driven = Self::driven(weight);
        let kind = match meeting {
            Meeting::Struck(kind) => kind,
            // Cushioned: lower, longer and well down on anything struck. A
            // first touch takes the pace OFF the ball, and a reception that
            // cracks like a pass is the thing that would make a move sound
            // like it happened twice. No `driven` term at all — receiving a
            // rocket is not a hard contact, it is a man absorbing one.
            Meeting::Received => {
                return Knock {
                    from: 108.0,
                    to: 46.0,
                    fall: 0.075,
                    edge: 700.0,
                    q: 0.7,
                    snap: 0.030,
                    level: 0.16 + 0.10 * weight,
                };
            }
        };
        match kind {
            // The full range: a ball rolled five metres and a shot struck at
            // forty are both this, and they sound nothing alike.
            Strike::Boot => Knock {
                from: 140.0 + 95.0 * weight,
                to: 52.0,
                fall: 0.05 + 0.03 * weight,
                edge: 1300.0 + 2200.0 * weight,
                q: 0.9,
                snap: 0.028 + 0.032 * weight,
                level: 0.24 + 0.14 * weight + 0.24 * driven,
            },
            // Duller and lower than any boot: a head has no hard surface on
            // it, and the sound is mostly the ball rather than the contact.
            // It gets a share of the drive — a header at goal is struck — but
            // a smaller one, because a neck cannot do what a leg does.
            Strike::Head => Knock {
                from: 122.0,
                to: 60.0,
                fall: 0.08,
                edge: 640.0,
                q: 0.6,
                snap: 0.048,
                level: 0.20 + 0.12 * weight + 0.10 * driven,
            },
            // A goalkeeper's throw and a throw-in: hands, not boots. Well
            // down, and they have to be — a match has forty-odd throw-ins in
            // it and every one of them cracking like a shot is a match that
            // sounds wrong in a way nobody can name. Nobody shoots with a
            // throw, so there is no drive in it either.
            Strike::Throw | Strike::ThrowIn => Knock {
                from: 100.0,
                to: 64.0,
                fall: 0.06,
                edge: 940.0,
                q: 0.5,
                snap: 0.038,
                level: 0.11 + 0.06 * weight,
            },
        }
    }

    /// Where the pace of a strike stops being a ball played somewhere and
    /// starts being a ball hit at goal, as a share of
    /// [`Actors::HAMMERED`](crate::players::actors::Actors).
    ///
    /// 0.55 of the p90 is about 21 metres a second, which is above a driven
    /// cross and below anything anybody would call a shot.
    const DRIVEN: f32 = 0.55;

    /// How much of a strike is drive, 0..1 — smooth across
    /// [`Self::DRIVEN`] rather than switched at it.
    ///
    /// A step here would put a cliff in the middle of the one range the ear
    /// spends the match comparing: two balls struck a metre a second apart
    /// would land a full step in level apart, which reads as two different
    /// sounds rather than as two shots. Smoothstep has zero slope at both
    /// ends, so a hard pass grows into a shot instead of becoming one.
    fn driven(weight: f32) -> f32 {
        let over = ((weight - Self::DRIVEN) / (1.0 - Self::DRIVEN)).clamp(0.0, 1.0);
        over * over * (3.0 - 2.0 * over)
    }
}

/// The audio graph, and the browser handle it hangs off.
///
/// ```text
///   one-shots --> filter --> gain --> panner --> master --> ceiling --> out
/// ```
///
/// Flat and short on purpose. `ceiling` is a limiter and only a limiter: a
/// threshold near the top of the scale and a fast attack, so it catches the
/// instant three contacts land together and does nothing at all the rest of
/// the time.
///
/// The first version of this put a proper compressor here, under a crowd bed
/// that pinned it into permanent gain reduction — and every ball sound was
/// squashed by 12 dB before it reached the output. That is what "I do not
/// hear ball interactions" was. **Nothing sustained is allowed on this bus.**
pub struct Mixer {
    context: AudioContext,
    /// Everything, so mute and the pause duck are one number.
    master: GainNode,
    /// A few seconds of pink noise, shared by every one-shot in here.
    /// Generated once when the graph is built; see [`Mixer::weave`].
    noise: AudioBuffer,
    /// State for the little generator that keeps two one-shots from being
    /// bit-identical. `Cell` because everything in this file takes `&self`:
    /// the mixer is an instrument, and playing a note does not change it.
    seed: Cell<u32>,
    /// The last level sent to the master, so a setting that has not moved is
    /// not re-sent — [`Soundtrack`](super::matchday::Soundtrack) calls
    /// [`Self::listen`] on every frame, and each call would otherwise schedule
    /// an automation event on a parameter that has been at 1.0 for a minute.
    listened: Cell<f32>,
}

impl Mixer {
    /// Seconds of noise generated when the graph is built.
    ///
    /// Only ever read in short grains from a random offset, so this is a
    /// library rather than a loop: long enough that two touches in a row never
    /// draw the same slice, small enough to stay well under a megabyte.
    const NOISE_SECONDS: f32 = 2.5;

    /// How far ahead of the audio clock a sound may be scheduled at the
    /// earliest.
    ///
    /// `currentTime` advances in blocks of 128 samples, so an event scheduled
    /// at exactly `currentTime` is already in the past by the time the render
    /// quantum runs and its attack gets truncated — which on a three
    /// millisecond attack is most of the sound. A hair over two blocks is
    /// enough, and is far below anything anybody can hear as late.
    const LOOKAHEAD: f64 = 0.006;

    /// Opens the browser's audio engine and wires the graph up.
    ///
    /// `None` for any of the ways this can fail — no Web Audio at all, a
    /// context refused, a node that would not connect. Every one of them ends
    /// the same way: the replay plays silently, which is the only failure this
    /// crate is willing to have. Nothing about a football match needs sound to
    /// be legible, and a viewer is not owed a broken picture because a filter
    /// would not build.
    pub fn open() -> Option<Mixer> {
        let context = AudioContext::new()
            .inspect_err(|_| warn!("no Web Audio: the replay will be silent"))
            .ok()?;

        let out = context.destination();
        // A limiter, not a compressor — see the note on the struct.
        //
        // ⚠ **The threshold has to sit ABOVE the loudest single sound in the
        // match, not below it.** At -3 dB it was under the peak of a hard
        // shot AND under the peak of an ordinary pass, so it pulled the two
        // of them down to within a couple of decibels of each other — which
        // is a limiter doing exactly its job and destroying the one
        // distinction the mix is for. [`Knock`] is now scaled so the hardest
        // strike in football peaks just under this; what is left for the
        // limiter is the case it exists for, which is three contacts landing
        // in the same instant.
        let ceiling = context.create_dynamics_compressor().ok()?;
        ceiling.threshold().set_value(-1.0);
        ceiling.knee().set_value(1.0);
        ceiling.ratio().set_value(20.0);
        ceiling.attack().set_value(0.001);
        ceiling.release().set_value(0.10);
        ceiling.connect_with_audio_node(&out).ok()?;

        let master = context.create_gain().ok()?;
        // Silent until something asks to be heard — see [`Mixer::listen`]. The
        // graph is built before the first frame, and the first frame is not
        // necessarily one anybody wanted a noise on.
        master.gain().set_value(0.0);
        master.connect_with_audio_node(&ceiling).ok()?;

        let noise = Self::weave(&context)?;

        Some(Mixer {
            context,
            master,
            noise,
            seed: Cell::new(0x9E37_79B9),
            // Off the scale, so the first call is always sent however small
            // the value it carries.
            listened: Cell::new(f32::NAN),
        })
    }

    /// Generates the noise every one-shot is cut from.
    ///
    /// **Pink**, not white. White noise is a hiss — rain on a window. Pink has
    /// equal energy per octave, which is both what a large number of
    /// uncorrelated sources sounds like and, filtered to a band and given a
    /// twenty-millisecond decay, what a boot striking a leather panel sounds
    /// like. The three-pole filter below is the standard economical
    /// approximation to a -3 dB per octave tilt.
    ///
    /// No loop seam to worry about: nothing here loops. Every voice takes a
    /// short grain from a random offset and stops.
    fn weave(context: &AudioContext) -> Option<AudioBuffer> {
        let rate = context.sample_rate();
        let length = (rate * Self::NOISE_SECONDS) as usize;
        let buffer = context.create_buffer(1, length as u32, rate).ok()?;

        let mut noise = vec![0.0f32; length];
        let mut seed = 0x2545_F491u32;
        let (mut b0, mut b1, mut b2) = (0.0f32, 0.0f32, 0.0f32);
        for sample in noise.iter_mut() {
            seed = Self::churn(seed);
            let white = (seed >> 8) as f32 / 8_388_608.0 - 1.0;
            b0 = 0.99765 * b0 + white * 0.0990460;
            b1 = 0.96300 * b1 + white * 0.2965164;
            b2 = 0.57000 * b2 + white * 1.0526913;
            *sample = (b0 + b1 + b2 + white * 0.1848) * 0.30;
        }

        buffer.copy_to_channel(&noise, 0).ok()?;
        Some(buffer)
    }

    /// Wakes the browser's audio engine when it has gone to sleep.
    ///
    /// A context opened before anybody has touched the page starts
    /// `suspended`, and one that has been left alone can be suspended again by
    /// the browser at any time. Both come back with the same call, and it is
    /// cheap enough to make on every frame it is wanted — a resume on a
    /// running context resolves immediately.
    ///
    /// The returned promise is dropped: there is nothing to do on either
    /// outcome. Either it works and the next frame is audible, or it does not,
    /// because the viewer has not interacted with the page yet — and the frame
    /// after the one where they do will be.
    pub fn wake(&self) {
        if self.context.state() == AudioContextState::Suspended {
            let _ = self.context.resume();
        }
    }

    /// How much of all this reaches the viewer, 0..1.
    ///
    /// Ramped rather than set: mute, unmute and the duck when the replay is
    /// paused all come through here, and a gain that jumps is a click at the
    /// exact moment somebody asked for silence.
    pub fn listen(&self, level: f32) {
        let level = level.clamp(0.0, 1.0);
        if (self.listened.get() - level).abs() < 0.02 {
            return;
        }
        self.listened.set(level);
        let now = self.context.current_time();
        let _ = self.master.gain().set_target_at_time(level, now, 0.08);
    }

    /// One contact with the ball: a touch, a pass, a header, a shot.
    ///
    /// `weight` is 0..1 off the speed it leaves at, `pan` is -1..1 across the
    /// picture and `delay` is seconds from now — the caller often knows a
    /// strike is coming before it lands (the recording is all in memory; see
    /// [`Contact`](crate::players::actors::Contact)), so the sound is
    /// SCHEDULED on the audio clock rather than fired on a frame. That is
    /// worth the trouble: an impact placed by the renderer lands up to a frame
    /// late and, worse, jitters, and the ear reads jitter on a percussive
    /// sound as a different sound.
    pub fn touch(&self, meeting: Meeting, weight: f32, pan: f32, delay: f32) {
        let knock = Knock::of(meeting, weight.clamp(0.0, 1.0));
        let at = self.context.current_time() + (delay.max(0.0) as f64).max(Self::LOOKAHEAD);
        let Some(panner) = self.aim(pan) else {
            return;
        };

        // The body. A sine rather than anything richer: the harmonics of a
        // struck ball live in the edge layer below, and a sine sweep is the
        // cleanest thump there is at this length.
        if let (Ok(thump), Ok(gain)) =
            (self.context.create_oscillator(), self.context.create_gain())
        {
            thump.set_type(OscillatorType::Sine);
            let hz = thump.frequency();
            let _ = hz.set_value_at_time(knock.from, at);
            let _ = hz.exponential_ramp_to_value_at_time(knock.to, at + knock.fall as f64);
            let param = gain.gain();
            let _ = param.set_value_at_time(0.0001, at);
            let _ = param.linear_ramp_to_value_at_time(knock.level, at + 0.003);
            let _ = param.exponential_ramp_to_value_at_time(0.0001, at + knock.fall as f64 * 1.8);
            if thump.connect_with_audio_node(&gain).is_ok()
                && gain.connect_with_audio_node(&panner).is_ok()
            {
                let _ = thump.start_with_when(at);
                let _ = thump.stop_with_when(at + knock.fall as f64 * 2.0 + 0.02);
            }
        }

        // The edge: the slap off the panel, and the layer that carries which
        // kind of contact this was.
        if let (Some((source, gain)), Ok(edge)) = (self.cut(), self.context.create_biquad_filter())
        {
            edge.set_type(BiquadFilterType::Bandpass);
            edge.frequency().set_value(knock.edge);
            edge.q().set_value(knock.q);
            let param = gain.gain();
            let _ = param.set_value_at_time(0.0001, at);
            let _ = param.linear_ramp_to_value_at_time(knock.level * 0.9, at + 0.0015);
            let _ = param.exponential_ramp_to_value_at_time(0.0001, at + knock.snap as f64);
            if source.connect_with_audio_node(&edge).is_ok()
                && edge.connect_with_audio_node(&gain).is_ok()
                && gain.connect_with_audio_node(&panner).is_ok()
            {
                let _ = source.start_with_when_and_grain_offset(at, self.somewhere());
                let _ = AudioScheduledSourceNode::stop_with_when(
                    &source,
                    at + knock.snap as f64 + 0.06,
                );
            }
        }
    }

    /// The ball going into the netting: a soft, spread-out rustle rather than
    /// an impact — the half second of texture that says the ball is in rather
    /// than past the post.
    pub fn net(&self, pan: f32) {
        let now = self.context.current_time() + Self::LOOKAHEAD;
        let Some(panner) = self.aim(pan) else {
            return;
        };
        let Some((source, gain)) = self.cut() else {
            return;
        };
        let Ok(mesh) = self.context.create_biquad_filter() else {
            return;
        };
        mesh.set_type(BiquadFilterType::Bandpass);
        mesh.frequency().set_value(3000.0);
        mesh.q().set_value(0.5);

        let param = gain.gain();
        let _ = param.set_value_at_time(0.0001, now);
        let _ = param.linear_ramp_to_value_at_time(0.45, now + 0.012);
        let _ = param.exponential_ramp_to_value_at_time(0.0001, now + 0.34);
        if source.connect_with_audio_node(&mesh).is_ok()
            && mesh.connect_with_audio_node(&gain).is_ok()
            && gain.connect_with_audio_node(&panner).is_ok()
        {
            let _ = source.start_with_when_and_grain_offset(now, self.somewhere());
            let _ = AudioScheduledSourceNode::stop_with_when(&source, now + 0.40);
        }
    }

    /// A one-shot tap off the noise buffer, and the gain it plays into.
    ///
    /// Neither end is connected to anything: every caller has its own filter
    /// to put between them and its own idea of where the result goes.
    fn cut(&self) -> Option<(AudioBufferSourceNode, GainNode)> {
        let source = self.context.create_buffer_source().ok()?;
        source.set_buffer(Some(&self.noise));
        let gain = self.context.create_gain().ok()?;
        gain.gain().set_value(0.0001);
        Some((source, gain))
    }

    /// A panner already wired into the master.
    ///
    /// The main camera sits on a touchline (see
    /// [`TvCamera`](crate::broadcast::camera::TvCamera)), so the pitch's
    /// LENGTH runs across the picture and a kick at one end really is off to
    /// one side. Held well short of hard left and right by the caller: a
    /// broadcast mix is nearly mono, and a fully panned thump reads as a fault
    /// in the headphones rather than as a position on the pitch.
    fn aim(&self, pan: f32) -> Option<StereoPannerNode> {
        let panner = self.context.create_stereo_panner().ok()?;
        panner.pan().set_value(pan.clamp(-1.0, 1.0));
        panner.connect_with_audio_node(&self.master).ok()?;
        Some(panner)
    }

    /// Somewhere in the noise buffer to start a one-shot from, so two touches
    /// in a row are not the same sample twice. Seconds, and always a clear
    /// half-second short of the end so the longest grain still has buffer
    /// under it.
    fn somewhere(&self) -> f64 {
        let next = Self::churn(self.seed.get());
        self.seed.set(next);
        (next >> 8) as f64 / 16_777_216.0 * (Self::NOISE_SECONDS as f64 - 0.5)
    }

    /// xorshift32. The crate has no random number generator and wants one in
    /// exactly two places, both of them here — the noise itself and the offset
    /// a grain is cut from — and neither is asking for statistical quality.
    fn churn(seed: u32) -> u32 {
        let mut seed = seed;
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    }
}

/// The one claim in this file that is about football rather than about audio:
/// that the four surfaces sound different, and that a shot and a touch off the
/// same boot do too.
#[cfg(test)]
mod knocks {
    use super::*;

    /// A shot is not a loud pass. It is louder, and it is brighter, and it
    /// rings for longer — scaling gain alone is what makes a synthesised
    /// impact sound synthesised.
    #[test]
    fn a_shot_is_brighter_than_a_touch_and_not_merely_louder() {
        let touch = Knock::of(Meeting::Struck(Strike::Boot), 0.0);
        let shot = Knock::of(Meeting::Struck(Strike::Boot), 1.0);
        assert!(shot.level > touch.level, "a shot is louder");
        assert!(shot.edge > touch.edge * 2.0, "a shot cracks");
        assert!(shot.snap > touch.snap, "and it rings on");
    }

    /// **The softest touch in the match still has to arrive.** A five-metre
    /// square ball is most of what happens in football, and the first cut of
    /// this put it far enough down that it could not be heard at all.
    ///
    /// An ABSOLUTE floor, deliberately, and not a share of the loudest thing
    /// in the mix. It was written as a ratio while a crowd was masking
    /// everything; there is no crowd now, so what decides whether a gentle
    /// pass can be heard is its own level against silence — and holding it as
    /// a ratio would have blocked the whole point of
    /// [`Knock::driven`](super::Knock), which is to open a gap above it.
    #[test]
    fn the_gentlest_pass_is_still_well_up_in_the_mix() {
        let softest = Knock::of(Meeting::Struck(Strike::Boot), 0.0);
        assert!(
            softest.level > 0.20,
            "a pass at {} is inaudible against silence",
            softest.level
        );
    }

    /// **A shot has to land harder than a pass.** Six decibels — a doubling
    /// of amplitude — which is plainly more without being a different sound.
    ///
    /// The weights are real: `weight` is the ball's speed over
    /// [`Actors::HAMMERED`](crate::players::actors::Actors), so 0.37 is a
    /// fourteen-metre-a-second pass and 1.0 is the hardest anybody strikes
    /// one.
    #[test]
    fn a_shot_lands_about_six_decibels_over_a_pass() {
        let pass = Knock::of(Meeting::Struck(Strike::Boot), 0.37).level;
        let shot = Knock::of(Meeting::Struck(Strike::Boot), 1.0).level;
        let decibels = 20.0 * (shot / pass).log10();
        assert!(
            (5.0..7.5).contains(&decibels),
            "a shot is {decibels:.1} dB over a pass, wanted about six"
        );
    }

    /// …and it has to stay under the limiter, or the limiter takes the six
    /// decibels straight back off again — which is what it was doing.
    ///
    /// The two layers of a contact overlap at the front, so the peak that
    /// reaches the bus is worth about half again the table's figure. The
    /// ceiling is at -1 dBFS.
    #[test]
    fn the_hardest_strike_in_football_still_clears_the_ceiling() {
        let hardest = Knock::of(Meeting::Struck(Strike::Boot), 1.0);
        let peak = hardest.level * 1.45;
        assert!(peak < 0.95, "a shot peaks at {peak} and would be squashed");
    }

    /// The rise into a shot is a curve, not a step. Two balls struck a metre
    /// a second apart must not land a whole step in level apart.
    #[test]
    fn the_drive_grows_in_rather_than_switching_on() {
        assert_eq!(
            Knock::driven(Knock::DRIVEN),
            0.0,
            "nothing at the threshold"
        );
        assert_eq!(Knock::driven(1.0), 1.0, "and all of it at the top");
        // Either side of the threshold, and either side of the top, the curve
        // is flat — which is what stops it reading as two sounds.
        let just_over = Knock::driven(Knock::DRIVEN + 0.02);
        assert!(just_over < 0.02, "a cliff at the threshold: {just_over}");
        assert!(Knock::driven(0.98) > 0.99, "and one at the top");
        // …and monotone in between.
        let mut last = 0.0;
        for step in 0..=20 {
            let now = Knock::driven(step as f32 / 20.0);
            assert!(now >= last, "the drive went backwards at {step}");
            last = now;
        }
    }

    /// **The two halves of a pass have to be told apart by ear.** A move is
    /// heard as a ball going and a ball arriving, and the arrival is the
    /// quieter, duller of the two — it is the pace coming OFF the ball.
    #[test]
    fn a_reception_is_the_quieter_half_of_a_pass() {
        let sent = Knock::of(Meeting::Struck(Strike::Boot), 0.35);
        let taken = Knock::of(Meeting::Received, 0.35);
        assert!(
            taken.level < sent.level,
            "the arrival is under the departure"
        );
        assert!(taken.edge < sent.edge * 0.7, "and duller: nothing cracks");
        assert!(taken.fall > sent.fall, "the ball dies rather than leaves");
    }

    /// A header has no hard surface in it, so it must not be allowed to crack
    /// like a boot however hard the ball was moving afterwards.
    #[test]
    fn a_head_never_cracks() {
        let header = Knock::of(Meeting::Struck(Strike::Head), 1.0);
        let volley = Knock::of(Meeting::Struck(Strike::Boot), 1.0);
        assert!(header.edge < volley.edge * 0.5);
        assert!(header.level < volley.level);
    }

    /// A match has forty-odd throw-ins in it and they are the one contact
    /// nobody in the ground reacts to. Quieter than anything else, at every
    /// weight, or the mix fills up with them.
    #[test]
    fn a_throw_in_stays_out_of_the_way() {
        let loudest_throw = Knock::of(Meeting::Struck(Strike::ThrowIn), 1.0);
        let softest_boot = Knock::of(Meeting::Struck(Strike::Boot), 0.0);
        assert!(loudest_throw.level < softest_boot.level);
        assert_eq!(
            Knock::of(Meeting::Struck(Strike::Throw), 0.5).level,
            Knock::of(Meeting::Struck(Strike::ThrowIn), 0.5).level,
            "both are hands, and sound like it"
        );
    }
}
