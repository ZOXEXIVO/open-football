use crate::r#match::engine::events::dispatcher::EventCollection;
use crate::r#match::engine::goal::handle_goal_reset;
use crate::r#match::engine::rating::calculate_match_rating;
use crate::r#match::engine::substitutions::process_substitutions;
use crate::r#match::events::EventDispatcher;
use crate::r#match::field::MatchField;
use crate::r#match::result::ResultMatchPositionData;
use crate::r#match::PlayerMatchEndStats;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer, MatchResultRaw, MatchSquad, MatchState, Score, StateManager, SubstitutionInfo};
use crate::{PlayerFieldPositionGroup, PlayerPositionType, Tactics};
use rand::RngExt;
use std::collections::HashMap;

// ───────────────────────────────────────────────────────────────────────────────
// FootballEngine — match orchestration
// ───────────────────────────────────────────────────────────────────────────────

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

    pub fn play(left_squad: MatchSquad, right_squad: MatchSquad, match_recordings: bool, is_friendly: bool) -> MatchResultRaw {
        let score = Score::new(left_squad.team_id, right_squad.team_id);

        let players = MatchPlayerCollection::from_squads(&left_squad, &right_squad);

        let mut match_position_data = if !match_recordings {
            ResultMatchPositionData::empty()
        } else if crate::is_match_events_mode() {
            ResultMatchPositionData::new_with_tracking()
        } else {
            ResultMatchPositionData::new()
        };

        let mut field = MatchField::new(W, H, left_squad, right_squad);

        let mut context = MatchContext::new(&field, players, score, is_friendly);

        if crate::is_match_events_mode() {
            context.enable_logging();
        }

        let mut state_manager = StateManager::new();

        while let Some(state) = state_manager.next() {
            context.state.set(state);

            let play_state_result =
                Self::play_inner(&mut field, &mut context, &mut match_position_data);

            StateManager::handle_state_finish(&mut context, &mut field, play_state_result);

        }

        Self::build_result(field, context, match_position_data)
    }

    fn build_result(
        field: MatchField,
        mut context: MatchContext,
        match_position_data: ResultMatchPositionData,
    ) -> MatchResultRaw {
        let mut result = MatchResultRaw::with_match_time(context.total_match_time);

        context.fill_details();

        result.score = Some(context.score.clone());

        // Assign squads based on team IDs, not field positions
        let left_side_squad = field.left_side_players.expect("left team players");
        let right_side_squad = field.right_side_players.expect("right team players");

        if left_side_squad.team_id == field.home_team_id {
            result.left_team_players = left_side_squad;
            result.right_team_players = right_side_squad;
        } else {
            result.left_team_players = right_side_squad;
            result.right_team_players = left_side_squad;
        }

        // Copy substitution records to result
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
            let goals = player.statistics.goals_count();
            let assists = player.statistics.assists_count();

            let (player_team_goals, opponent_goals) = if player.team_id == home_team_id {
                (home_goals, away_goals)
            } else {
                (away_goals, home_goals)
            };

            let match_rating = calculate_match_rating(
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
                xg: player.memory.xg_total,
            });
        }

        // Include stats from substituted-out players
        for (player_id, mut stats) in context.substituted_out_stats.drain(..) {
            let team_id = context.substitutions.iter()
                .find(|s| s.player_out_id == player_id)
                .map(|s| s.team_id);

            let (player_team_goals, opponent_goals) = if team_id == Some(home_team_id) {
                (home_goals, away_goals)
            } else {
                (away_goals, home_goals)
            };

            stats.match_rating = calculate_match_rating(
                stats.goals,
                stats.assists,
                stats.passes_attempted,
                stats.passes_completed,
                stats.shots_on_target,
                stats.shots_total,
                stats.tackles,
                player_team_goals,
                opponent_goals,
                PlayerFieldPositionGroup::Midfielder,
            );

            result.player_stats.insert(player_id, stats);
        }

        result
    }

    // ───────────────────────────────────────────────────────────────────────
    // Match state loop
    // ───────────────────────────────────────────────────────────────────────

    fn play_inner(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
    ) -> PlayMatchStateResult {
        let result = PlayMatchStateResult::default();

        let mut next_sub_time_ms: u64 = 0;
        let mut sub_times_initialized = false;

        let mut tick_ctx = GameTickContext::new(field);
        let mut events = EventCollection::with_capacity(10);

        let mut tick_parity: u32 = 0;
        let mut coach_eval_counter: u32 = 0;

        while context.increment_time() {
            tick_parity += 1;
            coach_eval_counter += 1;

            // Coach evaluates every 500 ticks (~5 seconds of match time)
            if coach_eval_counter >= 500 {
                coach_eval_counter = 0;
                Self::evaluate_coaches(field, context);
            }

            // Full tick: ball + player AI + events
            // Light tick: ball + player movement only (no AI re-evaluation)
            if tick_parity & 1 == 0 {
                Self::game_tick_light(field, context, match_data, &mut events);
            } else {
                Self::game_tick_inner(field, context, match_data, &mut tick_ctx, &mut events);
            }

            // Substitutions only during second half
            if context.state.match_state == MatchState::SecondHalf {
                if !sub_times_initialized {
                    let mut rng = rand::rng();
                    next_sub_time_ms = rng.random_range(10..20) * 60 * 1000;
                    sub_times_initialized = true;
                }

                let period_time = context.time.time;
                if period_time >= next_sub_time_ms {
                    process_substitutions(field, context, 2);
                    let mut rng = rand::rng();
                    next_sub_time_ms = period_time + rng.random_range(5..15) * 60 * 1000;
                }
            }
        }

        result
    }

    // ───────────────────────────────────────────────────────────────────────
    // Coach evaluation
    // ───────────────────────────────────────────────────────────────────────

    fn evaluate_coaches(field: &MatchField, context: &mut MatchContext) {
        let home_goals = context.score.home_team.get() as i8;
        let away_goals = context.score.away_team.get() as i8;
        let current_tick = context.current_tick();

        let match_progress = context.total_match_time as f32 / MATCH_TIME_MS as f32;

        let (home_condition_sum, home_count, away_condition_sum, away_count) =
            field.players.iter().fold(
                (0.0f32, 0u32, 0.0f32, 0u32),
                |(hc, hn, ac, an), player| {
                    let cond = player.player_attributes.condition as f32 / 10000.0;
                    if player.team_id == context.field_home_team_id {
                        (hc + cond, hn + 1, ac, an)
                    } else {
                        (hc, hn, ac + cond, an + 1)
                    }
                },
            );

        let home_avg_condition = if home_count > 0 { home_condition_sum / home_count as f32 } else { 0.5 };
        let away_avg_condition = if away_count > 0 { away_condition_sum / away_count as f32 } else { 0.5 };

        context.coach_home.evaluate(
            home_goals - away_goals,
            match_progress,
            home_avg_condition,
            current_tick,
        );

        context.coach_away.evaluate(
            away_goals - home_goals,
            match_progress,
            away_avg_condition,
            current_tick,
        );
    }

    // ───────────────────────────────────────────────────────────────────────
    // Tick processing
    // ───────────────────────────────────────────────────────────────────────

    pub fn game_tick(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
        tick_ctx: &mut GameTickContext,
    ) {
        let mut events = EventCollection::with_capacity(10);
        Self::game_tick_inner(field, context, match_data, tick_ctx, &mut events);
    }

    /// Light tick: full ball logic (physics, ownership, goals) but players only move.
    fn game_tick_light(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
        events: &mut EventCollection,
    ) {
        events.clear();

        field.ball.update_light(context, &field.players, events);

        for player in field.players.iter_mut() {
            player.check_boundary_collision(context);
            player.move_to();
        }

        if events.has_events() {
            EventDispatcher::dispatch(events, field, context, match_data, true);
            handle_goal_reset(field, context);
        }

        Self::write_match_positions(field, context.total_match_time, match_data);
    }

    fn game_tick_inner(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
        tick_ctx: &mut GameTickContext,
        events: &mut EventCollection,
    ) {
        tick_ctx.update(field);

        events.clear();

        Self::play_ball(field, context, tick_ctx, events);
        Self::play_players(field, context, tick_ctx, events);

        EventDispatcher::dispatch(events, field, context, match_data, true);

        handle_goal_reset(field, context);

        Self::write_match_positions(field, context.total_match_time, match_data);
    }

    // ───────────────────────────────────────────────────────────────────────
    // Position recording
    // ───────────────────────────────────────────────────────────────────────

    /// Record positions every 30ms (every 3rd tick) instead of every 10ms.
    const POSITION_RECORD_INTERVAL_MS: u64 = 30;

    #[inline]
    pub fn write_match_positions(
        field: &mut MatchField,
        timestamp: u64,
        match_data: &mut ResultMatchPositionData,
    ) {
        if !match_data.is_tracking_positions() {
            return;
        }

        if timestamp % Self::POSITION_RECORD_INTERVAL_MS != 0 {
            return;
        }

        let track_events = match_data.is_tracking_events();

        field.players.iter().for_each(|player| {
            match_data.add_player_positions(player.id, timestamp, player.position);
            if track_events {
                match_data.add_player_state(player.id, timestamp, player.state.compact_id(), &player.state);
            }
        });

        match_data.add_ball_positions(timestamp, field.ball.position);
    }

    // ───────────────────────────────────────────────────────────────────────
    // Ball & player dispatchers
    // ───────────────────────────────────────────────────────────────────────

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
            .for_each(|player| player.update(context, tick_context, events));
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// Match events enum
// ───────────────────────────────────────────────────────────────────────────────

pub enum MatchEvent {
    MatchPlayed(u32, bool, u8),
    Goal(u32),
    Assist(u32),
    Injury(u32),
}

// ───────────────────────────────────────────────────────────────────────────────
// Types: BallSide, TeamsTactics, MatchFieldSize
// ───────────────────────────────────────────────────────────────────────────────

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

// ───────────────────────────────────────────────────────────────────────────────
// Types: PlayerEntry, MatchPlayerCollection
// ───────────────────────────────────────────────────────────────────────────────

/// Compact player entry for fast iteration in hot loops
#[derive(Clone, Copy)]
pub struct PlayerEntry {
    pub id: u32,
    pub team_id: u32,
    pub position: PlayerPositionType,
}

pub struct MatchPlayerCollection {
    players: HashMap<u32, MatchPlayer>,
    /// Compact index for fast cache-friendly iteration
    pub entries: Vec<PlayerEntry>,
}

impl MatchPlayerCollection {
    pub fn from_squads(home_squad: &MatchSquad, away_squad: &MatchSquad) -> Self {
        let mut players = HashMap::new();
        let mut entries = Vec::with_capacity(44);

        let add = |p: &MatchPlayer, map: &mut HashMap<u32, MatchPlayer>, entries: &mut Vec<PlayerEntry>| {
            entries.push(PlayerEntry {
                id: p.id,
                team_id: p.team_id,
                position: p.tactical_position.current_position,
            });
            map.insert(p.id, p.clone());
        };

        for p in &home_squad.main_squad { add(p, &mut players, &mut entries); }
        for p in &home_squad.substitutes { add(p, &mut players, &mut entries); }
        for p in &away_squad.main_squad { add(p, &mut players, &mut entries); }
        for p in &away_squad.substitutes { add(p, &mut players, &mut entries); }

        MatchPlayerCollection { players, entries }
    }

    pub fn by_id(&self, player_id: u32) -> Option<&MatchPlayer> {
        self.players.get(&player_id)
    }

    pub fn raw_players(&self) -> impl Iterator<Item = &MatchPlayer> {
        self.players.values()
    }

    pub fn remove_player(&mut self, player_id: u32) {
        self.players.remove(&player_id);
        self.entries.retain(|e| e.id != player_id);
    }

    pub fn update_player(&mut self, player_id: u32, player: MatchPlayer) {
        let pos = player.tactical_position.current_position;
        let team_id = player.team_id;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == player_id) {
            entry.position = pos;
            entry.team_id = team_id;
        } else {
            self.entries.push(PlayerEntry { id: player_id, team_id, position: pos });
        }
        self.players.insert(player_id, player);
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// Types: MatchTime, PlayMatchStateResult
// ───────────────────────────────────────────────────────────────────────────────

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

#[derive(Default, Clone)]
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
