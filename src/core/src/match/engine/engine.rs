use crate::r#match::ball::events::GoalSide;
use crate::r#match::engine::events::dispatcher::EventCollection;
use crate::r#match::events::EventDispatcher;
use crate::r#match::field::MatchField;
use crate::r#match::result::ResultMatchPositionData;
use crate::r#match::squad::TeamSquad;
use crate::r#match::{
    GameState, GameTickContext, GoalDetail, MatchPlayer, MatchResultRaw, Score, StateManager,
};
use crate::Tactics;
use nalgebra::Vector3;
use std::collections::HashMap;

pub struct FootballEngine<const W: usize, const H: usize> {}

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    pub fn new() -> Self {
        FootballEngine {}
    }

    pub fn play(left_squad: TeamSquad, right_squad: TeamSquad) -> MatchResultRaw {
        let score = Score::new(left_squad.team_id, right_squad.team_id);

        let players = MatchPlayerCollection::from_squads(&left_squad, &right_squad);

        let mut match_position_data = ResultMatchPositionData::new();

        let mut field = MatchField::new(W, H, left_squad, right_squad);

        let mut context = MatchContext::new(&field, players, score);

        let mut state_manager = StateManager::new();

        while let Some(state) = state_manager.next() {
            context.state.set(state);

            let play_state_result =
                Self::play_inner(&mut field, &mut context, &mut match_position_data);

            StateManager::handle_state_finish(&mut context, &mut field, play_state_result);
        }

        let mut result = MatchResultRaw::with_match_time(MATCH_HALF_TIME_MS);

        context.fill_details();

        result.score = Some(context.score.clone());

        result.left_team_players = field.left_side_players.expect("left team players");
        result.right_team_players = field.right_side_players.expect("right team players");

        result.position_data = match_position_data;

        result
    }

    fn play_inner(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
    ) -> PlayMatchStateResult {
        let result = PlayMatchStateResult::default();

        while context.increment_time() {
            Self::game_tick(field, context, match_data);
        }

        result
    }

    pub fn game_tick(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
    ) {
        let game_tick_context = GameTickContext::new(field);

        let mut events = EventCollection::new();

        Self::play_ball(field, context, &game_tick_context, &mut events);
        Self::play_players(field, context, &game_tick_context, &mut events);

        // dispatch events
        EventDispatcher::dispatch(events.to_vec(), field, context, true);

        Self::write_match_positions(field, context.time.time, match_data);
    }

    pub fn write_match_positions(
        field: &mut MatchField,
        timestamp: u64,
        match_data: &mut ResultMatchPositionData,
    ) {
        // player positions
        field.players.iter().for_each(|player| {
            match_data.add_player_positions(player.id, timestamp, player.position);
        });

        // player positions
        field.substitutes.iter().for_each(|sub_player| {
            match_data.add_player_positions(sub_player.id, timestamp, sub_player.position);
        });

        // write positions
        match_data.add_ball_positions(timestamp, field.ball.position);
    }

    fn play_ball(
        field: &mut MatchField,
        context: &MatchContext,
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        field
            .ball
            .update(context, &field.players, tick_context, events);
    }

    fn play_players(
        field: &mut MatchField,
        context: &mut MatchContext,
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        field
            .players
            .iter_mut()
            .map(|player| player.update(context, tick_context, events))
            .collect()
    }
}

pub enum MatchEvent {
    MatchPlayed(u32, bool, u8),
    Goal(u32),
    Assist(u32),
    Injury(u32),
}

pub struct MatchContext {
    pub state: GameState,
    pub time: MatchTime,
    pub score: Score,
    pub field_size: MatchFieldSize,
    pub players: MatchPlayerCollection,
    pub goal_positions: GoalPosition,

    pub tactics: TeamsTactics,
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
            tactics: TeamsTactics::from_field(&field),
        }
    }

    pub fn increment_time(&mut self) -> bool {
        self.time.increment(MATCH_TIME_INCREMENT_MS) < MATCH_HALF_TIME_MS
    }

    pub fn add_time(&mut self, time: u64) {
        self.time.increment(time);
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

    pub fn penalty_area(&self, is_home_team: bool) -> PenaltyArea {
        let field_width = self.field_size.width as f32;
        let field_height = self.field_size.height as f32;
        let penalty_area_width = 16.5; // Standard width of penalty area
        let penalty_area_depth = 40.3; // Standard depth of penalty area

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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BallSide {
    Left,
    Right,
}

impl From<BallSide> for u8 {
    fn from(side: BallSide) -> Self {
        match side {
            BallSide::Left => 0,
            BallSide::Right => 1,
        }
    }
}

#[derive(Clone)]
pub struct TeamsTactics {
    pub left: Tactics,
    pub right: Tactics,
}

impl TeamsTactics {
    pub fn from_field(field: &MatchField) -> Self {
        TeamsTactics {
            left: field.left_team_tactics.clone(),
            right: field.right_team_tactics.clone(),
        }
    }
}

#[derive(Clone)]
pub struct GoalPosition {
    pub left: Vector3<f32>,
    pub right: Vector3<f32>,
}

impl From<&MatchFieldSize> for GoalPosition {
    fn from(value: &MatchFieldSize) -> Self {
        // Left goal at x = 0, centered on width
        let left_goal = Vector3::new(0.0, value.height as f32 / 2.0, 0.0);

        // Right goal at x = length, centered on width
        let right_goal = Vector3::new(value.width as f32, (value.height / 2usize) as f32, 0.0);

        GoalPosition {
            left: left_goal,
            right: right_goal,
        }
    }
}

pub const GOAL_WIDTH: f32 = 60.0;

impl GoalPosition {
    pub fn is_goal(&self, ball_position: Vector3<f32>) -> Option<GoalSide> {
        const EPSILON: f32 = 0.5;

        if (ball_position.x - self.left.x).abs() < EPSILON {
            let top_goal_bound = self.left.y - GOAL_WIDTH;
            let bottom_goal_bound = self.left.y + GOAL_WIDTH;

            if ball_position.y >= top_goal_bound && ball_position.y <= bottom_goal_bound {
                return Some(GoalSide::Home);
            }
        }

        if (ball_position.x - self.right.x).abs() < EPSILON {
            let top_goal_bound = self.right.y - GOAL_WIDTH;
            let bottom_goal_bound = self.right.y + GOAL_WIDTH;

            if ball_position.y >= top_goal_bound && ball_position.y <= bottom_goal_bound {
                return Some(GoalSide::Away);
            }
        }

        None
    }
}

#[derive(Clone)]
pub struct MatchFieldSize {
    pub width: usize,
    pub height: usize,

    pub half_width: usize,
}

impl MatchFieldSize {
    pub fn new(width: usize, height: usize) -> Self {
        MatchFieldSize {
            width,
            height,
            half_width: width / 2,
        }
    }
}

pub struct MatchPlayerCollection {
    pub players: HashMap<u32, MatchPlayer>,
}

impl MatchPlayerCollection {
    pub fn from_squads(home_squad: &TeamSquad, away_squad: &TeamSquad) -> Self {
        let mut result = HashMap::new();

        // home_main
        for hs_m in &home_squad.main_squad {
            result.insert(hs_m.id, hs_m.clone());
        }

        // home_subs
        for hs_s in &home_squad.substitutes {
            result.insert(hs_s.id, hs_s.clone());
        }

        // home_main
        for as_m in &away_squad.main_squad {
            result.insert(as_m.id, as_m.clone());
        }

        // home_subs
        for as_s in &away_squad.substitutes {
            result.insert(as_s.id, as_s.clone());
        }

        MatchPlayerCollection { players: result }
    }

    pub fn by_id(&self, player_id: u32) -> Option<&MatchPlayer> {
        self.players.get(&player_id)
    }

    pub fn raw_players(&self) -> Vec<&MatchPlayer> {
        self.players.values().collect()
    }
}

const MATCH_TIME_INCREMENT_MS: u64 = 10;

#[cfg(debug_assertions)]
pub const MATCH_HALF_TIME_MS: u64 = 5 * 60 * 1000;
#[cfg(not(debug_assertions))]
pub const MATCH_HALF_TIME_MS: u64 = 45 * 60 * 1000;

pub const MATCH_TIME_MS: u64 = MATCH_HALF_TIME_MS * 2;

pub struct MatchTime {
    pub time: u64,
}

impl MatchTime {
    pub fn new() -> Self {
        MatchTime { time: 0 }
    }

    #[inline]
    pub fn increment(&mut self, val: u64) -> u64 {
        self.time += val;
        self.time
    }

    pub fn is_running_out(&self) -> bool {
        self.time > (2 * MATCH_TIME_MS / 3)
    }
}

#[derive(Default)]
pub struct PlayMatchStateResult {
    pub additional_time: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialization() {
        let match_time = MatchTime::new();
        assert_eq!(match_time.time, 0);
    }

    #[test]
    fn test_increment() {
        let mut match_time = MatchTime::new();

        let incremented_time = match_time.increment(10);
        assert_eq!(match_time.time, 10);
        assert_eq!(incremented_time, 10);

        let incremented_time_again = match_time.increment(5);
        assert_eq!(match_time.time, 15);
        assert_eq!(incremented_time_again, 15);
    }
}
