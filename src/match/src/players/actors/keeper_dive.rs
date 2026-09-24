//! **A dive as a body makes it**: the load and the push before he leaves the
//! ground, the side he goes down on, the whole figure turning through the
//! flight, the spring in the extension, the thud of the landing and the
//! get-up. The recording says when and where; this says how.
use super::*;

/// **A take-off the recording has coming**: how long until he leaves the
/// ground, in seconds of match time, which way the flight goes, flat and in
/// world space, and how fast across the ground.
#[derive(Clone, Copy)]
pub(super) struct TakeOff {
    pub(super) delay: f32,
    pub(super) way: Vec2,
    pub(super) pace: f32,
}

impl Actors {
    /// How many probes ahead a take-off is looked for, thirty milliseconds
    /// apart: far enough for the whole load, near enough that a split-step
    /// is never mistaken for one — see [`Actors::HOP_CEILING`].
    const TAKE_OFF_AHEAD: u32 = 5;
    /// **How far off his facing a body goes over**, in radians, at the least:
    /// a keeper lands on his side whichever way the flight took him. Toppled
    /// along the flight itself, the half of the recorded dives that travel up
    /// the pitch were drawn as a plank falling on its face.
    pub(super) const FALLS_SIDEWAYS: f32 = 1.40;
    /// Seconds from take-off for the whole figure to turn to its flight
    /// angle. Fast out of the push and easing into the stretch, the way a
    /// thrown body turns — never the other way round, which is a man jumping
    /// upright and tipping over at the top.
    pub(super) const TOPPLE_TIME: f32 = 0.30;
    /// The spring the extension rides: under-damped, so the arms are thrown
    /// a shade past full stretch and settle back onto it.
    pub(super) const STRETCH_SPRING: Spring = Spring {
        period: 0.5,
        damping: 0.6,
    };
    /// How he comes back up off the grass: critically damped, so the get-up
    /// starts without a jolt and finishes without a crawl.
    pub(super) const RECOVERY_SPRING: Spring = Spring {
        period: 1.05,
        damping: 1.0,
    };
    /// **The landing**, which a body answers with its weight: pressed into
    /// the turf and rebounding off it, twice, smaller the second time.
    pub(super) const THUD_SPRING: Spring = Spring {
        period: 0.42,
        damping: 0.35,
    };
    /// …and how hard it is struck, per metre a second he comes down at.
    pub(super) const THUD_KICK: f32 = 2.4;
    /// The spring the push and the lead are drawn through: quick and
    /// critically damped, a few hundredths of a second behind what they
    /// follow and never a frame's jump.
    const LIMB_SPRING: Spring = Spring {
        period: 0.12,
        damping: 1.0,
    };
    /// **The load and the push**, in seconds before the take-off: he sinks
    /// into his legs from the first figure, is lowest at the second, and
    /// drives out of them over the rest — so the push peaks on the frame he
    /// leaves the ground, and is out of his legs [`Actors::PUSH_TIME`] after.
    const LOAD_FROM: f32 = 0.18;
    const PUSH_FROM: f32 = 0.06;
    pub(super) const PUSH_TIME: f32 = 0.16;
    /// How deep the load goes, as a share of the landing crouch.
    const LOAD_DEPTH: f32 = 0.8;
    /// Ground speed across which a take-off stops being straight up and
    /// starts being thrown sideways, in metres a second: a leap at a cross
    /// leaves at about one, a dive at two and up.
    const THROWN: (f32, f32) = (1.0, 3.0);

    /// **The take-off the recording has coming**, if one is within the
    /// window: the instant the recorded height crosses the grass, found
    /// between the two probes either side of it rather than at the probe —
    /// probed every thirty milliseconds, the load stepped every other frame.
    /// Never read across missing chunks or a restart teleport, and never
    /// for a hop too small to be a flight.
    pub(super) fn keeper_takeoff(track: &mut Track, now: f64) -> Option<TakeOff> {
        let here = track.position_ahead(now)?;
        if here[2] > Self::AIRBORNE_FEET {
            return None;
        }
        let mut before = 0.0f64;
        for step in 1..=Self::TAKE_OFF_AHEAD {
            let delay = step as f64 * 30.0;
            let next = track.position_ahead(now + delay)?;
            if next[2] <= Self::AIRBORNE_FEET {
                before = delay;
                continue;
            }
            let up = track.position_ahead(now + delay + 90.0)?;
            let across = Vec2::new(up[0] - next[0], up[1] - next[1]) * Field::METERS_PER_UNIT;
            let travelled =
                Vec2::new(next[0] - here[0], next[1] - here[1]).length() * Field::METERS_PER_UNIT;
            if up[2] < Self::HOP_CEILING || travelled > Self::TELEPORT * delay as f32 * 0.001 {
                return None;
            }
            // The height between two probes is not a straight line — the
            // recording's own samples bend it — so the crossing is found on
            // the track, halving the gap: half a millisecond in six probes.
            let (mut grounded, mut flying) = (before, delay);
            for _ in 0..6 {
                let middle = (grounded + flying) * 0.5;
                if track.position_ahead(now + middle)?[2] > Self::AIRBORNE_FEET {
                    flying = middle;
                } else {
                    grounded = middle;
                }
            }
            return Some(TakeOff {
                delay: ((grounded + flying) * 0.5) as f32 * 0.001,
                way: across.normalize_or_zero(),
                pace: across.length() / 0.09,
            });
        }
        None
    }

    /// A body going over along `way`, turned out onto `side` until it is at
    /// least [`Actors::FALLS_SIDEWAYS`] off his facing, forwards or back.
    fn sideways(way: Vec2, side: f32) -> Vec2 {
        let bearing = way.x.atan2(way.y).abs();
        let turned = bearing.clamp(Self::FALLS_SIDEWAYS, PI - Self::FALLS_SIDEWAYS);
        Vec2::new(side * turned.sin(), turned.cos())
    }
}

impl PlayerActor {
    /// A flat world-space direction turned into his own frame: `x` across
    /// him to his right, `y` out in front.
    fn in_his_frame(&self, way: Vec2) -> Vec2 {
        let (sin, cos) = self.heading.sin_cos();
        Vec2::new(way.x * cos - way.y * sin, way.x * sin + way.y * cos)
    }

    /// Which side he is going down on: the side the flight goes, and for a
    /// flight straight up or down the pitch the side he favours. Chosen
    /// while he loads for it and held through the dive — a body cannot
    /// change the shoulder it is falling onto in mid-air — and left
    /// undecided, 0, for a flight with no direction yet: chosen off his
    /// favoured foot there, a take-off nothing saw coming went down on the
    /// wrong side of his own dive.
    pub(super) fn choose_side(&mut self) {
        let flight = self
            .takeoff
            .map_or(self.tip, |takeoff| self.in_his_frame(takeoff.way));
        self.side = if flight == Vec2::ZERO {
            0.0
        } else if flight.x.abs() > 0.2 * flight.length() {
            flight.x.signum()
        } else {
            Complexion::footedness(self.id)
        };
    }

    /// **How much of a dive the coming flight is**, going by where it goes
    /// against where he faces: a keeper catching a ball backpedalling goes
    /// up for it, he does not go over backwards.
    pub(super) fn forwardness(&self) -> f32 {
        let way = self
            .takeoff
            .map(|takeoff| takeoff.way)
            .or_else(|| Vec2::new(self.flight.x, self.flight.z).try_normalize());
        way.map_or(1.0, |way| {
            Actors::ease((self.in_his_frame(way).y + 0.85) / 0.3)
        })
    }

    /// **The way the body goes over**, as a unit direction in his own
    /// frame. See [`Actors::FALLS_SIDEWAYS`].
    pub(super) fn fallen(&self) -> Option<Vec2> {
        let way = self.tip.try_normalize()?;
        let side = if self.side == 0.0 {
            way.x.signum()
        } else {
            self.side
        };
        Some(Actors::sideways(way, side))
    }

    /// **How far in front of him the ball is at full stretch**, 0 over his
    /// head to 1 straight out from his chest: what turning the fall out onto
    /// his side took off the flight. A smother at a striker's feet is
    /// reached for along the grass in front of the body, not above its head.
    /// Only a flight FORWARD of the fall counts — one going back over him is
    /// reached for overhead, which is where that ball is.
    pub(super) fn smother(&self) -> f32 {
        let (Some(flight), Some(fallen)) = (self.tip.try_normalize(), self.fallen()) else {
            return 0.0;
        };
        if flight.y <= fallen.y {
            return 0.0;
        }
        (flight.angle_to(fallen).abs() / Actors::FALLS_SIDEWAYS).clamp(0.0, 1.0)
    }

    /// Where a ball claimed at full stretch is drawn: between the gloves
    /// wherever the reach has put them.
    pub(super) fn claim(&self) -> Vec3 {
        let gait = self.gait();
        Physique::catch(gait.lead).lerp(Physique::hands(gait), gait.smother)
    }

    /// The whole figure's turn through the flight, 0 at take-off and 1 at
    /// its flight angle: `1 − (1 − t)²`, quick out of the push.
    pub(super) fn turning(&self) -> f32 {
        let t = (self.air / Actors::TOPPLE_TIME).clamp(0.0, 1.0);
        1.0 - (1.0 - t) * (1.0 - t)
    }

    /// **How deep he has sunk into his legs for the take-off**, 0..1 — down
    /// from [`Actors::LOAD_FROM`] before it and handed over to the push.
    pub(super) fn loading(&self) -> f32 {
        let Some(takeoff) = self.takeoff.filter(|_| self.air <= 0.0) else {
            return 0.0;
        };
        let d = takeoff.delay;
        Actors::LOAD_DEPTH
            * Actors::ease((Actors::LOAD_FROM - d) / (Actors::LOAD_FROM - Actors::PUSH_FROM))
            * (1.0 - Actors::ease((Actors::PUSH_FROM - d) / Actors::PUSH_FROM))
    }

    /// **How much of the push is in his legs**: driving up to the frame he
    /// leaves the ground, and gone by [`Actors::PUSH_TIME`] after it.
    pub(super) fn pushing(&self) -> f32 {
        if self.air > 0.0 {
            if self.bounce {
                return 0.0;
            }
            return 1.0 - Actors::ease(self.air / Actors::PUSH_TIME);
        }
        self.takeoff.map_or(0.0, |takeoff| {
            Actors::ease((Actors::PUSH_FROM - takeoff.delay) / Actors::PUSH_FROM)
        })
    }

    /// The way the body will go over, in his own frame, for the take-off he
    /// is loading for — scaled by how thrown the flight is, so a leap at a
    /// cross is taken off both feet and a dive off one.
    fn going(&self) -> Vec2 {
        let Some(takeoff) = self.takeoff.filter(|_| self.air <= 0.0) else {
            return Vec2::ZERO;
        };
        let thrown =
            Actors::ease((takeoff.pace - Actors::THROWN.0) / (Actors::THROWN.1 - Actors::THROWN.0));
        Actors::sideways(self.in_his_frame(takeoff.way), self.side) * thrown
    }

    /// The take-off he is loading for, scaled by how much of the load and
    /// the push he is in — see [`Gait::coil`].
    pub(super) fn coiled(&self) -> Vec2 {
        self.going() * self.loading().max(self.pushing())
    }

    /// **Which leg drives while he is still on the ground** — the lead the
    /// dive will have once he is off it, so the far knee is already coming
    /// through when he leaves the grass rather than arriving on that frame.
    pub(super) fn loading_side(&self) -> f32 {
        self.going().x
    }

    /// Strikes the landing — kicked by how fast he came down, and let
    /// ring — and carries the push and the lead onto what the flight asks of
    /// them. See [`Actors::LIMB_SPRING`].
    pub(super) fn settle_dive(&mut self, landed: bool, match_delta: f32, seeked: bool) {
        let (push, lead) = (self.pushing(), self.lead());
        if seeked {
            Actors::THUD_SPRING.snap(&mut self.thud, &mut self.thud_rate, 0.0);
            Actors::LIMB_SPRING.snap(&mut self.push, &mut self.push_rate, push);
            Actors::LIMB_SPRING.snap(&mut self.lead_drawn, &mut self.lead_rate, lead);
            return;
        }
        if landed {
            let falling = (-self.vertical_speed).max(0.0) + 0.25 * self.speed;
            self.thud_rate += Actors::THUD_KICK * falling * self.committed();
        }
        Actors::THUD_SPRING.settle(&mut self.thud, &mut self.thud_rate, 0.0, match_delta);
        Actors::LIMB_SPRING.settle(&mut self.push, &mut self.push_rate, push, match_delta);
        Actors::LIMB_SPRING.settle(&mut self.lead_drawn, &mut self.lead_rate, lead, match_delta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::replay::Sample;

    fn takeoff_at(height: f32, across: f32) -> Track {
        let mut track = Track::default();
        track.merge(
            (0..=12)
                .map(|i| Sample {
                    t: i * 30,
                    x: 20.0 + if i < 6 { 0.0 } else { across * (i - 5) as f32 },
                    y: 275.0,
                    z: if i < 6 { 0.0 } else { height },
                })
                .collect(),
        );
        track
    }

    /// A take-off is read to the millisecond, and never for a hop, a gap
    /// in the recording, or a man already in the air.
    #[test]
    fn a_takeoff_is_seen_coming_and_nothing_else_is() {
        let mut leap = takeoff_at(0.3, 2.0);
        assert!(
            Actors::keeper_takeoff(&mut leap, 0.0).is_none(),
            "seen too early"
        );
        let mut last = f32::MAX;
        for now in (40..=150).step_by(10) {
            let takeoff = Actors::keeper_takeoff(&mut leap, now as f64).expect("a take-off");
            assert!(takeoff.delay < last, "the take-off does not come closer");
            assert!(
                last == f32::MAX || last - takeoff.delay < 0.02,
                "the take-off jumps {:.3} s closer in ten milliseconds",
                last - takeoff.delay
            );
            last = takeoff.delay;
        }
        let takeoff = Actors::keeper_takeoff(&mut leap, 100.0).unwrap();
        assert!(takeoff.way.x > 0.99, "flies {:?}", takeoff.way);
        assert!(Actors::keeper_takeoff(&mut leap, 360.0).is_none());
        assert!(Actors::keeper_takeoff(&mut takeoff_at(0.05, 2.0), 120.0).is_none());
        assert!(Actors::keeper_takeoff(&mut Track::default(), 0.0).is_none());
    }

    /// The load comes on and goes over into the push without a step, and
    /// the push is whole on the frame he leaves the ground.
    #[test]
    fn he_loads_and_drives_out_of_it() {
        let mut actor = PlayerActor::new(100, true, true);
        let mut last = (0.0f32, 0.0f32);
        let mut deepest = 0.0f32;
        for thousandth in (0..=180).rev() {
            let delay = thousandth as f32 * 0.001;
            actor.takeoff = Some(TakeOff {
                delay,
                way: Vec2::X,
                pace: 5.0,
            });
            let now = (actor.loading(), actor.pushing());
            assert!(
                (now.0 - last.0).abs() < 0.05 && (now.1 - last.1).abs() < 0.05,
                "a step at {delay:.3} s: {last:?} -> {now:?}"
            );
            deepest = deepest.max(now.0);
            last = now;
        }
        assert!(deepest > 0.6, "he never sinks into it: {deepest}");
        assert!(
            last.0 < 0.01 && last.1 > 0.99,
            "not driving at take-off: {last:?}"
        );
    }

    /// A dive along the floor, flown at 5 m/s in `way` and landed, as the
    /// pipeline would have tipped him.
    fn flown(way: Vec2) -> PlayerActor {
        const HEIGHTS: [f32; 16] = [
            0.04, 0.10, 0.16, 0.21, 0.25, 0.27, 0.28, 0.27, 0.25, 0.21, 0.16, 0.10, 0.04, 0.0, 0.0,
            0.0,
        ];
        let mut actor = PlayerActor::new(100, true, true);
        actor.declared = KeeperFlight::Dive;
        for height in HEIGHTS {
            actor.height = height;
            actor.speed = 5.0;
            let ground = if height > Actors::AIRBORNE_FEET {
                5.0
            } else {
                0.0
            };
            if actor.track_flight(0.03, 5.0, ground, false) {
                actor.tip = way * actor.flat;
                if actor.side == 0.0 {
                    actor.choose_side();
                }
            }
        }
        actor
    }

    /// **He goes down on his side whichever way he flew** — a dive up the
    /// pitch at a striker's feet included, which toppled along its own
    /// flight was a plank landing on its face.
    #[test]
    fn a_dive_lands_on_his_side_whichever_way_it_went() {
        for way in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::new(0.6, 0.8)] {
            let (pitch, roll) = flown(way).topple();
            assert!(
                roll.abs() > 1.3 && pitch.abs() < 0.35,
                "flying {way} he lands at pitch {pitch:.2}, roll {roll:.2}"
            );
        }
    }

    /// …and a dive at a man's feet reaches for the ball along the grass in
    /// front of him, where one across the goal reaches past his head.
    #[test]
    fn a_smother_reaches_out_in_front() {
        assert!(flown(Vec2::Y).smother() > 0.9);
        assert!(flown(Vec2::X).smother() < 0.1);
        let reach = |smother: f32| {
            let mut gait = Gait {
                keeper: 1.0,
                dive: 1.0,
                stretch: 1.0,
                reach: 1.0,
                smother,
                ..Gait::resting()
            };
            gait.lead = 1.0;
            Physique::glove(1.0, gait)
        };
        let (over, out) = (reach(0.0), reach(1.0));
        assert!(
            out.z > over.z + 0.25 && out.y < over.y - 0.25,
            "a smother reaches to {out:?} against {over:?} over his head"
        );
    }

    /// A flight going back over him is a leap, not a dive: a keeper
    /// backpedalling to a ball goes up for it.
    #[test]
    fn a_flight_backwards_is_a_leap() {
        let mut actor = PlayerActor::new(100, true, true);
        for (way, wanted) in [(Vec2::NEG_Y, 0.0), (Vec2::Y, 1.0), (Vec2::X, 1.0)] {
            actor.takeoff = Some(TakeOff {
                delay: 0.02,
                way,
                pace: 4.0,
            });
            assert!(
                (actor.forwardness() - wanted).abs() < 1e-3,
                "flying {way} reads as {:.2} of a dive",
                actor.forwardness()
            );
        }
    }

    /// The landing gives and comes back, twice, smaller the second time —
    /// and the whole of it is over inside a second.
    #[test]
    fn the_landing_gives_and_springs_back() {
        let mut actor = flown(Vec2::X);
        actor.vertical_speed = -2.0;
        actor.settle_dive(true, 1.0 / 60.0, false);
        let mut peaks = Vec::new();
        let (mut last, mut rising) = (actor.thud, true);
        for _ in 0..60 {
            actor.settle_dive(false, 1.0 / 60.0, false);
            if rising && actor.thud < last {
                peaks.push(last);
            }
            if !rising && actor.thud > last {
                peaks.push(last);
            }
            rising = actor.thud > last;
            last = actor.thud;
        }
        assert!(peaks.len() >= 2, "no rebound: {peaks:?}");
        assert!(
            peaks[0] > 0.1 && peaks[1] < 0.0,
            "not a give and a rebound: {peaks:?}"
        );
        assert!(
            peaks[1].abs() < peaks[0],
            "the rebound outgrows the give: {peaks:?}"
        );
        assert!(actor.thud.abs() < 0.02, "still ringing after a second");
    }
}
