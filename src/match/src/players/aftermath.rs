//! The seconds after the ball hits the net.
//!
//! # Why the viewer has to know about goals at all
//!
//! Everything else in this crate is derived from the recording, and the
//! recording is positions. That works because almost everything a footballer
//! does is legible from where he is: a stride from the ground he covers, a
//! strike from what the ball does next, a dive from leaving the turf.
//!
//! A reaction is not. Twenty-two men stand still for a few seconds after a
//! goal and eleven of them are elated and eleven are sick, and the position
//! track says only that nobody moved. Drawn from positions alone the whole
//! celebration reads as a crowd of people waiting for a bus — which is
//! exactly how it was reported: the beaten goalkeeper "doesn't get upset,
//! he just folds his arms and turns round".
//!
//! (The turning round was real and separate: a man below
//! [`Actors::MOVING`](crate::players::actors::Actors) is drawn facing the BALL,
//! and for the conceding side the ball is behind them in their own net,
//! rattling in the mesh. So they walked backwards out of their own goal,
//! swivelling to keep watching it. Nobody in football watches the ball after a
//! goal; [`Aftermath`] is also what switches that rule off.)
//!
//! # The signal
//!
//! The page already hands the viewer the goals — `ViewerConfig::goals`, used
//! until now only to put markers on the seek bar. It carries the match clock
//! and the scorer, and `ViewerConfig::goal_belongs_to_home` already resolves
//! own goals, so which side is celebrating and which is not is a lookup
//! rather than an inference. Nothing new goes over the wire.

use crate::app::config::ViewerConfig;
use crate::recording::playback::Playback;
use bevy::prelude::*;

/// How the two sides are feeling at the playhead.
///
/// A resource rather than a per-player component because it is one fact
/// about the match clock; who it applies to is decided per actor by
/// [`Aftermath::despair`] / [`Aftermath::elation`], which take the only
/// thing that varies — which side he is on.
#[derive(Resource, Default)]
pub struct Aftermath {
    /// Which side put the ball in: `Some(true)` when the goal counts for the
    /// home team. `None` outside any goal's window.
    scored_by_home: Option<bool>,
    /// Strength of the reaction, 0..1, fading out at the end of the window.
    weight: f32,
}

impl Aftermath {
    /// How long the reaction holds at full strength, in ms of match clock.
    ///
    /// Matched to the engine's own choreography: `GoalCelebration::HUDDLE_MS`
    /// is 12 s, after which the pile-on breaks up and everybody walks back
    /// toward the restart. Holding the slump past that would have the
    /// conceding side trudging into position with their hands on their heads
    /// for another forty seconds.
    const HOLD_MS: f64 = 12_000.0;
    /// …and how long it takes to let go of it afterwards.
    const FADE_MS: f64 = 3_000.0;
    /// Below this there is nothing to draw and every consumer can
    /// short-circuit. Not zero, because the per-actor ramps in
    /// [`Actors::animate`](crate::players::actors::Actors::animate) trail the
    /// resource by a few frames.
    pub const NOTHING: f32 = 1e-3;

    /// Reads the playhead against the fixture's goals, once a frame.
    ///
    /// Linear scan over the goal list, which is at most a dozen entries and
    /// usually two or three — a cursor like
    /// [`Track`](crate::recording::replay::Track)'s would be more code than the
    /// thing it optimised.
    pub fn follow_playhead(
        config: Res<ViewerConfig>,
        playback: Res<Playback>,
        mut aftermath: ResMut<Aftermath>,
    ) {
        let now = playback.time_ms;
        let mut latest: Option<(f64, bool)> = None;
        for goal in &config.goals {
            // Strictly after the goal: on the frame the ball crosses the line
            // nobody has reacted yet.
            if goal.time > now {
                continue;
            }
            let home = config.goal_belongs_to_home(goal);
            if latest.is_none_or(|(best, _)| goal.time > best) {
                latest = Some((goal.time, home));
            }
        }

        let Some((time, home)) = latest else {
            aftermath.scored_by_home = None;
            aftermath.weight = 0.0;
            return;
        };
        let since = now - time;
        let weight = if since <= Self::HOLD_MS {
            1.0
        } else {
            (1.0 - (since - Self::HOLD_MS) / Self::FADE_MS).clamp(0.0, 1.0)
        };
        aftermath.scored_by_home = (weight > 0.0).then_some(home);
        aftermath.weight = weight as f32;
    }

    /// How sick this side is, 0..1.
    pub fn despair(&self, is_home: bool) -> f32 {
        match self.scored_by_home {
            Some(scored_by_home) if scored_by_home != is_home => self.weight,
            _ => 0.0,
        }
    }

    /// …and how delighted.
    pub fn elation(&self, is_home: bool) -> f32 {
        match self.scored_by_home {
            Some(scored_by_home) if scored_by_home == is_home => self.weight,
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(scored_by_home: Option<bool>, weight: f32) -> Aftermath {
        Aftermath {
            scored_by_home,
            weight,
        }
    }

    /// The whole point: the two sides do OPPOSITE things, and the side that
    /// scored is the side the goal belongs to — which for an own goal is not
    /// the scorer's side. `ViewerConfig::goal_belongs_to_home` owns that
    /// rule; this owns the half that turns it into two moods.
    #[test]
    fn the_side_that_conceded_is_the_one_that_did_not_score() {
        let home_scored = at(Some(true), 1.0);
        assert_eq!(home_scored.elation(true), 1.0);
        assert_eq!(home_scored.despair(true), 0.0);
        assert_eq!(home_scored.despair(false), 1.0);
        assert_eq!(home_scored.elation(false), 0.0);
    }

    /// And with no goal behind the playhead, nobody is doing anything —
    /// which has to be the answer for the ninety minutes of a match that
    /// are not the ten seconds after a goal.
    #[test]
    fn nothing_is_happening_for_most_of_a_match() {
        let quiet = at(None, 0.0);
        for side in [true, false] {
            assert_eq!(quiet.despair(side), 0.0);
            assert_eq!(quiet.elation(side), 0.0);
        }
    }
}
