//! **The match engine's acceptance bands.** One accumulator, fed with
//! finished match results, and the real-football range every line it
//! reports is held to. The `dev_match stats` harness prints the report and
//! the seeded batch test fails on any line out of its band.
//!
//! Everything is read off the result — the per-player stat lines, the goal
//! details and the match's own [`MatchTally`] — so matches played in
//! parallel can be recorded in any order.

use crate::r#match::engine::player::injury::InjuryGrade;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{GoalOrigin, MatchResultRaw, MatchTally, PassOriginRestart, PeriodKind};

#[derive(Debug, Default, Clone)]
pub struct MatchCalibrationStats {
    pub matches: u32,
    pub goals: u64,
    pub own_goals: u64,
    pub penalty_goals: u64,
    pub set_piece_goals: u64,
    pub shots: u64,
    pub shots_on_target: u64,
    pub xg_total: f64,
    pub passes_attempted: u64,
    pub passes_completed: u64,
    pub crosses: u64,
    pub crosses_completed: u64,
    pub fouls: u64,
    pub yellow_cards: u64,
    pub red_cards: u64,
    pub offsides: u64,
    pub saves: u64,
    pub dribbles_attempted: u64,
    pub dribbles_succeeded: u64,
    pub miscontrols: u64,
    pub key_passes: u64,
    pub progressive_passes: u64,
    pub progressive_carries: u64,
    pub successful_pressures: u64,
    pub errors_leading_to_shot: u64,
    pub errors_leading_to_goal: u64,
    pub home_wins: u64,
    pub draws: u64,
    pub away_wins: u64,
    pub home_goals: u64,
    pub away_goals: u64,
    pub tally: MatchTally,
}

#[derive(Debug, Clone, Copy)]
pub struct CalibrationLine {
    pub name: &'static str,
    pub value: f64,
    pub accept_min: f64,
    pub accept_max: f64,
}

impl CalibrationLine {
    pub fn in_band(&self) -> bool {
        self.value >= self.accept_min && self.value <= self.accept_max
    }
}

impl MatchCalibrationStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one finished match into the accumulator.
    pub fn record(&mut self, result: &MatchResultRaw) {
        self.matches += 1;

        for s in result.player_stats.values() {
            self.goals += s.goals as u64;
            self.own_goals += s.own_goals as u64;
            self.shots += s.shots_total as u64;
            self.shots_on_target += s.shots_on_target as u64;
            self.xg_total += s.xg as f64;
            self.passes_attempted += s.passes_attempted as u64;
            self.passes_completed += s.passes_completed as u64;
            self.crosses += s.crosses_attempted as u64;
            self.crosses_completed += s.crosses_completed as u64;
            self.fouls += s.fouls as u64;
            self.yellow_cards += s.yellow_cards as u64;
            self.red_cards += s.red_cards as u64;
            self.offsides += s.offsides as u64;
            self.saves += s.saves as u64;
            self.dribbles_attempted += s.attempted_dribbles as u64;
            self.dribbles_succeeded += s.successful_dribbles as u64;
            self.miscontrols += s.miscontrols as u64;
            self.key_passes += s.key_passes as u64;
            self.progressive_passes += s.progressive_passes as u64;
            self.progressive_carries += s.progressive_carries as u64;
            self.successful_pressures += s.successful_pressures as u64;
            self.errors_leading_to_shot += s.errors_leading_to_shot as u64;
            self.errors_leading_to_goal += s.errors_leading_to_goal as u64;
        }

        if let Some(score) = result.score.as_ref() {
            for goal in score
                .detail()
                .iter()
                .filter(|g| g.stat_type == MatchStatisticType::Goal)
            {
                if goal.origin == GoalOrigin::Penalty {
                    self.penalty_goals += 1;
                }
                if goal.origin.is_set_piece() {
                    self.set_piece_goals += 1;
                }
            }
            let (home, away) = (score.home_team.get(), score.away_team.get());
            self.home_goals += home as u64;
            self.away_goals += away as u64;
            match home.cmp(&away) {
                std::cmp::Ordering::Greater => self.home_wins += 1,
                std::cmp::Ordering::Less => self.away_wins += 1,
                std::cmp::Ordering::Equal => self.draws += 1,
            }
        }

        self.tally.absorb(&result.tally);
    }

    pub fn report_lines(&self) -> Vec<CalibrationLine> {
        let n = self.matches.max(1) as f64;
        let per = |v: f64| v / n;
        let ratio = |num: u64, den: u64| {
            if den == 0 {
                0.0
            } else {
                num as f64 / den as f64
            }
        };
        let all_goals = self.goals + self.own_goals;
        let decided = self.home_wins + self.draws + self.away_wins;
        let minutes = |ms: u64| per(ms as f64 / 60_000.0);
        let restarts = |origin: PassOriginRestart| per(self.tally.restarts_of(origin) as f64);
        let line = |name, value, accept_min, accept_max| CalibrationLine {
            name,
            value,
            accept_min,
            accept_max,
        };

        vec![
            line("goals/match", per(all_goals as f64), 2.3, 3.4),
            line("shots/match", per(self.shots as f64), 18.0, 32.0),
            line("shots-on-target %", ratio(self.shots_on_target, self.shots), 0.28, 0.42),
            line("xG/match", per(self.xg_total), 2.2, 3.4),
            line("save %", ratio(self.saves, self.shots_on_target), 0.62, 0.76),
            line("pass-completion %", ratio(self.passes_completed, self.passes_attempted), 0.75, 0.88),
            line("crosses/match", per(self.crosses as f64), 22.0, 45.0),
            line("cross-completion %", ratio(self.crosses_completed, self.crosses), 0.18, 0.32),
            line("dribbles attempted/match", per(self.dribbles_attempted as f64), 20.0, 45.0),
            line(
                "dribble success %",
                ratio(self.dribbles_succeeded, self.dribbles_attempted),
                0.35,
                0.55,
            ),
            line("miscontrols/match", per(self.miscontrols as f64), 8.0, 18.0),
            line("key passes/match", per(self.key_passes as f64), 12.0, 28.0),
            line("progressive passes/match", per(self.progressive_passes as f64), 30.0, 90.0),
            line("progressive carries/match", per(self.progressive_carries as f64), 12.0, 40.0),
            line("successful pressures/match", per(self.successful_pressures as f64), 20.0, 45.0),
            line("errors leading to shot/match", per(self.errors_leading_to_shot as f64), 1.0, 4.0),
            line("errors leading to goal/match", per(self.errors_leading_to_goal as f64), 0.05, 0.35),
            line("fouls/match", per(self.fouls as f64), 18.0, 32.0),
            line("yellow-cards/match", per(self.yellow_cards as f64), 2.5, 5.5),
            line("red-cards/match", per(self.red_cards as f64), 0.08, 0.28),
            line("offsides/match", per(self.offsides as f64), 2.0, 6.0),
            line("corners/match", restarts(PassOriginRestart::Corner), 7.0, 13.0),
            line("throw-ins/match", restarts(PassOriginRestart::ThrowIn), 30.0, 55.0),
            line("goal-kicks/match", restarts(PassOriginRestart::GoalKick), 10.0, 22.0),
            line("penalties/match", restarts(PassOriginRestart::Penalty), 0.18, 0.35),
            line("penalty goals/match", per(self.penalty_goals as f64), 0.12, 0.30),
            line("own goals/match", per(self.own_goals as f64), 0.04, 0.16),
            line("set-piece goal share", ratio(self.set_piece_goals, all_goals), 0.20, 0.36),
            line("ball-in-play minutes/match", minutes(self.tally.ball_in_play_ms), 52.0, 62.0),
            line(
                "first-half added minutes",
                minutes(self.tally.added_time_ms[PeriodKind::FirstHalf.index()]),
                1.0,
                4.0,
            ),
            line(
                "second-half added minutes",
                minutes(self.tally.added_time_ms[PeriodKind::SecondHalf.index()]),
                3.0,
                8.0,
            ),
            // Every hurt or worse is carried out of the match, so the band is
            // the world's season injury volume, not the real time-loss rate.
            line(
                "injuries (hurt or worse)/match",
                per(self.tally.injuries_from(InjuryGrade::Hurt) as f64),
                0.04,
                0.14,
            ),
            line(
                "home goal edge/match (equal teams)",
                per(self.home_goals as f64 - self.away_goals as f64),
                0.25,
                0.45,
            ),
            line("home win % (equal teams)", ratio(self.home_wins, decided), 0.42, 0.48),
            line("draw % (equal teams)", ratio(self.draws, decided), 0.23, 0.30),
            line("away win % (equal teams)", ratio(self.away_wins, decided), 0.27, 0.34),
        ]
    }

    pub fn print_report(&self) {
        println!("Calibration report ({} matches)", self.matches);
        for line in self.report_lines() {
            println!(
                "  [{:>3}] {:<32} {:>8.3}    accept [{:.3} .. {:.3}]",
                if line.in_band() { "OK" } else { "OUT" },
                line.name,
                line.value,
                line.accept_min,
                line.accept_max
            );
        }
    }

    /// The lines outside their acceptance range.
    pub fn out_of_range(&self) -> Vec<CalibrationLine> {
        self.report_lines()
            .into_iter()
            .filter(|l| !l.in_band())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerFieldPositionGroup;
    use crate::r#match::engine::result::{GoalDetail, PlayerMatchEndStats, Score};

    fn scorer() -> PlayerMatchEndStats {
        PlayerMatchEndStats {
            shots_on_target: 2,
            shots_total: 3,
            passes_attempted: 0,
            passes_completed: 0,
            tackles: 0,
            interceptions: 0,
            saves: 0,
            shots_faced: 0,
            goals: 1,
            assists: 0,
            match_rating: 7.0,
            raw_match_rating: 7.0,
            xg: 0.8,
            position_group: PlayerFieldPositionGroup::Forward,
            fouls: 2,
            yellow_cards: 0,
            red_cards: 0,
            minutes_played: 90,
            key_passes: 0,
            progressive_passes: 0,
            progressive_carries: 0,
            successful_dribbles: 0,
            attempted_dribbles: 0,
            successful_pressures: 0,
            pressures: 0,
            blocks: 0,
            clearances: 0,
            passes_into_box: 0,
            crosses_attempted: 0,
            crosses_completed: 0,
            xg_chain: 0.0,
            xg_buildup: 0.0,
            miscontrols: 0,
            heavy_touches: 0,
            carry_distance: 0,
            errors_leading_to_shot: 0,
            errors_leading_to_goal: 0,
            xg_prevented: 0.0,
            xg_faced: 0.0,
            offsides: 0,
            own_goals: 0,
            zone_stats: Default::default(),
        }
    }

    fn line(stats: &MatchCalibrationStats, name: &str) -> f64 {
        stats
            .report_lines()
            .into_iter()
            .find(|l| l.name == name)
            .map(|l| l.value)
            .unwrap()
    }

    #[test]
    fn empty_stats_read_zero() {
        let stats = MatchCalibrationStats::new();
        assert!(stats.report_lines().iter().all(|l| l.value == 0.0));
    }

    #[test]
    fn a_match_is_read_off_its_result_and_tally() {
        let mut result = MatchResultRaw::with_match_time(90 * 60 * 1000);
        let mut score = Score::new(1, 2);
        score.increment_home_goals();
        score.add_goal_detail(GoalDetail {
            player_id: 10,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal: false,
            time: 600_000,
            origin: GoalOrigin::Penalty,
        });
        result.score = Some(score);
        result.player_stats.insert(10, scorer());
        result.tally.note_restart(PassOriginRestart::Penalty);
        result.tally.note_restart(PassOriginRestart::Corner);

        let mut stats = MatchCalibrationStats::new();
        stats.record(&result);
        stats.record(&result);

        assert_eq!(line(&stats, "penalties/match"), 1.0);
        assert_eq!(line(&stats, "penalty goals/match"), 1.0);
        assert_eq!(line(&stats, "corners/match"), 1.0);
        assert_eq!(line(&stats, "set-piece goal share"), 1.0);
        assert_eq!(line(&stats, "fouls/match"), 2.0);
        assert_eq!(line(&stats, "home win % (equal teams)"), 1.0);
    }
}
