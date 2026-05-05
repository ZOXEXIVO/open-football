use crate::r#match::PlayerMatchEndStats;
use crate::r#match::PlayerSide;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::context::SubstitutionRecord;
use crate::r#match::engine::events::dispatcher::EventCollection;
use crate::r#match::engine::goal::{assign_kickoff, handle_goal_reset};
#[cfg(feature = "match-logs")]
use crate::r#match::engine::player::events::players::save_accounting_stats;
use crate::r#match::engine::rating::calculate_match_rating;
use crate::r#match::engine::substitutions::process_substitutions;
use crate::r#match::events::EventDispatcher;
use crate::r#match::field::MatchField;
#[cfg(feature = "match-logs")]
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::result::ResultMatchPositionData;
use crate::r#match::{
    GameTickContext, MatchContext, MatchPlayer, MatchResultRaw, MatchSquad, MatchState,
    PenaltyShootoutKick, Score, StateManager, SubstitutionInfo,
};
use crate::{PlayerFieldPositionGroup, PlayerPositionType, Tactics, is_match_events_mode};
#[cfg(feature = "match-logs")]
use crate::{match_log_debug, match_log_info};
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

    pub fn play(
        left_squad: MatchSquad,
        right_squad: MatchSquad,
        match_recordings: bool,
        is_friendly: bool,
        is_knockout: bool,
    ) -> MatchResultRaw {
        let score = Score::new(left_squad.team_id, right_squad.team_id);

        let players = MatchPlayerCollection::from_squads(&left_squad, &right_squad);

        let mut match_position_data = if !match_recordings {
            ResultMatchPositionData::empty()
        } else if is_match_events_mode() {
            ResultMatchPositionData::new_with_tracking()
        } else {
            ResultMatchPositionData::new()
        };

        let mut field = MatchField::new(W, H, left_squad, right_squad);

        let mut context = MatchContext::new(&field, players, score, is_friendly, is_knockout);

        if is_match_events_mode() {
            context.enable_logging();
        }

        let mut state_manager = StateManager::new();

        // Match kickoff — home team (playing Left in the first half)
        // starts the game with possession on the centre spot. Without
        // this the ball sits at centre until the emergency chaser
        // override fires, producing a ~14-second dead patch.
        assign_kickoff(&mut field, PlayerSide::Left);

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

        result.additional_time_ms = context.additional_time_ms;
        result.penalty_shootout = context.penalty_shootout_kicks.clone();
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
                result
                    .left_team_players
                    .mark_substitute_used(sub_record.player_in_id);
            } else {
                result
                    .right_team_players
                    .mark_substitute_used(sub_record.player_in_id);
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
                shots_faced: player.statistics.shots_faced,
                goals,
                assists,
                match_rating: 0.0,
                xg: player.memory.xg_total,
                position_group,
                fouls,
                yellow_cards,
                red_cards,
                minutes_played: ((context.total_match_time / 60_000) as u16).min(120),
                key_passes: player.statistics.key_passes,
                progressive_passes: player.statistics.progressive_passes,
                progressive_carries: player.statistics.progressive_carries,
                successful_dribbles: player.statistics.successful_dribbles,
                attempted_dribbles: player.statistics.attempted_dribbles,
                successful_pressures: player.statistics.successful_pressures,
                pressures: player.statistics.pressures,
                blocks: player.statistics.blocks,
                clearances: player.statistics.clearances,
                passes_into_box: player.statistics.passes_into_box,
                crosses_attempted: player.statistics.crosses_attempted,
                crosses_completed: player.statistics.crosses_completed,
                xg_chain: player.statistics.xg_chain,
                xg_buildup: player.statistics.xg_buildup,
                miscontrols: player.statistics.miscontrols,
                heavy_touches: player.statistics.heavy_touches,
                carry_distance: player.statistics.carry_distance,
                errors_leading_to_shot: player.statistics.errors_leading_to_shot,
                errors_leading_to_goal: player.statistics.errors_leading_to_goal,
                xg_prevented: player.statistics.xg_prevented,
            };
            stats.match_rating = calculate_match_rating(&stats, player_team_goals, opponent_goals);

            result.player_stats.insert(player.id, stats);
        }

        // Include stats from substituted-out players
        for (player_id, mut stats) in context.substituted_out_stats.drain(..) {
            let team_id = context
                .substitutions
                .iter()
                .find(|s| s.player_out_id == player_id)
                .map(|s| s.team_id);

            let (player_team_goals, opponent_goals) = if team_id == Some(home_team_id) {
                (home_goals, away_goals)
            } else {
                (away_goals, home_goals)
            };

            stats.match_rating = calculate_match_rating(&stats, player_team_goals, opponent_goals);

            result.player_stats.insert(player_id, stats);
        }

        // Blowout diagnostic — off by default (see `match-logs` feature).
        // The aggregation itself is O(players × stats), so we skip the
        // whole block in production. Enable with `--features match-logs`
        // in `.dev/match` to analyse shot / goal sources.
        #[cfg(feature = "match-logs")]
        {
            let total_goals = home_goals + away_goals;
            if total_goals >= 8 {
                let away_team_id = if field.home_team_id == home_team_id {
                    field.away_team_id
                } else {
                    field.home_team_id
                };
                Self::log_blowout_profile(
                    &field.players,
                    &context.substitutions,
                    &result,
                    home_team_id,
                    away_team_id,
                    home_goals,
                    away_goals,
                );
            }
        }

        result
    }

    /// Aggregate per-team skill dump for post-match analysis. Runs only
    /// on 8+ goal matches when the `match-logs` feature is enabled.
    /// Skipped entirely in production builds.
    #[cfg(feature = "match-logs")]
    fn log_blowout_profile(
        players: &[MatchPlayer],
        substitutions: &[SubstitutionRecord],
        result: &MatchResultRaw,
        home_team_id: u32,
        away_team_id: u32,
        home_goals: u8,
        away_goals: u8,
    ) {
        struct TeamAgg {
            fwd_finishing: f32,
            fwd_technique: f32,
            fwd_composure: f32,
            fwd_count: u32,
            def_marking: f32,
            def_tackling: f32,
            def_positioning: f32,
            def_count: u32,
            gk_handling: f32,
            gk_reflexes: f32,
            gk_agility: f32,
            gk_count: u32,
            total_shots: u16,
            total_on_target: u16,
            passes_attempted: u32,
            passes_completed: u32,
            tackles: u32,
            interceptions: u32,
            saves: u32,
            fouls: u32,
            xg_total: f32,
        }
        impl TeamAgg {
            fn new() -> Self {
                Self {
                    fwd_finishing: 0.0,
                    fwd_technique: 0.0,
                    fwd_composure: 0.0,
                    fwd_count: 0,
                    def_marking: 0.0,
                    def_tackling: 0.0,
                    def_positioning: 0.0,
                    def_count: 0,
                    gk_handling: 0.0,
                    gk_reflexes: 0.0,
                    gk_agility: 0.0,
                    gk_count: 0,
                    total_shots: 0,
                    total_on_target: 0,
                    passes_attempted: 0,
                    passes_completed: 0,
                    tackles: 0,
                    interceptions: 0,
                    saves: 0,
                    fouls: 0,
                    xg_total: 0.0,
                }
            }
        }

        let mut home_agg = TeamAgg::new();
        let mut away_agg = TeamAgg::new();

        // Skill profile aggregation runs over the current XI only — sub-out
        // players' skills aren't retained on the field after they leave.
        for player in players {
            let agg = if player.team_id == home_team_id {
                &mut home_agg
            } else {
                &mut away_agg
            };
            match player.tactical_position.current_position.position_group() {
                PlayerFieldPositionGroup::Forward => {
                    agg.fwd_finishing += player.skills.technical.finishing;
                    agg.fwd_technique += player.skills.technical.technique;
                    agg.fwd_composure += player.skills.mental.composure;
                    agg.fwd_count += 1;
                }
                PlayerFieldPositionGroup::Defender => {
                    agg.def_marking += player.skills.technical.marking;
                    agg.def_tackling += player.skills.technical.tackling;
                    agg.def_positioning += player.skills.mental.positioning;
                    agg.def_count += 1;
                }
                PlayerFieldPositionGroup::Goalkeeper => {
                    agg.gk_handling += player.skills.goalkeeping.handling;
                    agg.gk_reflexes += player.skills.goalkeeping.reflexes;
                    agg.gk_agility += player.skills.physical.agility;
                    agg.gk_count += 1;
                }
                _ => {}
            }
        }

        // Shot/OT aggregation must include sub-out players: their goals
        // count toward the team total, so their attempts must too.
        // Otherwise the log shows "7 goals from 2 OT" simply because the
        // hat-trick scorer was substituted off and their stats fell out
        // of the field.players iteration.
        let team_for = |player_id: u32| -> Option<u32> {
            players
                .iter()
                .find(|p| p.id == player_id)
                .map(|p| p.team_id)
                .or_else(|| {
                    substitutions
                        .iter()
                        .find(|s| s.player_out_id == player_id)
                        .map(|s| s.team_id)
                })
        };
        for (player_id, stats) in result.player_stats.iter() {
            let Some(team_id) = team_for(*player_id) else {
                continue;
            };
            let agg = if team_id == home_team_id {
                &mut home_agg
            } else {
                &mut away_agg
            };
            agg.total_shots += stats.shots_total;
            agg.total_on_target += stats.shots_on_target;
            agg.passes_attempted += stats.passes_attempted as u32;
            agg.passes_completed += stats.passes_completed as u32;
            agg.tackles += stats.tackles as u32;
            agg.interceptions += stats.interceptions as u32;
            agg.saves += stats.saves as u32;
            agg.fouls += stats.fouls as u32;
            agg.xg_total += stats.xg;
        }

        // Goal source breakdown — distinguishes "proper" shot goals from
        // own goals and from goals credited to a scorer who never actually
        // took a shot in the match (so the goal came from a pass, a
        // clearance-into-net, or a loose-ball scramble that the engine
        // credits via the `current_owner.or(previous_owner)` fallback).
        // The last bucket is the one we need to watch — goals should flow
        // almost exclusively from shots.
        let score_ref = result.score.as_ref().unwrap();
        let mut home_own: u16 = 0;
        let mut away_own: u16 = 0;
        let mut home_nonshot: u16 = 0;
        let mut away_nonshot: u16 = 0;
        for g in score_ref.detail() {
            if g.stat_type != MatchStatisticType::Goal {
                continue;
            }
            let scorer_team = team_for(g.player_id);
            let is_home_scorer = scorer_team == Some(home_team_id);
            if g.is_auto_goal {
                // Own goal — credited to opponent team
                if is_home_scorer {
                    away_own += 1;
                } else {
                    home_own += 1;
                }
                continue;
            }
            // Non-auto goal. Did this scorer take ANY shot in the match?
            let took_shot = result
                .player_stats
                .get(&g.player_id)
                .map(|s| s.shots_total > 0)
                .unwrap_or(false);
            if !took_shot {
                if is_home_scorer {
                    home_nonshot += 1;
                } else {
                    away_nonshot += 1;
                }
            }
        }

        let total_passes = (home_agg.passes_attempted + away_agg.passes_attempted).max(1) as f32;
        let home_possession = home_agg.passes_attempted as f32 / total_passes * 100.0;
        let away_possession = away_agg.passes_attempted as f32 / total_passes * 100.0;

        let fmt_team = |tag: &str,
                        team_id: u32,
                        goals: u8,
                        agg: &TeamAgg,
                        own_against: u16,
                        nonshot: u16,
                        possession: f32| {
            let fc = agg.fwd_count.max(1) as f32;
            let dc = agg.def_count.max(1) as f32;
            let gc = agg.gk_count.max(1) as f32;
            let pass_acc = if agg.passes_attempted > 0 {
                agg.passes_completed as f32 / agg.passes_attempted as f32 * 100.0
            } else {
                0.0
            };
            // xG overperformance: goals above what the shot quality
            // predicted. Real football: |diff| rarely exceeds xG by more
            // than ~1.5. Big positive means clinical finishing or a
            // generous finishing roll; big negative means wasted chances.
            let xg_delta = goals as f32 - agg.xg_total;
            // Shot volume per xG: a team that took 150 shots for only 4.5
            // xG was firing from impossible angles / long range — a
            // marker of blind shooting during desperation mode.
            let shots_per_xg = if agg.xg_total > 0.01 {
                agg.total_shots as f32 / agg.xg_total
            } else {
                0.0
            };
            format!(
                "{} team={} gls={} (own-ag={} non-shot={}) shots={} ot={} xG={:.2} (Δ{:+.2}, s/xG={:.0}) | poss={:.0}% pass={}/{} ({:.0}%) tck={} int={} sv={} fl={} | FWD fin={:.1} tec={:.1} com={:.1} | DEF mrk={:.1} tck={:.1} pos={:.1} | GK hnd={:.1} ref={:.1} agi={:.1}",
                tag,
                team_id,
                goals,
                own_against,
                nonshot,
                agg.total_shots,
                agg.total_on_target,
                agg.xg_total,
                xg_delta,
                shots_per_xg,
                possession,
                agg.passes_completed,
                agg.passes_attempted,
                pass_acc,
                agg.tackles,
                agg.interceptions,
                agg.saves,
                agg.fouls,
                agg.fwd_finishing / fc,
                agg.fwd_technique / fc,
                agg.fwd_composure / fc,
                agg.def_marking / dc,
                agg.def_tackling / dc,
                agg.def_positioning / dc,
                agg.gk_handling / gc,
                agg.gk_reflexes / gc,
                agg.gk_agility / gc,
            )
        };

        match_log_info!(
            "BLOWOUT {}-{} (total {}g)",
            home_goals,
            away_goals,
            home_goals + away_goals
        );
        // Notation:
        //   own-ag   own goals (our player into our own net)
        //   non-shot goals credited to a scorer who took zero shots — came
        //            via pass/scramble/deflection path
        //   xG       sum of expected-goals across all shots we took
        //   Δ        goals minus xG (overperformance if positive)
        //   s/xG     shots per xG — high = blind long-range spam
        //   poss     share of total pass attempts (possession proxy)
        //   pass     completed / attempted (acc %)
        //   tck/int/sv/fl  tackles / interceptions / saves / fouls
        match_log_info!(
            "  {}",
            fmt_team(
                "HOME",
                home_team_id,
                home_goals,
                &home_agg,
                home_own,
                home_nonshot,
                home_possession
            )
        );
        match_log_info!(
            "  {}",
            fmt_team(
                "AWAY",
                away_team_id,
                away_goals,
                &away_agg,
                away_own,
                away_nonshot,
                away_possession
            )
        );
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
        let mut tactical_eval_counter: u32 = 0;
        const TACTICAL_INTERVAL_TICKS: u32 = 10;

        while context.increment_time() {
            tick_parity += 1;
            coach_eval_counter += 1;
            tactical_eval_counter += 1;

            // Coach evaluates every 500 ticks (~5 seconds of match time)
            if coach_eval_counter >= 500 {
                coach_eval_counter = 0;
                Self::evaluate_coaches(field, context);
                // Once every coach-eval slice, also probe for situational
                // formation overrides — the manager swap to a chasing /
                // protecting shape based on score and minute. Cheap: a
                // single match arm and an equality check against the
                // current type per side.
                Self::evaluate_situational_shape(field, context);
            }

            // Team-level tactical state (phase, possession timers, line
            // height) refreshes every 10 ticks — too fast and we chase
            // flicker in the ball-owner signal; too slow and transition
            // windows (≤50 ticks) lose resolution.
            if tactical_eval_counter >= TACTICAL_INTERVAL_TICKS {
                let interval = tactical_eval_counter;
                tactical_eval_counter = 0;
                Self::refresh_tactical_states(field, context, interval);
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
                // Grant the ET bonus once — bumps the cap by 1 for both
                // sides — but only when the active rule set allows it.
                // Friendlies (cap = usize::MAX) skip the increment.
                if context.state.match_state == MatchState::ExtraTime
                    && !et_bonus_granted
                    && context.allow_extra_time_extra_sub
                {
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
                    // Wall-clock today — the engine doesn't track sim
                    // date directly. Used only for the youth-protection
                    // sub branch, where the comparison is age <= 17.
                    let today = chrono::Utc::now().naive_utc().date();
                    let per_pass_cap = context.max_substitutions_per_pass;
                    process_substitutions(field, context, per_pass_cap, today);
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

    /// In-match shape change. Probes `TacticsSelector::situational_shape`
    /// for both sides; if the helper returns a new formation different
    /// from the side's current `tactic_type`, the side's `Tactics`
    /// struct is rebuilt around the new shape. The team-tactical refresh
    /// pipeline picks the new shape up on its next pass — pressing /
    /// line height / mentality already key off `tactical_style()`.
    fn evaluate_situational_shape(field: &mut MatchField, context: &MatchContext) {
        use crate::club::team::tactics::tactics::TacticsSelector;
        let minutes = (context.total_match_time / 60_000).min(120) as u8;
        let home_diff =
            (context.score.home_team.get() as i16 - context.score.away_team.get() as i16)
                .clamp(-100, 100) as i8;
        let away_diff = -home_diff;

        // Map left/right squad → home/away by checking which side the
        // home squad currently occupies. Sides swap at half-time so we
        // can't hardcode left=home.
        let home_is_left = field
            .left_side_players
            .as_ref()
            .map(|s| s.team_id == context.field_home_team_id)
            .unwrap_or(true);

        let probe = |side_tactics: &mut crate::Tactics, is_home: bool, score_diff: i8| {
            if let Some(new_shape) = TacticsSelector::situational_shape(
                side_tactics.tactic_type,
                is_home,
                score_diff,
                minutes,
            ) {
                *side_tactics = crate::Tactics::with_reason(
                    new_shape,
                    crate::TacticSelectionReason::GameSituation,
                    side_tactics.formation_strength,
                );
            }
        };

        if home_is_left {
            probe(&mut field.left_team_tactics, true, home_diff);
            probe(&mut field.right_team_tactics, false, away_diff);
        } else {
            probe(&mut field.right_team_tactics, true, home_diff);
            probe(&mut field.left_team_tactics, false, away_diff);
        }
    }

    fn evaluate_coaches(field: &MatchField, context: &mut MatchContext) {
        let home_goals = context.score.home_team.get() as i8;
        let away_goals = context.score.away_team.get() as i8;
        let current_tick = context.current_tick();

        // Regulation progress capped at 1.0. In extra time `total_match_time`
        // keeps climbing past 90 min; without the clamp `is_late_game` and
        // `is_very_late` stay true but `is_first_half_end` (0.45..0.55) goes
        // stale and the `match` branches misbehave for losing teams.
        let match_progress = (context.total_match_time as f32 / MATCH_TIME_MS as f32).min(1.0);

        let (home_condition_sum, home_count, away_condition_sum, away_count) = field
            .players
            .iter()
            .fold((0.0f32, 0u32, 0.0f32, 0u32), |(hc, hn, ac, an), player| {
                let cond = player.player_attributes.condition as f32 / 10000.0;
                if player.team_id == context.field_home_team_id {
                    (hc + cond, hn + 1, ac, an)
                } else {
                    (hc, hn, ac + cond, an + 1)
                }
            });

        let home_avg_condition = if home_count > 0 {
            home_condition_sum / home_count as f32
        } else {
            0.5
        };
        let away_avg_condition = if away_count > 0 {
            away_condition_sum / away_count as f32
        } else {
            0.5
        };

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

    /// Refresh the team-level tactical state (phase, possession timers,
    /// defensive-line height) for both sides. Reads only the ball and
    /// players; mutates the `tactical_home` / `tactical_away` fields on
    /// `MatchContext`. `tick_interval` is how many ticks elapsed since
    /// the last refresh — rolling counters scale with it.
    fn refresh_tactical_states(field: &MatchField, context: &mut MatchContext, tick_interval: u32) {
        use crate::r#match::{CoachInstruction, TacticalRefreshInputs, TeamTacticalState};
        let home_high_press = matches!(
            context.coach_home.instruction,
            CoachInstruction::PushForward | CoachInstruction::AllOutAttack
        );
        let away_high_press = matches!(
            context.coach_away.instruction,
            CoachInstruction::PushForward | CoachInstruction::AllOutAttack
        );

        // One pass over players collects ability + condition aggregates
        // for both sides. Avoids walking the player list four times.
        let (home_ca_sum, home_count, home_cond_sum, away_ca_sum, away_count, away_cond_sum) =
            field.players.iter().fold(
                (0u32, 0u32, 0.0f32, 0u32, 0u32, 0.0f32),
                |(hca, hc, hcond, aca, ac, acond), p| {
                    let ca = p.player_attributes.current_ability as u32;
                    let cond = p.player_attributes.condition as f32 / 10000.0;
                    if p.team_id == context.field_home_team_id {
                        (hca + ca, hc + 1, hcond + cond, aca, ac, acond)
                    } else {
                        (hca, hc, hcond, aca + ca, ac + 1, acond + cond)
                    }
                },
            );
        let home_avg = if home_count > 0 {
            (home_ca_sum / home_count) as u16
        } else {
            0
        };
        let away_avg = if away_count > 0 {
            (away_ca_sum / away_count) as u16
        } else {
            0
        };
        let home_avg_cond = if home_count > 0 {
            home_cond_sum / home_count as f32
        } else {
            0.5
        };
        let away_avg_cond = if away_count > 0 {
            away_cond_sum / away_count as f32
        } else {
            0.5
        };

        let home_goals = context.score.home_team.get() as i16;
        let away_goals = context.score.away_team.get() as i16;
        let home_score_diff = (home_goals - away_goals).clamp(-100, 100) as i8;

        // Tactics are stored on the field side-keyed (left/right). Map
        // them to home/away by checking which side the home squad
        // currently occupies — sides swap at half-time.
        let home_is_left = field
            .left_side_players
            .as_ref()
            .map(|s| s.team_id == context.field_home_team_id)
            .unwrap_or(true);
        let (home_tactics, away_tactics) = if home_is_left {
            (&field.left_team_tactics, &field.right_team_tactics)
        } else {
            (&field.right_team_tactics, &field.left_team_tactics)
        };

        let inputs = TacticalRefreshInputs {
            field,
            home_team_id: context.field_home_team_id,
            tick_interval,
            coach_wants_high_press_home: home_high_press,
            coach_wants_high_press_away: away_high_press,
            home_score_diff,
            match_time_ms: context.total_match_time,
            home_avg_ability: home_avg,
            away_avg_ability: away_avg,
            home_avg_condition: home_avg_cond,
            away_avg_condition: away_avg_cond,
            home_tactics,
            away_tactics,
        };
        TeamTacticalState::refresh(
            &mut context.tactical_home,
            &mut context.tactical_away,
            &inputs,
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
        Self::apply_pending_set_piece_teleport(field);
        Self::apply_pending_save_credit(field);

        // Shot-flight GK reactivity: normally light ticks skip player
        // AI to save CPU, but during a shot the keeper needs continuous
        // decisions to close on the intercept line. Run just the two
        // goalkeepers (cheap, ~2 of 22 players) when a shot is in flight.
        if field.ball.cached_shot_target.is_some() {
            let mut tick_ctx = GameTickContext::new(field);
            tick_ctx.update(field);
            Self::play_goalkeepers(field, context, &tick_ctx, events);
        }

        // Skip sent-off players: they've been stashed at (-500, -500). A
        // boundary clamp here would drag them to (0, 0) — the pitch's
        // top-left corner — which then gets recorded as a ghost sample
        // by `write_match_positions`.
        for player in field.players.iter_mut().filter(|p| !p.is_sent_off) {
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
        Self::apply_pending_set_piece_teleport(field);
        Self::apply_pending_save_credit(field);
        // Ownership may have changed inside play_ball (new claim, pass
        // target receive, etc.). Refresh the ball view so player state
        // dispatch sees the current owner — without this, the
        // TakeBall force-override fires for a player who already has
        // the ball.
        tick_ctx.refresh_ball(field);
        Self::play_players(field, context, tick_ctx, events);

        EventDispatcher::dispatch(events, field, context, match_data, true);

        handle_goal_reset(field, context);

        Self::write_match_positions(field, context.total_match_time, match_data);
    }

    /// Corner kicks and goal kicks rewrite ball ownership inside `ball.update`,
    /// but ball.rs only has `&[MatchPlayer]` — it can't teleport the designated
    /// taker to the ball. Instead it stashes the teleport intent on the Ball;
    /// we drain it here, now that we have `&mut field.players`. Without this,
    /// the ball sits at the corner flag / goal area with ownership assigned
    /// to a player 30-200 units away, and `move_to`'s 15-unit distance check
    /// nulls ownership on the very next tick — ball stalls for seconds.
    fn apply_pending_set_piece_teleport(field: &mut MatchField) {
        if let Some((player_id, ball_pos)) = field.ball.pending_set_piece_teleport.take() {
            if let Some(p) = field.players.iter_mut().find(|p| p.id == player_id) {
                p.position = ball_pos;
                p.velocity = nalgebra::Vector3::zeros();
                p.in_state_time = 0;
            }
        }
    }

    /// Consume `Ball::pending_save_credit` left behind by the physics
    /// save (`try_save_shot`). When the keeper actually changed ball
    /// state mid-flight (catch, safe parry, dangerous parry), this fires
    /// the save stat for the keeper and the on-target stat for the
    /// shooter — matching the events the GK state machine would have
    /// emitted if the physics save hadn't pre-empted it.
    fn apply_pending_save_credit(field: &mut MatchField) {
        let Some((keeper_id, shooter_id)) = field.ball.pending_save_credit.take() else {
            return;
        };
        // Validate teams differ — defence in depth against any
        // accidental same-team shooter (deflections that route through
        // the save handler should already be filtered upstream).
        let keeper_team = field
            .players
            .iter()
            .find(|p| p.id == keeper_id)
            .map(|p| p.team_id);
        let shooter_team = field
            .players
            .iter()
            .find(|p| p.id == shooter_id)
            .map(|p| p.team_id);
        if keeper_team.is_none() || shooter_team.is_none() || keeper_team == shooter_team {
            return;
        }
        if let Some(gk) = field.players.iter_mut().find(|p| p.id == keeper_id) {
            gk.statistics.saves += 1;
            gk.statistics.shots_faced += 1;
        }
        if let Some(shooter) = field.players.iter_mut().find(|p| p.id == shooter_id) {
            shooter.memory.credit_shot_on_target();
        }
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            // Re-use the "catch" site bucket — physics-save outcomes are
            // catches, parries, and dangerous parries indistinguishably
            // from the stats viewpoint. The save_pipeline counters above
            // already separate them at the physics layer.
            save_accounting_stats::SAVES_CREDITED[1].fetch_add(1, Ordering::Relaxed);
            save_accounting_stats::ON_TARGET_PAIRED[1].fetch_add(1, Ordering::Relaxed);
        }
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

        // Don't record sent-off players — their state doesn't advance and
        // their position is a dummy off-pitch stash. A recorded sample
        // would show them as a ghost sprite in the replay viewer.
        field.players.iter().filter(|p| !p.is_sent_off).for_each(|player| {
            // Diagnostic: catch players pinned at ANY field boundary.
            // `check_boundary_collision` clamps to 0..=field_width and
            // 0..=field_height; a steering error that consistently
            // points off-pitch leaves the player stuck there.
            // Rate-limit to once per 30s of match time per player so a
            // persistent stall doesn't spam the log every 30ms sample.
            let field_w = field.size.width as f32;
            let field_h = field.size.height as f32;
            let near_left = player.position.x < 1.0;
            let near_right = player.position.x > field_w - 1.0;
            let near_top = player.position.y < 1.0;
            let near_bottom = player.position.y > field_h - 1.0;
            if (near_left || near_right) && (near_top || near_bottom)
                && timestamp % 30_000 < Self::POSITION_RECORD_INTERVAL_MS
            {
                match_log_debug!(
                    "player at corner: t={}ms id={} team={} state={:?} tactical={:?} pos=({:.1}, {:.1}) velocity=({:.2}, {:.2})",
                    timestamp,
                    player.id,
                    player.team_id,
                    player.state,
                    player.tactical_position.current_position,
                    player.position.x,
                    player.position.y,
                    player.velocity.x,
                    player.velocity.y,
                );
            }
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

    /// Run the AI for *only* the goalkeepers this tick. Used during
    /// shot flight on light ticks: the 50% AI cadence across all 22
    /// players was fine for normal play, but during a ~10 tick shot
    /// window the GK missed half their decisions and never closed on
    /// the intercept. This fills in those ticks without re-evaluating
    /// 20 outfielders for zero behavioural gain.
    fn play_goalkeepers(
        field: &mut MatchField,
        context: &mut MatchContext,
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        field
            .players
            .iter_mut()
            .filter(|player| !player.is_sent_off)
            .filter(|player| {
                player.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
            })
            .for_each(|player| player.update(context, tick_context, events));
    }

    // ───────────────────────────────────────────────────────────────────────
    // Penalty shootout — discrete resolver, not tick-based
    // ───────────────────────────────────────────────────────────────────────

    fn run_penalty_shootout(field: &mut MatchField, context: &mut MatchContext) {
        use rand::RngExt;

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
                    let score = t.penalty_taking * 0.45
                        + t.finishing * 0.25
                        + t.technique * 0.15
                        + m.composure * 0.15;
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
                ((t.penalty_taking * 0.40
                    + t.finishing * 0.20
                    + t.technique * 0.10
                    + m.composure * 0.20
                    + pressure * 0.10)
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
                        ((g.handling * 0.20
                            + g.one_on_ones * 0.30
                            + g.reflexes * 0.30
                            + m.concentration * 0.10
                            + m.composure * 0.10)
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
            rng.random::<f32>() < goal_prob
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
                let scored = take_kick(id, away_keeper);
                context
                    .penalty_shootout_kicks
                    .push(PenaltyShootoutKick {
                        team_id: home_id,
                        taker_id: id,
                        goalkeeper_id: away_keeper,
                        round: round + 1,
                        scored,
                        sudden_death: false,
                    });
                if scored {
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
                let scored = take_kick(id, home_keeper);
                context
                    .penalty_shootout_kicks
                    .push(PenaltyShootoutKick {
                        team_id: away_id,
                        taker_id: id,
                        goalkeeper_id: home_keeper,
                        round: round + 1,
                        scored,
                        sudden_death: false,
                    });
                if scored {
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
            let home_taker = h.unwrap();
            let away_taker = a.unwrap();
            let round = 5 + sudden_rounds;
            let home_scored = take_kick(home_taker, away_keeper);
            context
                .penalty_shootout_kicks
                .push(PenaltyShootoutKick {
                    team_id: home_id,
                    taker_id: home_taker,
                    goalkeeper_id: away_keeper,
                    round,
                    scored: home_scored,
                    sudden_death: true,
                });
            if home_scored {
                home_score += 1;
            }
            let away_scored = take_kick(away_taker, home_keeper);
            context
                .penalty_shootout_kicks
                .push(PenaltyShootoutKick {
                    team_id: away_id,
                    taker_id: away_taker,
                    goalkeeper_id: home_keeper,
                    round,
                    scored: away_scored,
                    sudden_death: true,
                });
            if away_scored {
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

        let add = |p: &MatchPlayer,
                   map: &mut HashMap<u32, MatchPlayer>,
                   entries: &mut Vec<PlayerEntry>| {
            entries.push(PlayerEntry {
                id: p.id,
                team_id: p.team_id,
                position: p.tactical_position.current_position,
            });
            map.insert(p.id, p.clone());
        };

        for p in &home_squad.main_squad {
            add(p, &mut players, &mut entries);
        }
        for p in &away_squad.main_squad {
            add(p, &mut players, &mut entries);
        }

        let add_lookup_only = |p: &MatchPlayer, map: &mut HashMap<u32, MatchPlayer>| {
            map.insert(p.id, p.clone());
        };
        for p in &home_squad.substitutes {
            add_lookup_only(p, &mut players);
        }
        for p in &away_squad.substitutes {
            add_lookup_only(p, &mut players);
        }

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
            self.entries.push(PlayerEntry {
                id: player_id,
                team_id,
                position: pos,
            });
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
