//! The moments a match is remembered by: the goals, the near misses,
//! and the rule that decides which of the near misses reach the match
//! sheet.
//!
//! [`GoalDetail`] and [`ChanceDetail`] are stamped as they happen;
//! [`HighlightSelector`] can only run at full time, because how good a
//! chance was is only answerable against the other chances the same team
//! had.

use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::result::{GOAL_CLIP_POST_ROLL_MS, GOAL_CLIP_PRE_ROLL_MS};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalDetail {
    pub player_id: u32,
    pub stat_type: MatchStatisticType,
    pub is_auto_goal: bool,
    pub time: u64,
}

/// A goal-scoring situation that did not end in a goal — the save, the post,
/// the one dragged wide from six yards.
///
/// Recorded for every strike that clears [`HighlightSelector::MIN_XG`], which
/// is more of them than any match sheet wants; [`HighlightSelector`] cuts that
/// down to the handful worth keeping once the whistle has gone and the whole
/// match is in view. None of it can be decided while it is happening: how good
/// a chance was is only answerable against the other chances the same team had.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChanceDetail {
    pub player_id: u32,
    /// The side that created it — the shooter's, always. There is no own-goal
    /// case here: a strike at your own net is not a chance anybody wants to
    /// watch twice.
    pub team_id: u32,
    /// Milliseconds into the match, taken the instant the ball was struck. The
    /// same clock a [`GoalDetail`] is stamped with, and the same one the
    /// recorder cuts its clips against — the two have to agree exactly or a
    /// marker points at footage that was never kept.
    pub time: u64,
    /// Recorded (Opta-scale) expected goals of the strike. Population mean is
    /// around 0.11, so anything past 0.25 is a genuinely big chance.
    pub xg: f32,
    /// Whether the strike was aimed between the posts and under the bar, as
    /// judged the moment it left the boot — NOT where it finished. A shot
    /// aimed on target and blocked by a defender still reads `true`.
    pub on_target: bool,
}

/// Which of a match's near misses are worth a marker and a clip.
///
/// Every shot is a candidate and most of them are nothing; the point of this is
/// to end up with the two or three moments per side a highlight reel would
/// actually carry. Three rules, in the order of what they protect:
///
/// 1. **Quality.** Ranked on xG and nothing else — how the chance ended is
///    beside the point, because a header cleared off the line and a sitter
///    dragged wide were the same chance until somebody made a decision.
/// 2. **A quota per side.** Otherwise a one-sided match marks eight moments for
///    the team on top and none at all for the side that spent it defending.
/// 3. **Spacing.** Two markers a rebound apart land on the same pixel of the
///    timeline and describe one passage of play twice. The gap is a fraction of
///    the match rather than a fixed number of seconds, because the rail IS the
///    match: it has to hold at ninety minutes and at the five-minute halves a
///    debug build plays.
pub struct HighlightSelector;

impl HighlightSelector {
    /// At most this many per side. Two or three is a highlight reel; six is the
    /// match again.
    pub const PER_TEAM: usize = 5;

    /// Recorded xG a strike has to reach to count as a chance at all.
    ///
    /// Low on purpose, and it can afford to be: the quota above takes the BEST
    /// of what clears it, so a permissive bar costs a good match nothing and
    /// only decides what a bad one falls back on. Its whole job is to keep out
    /// the strike that was never going in — the hopeful thirty-yarder, the
    /// half-blocked stab — not to define a chance.
    ///
    /// ⚠ CALIBRATED AGAINST THE ENGINE'S OWN SCALE, NOT AN OPTA ONE. Recorded
    /// xG runs through `XG_REPORT_SCALE`, and `dev_match stats 20 14 14` puts
    /// the population at 25.5 shots a match with a MEAN of 0.050 and a 90th
    /// percentile of 0.108 — so 0.035 is about the median attempt, not the
    /// "big chance" the same number would mean in a published model. At this
    /// bar, 40 matches yield 12.4 candidates each and the shortlist keeps 2.3
    /// per side, with 78% of team-matches getting two markers or more and 8%
    /// getting none (a side that genuinely never threatened). Anything that
    /// moves the recorded-xG scale moves this with it — re-measure, don't
    /// convert.
    pub const MIN_XG: f32 = 0.035;

    /// How far apart two markers have to be, as a divisor of the match length —
    /// a thirtieth of ninety minutes is three of them.
    const SPACING_DIVISOR: u64 = 45;

    /// Trims `chances` to the shortlist and returns the timestamps that
    /// survived, in the order they happened — which is exactly the list the
    /// recorder needs to know which of its provisional clips to keep.
    ///
    /// `goals` are already on the reel and are never dropped; they take part
    /// here only by pushing chances out of the seconds around them. A save two
    /// seconds before a goal is not a separate moment, it is the goal's
    /// build-up.
    pub fn select(
        chances: &mut Vec<ChanceDetail>,
        goals: &[GoalDetail],
        total_match_time: u64,
    ) -> Vec<u64> {
        let spacing = (total_match_time / Self::SPACING_DIVISOR)
            .max(GOAL_CLIP_PRE_ROLL_MS + GOAL_CLIP_POST_ROLL_MS);

        // Everything already on the reel, which is what the spacing rule
        // measures against. Goals join it up front; each chance joins it as it
        // is taken, so three chances can never bunch either.
        let mut taken: Vec<u64> = goals
            .iter()
            .filter(|goal| goal.stat_type == MatchStatisticType::Goal)
            .map(|goal| goal.time)
            .collect();

        let mut ranked: Vec<usize> = (0..chances.len())
            .filter(|&index| chances[index].xg >= Self::MIN_XG)
            .collect();
        // Best first, and the earlier of two equal chances first — so the
        // shortlist is a function of the match rather than of the order some
        // earlier sort happened to leave things in.
        ranked.sort_by(|&a, &b| {
            chances[b]
                .xg
                .total_cmp(&chances[a].xg)
                .then(chances[a].time.cmp(&chances[b].time))
        });

        let mut kept = vec![false; chances.len()];
        let mut per_team: HashMap<u32, usize> = HashMap::new();
        for index in ranked {
            let chance = &chances[index];
            let quota = per_team.entry(chance.team_id).or_insert(0);
            if *quota >= Self::PER_TEAM {
                continue;
            }
            if taken
                .iter()
                .any(|already| already.abs_diff(chance.time) < spacing)
            {
                continue;
            }
            *quota += 1;
            taken.push(chance.time);
            kept[index] = true;
        }

        let mut index = 0;
        chances.retain(|_| {
            let keep = kept[index];
            index += 1;
            keep
        });
        chances.sort_by_key(|chance| chance.time);
        chances.iter().map(|chance| chance.time).collect()
    }
}

/// What reaches the match sheet out of a match's near misses.
///
/// Every rule here is one that only shows up in aggregate, which is why none of
/// them can be checked by looking at a single shot: a quota that doesn't bind
/// leaves a one-sided match with six markers for one team, a spacing rule that
/// doesn't bind puts three of them on the same pixel, and a goal window that
/// doesn't bind marks the saved shot that the rebound went in from as though it
/// were a separate moment.
#[cfg(test)]
mod highlight_selector_tests {
    use super::*;

    const HOME: u32 = 1;
    const AWAY: u32 = 2;
    /// Ninety minutes, so the spacing rule is three of them.
    const FULL_TIME: u64 = 90 * 60_000;

    fn chance(team_id: u32, minute: u64, xg: f32) -> ChanceDetail {
        ChanceDetail {
            player_id: team_id * 100 + minute as u32,
            team_id,
            time: minute * 60_000,
            xg,
            on_target: true,
        }
    }

    fn goal(minute: u64) -> GoalDetail {
        GoalDetail {
            player_id: 999,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal: false,
            time: minute * 60_000,
        }
    }

    fn minutes(chances: &[ChanceDetail]) -> Vec<u64> {
        chances.iter().map(|c| c.time / 60_000).collect()
    }

    #[test]
    fn the_best_chances_are_the_ones_kept() {
        let mut chances = vec![
            chance(HOME, 10, 0.10),
            chance(HOME, 25, 0.40),
            chance(HOME, 40, 0.15),
            chance(HOME, 55, 0.30),
        ];
        let kept = HighlightSelector::select(&mut chances, &[], FULL_TIME);

        assert_eq!(
            minutes(&chances),
            vec![25, 40, 55],
            "the shortlist is not the three best chances"
        );
        // …and chronological, because that is the order the timeline draws
        // them in and the order the recorder walks its clips.
        assert_eq!(kept, vec![25 * 60_000, 40 * 60_000, 55 * 60_000]);
    }

    #[test]
    fn each_side_gets_its_own_quota() {
        // Everything the home side had is better than anything the away side
        // did — and the away side still gets its best moments. A team that
        // spent the match defending has near misses too, and a reel that only
        // shows the better team's is a highlight package of half a match.
        let mut chances = vec![
            chance(HOME, 5, 0.50),
            chance(HOME, 15, 0.48),
            chance(HOME, 25, 0.46),
            chance(HOME, 35, 0.44),
            chance(AWAY, 45, 0.12),
            chance(AWAY, 60, 0.11),
        ];
        HighlightSelector::select(&mut chances, &[], FULL_TIME);

        let home_kept = chances.iter().filter(|c| c.team_id == HOME).count();
        let away_kept = chances.iter().filter(|c| c.team_id == AWAY).count();
        assert_eq!(home_kept, HighlightSelector::PER_TEAM);
        assert_eq!(away_kept, 2, "the away side's own chances were crowded out");
    }

    #[test]
    fn one_passage_of_play_is_marked_once() {
        // A shot, the rebound, and the follow-up: three chances inside twenty
        // seconds. They are one moment, they would share one clip, and three
        // markers on top of each other is three seeks to the same footage.
        let mut chances = vec![
            ChanceDetail {
                time: 30 * 60_000,
                ..chance(HOME, 30, 0.30)
            },
            ChanceDetail {
                time: 30 * 60_000 + 8_000,
                ..chance(HOME, 30, 0.45)
            },
            ChanceDetail {
                time: 30 * 60_000 + 19_000,
                ..chance(HOME, 30, 0.28)
            },
        ];
        HighlightSelector::select(&mut chances, &[], FULL_TIME);

        assert_eq!(chances.len(), 1, "the same scramble was marked three times");
        assert_eq!(
            chances[0].time,
            30 * 60_000 + 8_000,
            "the moment kept was not the best of the three"
        );
    }

    #[test]
    fn the_shot_a_goal_came_from_is_not_a_separate_moment() {
        // The save at 70:00 and the goal off the rebound two seconds later are
        // one passage of play, and the goal is the part of it worth a marker.
        // The chance an hour earlier is untouched by the goal and survives.
        let mut chances = vec![
            chance(HOME, 10, 0.20),
            ChanceDetail {
                time: 70 * 60_000,
                ..chance(HOME, 70, 0.55)
            },
        ];
        HighlightSelector::select(&mut chances, &[goal(70)], FULL_TIME);

        assert_eq!(
            minutes(&chances),
            vec![10],
            "the build-up to a goal was marked as a chance of its own"
        );
    }

    #[test]
    fn a_hopeful_effort_is_not_a_chance() {
        // Under the bar it is not a goal-scoring situation, it is a shot. A
        // match of nothing but these gets no markers rather than three bad
        // ones — the reel says "nobody threatened", which is the truth.
        let mut chances = vec![
            chance(HOME, 10, HighlightSelector::MIN_XG - 0.01),
            chance(HOME, 30, 0.02),
            chance(AWAY, 50, 0.01),
        ];
        let kept = HighlightSelector::select(&mut chances, &[], FULL_TIME);

        assert!(chances.is_empty());
        assert!(kept.is_empty());
    }

    #[test]
    fn spacing_scales_with_the_match_rather_than_the_clock() {
        // A debug build plays five-minute halves. A fixed three-minute gap
        // would allow one marker per half there and the reel would be empty;
        // the rule is a fraction of the match, so a ten-minute match spaces its
        // markers twenty seconds apart and keeps all three.
        const SHORT: u64 = 10 * 60_000;
        let mut chances = vec![
            ChanceDetail {
                time: 60_000,
                ..chance(HOME, 1, 0.30)
            },
            ChanceDetail {
                time: 120_000,
                ..chance(HOME, 2, 0.35)
            },
            ChanceDetail {
                time: 180_000,
                ..chance(HOME, 3, 0.25)
            },
        ];
        HighlightSelector::select(&mut chances, &[], SHORT);

        assert_eq!(chances.len(), 3, "a short match lost its whole reel");
    }

    #[test]
    fn markers_never_land_closer_than_a_clip_apart() {
        // The floor under the spacing rule. However short the match, two clips
        // that overlap are one segment, and two markers over one segment are a
        // lie about how many moments it holds.
        const VERY_SHORT: u64 = 60_000;
        let mut chances = vec![
            ChanceDetail {
                time: 20_000,
                ..chance(HOME, 0, 0.30)
            },
            ChanceDetail {
                time: 25_000,
                ..chance(HOME, 0, 0.35)
            },
        ];
        HighlightSelector::select(&mut chances, &[], VERY_SHORT);

        assert_eq!(chances.len(), 1);
        assert_eq!(chances[0].time, 25_000);
    }
}
