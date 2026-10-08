//! [`MatchContext`] — the per-match world every engine pass is handed.
//!
//! The struct and its construction live here; the passes that only read
//! or advance one slice of it are siblings in this group.

use super::config::MatchEngineConfig;
use super::substitution_record::{SubstitutionRecord, SubstitutionWindows};
use crate::MatchTacticType;
use crate::r#match::engine::chemistry::{ChemistryMap, TacticalFamiliarity};
#[cfg(feature = "match-logs")]
use crate::r#match::engine::engine::BoxEpisode;
use crate::r#match::engine::environment::{EnvModifiers, MatchEnvironment};
use crate::r#match::engine::flow::rng::MatchRng;
use crate::r#match::engine::flow::touchline::SubstitutionBreak;
use crate::r#match::engine::player::events::players::{FoulSeverity, FoulSource};
use nalgebra::Vector3;
use crate::r#match::engine::psychology::{Psychology, PsychologyState};
use crate::r#match::engine::officiating::management::{
    TimeWasting, TimeWastingLedger, TimeWastingRestart,
};
use crate::r#match::engine::referee::RefereeProfile;
use crate::r#match::engine::teamplay::coach::instruction::CoachInstruction;
use crate::r#match::engine::result::{
    ChanceDetail, MatchTally, PenaltyShootoutKick, PlayerMatchEndStats,
    PlayerMatchPhysicalSnapshot,
};
use crate::r#match::engine::set_pieces::SetPieceHistory;
use crate::r#match::engine::teamplay::standard::StandardReading;
use crate::r#match::rules::MatchRules;
use crate::r#match::CompetitionKind;
use crate::r#match::{
    AttackPlan, DefensivePlan, GameState, GoalCelebration, GoalDetail, GoalPosition, MatchCoach,
    MatchField, MatchFieldSize, MatchPlayer, MatchPlayerCollection, MatchTime, Score, TeamShape,
    TeamSkillAggregates, TeamTacticalState, TeamsTactics,
};
use chrono::{NaiveDate, Utc};

pub struct MatchContext {
    pub state: GameState,
    pub time: MatchTime,
    pub score: Score,
    pub field_size: MatchFieldSize,
    pub players: MatchPlayerCollection,
    pub goal_positions: GoalPosition,
    pub tactics: TeamsTactics,

    // Team IDs for determining which goal to shoot at
    pub field_home_team_id: u32,
    pub field_away_team_id: u32,

    pub(crate) logging_enabled: bool,

    // Track cumulative time across all match states
    pub total_match_time: u64,

    pub substitutions: Vec<SubstitutionRecord>,
    pub max_substitutions_per_team: usize,
    /// Per-stoppage cap. The substitutions pass runs every 5–15 in-match
    /// minutes and is allowed to make at most this many subs per call.
    /// Sourced from `MatchRules.max_substitutions_per_pass`.
    pub max_substitutions_per_pass: usize,
    /// Whether the knockout-tie ET bonus substitution is allowed for
    /// this match. Mirrors `MatchRules.allow_extra_time_extra_sub`.
    pub allow_extra_time_extra_sub: bool,
    /// How many separate stoppages each side has already interrupted to
    /// make a change. The Law allows three, plus the interval free — see
    /// [`SubstitutionWindows`].
    pub substitution_windows: SubstitutionWindows,
    pub additional_time_ms: u64,
    pub period_stoppage_time_ms: u64,
    /// Fraction of a millisecond of stoppage owed but not yet added — the
    /// referee adds back a share of each 10 ms tick, not whole ticks.
    pub stoppage_credit_ms: f32,
    /// The team that kicked off the period being played. The other one
    /// kicks off the next.
    pub period_kickoff_team: Option<u32>,
    pub penalty_shootout_kicks: Vec<PenaltyShootoutKick>,
    /// Keepers who kept out a penalty during play, one entry per save.
    pub penalty_saves: Vec<u32>,

    /// Every strike that was worth calling a chance, in the order they were
    /// taken — candidates, not the shortlist. `HighlightSelector::select`
    /// reduces this to the two or three per side that reach the match sheet,
    /// at full time, when there is a whole match to rank them against.
    pub chances: Vec<ChanceDetail>,

    /// The box-possession episode currently running, for the `match-logs`
    /// census. Per-match state, so it cannot be a global counter: matches
    /// run in parallel.
    #[cfg(feature = "match-logs")]
    pub box_episode: BoxEpisode,

    // Global goal cooldown: tick when last goal was scored
    // Prevents immediate scoring after kickoff restart
    pub last_goal_tick: u64,

    /// Per-side tick at which that side last conceded a goal. Used to
    /// model "post-concede rattle" — the well-documented real-football
    /// dynamic where teams that just conceded a goal under-perform on
    /// their next attacking moves (decision-making rushed, body language
    /// shaken, defensive lines slow to reset). The forward shot-decision
    /// helper reads this to dampen willingness for ~1 minute of match
    /// time after a goal. Without this, the engine's equal-skill
    /// scoreline distribution showed 2-2 / 3-3 / 4-4 draws at 2-4× real
    /// PL rates — every concede triggered an immediate equalizer.
    /// Index 0 = home, index 1 = away. `u64::MAX` = never conceded.
    pub last_conceded_tick: [u64; 2],

    // Stats for players who were substituted out (preserved before replacement)
    pub substituted_out_stats: Vec<(u32, PlayerMatchEndStats)>,

    /// Physical snapshots for players who were substituted off, captured
    /// at the moment of the swap. `build_result` folds these into the
    /// per-match `MatchResultRaw.physical_snapshots` map alongside the
    /// snapshots for players who finished the match on the pitch. Lets
    /// the post-match condition-drop formula read the actual energy
    /// state at the player's exit minute instead of an artificial
    /// full-time value.
    pub substituted_out_physical_snapshots: Vec<PlayerMatchPhysicalSnapshot>,

    /// Coach state for each team (home = left initially, away = right initially)
    pub coach_home: MatchCoach,
    pub coach_away: MatchCoach,

    /// Team-level tactical state (phase, possession timers, defensive
    /// line height) shared across every player on that side. Keyed the
    /// same way as `coach_home/away`. Updated by
    /// `tactical::update_tactical_states` every ~10 ticks from the
    /// engine tick loop.
    pub tactical_home: TeamTacticalState,
    pub tactical_away: TeamTacticalState,

    /// Per-possession attacking role assignment for each side — who is
    /// the primary target, who occupies which patch of the box, who holds
    /// as rest defence. Refreshed alongside `tactical_home/away` and read
    /// by off-ball movement and the pass evaluator so eleven players stop
    /// independently computing the same destination. See
    /// [`AttackPlan`](crate::r#match::AttackPlan).
    pub attack_home: AttackPlan,
    pub attack_away: AttackPlan,

    /// Per-possession defensive duty assignment for each side — who
    /// presses, who covers, and which opponent each remaining defender is
    /// responsible for. Refreshed alongside `tactical_home/away`. Without
    /// it, defensive position is computed against the ball and the
    /// kickoff formation and never against an opponent. See
    /// [`DefensivePlan`](crate::r#match::DefensivePlan).
    pub defence_home: DefensivePlan,
    pub defence_away: DefensivePlan,

    /// The live positional block for each side, and every player's anchor
    /// inside it. The three plans above only ever name the handful of
    /// players involved with the ball; this one covers all eleven, in
    /// every phase, and is what off-ball movement steers at instead of the
    /// static kickoff dot. See [`TeamShape`](crate::r#match::TeamShape).
    pub shape_home: TeamShape,
    pub shape_away: TeamShape,

    /// Knockout-format match — enables extra time + penalty shootout when
    /// the score is level at the end of regulation.
    pub is_knockout: bool,

    /// Weather + pitch + crowd + importance. Defaults to a neutral
    /// fixture; harnesses can override before kickoff.
    pub environment: MatchEnvironment,
    /// `environment.modifiers()`, drawn once: the deltas the weather and
    /// the pitch put on the match.
    pub conditions: EnvModifiers,

    /// Referee strictness/leniency/card profile. Defaults to a balanced
    /// referee.
    pub referee: RefereeProfile,

    /// Recent corner routine history per team — drives anti-repetition
    /// blocking in `pick_corner_routine`.
    pub set_piece_history: SetPieceHistory,

    /// Match-time psychology — per-player confidence/nervousness +
    /// per-team momentum. Lazily populated as players are touched by
    /// goal/error/card events.
    pub psychology: PsychologyState,

    /// Pair-keyed teammate chemistry cache. Lazily populated by
    /// callers that compute one-touch passing / handoff success.
    pub chemistry: ChemistryMap,

    /// Tactical familiarity per side (0..1) — drives press timing /
    /// offside trap synchronisation.
    pub tactical_familiarity_home: TacticalFamiliarity,
    pub tactical_familiarity_away: TacticalFamiliarity,

    /// Sim-tick at which the last shape change fired for either side.
    /// Used by `evaluate_situational_shape` to enforce a hysteresis
    /// window so coaches don't flip shape every 5-second eval slice
    /// after a single goal. Initialised to `u64::MAX` so the first
    /// change is always allowed.
    pub last_shape_change_tick: u64,
    /// Match-clock timestamp (ms) until which play is DEAD after a
    /// goal — celebration, walk-back, reorganisation, the referee's
    /// restart. While `total_match_time` is below this, the engine
    /// loop advances only the clock: no ball physics, no player AI,
    /// no events. Real matches lose 45-75 s per goal here, and the
    /// pause is load-bearing for realism in two ways: it consumes the
    /// post-goal window in which the engine's freshly-reset formations
    /// were measurably easy to attack (goals begat goals — the
    /// equalizer-within-5-minutes rate ran 2.5x real), and it means
    /// play always resumes against a SET defense. 0 = play is live.
    pub dead_ball_until_ms: u64,
    /// The goal currently being celebrated, if any — the choreography that
    /// plays out inside the `dead_ball_until_ms` window and performs the
    /// restart at the end of it. `None` whenever play is live. See
    /// [`GoalCelebration`](crate::r#match::GoalCelebration).
    pub goal_celebration: Option<GoalCelebration>,
    /// The substitution being made, if any — the other cutscene that runs
    /// inside the `dead_ball_until_ms` window, and the one that changes the
    /// roster at the end of it.
    ///
    /// It can never overlap [`Self::goal_celebration`]: the substitution pass
    /// sits below the dead-ball `continue` in `play_inner`, so no swap can be
    /// staged while a goal is being celebrated, and no goal can be scored
    /// while a substitution is being made because there is no ball physics
    /// inside either window. See
    /// [`SubstitutionBreak`](super::super::touchline::SubstitutionBreak).
    pub substitution_break: Option<SubstitutionBreak>,
    /// Sim-minute at which the FIRST shape change fired in this match
    /// (any side). Stamped once and never overwritten so the result
    /// summary can show the moment the manager pivoted. `None` while
    /// no shape change has happened yet.
    pub first_shape_change_minute: Option<u8>,
    /// Tactics each team started the match with. Captured by
    /// `FootballEngine::play` from the kickoff `MatchSquad` so the
    /// result can show "started 4-4-2 → finished 4-3-3" without the
    /// engine needing to thread the squads through every state
    /// transition.
    pub starting_home_tactic: Option<MatchTacticType>,
    pub starting_away_tactic: Option<MatchTacticType>,

    /// Per-team skill composite aggregates, cached between refresh
    /// passes. The raw recompute walks every active player and
    /// queries 6-8 fatigue-aware skill composites, so we recompute
    /// only every ~100 ticks (~1 sim-second) instead of on every
    /// tactical refresh. Invalidated immediately by substitutions,
    /// red cards, or halftime side swaps via
    /// `invalidate_skill_aggregates`.
    pub home_skill_aggregates: TeamSkillAggregates,
    pub away_skill_aggregates: TeamSkillAggregates,
    pub last_skill_aggregate_tick: u64,
    /// True until the first compute. Marked true again whenever the
    /// active roster changes (sub / red card / formation swap).
    pub skill_aggregates_dirty: bool,

    /// The standard of football in this fixture, read once at the first
    /// aggregate pass and then held — see `MatchStandard`. `None` until
    /// then, which reads as "nothing to measure this match against yet".
    pub standard: Option<StandardReading>,

    /// Match-owned seedable RNG. Engine decision paths draw from this
    /// (substitution timing, shootout, foul cards, corner contest,
    /// passing / shooting / save / first-touch / tackle rolls, every
    /// player state) so a fixed seed produces a fixed sequence of
    /// rolls. `from_entropy` remains the production default; only the
    /// `MatchEngineConfig::seed = Some(_)` path pins the stream.
    /// Match-critical code should prefer `context.rng.unit_f32()` over
    /// `rand::random::<f32>()`.
    pub rng: MatchRng,

    /// Deterministic "today" used by substitution-eligibility checks
    /// (youth-protection branch in `process_substitutions`). Replaces
    /// the previous `Utc::now().naive_utc().date()` call inside the
    /// engine's hot loop. Sourced from `MatchEngineConfig::today`;
    /// defaults to the current wall-clock day for paths that don't
    /// pass a config.
    pub today: NaiveDate,

    /// Active "advantage" — the referee has spotted a foul but elected
    /// to let play continue because the fouled team is in a good
    /// position. Foul stats / card decisions are deferred until either:
    ///   * the advantage materialises (a shot, deep entry, sustained
    ///     possession past the window) → card recorded with no whistle,
    ///   * possession is lost inside the window → whistle goes back,
    ///     restart awarded, card decision applies at the moment of
    ///     the original foul,
    ///   * the window expires without either → play continues, card
    ///     decision still applies (delayed booking).
    ///
    /// `None` whenever no advantage is in play.
    pub pending_advantage: Option<PendingAdvantage>,
    /// The match-report lines kept as they happen — see [`MatchTally`].
    pub tally: MatchTally,
    pub time_wasting: TimeWastingLedger,
}

/// Snapshot of a foul that the referee elected to let play continue
/// on. The card decision is locked in at the time the foul occurred
/// so a tail-of-window booking matches what the ref saw, not the
/// state at expiry.
#[derive(Debug, Clone, Copy)]
pub struct PendingAdvantage {
    pub fouler_id: u32,
    /// Tick at which the foul happened — `expire_tick - this` gives
    /// elapsed window length.
    pub start_tick: u64,
    /// Tick at which the advantage window closes. If possession is
    /// lost before this, the foul is whistled retroactively. After
    /// this, play continues even on possession loss.
    pub expire_tick: u64,
    /// The fouled team (team that should KEEP possession for the
    /// advantage to materialise).
    pub fouled_team_id: u32,
    /// Severity of the original foul — drives the card decision.
    pub severity: FoulSeverity,
    pub source: FoulSource,
    /// Where the offence was. A pulled-back advantage restarts here, not
    /// wherever the ball has got to since.
    pub spot: Vector3<f32>,
    /// Card decision pre-computed at foul time so referee bias /
    /// match temperature at the moment of the foul govern the booking.
    pub yellow_prob: f32,
    pub red_prob: f32,
}

impl MatchContext {
    /// Seed `player`'s psychology from the state of mind he brought: the
    /// starters when the standard of the match is first read, a substitute
    /// when he comes on. Nothing until the standard is known.
    pub fn seed_psychology(&mut self, player: &MatchPlayer) {
        if let Some(reading) = self.standard {
            let occasion = Psychology::occasion(&self.environment, self.is_knockout);
            self.psychology.seed(player, reading.outfield, occasion);
        }
    }

    pub fn new(
        field: &MatchField,
        players: MatchPlayerCollection,
        score: Score,
        is_friendly: bool,
        is_knockout: bool,
    ) -> Self {
        Self::new_with_rules(
            field,
            players,
            score,
            is_friendly,
            is_knockout,
            MatchRules::resolve_default(if is_friendly {
                CompetitionKind::Friendly
            } else {
                CompetitionKind::League
            }),
        )
    }

    /// Build a context with an explicit RNG seed. Two matches built
    /// with the same seed will emit identical sequences from
    /// `context.rng` — the foundation for deterministic replay.
    pub fn new_with_seed(
        field: &MatchField,
        players: MatchPlayerCollection,
        score: Score,
        is_friendly: bool,
        is_knockout: bool,
        seed: u64,
    ) -> Self {
        let mut ctx = Self::new_with_rules(
            field,
            players,
            score,
            is_friendly,
            is_knockout,
            MatchRules::resolve_default(if is_friendly {
                CompetitionKind::Friendly
            } else {
                CompetitionKind::League
            }),
        );
        ctx.rng = MatchRng::from_seed(seed);
        ctx
    }

    pub fn new_with_rules(
        field: &MatchField,
        players: MatchPlayerCollection,
        score: Score,
        _is_friendly: bool,
        is_knockout: bool,
        rules: MatchRules,
    ) -> Self {
        MatchContext {
            state: GameState::new(),
            time: MatchTime::new(),
            score,
            field_size: MatchFieldSize::clone(&field.size),
            players,
            goal_positions: GoalPosition::from(&field.size),
            tactics: TeamsTactics::from_field(field),
            field_home_team_id: field.home_team_id,
            field_away_team_id: field.away_team_id,
            logging_enabled: false,
            total_match_time: 0,
            substitutions: Vec::new(),
            // Total substitution budget is sourced from the competition
            // rule set. Friendlies pass `usize::MAX` to waive the cap.
            // Knockout ties may add one more on entering ET — the
            // engine handles that bump independently.
            max_substitutions_per_team: rules.max_substitutions_per_team,
            max_substitutions_per_pass: rules.max_substitutions_per_pass,
            allow_extra_time_extra_sub: rules.allow_extra_time_extra_sub,
            substitution_windows: SubstitutionWindows::default(),
            additional_time_ms: 0,
            period_stoppage_time_ms: 0,
            stoppage_credit_ms: 0.0,
            period_kickoff_team: None,
            penalty_shootout_kicks: Vec::new(),
            penalty_saves: Vec::new(),
            chances: Vec::new(),
            #[cfg(feature = "match-logs")]
            box_episode: Default::default(),
            last_goal_tick: 0,
            last_conceded_tick: [u64::MAX, u64::MAX],
            substituted_out_stats: Vec::new(),
            substituted_out_physical_snapshots: Vec::new(),
            coach_home: MatchCoach::new(),
            coach_away: MatchCoach::new(),
            tactical_home: TeamTacticalState::initial(),
            tactical_away: TeamTacticalState::initial(),
            attack_home: AttackPlan::idle(),
            attack_away: AttackPlan::idle(),
            defence_home: DefensivePlan::idle(),
            defence_away: DefensivePlan::idle(),
            shape_home: TeamShape::idle(),
            shape_away: TeamShape::idle(),
            is_knockout,
            environment: MatchEnvironment::default(),
            conditions: EnvModifiers::default(),
            referee: RefereeProfile::default(),
            set_piece_history: SetPieceHistory::default(),
            psychology: PsychologyState::default(),
            chemistry: ChemistryMap::default(),
            tactical_familiarity_home: TacticalFamiliarity::default(),
            tactical_familiarity_away: TacticalFamiliarity::default(),
            last_shape_change_tick: u64::MAX,
            dead_ball_until_ms: 0,
            goal_celebration: None,
            substitution_break: None,
            first_shape_change_minute: None,
            starting_home_tactic: None,
            starting_away_tactic: None,
            home_skill_aggregates: TeamSkillAggregates::neutral(),
            away_skill_aggregates: TeamSkillAggregates::neutral(),
            last_skill_aggregate_tick: 0,
            skill_aggregates_dirty: true,
            standard: None,
            rng: MatchRng::from_entropy(),
            today: Utc::now().naive_utc().date(),
            pending_advantage: None,
            tally: MatchTally::default(),
            time_wasting: TimeWastingLedger::default(),
        }
    }

    /// Build a context from a `MatchEngineConfig`. Seed, fixture date,
    /// environment, referee profile, is_friendly, and is_knockout are
    /// all sourced from the config rather than patched on after
    /// construction — so a rainy / strict-ref / replayable test no
    /// longer has to construct a context, mutate fields, and hope
    /// nothing read them in between.
    pub fn new_with_config(
        field: &MatchField,
        players: MatchPlayerCollection,
        score: Score,
        config: &MatchEngineConfig,
    ) -> Self {
        let mut ctx = Self::new_with_rules(
            field,
            players,
            score,
            config.is_friendly(),
            config.is_knockout,
            MatchRules::resolve_default(config.competition),
        );
        ctx.rng = match config.seed {
            Some(s) => MatchRng::from_seed(s),
            None => MatchRng::from_entropy(),
        };
        ctx.today = config.today;
        ctx.environment = config.environment;
        ctx.environment.clamp_inputs();
        ctx.conditions = ctx.environment.modifiers();
        ctx.referee = config.referee;
        ctx.referee.clamp_inputs();
        ctx
    }

    /// Mark the per-team skill composite cache as stale so the next
    /// tactical refresh recomputes it. Call this whenever the active
    /// XI changes — substitution, red card, halftime side swap (the
    /// per-side fatigue lookups are positionally bound).
    #[inline]
    pub fn invalidate_skill_aggregates(&mut self) {
        self.skill_aggregates_dirty = true;
    }

    /// This team's attacking role assignment for the current possession.
    pub fn attack_plan_for_team(&self, team_id: u32) -> &AttackPlan {
        if team_id == self.field_home_team_id {
            &self.attack_home
        } else {
            &self.attack_away
        }
    }

    /// This team's defensive duty assignment for the current possession.
    pub fn defence_plan_for_team(&self, team_id: u32) -> &DefensivePlan {
        if team_id == self.field_home_team_id {
            &self.defence_home
        } else {
            &self.defence_away
        }
    }

    /// This team's live positional block.
    pub fn shape_for_team(&self, team_id: u32) -> &TeamShape {
        if team_id == self.field_home_team_id {
            &self.shape_home
        } else {
            &self.shape_away
        }
    }

    pub fn tactical_for_team(&self, team_id: u32) -> &TeamTacticalState {
        if team_id == self.field_home_team_id {
            &self.tactical_home
        } else {
            &self.tactical_away
        }
    }

    pub fn fill_details(&mut self) {
        for player in self
            .players
            .raw_players()
            .filter(|p| !p.statistics.is_empty())
        {
            for stat in &player.statistics.items {
                let detail = GoalDetail {
                    player_id: player.id,
                    time: stat.match_second,
                    stat_type: stat.stat_type,
                    is_auto_goal: stat.is_auto_goal,
                    origin: stat.origin,
                };

                self.score.add_goal_detail(detail);
            }
        }
    }

    pub fn enable_logging(&mut self) {
        self.logging_enabled = true;
    }

    pub fn familiarity_for_team(&self, team_id: u32) -> &TacticalFamiliarity {
        if team_id == self.field_home_team_id {
            &self.tactical_familiarity_home
        } else {
            &self.tactical_familiarity_away
        }
    }

    pub fn skill_aggregates_for_team(&self, team_id: u32) -> &TeamSkillAggregates {
        if team_id == self.field_home_team_id {
            &self.home_skill_aggregates
        } else {
            &self.away_skill_aggregates
        }
    }

    pub fn coach_for_team(&self, team_id: u32) -> &MatchCoach {
        if team_id == self.field_home_team_id {
            &self.coach_home
        } else {
            &self.coach_away
        }
    }

    /// How long `team_id` drags out a restart of this kind: nothing unless
    /// it is ahead late and its manager has told it to slow the game.
    pub fn time_wasting_delay_ms(
        &self,
        team_id: u32,
        aggression: f32,
        kind: TimeWastingRestart,
    ) -> u64 {
        let share = match self.coach_for_team(team_id).instruction {
            CoachInstruction::WasteTime => 1.0,
            CoachInstruction::SlowDown => 0.5,
            _ => return 0,
        };
        let minute = (self.total_match_time / 60_000) as u32;
        (TimeWasting::delay_ms(self.lead(team_id), minute, kind, aggression) as f32 * share) as u64
    }

    /// How far the clock is against `team_id` getting a restart taken,
    /// 0..1: the share of the second half gone while it is behind. Level
    /// or ahead it has no reason to hurry.
    pub fn restart_hurry(&self, team_id: u32) -> f32 {
        if self.lead(team_id) >= 0 {
            return 0.0;
        }
        let minute = self.total_match_time as f32 / 60_000.0;
        ((minute - 45.0) / 45.0).clamp(0.0, 1.0)
    }

    fn lead(&self, team_id: u32) -> i32 {
        let (home, away) = (
            self.score.home_team.get() as i32,
            self.score.away_team.get() as i32,
        );
        if team_id == self.field_home_team_id {
            home - away
        } else {
            away - home
        }
    }

    /// The referee's patience with a side slowing the game, spent by
    /// `delay_ms` more of it. True when it has run out and the man
    /// delaying is booked.
    pub fn time_wasting_caution(&mut self, team_id: u32, delay_ms: u64) -> bool {
        let is_home = team_id == self.field_home_team_id;
        let total = self.time_wasting.note_delay(is_home, delay_ms);
        let p = TimeWasting::yellow_prob(
            total,
            self.referee.strictness,
            self.time_wasting.offences(is_home),
        );
        if p > 0.0 && self.rng.unit_f32() < p {
            self.time_wasting.note_offence(is_home);
            true
        } else {
            false
        }
    }

    pub fn coach_for_team_mut(&mut self, team_id: u32) -> &mut MatchCoach {
        if team_id == self.field_home_team_id {
            &mut self.coach_home
        } else {
            &mut self.coach_away
        }
    }
}
