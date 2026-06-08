use super::*;
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::engine::rating::{RatingExpectationContext, TeamRatingSummary};

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    pub fn new() -> Self {
        FootballEngine {}
    }

    #[allow(unreachable_code)]
    pub fn play(
        left_squad: MatchSquad,
        right_squad: MatchSquad,
        match_recordings: bool,
        is_friendly: bool,
        is_knockout: bool,
    ) -> MatchResultRaw {
        let mut config = MatchEngineConfig::default();
        config.match_recordings = match_recordings;
        config.is_friendly = is_friendly;
        config.is_knockout = is_knockout;
        Self::play_with_config(left_squad, right_squad, config)
    }

    /// Seeded entry point. Compatibility wrapper around
    /// `play_with_config`. `seed = Some(_)` pins the engine's owned
    /// RNG (substitution timing, penalty shootout, foul card rolls,
    /// corner aerial contest, every converted player decision).
    /// `None` falls back to OS entropy, matching legacy behaviour.
    #[allow(unreachable_code)]
    pub fn play_seeded(
        left_squad: MatchSquad,
        right_squad: MatchSquad,
        match_recordings: bool,
        is_friendly: bool,
        is_knockout: bool,
        seed: Option<u64>,
    ) -> MatchResultRaw {
        let mut config = MatchEngineConfig::default();
        config.seed = seed;
        config.match_recordings = match_recordings;
        config.is_friendly = is_friendly;
        config.is_knockout = is_knockout;
        Self::play_with_config(left_squad, right_squad, config)
    }

    /// Full-config entry point. Lets the caller inject seed, fixture
    /// date, environment (weather/pitch/crowd/importance/derby),
    /// referee profile, friendly/knockout flags, and the
    /// match_recordings switch in one place — instead of patching the
    /// context after construction. Required by the calibration harness
    /// to run a real rainy match or a strict-referee fixture, and by
    /// any replay test that needs exact-seed control over today's
    /// date.
    #[allow(unreachable_code)]
    pub fn play_with_config(
        left_squad: MatchSquad,
        right_squad: MatchSquad,
        config: MatchEngineConfig,
    ) -> MatchResultRaw {
        // Profiling shortcut — see the `match-stub` feature in
        // `core/Cargo.toml`. Skips the simulation entirely and returns
        // a 0-0 result with just enough metadata (team IDs, player
        // IDs) for the surrounding pipeline to run.
        #[cfg(feature = "match-stub")]
        {
            let _ = &config;
            return Self::play_stub(left_squad, right_squad);
        }

        let perf = PerfCounters::instance();
        let match_start = Instant::now();
        let score = Score::new(left_squad.team_id, right_squad.team_id);

        // Snapshot starting tactics by team-id BEFORE the squads move
        // into `MatchField::new`. The first half always has the home
        // team on the left side, so left == home / right == away here.
        let starting_home_tactic = Some(left_squad.tactics.tactic_type);
        let starting_away_tactic = Some(right_squad.tactics.tactic_type);

        let players = MatchPlayerCollection::from_squads(&left_squad, &right_squad);

        let mut match_position_data = if !config.match_recordings {
            ResultMatchPositionData::empty()
        } else if MatchRuntime::events_mode() {
            ResultMatchPositionData::new_with_tracking()
        } else {
            ResultMatchPositionData::new()
        };

        let mut field = MatchField::new(W, H, left_squad, right_squad);

        let mut context = MatchContext::new_with_config(&field, players, score, &config);
        // Stash the starting tactics inside the context's match plan so
        // `build_result` can read them — no extra parameters threaded
        // through the state machine.
        context.starting_home_tactic = starting_home_tactic;
        context.starting_away_tactic = starting_away_tactic;

        // Seed the chemistry map from the kickoff XI of each side.
        // Pair scores stay constant for the match — live events could
        // adjust them, but the initial baseline is what feeds the pass
        // evaluator's one-touch bonus from the first whistle.
        let chemistry_roster: Vec<(u32, u32, PlayerFieldPositionGroup, f32, f32)> = field
            .players
            .iter()
            .map(|p| {
                (
                    p.id,
                    p.team_id,
                    p.tactical_position.current_position.position_group(),
                    p.position.y,
                    p.skills.mental.teamwork,
                )
            })
            .collect();
        let field_h = field.size.height as f32;
        context.chemistry.seed_from_roster(&chemistry_roster, field_h);

        if MatchRuntime::events_mode() {
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

        let result_start = Instant::now();
        let result = Self::build_result(field, context, match_position_data);
        perf.record_match_result_processing(result_start.elapsed());
        perf.record_match_total(match_start.elapsed());
        result
    }

    /// Stub match: skips the whole simulation and returns a 0-0
    /// scoreline with the minimum data downstream consumers expect
    /// (team IDs in `Score`, player IDs in the field squads). Gated
    /// on the `match-stub` Cargo feature; intended for profiling the
    /// pipeline around the engine.
    #[cfg(feature = "match-stub")]
    pub(super) fn play_stub(left_squad: MatchSquad, right_squad: MatchSquad) -> MatchResultRaw {
        use crate::r#match::engine::result::FieldSquad;

        let mut result = MatchResultRaw::with_match_time(90 * 60 * 1000);
        result.score = Some(Score::new(left_squad.team_id, right_squad.team_id));
        result.left_team_players = FieldSquad::from_team(&left_squad);
        result.right_team_players = FieldSquad::from_team(&right_squad);
        result
    }

    pub(super) fn build_result(
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

        // The engine swaps `left_team_tactics` / `right_team_tactics` on
        // every halftime swap, so at this point those fields track the
        // tactic the team currently ON the left / right is using —
        // including any mid-match shape change applied by
        // `evaluate_situational_shape`. Map them back to home / away
        // using the same team-id rule we use for player squads.
        let left_team_tac = field.left_team_tactics.tactic_type;
        let right_team_tac = field.right_team_tactics.tactic_type;

        if left_side_squad.team_id == field.home_team_id {
            result.left_team_players = left_side_squad;
            result.right_team_players = right_side_squad;
            result.final_home_tactic = Some(left_team_tac);
            result.final_away_tactic = Some(right_team_tac);
        } else {
            result.left_team_players = right_side_squad;
            result.right_team_players = left_side_squad;
            result.final_home_tactic = Some(right_team_tac);
            result.final_away_tactic = Some(left_team_tac);
        }
        result.starting_home_tactic = context.starting_home_tactic;
        result.starting_away_tactic = context.starting_away_tactic;
        result.shape_change_minute = context.first_shape_change_minute;

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
                reason: sub_record.reason,
            });
        }

        result.position_data = match_position_data;

        // Extract per-player stats and calculate match ratings.
        //
        // Two passes are needed because the contextual (public) rating
        // reads team-behaviour summaries folded from EVERY player's stat
        // line — who shot, who held the ball, who had to defend. So we
        // first materialise all the stat lines and physical snapshots
        // (subs included), then build the per-side summaries, then score
        // each player against their own / opponent summary plus their
        // own physical snapshot.
        let score_ref = result.score.as_ref().unwrap();
        let home_goals = score_ref.home_team.get();
        let away_goals = score_ref.away_team.get();
        let home_team_id = score_ref.home_team.team_id;

        // ── Pass 1: materialise stat lines + physical snapshots ──────────
        for player in &field.players {
            let minutes = player.minutes_played_at(context.total_match_time);
            let stats = player.to_match_end_stats(minutes);
            result.player_stats.insert(player.id, stats);

            // Final-whistle physical snapshot — every player still on the
            // pitch at full time. Captured here because `field.players`
            // is the canonical "who was on the pitch when the whistle
            // blew" view, and the engine's in-match condition drain
            // has been applied to each `MatchPlayer` by now. Players
            // who were substituted off are folded in below from
            // `context.substituted_out_physical_snapshots` so the same
            // player never gets two snapshots.
            let phys_snapshot = player.to_physical_snapshot(context.total_match_time);
            result.physical_snapshots.insert(player.id, phys_snapshot);
        }

        // Stat lines for substituted-out players (their snapshot was taken
        // at the swap minute; folded in just below). Ratings are filled
        // in Pass 2 alongside everyone else.
        for (player_id, stats) in context.substituted_out_stats.drain(..) {
            result.player_stats.insert(player_id, stats);
        }

        // Fold every "subbed-off" physical snapshot into the result. The
        // snapshot was taken at the swap minute, so the persisted
        // condition drop will use the right exit-time energy. A
        // substitute coming on later replaces the same shirt but has
        // their own snapshot logged at full time above — these two
        // snapshots are keyed by `player_id`, so they cannot collide.
        for snapshot in context.substituted_out_physical_snapshots.drain(..) {
            result
                .physical_snapshots
                .insert(snapshot.player_id, snapshot);
        }

        // ── Pass 2a: per-side behaviour summaries ────────────────────────
        // Fold each side's stat lines (starters + used subs) into the
        // team summary the expectation layer reads.
        let left_summary = {
            let side = &result.left_team_players;
            TeamRatingSummary::from_stats(
                side.main
                    .iter()
                    .chain(side.substitutes_used.iter())
                    .filter_map(|id| result.player_stats.get(id)),
            )
        };
        let right_summary = {
            let side = &result.right_team_players;
            TeamRatingSummary::from_stats(
                side.main
                    .iter()
                    .chain(side.substitutes_used.iter())
                    .filter_map(|id| result.player_stats.get(id)),
            )
        };
        let left_team_id = result.left_team_players.team_id;

        // ── Pass 2b: raw (stat-line) + contextual (public) ratings ───────
        // `raw_match_rating` stays the pure `calculate()` verdict; the
        // public `match_rating` seed layers Stage-2 (team expectation)
        // and Stage-3 (condition) on top. Reputation / personality
        // shaping is added later by the league pipeline.
        let rated_ids: Vec<u32> = result.player_stats.keys().copied().collect();
        for pid in rated_ids {
            let on_left = result.left_team_players.main.contains(&pid)
                || result.left_team_players.substitutes_used.contains(&pid);
            let team_id = if on_left {
                left_team_id
            } else {
                result.right_team_players.team_id
            };
            let (team_goals, opponent_goals) = if team_id == home_team_id {
                (home_goals, away_goals)
            } else {
                (away_goals, home_goals)
            };
            let (own_summary, opp_summary) = if on_left {
                (&left_summary, &right_summary)
            } else {
                (&right_summary, &left_summary)
            };
            let expectation = RatingExpectationContext::from_match(
                own_summary,
                opp_summary,
                team_goals,
                opponent_goals,
                result.physical_snapshots.get(&pid),
            );
            if let Some(stats) = result.player_stats.get_mut(&pid) {
                let (raw, contextual) = {
                    let ctx = RatingContext::new(stats, team_goals, opponent_goals);
                    (ctx.calculate(), ctx.calculate_contextual(&expectation))
                };
                stats.raw_match_rating = raw;
                stats.match_rating = contextual;
            }
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
    pub(super) fn log_blowout_profile(
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
        // Raw `.skills.*` reads here are intentional: this aggregator
        // feeds the post-match log (a snapshot of "what skill profile
        // does this XI have on paper?"), not a live decision — fatigue
        // would only confuse the report.
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

    pub(super) fn play_inner(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
    ) -> PlayMatchStateResult {
        let result = PlayMatchStateResult::default();
        let inner_start = Instant::now();
        let mut tick_count: u64 = 0;

        let mut next_sub_time_ms: u64 = 0;
        let mut sub_times_initialized = false;
        let mut et_bonus_granted = false;

        let mut tick_ctx = GameTickContext::new(field);
        let mut events = EventCollection::with_capacity(10);

        let mut tick_parity: u32 = 0;
        let mut coach_eval_counter: u32 = 0;
        let mut tactical_eval_counter: u32 = 0;
        // Tactical refresh uses an adaptive cadence: BASE during stable
        // play, TRANSITION right after possession swings / set-piece
        // restarts / goals / coach-instruction changes / ball entering
        // or leaving the attacking third. Each "transition trigger"
        // opens a TRANSITION_WINDOW_TICKS window during which the
        // cheaper TRANSITION interval is used.
        const BASE_TACTICAL_INTERVAL_TICKS: u32 = 25;
        const TRANSITION_TACTICAL_INTERVAL_TICKS: u32 = 10;
        const TRANSITION_WINDOW_TICKS: u32 = 40;
        let mut transition_window_remaining: u32 = TRANSITION_WINDOW_TICKS;
        // Snapshots used to detect transition triggers between refresh
        // points without a per-tick walk over players.
        let mut last_owner_id: Option<u32> = field.ball.current_owner;
        let mut last_possession_team: Option<u32> = last_owner_id
            .and_then(|id| field.players.iter().find(|p| p.id == id).map(|p| p.team_id));
        let mut last_home_score: u8 = context.score.home_team.get();
        let mut last_away_score: u8 = context.score.away_team.get();
        let mut last_home_instruction = context.coach_home.instruction;
        let mut last_away_instruction = context.coach_away.instruction;
        let mut last_home_zone = context.tactical_home.ball_zone;
        let mut last_away_zone = context.tactical_away.ball_zone;
        // Position recording cursor — replaces the per-tick
        // `timestamp % POSITION_RECORD_INTERVAL_MS == 0` check. Round
        // the starting timestamp UP to the next multiple of the
        // recording interval so a half restart preserves the original
        // 30 ms cadence (the loop increments time *before* the body,
        // so we never see `t == 0`).
        let initial_t = context.total_match_time;
        let mut next_position_record_ms: u64 =
            (initial_t / Self::POSITION_RECORD_INTERVAL_MS + 1) * Self::POSITION_RECORD_INTERVAL_MS;
        let track_positions = match_data.is_tracking_positions();

        while context.increment_time() {
            tick_count += 1;
            tick_parity += 1;
            coach_eval_counter += 1;
            tactical_eval_counter += 1;
            if transition_window_remaining > 0 {
                transition_window_remaining -= 1;
            }

            // Coach evaluates every 500 ticks (~5 seconds of match time)
            if coach_eval_counter >= 500 {
                coach_eval_counter = 0;
                Self::evaluate_coaches(field, context);
                // Once every coach-eval slice, also probe for situational
                // formation overrides — the manager swap to a chasing /
                // protecting shape based on score and minute. Cheap: a
                // single match arm and an equality check against the
                // current type per side.
                Self::evaluate_situational_shape(field, &mut *context);
            }

            // Team-level tactical state (phase, possession timers, line
            // height) used a fixed 10-tick cadence. Adaptive cadence:
            // stable possession uses BASE (25 ticks), while a 40-tick
            // window after any transition trigger drops to TRANSITION
            // (10 ticks) so phase/line-height/transition windows still
            // resolve crisply when the game state actually shifts.
            //
            // Triggers (each cheap, no per-tick player walks):
            //   • possession owner team changed
            //   • score changed (goal scored — handled via reset path)
            //   • coach instruction changed for either side
            //   • ball zone moved into / out of attacking third for
            //     either side
            //
            // Set-piece restarts are covered indirectly: kickoff /
            // corner / goal kick all reassign the ball owner, which
            // flips `last_possession_team` and re-opens the window.
            //
            // Cheap fast path: most ticks have the same `current_owner`
            // as the previous tick (passes/dribbles span many ticks).
            // Only re-resolve `team_id` via a 22-element scan when the
            // raw id actually changed since the last evaluation.
            let raw_owner = field.ball.current_owner;
            let current_owner_team = if raw_owner == last_owner_id {
                last_possession_team
            } else {
                last_owner_id = raw_owner;
                raw_owner
                    .and_then(|id| field.players.iter().find(|p| p.id == id).map(|p| p.team_id))
            };
            let possession_changed =
                current_owner_team != last_possession_team && current_owner_team.is_some();
            let home_score_now = context.score.home_team.get();
            let away_score_now = context.score.away_team.get();
            let score_changed =
                home_score_now != last_home_score || away_score_now != last_away_score;
            let home_instr_now = context.coach_home.instruction;
            let away_instr_now = context.coach_away.instruction;
            let instr_changed =
                home_instr_now != last_home_instruction || away_instr_now != last_away_instruction;
            let home_zone_now = context.tactical_home.ball_zone;
            let away_zone_now = context.tactical_away.ball_zone;
            // Attacking-third entry/exit on either side.
            use crate::r#match::BallZone;
            let zone_changed = matches!(home_zone_now, BallZone::AttackingThird)
                != matches!(last_home_zone, BallZone::AttackingThird)
                || matches!(away_zone_now, BallZone::AttackingThird)
                    != matches!(last_away_zone, BallZone::AttackingThird);
            if possession_changed || score_changed || instr_changed || zone_changed {
                transition_window_remaining = TRANSITION_WINDOW_TICKS;
                if possession_changed {
                    last_possession_team = current_owner_team;
                }
                if score_changed {
                    last_home_score = home_score_now;
                    last_away_score = away_score_now;
                }
                if instr_changed {
                    last_home_instruction = home_instr_now;
                    last_away_instruction = away_instr_now;
                }
                if zone_changed {
                    last_home_zone = home_zone_now;
                    last_away_zone = away_zone_now;
                }
            }

            let tactical_interval = if transition_window_remaining > 0 {
                TRANSITION_TACTICAL_INTERVAL_TICKS
            } else {
                BASE_TACTICAL_INTERVAL_TICKS
            };
            if tactical_eval_counter >= tactical_interval {
                let interval = tactical_eval_counter;
                tactical_eval_counter = 0;
                Self::refresh_tactical_states(field, context, interval);
                // refresh_tactical_states may have repointed
                // ball_zone — re-snapshot to avoid spuriously
                // re-triggering the window on the next tick.
                last_home_zone = context.tactical_home.ball_zone;
                last_away_zone = context.tactical_away.ball_zone;
            }

            // Full tick: ball + player AI + events
            // Light tick: ball + player movement only (no AI re-evaluation)
            if tick_parity & 1 == 0 {
                Self::game_tick_light(field, context, match_data, &mut tick_ctx, &mut events);
            } else {
                Self::game_tick_inner(field, context, match_data, &mut tick_ctx, &mut events);
            }

            // Replay-position recording, gated by a cursor instead of
            // a per-tick modulo. Same 30 ms cadence as before; just one
            // u64 comparison + add per tick when nothing is being
            // tracked (the dominant production case).
            if track_positions && context.total_match_time >= next_position_record_ms {
                Self::write_match_positions(field, context.total_match_time, match_data);
                next_position_record_ms += Self::POSITION_RECORD_INTERVAL_MS;
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
                    next_sub_time_ms = context.rng.range_u64(10, 20) * 60 * 1000;
                    sub_times_initialized = true;
                }

                let period_time = context.time.time;
                if period_time >= next_sub_time_ms {
                    // Deterministic "today" — captured at context
                    // construction. Used only for the youth-protection
                    // sub branch, where the comparison is age <= 17.
                    let today = context.today;
                    let per_pass_cap = context.max_substitutions_per_pass;
                    process_substitutions(field, context, per_pass_cap, today);
                    next_sub_time_ms = period_time + context.rng.range_u64(5, 15) * 60 * 1000;
                }
            }
        }

        PerfCounters::instance().record_play_inner(tick_count, inner_start.elapsed());
        result
    }
}
