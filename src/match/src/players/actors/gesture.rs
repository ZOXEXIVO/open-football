//! What a man with nothing to do does with his hands, and how long the
//! engine has to have said what he is doing before his body answers it.
use super::*;

/// What a man with nothing to do is doing with his hands — see
/// [`PlayerActor::gesturing`]. Each 0..1, and for most of the pitch for
/// most of a match all of them zero.
#[derive(Clone, Copy, Default)]
pub(super) struct Idle {
    pub(super) urging: f32,
    pub(super) pointing: f32,
    pub(super) hands_on_hips: f32,
    pub(super) doubled_over: f32,
}

impl Idle {
    fn channels(&mut self) -> [&mut f32; 4] {
        [
            &mut self.urging,
            &mut self.pointing,
            &mut self.hands_on_hips,
            &mut self.doubled_over,
        ]
    }
}

impl Actors {
    /// Seconds the engine has to keep naming a state before a man's hands
    /// answer it. Two thirds of recorded state spans are under 300 ms — a
    /// standing defender is `Running`, `Guarding`, `Returning` and `Pressing`
    /// inside a tenth of a second — and a gesture retargeted on every one
    /// of them is an arm snapping ninety degrees a frame.
    const ATTITUDE_HOLD: f32 = 0.5;
    /// How a gesture comes on and goes off: critically damped, so an arm
    /// eases out of where it was and into where it is going whatever made
    /// him change his mind.
    const GESTURE_SPRING: Spring = Spring {
        period: 0.6,
        damping: 1.0,
    };
}

impl PlayerActor {
    /// Takes the engine's name for what he is doing, once it has held for
    /// [`Actors::ATTITUDE_HOLD`] of match time.
    pub(super) fn hear(&mut self, named: Attitude, delta: f32, seeked: bool) {
        if named != self.heard {
            self.heard = named;
            self.heard_for = 0.0;
        }
        self.heard_for += delta;
        if seeked || self.heard_for >= Actors::ATTITUDE_HOLD {
            self.attitude = named;
        }
    }

    /// Carries his hands toward whatever [`Self::gesturing`] says this frame.
    pub(super) fn settle_gesture(&mut self, delta: f32, seeked: bool) {
        let mut wanted = self.gesturing();
        let spring = Actors::GESTURE_SPRING;
        for ((value, rate), target) in self
            .gesture
            .channels()
            .into_iter()
            .zip(self.gesture_rate.channels())
            .zip(wanted.channels())
        {
            if seeked {
                spring.snap(value, rate, *target);
            } else {
                spring.settle(value, rate, *target, delta);
            }
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
    ///
    /// **And the same question for the other twenty**, who used to have no
    /// answer at all: an outfielder is still for a tenth of a match and
    /// stood through all of it with his arms at his sides. What he does is
    /// read off what the recording says he is doing ([`Attitude`]) and how
    /// hard he has just been working ([`Self::effort`]): a resting man
    /// puts his hands on his hips, or on his knees if he is blowing; a man
    /// holding a line organises the men beside him; a man making himself
    /// available points where he wants it.
    pub(super) fn gesturing(&self) -> Idle {
        let spare = (1.0 - (self.speed / Actors::MOVING).clamp(0.0, 1.0))
            * (1.0 - self.carry)
            * (1.0 - self.carrying)
            * (1.0 - self.dive)
            * (1.0 - self.reaction)
            * (1.0 - self.despair.max(self.elation))
            * (1.0 - self.kick.map_or(0.0, |kick| kick.blend))
            * f32::from(self.at_attention.is_none())
            // A keeper whose ball is near his goal has something to do; an
            // outfielder who stopped half a second ago is about to.
            * if self.is_goalkeeper {
                1.0 - self.set
            } else {
                Actors::ease((self.still - Actors::IDLE_ONSET.0) / Actors::IDLE_ONSET.1)
            };
        if spare <= 1e-3 {
            return Idle::default();
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
        match (self.is_goalkeeper, self.attitude) {
            (true, _) => Idle {
                urging: window(1.2, Actors::GESTURE_HOLD),
                pointing: window(6.0, Actors::GESTURE_HOLD) * hand,
                hands_on_hips: window(9.8, Actors::GESTURE_STANCE),
                doubled_over: 0.0,
            },
            (false, Attitude::Resting) => {
                let winded = Actors::ease((self.effort - Actors::WINDED.0) / Actors::WINDED.1);
                let rest = window(0.8, Actors::GESTURE_REST);
                Idle {
                    hands_on_hips: rest * (1.0 - winded),
                    doubled_over: rest * winded,
                    ..Idle::default()
                }
            }
            (false, Attitude::Alert) => Idle {
                pointing: window(5.5, Actors::GESTURE_HOLD) * hand,
                ..Idle::default()
            },
            (false, Attitude::Calling) => Idle {
                pointing: window(2.0, Actors::GESTURE_HOLD) * hand,
                urging: window(8.5, Actors::GESTURE_HOLD),
                ..Idle::default()
            },
            (false, Attitude::Neutral) => Idle {
                hands_on_hips: window(9.8, Actors::GESTURE_STANCE),
                ..Idle::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state the engine names for a frame at a time is not one his body
    /// takes up; one it keeps naming is.
    #[test]
    fn a_flickering_state_does_not_move_his_hands() {
        let mut actor = PlayerActor::new(207, false, true);
        let frame = 1.0 / 60.0;
        for step in 0..120 {
            let named = if step % 4 < 2 {
                Attitude::Calling
            } else {
                Attitude::Alert
            };
            actor.hear(named, frame, false);
            assert_eq!(actor.attitude, Attitude::Neutral, "took up a flicker");
        }
        for _ in 0..40 {
            actor.hear(Attitude::Resting, frame, false);
        }
        assert_eq!(actor.attitude, Attitude::Resting);
    }

    /// However suddenly the target moves, his hands ease out of rest and
    /// into the new pose without overshooting it: the rate never jumps
    /// between two frames, which an exponential catch-up (a fifth of a
    /// second's, 5 per second on its first frame) and a switch both do.
    #[test]
    fn a_gesture_eases_on_and_off() {
        let mut actor = PlayerActor::new(207, false, true);
        actor.still = 10.0;
        actor.clock = 9.8 + 1.0 - Complexion::carriage(207) * Actors::GESTURE_CYCLE;
        actor.attitude = Attitude::Neutral;
        let frame = 1.0 / 60.0;
        let mut last = (0.0f32, 0.0f32);
        for step in 0..90 {
            // The window flips shut halfway through: a target that jumps.
            if step == 45 {
                actor.attitude = Attitude::Alert;
            }
            actor.settle_gesture(frame, false);
            let now = actor.gesture.hands_on_hips;
            let rate = (now - last.0) / frame;
            assert!(
                (rate - last.1).abs() < 2.5,
                "the gesture jolts at frame {step}: {:.1} -> {:.1} per second",
                last.1,
                rate
            );
            assert!((-1e-3..=1.0 + 1e-3).contains(&now), "overshot to {now}");
            last = (now, rate);
        }
    }
}
