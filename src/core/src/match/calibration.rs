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
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::delivery_diag::DeliveryCensus;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::result::team_census::LedgerState;
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
    /// Each match's final score, home first.
    pub scores: Vec<(u8, u8)>,
    /// The shape of every scoreline, summed over the batch.
    pub timeline: GoalTimeline,
    /// Goals by fifteen-minute period of play, stoppage time in the period
    /// it was added to.
    pub goal_profile: [u64; 6],
    /// Goals by minute of the match clock, for the independent reference.
    pub goal_minutes: Vec<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct CalibrationLine {
    pub name: &'static str,
    pub value: f64,
    pub accept_min: f64,
    pub accept_max: f64,
    /// A tracked line is reported and diffed against the baseline but never
    /// out of band; its range is the real reference, when there is one.
    pub checked: bool,
}

impl CalibrationLine {
    pub fn in_band(&self) -> bool {
        !self.checked || (self.value >= self.accept_min && self.value <= self.accept_max)
    }
}

/// One scoreline read for its shape: how long each side spent ahead, level
/// and behind before and after the 62nd minute and what it scored there,
/// which goals put a side ahead and how many of them were answered within
/// fifteen minutes, and how often the lead changed hands. Summed over a
/// batch with [`absorb`](Self::absorb).
#[derive(Debug, Clone, Copy, Default)]
pub struct GoalTimeline {
    /// `[leading, level, trailing][before 62', from 62']`, in team-ms.
    pub state_ms: [[f64; 2]; 3],
    pub state_goals: [[u32; 2]; 3],
    /// Goals that took a side from level into the lead.
    pub go_ahead: u32,
    /// …and the ones after which the score was level again within 15'.
    pub go_ahead_equalised: u32,
    pub lead_changes: u32,
    /// Matches in which the lead changed hands at least once.
    pub matches_with_lead_change: u32,
}

impl GoalTimeline {
    pub const ERA_SPLIT_MS: u64 = 62 * 60_000;
    pub const EQUALISER_WINDOW_MS: u64 = 15 * 60_000;

    /// `goals` in the order scored, each with whether the home side scored
    /// it; `end_ms` is when the match ended.
    pub fn read(goals: &[(u64, bool)], end_ms: u64) -> Self {
        let mut t = Self::default();
        let (mut home, mut away) = (0i32, 0i32);
        let mut previous = 0u64;
        let mut leader: Option<bool> = None;
        for (i, &(time, home_scored)) in goals.iter().enumerate() {
            t.add_spell(previous, time, home - away);
            let home_state = Self::state(home - away);
            let era = usize::from(time >= Self::ERA_SPLIT_MS);
            let scorer_state = if home_scored {
                home_state
            } else {
                2 - home_state
            };
            t.state_goals[scorer_state][era] += 1;
            let before = home - away;
            if home_scored {
                home += 1;
            } else {
                away += 1;
            }
            let after = home - away;
            if before == 0 {
                t.go_ahead += 1;
                let mut difference = after;
                for &(later, home_later) in &goals[i + 1..] {
                    if later.saturating_sub(time) > Self::EQUALISER_WINDOW_MS {
                        break;
                    }
                    difference += if home_later { 1 } else { -1 };
                    if difference == 0 {
                        t.go_ahead_equalised += 1;
                        break;
                    }
                }
            }
            let now = (after != 0).then_some(after > 0);
            if let (Some(was), Some(is)) = (leader, now)
                && was != is
            {
                t.lead_changes += 1;
            }
            if now.is_some() {
                leader = now;
            }
            previous = time;
        }
        t.add_spell(previous, end_ms.max(previous), home - away);
        t.matches_with_lead_change = u32::from(t.lead_changes > 0);
        t
    }

    pub fn absorb(&mut self, other: &GoalTimeline) {
        for (mine, theirs) in self.state_ms.iter_mut().zip(other.state_ms.iter()) {
            mine[0] += theirs[0];
            mine[1] += theirs[1];
        }
        for (mine, theirs) in self.state_goals.iter_mut().zip(other.state_goals.iter()) {
            mine[0] += theirs[0];
            mine[1] += theirs[1];
        }
        self.go_ahead += other.go_ahead;
        self.go_ahead_equalised += other.go_ahead_equalised;
        self.lead_changes += other.lead_changes;
        self.matches_with_lead_change += other.matches_with_lead_change;
    }

    /// A leading side's scoring rate over a trailing side's, in one era.
    pub fn lead_trail_ratio(&self, era: usize) -> f64 {
        let rate = |state: usize| {
            if self.state_ms[state][era] <= 0.0 {
                0.0
            } else {
                self.state_goals[state][era] as f64 / self.state_ms[state][era]
            }
        };
        let trailing = rate(2);
        if trailing <= 0.0 {
            0.0
        } else {
            rate(0) / trailing
        }
    }

    fn state(home_difference: i32) -> usize {
        match home_difference {
            d if d > 0 => 0,
            0 => 1,
            _ => 2,
        }
    }

    fn add_spell(&mut self, from: u64, to: u64, home_difference: i32) {
        let home = Self::state(home_difference);
        let split = Self::ERA_SPLIT_MS;
        let before = to.min(split).saturating_sub(from.min(split)) as f64;
        let after = to.max(split).saturating_sub(from.max(split)) as f64;
        for state in [home, 2 - home] {
            self.state_ms[state][0] += before;
            self.state_ms[state][1] += after;
        }
    }
}

/// What two sides scoring independently of each other, at a batch's own
/// rates and with its own goal times, make of the equaliser and the lead
/// change: the reference the batch's own shares are read beside.
pub struct IndependentScoring;

impl IndependentScoring {
    const MATCHES: u32 = 20_000;

    /// `(go-ahead goals equalised within 15', matches with a lead change)`.
    pub fn reference(home_rate: f64, away_rate: f64, goal_minutes: &[u32]) -> (f64, f64) {
        let total: u64 = goal_minutes.iter().map(|&n| n as u64).sum();
        if total == 0 {
            return (0.0, 0.0);
        }
        let mut state = 0x7EA4_5C0E_D15E_A5E5u64;
        let mut timeline = GoalTimeline::default();
        let mut goals: Vec<(u64, bool)> = Vec::new();
        for _ in 0..Self::MATCHES {
            goals.clear();
            for (rate, home) in [(home_rate, true), (away_rate, false)] {
                for _ in 0..Self::poisson(rate, &mut state) {
                    let mut pick = (Self::unit(&mut state) * total as f64) as u64;
                    let minute = goal_minutes
                        .iter()
                        .position(|&n| {
                            if pick < n as u64 {
                                true
                            } else {
                                pick -= n as u64;
                                false
                            }
                        })
                        .unwrap_or(goal_minutes.len() - 1);
                    let ms = (minute as f64 + Self::unit(&mut state)) * 60_000.0;
                    goals.push((ms as u64, home));
                }
            }
            goals.sort_by_key(|&(time, _)| time);
            timeline.absorb(&GoalTimeline::read(&goals, 0));
        }
        let equalised = if timeline.go_ahead == 0 {
            0.0
        } else {
            timeline.go_ahead_equalised as f64 / timeline.go_ahead as f64
        };
        (
            equalised,
            timeline.matches_with_lead_change as f64 / Self::MATCHES as f64,
        )
    }

    fn poisson(mean: f64, state: &mut u64) -> u32 {
        let limit = (-mean).exp();
        let mut product = Self::unit(state);
        let mut count = 0;
        while product > limit && count < 30 {
            product *= Self::unit(state);
            count += 1;
        }
        count
    }

    /// splitmix64 into `[0, 1)`.
    fn unit(state: &mut u64) -> f64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
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

        if let Some(score) = result.score.as_ref() {
            self.scores
                .push((score.home_team.get(), score.away_team.get()));
        }
        let goals = result.goal_timeline();
        self.timeline
            .absorb(&GoalTimeline::read(&goals, result.match_time_ms));
        let first_half_ends = 45 * 60_000 + result.tally.added_time_ms[PeriodKind::FirstHalf.index()];
        let second_half_ends = first_half_ends
            + 45 * 60_000
            + result.tally.added_time_ms[PeriodKind::SecondHalf.index()];
        for &(time, _) in &goals {
            let minute = (time / 60_000) as usize;
            if self.goal_minutes.len() <= minute {
                self.goal_minutes.resize(minute + 1, 0);
            }
            self.goal_minutes[minute] += 1;
            let period = if time < first_half_ends {
                (time / (15 * 60_000)).min(2)
            } else if time < second_half_ends {
                3 + ((time - first_half_ends) / (15 * 60_000)).min(2)
            } else {
                continue;
            };
            self.goal_profile[period as usize] += 1;
        }
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
            checked: true,
        };
        let tracked = |name, value, reference| CalibrationLine {
            name,
            value,
            accept_min: reference,
            accept_max: reference,
            checked: false,
        };
        let (home_rate, away_rate) = (per(self.home_goals as f64), per(self.away_goals as f64));
        let (equalised_if_independent, lead_change_if_independent) =
            IndependentScoring::reference(home_rate, away_rate, &self.goal_minutes);
        let profile_goals: u64 = self.goal_profile.iter().sum();
        let profile = |period: usize| ratio(self.goal_profile[period], profile_goals);

        #[allow(unused_mut)]
        let mut lines = vec![
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
            line("goal correlation (equal teams)", self.goal_correlation(), -0.08, 0.08),
            line("draw surplus over independent", self.draw_surplus(), -0.03, 0.03),
            line("lead/trail scoring rate <62'", self.timeline.lead_trail_ratio(0), 0.75, 1.33),
            line("lead/trail scoring rate >=62'", self.timeline.lead_trail_ratio(1), 0.75, 1.33),
            tracked(
                "go-ahead goals level in 15' %",
                ratio(self.timeline.go_ahead_equalised as u64, self.timeline.go_ahead as u64),
                0.0,
            ),
            tracked("level in 15' if independent %", equalised_if_independent, 0.0),
            tracked(
                "lead changes hands % matches",
                ratio(self.timeline.matches_with_lead_change as u64, self.matches as u64),
                0.0,
            ),
            tracked("lead changes if independent %", lead_change_if_independent, 0.0),
            tracked("goal share 0-15'", profile(0), 0.11),
            tracked("goal share 15-30'", profile(1), 0.14),
            tracked("goal share 30-45'", profile(2), 0.16),
            tracked("goal share 45-60'", profile(3), 0.15),
            tracked("goal share 60-75'", profile(4), 0.18),
            tracked("goal share 75-90'", profile(5), 0.26),
        ];
        #[cfg(feature = "match-logs")]
        {
            let census = &self.tally.team_census;
            let share = |state: LedgerState| {
                let ledger = census.pooled(state);
                ratio(ledger.possession_ticks, ledger.held_ticks)
            };
            let level = census.pooled(LedgerState::Level);
            lines.push(line(
                "possession given up 1 goal up",
                if level.held_ticks == 0 {
                    0.0
                } else {
                    share(LedgerState::Level) - share(LedgerState::LeadingByOne)
                },
                -0.02,
                0.10,
            ));
            let delivery = &self.tally.delivery;
            let (open_play, byline) = delivery.open_play_byline();
            lines.push(line(
                "byline share of open-play deliveries",
                ratio(byline as u64, open_play as u64),
                0.15,
                0.35,
            ));
            lines.push(tracked("open-play deliveries a team", per(open_play as f64) / 2.0, 16.5));
            lines.push(tracked(
                "open-play deliveries blocked (approx.)",
                ratio(delivery.open_play_blocked() as u64, open_play as u64),
                0.20,
            ));
            lines.push(tracked(
                "open-play cross headers at goal/match",
                per(delivery.open_play_headers_at_goal() as f64),
                0.0,
            ));
            for (name, n) in DeliveryCensus::CORNER_LINES.iter().zip(delivery.corners) {
                lines.push(tracked(name, per(n as f64), 0.0));
            }
            for (name, halves) in DeliveryCensus::THROW_LINES.iter().zip(delivery.throw_ins) {
                lines.push(tracked(name, per(halves.iter().sum::<u32>() as f64), 0.0));
            }
            for (name, outcomes) in DeliveryCensus::OWN_GOAL_LINES.iter().zip(delivery.own_goals) {
                lines.push(tracked(name, per(outcomes[DeliveryCensus::OWN_GOAL] as f64), 0.0));
            }
            let motion = &self.tally.outfield_motion;
            let minutes_played = motion.ticks as f64 / 6000.0;
            let per_player_minute = if motion.ticks == 0 {
                0.0
            } else {
                motion.state_changes as f64 / minutes_played
            };
            let km_per_90 = if motion.ticks == 0 {
                0.0
            } else {
                motion.path_mm as f64 / 1_000_000.0 / minutes_played * 90.0
            };
            lines.push(line(
                "state changes/player/minute",
                per_player_minute,
                0.0,
                45.0,
            ));
            lines.push(line(
                "distance km/outfield player/90",
                km_per_90,
                10.0,
                13.0,
            ));
            let defenders = &self.tally.defender_motion;
            let defender_minutes = defenders.ticks as f64 / 6000.0;
            lines.push(line(
                "distance km/defender/90",
                if defenders.ticks == 0 {
                    0.0
                } else {
                    defenders.path_mm as f64 / 1_000_000.0 / defender_minutes * 90.0
                },
                9.5,
                12.0,
            ));
            lines.push(line(
                "defender time at or below a walk %",
                if defenders.live_ticks == 0 {
                    0.0
                } else {
                    defenders.slow_live_ticks as f64 / defenders.live_ticks as f64
                },
                0.50,
                0.75,
            ));
            lines.push(line(
                "state changes/defender/minute",
                if defenders.ticks == 0 {
                    0.0
                } else {
                    defenders.state_changes as f64 / defender_minutes
                },
                0.0,
                20.0,
            ));
            let midfielders = &self.tally.midfielder_motion;
            let midfielder_minutes = midfielders.ticks as f64 / 6000.0;
            lines.push(line(
                "distance km/midfielder/90",
                if midfielders.ticks == 0 {
                    0.0
                } else {
                    midfielders.path_mm as f64 / 1_000_000.0 / midfielder_minutes * 90.0
                },
                10.5,
                12.5,
            ));
            lines.push(line(
                "midfielder time at or below a walk %",
                if midfielders.live_ticks == 0 {
                    0.0
                } else {
                    midfielders.slow_live_ticks as f64 / midfielders.live_ticks as f64
                },
                0.45,
                0.70,
            ));
            lines.push(line(
                "state changes/midfielder/minute",
                if midfielders.ticks == 0 {
                    0.0
                } else {
                    midfielders.state_changes as f64 / midfielder_minutes
                },
                0.0,
                24.0,
            ));
            let forwards = &self.tally.forward_motion;
            let forward_minutes = forwards.ticks as f64 / 6000.0;
            lines.push(line(
                "distance km/forward/90",
                if forwards.ticks == 0 {
                    0.0
                } else {
                    forwards.path_mm as f64 / 1_000_000.0 / forward_minutes * 90.0
                },
                9.5,
                11.5,
            ));
            lines.push(line(
                "forward time at or below a walk %",
                if forwards.live_ticks == 0 {
                    0.0
                } else {
                    forwards.slow_live_ticks as f64 / forwards.live_ticks as f64
                },
                0.50,
                0.75,
            ));
            lines.push(line(
                "state changes/forward/minute",
                if forwards.ticks == 0 {
                    0.0
                } else {
                    forwards.state_changes as f64 / forward_minutes
                },
                0.0,
                22.0,
            ));
            let ground = &self.tally.keeper_ground;
            let seconds_down = ground.ticks as f64 / 100.0;
            lines.push(line(
                "keeper time down (s)",
                if ground.spells == 0 {
                    0.0
                } else {
                    seconds_down / ground.spells as f64
                },
                0.6,
                1.2,
            ));
            lines.push(line(
                "keeper speed while down (m/s)",
                if ground.ticks == 0 {
                    0.0
                } else {
                    ground.path_mm as f64 / 1000.0 / seconds_down
                },
                0.0,
                1.5,
            ));
        }
        lines
    }

    pub fn print_report(&self) {
        println!("Calibration report ({} matches)", self.matches);
        for line in self.report_lines() {
            if !line.checked {
                let reference = if line.accept_min > 0.0 {
                    format!("real ~{:.3}", line.accept_min)
                } else {
                    String::new()
                };
                println!(
                    "  [TRK] {:<32} {:>8.3}    {}",
                    line.name, line.value, reference
                );
                continue;
            }
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

    /// Pearson correlation of the two sides' goal counts.
    pub fn goal_correlation(&self) -> f64 {
        let n = self.scores.len() as f64;
        if n < 2.0 {
            return 0.0;
        }
        let mean = |pick: fn(&(u8, u8)) -> u8| self.scores.iter().map(|s| pick(s) as f64).sum::<f64>() / n;
        let (mean_home, mean_away) = (mean(|s| s.0), mean(|s| s.1));
        let (mut covariance, mut var_home, mut var_away) = (0.0, 0.0, 0.0);
        for &(home, away) in &self.scores {
            let (h, a) = (home as f64 - mean_home, away as f64 - mean_away);
            covariance += h * a;
            var_home += h * h;
            var_away += a * a;
        }
        if var_home <= 0.0 || var_away <= 0.0 {
            return 0.0;
        }
        covariance / (var_home * var_away).sqrt()
    }

    /// The share of draws over what the same two goal distributions would
    /// draw if the sides scored independently.
    pub fn draw_surplus(&self) -> f64 {
        let n = self.scores.len() as f64;
        if n == 0.0 {
            return 0.0;
        }
        let mut home = [0.0f64; 16];
        let mut away = [0.0f64; 16];
        let mut draws = 0.0;
        for &(h, a) in &self.scores {
            home[(h as usize).min(15)] += 1.0;
            away[(a as usize).min(15)] += 1.0;
            if h == a {
                draws += 1.0;
            }
        }
        let independent: f64 = home.iter().zip(away.iter()).map(|(h, a)| (h / n) * (a / n)).sum();
        draws / n - independent
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

    /// A result with the goals as listed — `(minute, home scored)` — between
    /// sides 1 (player 10) and 2 (player 20).
    fn scoreline(goals: &[(u64, bool)]) -> MatchResultRaw {
        let mut result = MatchResultRaw::with_match_time(90 * 60 * 1000);
        result.left_team_players.team_id = 1;
        result.left_team_players.main = vec![10];
        result.right_team_players.team_id = 2;
        result.right_team_players.main = vec![20];
        let mut score = Score::new(1, 2);
        for &(minute, home) in goals {
            if home {
                score.increment_home_goals();
            } else {
                score.increment_away_goals();
            }
            score.add_goal_detail(GoalDetail {
                player_id: if home { 10 } else { 20 },
                stat_type: MatchStatisticType::Goal,
                is_auto_goal: false,
                time: minute * 60_000,
                origin: GoalOrigin::OpenPlay,
            });
        }
        result.score = Some(score);
        result
    }

    #[test]
    fn a_go_ahead_goal_is_equalised_counted_from_itself() {
        let soon = GoalTimeline::read(&[(30 * 60_000, true), (40 * 60_000, false)], 90 * 60_000);
        assert_eq!((soon.go_ahead, soon.go_ahead_equalised), (1, 1));
        let late = GoalTimeline::read(&[(30 * 60_000, true), (50 * 60_000, false)], 90 * 60_000);
        assert_eq!((late.go_ahead, late.go_ahead_equalised), (1, 0));
        // Whatever comes between: 1-0, 2-0, 2-1, 2-2 inside fifteen minutes.
        let through = GoalTimeline::read(
            &[
                (30 * 60_000, true),
                (33 * 60_000, true),
                (38 * 60_000, false),
                (44 * 60_000, false),
            ],
            90 * 60_000,
        );
        assert_eq!((through.go_ahead, through.go_ahead_equalised), (1, 1));
    }

    #[test]
    fn an_own_goal_counts_for_the_other_side() {
        let mut result = scoreline(&[]);
        let mut score = Score::new(1, 2);
        score.increment_away_goals();
        score.add_goal_detail(GoalDetail {
            player_id: 10,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal: true,
            time: 600_000,
            origin: GoalOrigin::OpenPlay,
        });
        result.score = Some(score);
        assert_eq!(result.goal_timeline(), vec![(600_000, false)]);
    }

    #[test]
    fn a_draw_machine_reads_out_of_band() {
        let mut stats = MatchCalibrationStats::new();
        for i in 0..200u64 {
            let goals: Vec<(u64, bool)> = (0..i % 4)
                .flat_map(|g| [(10 + g * 20, true), (12 + g * 20, false)])
                .collect();
            stats.record(&scoreline(&goals));
        }
        let misses: Vec<&str> = stats.out_of_range().iter().map(|l| l.name).collect();
        assert!(misses.contains(&"goal correlation (equal teams)"), "{misses:?}");
        assert!(misses.contains(&"draw surplus over independent"), "{misses:?}");
    }

    #[test]
    fn a_frozen_leader_reads_out_of_band() {
        let mut stats = MatchCalibrationStats::new();
        for _ in 0..100 {
            stats.record(&scoreline(&[(10, true), (70, false), (80, false)]));
            stats.record(&scoreline(&[(20, false), (65, true), (85, true)]));
        }
        assert_eq!(line(&stats, "lead/trail scoring rate >=62'"), 0.0);
        let misses: Vec<&str> = stats.out_of_range().iter().map(|l| l.name).collect();
        assert!(misses.contains(&"lead/trail scoring rate >=62'"), "{misses:?}");
    }

    #[test]
    fn independent_scoring_reads_its_own_rates() {
        let minutes = vec![10u32; 90];
        let (equalised, changed) = IndependentScoring::reference(1.4, 1.2, &minutes);
        assert!((0.15..0.45).contains(&equalised), "{equalised}");
        assert!((0.05..0.35).contains(&changed), "{changed}");
        assert_eq!(IndependentScoring::reference(1.4, 1.2, &[]), (0.0, 0.0));
    }

    #[test]
    fn the_goal_profile_puts_stoppage_time_in_its_period() {
        let mut result = scoreline(&[(46, true), (47, false), (95, true)]);
        result.tally.added_time_ms[PeriodKind::FirstHalf.index()] = 3 * 60_000;
        result.tally.added_time_ms[PeriodKind::SecondHalf.index()] = 5 * 60_000;
        let mut stats = MatchCalibrationStats::new();
        stats.record(&result);
        assert_eq!(stats.goal_profile, [0, 0, 2, 0, 0, 1]);
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
