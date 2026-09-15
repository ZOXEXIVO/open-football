//! **The shot a goal gets**: the ball in the netting, and then the men who put
//! it there.
//!
//! # What the camera used to do about a goal
//!
//! Nothing. The broadcast rig has exactly one subject — the ball — and after a
//! goal the ball is the least interesting object on the ground. Measured off
//! real recordings, it lies motionless in the mesh for a median 3.6 s, is then
//! picked out by an opponent and carried to the centre spot, and sits there in
//! his hands until the restart. The camera went with it: the shot swung away
//! from the goalmouth a few seconds after the ball crossed the line and ended
//! up at the halfway line, framing a man holding a football, while eight of
//! his opponents were still piled on in the corner fifty metres away. On a
//! recording long enough to show it, that is most of a minute of midfield.
//!
//! # What it does instead
//!
//! The picture stays on the ball for [`GoalShot::NET_MS`] — the net stretching,
//! which is the shot everybody replays — and then leaves it for the
//! celebration, and hands it back when the celebration is over. The rig never
//! moves: this changes what the gantry is POINTED AT and how tightly, which is
//! what a main camera does about a goal. The two written shots that do take the
//! camera off its gantry are a substitution and the line-up, and both of those
//! outrank this one where they overlap — see
//! [`TvCamera::follow_play`](crate::broadcast::camera::TvCamera::follow_play).
//!
//! # Who the shot is on
//!
//! **Whoever the celebration has formed around**, measured off the bodies
//! rather than read off the team sheet. Every man is weighted by how many of
//! his own side are within [`GoalShot::MOB`] of him, and the shot is aimed at
//! that weighted mean — so six men piled on each other outweigh five spread
//! across the pitch, and the aim lands on the pile-on.
//!
//! That the subject is never NAMED is the whole reason it survives an own
//! goal. There is no hero to follow when a defender puts it in his own net —
//! the engine has no one to mob (`GoalCelebration::arm`) and the party forms at
//! the corner flag by itself — and a shot written as "point at the scorer"
//! would spend it on the one man on the field who is not celebrating. A shot
//! written as "point at wherever they have piled up" needs no second rule for
//! it. Measured against the engine's own party spot over seven goals, the aim
//! is within one to four metres of it from +4 s and holds there until the
//! huddle breaks.
//!
//! The same weighting decides HOW FAR the shot commits: a side merely bunched
//! in the box as the ball goes in is a loose crowd and barely moves the
//! camera, and the shot only leaves the ball outright once the knot is a real
//! one. See [`GoalShot::committed`].

use crate::broadcast::Grip;
use crate::broadcast::camera::CameraFlight;
use crate::broadcast::focus::CameraSubject;
use crate::broadcast::lineup::Lineup;
use crate::players::actors::PlayerActor;
use crate::players::aftermath::Aftermath;
use crate::recording::playback::Playback;
use crate::scene::field::Field;
use bevy::prelude::*;

/// The camera a goal is watched from.
#[derive(Resource, Default)]
pub struct GoalShot {
    /// Where the celebration is, on the ground.
    ///
    /// Held rather than recomputed in [`Self::blend`] for the reason
    /// [`ChangeoverShot`](crate::broadcast::changeover::ChangeoverShot) holds
    /// its portrait: it is measured off the men's own transforms, which only
    /// the system that queries the actors can see. It is also what the walk
    /// home blends OUT of — the crew empties as the reaction fades, and a
    /// point cleared on that frame would snap the shot back to the ball
    /// instead of easing it.
    at: Vec3,
    /// How far the shot has left the ball for them, 0..1.
    grip: f32,
}

impl GoalShot {
    /// How long the picture stays on the ball after it crosses the line, in ms
    /// of match clock.
    ///
    /// The net stretching is the shot, and it is the one moment of a goal that
    /// happens where the ball is. It also costs nothing: the men are still
    /// setting off. Measured over seven goals, the first of them is within
    /// five metres of another at +2 s and the pile-on is not worth looking at
    /// before then.
    const NET_MS: f64 = 1_600.0;

    /// Seconds for the shot to swing off the ball onto them, and to give it
    /// back.
    ///
    /// Longer than a substitution's close-up ramp, because this is a pan of
    /// forty metres at a hundred, and a broadcast operator takes about this
    /// long over one. [`TvCamera::RESPONSE`](crate::broadcast::camera::TvCamera)
    /// smooths another three tenths on top, so the swing reads as a second and
    /// a half.
    const CLOSE_TIME: f32 = 1.2;

    /// How close two men have to be to count as piled on each other, in
    /// metres.
    ///
    /// Five and not ten: at ten a back four standing in its own shape scores
    /// as heavily as a pile-on, and the measurement that has to separate them
    /// stops separating anything. At five, ordinary play puts a median of one
    /// man inside another's radius and a celebration puts four to six.
    const MOB: f32 = 5.0;

    /// The crowding either end of [`Self::committed`]: men who merely happen
    /// to be near each other, and men who are on top of each other.
    ///
    /// Both are counts of TEAM-MATES WITHIN [`Self::MOB`], so neither counts
    /// the man himself. Measured across seven goals: 0-3 as the ball crosses
    /// the line, 4-6 once the party has formed, and it holds there for the
    /// twelve seconds of the engine's huddle.
    const LOOSE: f32 = 2.0;
    const TIGHT: f32 = 4.0;

    /// How much tighter the lens is held on the celebration, as a multiple of
    /// the wheel's own factor.
    ///
    /// Modest, and it has to be. The pile-on is ten to fifteen metres across
    /// and a hundred from the lens, where the resting frame is some
    /// twenty-seven metres tall; at this it is eighteen, which holds the knot
    /// and the men still arriving at it. Anything tighter frames the huddle
    /// and loses the run into it, which is the half of a celebration with
    /// movement in it.
    const CLOSE: f32 = 1.5;

    /// The most bodies the crowd is measured over.
    ///
    /// A stack array rather than a `Vec`, like the row a substitution pans
    /// across: this runs every frame of every celebration and one side of a
    /// football match is eleven men.
    const CAST: usize = 11;

    /// Where the celebration is and how far the shot has gone to it.
    ///
    /// Runs after the bodies have been placed, like
    /// [`CameraSubject::settle`] and `ChangeoverShot::settle`, and for the same
    /// reason: the subject is a property of where the men have just been put,
    /// and [`TvCamera::follow_play`](crate::broadcast::camera::TvCamera::follow_play)
    /// reads what this writes.
    pub fn settle(
        time: Res<Time>,
        playback: Res<Playback>,
        aftermath: Res<Aftermath>,
        flight: Res<CameraFlight>,
        subject: Res<CameraSubject>,
        lineup: Res<Lineup>,
        actors: Query<(&PlayerActor, &Transform, &Visibility)>,
        mut shot: ResMut<GoalShot>,
    ) {
        // A man being followed by hand, a rig in flight, and the ceremony
        // before the first whistle are all something else having the picture.
        // The ORBIT is not, and is the one difference from the substitution
        // shot: this one never leaves the gantry, so a viewer who has walked
        // the rig round to behind the goal gets his goal shot from there.
        let hands_off = flight.airborne() || subject.locked() || lineup.framing().is_some();
        let celebrating = aftermath.since().is_some_and(|since| since >= Self::NET_MS);

        let crowd = (!hands_off && celebrating)
            .then(|| Self::crowd(&actors))
            .flatten();

        let wanted = crowd.map_or(0.0, |(_, packed)| Self::committed(packed));
        // Scrubbed rather than played: the shot the playhead landed in is the
        // one to be in, not one to swing into over the next second and a half.
        // The same rule the rig's own focus keeps across a seek.
        let grip = if playback.seeked {
            wanted
        } else {
            Grip::toward(shot.grip, wanted, Self::CLOSE_TIME, &time)
        };

        // Same rule the rest of the rig keeps about writes that change
        // nothing: a resource dirtied every frame of a match with no goal in
        // it is eighty minutes of change detection for one that never came.
        if let Some((at, _)) = crowd
            && shot.at != at
        {
            shot.at = at;
        }
        if shot.grip != grip {
            shot.grip = grip;
        }
    }

    /// **Where the celebrating side has piled up, and how tightly** — or
    /// `None` when none of them is drawn.
    ///
    /// Only the men on the FIELD. The substitutes of the side that has just
    /// scored are as elated as anybody and are stood in a row at the gate a
    /// metre apart, which is the tightest knot on the ground and the one place
    /// a goal is certainly not being celebrated. The touchline is the line
    /// between the two, and a celebration never crosses it — the engine keeps
    /// its cast a metre and a half inside (`GoalCelebration::TOUCHLINE_MARGIN`).
    fn crowd(actors: &Query<(&PlayerActor, &Transform, &Visibility)>) -> Option<(Vec3, f32)> {
        let mut men = [Vec3::ZERO; Self::CAST];
        let mut drawn = 0;
        for (actor, at, visibility) in actors {
            if *visibility == Visibility::Hidden
                || actor.elation() <= Aftermath::NOTHING
                || at.translation.z.abs() > Field::HALF_WIDTH
                || drawn == men.len()
            {
                continue;
            }
            men[drawn] = at.translation.with_y(0.0);
            drawn += 1;
        }
        Self::around(&men[..drawn])
    }

    /// **Where a set of men on the ground have piled up, and how tightly** —
    /// the weighted mean of them, and the largest number any one of them has
    /// within [`Self::MOB`].
    ///
    /// Split from the query above because it is the half that is about
    /// football: which bodies are in it is a question for the ECS, and where
    /// they have gathered is a question that can be asked of eleven
    /// coordinates.
    fn around(men: &[Vec3]) -> Option<(Vec3, f32)> {
        let mut centre = Vec3::ZERO;
        let mut total = 0.0;
        let mut packed = 0.0f32;
        for man in men {
            let near = men
                .iter()
                .filter(|other| (**other - *man).length() <= Self::MOB)
                .count()
                - 1;
            packed = packed.max(near as f32);
            // Squared, so the knot outweighs the stragglers rather than merely
            // outvoting them. Measured against the engine's party spot over
            // seven recorded goals, the linear weight lands one to five metres
            // off it and this lands one to two.
            let weight = (near * near) as f32;
            total += weight;
            centre += *man * weight;
        }
        (total > 0.0).then(|| (centre / total, packed))
    }

    /// How far the shot commits to them, 0..1, given the tightest knot among
    /// them.
    ///
    /// A ramp rather than a threshold, and that is what lets one rule serve a
    /// goal celebrated by nine men and one celebrated by nobody. A side that
    /// is merely bunched in the box as the ball goes in reads as a loose crowd
    /// and barely moves the camera; a pile-on takes it outright; a consolation
    /// in the ninetieth that nobody runs to leaves the shot on the ball, which
    /// is where a shot with nothing to cut to belongs.
    fn committed(packed: f32) -> f32 {
        ((packed - Self::LOOSE) / (Self::TIGHT - Self::LOOSE)).clamp(0.0, 1.0)
    }

    /// The point the rig is framing, blended off the ball towards the
    /// celebration by [`Self::grip`].
    ///
    /// Both ends every frame and mixed, rather than switched between, so the
    /// swing out and the swing back are one path — the same arrangement the
    /// substitution shot's own blend keeps.
    pub fn blend(&self, framing: Vec3) -> Vec3 {
        if self.grip <= 0.0 {
            return framing;
        }
        framing.lerp(self.at, self.grip)
    }

    /// How the lens is held while the shot is on, as a multiple of the wheel's
    /// own factor.
    pub fn magnification(&self) -> f32 {
        1.0 + (Self::CLOSE - 1.0) * self.grip
    }

    /// How far the shot has left the ball for the celebration, 0..1.
    ///
    /// Read by the rig as well as used here: pointing the camera at the
    /// corner is only half of getting the celebration on screen, and the
    /// other half is the framing constants the rig holds for a shot that has
    /// a subject — see
    /// [`TvCamera::LOCK_AIM_ACROSS`](crate::broadcast::camera::TvCamera).
    pub fn grip(&self) -> f32 {
        self.grip
    }
}

/// What these rules are about is football rather than pixels — which of the
/// twenty-two a goal is about, and when — so they can be checked without a
/// camera to look through.
#[cfg(test)]
mod tests {
    use super::*;

    /// Men standing on the ground, as [`GoalShot::crowd`] would have collected
    /// them off their transforms.
    fn knot(men: &[Vec2]) -> Option<(Vec3, f32)> {
        let men: Vec<Vec3> = men.iter().map(|at| Vec3::new(at.x, 0.0, at.y)).collect();
        GoalShot::around(&men)
    }

    /// A pile-on in the corner and four men jogging back up the pitch: the
    /// shot is of the corner. This is the whole of what the crowding weight
    /// exists to do — a plain mean of the eleven sits out in the middle of
    /// the pitch, which is the framing being fixed.
    #[test]
    fn the_shot_is_on_the_pile_on_rather_than_on_the_side_that_scored() {
        let mut men = vec![
            Vec2::new(40.0, 26.0),
            Vec2::new(41.5, 27.0),
            Vec2::new(39.0, 27.5),
            Vec2::new(40.5, 28.5),
            Vec2::new(42.0, 25.5),
        ];
        let stragglers = [
            Vec2::new(-10.0, 0.0),
            Vec2::new(0.0, -12.0),
            Vec2::new(12.0, 8.0),
            Vec2::new(-25.0, 15.0),
        ];
        men.extend_from_slice(&stragglers);

        let (at, packed) = knot(&men).expect("a pile-on is a crowd");
        let party = Vec3::new(40.6, 0.0, 26.9);
        assert!(
            at.distance(party) < 2.0,
            "the aim landed {:.1} m from the pile-on at {party:?}: {at:?}",
            at.distance(party)
        );
        assert!(
            packed >= GoalShot::TIGHT,
            "five men on top of each other did not read as a pile-on: {packed}"
        );

        let mean = men
            .iter()
            .fold(Vec3::ZERO, |sum, at| sum + Vec3::new(at.x, 0.0, at.y))
            / men.len() as f32;
        assert!(
            mean.distance(party) > 20.0,
            "the test is not testing anything: an unweighted mean was already on the party"
        );
    }

    /// …and a side merely standing in its shape is not a celebration, so the
    /// camera stays on the football. A goal is scored into a crowded box and
    /// the shot must not leave the net for the crowd that was already there.
    #[test]
    fn a_side_in_its_own_shape_does_not_take_the_camera() {
        let shape: Vec<Vec2> = (0..11)
            .map(|man| Vec2::new(-30.0 + man as f32 * 6.0, (man % 3) as f32 * 9.0 - 9.0))
            .collect();
        let packed = knot(&shape).map_or(0.0, |(_, packed)| packed);
        assert!(
            GoalShot::committed(packed) <= 0.0,
            "a team standing in a formation read as a pile-on: {packed} men close"
        );
    }

    /// The commitment is a ramp and not a switch: a knot that is forming takes
    /// the camera part of the way, which is what makes the swing one move
    /// rather than a cut on the frame the fourth man arrives.
    #[test]
    fn the_shot_leans_towards_a_crowd_that_is_still_gathering() {
        assert_eq!(GoalShot::committed(GoalShot::LOOSE), 0.0);
        assert_eq!(GoalShot::committed(GoalShot::TIGHT), 1.0);
        assert_eq!(GoalShot::committed(GoalShot::TIGHT + 3.0), 1.0);
        let half = GoalShot::committed((GoalShot::LOOSE + GoalShot::TIGHT) * 0.5);
        assert!(
            (half - 0.5).abs() < 1e-6,
            "half a knot did not read as half a commitment: {half}"
        );
    }

    /// Nothing is framed anywhere but the ball until the shot has been given
    /// something, and the walk home blends out of wherever it last was rather
    /// than cutting.
    #[test]
    fn the_ball_is_the_shot_until_the_celebration_takes_it() {
        let ball = Vec3::new(54.3, 0.0, -1.2);
        let party = Vec3::new(40.5, 0.0, 28.6);

        let cold = GoalShot::default();
        assert_eq!(
            cold.blend(ball),
            ball,
            "the shot moved with no goal behind it"
        );
        assert_eq!(cold.magnification(), 1.0);

        let mut running = GoalShot {
            at: party,
            grip: 1.0,
        };
        assert_eq!(running.blend(ball), party);
        assert_eq!(running.magnification(), GoalShot::CLOSE);

        running.grip = 0.5;
        let midway = running.blend(ball);
        assert!(
            midway.distance(ball) > 1.0 && midway.distance(party) > 1.0,
            "a half-run ramp cut instead of blending: {midway:?}"
        );
    }
}
