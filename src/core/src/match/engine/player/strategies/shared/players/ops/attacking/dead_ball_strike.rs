//! **Strikes from a dead ball**: where a penalty or a direct free kick
//! goes, how hard and how high, and how often it misses the target. A
//! dead ball struck with nobody near the taker is not an open-play shot:
//! the open-play miss rolls, built for a man shooting on the move under
//! pressure, put half of all penalties off target against a real one in
//! fourteen, and drove free kicks flat into the wall.

use crate::r#match::MatchRng;
use crate::r#match::engine::ball::ball::motion::SpinModel;
use crate::r#match::engine::ball::ball::{Ball, GRAVITY_PER_TICK};
use crate::r#match::engine::goal::GOAL_WIDTH;
use nalgebra::Vector3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeadBallStrike {
    /// Where the ball crosses the line, from the centre of the goal, in
    /// units. Beyond the post for a kick dragged wide.
    pub offset: f32,
    /// How high it peaks, in metres.
    pub apex: f32,
    /// Skied over the bar.
    pub over: bool,
    /// Its pace along the ground, in units per tick, where the kick decides
    /// it rather than the shot model…
    pub pace: Option<f32>,
    /// …and the topspin it carries, in ball radii of contact over the top.
    pub dip: Option<f32>,
}

pub struct PenaltyKick;

impl PenaltyKick {
    /// About one in six penalties goes down the middle.
    const CENTRAL: f32 = 0.17;
    /// The best takers miss the target about one time in thirty, the
    /// worst one in seven; an ordinary one one in fifteen.
    const MISS_FLOOR: f32 = 0.03;
    const MISS_SPREAD: f32 = 0.12;

    /// The strike of a taker whose penalty-taking reads `execution`
    /// (0..1, fatigue and nerve already in it).
    pub fn strike(execution: f32, rng: &MatchRng) -> DeadBallStrike {
        let execution = execution.clamp(0.0, 1.0);
        let side = if rng.unit_f32() < 0.5 { -1.0 } else { 1.0 };
        let miss = Self::MISS_FLOOR + (1.0 - execution) * Self::MISS_SPREAD;
        if rng.unit_f32() < miss {
            // Two in five misses go over, the rest past the post. Eleven
            // metres is 0.37 s of flight, so a skied kick has to peak above
            // five metres to still be rising past the bar when it gets there.
            return if rng.unit_f32() < 0.4 {
                DeadBallStrike {
                    offset: side * rng.range_f32(0.0, 0.9) * GOAL_WIDTH,
                    apex: rng.range_f32(5.5, 10.0),
                    over: true,
                    pace: None,
                    dip: None,
                }
            } else {
                DeadBallStrike {
                    offset: side * rng.range_f32(1.04, 1.35) * GOAL_WIDTH,
                    apex: rng.range_f32(0.2, 1.6),
                    over: false,
                    pace: None,
                    dip: None,
                }
            };
        }
        if rng.unit_f32() < Self::CENTRAL {
            return DeadBallStrike {
                offset: rng.range_f32(-0.15, 0.15) * GOAL_WIDTH,
                apex: rng.range_f32(0.6, 2.0),
                over: false,
                pace: None,
                dip: None,
            };
        }
        // How near the post he puts it: anywhere from two metres out to
        // the inside of the post, a good taker toward the post and a poor
        // one further in, where a keeper who has guessed right gets to it.
        let depth = (0.45 + 0.40 * execution + rng.range_f32(-0.18, 0.18)).min(0.97);
        DeadBallStrike {
            offset: side * depth * GOAL_WIDTH,
            apex: rng.range_f32(0.2, 2.6),
            over: false,
            pace: None,
            dip: None,
        }
    }
}

/// **The direct free kick**: over the wall and down under the bar, or over
/// the bar, or wide.
///
/// The flight is solved with the engine's own ballistics (`Ball::
/// ballistic_crossing`: drag and the topspin the kick carries) rather than
/// a drag-free parabola. A parabola that clears a wall nine metres out is
/// still rising at a goal twenty metres away unless the ball is floated at
/// a walking pace, and a floated free kick is one the keeper walks out and
/// catches; topspin is what brings a ball struck at real pace down under
/// the bar.
pub struct FreeKickShot;

impl FreeKickShot {
    /// The best free-kick takers miss the target about two times in five,
    /// a poor one two in three.
    const MISS_FLOOR: f32 = 0.38;
    const MISS_SPREAD: f32 = 0.30;
    /// Three in five misses go over the bar, the rest wide.
    const OVER_SHARE: f32 = 0.6;
    /// The ball has to pass a jumping wall above this, in metres…
    const WALL_CLEARANCE: f32 = 2.4;
    /// …and one that does not reaches it between the knees and the head.
    const INTO_WALL: (f32, f32) = (0.6, 1.9);
    /// …come down under the bar by this…
    const UNDER_BAR: f32 = 2.25;
    /// …and not run into the grass in front of the line.
    const OFF_GROUND: f32 = 0.2;
    /// How hard he hits it, in units per tick: 21 to 27 m/s.
    const PACE_FLOOR: f32 = 1.7;
    const PACE_SPREAD: f32 = 0.4;
    /// How far over the ball he gets his laces, in ball radii: the topspin
    /// that dips it.
    const DIP_FLOOR: f32 = 0.35;
    const DIP_SPREAD: f32 = 0.3;
    /// He takes the pace off the ball until it can clear the wall and come
    /// down in time, a step at a time.
    const PACE_STEP: f32 = 0.08;
    const SLOWEST: f32 = 1.0;

    /// A free kick `distance` units from goal, with a wall `wall` units out
    /// if one stands, by a taker whose free-kick striking reads
    /// `execution` (0..1, fatigue and nerve already in it). `into_wall` is
    /// the kick that does not get over it: where on the goal line the
    /// line through the man it hits would cross.
    pub fn strike(
        execution: f32,
        distance: f32,
        wall: Option<f32>,
        into_wall: Option<f32>,
        rng: &MatchRng,
    ) -> DeadBallStrike {
        let execution = execution.clamp(0.0, 1.0);
        let side = if rng.unit_f32() < 0.5 { -1.0 } else { 1.0 };
        let dip = Self::DIP_FLOOR + Self::DIP_SPREAD * execution;
        let mut pace = Self::PACE_FLOOR + Self::PACE_SPREAD * execution + rng.range_f32(-0.1, 0.1);
        let floor = |pace: f32| Self::rise_to(distance, Self::OFF_GROUND, pace, dip);
        let (low, high) = loop {
            let high = Self::rise_to(distance, Self::UNDER_BAR, pace, dip);
            let low = wall.map_or(floor(pace), |w| {
                Self::rise_to(w, Self::WALL_CLEARANCE, pace, dip).max(floor(pace))
            });
            if low <= high || pace <= Self::SLOWEST {
                break (low.min(high), high);
            }
            pace -= Self::PACE_STEP;
        };
        let miss = Self::MISS_FLOOR + (1.0 - execution) * Self::MISS_SPREAD;
        let (offset, rise, over) = if let (Some(offset), Some(at)) = (into_wall, wall) {
            (
                offset,
                Self::rise_to(
                    at,
                    rng.range_f32(Self::INTO_WALL.0, Self::INTO_WALL.1),
                    pace,
                    dip,
                ),
                false,
            )
        } else if rng.unit_f32() < miss {
            if rng.unit_f32() < Self::OVER_SHARE {
                (
                    side * rng.range_f32(0.0, 0.95) * GOAL_WIDTH,
                    Self::rise_to(distance, rng.range_f32(3.0, 5.0), pace, dip).max(high),
                    true,
                )
            } else {
                (
                    side * rng.range_f32(1.1, 1.45) * GOAL_WIDTH,
                    rng.range_f32(low, high),
                    false,
                )
            }
        } else {
            // How near the post he gets it: a free kick on target is aimed
            // for a corner, and a poor taker drifts in towards the keeper.
            let depth = (0.62 + 0.30 * execution + rng.range_f32(-0.15, 0.15)).min(0.97);
            (side * depth * GOAL_WIDTH, rng.range_f32(low, high), false)
        };
        DeadBallStrike {
            offset,
            apex: rise * rise / (2.0 * GRAVITY_PER_TICK),
            over,
            pace: Some(pace),
            dip: Some(dip),
        }
    }

    /// The upward launch speed, in metres per tick, that puts a ball struck
    /// at `pace` with `dip` of topspin `height` metres up as it passes `at`
    /// units out. Bisected on the flight the engine will actually fly.
    fn rise_to(at: f32, height: f32, pace: f32, dip: f32) -> f32 {
        let along = Vector3::new(1.0, 0.0, 0.0);
        let spin = SpinModel::clamped(SpinModel::from_strike(along, 0.0, -dip, 1.0));
        let height_with = |rise: f32| {
            Ball::ballistic_crossing(Vector3::zeros(), Vector3::new(pace, 0.0, rise), spin, at)
                .map_or(0.0, |(_, z, _)| z)
        };
        let (mut low, mut high) = (0.0_f32, 0.25_f32);
        for _ in 0..24 {
            let mid = 0.5 * (low + high);
            if height_with(mid) < height {
                low = mid;
            } else {
                high = mid;
            }
        }
        0.5 * (low + high)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kicks(execution: f32) -> Vec<DeadBallStrike> {
        let rng = MatchRng::from_seed(0x9E4A);
        (0..4_000)
            .map(|_| PenaltyKick::strike(execution, &rng))
            .collect()
    }

    fn on_target(kick: &DeadBallStrike) -> bool {
        !kick.over && kick.offset.abs() < GOAL_WIDTH
    }

    #[test]
    fn an_ordinary_taker_hits_the_target_about_fourteen_times_in_fifteen() {
        let all = kicks(0.70);
        let share = all.iter().filter(|k| on_target(k)).count() as f32 / all.len() as f32;
        assert!((0.91..0.95).contains(&share), "{share}");
    }

    #[test]
    fn a_better_taker_misses_less_and_goes_nearer_the_post() {
        let (good, poor) = (kicks(0.95), kicks(0.40));
        let rate = |all: &[DeadBallStrike]| {
            all.iter().filter(|k| on_target(k)).count() as f32 / all.len() as f32
        };
        let depth = |all: &[DeadBallStrike]| {
            let wide: Vec<f32> = all
                .iter()
                .filter(|k| on_target(k) && k.offset.abs() > 8.0)
                .map(|k| k.offset.abs())
                .collect();
            wide.iter().sum::<f32>() / wide.len() as f32
        };
        assert!(rate(&good) > rate(&poor));
        assert!(depth(&good) > depth(&poor));
    }

    /// The height of a kick as it passes `at` units out, on the flight the
    /// engine flies.
    fn height_at(kick: &DeadBallStrike, at: f32) -> f32 {
        let along = Vector3::new(1.0, 0.0, 0.0);
        let spin = SpinModel::clamped(SpinModel::from_strike(along, 0.0, -kick.dip.unwrap(), 1.0));
        let rise = Ball::launch_speed_for_apex(kick.apex);
        Ball::ballistic_crossing(
            Vector3::zeros(),
            Vector3::new(kick.pace.unwrap(), 0.0, rise),
            spin,
            at,
        )
        .map_or(0.0, |(_, z, _)| z)
    }

    fn free_kicks(execution: f32) -> Vec<DeadBallStrike> {
        let rng = MatchRng::from_seed(0x9E4A);
        (0..600)
            .map(|_| FreeKickShot::strike(execution, 176.0, Some(77.2), None, &rng))
            .collect()
    }

    #[test]
    fn a_free_kick_on_target_clears_the_wall_and_dips_under_the_bar() {
        for kick in free_kicks(0.6).iter().filter(|k| on_target(k)) {
            assert!(height_at(kick, 77.2) >= 2.39, "{kick:?}");
            assert!(height_at(kick, 176.0) <= 2.26, "{kick:?}");
        }
    }

    #[test]
    fn a_better_free_kick_taker_misses_less() {
        let rate = |all: &[DeadBallStrike]| {
            all.iter().filter(|k| on_target(k)).count() as f32 / all.len() as f32
        };
        let (good, poor) = (rate(&free_kicks(0.9)), rate(&free_kicks(0.3)));
        assert!(good > poor, "{good} vs {poor}");
        assert!((0.50..0.65).contains(&good), "{good}");
    }
}
