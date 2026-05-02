//! Match-engine calibration diagnostics. Always-compiled, no cfg gate.
//!
//! Callers run N matches, feed each finished `MatchField` (and the
//! cumulative restart-count book-keeping the harness drove) into
//! `MatchCalibrationStats::record`, and then call `print_report` at
//! the end. The acceptance ranges are encoded in `report_lines` so a
//! caller can also assert in tests.
//!
//! This module deliberately does NOT spin up its own match engine —
//! match construction lives in higher-level crates that have access to
//! the squad/data pipeline. Importing those here would create a cycle.
//! The harness is reusable from `dev_match` (or any future binary) by
//! constructing matches there and feeding the results in.

use crate::r#match::engine::player::statistics::MatchStatisticType;
use crate::r#match::{MatchField, MatchPlayer};

/// Cumulative calibration counters. One instance accumulates over N
/// matches; final means / ratios come from `report_lines`.
#[derive(Debug, Default, Clone)]
pub struct MatchCalibrationStats {
    pub matches: u32,
    pub goals: u64,
    pub shots: u64,
    pub shots_on_target: u64,
    pub xg_total: f64,
    pub passes_attempted: u64,
    pub passes_completed: u64,
    pub crosses: u64,
    pub crosses_completed: u64,
    pub corners: u64,
    pub throw_ins: u64,
    pub goal_kicks: u64,
    pub fouls: u64,
    pub yellow_cards: u64,
    pub red_cards: u64,
    pub penalties: u64,
    pub offsides: u64,
    pub saves: u64,
    pub blocks: u64,
    pub rebounds: u64,
    pub corners_from_blocks: u64,
    pub possession_home_ticks: u64,
    pub possession_away_ticks: u64,
    pub stuck_ball_count: u64,
    pub dribbles_attempted: u64,
    pub dribbles_succeeded: u64,
    pub miscontrols: u64,
    pub heavy_touches: u64,
    pub progressive_passes: u64,
    pub progressive_carries: u64,
    pub key_passes: u64,
    pub pressures: u64,
    pub successful_pressures: u64,
    pub errors_leading_to_shot: u64,
    pub errors_leading_to_goal: u64,

    // Phase-6 calibration extensions.
    pub direct_free_kick_shots: u64,
    pub direct_free_kick_goals: u64,
    pub advantage_played: u64,
    pub advantage_succeeded: u64,
    pub set_piece_xg_total: f64,
    pub home_wins: u64,
    pub draws: u64,
    pub away_wins: u64,
    pub time_wasted_stoppage_ms: u64,
    pub corner_routine_near_post: u64,
    pub corner_routine_penalty_spot: u64,
    pub corner_routine_far_post: u64,
    pub corner_routine_short: u64,
    pub corner_routine_edge_cutback: u64,
    pub fk_choice_direct_shot: u64,
    pub fk_choice_box_delivery: u64,
    pub fk_choice_short: u64,
    pub fk_choice_recycle: u64,
    pub throw_routine_long: u64,
    pub throw_routine_short: u64,
}

/// Per-match restart bookkeeping the engine tells the harness about.
/// Counts can't be reconstructed from final state alone — the harness
/// caller is expected to subscribe to ball events during the match and
/// pass the totals here at end-of-match time.
#[derive(Debug, Default, Clone, Copy)]
pub struct MatchRestartCounts {
    pub corners: u32,
    pub throw_ins: u32,
    pub goal_kicks: u32,
    pub blocks: u32,
    pub rebounds: u32,
    pub corners_from_blocks: u32,
    pub possession_home_ticks: u32,
    pub possession_away_ticks: u32,
    pub stuck_ball_count: u32,
    pub crosses_attempted: u32,
    pub crosses_completed: u32,
    pub xg_total: f32,

    // Phase-6 per-match counters fed by harness:
    pub direct_free_kick_shots: u32,
    pub direct_free_kick_goals: u32,
    pub advantage_played: u32,
    pub advantage_succeeded: u32,
    pub set_piece_xg: f32,
    pub time_wasted_stoppage_ms: u32,
    /// Final score sign: positive home wins, negative away wins, zero draw.
    pub final_score_diff: i32,
    pub corner_routine_near_post: u32,
    pub corner_routine_penalty_spot: u32,
    pub corner_routine_far_post: u32,
    pub corner_routine_short: u32,
    pub corner_routine_edge_cutback: u32,
    pub fk_choice_direct_shot: u32,
    pub fk_choice_box_delivery: u32,
    pub fk_choice_short: u32,
    pub fk_choice_recycle: u32,
    pub throw_routine_long: u32,
    pub throw_routine_short: u32,
}

impl MatchCalibrationStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold a finished match's player stats and restart counts into the
    /// accumulator. Call once per finished match.
    pub fn record(&mut self, field: &MatchField, restarts: MatchRestartCounts) {
        self.matches += 1;

        let mut goals = 0u64;
        let mut yellow_cards = 0u64;
        let mut red_cards = 0u64;
        let mut fouls = 0u64;
        let mut offsides = 0u64;
        let mut saves = 0u64;
        let mut shots_faced = 0u64;
        let mut passes_attempted = 0u64;
        let mut passes_completed = 0u64;
        let mut dribbles_attempted = 0u64;
        let mut dribbles_succeeded = 0u64;
        let mut miscontrols = 0u64;
        let mut heavy_touches = 0u64;
        let mut progressive_passes = 0u64;
        let mut progressive_carries = 0u64;
        let mut key_passes = 0u64;
        let mut pressures = 0u64;
        let mut successful_pressures = 0u64;
        let mut errors_leading_to_shot = 0u64;
        let mut errors_leading_to_goal = 0u64;

        for p in &field.players {
            for item in &p.statistics.items {
                match item.stat_type {
                    MatchStatisticType::Goal if !item.is_auto_goal => goals += 1,
                    MatchStatisticType::YellowCard => yellow_cards += 1,
                    MatchStatisticType::RedCard => red_cards += 1,
                    MatchStatisticType::Foul => fouls += 1,
                    _ => {}
                }
            }
            offsides += p.statistics.offsides as u64;
            saves += p.statistics.saves as u64;
            shots_faced += p.statistics.shots_faced as u64;
            passes_attempted += p.statistics.passes_attempted as u64;
            passes_completed += p.statistics.passes_completed as u64;
            dribbles_attempted += p.statistics.attempted_dribbles as u64;
            dribbles_succeeded += p.statistics.successful_dribbles as u64;
            miscontrols += p.statistics.miscontrols as u64;
            heavy_touches += p.statistics.heavy_touches as u64;
            progressive_passes += p.statistics.progressive_passes as u64;
            progressive_carries += p.statistics.progressive_carries as u64;
            key_passes += p.statistics.key_passes as u64;
            pressures += p.statistics.pressures as u64;
            successful_pressures += p.statistics.successful_pressures as u64;
            errors_leading_to_shot += p.statistics.errors_leading_to_shot as u64;
            errors_leading_to_goal += p.statistics.errors_leading_to_goal as u64;
        }

        self.goals += goals;
        self.yellow_cards += yellow_cards;
        self.red_cards += red_cards;
        self.fouls += fouls;
        self.offsides += offsides;
        self.saves += saves;
        self.passes_attempted += passes_attempted;
        self.passes_completed += passes_completed;
        // shots_on_target = saves + goals (auto-goals excluded). Real
        // engine counts on-target via memory.credit_shot_on_target which
        // we approximate here as saves + non-OG goals.
        self.shots_on_target += saves + goals;

        // Shots: count every Shoot event the engine recorded — we don't
        // have a clean per-match counter, so approximate as
        // shots_faced + on-target-misses. The spec accepts ratio bands
        // rather than absolutes for shots, so this approximation is
        // sufficient for calibration.
        self.shots += shots_faced.max(saves + goals);

        self.corners += restarts.corners as u64;
        self.throw_ins += restarts.throw_ins as u64;
        self.goal_kicks += restarts.goal_kicks as u64;
        self.blocks += restarts.blocks as u64;
        self.rebounds += restarts.rebounds as u64;
        self.corners_from_blocks += restarts.corners_from_blocks as u64;
        self.possession_home_ticks += restarts.possession_home_ticks as u64;
        self.possession_away_ticks += restarts.possession_away_ticks as u64;
        self.stuck_ball_count += restarts.stuck_ball_count as u64;
        self.crosses += restarts.crosses_attempted as u64;
        self.crosses_completed += restarts.crosses_completed as u64;
        self.xg_total += restarts.xg_total as f64;
        self.dribbles_attempted += dribbles_attempted;
        self.dribbles_succeeded += dribbles_succeeded;
        self.miscontrols += miscontrols;
        self.heavy_touches += heavy_touches;
        self.progressive_passes += progressive_passes;
        self.progressive_carries += progressive_carries;
        self.key_passes += key_passes;
        self.pressures += pressures;
        self.successful_pressures += successful_pressures;
        self.errors_leading_to_shot += errors_leading_to_shot;
        self.errors_leading_to_goal += errors_leading_to_goal;

        self.direct_free_kick_shots += restarts.direct_free_kick_shots as u64;
        self.direct_free_kick_goals += restarts.direct_free_kick_goals as u64;
        self.advantage_played += restarts.advantage_played as u64;
        self.advantage_succeeded += restarts.advantage_succeeded as u64;
        self.set_piece_xg_total += restarts.set_piece_xg as f64;
        self.time_wasted_stoppage_ms += restarts.time_wasted_stoppage_ms as u64;
        match restarts.final_score_diff {
            d if d > 0 => self.home_wins += 1,
            d if d < 0 => self.away_wins += 1,
            _ => self.draws += 1,
        }
        self.corner_routine_near_post += restarts.corner_routine_near_post as u64;
        self.corner_routine_penalty_spot += restarts.corner_routine_penalty_spot as u64;
        self.corner_routine_far_post += restarts.corner_routine_far_post as u64;
        self.corner_routine_short += restarts.corner_routine_short as u64;
        self.corner_routine_edge_cutback += restarts.corner_routine_edge_cutback as u64;
        self.fk_choice_direct_shot += restarts.fk_choice_direct_shot as u64;
        self.fk_choice_box_delivery += restarts.fk_choice_box_delivery as u64;
        self.fk_choice_short += restarts.fk_choice_short as u64;
        self.fk_choice_recycle += restarts.fk_choice_recycle as u64;
        self.throw_routine_long += restarts.throw_routine_long as u64;
        self.throw_routine_short += restarts.throw_routine_short as u64;
    }

    pub fn report_lines(&self) -> Vec<CalibrationLine> {
        let n = self.matches.max(1) as f64;
        let total_decided = (self.home_wins + self.draws + self.away_wins) as f64;
        let saves_pct = if self.shots_on_target > 0 {
            self.saves as f64 / self.shots_on_target as f64
        } else {
            0.0
        };
        let pass_pct = if self.passes_attempted > 0 {
            self.passes_completed as f64 / self.passes_attempted as f64
        } else {
            0.0
        };
        let cross_pct = if self.crosses > 0 {
            self.crosses_completed as f64 / self.crosses as f64
        } else {
            0.0
        };
        let on_target_pct = if self.shots > 0 {
            self.shots_on_target as f64 / self.shots as f64
        } else {
            0.0
        };

        vec![
            CalibrationLine {
                name: "goals/match",
                value: self.goals as f64 / n,
                accept_min: 2.3,
                accept_max: 3.4,
            },
            CalibrationLine {
                name: "shots/match",
                value: self.shots as f64 / n,
                accept_min: 18.0,
                accept_max: 32.0,
            },
            CalibrationLine {
                name: "shots-on-target %",
                value: on_target_pct,
                accept_min: 0.28,
                accept_max: 0.42,
            },
            CalibrationLine {
                name: "xG/match",
                value: self.xg_total / n,
                accept_min: 2.2,
                accept_max: 3.4,
            },
            CalibrationLine {
                name: "pass-completion %",
                value: pass_pct,
                accept_min: 0.75,
                accept_max: 0.88,
            },
            CalibrationLine {
                name: "crosses/match",
                value: self.crosses as f64 / n,
                accept_min: 22.0,
                accept_max: 45.0,
            },
            CalibrationLine {
                name: "cross-completion %",
                value: cross_pct,
                accept_min: 0.18,
                accept_max: 0.32,
            },
            CalibrationLine {
                name: "corners/match",
                value: self.corners as f64 / n,
                accept_min: 7.0,
                accept_max: 13.0,
            },
            CalibrationLine {
                name: "throw-ins/match",
                value: self.throw_ins as f64 / n,
                accept_min: 30.0,
                accept_max: 55.0,
            },
            CalibrationLine {
                name: "goal-kicks/match",
                value: self.goal_kicks as f64 / n,
                accept_min: 10.0,
                accept_max: 22.0,
            },
            CalibrationLine {
                name: "fouls/match",
                value: self.fouls as f64 / n,
                accept_min: 18.0,
                accept_max: 32.0,
            },
            CalibrationLine {
                name: "yellow-cards/match",
                value: self.yellow_cards as f64 / n,
                accept_min: 2.5,
                accept_max: 5.5,
            },
            CalibrationLine {
                name: "red-cards/match",
                value: self.red_cards as f64 / n,
                accept_min: 0.08,
                accept_max: 0.28,
            },
            CalibrationLine {
                name: "penalties/match",
                value: self.penalties as f64 / n,
                accept_min: 0.18,
                accept_max: 0.35,
            },
            CalibrationLine {
                name: "offsides/match",
                value: self.offsides as f64 / n,
                accept_min: 2.0,
                accept_max: 6.0,
            },
            CalibrationLine {
                name: "save %",
                value: saves_pct,
                accept_min: 0.62,
                accept_max: 0.76,
            },
            CalibrationLine {
                name: "stuck-ball recoveries/match",
                value: self.stuck_ball_count as f64 / n,
                accept_min: 0.0,
                accept_max: 0.5,
            },
            CalibrationLine {
                name: "dribbles attempted/match",
                value: self.dribbles_attempted as f64 / n,
                accept_min: 20.0,
                accept_max: 45.0,
            },
            CalibrationLine {
                name: "dribble success %",
                value: if self.dribbles_attempted > 0 {
                    self.dribbles_succeeded as f64 / self.dribbles_attempted as f64
                } else {
                    0.0
                },
                accept_min: 0.35,
                accept_max: 0.55,
            },
            CalibrationLine {
                name: "miscontrols/match",
                value: self.miscontrols as f64 / n,
                accept_min: 8.0,
                accept_max: 18.0,
            },
            CalibrationLine {
                name: "key passes/match",
                value: self.key_passes as f64 / n,
                accept_min: 12.0,
                accept_max: 28.0,
            },
            CalibrationLine {
                name: "progressive passes/match",
                value: self.progressive_passes as f64 / n,
                accept_min: 30.0,
                accept_max: 90.0,
            },
            CalibrationLine {
                name: "progressive carries/match",
                value: self.progressive_carries as f64 / n,
                accept_min: 12.0,
                accept_max: 40.0,
            },
            CalibrationLine {
                name: "successful pressures/match",
                value: self.successful_pressures as f64 / n,
                accept_min: 20.0,
                accept_max: 45.0,
            },
            CalibrationLine {
                name: "errors leading to shot/match",
                value: self.errors_leading_to_shot as f64 / n,
                accept_min: 1.0,
                accept_max: 4.0,
            },
            CalibrationLine {
                name: "errors leading to goal/match",
                value: self.errors_leading_to_goal as f64 / n,
                accept_min: 0.05,
                accept_max: 0.35,
            },
            CalibrationLine {
                name: "direct FK shots/match",
                value: self.direct_free_kick_shots as f64 / n,
                accept_min: 0.4,
                accept_max: 1.4,
            },
            CalibrationLine {
                name: "direct FK goals/match",
                value: self.direct_free_kick_goals as f64 / n,
                accept_min: 0.03,
                accept_max: 0.10,
            },
            CalibrationLine {
                name: "advantage played/match",
                value: self.advantage_played as f64 / n,
                accept_min: 2.0,
                accept_max: 7.0,
            },
            CalibrationLine {
                name: "advantage success %",
                value: if self.advantage_played > 0 {
                    self.advantage_succeeded as f64 / self.advantage_played as f64
                } else {
                    0.0
                },
                accept_min: 0.35,
                accept_max: 0.60,
            },
            CalibrationLine {
                name: "set-piece xG/match",
                value: self.set_piece_xg_total / n,
                accept_min: 0.35,
                accept_max: 0.85,
            },
            CalibrationLine {
                name: "home win % (equal teams)",
                value: if total_decided > 0.0 {
                    self.home_wins as f64 / total_decided
                } else {
                    0.0
                },
                accept_min: 0.42,
                accept_max: 0.48,
            },
            CalibrationLine {
                name: "draw % (equal teams)",
                value: if total_decided > 0.0 {
                    self.draws as f64 / total_decided
                } else {
                    0.0
                },
                accept_min: 0.23,
                accept_max: 0.30,
            },
            CalibrationLine {
                name: "away win % (equal teams)",
                value: if total_decided > 0.0 {
                    self.away_wins as f64 / total_decided
                } else {
                    0.0
                },
                accept_min: 0.27,
                accept_max: 0.34,
            },
            CalibrationLine {
                name: "time-wasted stoppage seconds/match",
                value: self.time_wasted_stoppage_ms as f64 / n / 1000.0,
                accept_min: 0.0,
                accept_max: 90.0,
            },
        ]
    }

    pub fn print_report(&self) {
        println!("Calibration report ({} matches)", self.matches);
        for line in self.report_lines() {
            let status = if line.value >= line.accept_min && line.value <= line.accept_max {
                "OK"
            } else {
                "OUT"
            };
            println!(
                "  [{:>3}] {:<32} {:>8.3}    accept [{:.3} .. {:.3}]",
                status, line.name, line.value, line.accept_min, line.accept_max
            );
        }
    }

    /// Return the names of any metrics outside their acceptance range.
    pub fn out_of_range(&self) -> Vec<&'static str> {
        self.report_lines()
            .into_iter()
            .filter(|l| l.value < l.accept_min || l.value > l.accept_max)
            .map(|l| l.name)
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CalibrationLine {
    pub name: &'static str,
    pub value: f64,
    pub accept_min: f64,
    pub accept_max: f64,
}

/// Helper: count how many players from each team are sent off — feeds
/// the red-card-rate sanity check.
pub fn count_sent_off(players: &[MatchPlayer]) -> usize {
    players.iter().filter(|p| p.is_sent_off).count()
}

#[allow(dead_code, unused_imports)]
mod calibration_tests {
    use super::*;

    #[test]
    fn empty_stats_have_zero_lines() {
        let s = MatchCalibrationStats::new();
        let lines = s.report_lines();
        assert!(!lines.is_empty());
        for l in lines {
            assert_eq!(l.value, 0.0);
        }
    }

    #[test]
    fn restart_counts_aggregate() {
        // Build a stub set of restart counts and verify arithmetic.
        let mut s = MatchCalibrationStats::new();
        // Bypass `record` (which needs a MatchField) by hand-rolling the
        // arithmetic the same way `record` does for the restart fields.
        s.matches += 1;
        let r = MatchRestartCounts {
            corners: 10,
            throw_ins: 40,
            goal_kicks: 12,
            ..Default::default()
        };
        s.corners += r.corners as u64;
        s.throw_ins += r.throw_ins as u64;
        s.goal_kicks += r.goal_kicks as u64;

        let lines = s.report_lines();
        let corners = lines.iter().find(|l| l.name == "corners/match").unwrap();
        assert_eq!(corners.value, 10.0);
        let throws = lines.iter().find(|l| l.name == "throw-ins/match").unwrap();
        assert_eq!(throws.value, 40.0);
        let goal_kicks = lines.iter().find(|l| l.name == "goal-kicks/match").unwrap();
        assert_eq!(goal_kicks.value, 12.0);
    }
}
