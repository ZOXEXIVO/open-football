//! **The stubbed match** (`match-stub` feature) — a full-time result read
//! off the two squads instead of simulated. Goals come from the gap between
//! the starting elevens' ability and the home edge; scorers, assists,
//! substitutions, minutes and ratings are drawn to go with them, so a
//! stubbed world still builds league tables, promotions, careers and form
//! at a fraction of the engine's cost.

use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::engine::result::{
    FieldSquad, GoalDetail, PlayerMatchEndStats, SubstitutionReason,
};
use crate::r#match::engine::teamplay::standard::MatchStandard;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{
    GoalOrigin, MatchPlayer, MatchResultRaw, MatchRng, MatchSquad, Score, SubstitutionInfo,
    TeamSkillAggregates,
};

/// One stubbed match between a home and an away squad.
pub(in crate::r#match::engine::engine) struct StubMatch<'a> {
    home: &'a MatchSquad,
    away: &'a MatchSquad,
    config: &'a MatchEngineConfig,
    rng: MatchRng,
}

/// A player's time on the pitch, and what the draw needs to know of him.
struct Stint {
    player_id: u32,
    is_home: bool,
    group: PlayerFieldPositionGroup,
    ability: f32,
    finishing: f32,
    passing: f32,
    on_ms: u64,
    off_ms: u64,
    goals: u16,
    assists: u16,
}

impl Stint {
    fn new(player: &MatchPlayer, is_home: bool, on_ms: u64, off_ms: u64) -> Self {
        Stint {
            player_id: player.id,
            is_home,
            group: player.tactical_position.current_position.position_group(),
            ability: player.player_attributes.current_ability as f32,
            finishing: player.skills.technical.finishing,
            passing: player.skills.technical.passing,
            on_ms,
            off_ms,
            goals: 0,
            assists: 0,
        }
    }

    fn on_pitch_at(&self, ms: u64) -> bool {
        self.on_ms <= ms && ms < self.off_ms
    }

    fn minutes(&self) -> u16 {
        ((self.off_ms - self.on_ms) / 60_000) as u16
    }

    fn scoring_weight(&self) -> f32 {
        let by_position = match self.group {
            PlayerFieldPositionGroup::Forward => 5.0,
            PlayerFieldPositionGroup::Midfielder => 2.5,
            PlayerFieldPositionGroup::Defender => 0.8,
            PlayerFieldPositionGroup::Goalkeeper => 0.0,
        };
        by_position * (0.5 + self.finishing / 20.0)
    }

    fn assist_weight(&self) -> f32 {
        let by_position = match self.group {
            PlayerFieldPositionGroup::Forward => 2.0,
            PlayerFieldPositionGroup::Midfielder => 3.0,
            PlayerFieldPositionGroup::Defender => 1.2,
            PlayerFieldPositionGroup::Goalkeeper => 0.1,
        };
        by_position * (0.5 + self.passing / 20.0)
    }

    fn substitution_weight(&self) -> f32 {
        match self.group {
            PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => 3.0,
            PlayerFieldPositionGroup::Defender => 1.0,
            PlayerFieldPositionGroup::Goalkeeper => 0.0,
        }
    }
}

/// One side's goals, in match milliseconds.
struct SideGoals {
    rate: f32,
    times: Vec<u64>,
}

impl<'a> StubMatch<'a> {
    const REGULATION_MS: u64 = 90 * 60 * 1000;
    const EXTRA_TIME_MS: u64 = 30 * 60 * 1000;

    /// Goals a side scores against an equal one on neutral ground. Two
    /// such sides share about 2.7 goals a match.
    const NEUTRAL_GOAL_RATE: f32 = 1.35;
    /// Log goal-rate edge per side at full home advantage. A fixture's
    /// typical advantage of ~0.55 gives the home side about 1.5 goals to
    /// the away side's 1.2.
    const HOME_EDGE: f32 = 0.2;
    /// Log goal-rate shift per side per point of current-ability gap
    /// between the two elevens. Forty points — a title side against a
    /// relegation one — is roughly 2.3 goals against 0.8.
    const ABILITY_SLOPE: f32 = 0.013;
    const MIN_GOAL_RATE: f32 = 0.1;
    const MAX_GOAL_RATE: f32 = 5.0;
    /// Extra time is a third of regulation, played tired.
    const EXTRA_TIME_RATE_SHARE: f32 = 0.3;
    const PENALTY_CONVERSION: f32 = 0.76;
    const ASSISTED_SHARE: f32 = 0.75;
    /// Shots a keeper saves per goal his opponents are expected to score.
    const SAVES_PER_EXPECTED_GOAL: f32 = 2.2;

    const MIN_SUBSTITUTIONS: i32 = 3;
    const MAX_SUBSTITUTIONS: i32 = 5;
    const FIRST_SUBSTITUTION_MINUTE: u64 = 55;
    const LAST_SUBSTITUTION_MINUTE: u64 = 85;

    const BASE_RATING: f32 = 6.6;
    const RESULT_RATING: f32 = 0.3;
    const GOAL_RATING: f32 = 0.9;
    const ASSIST_RATING: f32 = 0.5;
    const CLEAN_SHEET_RATING: f32 = 0.35;
    const CONCEDED_RATING: f32 = 0.15;
    const MAX_CONCEDED_PENALTY: f32 = 0.9;
    const SAVE_RATING: f32 = 0.1;
    const ABILITY_RATING: f32 = 0.008;
    const RATING_NOISE: f32 = 0.35;
    /// Under this many minutes a rating is pulled back toward the base:
    /// a cameo says little either way.
    const SETTLED_MINUTES: f32 = 30.0;

    /// Left is the home side, as in [`FootballEngine::play_with_config`].
    pub(in crate::r#match::engine::engine) fn new(
        home: &'a MatchSquad,
        away: &'a MatchSquad,
        config: &'a MatchEngineConfig,
    ) -> Self {
        let rng = config
            .seed
            .map(MatchRng::from_seed)
            .unwrap_or_else(MatchRng::from_entropy);
        StubMatch {
            home,
            away,
            config,
            rng,
        }
    }

    pub(in crate::r#match::engine::engine) fn play(self) -> MatchResultRaw {
        let (home_rate, away_rate) = self.goal_rates();
        let mut home = SideGoals {
            rate: home_rate,
            times: self.goal_times(home_rate, 0, Self::REGULATION_MS),
        };
        let mut away = SideGoals {
            rate: away_rate,
            times: self.goal_times(away_rate, 0, Self::REGULATION_MS),
        };

        let mut full_time = Self::REGULATION_MS;
        if self.config.is_knockout && home.times.len() == away.times.len() {
            full_time += Self::EXTRA_TIME_MS;
            let share = Self::EXTRA_TIME_RATE_SHARE;
            home.times
                .extend(self.goal_times(home.rate * share, Self::REGULATION_MS, full_time));
            away.times
                .extend(self.goal_times(away.rate * share, Self::REGULATION_MS, full_time));
        }

        let mut result = MatchResultRaw::with_match_time(full_time);
        result.left_team_players = FieldSquad::from_team(self.home);
        result.right_team_players = FieldSquad::from_team(self.away);
        result.weather = self.config.environment.weather;
        result.pitch = self.config.environment.pitch;
        result.standard_of_football = MatchStandard::of(
            &TeamSkillAggregates::at_kickoff(&self.home.main_squad),
            &TeamSkillAggregates::at_kickoff(&self.away.main_squad),
        );

        let mut stints = self.stints(true, full_time, &mut result);
        stints.extend(self.stints(false, full_time, &mut result));

        let mut score = Score::new(self.home.team_id, self.away.team_id);
        let mut goals: Vec<(u64, bool)> = home
            .times
            .iter()
            .map(|t| (*t, true))
            .chain(away.times.iter().map(|t| (*t, false)))
            .collect();
        goals.sort_unstable();
        for (time, is_home) in goals {
            self.credit_goal(&mut stints, &mut score, time, is_home);
        }
        if self.config.is_knockout && score.is_tied() {
            let (home_kicks, away_kicks) = self.shootout();
            score.home_shootout = home_kicks;
            score.away_shootout = away_kicks;
        }

        let home_goals = score.home_team.get();
        let away_goals = score.away_team.get();
        for stint in &stints {
            let stats = self.stat_line(stint, &home, &away, home_goals, away_goals);
            result.player_stats.insert(stint.player_id, stats);
        }
        result.score = Some(score);
        result
    }

    /// Each side's goal rate from the home edge and the ability gap
    /// between the two starting elevens.
    fn goal_rates(&self) -> (f32, f32) {
        let gap = Self::mean_ability(self.home) - Self::mean_ability(self.away);
        let edge =
            Self::HOME_EDGE * self.config.environment.home_advantage + Self::ABILITY_SLOPE * gap;
        let rate = |e: f32| {
            (Self::NEUTRAL_GOAL_RATE * e.exp()).clamp(Self::MIN_GOAL_RATE, Self::MAX_GOAL_RATE)
        };
        (rate(edge), rate(-edge))
    }

    fn mean_ability(squad: &MatchSquad) -> f32 {
        if squad.main_squad.is_empty() {
            return 0.0;
        }
        squad
            .main_squad
            .iter()
            .map(|p| p.player_attributes.current_ability as f32)
            .sum::<f32>()
            / squad.main_squad.len() as f32
    }

    fn goal_times(&self, rate: f32, from_ms: u64, to_ms: u64) -> Vec<u64> {
        let count = self.poisson(rate);
        let mut times: Vec<u64> = (0..count)
            .map(|_| self.rng.range_u64(from_ms, to_ms))
            .collect();
        times.sort_unstable();
        times
    }

    /// Knuth's draw — exact, and cheap at football's goal rates.
    fn poisson(&self, rate: f32) -> u32 {
        let floor = (-rate).exp();
        let mut product = self.rng.unit_f32();
        let mut count = 0;
        while product > floor {
            product *= self.rng.unit_f32();
            count += 1;
        }
        count
    }

    /// Every player who takes the pitch for one side: the eleven, and the
    /// substitutes who come on for outfield starters in the last half hour.
    fn stints(&self, is_home: bool, full_time: u64, result: &mut MatchResultRaw) -> Vec<Stint> {
        let squad = if is_home { self.home } else { self.away };
        let mut stints: Vec<Stint> = squad
            .main_squad
            .iter()
            .map(|p| Stint::new(p, is_home, 0, full_time))
            .collect();
        let mut bench: Vec<&MatchPlayer> = squad
            .substitutes
            .iter()
            .filter(|p| {
                p.tactical_position.current_position.position_group()
                    != PlayerFieldPositionGroup::Goalkeeper
            })
            .collect();

        let wanted = self
            .rng
            .range_i32(Self::MIN_SUBSTITUTIONS, Self::MAX_SUBSTITUTIONS + 1)
            as usize;
        let mut minutes: Vec<u64> = (0..wanted.min(bench.len()))
            .map(|_| {
                self.rng.range_u64(
                    Self::FIRST_SUBSTITUTION_MINUTE,
                    Self::LAST_SUBSTITUTION_MINUTE + 1,
                )
            })
            .collect();
        minutes.sort_unstable();

        let starters = stints.len();
        for minute in minutes {
            let at_ms = minute * 60 * 1000;
            let Some(off) = self.pick(&stints[..starters], |s| {
                if s.off_ms == full_time {
                    s.substitution_weight()
                } else {
                    0.0
                }
            }) else {
                break;
            };
            let group = stints[off].group;
            let same_group: Vec<usize> = (0..bench.len())
                .filter(|i| {
                    bench[*i]
                        .tactical_position
                        .current_position
                        .position_group()
                        == group
                })
                .collect();
            let pool: Vec<usize> = if same_group.is_empty() {
                (0..bench.len()).collect()
            } else {
                same_group
            };
            let incoming =
                bench.swap_remove(pool[self.rng.range_u64(0, pool.len() as u64) as usize]);

            stints[off].off_ms = at_ms;
            stints.push(Stint::new(incoming, is_home, at_ms, full_time));
            let field = if is_home {
                &mut result.left_team_players
            } else {
                &mut result.right_team_players
            };
            field.mark_substitute_used(incoming.id);
            result.substitutions.push(SubstitutionInfo {
                team_id: squad.team_id,
                player_out_id: stints[off].player_id,
                player_in_id: incoming.id,
                match_time_ms: at_ms,
                reason: SubstitutionReason::Discretionary,
                break_ms: 0,
            });
        }
        stints
    }

    /// Index of one stint, drawn in proportion to `weight`; `None` when
    /// nobody carries any weight.
    fn pick(&self, stints: &[Stint], weight: impl Fn(&Stint) -> f32) -> Option<usize> {
        let total: f32 = stints.iter().map(&weight).sum();
        if total <= 0.0 {
            return None;
        }
        let mut roll = self.rng.unit_f32() * total;
        let mut last = None;
        for (i, stint) in stints.iter().enumerate() {
            let w = weight(stint);
            if w <= 0.0 {
                continue;
            }
            last = Some(i);
            if roll < w {
                return Some(i);
            }
            roll -= w;
        }
        last
    }

    fn credit_goal(&self, stints: &mut [Stint], score: &mut Score, time: u64, is_home: bool) {
        if is_home {
            score.increment_home_goals();
        } else {
            score.increment_away_goals();
        }
        let on_side = |s: &Stint| s.is_home == is_home && s.on_pitch_at(time);
        let Some(scorer) = self.pick(
            stints,
            |s| {
                if on_side(s) { s.scoring_weight() } else { 0.0 }
            },
        ) else {
            return;
        };
        stints[scorer].goals += 1;
        score.add_goal_detail(GoalDetail {
            player_id: stints[scorer].player_id,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal: false,
            time,
            origin: GoalOrigin::OpenPlay,
        });

        if !self.rng.bernoulli(Self::ASSISTED_SHARE) {
            return;
        }
        let scorer_id = stints[scorer].player_id;
        if let Some(assist) = self.pick(stints, |s| {
            if on_side(s) && s.player_id != scorer_id {
                s.assist_weight()
            } else {
                0.0
            }
        }) {
            stints[assist].assists += 1;
            score.add_goal_detail(GoalDetail {
                player_id: stints[assist].player_id,
                stat_type: MatchStatisticType::Assist,
                is_auto_goal: false,
                time,
                origin: GoalOrigin::OpenPlay,
            });
        }
    }

    /// Five kicks each, then sudden death.
    fn shootout(&self) -> (u8, u8) {
        let (mut home, mut away) = (0u8, 0u8);
        for round in 0..u8::MAX {
            home += self.rng.bernoulli(Self::PENALTY_CONVERSION) as u8;
            away += self.rng.bernoulli(Self::PENALTY_CONVERSION) as u8;
            if round >= 4 && home != away {
                return (home, away);
            }
        }
        if self.rng.bernoulli(0.5) {
            (home + 1, away)
        } else {
            (home, away + 1)
        }
    }

    fn stat_line(
        &self,
        stint: &Stint,
        home: &SideGoals,
        away: &SideGoals,
        home_goals: u8,
        away_goals: u8,
    ) -> PlayerMatchEndStats {
        let (against, opponent_rate, scored, conceded_total) = if stint.is_home {
            (away, away.rate, home_goals, away_goals)
        } else {
            (home, home.rate, away_goals, home_goals)
        };
        let conceded = against
            .times
            .iter()
            .filter(|t| stint.on_pitch_at(**t))
            .count() as u16;
        let minutes = stint.minutes();
        let share = (stint.off_ms - stint.on_ms) as f32 / Self::REGULATION_MS as f32;

        let mut stats = PlayerMatchEndStats::neutral(stint.group, minutes, Self::BASE_RATING);
        stats.goals = stint.goals;
        stats.assists = stint.assists;
        stats.shots_on_target = stint.goals;
        stats.shots_total = stint.goals;
        if stint.group == PlayerFieldPositionGroup::Goalkeeper {
            stats.saves =
                self.poisson(Self::SAVES_PER_EXPECTED_GOAL * opponent_rate * share) as u16;
            stats.shots_faced = stats.saves + conceded;
        }

        let mut rating = Self::BASE_RATING
            + Self::RESULT_RATING * (scored as f32 - conceded_total as f32).signum()
            + Self::GOAL_RATING * stint.goals as f32
            + Self::ASSIST_RATING * stint.assists as f32
            + Self::SAVE_RATING * stats.saves as f32
            + Self::ABILITY_RATING * (stint.ability - self.match_ability())
            + self.rng.jitter(0.0, Self::RATING_NOISE)
            + self.rng.jitter(0.0, Self::RATING_NOISE);
        if matches!(
            stint.group,
            PlayerFieldPositionGroup::Goalkeeper | PlayerFieldPositionGroup::Defender
        ) {
            rating += if conceded == 0 {
                Self::CLEAN_SHEET_RATING * share.min(1.0)
            } else {
                -(Self::CONCEDED_RATING * conceded as f32).min(Self::MAX_CONCEDED_PENALTY)
            };
        }
        let settled = (minutes as f32 / Self::SETTLED_MINUTES).min(1.0);
        let rating = (Self::BASE_RATING + (rating - Self::BASE_RATING) * settled).clamp(3.0, 10.0);
        stats.match_rating = rating;
        stats.raw_match_rating = rating;
        stats
    }

    fn match_ability(&self) -> f32 {
        (Self::mean_ability(self.home) + Self::mean_ability(self.away)) * 0.5
    }
}
