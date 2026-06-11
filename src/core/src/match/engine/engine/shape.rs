use super::*;
use crate::MatchTacticType;
use crate::TacticSelectionReason;
use crate::Tactics;
use crate::r#match::MatchCoach;
use crate::r#match::RollingTeamMetrics;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    // ───────────────────────────────────────────────────────────────────────
    // Coach evaluation
    // ───────────────────────────────────────────────────────────────────────

    /// Minimum sim-minutes between successive shape changes for a
    /// match. Stops the coach from flipping shape at every 5-second
    /// eval slice when a goal arrives — real coaches give a setup at
    /// least 10-15 minutes to settle before re-evaluating.
    const SHAPE_CHANGE_MIN_MINUTES_GAP: u8 = 12;

    /// In-match shape change. Probes `TacticsSelector::situational_shape`
    /// for both sides; if the helper returns a new formation different
    /// from the side's current `tactic_type` AND the hysteresis window
    /// has elapsed, the side's `Tactics` struct is rebuilt around the
    /// new shape. The team-tactical refresh pipeline picks the new
    /// shape up on its next pass — pressing / line height / mentality
    /// already key off `tactical_style()`.
    pub(super) fn evaluate_situational_shape(field: &mut MatchField, context: &mut MatchContext) {
        use crate::club::team::tactics::tactics::TacticsSelector;
        let minutes = (context.total_match_time / 60_000).min(120) as u8;
        // Shape changes are score-reactive behavior — same visibility
        // gate as coach instructions / tactics (final ~28 min only).
        let home_diff = if !context.behavioral_score_visible() {
            0
        } else {
            (context.score.home_team.get() as i16 - context.score.away_team.get() as i16)
                .clamp(-100, 100) as i8
        };
        let away_diff = -home_diff;

        // Hysteresis: skip the probe entirely if the last shape change
        // (any side) was within `SHAPE_CHANGE_MIN_MINUTES_GAP` minutes.
        // First change is always allowed because `last_shape_change_tick`
        // starts at u64::MAX.
        let cooldown_active = if context.last_shape_change_tick == u64::MAX {
            false
        } else {
            let elapsed_ms = context
                .total_match_time
                .saturating_sub(context.last_shape_change_tick);
            elapsed_ms < (Self::SHAPE_CHANGE_MIN_MINUTES_GAP as u64) * 60_000
        };
        if cooldown_active {
            return;
        }

        // Map left/right squad → home/away by checking which side the
        // home squad currently occupies. Sides swap at half-time so we
        // can't hardcode left=home.
        let home_is_left = field
            .left_side_players
            .as_ref()
            .map(|s| s.team_id == context.field_home_team_id)
            .unwrap_or(true);

        // First-pass: figure out what each side WOULD change to,
        // without mutating yet. Lets us stamp the change minute and
        // last-change tick exactly once per probe even when both
        // sides flip simultaneously.
        let probe_target = |current: MatchTacticType, is_home: bool, score_diff: i8| {
            TacticsSelector::situational_shape(current, is_home, score_diff, minutes)
        };

        let (home_tactics_ref, away_tactics_ref) = if home_is_left {
            (&mut field.left_team_tactics, &mut field.right_team_tactics)
        } else {
            (&mut field.right_team_tactics, &mut field.left_team_tactics)
        };

        let home_target = probe_target(home_tactics_ref.tactic_type, true, home_diff);
        let away_target = probe_target(away_tactics_ref.tactic_type, false, away_diff);

        let mut any_change = false;
        if let Some(new_shape) = home_target {
            *home_tactics_ref = Tactics::with_reason(
                new_shape,
                TacticSelectionReason::GameSituation,
                home_tactics_ref.formation_strength,
            );
            any_change = true;
        }
        if let Some(new_shape) = away_target {
            *away_tactics_ref = Tactics::with_reason(
                new_shape,
                TacticSelectionReason::GameSituation,
                away_tactics_ref.formation_strength,
            );
            any_change = true;
        }

        if any_change {
            context.last_shape_change_tick = context.total_match_time;
            if context.first_shape_change_minute.is_none() {
                context.first_shape_change_minute = Some(minutes);
            }
        }
    }

    pub(super) fn evaluate_coaches(field: &MatchField, context: &mut MatchContext) {
        // Coaches see the real scoreline only once score-reactive
        // football engages (final ~28 min — see
        // `SCORE_REACTION_FROM_MINUTE`); before that, and under the
        // OF_SCORE_BLIND diagnostic, they coach as if level.
        let (home_goals, away_goals) = if !context.behavioral_score_visible() {
            (0i8, 0i8)
        } else {
            (
                context.score.home_team.get() as i8,
                context.score.away_team.get() as i8,
            )
        };
        let current_tick = context.current_tick();

        // Regulation progress capped at 1.0. In extra time `total_match_time`
        // keeps climbing past 90 min; without the clamp `is_late_game` and
        // `is_very_late` stay true but `is_first_half_end` (0.45..0.55) goes
        // stale and the `match` branches misbehave for losing teams.
        let match_progress = (context.total_match_time as f32 / MATCH_TIME_MS as f32).min(1.0);

        // One pass over the player list collects condition + cumulative
        // metric totals (xG, shots, press, deep entries, dangerous
        // turnovers) for both sides. The smarter
        // `evaluate_with_metrics` consumes the deltas vs a rolling
        // 15-min snapshot we maintain on each coach state.
        let mut home_cond_sum = 0.0f32;
        let mut home_count = 0u32;
        let mut away_cond_sum = 0.0f32;
        let mut away_count = 0u32;
        let mut home_xg = 0.0f32;
        let mut away_xg = 0.0f32;
        let mut home_shots = 0u32;
        let mut away_shots = 0u32;
        let mut home_pressures = 0u32;
        let mut away_pressures = 0u32;
        let mut home_successful_pressures = 0u32;
        let mut away_successful_pressures = 0u32;
        let mut home_deep_entries = 0u32;
        let mut away_deep_entries = 0u32;
        let mut home_dangerous_turnovers = 0u32;
        let mut away_dangerous_turnovers = 0u32;

        for p in field.players.iter() {
            let cond = p.player_attributes.condition as f32 / 10000.0;
            let xg = p.memory.xg_total;
            let shots = p.memory.shots_taken as u32;
            let pressures = p.statistics.pressures as u32;
            let succ = p.statistics.successful_pressures as u32;
            // Passes that arrived in the opponent's box. Our nearest
            // proxy for "deep territorial entries" — every pass-into-
            // box advances the team's threat into the danger zone.
            let deep_entries = p.statistics.passes_into_box as u32;
            // Coach-side "dangerous turnovers" = errors that produced an
            // opposition shot PLUS any giveaway inside the team's own
            // box (irrespective of whether the opponent converted
            // within the response window). The own-third bucket is
            // deliberately NOT summed — that's a much wider net and
            // would dilute the smart-coach trigger. Aligns with the
            // rating helper's zone counters: an own-box turnover is
            // rated as the worst non-error event, so the coach
            // metric matches that severity.
            let dangerous_turnovers = (p.statistics.errors_leading_to_shot as u32)
                + (p.statistics.zone_stats.dangerous_turnovers_own_box as u32);
            if p.team_id == context.field_home_team_id {
                home_cond_sum += cond;
                home_count += 1;
                home_xg += xg;
                home_shots += shots;
                home_pressures += pressures;
                home_successful_pressures += succ;
                home_deep_entries += deep_entries;
                home_dangerous_turnovers += dangerous_turnovers;
            } else {
                away_cond_sum += cond;
                away_count += 1;
                away_xg += xg;
                away_shots += shots;
                away_pressures += pressures;
                away_successful_pressures += succ;
                away_deep_entries += deep_entries;
                away_dangerous_turnovers += dangerous_turnovers;
            }
        }

        let home_avg_condition = if home_count > 0 {
            home_cond_sum / home_count as f32
        } else {
            0.5
        };
        let away_avg_condition = if away_count > 0 {
            away_cond_sum / away_count as f32
        } else {
            0.5
        };

        // Build per-side rolling metrics by diffing against a snapshot
        // taken ~15 sim-minutes ago. After the window expires, advance
        // the snapshot to the current totals so the next pass starts a
        // fresh window. xG_against is the OTHER team's xG_for delta.
        let home_input = RollingMetricsInput {
            cum_xg_for: home_xg,
            cum_xg_against: away_xg,
            cum_shots_for: home_shots,
            cum_pressures: home_pressures,
            cum_successful_pressures: home_successful_pressures,
            cum_deep_entries: home_deep_entries,
            cum_dangerous_turnovers: home_dangerous_turnovers,
        };
        let away_input = RollingMetricsInput {
            cum_xg_for: away_xg,
            cum_xg_against: home_xg,
            cum_shots_for: away_shots,
            cum_pressures: away_pressures,
            cum_successful_pressures: away_successful_pressures,
            cum_deep_entries: away_deep_entries,
            cum_dangerous_turnovers: away_dangerous_turnovers,
        };
        let home_metrics =
            Self::build_rolling_metrics(&mut context.coach_home, current_tick, &home_input);
        let away_metrics =
            Self::build_rolling_metrics(&mut context.coach_away, current_tick, &away_input);

        context.coach_home.evaluate_with_metrics(
            home_goals - away_goals,
            match_progress,
            home_avg_condition,
            current_tick,
            home_metrics,
        );
        context.coach_away.evaluate_with_metrics(
            away_goals - home_goals,
            match_progress,
            away_avg_condition,
            current_tick,
            away_metrics,
        );
    }

    /// Build a `RollingTeamMetrics` from current cumulative totals and
    /// the per-coach snapshot. Window-rolls the snapshot when 15 sim
    /// minutes (≈ 90 000 ticks) of play have elapsed since the last
    /// rotation.
    pub(super) fn build_rolling_metrics(
        coach: &mut MatchCoach,
        current_tick: u64,
        input: &RollingMetricsInput,
    ) -> RollingTeamMetrics {
        use crate::r#match::engine::coach::MetricSnapshot;
        const WINDOW_TICKS: u64 = 90_000; // 15 sim minutes
        const POSSESSION_WINDOW_TICKS: u64 = 60_000; // 10 sim minutes
        let snap = coach.metric_snapshot;
        let elapsed = current_tick.saturating_sub(snap.tick);

        let xg_for = (input.cum_xg_for - snap.xg_for).max(0.0);
        let xg_against = (input.cum_xg_against - snap.xg_against).max(0.0);
        let shots_for = input.cum_shots_for.saturating_sub(snap.shots_for) as u16;
        let pressures = input.cum_pressures.saturating_sub(snap.pressures);
        let successful_pressures = input
            .cum_successful_pressures
            .saturating_sub(snap.successful_pressures);
        let press_success_rate = if pressures > 0 {
            successful_pressures as f32 / pressures as f32
        } else {
            0.5
        };
        let deep_entries_for_last_15 =
            input.cum_deep_entries.saturating_sub(snap.deep_entries_for) as u16;
        let dangerous_turnovers_last_10 = input
            .cum_dangerous_turnovers
            .saturating_sub(snap.dangerous_turnovers)
            as u16;

        // Possession + field-tilt run as cumulative tick counters on
        // the coach itself, updated by `refresh_tactical_states`. The
        // delta over the snapshot window is the share-of-play we want.
        let poss_delta = coach
            .cum_possession_ticks
            .saturating_sub(snap.possession_ticks) as u64;
        let tilt_delta = coach
            .cum_field_tilt_ticks
            .saturating_sub(snap.field_tilt_ticks) as u64;
        let possession_window = elapsed.min(POSSESSION_WINDOW_TICKS).max(1);
        let possession_last_10 = (poss_delta as f32 / possession_window as f32).clamp(0.0, 1.0);
        let field_tilt_last_10 = (tilt_delta as f32 / possession_window as f32).clamp(0.0, 1.0);

        if elapsed >= WINDOW_TICKS {
            coach.metric_snapshot = MetricSnapshot {
                tick: current_tick,
                xg_for: input.cum_xg_for,
                xg_against: input.cum_xg_against,
                shots_for: input.cum_shots_for,
                pressures: input.cum_pressures,
                successful_pressures: input.cum_successful_pressures,
                deep_entries_for: input.cum_deep_entries,
                dangerous_turnovers: input.cum_dangerous_turnovers,
                possession_ticks: coach.cum_possession_ticks,
                field_tilt_ticks: coach.cum_field_tilt_ticks,
            };
        }

        let mut metrics = RollingTeamMetrics::default();
        metrics.xg_for_last_15 = xg_for;
        metrics.xg_against_last_15 = xg_against;
        metrics.shots_for_last_15 = shots_for;
        metrics.deep_entries_for_last_15 = deep_entries_for_last_15;
        metrics.field_tilt_last_10 = field_tilt_last_10;
        metrics.possession_last_10 = possession_last_10;
        metrics.dangerous_turnovers_last_10 = dangerous_turnovers_last_10;
        metrics.press_success_rate_last_10 = press_success_rate;
        // `avg_defensive_line_breaks` is intentionally left at 0 — the
        // engine doesn't currently track per-side line-break events
        // (would need to instrument the offside trap / through-ball
        // resolver). The smart coach evaluator falls back gracefully
        // when this is 0; remove or implement once the underlying
        // signal exists.
        metrics.avg_defensive_line_breaks = 0.0;
        metrics
    }

    /// Refresh the team-level tactical state (phase, possession timers,
    /// defensive-line height) for both sides. Reads only the ball and
    /// players; mutates the `tactical_home` / `tactical_away` fields on
    /// `MatchContext`. `tick_interval` is how many ticks elapsed since
    /// the last refresh — rolling counters scale with it.
    pub(super) fn refresh_tactical_states(
        field: &MatchField,
        context: &mut MatchContext,
        tick_interval: u32,
    ) {
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

        // Per-team skill composite aggregates. Recomputed only every
        // SKILL_AGGREGATE_INTERVAL_TICKS (or on roster invalidation) —
        // each pass walks 22 players and calls 6-8 fatigue-aware
        // composite helpers per player, so dropping the cadence from
        // every-tactical-refresh to ~once per second cuts a sizable
        // chunk of refresh CPU. The previous fixed-cadence value
        // could be tens of thousands of times per match; the cache
        // brings it down to ~5_400.
        const SKILL_AGGREGATE_INTERVAL_TICKS: u64 = 100;
        let current_tick = context.current_tick();
        let needs_recompute = context.skill_aggregates_dirty
            || current_tick.saturating_sub(context.last_skill_aggregate_tick)
                >= SKILL_AGGREGATE_INTERVAL_TICKS;
        if needs_recompute {
            let minute_now = sc::minute_from_ms(context.total_match_time);
            let mut home_skills = SkillAccumulator::new();
            let mut away_skills = SkillAccumulator::new();
            for p in field.players.iter().filter(|p| !p.is_sent_off) {
                let bucket = if p.team_id == context.field_home_team_id {
                    &mut home_skills
                } else {
                    &mut away_skills
                };
                bucket.add(p, minute_now);
            }
            context.home_skill_aggregates = home_skills.finalize();
            context.away_skill_aggregates = away_skills.finalize();
            context.last_skill_aggregate_tick = current_tick;
            context.skill_aggregates_dirty = false;
        }
        let home_skill_aggregates = context.home_skill_aggregates;
        let away_skill_aggregates = context.away_skill_aggregates;

        let home_goals = context.score.home_team.get() as i16;
        let away_goals = context.score.away_team.get() as i16;
        // Tactics see the real scoreline only once score-reactive
        // football engages (see `SCORE_REACTION_FROM_MINUTE`).
        let home_score_diff = if !context.behavioral_score_visible() {
            0
        } else {
            (home_goals - away_goals).clamp(-100, 100) as i8
        };

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
            home_skills: home_skill_aggregates,
            away_skills: away_skill_aggregates,
            home_edge: context.environment.crowd_intensity * context.environment.home_advantage,
        };
        TeamTacticalState::refresh(
            &mut context.tactical_home,
            &mut context.tactical_away,
            &inputs,
        );

        // Cumulative possession + field-tilt counters feed the rolling
        // metrics consumed by the smart coach evaluator. Updated here
        // (every ~10 ticks) so the per-coach-eval pass doesn't have to
        // re-derive them from scratch.
        use crate::r#match::BallZone;
        if context.tactical_home.in_possession {
            context.coach_home.cum_possession_ticks = context
                .coach_home
                .cum_possession_ticks
                .saturating_add(tick_interval);
        }
        if context.tactical_away.in_possession {
            context.coach_away.cum_possession_ticks = context
                .coach_away
                .cum_possession_ticks
                .saturating_add(tick_interval);
        }
        // Ball in *our* attacking third counts as field-tilt for us.
        if matches!(context.tactical_home.ball_zone, BallZone::AttackingThird) {
            context.coach_home.cum_field_tilt_ticks = context
                .coach_home
                .cum_field_tilt_ticks
                .saturating_add(tick_interval);
        }
        if matches!(context.tactical_away.ball_zone, BallZone::AttackingThird) {
            context.coach_away.cum_field_tilt_ticks = context
                .coach_away
                .cum_field_tilt_ticks
                .saturating_add(tick_interval);
        }
    }
}
