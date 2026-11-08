//! **The stubbed match** (`match-stub` feature) — a full-time result read
//! off the two squads instead of simulated. Goals come from the gap between
//! the two elevens and the home edge; each man's line is drawn from how he
//! played, and the engine's own rating model reads it, so a stubbed world
//! still builds league tables, promotions, careers and the evidence clubs
//! judge men on, at a fraction of the engine's cost.

mod line;
mod performance;

use self::line::{StubDraw, StubLine};
use self::performance::{StubFixture, StubPerformance};
use crate::PlayerFieldPositionGroup;
use crate::club::player::events::{MatchExertionInputs, PositionLoad};
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::engine::player::injury::{InjuryCause, InjuryGrade, InjuryRisk};
use crate::r#match::engine::psychology::Psychology;
use crate::r#match::engine::rating::{MatchRatingPass, RatingVolumes};
use crate::r#match::engine::result::{
    FieldSquad, GoalDetail, PlayerMatchEndStats, PlayerMatchPhysicalSnapshot, SubstitutionReason,
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
    fixture: StubFixture,
    home_ability: f32,
    away_ability: f32,
    injury_conditions: f32,
}

/// A foul he commits, and what the referee did about it.
struct StubFoul {
    at_ms: u64,
    booked: bool,
    dismissed: bool,
}

/// A player's time on the pitch, and what the draw needs to know of him.
struct Stint<'a> {
    player: &'a MatchPlayer,
    is_home: bool,
    group: PlayerFieldPositionGroup,
    performance: f32,
    on_ms: u64,
    off_ms: u64,
    goals: u16,
    assists: u16,
    fouls: Vec<StubFoul>,
    sent_off: bool,
    injury: Option<InjuryGrade>,
}

impl Stint<'_> {
    fn on_pitch_at(&self, ms: u64) -> bool {
        self.on_ms <= ms && ms < self.off_ms
    }

    fn minutes(&self) -> u16 {
        ((self.off_ms - self.on_ms) / 60_000) as u16
    }

    fn scoring_weight(&self) -> f32 {
        // Goals by line read ~10 / 32 / 58 across a world that fields about
        // 4.2 defenders, 4.0 midfielders and 1.8 forwards.
        let by_position = match self.group {
            PlayerFieldPositionGroup::Forward => 8.0,
            PlayerFieldPositionGroup::Midfielder => 2.0,
            PlayerFieldPositionGroup::Defender => 0.55,
            PlayerFieldPositionGroup::Goalkeeper => 0.0,
        };
        let finishing = self.player.skills.technical.finishing;
        by_position
            * (self.performance + StubPerformance::skill_shift(self.player, finishing)).exp()
    }

    fn assist_weight(&self) -> f32 {
        let by_position = match self.group {
            PlayerFieldPositionGroup::Forward => 3.0,
            PlayerFieldPositionGroup::Midfielder => 3.0,
            PlayerFieldPositionGroup::Defender => 1.2,
            PlayerFieldPositionGroup::Goalkeeper => 0.1,
        };
        let creative =
            (self.player.skills.technical.passing + self.player.skills.mental.vision) * 0.5;
        by_position * (self.performance + StubPerformance::skill_shift(self.player, creative)).exp()
    }

    fn substitution_weight(&self) -> f32 {
        match self.group {
            PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => 3.0,
            PlayerFieldPositionGroup::Defender => 1.0,
            PlayerFieldPositionGroup::Goalkeeper => 0.0,
        }
    }

    /// Fouls he was still on the pitch to commit. The one that sends a man
    /// off falls on the minute his stint ends.
    fn fouls_committed(&self) -> impl Iterator<Item = &StubFoul> {
        self.fouls
            .iter()
            .filter(|f| f.at_ms < self.off_ms || (self.sent_off && f.at_ms == self.off_ms))
    }

    /// What the post-match exertion pass reads, as an engine match hands it.
    fn physical_snapshot(&self) -> PlayerMatchPhysicalSnapshot {
        let minutes_played = (self.off_ms - self.on_ms) as f32 / 60_000.0;
        let starting_condition = self.player.player_attributes.condition;
        PlayerMatchPhysicalSnapshot {
            player_id: self.player.id,
            minutes_played,
            starting_condition,
            final_match_energy: MatchExertionInputs::estimated_final_energy(
                starting_condition,
                minutes_played,
            ),
            high_intensity_load_hint: PositionLoad::high_intensity_share(self.group),
            injury: self.injury,
        }
    }
}

/// One side's goals, in match milliseconds.
struct SideGoals {
    rate: f32,
    times: Vec<u64>,
}

/// Something that changes who is on the pitch.
enum StintEvent {
    Substitution,
    SentOff(usize),
    Injured(usize, InjuryGrade),
}

/// One side's time on the pitch while it is being drawn.
struct SideTimeline<'a> {
    is_home: bool,
    team_id: u32,
    full_time: u64,
    stints: Vec<Stint<'a>>,
    events: Vec<(u64, StintEvent)>,
    bench: Vec<&'a MatchPlayer>,
    substitutions_used: usize,
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
    const MIN_GOAL_RATE: f32 = 0.1;
    const MAX_GOAL_RATE: f32 = 5.0;
    /// Extra time is a third of regulation, played tired.
    const EXTRA_TIME_RATE_SHARE: f32 = 0.3;
    const PENALTY_CONVERSION: f32 = 0.76;
    const ASSISTED_SHARE: f32 = 0.75;

    const MIN_SUBSTITUTIONS: i32 = 3;
    const MAX_SUBSTITUTIONS: usize = 5;
    const FIRST_SUBSTITUTION_MINUTE: u64 = 55;
    const LAST_SUBSTITUTION_MINUTE: u64 = 85;

    /// About 22 whistled a match, as `dev_match` reads real football.
    const OUTFIELD_FOULS_PER_90: f32 = 1.1;
    const KEEPER_FOULS_PER_90: f32 = 0.03;
    /// Log fouls per ten points of aggression and dirtiness over neutral.
    const TEMPER_GAIN: f32 = 0.6;
    /// Log card chance per ten points of dirtiness over neutral.
    const DIRTINESS_CARD_GAIN: f32 = 0.5;
    const NEUTRAL_TEMPER: f32 = 10.0;
    /// About 3.8 yellows and 0.1 straight reds per 22 fouls; second
    /// yellows bring reds to `dev_match`'s real ~0.17 a match.
    const YELLOW_PER_FOUL: f32 = 0.173;
    const STRAIGHT_RED_PER_FOUL: f32 = 0.0045;
    /// A man on a booking pulls out of the next one, and a referee thinks
    /// twice about the second card.
    const SECOND_BOOKING_SHARE: f32 = 0.2;
    const NEUTRAL_CARD_HAPPINESS: f32 = 0.5;

    /// What a man takes in contacts over ninety minutes, in foul impulse:
    /// one foul suffered and a couple of clean tackles.
    const CONTACT_IMPULSE_PER_90: f32 = 0.5;
    /// `InjuryRisk`'s load checks run every 6-14 minutes.
    const LOAD_CHECKS_PER_90: f32 = 9.0;
    /// The impulse of an ordinary foul, for how bad a contact injury is.
    const TYPICAL_CONTACT: f32 = 0.25;

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
        let fixture = StubFixture {
            standard: MatchStandard::of(
                &TeamSkillAggregates::at_kickoff(&home.main_squad),
                &TeamSkillAggregates::at_kickoff(&away.main_squad),
            ),
            occasion: Psychology::occasion(&config.environment, config.is_knockout),
            today: config.today,
        };
        StubMatch {
            home,
            away,
            config,
            rng,
            fixture,
            home_ability: Self::mean_ability(home),
            away_ability: Self::mean_ability(away),
            injury_conditions: config.environment.modifiers().injury_risk,
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
        result.standard_of_football = self.fixture.standard;

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

        for stint in &stints {
            let against = if stint.is_home { &away } else { &home };
            result
                .player_stats
                .insert(stint.player.id, self.stat_line(stint, against));
            result
                .physical_snapshots
                .insert(stint.player.id, stint.physical_snapshot());
        }
        result.score = Some(score);
        MatchRatingPass::rate(&mut result, RatingVolumes::RealUnits);
        result
    }

    /// Each side's goal rate from the home edge and how the two elevens
    /// play today against each other.
    fn goal_rates(&self) -> (f32, f32) {
        let home = StubPerformance::of_eleven(self.home, self.away_ability, &self.fixture);
        let away = StubPerformance::of_eleven(self.away, self.home_ability, &self.fixture);
        let edge = Self::HOME_EDGE * self.config.environment.home_advantage + (home - away) * 0.5;
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

    fn draw(&self) -> StubDraw<'_> {
        StubDraw { rng: &self.rng }
    }

    fn goal_times(&self, rate: f32, from_ms: u64, to_ms: u64) -> Vec<u64> {
        let count = self.draw().count(rate);
        let mut times: Vec<u64> = (0..count)
            .map(|_| self.rng.range_u64(from_ms, to_ms))
            .collect();
        times.sort_unstable();
        times
    }

    /// Whole minutes, so a side's minutes add up across a change.
    fn minute_within(&self, on_ms: u64, off_ms: u64) -> u64 {
        self.rng.range_u64(on_ms / 60_000 + 1, off_ms / 60_000 + 1) * 60_000
    }

    /// Everyone who takes the pitch for one side, in time order: the
    /// eleven, the changes the manager makes in the last half hour, and
    /// whatever a foul or an injury does to either.
    fn stints(&self, is_home: bool, full_time: u64, result: &mut MatchResultRaw) -> Vec<Stint<'a>> {
        let squad = if is_home { self.home } else { self.away };
        let mut side = SideTimeline {
            is_home,
            team_id: squad.team_id,
            full_time,
            stints: Vec::with_capacity(squad.main_squad.len() + Self::MAX_SUBSTITUTIONS),
            events: Vec::with_capacity(Self::MAX_SUBSTITUTIONS + 4),
            bench: squad.substitutes.iter().collect(),
            substitutions_used: 0,
        };
        for player in &squad.main_squad {
            self.enter(&mut side, player, 0);
        }
        let starters = side.stints.len();

        let wanted = self
            .rng
            .range_i32(Self::MIN_SUBSTITUTIONS, Self::MAX_SUBSTITUTIONS as i32 + 1)
            as usize;
        let outfield_bench = side
            .bench
            .iter()
            .filter(|p| Self::group_of(p) != PlayerFieldPositionGroup::Goalkeeper)
            .count();
        for _ in 0..wanted.min(outfield_bench) {
            let minute = self.rng.range_u64(
                Self::FIRST_SUBSTITUTION_MINUTE,
                Self::LAST_SUBSTITUTION_MINUTE + 1,
            );
            side.events
                .push((minute * 60_000, StintEvent::Substitution));
        }

        while let Some(next) = (0..side.events.len()).min_by_key(|i| side.events[*i].0) {
            let (at_ms, event) = side.events.swap_remove(next);
            match event {
                StintEvent::Substitution => {
                    let off = self.pick(&side.stints[..starters], |s| {
                        if s.on_pitch_at(at_ms) {
                            s.substitution_weight()
                        } else {
                            0.0
                        }
                    });
                    if let Some(off) = off {
                        self.substitute(
                            &mut side,
                            off,
                            at_ms,
                            SubstitutionReason::Discretionary,
                            result,
                        );
                    }
                }
                StintEvent::SentOff(stint) => {
                    if side.stints[stint].on_pitch_at(at_ms) {
                        side.stints[stint].off_ms = at_ms;
                        side.stints[stint].sent_off = true;
                    }
                }
                StintEvent::Injured(stint, grade) => {
                    if !side.stints[stint].on_pitch_at(at_ms) {
                        continue;
                    }
                    side.stints[stint].injury = Some(grade);
                    if grade == InjuryGrade::Knock {
                        continue;
                    }
                    let replaced = self.substitute(
                        &mut side,
                        stint,
                        at_ms,
                        SubstitutionReason::CriticalInjury,
                        result,
                    );
                    if !replaced && grade == InjuryGrade::Serious {
                        side.stints[stint].off_ms = at_ms;
                    }
                }
            }
        }
        side.stints
    }

    fn group_of(player: &MatchPlayer) -> PlayerFieldPositionGroup {
        player.tactical_position.current_position.position_group()
    }

    /// He walks on: how he plays, what he will do to get booked and whether
    /// he gets hurt are drawn for the rest of the match, and whatever falls
    /// after he has already left never happens.
    fn enter(&self, side: &mut SideTimeline<'a>, player: &'a MatchPlayer, on_ms: u64) {
        let opposition = if side.is_home {
            self.away_ability
        } else {
            self.home_ability
        };
        let group = Self::group_of(player);
        let fouls = self.fouls(player, group, on_ms, side.full_time);
        let index = side.stints.len();
        if let Some(foul) = fouls.iter().find(|f| f.dismissed) {
            side.events.push((foul.at_ms, StintEvent::SentOff(index)));
        }
        if let Some((at_ms, grade)) = self.injury(player, on_ms, side.full_time) {
            side.events.push((at_ms, StintEvent::Injured(index, grade)));
        }
        side.stints.push(Stint {
            player,
            is_home: side.is_home,
            group,
            performance: StubPerformance::read(player, opposition, &self.fixture),
            on_ms,
            off_ms: side.full_time,
            goals: 0,
            assists: 0,
            fouls,
            sent_off: false,
            injury: None,
        });
    }

    /// Replaces a man if the bench and the rules allow; false if they don't.
    /// A keeper leaves only for a keeper, and nobody else does.
    fn substitute(
        &self,
        side: &mut SideTimeline<'a>,
        off: usize,
        at_ms: u64,
        reason: SubstitutionReason,
        result: &mut MatchResultRaw,
    ) -> bool {
        if side.substitutions_used >= Self::MAX_SUBSTITUTIONS || at_ms >= side.full_time {
            return false;
        }
        let keeper = PlayerFieldPositionGroup::Goalkeeper;
        let group = side.stints[off].group;
        let same_group = side.bench.iter().any(|p| Self::group_of(p) == group);
        if group == keeper && !same_group {
            return false;
        }
        let incoming = self.pick(&side.bench, |p| {
            let bench_group = Self::group_of(p);
            let eligible = if same_group {
                bench_group == group
            } else {
                bench_group != keeper
            };
            eligible as u8 as f32
        });
        let Some(incoming) = incoming else {
            return false;
        };
        let incoming = side.bench.swap_remove(incoming);

        side.stints[off].off_ms = at_ms;
        side.substitutions_used += 1;
        let field = if side.is_home {
            &mut result.left_team_players
        } else {
            &mut result.right_team_players
        };
        field.mark_substitute_used(incoming.id);
        result.substitutions.push(SubstitutionInfo {
            team_id: side.team_id,
            player_out_id: side.stints[off].player.id,
            player_in_id: incoming.id,
            match_time_ms: at_ms,
            reason,
            break_ms: 0,
        });
        self.enter(side, incoming, at_ms);
        true
    }

    /// His fouls for the rest of the match, up to the one that sends him
    /// off. Temper is what commits them; the referee and dirtiness are what
    /// turn one into a card.
    fn fouls(
        &self,
        player: &MatchPlayer,
        group: PlayerFieldPositionGroup,
        on_ms: u64,
        off_ms: u64,
    ) -> Vec<StubFoul> {
        let per_90 = if group == PlayerFieldPositionGroup::Goalkeeper {
            Self::KEEPER_FOULS_PER_90
        } else {
            Self::OUTFIELD_FOULS_PER_90
        };
        let dirtiness = player.attributes.dirtiness;
        let temper = (player.skills.mental.aggression + dirtiness) * 0.5;
        let share = (off_ms - on_ms) as f32 / Self::REGULATION_MS as f32;
        let mean =
            per_90 * share * (Self::TEMPER_GAIN * (temper - Self::NEUTRAL_TEMPER) / 10.0).exp();
        let count = self.draw().count(mean);
        if count == 0 {
            return Vec::new();
        }

        let card_odds = self.config.referee.card_happiness / Self::NEUTRAL_CARD_HAPPINESS
            * (Self::DIRTINESS_CARD_GAIN * (dirtiness - Self::NEUTRAL_TEMPER) / 10.0).exp();
        let mut minutes: Vec<u64> = (0..count)
            .map(|_| self.minute_within(on_ms, off_ms))
            .collect();
        minutes.sort_unstable();

        let mut fouls = Vec::with_capacity(minutes.len());
        let mut already_booked = false;
        for at_ms in minutes {
            let roll = self.rng.unit_f32();
            let straight_red = roll < Self::STRAIGHT_RED_PER_FOUL * card_odds;
            let yellow_odds = if already_booked {
                Self::YELLOW_PER_FOUL * Self::SECOND_BOOKING_SHARE
            } else {
                Self::YELLOW_PER_FOUL
            };
            let booked =
                !straight_red && roll < (Self::STRAIGHT_RED_PER_FOUL + yellow_odds) * card_odds;
            let dismissed = straight_red || (booked && already_booked);
            fouls.push(StubFoul {
                at_ms,
                booked,
                dismissed,
            });
            if dismissed {
                break;
            }
            already_booked |= booked;
        }
        fouls
    }

    /// One draw for his whole stint, from the same two channels the engine
    /// rolls through the match: the contacts he takes and the load he
    /// carries, against his body and his condition.
    fn injury(&self, player: &MatchPlayer, on_ms: u64, off_ms: u64) -> Option<(u64, InjuryGrade)> {
        let share = (off_ms - on_ms) as f32 / Self::REGULATION_MS as f32;
        let midpoint_minute = ((on_ms + off_ms) / 2 / 60_000) as u32;
        let conditions = self.injury_conditions;
        let contact =
            InjuryRisk::contact_chance(player, Self::CONTACT_IMPULSE_PER_90, conditions) * share;
        let load = InjuryRisk::load_chance(player, midpoint_minute, conditions)
            * Self::LOAD_CHECKS_PER_90
            * share;
        let roll = self.rng.unit_f32();
        if roll >= contact + load {
            return None;
        }
        let (cause, impulse) = if roll < contact {
            (InjuryCause::Contact, Self::TYPICAL_CONTACT)
        } else {
            (InjuryCause::Load, 0.0)
        };
        let grade = InjuryRisk::severity(cause, impulse, player, self.rng.unit_f32());
        Some((self.minute_within(on_ms, off_ms), grade))
    }

    /// Index of one item, drawn in proportion to `weight`; `None` when
    /// nothing carries any weight.
    fn pick<T>(&self, items: &[T], weight: impl Fn(&T) -> f32) -> Option<usize> {
        let total: f32 = items.iter().map(&weight).sum();
        if total <= 0.0 {
            return None;
        }
        let mut roll = self.rng.unit_f32() * total;
        let mut last = None;
        for (i, item) in items.iter().enumerate() {
            let w = weight(item);
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
            player_id: stints[scorer].player.id,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal: false,
            time,
            origin: GoalOrigin::OpenPlay,
        });

        if !self.rng.bernoulli(Self::ASSISTED_SHARE) {
            return;
        }
        let scorer_id = stints[scorer].player.id;
        if let Some(assist) = self.pick(stints, |s| {
            if on_side(s) && s.player.id != scorer_id {
                s.assist_weight()
            } else {
                0.0
            }
        }) {
            stints[assist].assists += 1;
            score.add_goal_detail(GoalDetail {
                player_id: stints[assist].player.id,
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

    fn stat_line(&self, stint: &Stint, against: &SideGoals) -> PlayerMatchEndStats {
        let minutes = stint.minutes();
        let mut stats = PlayerMatchEndStats::neutral(stint.group, minutes, 0.0);
        stats.goals = stint.goals;
        stats.assists = stint.assists;
        for foul in stint.fouls_committed() {
            stats.fouls += 1;
            stats.yellow_cards += foul.booked as u16;
            stats.red_cards += foul.dismissed as u16;
        }

        let line = StubLine {
            draw: self.draw(),
            player: stint.player,
            performance: stint.performance,
            share: minutes as f32 / 90.0,
        };
        if stint.group == PlayerFieldPositionGroup::Goalkeeper {
            let conceded = against
                .times
                .iter()
                .filter(|t| stint.on_pitch_at(**t))
                .count() as u16;
            line.keeper(&mut stats, conceded, against.rate);
        } else {
            line.outfield(&mut stats);
        }
        stats
    }
}
