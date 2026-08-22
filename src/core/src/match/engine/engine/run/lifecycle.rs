//! **The match lifecycle** — the `play()` entry points and the config
//! they build, plus `play_inner`, the per-state loop that runs the ticks
//! for one `MatchState` and hands back its stoppage time.

use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::engine::engine::phase_prof::PhaseProf;
use crate::r#match::engine::engine::*;

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

        PhaseProf::init_from_env();
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
            ResultMatchPositionData::new_with_tracking().with_scope(MatchRuntime::recording_scope())
        } else {
            ResultMatchPositionData::new().with_scope(MatchRuntime::recording_scope())
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
        context
            .chemistry
            .seed_from_roster(&chemistry_roster, field_h);

        // Home-crowd arousal — the play-quality half of home advantage.
        // Stamped once on every match player (starters AND bench, so
        // substitutes carry it on) and consumed inside `effective_skill`.
        // Scaled by the environment so an empty-stadium friendly confers
        // ~nothing (matching the COVID ghost-game finding that home
        // advantage largely vanishes without crowds) and a packed derby
        // confers the full edge. At the default environment (crowd 0.55
        // × home_advantage 0.50 → edge 0.275) this is home ≈ +3.3% /
        // away ≈ −2.75% effective skill. Magnitude was titrated against
        // the engine's measured response: ±(+1.7/−1.4)% produced a
        // +5.5pp home-win gap at equal strength; the documented real
        // split (~45/25/30, +0.35 home goals) needs roughly double
        // that, landing here. The officiating half (referee marginal
        // calls) stacks on top via `RefereeProfile::home_bias`.
        let home_edge = (context.environment.crowd_intensity * context.environment.home_advantage)
            .clamp(0.0, 1.0);
        // Re-titrated 2026-08-08 (0.12 / 0.07 → 0.26 / 0.15). The
        // comment above records a titration against the engine's
        // measured response, and that response moved: with defenders
        // finally getting goal-side of the ball (`DefensiveRecovery`) a
        // small effective-skill edge converts into far fewer goals than
        // it did when every shot was a one-on-one with the keeper.
        // Re-measured over 3000 matches at the old values the home edge
        // had decayed to **+0.097 goals/match against a real +0.35**,
        // with results at 35.6 / 36.3 / 28.1 against this module's own
        // documented target of 42-48 / 23-30 / 27-34 — home wins 7pp
        // short and draws 6pp long.
        // Re-titrated again 2026-08-11 (0.26 / 0.15 → 0.13 / 0.075). The
        // engine's response to an effective-skill edge moved a SECOND
        // time, and in the opposite direction: with man-marking assigned
        // per-opponent and defenders actually engaging carriers, a small
        // skill edge now compounds through every duel in a possession
        // instead of washing out. Measured at the old values: **70.0 /
        // 25.0 / 5.0** with home 1.77 goals against away 0.43, against
        // this module's documented target of 42-48 / 23-30 / 27-34 and a
        // real home-goal edge of ~+0.35.
        //
        // The lesson the two re-titrations share: this constant is not a
        // property of home advantage, it is a property of how strongly
        // THIS engine converts skill into goals, so it has to be re-read
        // after any change to the duel model.
        let home_arousal = 1.0 + 0.13 * home_edge;
        let away_arousal = 1.0 - 0.075 * home_edge;
        let home_team_id = field.home_team_id;
        for p in field.players.iter_mut().chain(field.substitutes.iter_mut()) {
            p.crowd_arousal = if p.team_id == home_team_id {
                home_arousal
            } else {
                away_arousal
            };
        }

        if MatchRuntime::events_mode() {
            context.enable_logging();
        }

        let mut state_manager = StateManager::new();

        // Match kickoff — home team (playing Left in the first half)
        // starts the game with possession on the centre spot. Without
        // this the ball sits at centre until the emergency chaser
        // override fires, producing a ~14-second dead patch.
        assign_kickoff(&mut field, PlayerSide::Left, None);

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

        let result = Self::build_result(field, context, match_position_data);
        if PhaseProf::enabled() {
            PhaseProf::report_and_reset("match");
        }
        result
    }

    /// Stub match: skips the whole simulation and returns a 0-0
    /// scoreline with the minimum data downstream consumers expect
    /// (team IDs in `Score`, player IDs in the field squads). Gated
    /// on the `match-stub` Cargo feature; intended for profiling the
    /// pipeline around the engine.
    #[cfg(feature = "match-stub")]
    pub(in crate::r#match::engine::engine) fn play_stub(
        left_squad: MatchSquad,
        right_squad: MatchSquad,
    ) -> MatchResultRaw {
        use crate::r#match::engine::result::FieldSquad;

        let mut result = MatchResultRaw::with_match_time(90 * 60 * 1000);
        result.score = Some(Score::new(left_squad.team_id, right_squad.team_id));
        result.left_team_players = FieldSquad::from_team(&left_squad);
        result.right_team_players = FieldSquad::from_team(&right_squad);
        result
    }

    // ───────────────────────────────────────────────────────────────────────
    // Match state loop
    // ───────────────────────────────────────────────────────────────────────

    pub(in crate::r#match::engine::engine) fn play_inner(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
    ) -> PlayMatchStateResult {
        let result = PlayMatchStateResult::default();
        let prof_on = PhaseProf::enabled();

        let mut next_sub_time_ms: u64 = 0;
        let mut sub_times_initialized = false;
        let mut et_bonus_granted = false;
        // Medical (forced-injury) pass scheduling — independent of the
        // discretionary sub timer, re-armed at the start of each period.
        let mut next_medical_time_ms: u64 = 0;
        let mut medical_period: Option<MatchState> = None;

        let mut tick_ctx = GameTickContext::new(field, &context.players);
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
            // Post-goal dead time. No ball physics, no AI, no events, no
            // coach evals — see `MatchContext::dead_ball_until_ms` for why
            // the pause is load-bearing (it consumed the post-goal hot
            // window that made goals beget goals).
            //
            // What DOES run is the celebration: the ball settling in the
            // net, the scorer's run, the pile-on, somebody fetching the ball
            // back. It is a cutscene — no decision inside it can reach a
            // ball, a duel or the RNG stream — and the restart it performs
            // at the end of the window leaves precisely the state the old
            // instant reset left at the start of it. Recording it is the
            // point: with the tick body skipped outright the replay held
            // the last pre-goal frame for a minute, which is why the ball
            // appeared to stop dead on the goal line.
            if context.total_match_time < context.dead_ball_until_ms {
                let celebrating = advance_goal_celebration(field, context);
                if celebrating
                    && track_positions
                    && context.total_match_time >= next_position_record_ms
                {
                    Self::write_match_positions(field, context.total_match_time, match_data);
                    next_position_record_ms += Self::POSITION_RECORD_INTERVAL_MS;
                }
                continue;
            }

            // The window has closed. Any celebration still standing performs
            // its restart here, BEFORE the first live tick — so play resumes
            // from the kickoff set-up, exactly as it did when the reset
            // happened at the far end of the window. It cannot be done inside
            // the branch above, whose own condition is what keeps the clock
            // short of the restart instant.
            if context.goal_celebration.is_some() {
                advance_goal_celebration(field, context);
            }

            tick_parity += 1;
            coach_eval_counter += 1;
            tactical_eval_counter += 1;
            if transition_window_remaining > 0 {
                transition_window_remaining -= 1;
            }

            // Coach evaluates every 500 ticks (~5 seconds of match time)
            if coach_eval_counter >= 500 {
                coach_eval_counter = 0;
                let prof_t = prof_on.then(Instant::now);
                Self::evaluate_coaches(field, context);
                // Once every coach-eval slice, also probe for situational
                // formation overrides — the manager swap to a chasing /
                // protecting shape based on score and minute. Cheap: a
                // single match arm and an equality check against the
                // current type per side.
                Self::evaluate_situational_shape(field, &mut *context);
                if let Some(t) = prof_t {
                    PhaseProf::add(PhaseProf::P_COACH, t.elapsed().as_nanos() as u64);
                }
                // Condition-trajectory sampling for the dev harness —
                // average condition per position group per 15-min band.
                // Rides the coach cadence so it costs one 22-player walk
                // every 5 sim-seconds, match-logs builds only.
                #[cfg(feature = "match-logs")]
                {
                    use crate::r#match::player::strategies::players::ops::forward_shot_decision::time_band_diag;
                    use std::sync::atomic::Ordering;
                    let band =
                        time_band_diag::band_for_minute((context.total_match_time / 60_000) as u32);
                    for p in field.players.iter().filter(|p| !p.is_sent_off) {
                        let group = match p.tactical_position.current_position.position_group() {
                            crate::PlayerFieldPositionGroup::Goalkeeper => 0,
                            crate::PlayerFieldPositionGroup::Defender => 1,
                            crate::PlayerFieldPositionGroup::Midfielder => 2,
                            crate::PlayerFieldPositionGroup::Forward => 3,
                        };
                        time_band_diag::COND_SUM_BY_BAND_GROUP[band][group].fetch_add(
                            p.player_attributes.condition.max(0) as u64,
                            Ordering::Relaxed,
                        );
                        time_band_diag::COND_N_BY_BAND_GROUP[band][group]
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
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
                let prof_t = prof_on.then(Instant::now);
                Self::refresh_tactical_states(field, context, interval);
                if let Some(t) = prof_t {
                    PhaseProf::add(PhaseProf::P_TACTICAL, t.elapsed().as_nanos() as u64);
                }
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

            // Forced medical substitutions run in ANY playing period —
            // real football replaces an injured player whenever it
            // happens, first half included. The pass owns the in-match
            // injury roll; first check lands 3-8 minutes into each
            // period, then every 6-14 minutes.
            let medical_enabled = matches!(
                context.state.match_state,
                MatchState::FirstHalf | MatchState::SecondHalf | MatchState::ExtraTime
            );
            if medical_enabled {
                if medical_period != Some(context.state.match_state) {
                    medical_period = Some(context.state.match_state);
                    next_medical_time_ms =
                        context.time.time + context.rng.range_u64(3, 8) * 60 * 1000;
                }
                if context.time.time >= next_medical_time_ms {
                    Substitutions::process_medical(field, context);
                    next_medical_time_ms =
                        context.time.time + context.rng.range_u64(6, 14) * 60 * 1000;
                }
            }

            // Discretionary substitutions allowed from the second half
            // onwards, plus extra time when we reach it in a knockout
            // tie. First-half subs in real football are reactive
            // (injuries) — the medical pass above owns those. ET gets
            // one bonus sub on entry (FIFA rule).
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

        // The whistle can go while the ball is still in the net. Settle the
        // goal before the period boundary runs its own resets, so nothing
        // downstream sees a half-processed one.
        finish_goal_celebration(field, context);

        result
    }
}
