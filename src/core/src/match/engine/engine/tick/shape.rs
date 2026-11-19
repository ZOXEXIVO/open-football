//! **The plan-refresh side of the tick** — the interval-gated passes that
//! keep the two teams' plans current while the match runs: the coach's
//! moving read of the run of play, the plan changes he makes at a moment,
//! and the attack / defence / shape / tactical state refresh every player's
//! decision then reads.
//!
//! These run on their own cadences from `play_inner`, not on every tick,
//! which is why they sit beside the driver rather than inside it.

use crate::MatchTacticType;
use crate::TacticSelectionReason;
use crate::Tactics;
use crate::r#match::MatchCoach;
use crate::r#match::ReadCounters;
use crate::r#match::ShapeChoice;
use crate::r#match::engine::engine::*;
use crate::r#match::engine::flow::arena::formation_variant::FormationVariant;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    // ───────────────────────────────────────────────────────────────────────
    // Coach evaluation
    // ───────────────────────────────────────────────────────────────────────

    /// A coach at a moment decides whether to change the plan; a change of
    /// formation re-lays his side on the field and in the context's copy of
    /// the tactics.
    pub(crate) fn decide_plans(field: &mut MatchField, context: &mut MatchContext) {
        let minute = context.total_match_time as f32 / 60_000.0;
        let gain = MatchContext::stance_gain();
        let mut relaid = false;
        for team_id in [context.field_home_team_id, context.field_away_team_id] {
            if !context.coach_for_team(team_id).has_moment() {
                continue;
            }
            let tactics = if field.side_of(team_id) == PlayerSide::Left {
                &field.left_team_tactics
            } else {
                &field.right_team_tactics
            };
            let (shape, strength) = (tactics.tactic_type, tactics.formation_strength);
            let spare = context.coach_for_team(team_id).spare_line();
            let familiarity = context.formation_familiarity(team_id);
            let choices: Vec<ShapeChoice> = MatchTacticType::all()
                .into_iter()
                .map(Tactics::new)
                .filter(|t| {
                    t.tactic_type == shape
                        || FormationVariant::can_fill(field, team_id, t.positions(), spare)
                })
                .map(|t| ShapeChoice {
                    shape: t.tactic_type,
                    commitment: t.commitment(),
                    familiarity: familiarity.of(t.tactic_type),
                })
                .collect();
            let coach = context.coach_for_team_mut(team_id);
            let Some(change) = coach.decide(minute, shape, &choices, gain) else {
                continue;
            };
            if change.changes_shape() {
                field.change_formation(
                    team_id,
                    Tactics::with_reason(
                        change.shape_after,
                        TacticSelectionReason::GameSituation,
                        strength,
                    ),
                    spare,
                );
                relaid = true;
            }
        }
        if relaid {
            context.tactics = TeamsTactics::from_field(field);
            context.invalidate_skill_aggregates();
        }
    }

    pub(in crate::r#match::engine::engine) fn evaluate_coaches(
        field: &MatchField,
        context: &mut MatchContext,
    ) {
        // Coaches read the score from the goal; under the OF_SCORE_BLIND
        // diagnostic they coach as if level.
        let (home_goals, away_goals) = if !context.behavioral_score_visible() {
            (0i8, 0i8)
        } else {
            (
                context.score.home_team.get() as i8,
                context.score.away_team.get() as i8,
            )
        };
        // One pass over the player list collects the cumulative metric
        // totals (xG, shots, press, deep entries, dangerous turnovers) for
        // both sides; each coach folds them into his moving read.
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
        let on_pitch = |team_id: u32| {
            field
                .players
                .iter()
                .filter(|p| p.team_id == team_id && !p.off_pitch)
                .count() as i8
        };
        let home_on_pitch = on_pitch(context.field_home_team_id);
        let away_on_pitch = on_pitch(context.field_away_team_id);

        for p in field.players.iter() {
            let xg = p.memory.xg_total;
            let shots = p.memory.shots_taken;
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
            // would dilute the bench's read. Aligns with the
            // rating helper's zone counters: an own-box turnover is
            // rated as the worst non-error event, so the coach
            // metric matches that severity.
            let dangerous_turnovers = (p.statistics.errors_leading_to_shot as u32)
                + (p.statistics.zone_stats.dangerous_turnovers_own_box as u32);
            if p.team_id == context.field_home_team_id {
                home_xg += xg;
                home_shots += shots;
                home_pressures += pressures;
                home_successful_pressures += succ;
                home_deep_entries += deep_entries;
                home_dangerous_turnovers += dangerous_turnovers;
            } else {
                away_xg += xg;
                away_shots += shots;
                away_pressures += pressures;
                away_successful_pressures += succ;
                away_deep_entries += deep_entries;
                away_dangerous_turnovers += dangerous_turnovers;
            }
        }

        let counters = |coach: &MatchCoach,
                        xg_for,
                        xg_against,
                        shots_for,
                        pressures,
                        successful_pressures,
                        deep_entries,
                        dangerous_turnovers| ReadCounters {
            xg_for,
            xg_against,
            shots_for,
            pressures,
            successful_pressures,
            deep_entries,
            dangerous_turnovers,
            line_breaks_against: coach.cum_line_breaks_against,
            possession_ticks: coach.cum_possession_ticks,
            tilt_ticks: coach.cum_field_tilt_ticks,
            ticks: coach.cum_ticks,
        };
        let home_metrics = counters(
            &context.coach_home,
            home_xg,
            away_xg,
            home_shots,
            home_pressures,
            home_successful_pressures,
            home_deep_entries,
            home_dangerous_turnovers,
        );
        let away_metrics = counters(
            &context.coach_away,
            away_xg,
            home_xg,
            away_shots,
            away_pressures,
            away_successful_pressures,
            away_deep_entries,
            away_dangerous_turnovers,
        );

        let minute = context.total_match_time as f32 / 60_000.0;
        let minutes_left = MATCH_TIME_MS.saturating_sub(context.total_match_time) as f32 / 60_000.0;
        let home_difference = home_goals as i32 - away_goals as i32;
        context.coach_home.observe(
            Situation {
                difference: home_difference,
                minutes_left,
                own_men: home_on_pitch as u8,
                opp_men: away_on_pitch as u8,
            },
            minute,
            home_metrics,
        );
        context.coach_away.observe(
            Situation {
                difference: -home_difference,
                minutes_left,
                own_men: away_on_pitch as u8,
                opp_men: home_on_pitch as u8,
            },
            minute,
            away_metrics,
        );
        #[cfg(feature = "match-logs")]
        {
            context.coach_home.note_commitment(minute as usize);
            context.coach_away.note_commitment(minute as usize);
        }
    }

    /// Refresh the team-level tactical state (phase, possession timers,
    /// defensive-line height) for both sides. Reads the ball and players,
    /// and reads each man's adherence onto him with the skill aggregates;
    /// mutates the `tactical_home` / `tactical_away` fields on
    /// `MatchContext`. `tick_interval` is how many ticks elapsed since
    /// the last refresh — rolling counters scale with it.
    pub(crate) fn refresh_tactical_states(
        field: &mut MatchField,
        context: &mut MatchContext,
        possession: TeamPossession,
        tick_interval: u32,
    ) {
        // One pass over players collects condition for both sides.
        let (home_count, home_cond_sum, away_count, away_cond_sum) =
            field
                .players
                .iter()
                .fold((0u32, 0.0f32, 0u32, 0.0f32), |(hc, hcond, ac, acond), p| {
                    let cond = p.player_attributes.condition as f32 / 10000.0;
                    if p.team_id == context.field_home_team_id {
                        (hc + 1, hcond + cond, ac, acond)
                    } else {
                        (hc, hcond, ac + 1, acond + cond)
                    }
                });
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
            for p in field.players.iter().filter(|p| !p.off_pitch) {
                let bucket = if p.team_id == context.field_home_team_id {
                    &mut home_skills
                } else {
                    &mut away_skills
                };
                bucket.add(p, minute_now);
            }
            context.home_skill_aggregates = home_skills.finalize();
            context.away_skill_aggregates = away_skills.finalize();
            // How used each side is to the shape it is playing, from its
            // record: what its line steps with and what each man's
            // adherence reads.
            let familiarity = |team_id: u32| {
                let shape = if field.side_of(team_id) == PlayerSide::Left {
                    field.left_team_tactics.tactic_type
                } else {
                    field.right_team_tactics.tactic_type
                };
                context.formation_familiarity(team_id).of(shape)
            };
            let home_id = context.field_home_team_id;
            let (home, away) = (
                familiarity(home_id),
                familiarity(context.field_away_team_id),
            );
            context.tactical_familiarity_home.score = home;
            context.tactical_familiarity_away.score = away;
            if !MatchContext::adherence_off() {
                for p in field.players.iter_mut().filter(|p| !p.off_pitch) {
                    p.read_adherence(if p.team_id == home_id { home } else { away });
                }
            }
            context.last_skill_aggregate_tick = current_tick;
            context.skill_aggregates_dirty = false;
            let moment = context.pressure_moment();
            context
                .psychology
                .refresh(&field.players, current_tick, &moment);
        }
        let home_skill_aggregates = context.home_skill_aggregates;
        let away_skill_aggregates = context.away_skill_aggregates;

        // The stance reaches the levers scaled by the regime's one
        // amplitude, measured from the kickoff stance the plan carries.
        let gain = MatchContext::stance_gain();
        let home_stance = context.coach_home.stance_shift() * gain;
        let away_stance = context.coach_away.stance_shift() * gain;

        // Sides swap at half-time: read which end the home squad attacks.
        let home_is_left = field
            .left_side_players
            .as_ref()
            .map(|s| s.team_id == context.field_home_team_id)
            .unwrap_or(true);
        let home_side = if home_is_left {
            PlayerSide::Left
        } else {
            PlayerSide::Right
        };

        let inputs = TacticalRefreshInputs {
            field,
            possession_holder: possession.holder(),
            home_team_id: context.field_home_team_id,
            home_side,
            tick_interval,
            home_stance,
            away_stance,
            home_avg_condition: home_avg_cond,
            away_avg_condition: away_avg_cond,
            home_plan: context.coach_home.plan,
            away_plan: context.coach_away.plan,
            home_skills: home_skill_aggregates,
            away_skills: away_skill_aggregates,
            // A/B control — see `MatchContext::home_flat`.
            home_edge: if MatchContext::home_flat() {
                0.0
            } else {
                context.environment.crowd_intensity * context.environment.home_advantage
            },
            press_ceiling: 1.0 + context.conditions.high_press_intensity_cap,
        };
        TeamTacticalState::refresh(
            &mut context.tactical_home,
            &mut context.tactical_away,
            &inputs,
        );

        // Attacking roles are assigned on the same cadence and read the
        // tactical state that was just produced — the phase decides
        // WHETHER to commit bodies forward, the plan decides WHO and
        // WHERE. Without this second half every player derives his own
        // destination from the same geometry and they all arrive at the
        // same one; see `teamplay::attack`.
        let attack_inputs = AttackRefreshInputs {
            field,
            home_team_id: context.field_home_team_id,
            away_team_id: context.field_away_team_id,
            home_attacking: context.tactical_home.wants_bodies_forward(),
            away_attacking: context.tactical_away.wants_bodies_forward(),
            home_in_possession: context.tactical_home.in_possession,
            away_in_possession: context.tactical_away.in_possession,
            home_rest_defence: context.tactical_home.rest_defense_count,
            away_rest_defence: context.tactical_away.rest_defense_count,
        };
        AttackPlan::refresh(
            &mut context.attack_home,
            &mut context.attack_away,
            &attack_inputs,
        );

        // The defensive half of the same idea. Assigned on the same
        // cadence so the two plans always describe the same instant, and
        // unconditionally on phase — a side without the ball is defending
        // whatever the tactical phase says, and the duties are what stop
        // the back line computing its position from the ball alone.
        let defence_inputs = DefenceRefreshInputs {
            field,
            possession_holder: possession.holder(),
            home_team_id: context.field_home_team_id,
            away_team_id: context.field_away_team_id,
            // The voice behind each back line — see `KeeperVoice`. Read
            // off the aggregates rather than recomputed here so the
            // organising zone and `ShapeDiscipline::line_band` can never
            // disagree about how loud the same keeper is.
            home_keeper_voice: context.home_skill_aggregates.keeper_voice,
            away_keeper_voice: context.away_skill_aggregates.keeper_voice,
        };
        DefensivePlan::refresh(
            &mut context.defence_home,
            &mut context.defence_away,
            &defence_inputs,
        );
        let tick = context.current_tick();
        context.defence_home.call_line_step(
            field,
            context.field_home_team_id,
            context.tactical_familiarity_home.score,
            context.home_skill_aggregates.keeper_voice,
            tick,
        );
        context.defence_away.call_line_step(
            field,
            context.field_away_team_id,
            context.tactical_familiarity_away.score,
            context.away_skill_aggregates.keeper_voice,
            tick,
        );
        context.record_plans();

        // …and the positional layer under all three. The plans above name
        // the players involved with the ball; this gives EVERY player a
        // live place to be, so the seven or eight who aren't involved stop
        // steering at a kickoff dot they were drawn on before the match.
        // Runs last because it reads the tactical state the first refresh
        // produced (line height, compactness, width, press intensity).
        let shape_inputs = ShapeRefreshInputs {
            field,
            home_team_id: context.field_home_team_id,
            away_team_id: context.field_away_team_id,
            home_tactical: &context.tactical_home,
            away_tactical: &context.tactical_away,
            tick_interval,
        };
        TeamShape::refresh(
            &mut context.shape_home,
            &mut context.shape_away,
            &shape_inputs,
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
        context.coach_home.note_line(DefensivePlan::line_broken(
            field,
            context.field_home_team_id,
        ));
        context.coach_away.note_line(DefensivePlan::line_broken(
            field,
            context.field_away_team_id,
        ));
        context.coach_home.cum_ticks = context.coach_home.cum_ticks.saturating_add(tick_interval);
        context.coach_away.cum_ticks = context.coach_away.cum_ticks.saturating_add(tick_interval);
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
