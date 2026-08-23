//! **The per-tick driver** — the entry point every simulated millisecond
//! goes through, and the two tick bodies it picks between.
//!
//! `game_tick_light` is the cheap path: the ball integrates, the pending
//! set-piece and save-credit work is applied, and player AI is skipped
//! except for the keeper during a shot. `game_tick_inner` is the full
//! pass: the tick context refreshes, the ball runs, the engine's own
//! resolutions in [`resolve`](crate::r#match::engine::engine::resolve) fire,
//! all 22 players think, and the event dispatcher drains.
//!
//! Everything the driver calls between those phases lives in a sibling
//! group; this file is the running order, not the work.

#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::teleport as tc;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::engine::diagnostics::teleport_probe::TeleportProbe;
use crate::r#match::engine::engine::phase_prof::PhaseProf;
use crate::r#match::engine::engine::*;
use crate::r#match::engine::player::events::players::FoulResolver;
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
    pub(in crate::r#match::engine::engine) fn game_tick_light(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
        tick_ctx: &mut GameTickContext,
        events: &mut EventCollection,
    ) {
        events.clear();

        let prof_t = PhaseProf::enabled().then(Instant::now);

        #[cfg(feature = "match-logs")]
        let mut relocation = TeleportProbe::open(field);

        field.ball.update_light(context, &field.players, events);
        #[cfg(feature = "match-logs")]
        relocation.ball_update(field, tc::STAGE_L_BALL_UPDATE);
        Self::apply_pending_set_piece_teleport(field);
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_L_SET_PIECE);
        // A corner can become takeable on a light tick too, and a census
        // that only watched the full ones lost half its sample.
        #[cfg(feature = "match-logs")]
        Self::note_corner_setup_box_if_taken(field);
        Self::apply_pending_save_credit(field);
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_L_SAVE_CREDIT);

        // Shot-flight GK reactivity: normally light ticks skip player
        // AI to save CPU, but during a shot the keeper needs continuous
        // decisions to close on the intercept line. Run just the two
        // goalkeepers (cheap, ~2 of 22 players) when a shot is in
        // flight. Refresh the *existing* tick_ctx in place instead of
        // allocating a fresh GameTickContext (grid+space buffers) every
        // light tick during the shot window.
        let shot_in_flight = field.ball.cached_shot_target.is_some();
        if shot_in_flight {
            tick_ctx.update_for_goalkeeper_shot(field, &context.players);
            Self::play_goalkeepers(field, context, tick_ctx, events);
        }
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_L_GOALKEEPERS);

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
        // Ball direction stability, sampled on the same cadence as the
        // players. Every chase state aims at a point derived from the
        // ball, so this is the control for the per-state reversal table:
        // a chaser tracking a jittery ball and a chaser with a steering
        // bug produce identical numbers without it.
        #[cfg(feature = "match-logs")]
        let (ball_pos, ball_vel) = (field.ball.position, field.ball.velocity);
        #[cfg(feature = "match-logs")]
        {
            crate::r#match::player::motion_diag::note_ball(
                ball_vel,
                tick_ctx.positions.ball.velocity,
            );
        }

        // Read before the loop borrows `field.players` — the man fetching a
        // ball that has gone out of play is the one player allowed off the
        // pitch. See `MatchPlayer::check_boundary_collision`.
        let restart_taker = field.ball.awaiting_restart.map(|r| r.taker_id);

        for player in field.players.iter_mut().filter(|p| !p.is_sent_off) {
            // A keeper the shot branch above just ran has ALREADY been moved:
            // `MatchPlayer::update` ends in `move_to` + the boundary clamp.
            // Moving him again here integrated his velocity twice on every
            // light tick of every shot — so for the one passage of play the
            // branch exists to sharpen, the two goalkeepers covered double
            // the ground their own speed limit allows. Harmless-looking while
            // the vertical axis was dead; with a real leap it also applied
            // gravity twice and collapsed the arc.
            if shot_in_flight
                && player.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
            {
                continue;
            }
            // Move first, then clamp — see `MatchPlayer::update`.
            player.move_to();
            player.check_boundary_collision(context, restart_taker);
            #[cfg(feature = "match-logs")]
            player.trace_motion(context, ball_pos, ball_vel);
        }

        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_L_PLAYER_MOVE);

        if events.has_events() {
            EventDispatcher::dispatch(events, field, context, match_data, true);
            #[cfg(feature = "match-logs")]
            relocation.at(field, tc::STAGE_L_DISPATCH);
            // Before `handle_goal_reset`, which is what clears the flag. A
            // goals-only recording keeps the seconds either side of this
            // instant and drops the rest of the match (`mark_goal`).
            if Self::is_a_goal_worth_keeping(field) {
                match_data.mark_goal(context.total_match_time);
            }
            handle_goal_reset(field, context);
            #[cfg(feature = "match-logs")]
            relocation.at(field, tc::STAGE_L_GOAL_RESET);
            // Dispatch is where free kicks, penalties and offsides are
            // awarded, and each stages a teleport — see the full tick.
            Self::apply_pending_set_piece_teleport(field);
            #[cfg(feature = "match-logs")]
            relocation.at(field, tc::STAGE_L_SET_PIECE2);
        }

        if let Some(t) = prof_t {
            PhaseProf::add(PhaseProf::P_LIGHT, t.elapsed().as_nanos() as u64);
        }
    }

    pub(in crate::r#match::engine::engine) fn game_tick_inner(
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
        tick_ctx: &mut GameTickContext,
        events: &mut EventCollection,
    ) {
        let prof_on = PhaseProf::enabled();

        let t = prof_on.then(Instant::now);
        tick_ctx.update(field, &context.players);
        if let Some(t) = t {
            PhaseProf::add(PhaseProf::P_TICKCTX, t.elapsed().as_nanos() as u64);
        }

        events.clear();

        // Possession geography sample: where on the pitch is the ball
        // actually being held? Ground truth behind the shot mix.
        #[cfg(feature = "match-logs")]
        if let Some(owner_id) = field.ball.current_owner {
            if let Some(owner) = field.players.iter().find(|p| p.id == owner_id) {
                let goal_x = match owner.side {
                    Some(crate::r#match::PlayerSide::Left) => context.field_size.width as f32,
                    _ => 0.0,
                };
                let dx = owner.position.x - goal_x;
                let dy = owner.position.y - context.field_size.height as f32 / 2.0;
                let d = (dx * dx + dy * dy).sqrt();
                let band = crate::r#match::player::strategies::players::ops::forward_shot_decision::time_band_diag::band_for_distance(d);
                crate::r#match::player::strategies::players::ops::forward_shot_decision::time_band_diag::POSSESSION_TICKS_BY_DIST[band]
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        // Whole-tick relocation census. Every checkpoint below closes the
        // window on the function named above it; nothing between them
        // integrates the ball, so any movement they book is a write. See
        // [`teleport`](crate::r#match::engine::ball::ball::teleport).
        #[cfg(feature = "match-logs")]
        let mut relocation = TeleportProbe::open(field);

        let t = prof_on.then(Instant::now);
        Self::play_ball(field, context, tick_ctx, events);
        #[cfg(feature = "match-logs")]
        relocation.ball_update(field, tc::STAGE_BALL_UPDATE);
        Self::apply_pending_set_piece_teleport(field);
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_SET_PIECE);
        #[cfg(feature = "match-logs")]
        Self::note_corner_setup_box_if_taken(field);
        Self::apply_pending_save_credit(field);
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_SAVE_CREDIT);
        Self::resolve_corner_contest(field, context);
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_CORNER_CONTEST);
        Self::resolve_cross_contest(field, context);
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_CROSS_CONTEST);
        // Resolve any deferred-foul / advantage state. Cheap (one
        // Option read in the dominant no-advantage case) so we run it
        // every full tick rather than waiting for the next event.
        FoulResolver::tick_advantage(field, context);
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_FOUL_ADVANTAGE);
        // Ownership may have changed inside play_ball (new claim, pass
        // target receive, etc.). Refresh the ball view so player state
        // dispatch sees the current owner — without this, the
        // TakeBall force-override fires for a player who already has
        // the ball.
        tick_ctx.refresh_ball(field);
        if let Some(t) = t {
            PhaseProf::add(PhaseProf::P_BALL, t.elapsed().as_nanos() as u64);
        }

        #[cfg(feature = "match-logs")]
        Self::sample_defensive_shape(field, context);
        #[cfg(feature = "match-logs")]
        Self::sample_duel_gates(field, context);
        #[cfg(feature = "match-logs")]
        Self::sample_loose_chase(field);
        #[cfg(feature = "match-logs")]
        Self::sample_box_delivery(field, context);

        let t = prof_on.then(Instant::now);
        Self::play_players(field, context, tick_ctx, events);
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_PLAY_PLAYERS);
        if let Some(t) = t {
            PhaseProf::add(PhaseProf::P_PLAYERS, t.elapsed().as_nanos() as u64);
        }

        let t = prof_on.then(Instant::now);
        EventDispatcher::dispatch(events, field, context, match_data, true);
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_DISPATCH);
        // See the light tick: this has to read the flag before
        // `handle_goal_reset` takes it down.
        if Self::is_a_goal_worth_keeping(field) {
            match_data.mark_goal(context.total_match_time);
        }
        handle_goal_reset(field, context);
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_GOAL_RESET);
        // ⚠ DRAIN AFTER DISPATCH TOO, NOT ONLY AFTER `play_ball`.
        //
        // The drain above catches the set pieces the BALL awards inside
        // `play_ball` — corners, goal kicks, throw-ins. But free kicks,
        // penalties and offsides are awarded by their event handlers, which
        // run here, after that drain has already taken `None`. Their
        // teleport therefore sat staged until the next tick — and the next
        // tick starts with `ball.update`, whose `move_to` runs first, finds
        // the taker 30-200u from the spot and nulls the ownership. The ball
        // is dead by then (a restart placed it), so what is left is a ball
        // sitting motionless with no owner: `OWNER_TOO_FAR`.
        //
        // Measured before this: 47.3 drops a match, of which 71% had the
        // owner beyond 30u and **98% had the ball already stopped**, with
        // 0% in a keeper's gloves and 0% during a shot — a population that
        // is nothing but restarts. This function's own doc-comment has
        // always described the failure it exists to prevent; it was simply
        // never called after the phase that stages most of them.
        Self::apply_pending_set_piece_teleport(field);
        #[cfg(feature = "match-logs")]
        relocation.at(field, tc::STAGE_SET_PIECE_POST);
        if let Some(t) = t {
            PhaseProf::add(PhaseProf::P_DISPATCH, t.elapsed().as_nanos() as u64);
        }
    }

    /// Is there a goal here for the highlight recording to keep?
    ///
    /// `Ball::goal_scored` means the ball crossed the line and play must be
    /// restarted — which is true of one case that is NOT a goal anybody can
    /// watch: a ball that goes in off nobody, with no owner and no previous
    /// owner to credit. `check_goal` emits no `Goal` event for it, so no
    /// scorer reaches the scoreline, but it still raises the flag because
    /// the restart is real.
    ///
    /// A goals-only recording keyed on the flag alone therefore cut a
    /// ten-second clip around a goal the match does not have — caught by
    /// `goal_clip_recording_tests` as two segments for one goal. `in_net` is
    /// the honest discriminator: it is set by `enter_net` on exactly the
    /// path that credits a scorer, and cleared by the `reset()` the
    /// uncreditable case falls through to.
    fn is_a_goal_worth_keeping(field: &MatchField) -> bool {
        field.ball.goal_scored && field.ball.in_net.is_some()
    }
}
