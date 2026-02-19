use crate::r#match::ball::events::GoalSide;
use crate::r#match::engine::events::dispatcher::EventCollection;
use crate::r#match::events::EventDispatcher;
use crate::r#match::field::MatchField;
use crate::r#match::result::ResultMatchPositionData;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::PlayerMatchEndStats;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer, MatchResultRaw, MatchSquad, MatchState, Score, StateManager, SubstitutionInfo};
use crate::{PlayerFieldPositionGroup, PlayerPositionType, Tactics};
use nalgebra::Vector3;
use std::collections::HashMap;

pub struct FootballEngine<const W: usize, const H: usize> {}

impl<const W: usize, const H: usize> Default for FootballEngine<W, H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    pub fn new() -> Self {
        FootballEngine {}
    }

    pub fn play(left_squad: MatchSquad, right_squad: MatchSquad) -> MatchResultRaw {
        let score = Score::new(left_squad.team_id, right_squad.team_id);

        let players = MatchPlayerCollection::from_squads(&left_squad, &right_squad);

        let mut match_position_data = if crate::is_match_events_mode() {
            ResultMatchPositionData::new_with_tracking()
        } else {
            ResultMatchPositionData::new()
        };

        let mut field = MatchField::new(W, H, left_squad, right_squad);

        let mut context = MatchContext::new(&field, players, score);

        if crate::is_match_events_mode() {
            context.enable_logging();
        }

        let mut state_manager = StateManager::new();

        while let Some(state) = state_manager.next() {
            context.state.set(state);

            let play_state_result =
                Self::play_inner(&mut field, &mut context, &mut match_position_data);

            StateManager::handle_state_finish(&mut context, &mut field, play_state_result);

            // Halftime substitutions: after first half finishes, make up to 3 subs per team
            if state == MatchState::FirstHalf {
                Self::process_substitutions(&mut field, &mut context, 3);
            }
        }

        let mut result = MatchResultRaw::with_match_time(context.total_match_time);

        context.fill_details();

        result.score = Some(context.score.clone());

        // Assign squads based on team IDs, not field positions
        // left_team_players and right_team_players in result represent home and away teams
        let left_side_squad = field.left_side_players.expect("left team players");
        let right_side_squad = field.right_side_players.expect("right team players");

        // Check which field side has the home team using FieldSquad's team_id
        if left_side_squad.team_id == field.home_team_id {
            // Home team is on the left side
            result.left_team_players = left_side_squad;
            result.right_team_players = right_side_squad;
        } else {
            // Home team is on the right side (after swap)
            result.left_team_players = right_side_squad;
            result.right_team_players = left_side_squad;
        }

        // Mark substitutes used in FieldSquads and copy substitution records to result
        for sub_record in &context.substitutions {
            if sub_record.team_id == result.left_team_players.team_id {
                result.left_team_players.mark_substitute_used(sub_record.player_in_id);
            } else {
                result.right_team_players.mark_substitute_used(sub_record.player_in_id);
            }

            result.substitutions.push(SubstitutionInfo {
                team_id: sub_record.team_id,
                player_out_id: sub_record.player_out_id,
                player_in_id: sub_record.player_in_id,
                match_time_ms: sub_record.match_time,
            });
        }

        result.position_data = match_position_data;

        // Extract per-player stats and calculate match ratings
        let score_ref = result.score.as_ref().unwrap();
        let home_goals = score_ref.home_team.get();
        let away_goals = score_ref.away_team.get();
        let home_team_id = score_ref.home_team.team_id;

        for player in &field.players {
            let goals = player.statistics.items.iter()
                .filter(|i| i.stat_type == MatchStatisticType::Goal && !i.is_auto_goal)
                .count() as u16;
            let assists = player.statistics.items.iter()
                .filter(|i| i.stat_type == MatchStatisticType::Assist)
                .count() as u16;

            let (player_team_goals, opponent_goals) = if player.team_id == home_team_id {
                (home_goals, away_goals)
            } else {
                (away_goals, home_goals)
            };

            let match_rating = Self::calculate_match_rating(
                goals,
                assists,
                player.statistics.passes_attempted,
                player.statistics.passes_completed,
                player.memory.shots_on_target as u16,
                player.memory.shots_taken as u16,
                player.statistics.tackles,
                player_team_goals,
                opponent_goals,
                player.tactical_position.current_position.position_group(),
            );

            result.player_stats.insert(player.id, PlayerMatchEndStats {
                shots_on_target: player.memory.shots_on_target as u16,
                shots_total: player.memory.shots_taken as u16,
                passes_attempted: player.statistics.passes_attempted,
                passes_completed: player.statistics.passes_completed,
                tackles: player.statistics.tackles,
                goals,
                assists,
                match_rating,
            });
        }

        result
    }

    fn play_inner(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
    ) -> PlayMatchStateResult {
        let result = PlayMatchStateResult::default();

        // Track last substitution check time for periodic second-half subs
        let mut last_sub_check_ms: u64 = 0;
        // Check every 15 game-minutes during second half
        const SUB_CHECK_INTERVAL_MS: u64 = 15 * 60 * 1000;

        while context.increment_time() {
            Self::game_tick(field, context, match_data);

            // Periodic substitution check during second half
            if context.state.match_state == MatchState::SecondHalf {
                let period_time = context.time.time;
                if period_time >= last_sub_check_ms + SUB_CHECK_INTERVAL_MS {
                    last_sub_check_ms = period_time;
                    Self::process_substitutions(field, context, 1);
                }
            }
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
        EventDispatcher::dispatch(events.to_vec(), field, context, match_data, true);

        // After all events are dispatched, force-reset positions if a goal was scored.
        // This prevents stale events (ClaimBall, PassTo, etc.) from overriding the goal reset.
        if field.ball.goal_scored {
            field.reset_players_positions();
            field.ball.reset();
            field.ball.goal_scored = false;
        }

        // Use total cumulative match time for positions
        Self::write_match_positions(field, context.total_match_time, match_data);
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

    fn process_substitutions(
        field: &mut MatchField,
        context: &mut MatchContext,
        max_subs_per_team: usize,
    ) {
        let team_ids = [field.home_team_id, field.away_team_id];

        for &team_id in &team_ids {
            if !context.can_substitute(team_id) {
                continue;
            }

            // Collect candidates to substitute off, sorted by condition (worst first)
            let mut tired_players: Vec<(u32, i16, PlayerPositionType)> = field
                .players
                .iter()
                .filter(|p| p.team_id == team_id)
                .filter(|p| {
                    let condition = p.player_attributes.condition;
                    let is_gk = p.tactical_position.current_position == PlayerPositionType::Goalkeeper;
                    if is_gk {
                        // Only sub GK if critically low
                        condition < 2000
                    } else {
                        condition < 5000
                    }
                })
                .map(|p| (p.id, p.player_attributes.condition, p.tactical_position.current_position))
                .collect();

            // Sort by condition ascending (most tired first)
            tired_players.sort_by_key(|&(_, cond, _)| cond);

            let mut subs_made = 0;
            for (player_out_id, _, position) in &tired_players {
                if subs_made >= max_subs_per_team || !context.can_substitute(team_id) {
                    break;
                }

                // Check if there are bench players available for this team
                let has_bench = field.substitutes.iter().any(|p| p.team_id == team_id);
                if !has_bench {
                    break;
                }

                let position_group = position.position_group();

                // Try to find a bench player with matching position group
                let sub_id = Self::find_best_substitute(field, team_id, position_group);

                if let Some(player_in_id) = sub_id {
                    if field.substitute_player(*player_out_id, player_in_id) {
                        context.record_substitution(
                            team_id,
                            *player_out_id,
                            player_in_id,
                            context.total_match_time,
                        );

                        // Remove substituted-out player from context so AI
                        // strategies don't try to look up their position
                        context.players.remove_player(*player_out_id);

                        // Update the substitute's entry in context.players with
                        // their new tactical position/role from the field
                        if let Some(field_player) = field.get_player(player_in_id) {
                            context.players.update_player(player_in_id, field_player.clone());
                        }

                        // Mark in the appropriate FieldSquad
                        let left_squad = field.left_side_players.as_mut();
                        let right_squad = field.right_side_players.as_mut();
                        if let Some(squad) = left_squad {
                            if squad.team_id == team_id {
                                squad.mark_substitute_used(player_in_id);
                            }
                        }
                        if let Some(squad) = right_squad {
                            if squad.team_id == team_id {
                                squad.mark_substitute_used(player_in_id);
                            }
                        }

                        subs_made += 1;
                    }
                }
            }
        }
    }

    fn find_best_substitute(
        field: &MatchField,
        team_id: u32,
        position_group: PlayerFieldPositionGroup,
    ) -> Option<u32> {
        let team_subs: Vec<&MatchPlayer> = field
            .substitutes
            .iter()
            .filter(|p| p.team_id == team_id)
            .collect();

        if team_subs.is_empty() {
            return None;
        }

        // Try to find a sub with matching position group
        let position_match = team_subs
            .iter()
            .filter(|p| p.tactical_position.current_position.position_group() == position_group)
            .max_by_key(|p| p.player_attributes.current_ability);

        if let Some(sub) = position_match {
            return Some(sub.id);
        }

        // Fallback: best available outfield sub (avoid using GK as outfield replacement)
        let fallback = team_subs
            .iter()
            .filter(|p| p.tactical_position.current_position.position_group() != PlayerFieldPositionGroup::Goalkeeper)
            .max_by_key(|p| p.player_attributes.current_ability);

        if let Some(sub) = fallback {
            return Some(sub.id);
        }

        // Last resort: any available sub
        team_subs
            .iter()
            .max_by_key(|p| p.player_attributes.current_ability)
            .map(|p| p.id)
    }

    /// Calculate a Football Manager-style match rating (1.0 - 10.0, base 6.0)
    fn calculate_match_rating(
        goals: u16,
        assists: u16,
        passes_attempted: u16,
        passes_completed: u16,
        shots_on_target: u16,
        shots_total: u16,
        tackles: u16,
        team_goals: u8,
        opponent_goals: u8,
        position_group: PlayerFieldPositionGroup,
    ) -> f32 {
        let mut rating: f32 = 6.0;

        // Goals: +1.0 each, capped at +3.0
        rating += (goals as f32 * 1.0).min(3.0);

        // Assists: +0.5 each, capped at +1.5
        rating += (assists as f32 * 0.5).min(1.5);

        // Pass completion bonus/penalty
        if passes_attempted > 5 {
            let pass_pct = passes_completed as f32 / passes_attempted as f32;
            // 70% = neutral, 90%+ = +0.4, below 50% = -0.4
            let pass_bonus = (pass_pct - 0.70) * 2.0;
            rating += pass_bonus.clamp(-0.4, 0.5);
        }

        // Shooting accuracy (only meaningful if shots taken)
        if shots_total > 0 {
            let shot_accuracy = shots_on_target as f32 / shots_total as f32;
            let shot_bonus = (shot_accuracy - 0.4) * 0.6;
            rating += shot_bonus.clamp(-0.2, 0.3);
        }

        // Defensive contribution - tackles
        // Weighted more for defenders/defensive midfielders
        let tackle_weight = match position_group {
            PlayerFieldPositionGroup::Defender => 0.12,
            PlayerFieldPositionGroup::Midfielder => 0.08,
            _ => 0.05,
        };
        rating += (tackles as f32 * tackle_weight).min(0.5);

        // Team result
        if team_goals > opponent_goals {
            rating += 0.3; // Win bonus
        } else if team_goals < opponent_goals {
            rating -= 0.2; // Loss penalty
        }

        // Clean sheet bonus for defenders and goalkeepers
        if opponent_goals == 0 {
            match position_group {
                PlayerFieldPositionGroup::Goalkeeper => rating += 0.8,
                PlayerFieldPositionGroup::Defender => rating += 0.4,
                PlayerFieldPositionGroup::Midfielder => rating += 0.1,
                _ => {}
            }
        }

        // Conceding many goals penalty for defenders/GK
        if opponent_goals >= 3 {
            match position_group {
                PlayerFieldPositionGroup::Goalkeeper => rating -= 0.5,
                PlayerFieldPositionGroup::Defender => rating -= 0.3,
                _ => {}
            }
        }

        rating.clamp(1.0, 10.0)
    }
}

pub enum MatchEvent {
    MatchPlayed(u32, bool, u8),
    Goal(u32),
    Assist(u32),
    Injury(u32),
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
pub const GOAL_HEIGHT: f32 = 2.44; // Crossbar height in meters (z-axis is in meters)

impl GoalPosition {
    pub fn is_goal(&self, ball_position: Vector3<f32>) -> Option<GoalSide> {
        // Ball must be below the crossbar to count as a goal
        if ball_position.z > GOAL_HEIGHT {
            return None;
        }

        // Check if ball has crossed or reached the left goal line (x <= 0)
        if ball_position.x <= self.left.x {
            let top_goal_bound = self.left.y - GOAL_WIDTH;
            let bottom_goal_bound = self.left.y + GOAL_WIDTH;

            if ball_position.y >= top_goal_bound && ball_position.y <= bottom_goal_bound {
                return Some(GoalSide::Home);
            }
        }

        // Check if ball has crossed or reached the right goal line (x >= field_width)
        if ball_position.x >= self.right.x {
            let top_goal_bound = self.right.y - GOAL_WIDTH;
            let bottom_goal_bound = self.right.y + GOAL_WIDTH;

            if ball_position.y >= top_goal_bound && ball_position.y <= bottom_goal_bound {
                return Some(GoalSide::Away);
            }
        }

        None
    }

    /// Check if ball crossed the goal line within goal width but ABOVE the crossbar.
    /// Returns which side the ball went over (goal kick for the defending team).
    pub fn is_over_goal(&self, ball_position: Vector3<f32>) -> Option<GoalSide> {
        // Only triggers when ball is above the crossbar
        if ball_position.z <= GOAL_HEIGHT {
            return None;
        }

        // Check left goal line
        if ball_position.x <= self.left.x {
            let top_goal_bound = self.left.y - GOAL_WIDTH;
            let bottom_goal_bound = self.left.y + GOAL_WIDTH;

            if ball_position.y >= top_goal_bound && ball_position.y <= bottom_goal_bound {
                return Some(GoalSide::Home);
            }
        }

        // Check right goal line
        if ball_position.x >= self.right.x {
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
    pub fn from_squads(home_squad: &MatchSquad, away_squad: &MatchSquad) -> Self {
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

    pub fn remove_player(&mut self, player_id: u32) {
        self.players.remove(&player_id);
    }

    pub fn update_player(&mut self, player_id: u32, player: MatchPlayer) {
        self.players.insert(player_id, player);
    }
}

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
