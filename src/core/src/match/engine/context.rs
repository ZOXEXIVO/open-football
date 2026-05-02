use crate::r#match::engine::chemistry::{ChemistryMap, TacticalFamiliarity};
use crate::r#match::engine::environment::MatchEnvironment;
use crate::r#match::engine::psychology::PsychologyState;
use crate::r#match::engine::referee::RefereeProfile;
use crate::r#match::engine::set_pieces::SetPieceHistory;
use crate::r#match::engine::result::{PenaltyShootoutKick, PlayerMatchEndStats};
use crate::r#match::{
    GameState, GoalDetail, GoalPosition, MATCH_EXTRA_TIME_MS, MATCH_HALF_TIME_MS, MatchCoach,
    MatchField, MatchFieldSize, MatchPlayerCollection, MatchState, MatchTime, Score,
    TeamTacticalState, TeamsTactics,
};
use nalgebra::Vector3;

const MATCH_TIME_INCREMENT_MS: u64 = 10;
const MAX_STOPPAGE_PER_PERIOD_MS: u64 = 15 * 60 * 1000;

pub struct SubstitutionRecord {
    pub team_id: u32,
    pub player_out_id: u32,
    pub player_in_id: u32,
    pub match_time: u64,
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
    pub additional_time_ms: u64,
    pub period_stoppage_time_ms: u64,
    pub penalty_shootout_kicks: Vec<PenaltyShootoutKick>,

    // Global goal cooldown: tick when last goal was scored
    // Prevents immediate scoring after kickoff restart
    pub last_goal_tick: u64,

    // Stats for players who were substituted out (preserved before replacement)
    pub substituted_out_stats: Vec<(u32, PlayerMatchEndStats)>,

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
}

impl MatchContext {
    pub fn new(
        field: &MatchField,
        players: MatchPlayerCollection,
        score: Score,
        is_friendly: bool,
        is_knockout: bool,
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
            // Knockout ties get one extra substitution once ET begins (FIFA rule).
            // Represented here as a flat limit; ET bonus applied on entry.
            max_substitutions_per_team: if is_friendly { usize::MAX } else { 5 },
            additional_time_ms: 0,
            period_stoppage_time_ms: 0,
            penalty_shootout_kicks: Vec::new(),
            last_goal_tick: 0,
            substituted_out_stats: Vec::new(),
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
        }
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

    pub fn can_shoot_after_goal(&self) -> bool {
        true
    }

    pub fn record_goal_tick(&mut self) {
        self.last_goal_tick = self.current_tick();
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
    ) {
        self.substitutions.push(SubstitutionRecord {
            team_id,
            player_out_id,
            player_in_id,
            match_time,
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
