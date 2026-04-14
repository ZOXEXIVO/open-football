use crate::r#match::engine::events::dispatcher::EventCollection;
use crate::r#match::engine::goal::handle_goal_reset;
use crate::r#match::engine::rating::calculate_match_rating;
use crate::r#match::engine::substitutions::process_substitutions;
use crate::r#match::events::EventDispatcher;
use crate::r#match::field::MatchField;
use crate::r#match::result::ResultMatchPositionData;
use crate::r#match::PlayerMatchEndStats;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer, MatchResultRaw, MatchSquad, MatchState, Score, StateManager, SubstitutionInfo};
use crate::{PlayerPositionType, Tactics};
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

    pub fn play(left_squad: MatchSquad, right_squad: MatchSquad, match_recordings: bool, is_friendly: bool, is_knockout: bool) -> MatchResultRaw {
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

        let mut context = MatchContext::new(&field, players, score, is_friendly, is_knockout);

        if crate::is_match_events_mode() {
            context.enable_logging();
        }

        let mut state_manager = StateManager::new();

        while let Some(state) = state_manager.next(&context.score, context.is_knockout) {
            context.state.set(state);

            let play_state_result = match state {
                MatchState::PenaltyShootout => {
                    Self::run_penalty_shootout(&mut field, &mut context);
                    PlayMatchStateResult::default()
                }
                _ => Self::play_inner(&mut field, &mut context, &mut match_position_data),
            };

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
            let position_group = player.tactical_position.current_position.position_group();

            let (player_team_goals, opponent_goals) = if player.team_id == home_team_id {
                (home_goals, away_goals)
            } else {
                (away_goals, home_goals)
            };

            let yellow_cards = player.statistics.yellow_cards_count();
            let red_cards = player.statistics.red_cards_count();
            let fouls = player.fouls_committed as u16;
            let mut stats = PlayerMatchEndStats {
                shots_on_target: player.memory.shots_on_target as u16,
                shots_total: player.memory.shots_taken as u16,
                passes_attempted: player.statistics.passes_attempted,
                passes_completed: player.statistics.passes_completed,
                tackles: player.statistics.tackles,
                interceptions: player.statistics.interceptions,
                saves: player.statistics.saves,
                goals,
                assists,
                match_rating: 0.0,
                xg: player.memory.xg_total,
                position_group,
                fouls,
                yellow_cards,
                red_cards,
            };
            stats.match_rating = calculate_match_rating(
                &stats, player_team_goals, opponent_goals,
            );

            result.player_stats.insert(player.id, stats);
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
                &stats, player_team_goals, opponent_goals,
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
        let mut et_bonus_granted = false;

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

            // Substitutions allowed from the second half onwards, plus
            // extra time when we reach it in a knockout tie. First-half
            // subs in real football are reactive (injuries) — we defer
            // that to the injury/fitness pipeline rather than speculating
            // here. ET gets one bonus sub on entry (FIFA rule).
            let subs_enabled = matches!(
                context.state.match_state,
                MatchState::SecondHalf | MatchState::ExtraTime
            );

            if subs_enabled {
                // Grant the ET bonus once — bumps the cap from 5 → 6 for both sides.
                if context.state.match_state == MatchState::ExtraTime && !et_bonus_granted {
                    if context.max_substitutions_per_team < usize::MAX {
                        context.max_substitutions_per_team += 1;
                    }
                    et_bonus_granted = true;
                    // Reset the next-sub timer for the new period.
                    sub_times_initialized = false;
                }

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

        // Regulation progress capped at 1.0. In extra time `total_match_time`
        // keeps climbing past 90 min; without the clamp `is_late_game` and
        // `is_very_late` stay true but `is_first_half_end` (0.45..0.55) goes
        // stale and the `match` branches misbehave for losing teams.
        let match_progress = (context.total_match_time as f32 / MATCH_TIME_MS as f32).min(1.0);

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
            .filter(|player| !player.is_sent_off)
            .for_each(|player| player.update(context, tick_context, events));
    }

    // ───────────────────────────────────────────────────────────────────────
    // Penalty shootout — discrete resolver, not tick-based
    // ───────────────────────────────────────────────────────────────────────

    fn run_penalty_shootout(field: &mut MatchField, context: &mut MatchContext) {
        use rand::RngExt;
        use crate::PlayerFieldPositionGroup;

        let mut rng = rand::rng();
        let home_id = context.field_home_team_id;
        let away_id = context.field_away_team_id;

        // Sort available outfield takers by penalty skill + composure.
        // Sent-off players (and the keeper) can't take kicks.
        let takers_for = |team_id: u32| -> Vec<u32> {
            let mut candidates: Vec<(u32, f32)> = field
                .players
                .iter()
                .filter(|p| p.team_id == team_id && !p.is_sent_off)
                .filter(|p| {
                    p.tactical_position.current_position.position_group()
                        != PlayerFieldPositionGroup::Goalkeeper
                })
                .map(|p| {
                    let t = &p.skills.technical;
                    let m = &p.skills.mental;
                    let score =
                        t.penalty_taking * 0.45 + t.finishing * 0.25 + t.technique * 0.15 + m.composure * 0.15;
                    (p.id, score)
                })
                .collect();
            candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            candidates.into_iter().take(11).map(|(id, _)| id).collect()
        };

        // Active keeper per team — prefer the nominated GK. If sent off
        // without a replacement (used all subs), pick the outfielder with
        // the best innate goalkeeping ability. Real football: an outfield
        // player has to go in goal — their save probability is poor but
        // non-zero.
        let keeper_for = |team_id: u32| -> Option<u32> {
            // First: an actual goalkeeper still on the field.
            let gk = field.players.iter().find(|p| {
                p.team_id == team_id
                    && !p.is_sent_off
                    && p.tactical_position.current_position.position_group()
                        == PlayerFieldPositionGroup::Goalkeeper
            });
            if let Some(p) = gk {
                return Some(p.id);
            }
            // Fallback: outfielder with the best goalkeeping composite.
            // Most outfielders have near-zero reflexes/handling so this
            // typically yields a 5-15% save rate, not the ~0% that a
            // missing GK would imply.
            field
                .players
                .iter()
                .filter(|p| p.team_id == team_id && !p.is_sent_off)
                .max_by(|a, b| {
                    let sa = a.skills.goalkeeping.reflexes * 0.4
                        + a.skills.goalkeeping.handling * 0.3
                        + a.skills.goalkeeping.one_on_ones * 0.3;
                    let sb = b.skills.goalkeeping.reflexes * 0.4
                        + b.skills.goalkeeping.handling * 0.3
                        + b.skills.goalkeeping.one_on_ones * 0.3;
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|p| p.id)
        };

        let home_takers = takers_for(home_id);
        let away_takers = takers_for(away_id);
        let home_keeper = keeper_for(home_id);
        let away_keeper = keeper_for(away_id);

        // Pulls taker-side skill (0..1).
        let taker_prob_adj = |fld: &MatchField, id: u32| -> f32 {
            if let Some(p) = fld.players.iter().find(|p| p.id == id) {
                let t = &p.skills.technical;
                let m = &p.skills.mental;
                let pressure = p.attributes.pressure;
                ((t.penalty_taking * 0.40 + t.finishing * 0.20 + t.technique * 0.10
                    + m.composure * 0.20 + pressure * 0.10)
                    / 20.0)
                    .clamp(0.05, 1.0)
            } else {
                0.5
            }
        };
        // Pulls keeper-side save skill (0..1). None means no keeper → very low save chance.
        let gk_prob_adj = |fld: &MatchField, id: Option<u32>| -> f32 {
            match id {
                Some(gk_id) => {
                    if let Some(p) = fld.players.iter().find(|p| p.id == gk_id) {
                        let g = &p.skills.goalkeeping;
                        let m = &p.skills.mental;
                        ((g.handling * 0.20 + g.one_on_ones * 0.30 + g.reflexes * 0.30
                            + m.concentration * 0.10 + m.composure * 0.10)
                            / 20.0)
                            .clamp(0.05, 1.0)
                    } else {
                        0.5
                    }
                }
                None => 0.05,
            }
        };

        // Single kick: returns true if goal.
        let mut take_kick = |taker_id: u32, gk_id: Option<u32>| -> bool {
            let taker_q = taker_prob_adj(field, taker_id);
            let gk_q = gk_prob_adj(field, gk_id);
            // League average ≈ 0.76. Skill delta nudges 0.45..0.92.
            let goal_prob = (0.72 + (taker_q - gk_q) * 0.25).clamp(0.45, 0.92);
            let scored = rng.random::<f32>() < goal_prob;
            if scored {
                // Record the kick on the taker's stat sheet too.
                if let Some(p) = field.players.iter_mut().find(|p| p.id == taker_id) {
                    p.statistics.add_goal(context.total_match_time, false);
                }
            }
            scored
        };

        // Takers in rotation; sudden-death wraps the order.
        let mut home_idx: usize = 0;
        let mut away_idx: usize = 0;
        let next_home_taker = |idx: &mut usize| -> Option<u32> {
            if home_takers.is_empty() {
                return None;
            }
            let id = home_takers[*idx % home_takers.len()];
            *idx += 1;
            Some(id)
        };
        let next_away_taker = |idx: &mut usize| -> Option<u32> {
            if away_takers.is_empty() {
                return None;
            }
            let id = away_takers[*idx % away_takers.len()];
            *idx += 1;
            Some(id)
        };

        let mut home_score: u8 = 0;
        let mut away_score: u8 = 0;

        // Best-of-5 phase.
        for round in 0..5u8 {
            let home_remaining_kicks = 5 - round;
            let away_remaining_kicks = 5 - round;

            // Home kick.
            if let Some(id) = next_home_taker(&mut home_idx) {
                if take_kick(id, away_keeper) {
                    home_score += 1;
                }
            }
            // Early termination — if one side can no longer catch up, stop.
            if (home_score as i32 - away_score as i32).abs()
                > (home_remaining_kicks as i32 - 1).max(0) + away_remaining_kicks as i32
            {
                break;
            }

            // Away kick.
            if let Some(id) = next_away_taker(&mut away_idx) {
                if take_kick(id, home_keeper) {
                    away_score += 1;
                }
            }
            if (home_score as i32 - away_score as i32).abs()
                > (home_remaining_kicks as i32 - 1).max(0)
                    + (away_remaining_kicks as i32 - 1).max(0)
            {
                break;
            }
        }

        // Sudden death: one pair at a time until a decisive difference.
        // Hard cap at 30 rounds so we never loop indefinitely on bad data.
        let mut sudden_rounds = 0u8;
        while home_score == away_score && sudden_rounds < 30 {
            sudden_rounds += 1;
            let h = next_home_taker(&mut home_idx);
            let a = next_away_taker(&mut away_idx);
            if h.is_none() || a.is_none() {
                break; // Shouldn't happen — takers wrap — but guard anyway.
            }
            if take_kick(h.unwrap(), away_keeper) {
                home_score += 1;
            }
            if take_kick(a.unwrap(), home_keeper) {
                away_score += 1;
            }
        }

        context.score.home_shootout = home_score;
        context.score.away_shootout = away_score;
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

/// Extra time is a single continuous 30-minute period in this simulation.
/// Real football splits it into 2×15 with an interval; we skip the break
/// since there's no tactical depth to add between the two halves here.
#[cfg(debug_assertions)]
pub const MATCH_EXTRA_TIME_MS: u64 = 3 * 60 * 1000;
#[cfg(not(debug_assertions))]
pub const MATCH_EXTRA_TIME_MS: u64 = 30 * 60 * 1000;

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
