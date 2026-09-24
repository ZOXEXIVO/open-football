//! **A strike as the ball sees it**: the instant the boot meets it, to the
//! millisecond; one swing for one strike, however many samples the recording
//! spreads its departure over; and the ball drawn onto the boot or the head
//! that is striking it rather than wherever the engine let him reach it from.
use super::*;

/// **A boot or a head on the ball**: where, how far the ball has been brought
/// onto it, and when the contact is — seconds ahead, negative once made.
#[derive(Clone, Copy)]
pub(super) struct Claim {
    pub by: u32,
    pub point: Vec3,
    pub approach: f32,
    pub due: f32,
    /// A strike meets the ball wherever it is, a man's feet included, and
    /// at whatever height it is drawn at; a trap takes one that is
    /// travelling to it, onto the boot.
    pub strikes: bool,
}

impl Claim {
    pub fn placed(self, transform: &Transform) -> Claim {
        Claim {
            point: transform.transform_point(self.point),
            ..self
        }
    }
}

/// **The contacts the ball is between**: the latest one made, which it is
/// leaving, and the soonest still to come, which it is going to. Given to
/// whichever claim was nearest its own contact, a pass played a metre into
/// another man's path was fought over by the two boots — the ball drawn at
/// one and then the other, two metres apart, frame by frame.
#[derive(Default)]
pub(super) struct Claims {
    left: Option<Claim>,
    next: Option<Claim>,
}

impl Claims {
    pub fn offer(&mut self, claim: Claim) {
        if claim.due <= 0.0 {
            if self.left.is_none_or(|left| claim.due > left.due) {
                self.left = Some(claim);
            }
        } else if self.next.is_none_or(|next| claim.due < next.due) {
            self.next = Some(claim);
        }
    }

    /// **Where the ball is drawn**, from `loose` — where it is with nobody's
    /// boot on it: off the contact it has left and onto the next over the
    /// time between the two, so it is on each boot at that boot's contact
    /// and travels between them. `dribbled` is whose feet it is at and how
    /// far: a trap has to wait for it to be played from there, a strike
    /// takes it off them. `held` is how far it is in somebody's hands, which
    /// no boot takes it out of — it comes to one as the hands let it go.
    pub fn drawn(&self, loose: Vec3, dribbled: Option<(u32, f32)>, held: f32) -> Vec3 {
        let pull = self.next.map_or(0.0, |next| match dribbled {
            Some((by, lead)) if by != next.by && !next.strikes => next.approach * (1.0 - lead),
            _ => next.approach,
        });
        let travelled = match (self.left, self.next) {
            (Some(left), Some(next)) => (-left.due / (next.due - left.due)).min(pull),
            _ => 0.0,
        };
        let from = self
            .left
            .map_or(0.0, |left| left.approach.min(1.0 - travelled));
        let to = pull.min(1.0 - from);
        let onto = |claim: Option<Claim>, weight: f32| {
            claim.map_or(Vec3::ZERO, |claim| {
                // A ball the engine carries at chest height through a
                // dribble is drawn at his feet, and a boot meets it there
                // rather than lifting it a metre onto the swing.
                let point = if claim.strikes {
                    Vec3::new(claim.point.x, loose.y, claim.point.z)
                } else {
                    claim.point
                };
                (point - loose) * weight
            })
        };
        loose + (onto(self.left, from) + onto(self.next, to)) * (1.0 - held)
    }
}

impl Strike {
    /// Seconds a swing of this kind takes to come on — see [`Kick::blend`].
    pub(super) fn onset(self) -> f32 {
        match self {
            Strike::Throw | Strike::ThrowIn => Actors::WIND_UP,
            _ => Actors::KICK_ONSET,
        }
    }
}

impl Actors {
    /// Milliseconds of the ball's track a speed is read over while the
    /// instant of a strike is being found.
    const SLICE: f64 = 4.0;
    /// Seconds after a contact inside which another jump in the ball's speed
    /// is the same strike still leaving the boot: a ball struck hard leaves
    /// it over two or three samples of the recording, each of which reads as
    /// a strike of its own, and each of which snapped the leg back into a
    /// backswing.
    const REARM: f32 = 0.25;
    /// …and inside which two reads of a contact are the same contact.
    const SAME_CONTACT: f32 = 0.02;
    /// How much of the follow-through the ball takes to come off the boot
    /// back onto its own flight. A trapped ball comes off over the whole
    /// settle.
    const LEAVES_BOOT: f32 = 0.35;
    /// How fast a swing under way may run backwards and forwards to meet a
    /// fresh read of its contact, as multiples of its own countdown.
    const RETIME: (f32, f32) = (0.5, 4.0);

    /// **The instant the ball was struck**, in ms of match clock: the kink in
    /// its own track between `calm`, where a probe saw it still, and `going`,
    /// where one saw it gone. The probes are thirty milliseconds apart, and
    /// a swing timed off them jumped a fifth of the way through every other
    /// frame.
    pub(super) fn struck_at(ball: &mut Track, calm: f64, going: f64) -> Option<f64> {
        let speed = |ball: &mut Track, t: f64| -> Option<f32> {
            let [x, y, z] = ball.position_ahead(t)?;
            let [u, v, w] = ball.position_ahead(t + Self::SLICE)?;
            Some(
                (Field::to_world(u, v, w) - Field::to_world(x, y, z)).length()
                    / (Self::SLICE as f32 / 1000.0),
            )
        };
        let threshold = (speed(ball, calm)? + speed(ball, going - Self::SLICE)?) * 0.5;
        let (mut calm, mut going) = (calm, going - Self::SLICE);
        for _ in 0..7 {
            let middle = (calm + going) * 0.5;
            if speed(ball, middle)? > threshold {
                going = middle;
            } else {
                calm = middle;
            }
        }
        // A slice straddling the kink reads the midpoint speed with half of
        // itself past it.
        Some(going + Self::SLICE * 0.5)
    }
}

impl PlayerActor {
    /// **Whether an impact offered now is the strike he has already made**:
    /// one timed too soon after his swing's contact to be another. Once the
    /// swing has met its contact — by the pose, or by the clock it was timed
    /// on, which the pose meets a hair either side of — that is any read,
    /// the same contact read again on the frame the boot meets the ball
    /// included. Before it, a read a sample or more later is the ball still
    /// leaving the boot, or the engine setting it down past the end of a
    /// carry and taking it back, and the swing stays on the first.
    pub(super) fn same_strike(&self, impact: &Impact) -> bool {
        let Some(kick) = self.kick.filter(|kick| kick.kind != Strike::Trap) else {
            return false;
        };
        let gap = self.strike_clock + impact.contact.delay - kick.contact;
        let met = kick.swing >= 0.0 || kick.contact <= self.strike_clock;
        impact.contact.kind != Strike::Trap
            && gap < Actors::REARM
            && (met || gap > Actors::SAME_CONTACT)
    }

    /// **A swing already under way, re-timed onto a fresh read of its
    /// contact**, so a read that wanders — the lookahead's probes sliding
    /// past the recording's samples — steers the swing instead of jolting
    /// it. Late is the worse fault, a ball leaving before the boot arrives,
    /// so it may catch up quickly; going back is the stutter, so it never
    /// runs backwards faster than half its own countdown.
    pub(super) fn retimed(&self, read: f32, window: f32, match_delta: f32) -> f32 {
        let Some(kick) = self.kick.filter(|kick| kick.swing < 0.0) else {
            return read;
        };
        let step = match_delta / window;
        read.clamp(
            kick.swing - Actors::RETIME.0 * step,
            kick.swing + Actors::RETIME.1 * step,
        )
        .min(0.0)
    }

    /// **A strike somebody else is making**: the ball his swing was timed
    /// for is being struck, or taken, by another man — nearer it now than he
    /// is, where a moment ago he was the nearer. One impact is one kick, so
    /// his leg comes back out of the swing rather than kicking at nothing a
    /// couple of metres from the ball. A swing past contact is his own and
    /// finishes.
    pub(super) fn yield_strike(&mut self, struck: bool, taken: bool, match_delta: f32) {
        let elsewhere = self.kick.is_some_and(|kick| {
            kick.swing < 0.0
                && if kick.kind == Strike::Trap {
                    taken
                } else {
                    struck
                }
        });
        if elsewhere {
            self.unwind(match_delta);
        }
    }

    /// The swing he is in, let go of — twice as fast as it came on.
    pub(super) fn unwind(&mut self, match_delta: f32) {
        let Some(kick) = &mut self.kick else {
            return;
        };
        kick.blend -= 2.0 * match_delta / kick.kind.onset();
        if kick.blend <= 0.0 {
            self.kick = None;
        }
    }

    /// **A ball he is about to play off his body**, a kick or a header —
    /// neither with it in his hands.
    pub(super) fn playing_it(&self) -> bool {
        self.kick
            .is_some_and(|kick| matches!(kick.kind, Strike::Boot | Strike::Head))
    }

    /// How far a throw-in's wind-up has taken the ball out of his arms and up
    /// over his head, 0..1. Not a keeper's throw, whose other arm stays
    /// round the ball until it is gone — and which is only a throw while
    /// his arms are still round it: see [`PlayerActor::swing_leg`].
    pub(super) fn winding_up(&self) -> f32 {
        self.kick
            .filter(|kick| kick.kind == Strike::ThrowIn && kick.swing <= 0.0)
            .map_or(0.0, |kick| kick.blend)
    }

    /// **A ball he has just played**: past the contact of a strike, it is
    /// leaving him however slowly it goes, and is no longer at his feet.
    pub(super) fn playing_it_away(&self) -> bool {
        self.kick
            .is_some_and(|kick| kick.kind != Strike::Trap && kick.swing >= 0.0)
    }

    /// **Where a struck ball meets him**, in his own frame: under the
    /// striking boot, or his head, as the swing puts them at contact. The
    /// height is the ball's own — see [`Claim::strikes`].
    fn striking_point(&self, kick: Kick) -> Vec3 {
        let mut at_contact = self.pose;
        at_contact.swing = 0.0;
        let under = Physique::underside(at_contact);
        let part = match kick.kind {
            Strike::Head => under[8],
            _ if kick.foot < 0.0 => under[2],
            _ => under[3],
        };
        Vec3::new(part.x, 0.0, part.z)
    }

    /// **His claim on the ball**, in his own frame: an arriving pass brought
    /// onto the boot taking it, and a struck ball onto the boot or the head
    /// striking it — through the backswing to contact, and back off it onto
    /// its own flight after. `None` for a man not taking one or striking one.
    pub(super) fn meeting(&self) -> Option<Claim> {
        let kick = self.kick?;
        let (point, leaves) = match kick.kind {
            Strike::Trap => (Vec3::new(kick.at.x, 0.0, kick.at.y), 1.0),
            Strike::Boot | Strike::Head => (self.striking_point(kick), Actors::LEAVES_BOOT),
            Strike::Throw | Strike::ThrowIn => return None,
        };
        let approach = if kick.swing < 0.0 {
            Actors::ease(1.0 + kick.swing)
        } else {
            1.0 - Actors::ease(kick.swing / leaves)
        } * kick.blend;
        (approach > 1e-3).then_some(Claim {
            by: self.id,
            point,
            approach,
            due: kick.contact - self.strike_clock,
            strikes: kick.kind != Strike::Trap,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::replay::Sample;

    /// A ball sitting still and then struck, sampled every 30 ms, leaving
    /// over three samples the way a hard strike is recorded.
    fn struck(at: u32) -> Track {
        let mut track = Track::default();
        let mut x = 400.0f32;
        let mut samples = Vec::new();
        for i in 0..=20u32 {
            let t = i * 30;
            if t > at {
                let since = (t - at) as f32;
                x = 400.0 + since * 0.02 + (since - 30.0).max(0.0) * 0.1;
            }
            samples.push(Sample {
                t,
                x,
                y: 270.0,
                z: 0.0,
            });
        }
        track.merge(samples);
        track
    }

    /// The instant is found on the recording's own clock, not on the probe
    /// grid: read from a playhead moving a millisecond at a time it comes
    /// closer by a millisecond at a time.
    #[test]
    fn a_strike_is_timed_to_the_millisecond() {
        let mut ball = struck(300);
        let mut last: Option<f32> = None;
        for now in 200..290 {
            let contact = Actors::next_impact(&mut ball, now as f64).expect("a strike");
            if let Some(was) = last {
                let closer = was - contact.delay;
                assert!(
                    (closer - 0.001).abs() < 0.002,
                    "the strike comes {closer:.4} s closer in a millisecond at {now}"
                );
            }
            last = Some(contact.delay);
        }
    }

    /// One strike, one swing: the ball still leaving the boot a sample later
    /// is not a second strike, and the follow-through carries on through it.
    #[test]
    fn a_strike_leaving_over_several_samples_is_one_swing() {
        let mut actor = PlayerActor::new(7, false, true);
        let impact = |delay: f32| Impact {
            by: 7,
            contact: Contact {
                at: Vec3::ZERO,
                velocity: Vec3::new(0.0, 0.0, 20.0),
                delay,
                kind: Strike::Boot,
            },
        };
        let frame = 1.0 / 60.0;
        let mut delay = 0.15f32;
        while delay > 0.0 {
            actor.swing_leg(Some(impact(delay)), frame, false);
            delay -= frame;
        }
        let mut last = actor.kick.unwrap().swing;
        for step in 0..8 {
            // The recording's second and third samples of the same strike.
            let offered = (step < 4).then(|| impact(0.03 - step as f32 * frame));
            actor.swing_leg(offered, frame, false);
            let swing = actor.kick.unwrap().swing;
            assert!(swing >= last, "the swing goes back from {last} to {swing}");
            last = swing;
        }
        // …and a strike a second later is a strike of its own.
        for _ in 0..60 {
            actor.swing_leg(None, frame, false);
        }
        actor.swing_leg(Some(impact(0.12)), frame, false);
        assert!(actor.kick.is_some_and(|kick| kick.swing < -0.5));
    }

    fn touch(delay: f32, kind: Strike) -> Impact {
        Impact {
            by: 7,
            contact: Contact {
                at: Vec3::ZERO,
                velocity: Vec3::new(0.0, 0.0, if kind == Strike::Trap { -15.0 } else { 20.0 }),
                delay,
                kind,
            },
        }
    }

    /// A strike the lookahead finds on alternate frames is one swing, not
    /// a swing handed back and forth to the pass arriving after it.
    #[test]
    fn a_strike_read_on_alternate_frames_is_one_swing() {
        let mut actor = PlayerActor::new(7, false, true);
        let frame = 1.0 / 60.0;
        let mut last = -1.0f32;
        for step in 0..8 {
            let delay = 0.14 - step as f32 * frame;
            let coming = (step % 2 == 0).then(|| touch(delay, Strike::Boot));
            let arriving = Some(touch(delay + 0.1, Strike::Trap));
            let offered = actor.next_touch(coming, arriving);
            actor.swing_leg(offered, frame, false);
            let kick = actor.kick.expect("still swinging");
            assert!(
                kick.kind == Strike::Boot,
                "handed to {:?} at {step}",
                kick.kind
            );
            assert!(
                kick.swing > last,
                "the swing goes back to {} at {step}",
                kick.swing
            );
            last = kick.swing;
        }
    }

    /// One strike, one set of limbs: a ball read a centimetre either side of
    /// head height does not flick the swing between a kick and a header.
    #[test]
    fn a_strike_keeps_its_limbs() {
        let mut actor = PlayerActor::new(7, false, true);
        let frame = 1.0 / 60.0;
        for step in 0..8 {
            let kind = if step % 2 == 0 {
                Strike::Boot
            } else {
                Strike::Head
            };
            actor.swing_leg(Some(touch(0.14 - step as f32 * frame, kind)), frame, false);
            assert!(actor.kick.is_some_and(|kick| kick.kind == Strike::Boot));
        }
    }
}
