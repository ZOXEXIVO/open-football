use nalgebra::Vector3;
use crate::r#match::{GameState, GoalDetail, GoalPosition, MatchField, MatchFieldSize, MatchPlayerCollection, MatchState, MatchTime, Score, TeamsTactics, MATCH_HALF_TIME_MS};

const MATCH_TIME_INCREMENT_MS: u64 = 10;

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
}

impl MatchContext {
    pub fn new(field: &MatchField, players: MatchPlayerCollection, score: Score) -> Self {
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
        }
    }

    pub fn increment_time(&mut self) -> bool {
        let new_time = self.time.increment(MATCH_TIME_INCREMENT_MS);

        self.total_match_time += MATCH_TIME_INCREMENT_MS;

        match self.state.match_state {
            MatchState::FirstHalf | MatchState::SecondHalf => {
                new_time < MATCH_HALF_TIME_MS
            },
            _ => false
        }
    }

    pub fn reset_period_time(&mut self) {
        self.time = MatchTime::new();
    }

    pub fn add_time(&mut self, time: u64) {
        self.time.increment(time);
        self.total_match_time += time;
    }

    pub fn fill_details(&mut self) {
        for player in self
            .players
            .raw_players()
            .iter()
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

    pub fn enable_logging(&mut self) {
        self.logging_enabled = true;
    }

    pub fn penalty_area(&self, is_home_team: bool) -> PenaltyArea {
        let field_width = self.field_size.width as f32;
        let field_height = self.field_size.height as f32;
        let scale = field_width / 105.0; // Field units per real meter
        let penalty_area_width = 40.32 * scale; // 40.32m wide (centered on goal)
        let penalty_area_depth = 16.5 * scale;  // 16.5m deep from goal line

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