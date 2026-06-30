use super::*;
use super::phase_prof::PhaseProf;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::player::events::players::FoulResolver;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;
use std::time::Instant;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
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
        // Keep this public single-tick wrapper self-contained — the
        // play_inner loop now gates position recording with a cursor
        // (`next_position_record_ms`) for efficiency, but external
        // callers of `game_tick` still expect each call to emit a
        // position sample when the timestamp is on the 30 ms cadence.
        Self::write_match_positions(field, context.total_match_time, match_data);
    }

    /// Light tick: full ball logic (physics, ownership, goals) but players only move.
    pub(super) fn game_tick_light(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
        tick_ctx: &mut GameTickContext,
        events: &mut EventCollection,
    ) {
        events.clear();

        let prof_t = PhaseProf::enabled().then(Instant::now);

        field.ball.update_light(context, &field.players, events);
        Self::apply_pending_set_piece_teleport(field);
        Self::apply_pending_save_credit(field);

        // Shot-flight GK reactivity: normally light ticks skip player
        // AI to save CPU, but during a shot the keeper needs continuous
        // decisions to close on the intercept line. Run just the two
        // goalkeepers (cheap, ~2 of 22 players) when a shot is in
        // flight. Refresh the *existing* tick_ctx in place instead of
        // allocating a fresh GameTickContext (grid+space buffers) every
        // light tick during the shot window.
        if field.ball.cached_shot_target.is_some() {
            tick_ctx.update_for_goalkeeper_shot(field);
            Self::play_goalkeepers(field, context, tick_ctx, events);
        }

        // Skip sent-off players: they've been stashed at (-500, -500). A
        // boundary clamp here would drag them to (0, 0) — the pitch's
        // top-left corner — which then gets recorded as a ghost sample
        // by `write_match_positions`.
        //
        // Light ticks advance position from the velocity the last AI tick
        // set, but deliberately do NOT touch `in_state_time`: state
        // timeouts and the fatigue curve are calibrated in AI ticks (full
        // `game_tick_inner` passes), and the state machine only runs on
        // those. Advancing the timer here would double its rate relative
        // to AI decisions and halve every state timeout — a calibration
        // change, not a graph fix. See `MatchPlayer::in_state_time`.
        for player in field.players.iter_mut().filter(|p| !p.is_sent_off) {
            player.check_boundary_collision(context);
            player.move_to();
        }

        if events.has_events() {
            EventDispatcher::dispatch(events, field, context, match_data, true);
            handle_goal_reset(field, context);
        }

        if let Some(t) = prof_t {
            PhaseProf::add(PhaseProf::P_LIGHT, t.elapsed().as_nanos() as u64);
        }
    }

    pub(super) fn game_tick_inner(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
        tick_ctx: &mut GameTickContext,
        events: &mut EventCollection,
    ) {
        let prof_on = PhaseProf::enabled();

        let t = prof_on.then(Instant::now);
        tick_ctx.update(field);
        if let Some(t) = t {
            PhaseProf::add(PhaseProf::P_TICKCTX, t.elapsed().as_nanos() as u64);
        }

        events.clear();

        let t = prof_on.then(Instant::now);
        Self::play_ball(field, context, tick_ctx, events);
        Self::apply_pending_set_piece_teleport(field);
        Self::apply_pending_save_credit(field);
        Self::resolve_corner_contest(field, context);
        // Resolve any deferred-foul / advantage state. Cheap (one
        // Option read in the dominant no-advantage case) so we run it
        // every full tick rather than waiting for the next event.
        FoulResolver::tick_advantage(field, context);
        // Ownership may have changed inside play_ball (new claim, pass
        // target receive, etc.). Refresh the ball view so player state
        // dispatch sees the current owner — without this, the
        // TakeBall force-override fires for a player who already has
        // the ball.
        tick_ctx.refresh_ball(field);
        if let Some(t) = t {
            PhaseProf::add(PhaseProf::P_BALL, t.elapsed().as_nanos() as u64);
        }

        let t = prof_on.then(Instant::now);
        Self::play_players(field, context, tick_ctx, events);
        if let Some(t) = t {
            PhaseProf::add(PhaseProf::P_PLAYERS, t.elapsed().as_nanos() as u64);
        }

        let t = prof_on.then(Instant::now);
        EventDispatcher::dispatch(events, field, context, match_data, true);
        handle_goal_reset(field, context);
        if let Some(t) = t {
            PhaseProf::add(PhaseProf::P_DISPATCH, t.elapsed().as_nanos() as u64);
        }
    }

    /// Corner kicks and goal kicks rewrite ball ownership inside `ball.update`,
    /// but ball.rs only has `&[MatchPlayer]` — it can't teleport the designated
    /// taker to the ball. Instead it stashes the teleport intent on the Ball;
    /// we drain it here, now that we have `&mut field.players`. Without this,
    /// the ball sits at the corner flag / goal area with ownership assigned
    /// to a player 30-200 units away, and `move_to`'s 15-unit distance check
    /// nulls ownership on the very next tick — ball stalls for seconds.
    pub(super) fn apply_pending_set_piece_teleport(field: &mut MatchField) {
        if let Some((player_id, ball_pos)) = field.ball.pending_set_piece_teleport.take() {
            if let Some(idx) = field.player_index(player_id) {
                let p = &mut field.players[idx];
                p.position = ball_pos;
                p.velocity = Vector3::zeros();
                p.in_state_time = 0;
            }
        }

        // Corner dead-ball set-up: teleport the pushed-up centre-backs
        // into the box so they can attack the delivery (see
        // `Ball::pending_corner_teleports` — there's no stoppage in the
        // sim for them to walk up during, and they can't run the length
        // of the pitch inside the cross window).
        if !field.ball.pending_corner_teleports.is_empty() {
            let teleports = std::mem::take(&mut field.ball.pending_corner_teleports);
            for (player_id, pos) in teleports {
                if let Some(idx) = field.player_index(player_id) {
                    let p = &mut field.players[idx];
                    p.position = pos;
                    p.velocity = Vector3::zeros();
                    // Force the AttackingCorner state directly — the CB may
                    // have been in any defensive state when the corner was
                    // won, and not all of them carry the entry hook. This
                    // guarantees they attack the delivery. `transition_to`
                    // also resets in_state_time so the run starts at entry.
                    p.transition_to(
                        PlayerState::Defender(DefenderState::AttackingCorner),
                        TransitionSource::SetPiece,
                    );
                }
            }
        }
    }

    /// Discrete corner aerial contest — fires once, the instant the corner
    /// cross is airborne. A played-out lofted corner can't thread the
    /// congested box to the pushed-up centre-back: the cross is always
    /// claimed/cleared mid-flight (`CB header chances` stayed 0 through
    /// every piecemeal GK / defender-duel fix). So we resolve ONE
    /// skill-weighted aerial contest — the best attacking header (a
    /// pushed-up CB or a forward) vs the defending line + GK command of
    /// area — and, if the attacker wins, drop the ball onto their head.
    /// Their existing heading state then strikes it on goal through the
    /// NORMAL shot/save pipeline, so the goal / shot / xG / save stats all
    /// credit correctly (no bespoke scoring path). The win chance is tuned
    /// (~0.30, modulated by the aerial mismatch and the keeper) so that —
    /// carried by a corner header's ~0.10-0.14 xG in the shot pipeline —
    /// only ~3-4% of corners end in a goal (real ≈ 3%), giving defenders
    /// their realistic set-piece share without inflating totals.
    pub(super) fn resolve_corner_contest(field: &mut MatchField, context: &mut MatchContext) {
        use crate::r#match::PassOriginRestart;
        use nalgebra::Vector3;

        let ball = &field.ball;
        if ball.corner_contest_resolved || ball.pass_origin_restart != PassOriginRestart::Corner {
            return;
        }
        // [diag] reached with an armed Corner origin.
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::CORNER_CONTEST_SEEN.fetch_add(1, Ordering::Relaxed);
        // Only once the cross has actually left the taker and is airborne
        // (not the dead-ball set-up while the taker still holds it, and not
        // a short ground corner played along the floor).
        if ball.current_owner.is_some() {
            return;
        }
        // [diag] cross has left the taker (loose / in flight).
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::CORNER_CONTEST_FIRED.fetch_add(1, Ordering::Relaxed);
        if ball.position.z < 2.0 {
            return;
        }

        let minute = (context.total_match_time / 60_000) as u32;

        // The goal under attack is the one the corner is nearest to.
        let gl = context.goal_positions.left;
        let gr = context.goal_positions.right;
        let ball_pos = ball.position;
        let attacked_goal = if (ball_pos - gl).magnitude() < (ball_pos - gr).magnitude() {
            gl
        } else {
            gr
        };

        // Attacking team = the cross taker's team.
        let taker = ball.previous_owner.or(ball.current_owner);
        let att_team = match taker
            .and_then(|id| field.players.iter().find(|p| p.id == id))
            .map(|p| p.team_id)
        {
            Some(t) => t,
            None => {
                field.ball.corner_contest_resolved = true;
                return;
            }
        };

        // Best attacking header, best defending header, and GK command of
        // area — among the players inside the box (≈135u of the goal).
        let mut best_att: Option<(usize, f32)> = None;
        let mut best_def_score = 0.40_f32;
        let mut gk_command = 0.35_f32;
        for (i, p) in field.players.iter().enumerate() {
            if (p.position - attacked_goal).magnitude() > 135.0 {
                continue;
            }
            let is_gk = p.tactical_position.current_position.is_goalkeeper();
            if p.team_id == att_team {
                if is_gk {
                    continue;
                }
                let s = sc::aerial_outfield_attacker(p, minute);
                if best_att.map_or(true, |(_, bs)| s > bs) {
                    best_att = Some((i, s));
                }
            } else if is_gk {
                gk_command = (p.skills.goalkeeping.command_of_area * 0.6
                    + p.skills.goalkeeping.aerial_reach * 0.4)
                    / 20.0;
            } else {
                let s = sc::aerial_outfield_defender(p, minute);
                if s > best_def_score {
                    best_def_score = s;
                }
            }
        }

        let (att_idx, att_score) = match best_att {
            Some(v) => v,
            None => {
                field.ball.corner_contest_resolved = true;
                return;
            }
        };

        let att_win =
            (0.36 + (att_score - best_def_score) * 0.50 - gk_command * 0.18).clamp(0.10, 0.62);

        if context.rng.bernoulli(att_win) {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::CORNER_CONTEST_WON.fetch_add(1, Ordering::Relaxed);
            // Attacker wins: drop the ball just behind them at head height,
            // moving goalward, so it reads as an incoming header to their
            // state (the CB's AttackingCorner, or a forward's run→heading).
            // Loose so they head it; keep the Corner origin so the CB stays
            // in AttackingCorner through the strike.
            //
            // Drop kinematics = apex-of-flick hang time. The previous
            // (z 2.2, vz −1.0, 4.0 u/tick drift) fell through the entire
            // heading band [1.4, 2.5] in ONE tick and drifted out of
            // 6u header reach almost as fast — so only a CB already in
            // AttackingCorner (whose same-tick path runs right after
            // this resolver) ever struck it; a FORWARD winner spent the
            // only valid tick transitioning Running→Heading and found
            // the ball below threshold, and the loose ball was then
            // vacuumed by the intercept gate (z ≤ 2.5). Real contested
            // headers hang ~0.3-0.4 s at the apex: z 2.55 (one tick
            // above the intercept window) with vz −0.35 and a modest
            // 1.8 u/tick goalward drift keeps the ball in the heading
            // band and within reach for ~3 ticks — enough for ANY
            // winner's state machine to strike, which is what the
            // contest already decided should happen.
            let winner_pos = field.players[att_idx].position;
            let to_goal = attacked_goal - winner_pos;
            let dir = if to_goal.magnitude() > 0.01 {
                to_goal.normalize()
            } else {
                Vector3::new(1.0, 0.0, 0.0)
            };
            let b = &mut field.ball;
            b.position =
                Vector3::new(winner_pos.x - dir.x * 2.0, winner_pos.y - dir.y * 2.0, 2.55);
            b.velocity = Vector3::new(dir.x * 1.8, dir.y * 1.8, -0.35);
            b.current_owner = None;
            b.previous_owner = taker;
            b.flags.in_flight_state = 1;
        }
        // Otherwise the cross plays out — the keeper claims or a defender
        // clears (the realistic majority outcome).

        // The contest IS the resolution of the delivery — clear the
        // stale cross-target so the original aim point (often the OTHER
        // pushed-up CB) can't auto-claim the dropped ball through the
        // 100u receiver-priority radius. Before this, won headers were
        // routinely converted into a different player's chest-trap →
        // slow foot-shot, and "lost" contests were caught by the
        // attacking CB instead of playing out as GK claims/clearances.
        field.ball.pass_target_player_id = None;
        field.ball.clear_pending_pass_metadata();

        // Persist this corner's routine + estimated xG into the team's
        // history so `pick_corner_routine` can vary future deliveries.
        // The xG used here is a rough estimate (att_win × generic
        // header xG); the precise xG is computed downstream when the
        // header actually fires through the shot pipeline. The history
        // only needs the *flavour* of "did this routine produce a
        // chance" to gate repeats, so an approximate value is fine.
        if let Some(routine) = field.ball.pending_corner_routine.take() {
            let estimated_xg = att_win * 0.12; // ~0.12 header xG ceiling × win prob
            let is_home_attacking = att_team == context.field_home_team_id;
            context
                .set_piece_history
                .record_corner(is_home_attacking, routine, estimated_xg);
        }

        field.ball.corner_contest_resolved = true;
    }

    /// Consume `Ball::pending_save_credit` left behind by the physics
    /// save (`try_save_shot`). When the keeper actually changed ball
    /// state mid-flight (catch, safe parry, dangerous parry), this fires
    /// the save stat for the keeper and the on-target stat for the
    /// shooter — matching the events the GK state machine would have
    /// emitted if the physics save hadn't pre-empted it.
    pub(super) fn apply_pending_save_credit(field: &mut MatchField) {
        let Some((keeper_id, shooter_id)) = field.ball.pending_save_credit.take() else {
            return;
        };
        // One pass over the 22-player list resolves both ids. The team-
        // mismatch guard is defence in depth against any accidental
        // same-team shooter — deflections through the save handler
        // should already have been filtered upstream.
        let Some((keeper_idx, shooter_idx)) = field.two_player_indices(keeper_id, shooter_id)
        else {
            return;
        };
        let keeper_team = field.players[keeper_idx].team_id;
        let shooter_team = field.players[shooter_idx].team_id;
        if keeper_team == shooter_team {
            return;
        }
        let shot_xg = field.ball.last_shot_xg;
        {
            let gk = &mut field.players[keeper_idx];
            gk.statistics.saves += 1;
            gk.statistics.shots_faced += 1;
            // The GK denied a shot worth `shot_xg` xG — full credit goes
            // to xG prevented. Saves an above-baseline keeper from being
            // capped by the synthetic-proxy fallback in the rating helper.
            if shot_xg > 0.0 {
                gk.statistics.record_xg_prevented(shot_xg);
            }
        }
        field.players[shooter_idx].memory.credit_shot_on_target();
        // Shot has resolved (saved). Drop the metadata so any
        // subsequent goal / save event can't double-credit.
        field.ball.clear_shot_metadata();
        field.ball.pending_error_to_shot_player_id = None;
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
}
