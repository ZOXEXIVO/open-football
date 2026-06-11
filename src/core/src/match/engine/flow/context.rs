use crate::MatchTacticType;
use crate::r#match::engine::chemistry::{ChemistryMap, TacticalFamiliarity};
use crate::r#match::engine::environment::MatchEnvironment;
use crate::r#match::engine::flow::rng::MatchRng;
use crate::r#match::engine::player::events::players::FoulSeverity;
use crate::r#match::engine::psychology::PsychologyState;
use crate::r#match::engine::referee::RefereeProfile;
use crate::r#match::engine::result::{
    PenaltyShootoutKick, PlayerMatchEndStats, PlayerMatchPhysicalSnapshot,
};
use crate::r#match::engine::set_pieces::SetPieceHistory;
use crate::r#match::rules::MatchRules;
use chrono::{NaiveDate, Utc};

/// Full match-construction inputs. Replaces the loose
/// `play_seeded(.., seed)` signature for callers that need to inject
/// weather, referee profile, or fixture date alongside the seed —
/// notably the calibration harness and any replay/test path that wants
/// a real rainy / strict-ref / cup-final match instead of the
/// engine's neutral defaults.
///
/// `play` and `play_seeded` are kept as compatibility wrappers around
/// `play_with_config` so existing call sites don't move.
#[derive(Debug, Clone)]
pub struct MatchEngineConfig {
    pub seed: Option<u64>,
    pub today: NaiveDate,
    pub environment: MatchEnvironment,
    pub referee: RefereeProfile,
    pub is_friendly: bool,
    pub is_knockout: bool,
    pub match_recordings: bool,
}

impl Default for MatchEngineConfig {
    fn default() -> Self {
        MatchEngineConfig {
            seed: None,
            today: Utc::now().naive_utc().date(),
            environment: MatchEnvironment::default(),
            referee: RefereeProfile::default(),
            is_friendly: false,
            is_knockout: false,
            match_recordings: false,
        }
    }
}

impl MatchEngineConfig {
    /// Convenience: build a seeded config with everything else default.
    pub fn seeded(seed: u64) -> Self {
        MatchEngineConfig {
            seed: Some(seed),
            ..Default::default()
        }
    }
}
use crate::r#match::{
    GameState, GoalDetail, GoalPosition, MATCH_EXTRA_TIME_MS, MATCH_HALF_TIME_MS, MatchCoach,
    MatchField, MatchFieldSize, MatchPlayerCollection, MatchState, MatchTime, PlayerSide, Score,
    TeamSkillAggregates, TeamTacticalState, TeamsTactics,
};
use nalgebra::Vector3;

const MATCH_TIME_INCREMENT_MS: u64 = 10;
const MAX_STOPPAGE_PER_PERIOD_MS: u64 = 15 * 60 * 1000;

pub struct SubstitutionRecord {
    pub team_id: u32,
    pub player_out_id: u32,
    pub player_in_id: u32,
    pub match_time: u64,
    /// Reason the swap fired. Stamped at the call-site so post-match
    /// emit logic can distinguish protective swaps (injury / youth)
    /// from discretionary tactical hooks.
    pub reason: crate::r#match::engine::flow::result::SubstitutionReason,
}

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
    pub additional_time_ms: u64,
    pub period_stoppage_time_ms: u64,
    pub penalty_shootout_kicks: Vec<PenaltyShootoutKick>,

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

    /// Knockout-format match — enables extra time + penalty shootout when
    /// the score is level at the end of regulation.
    pub is_knockout: bool,

    /// Weather + pitch + crowd + importance. Defaults to a neutral
    /// fixture; harnesses can override before kickoff.
    pub environment: MatchEnvironment,

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
    /// `None` whenever no advantage is in play.
    pub pending_advantage: Option<PendingAdvantage>,
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
    /// Card decision pre-computed at foul time so referee bias /
    /// match temperature at the moment of the foul govern the booking.
    pub yellow_prob: f32,
    pub red_prob: f32,
}

impl MatchContext {
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
            MatchRules::resolve_default(is_friendly, is_knockout),
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
            MatchRules::resolve_default(is_friendly, is_knockout),
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
            additional_time_ms: 0,
            period_stoppage_time_ms: 0,
            penalty_shootout_kicks: Vec::new(),
            last_goal_tick: 0,
            last_conceded_tick: [u64::MAX, u64::MAX],
            substituted_out_stats: Vec::new(),
            substituted_out_physical_snapshots: Vec::new(),
            coach_home: MatchCoach::new(),
            coach_away: MatchCoach::new(),
            tactical_home: TeamTacticalState::initial(),
            tactical_away: TeamTacticalState::initial(),
            is_knockout,
            environment: MatchEnvironment::default(),
            referee: RefereeProfile::default(),
            set_piece_history: SetPieceHistory::default(),
            psychology: PsychologyState::default(),
            chemistry: ChemistryMap::default(),
            tactical_familiarity_home: TacticalFamiliarity::default(),
            tactical_familiarity_away: TacticalFamiliarity::default(),
            last_shape_change_tick: u64::MAX,
            dead_ball_until_ms: 0,
            first_shape_change_minute: None,
            starting_home_tactic: None,
            starting_away_tactic: None,
            home_skill_aggregates: TeamSkillAggregates::neutral(),
            away_skill_aggregates: TeamSkillAggregates::neutral(),
            last_skill_aggregate_tick: 0,
            skill_aggregates_dirty: true,
            rng: MatchRng::from_entropy(),
            today: Utc::now().naive_utc().date(),
            pending_advantage: None,
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
            config.is_friendly,
            config.is_knockout,
            MatchRules::resolve_default(config.is_friendly, config.is_knockout),
        );
        ctx.rng = match config.seed {
            Some(s) => MatchRng::from_seed(s),
            None => MatchRng::from_entropy(),
        };
        ctx.today = config.today;
        ctx.environment = config.environment;
        ctx.environment.clamp_inputs();
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

    pub fn tactical_for_team(&self, team_id: u32) -> &TeamTacticalState {
        if team_id == self.field_home_team_id {
            &self.tactical_home
        } else {
            &self.tactical_away
        }
    }

    pub fn increment_time(&mut self) -> bool {
        let new_time = self.time.increment(MATCH_TIME_INCREMENT_MS);

        self.total_match_time += MATCH_TIME_INCREMENT_MS;

        match self.state.match_state {
            MatchState::FirstHalf | MatchState::SecondHalf => {
                new_time < MATCH_HALF_TIME_MS + self.period_stoppage_time_ms
            }
            MatchState::ExtraTime => new_time < MATCH_EXTRA_TIME_MS + self.period_stoppage_time_ms,
            _ => false,
        }
    }

    pub fn reset_period_time(&mut self) {
        self.time = MatchTime::new();
        self.period_stoppage_time_ms = 0;
    }

    pub fn add_time(&mut self, time: u64) {
        self.time.increment(time);
        self.total_match_time += time;
    }

    pub fn record_stoppage_time(&mut self, time: u64) {
        if !matches!(
            self.state.match_state,
            MatchState::FirstHalf | MatchState::SecondHalf | MatchState::ExtraTime
        ) {
            return;
        }

        let room = MAX_STOPPAGE_PER_PERIOD_MS.saturating_sub(self.period_stoppage_time_ms);
        let added = time.min(room);
        self.period_stoppage_time_ms += added;
        self.additional_time_ms += added;
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
                };

                self.score.add_goal_detail(detail);
            }
        }
    }

    pub fn current_tick(&self) -> u64 {
        self.total_match_time / 10
    }

    /// Diagnostic switch: when the `OF_SCORE_BLIND` env var is set, all
    /// BEHAVIORAL reads of the scoreline return neutral (0-0) — coach
    /// instructions, tactical game management, chasing/protect lifts
    /// and desperation all act as if the match were level, while the
    /// real score still accumulates for the result. Used by the dev
    /// harness to measure how much of the engine's draw-correlation
    /// surplus is carried by the score-reactive regime versus emergent
    /// match state. Read once per process; keep for future calibration
    /// rounds (debug infrastructure — do not remove).
    pub fn score_blind() -> bool {
        use std::sync::OnceLock;
        static BLIND: OnceLock<bool> = OnceLock::new();
        *BLIND.get_or_init(|| std::env::var("OF_SCORE_BLIND").is_ok())
    }

    /// Match minute before which BEHAVIORAL score reactions stay off —
    /// teams play their football regardless of the scoreline until the
    /// final quarter, exactly like real sides do (managers don't park
    /// the bus at minute 30 or go all-out at minute 40; reactive
    /// game-state football is a post-~65' phenomenon, which is also
    /// where real substitution/instruction activity clusters).
    ///
    /// Why this gate is load-bearing: a score-blind A/B run measured
    /// the engine at rho = −0.05 / 23.5% draws (real: ~0 / 25%) with
    /// reactions off, versus rho = +0.51 / 43-46% draws with them on —
    /// the score-reactive regime, running from minute 1, carried the
    /// ENTIRE equal-strength draw surplus (trailing teams scored 2.35
    /// goals/90 vs leaders' 1.08; real football keeps game-state rates
    /// nearly equal). Bounding the regime to the final ~28 minutes
    /// keeps its realistic late-game drama while capping its
    /// correlation budget.
    pub const SCORE_REACTION_FROM_MINUTE: u32 = 62;

    /// The scoreline as BEHAVIOR is allowed to see it: 0-0 (level)
    /// before `SCORE_REACTION_FROM_MINUTE`, the real difference after.
    /// All tactical / coach / desperation score reads route through
    /// the three aggregation points that consume this.
    pub fn behavioral_score_visible(&self) -> bool {
        if Self::score_blind() {
            return false;
        }
        (self.total_match_time / 60_000) as u32 >= Self::SCORE_REACTION_FROM_MINUTE
    }

    pub fn can_shoot_after_goal(&self) -> bool {
        true
    }

    pub fn record_goal_tick(&mut self) {
        self.last_goal_tick = self.current_tick();
    }

    /// Mark that the given side just conceded a goal. Read by the
    /// forward shot decision to dampen willingness in the immediate
    /// post-concede window. See `last_conceded_tick` docs for the
    /// mechanism rationale.
    pub fn record_conceded(&mut self, side: PlayerSide) {
        let tick = self.current_tick();
        let idx = match side {
            PlayerSide::Left => 0,
            PlayerSide::Right => 1,
        };
        self.last_conceded_tick[idx] = tick;
    }

    /// Did the given side concede within the last `window_ticks` ticks?
    /// One tick is 10 ms of match time, so 6000 ticks ≈ 60 s.
    pub fn conceded_recently(&self, side: PlayerSide, window_ticks: u64) -> bool {
        let idx = match side {
            PlayerSide::Left => 0,
            PlayerSide::Right => 1,
        };
        let last = self.last_conceded_tick[idx];
        if last == u64::MAX {
            return false;
        }
        self.current_tick().saturating_sub(last) < window_ticks
    }

    pub fn enable_logging(&mut self) {
        self.logging_enabled = true;
    }

    pub fn subs_used_by_team(&self, team_id: u32) -> usize {
        self.substitutions
            .iter()
            .filter(|s| s.team_id == team_id)
            .count()
    }

    pub fn can_substitute(&self, team_id: u32) -> bool {
        self.subs_used_by_team(team_id) < self.max_substitutions_per_team
    }

    pub fn coach_for_team(&self, team_id: u32) -> &MatchCoach {
        if team_id == self.field_home_team_id {
            &self.coach_home
        } else {
            &self.coach_away
        }
    }

    pub fn coach_for_team_mut(&mut self, team_id: u32) -> &mut MatchCoach {
        if team_id == self.field_home_team_id {
            &mut self.coach_home
        } else {
            &mut self.coach_away
        }
    }

    pub fn record_substitution(
        &mut self,
        team_id: u32,
        player_out_id: u32,
        player_in_id: u32,
        match_time: u64,
        reason: crate::r#match::engine::flow::result::SubstitutionReason,
    ) {
        self.substitutions.push(SubstitutionRecord {
            team_id,
            player_out_id,
            player_in_id,
            match_time,
            reason,
        });
    }

    pub fn penalty_area(&self, is_home_team: bool) -> PenaltyArea {
        let field_width = self.field_size.width as f32;
        let field_height = self.field_size.height as f32;
        let scale = field_width / 105.0; // Field units per real meter
        let penalty_area_width = 40.32 * scale; // 40.32m wide (centered on goal)
        let penalty_area_depth = 16.5 * scale; // 16.5m deep from goal line

        if is_home_team {
            PenaltyArea::new(
                Vector3::new(0.0, (field_height - penalty_area_width) / 2.0, 0.0),
                Vector3::new(
                    penalty_area_depth,
                    (field_height + penalty_area_width) / 2.0,
                    0.0,
                ),
            )
        } else {
            PenaltyArea::new(
                Vector3::new(
                    field_width - penalty_area_depth,
                    (field_height - penalty_area_width) / 2.0,
                    0.0,
                ),
                Vector3::new(field_width, (field_height + penalty_area_width) / 2.0, 0.0),
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PenaltyArea {
    pub min: Vector3<f32>,
    pub max: Vector3<f32>,
}

impl PenaltyArea {
    pub fn new(min: Vector3<f32>, max: Vector3<f32>) -> Self {
        PenaltyArea { min, max }
    }

    pub fn contains(&self, point: &Vector3<f32>) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }
}
