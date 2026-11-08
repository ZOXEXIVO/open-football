//! Rating everyone who played, once every stat line exists.
//!
//! The contextual rating reads team-behaviour summaries folded from EVERY
//! player's stat line — who shot, who held the ball, who had to defend — so
//! the lines have to be finished first. Then each player is scored against
//! his own side's summary, the opponent's, and his own physical snapshot.

use super::{EngineVolumeCalibration, RatingContext, RatingExpectationContext, TeamRatingSummary};
use crate::r#match::{MatchResultRaw, PlayerMatchEndStats};

/// The units a result's stat lines are written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatingVolumes {
    /// The engine's raw counters, which run hotter than real football and
    /// are converted before the model reads them (see
    /// [`EngineVolumeCalibration`]). Persisted stats stay raw; only the
    /// rating input is converted.
    EngineCounters,
    /// Already real-football per-90 volumes: what the model is calibrated on.
    RealUnits,
}

/// Fills in every player's rating on a result whose stat lines are complete.
pub struct MatchRatingPass;

impl MatchRatingPass {
    /// `raw_match_rating` stays the pure stat-line verdict; the public
    /// `match_rating` seed layers team expectation and condition on top.
    /// Reputation and personality shaping is added later by the league
    /// pipeline.
    pub fn rate(result: &mut MatchResultRaw, volumes: RatingVolumes) {
        let score = result.score.as_ref().expect("a finished match has a score");
        let home_goals = score.home_team.get();
        let away_goals = score.away_team.get();
        let home_team_id = score.home_team.team_id;

        let left_summary = {
            let side = &result.left_team_players;
            TeamRatingSummary::from_stats(
                side.main
                    .iter()
                    .chain(side.substitutes_used.iter())
                    .filter_map(|id| result.player_stats.get(id)),
            )
        };
        let right_summary = {
            let side = &result.right_team_players;
            TeamRatingSummary::from_stats(
                side.main
                    .iter()
                    .chain(side.substitutes_used.iter())
                    .filter_map(|id| result.player_stats.get(id)),
            )
        };
        let left_team_id = result.left_team_players.team_id;

        let rated_ids: Vec<u32> = result.player_stats.keys().copied().collect();
        for pid in rated_ids {
            let on_left = result.left_team_players.main.contains(&pid)
                || result.left_team_players.substitutes_used.contains(&pid);
            let team_id = if on_left {
                left_team_id
            } else {
                result.right_team_players.team_id
            };
            let (team_goals, opponent_goals) = if team_id == home_team_id {
                (home_goals, away_goals)
            } else {
                (away_goals, home_goals)
            };
            let (own_summary, opp_summary) = if on_left {
                (&left_summary, &right_summary)
            } else {
                (&right_summary, &left_summary)
            };
            let expectation = RatingExpectationContext::from_match(
                own_summary,
                opp_summary,
                team_goals,
                opponent_goals,
                result.physical_snapshots.get(&pid),
            );
            if let Some(stats) = result.player_stats.get_mut(&pid) {
                let rate = |line: &PlayerMatchEndStats| {
                    let ctx = RatingContext::new(line, team_goals, opponent_goals);
                    (ctx.calculate(), ctx.calculate_contextual(&expectation))
                };
                let (raw, contextual) = match volumes {
                    RatingVolumes::EngineCounters => {
                        rate(&EngineVolumeCalibration::normalize(stats))
                    }
                    RatingVolumes::RealUnits => rate(stats),
                };
                stats.raw_match_rating = raw;
                stats.match_rating = contextual;
            }
        }
    }
}
