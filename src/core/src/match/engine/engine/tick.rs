use super::phase_prof::PhaseProf;
use super::*;
use crate::PlayerPositionType;
use crate::r#match::PassOriginRestart;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::engine::ball::ball::CornerWalk;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::teleport as tc;
use crate::r#match::engine::ball::ball::{AerialDelivery, AerialOutcome};
use crate::r#match::engine::corner_shape::{CornerRole, CornerShape};
use crate::r#match::engine::player::events::players::FoulResolver;
use crate::r#match::engine::set_pieces::{CORNER_DELIVERY_REFERENCE, CornerRoutine};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::passing::CrossType;
use crate::r#match::player::transition::TransitionSource;
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::{CrossDiag, SetPieceDiag};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;
use std::time::Instant;

/// Carries the ball's position from one census checkpoint to the next
/// through a tick.
///
/// It exists so the call sites in the tick loop read as one line each.
/// Nothing between two checkpoints integrates the ball — see
/// [`teleport`](crate::r#match::engine::ball::ball::teleport) — so the
/// probe holds the position and the velocity either side of `Ball::update`
/// and needs nothing else.
#[cfg(feature = "match-logs")]
struct TeleportProbe {
    pos: Vector3<f32>,
    entry_velocity: Vector3<f32>,
    dead: bool,
}

#[cfg(feature = "match-logs")]
impl TeleportProbe {
    fn open(field: &MatchField) -> Self {
        tc::TeleportCensus::note_tick();
        Self {
            pos: field.ball.position,
            entry_velocity: field.ball.velocity,
            // Sampled at the top of the tick: a relocation on a ball that
            // was already dead when the tick began is a dead-ball leak,
            // whereas a restart AWARDED during the tick legitimately
            // places one. Reading the flag afterwards would confuse them.
            dead: field.ball.awaiting_restart.is_some(),
        }
    }

    /// A checkpoint after `Ball::update`, where no travel is explained.
    fn at(&mut self, field: &MatchField, stage: usize) {
        self.pos = tc::TeleportCensus::checkpoint(stage, self.pos, field.ball.position, self.dead);
    }

    /// The ball's own pass, whose travel its velocity does explain.
    fn ball_update(&mut self, field: &MatchField, stage: usize) {
        self.pos = tc::TeleportCensus::note_ball_update(
            stage,
            self.pos,
            field.ball.position,
            self.entry_velocity,
            field.ball.velocity,
            self.dead,
        );
    }
}

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

    pub(super) fn game_tick_inner(
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
                #[cfg(feature = "match-logs")]
                {
                    tc::PlayerTeleportCensus::note_firing(tc::PSITE_SET_PIECE);
                    tc::PlayerTeleportCensus::note(tc::PSITE_SET_PIECE, p.position, ball_pos);
                }
                p.position = ball_pos;
                p.velocity = Vector3::zeros();
                p.in_state_time = 0;
            }
        }

        // A delivery has reached the man who won the duel for it: put him
        // into his role's heading state so he strikes it.
        //
        // Drained here, with `&mut field.players` in hand, because the
        // ball decides the arrival and `Ball::update` holds the players
        // immutably — the same reason as the set-piece teleport above.
        // See `AerialDelivery::force_heading` for why this happens on
        // arrival rather than at the strike.
        if let Some(player_id) = field.ball.pending_aerial_strike.take() {
            if let Some(idx) = field.player_index(player_id) {
                let p = &mut field.players[idx];
                let next = if p.tactical_position.current_position.is_forward() {
                    PlayerState::Forward(ForwardState::Heading)
                } else if p.tactical_position.current_position.is_midfielder() {
                    PlayerState::Midfielder(MidfielderState::Heading)
                } else {
                    PlayerState::Defender(DefenderState::Heading)
                };
                // `AttackingCorner` owns its own header — it has a
                // reach and a shot path of its own, and it is what the
                // corner contest's winners are usually in. Overriding it
                // here would take the corner's own heading route away.
                if p.state != PlayerState::Defender(DefenderState::AttackingCorner) {
                    p.transition_to(next, TransitionSource::EventHandler);
                }
            }
        }

        // The corner taker's own station: where he has to stand to take the
        // kick, while he walks the ball there. See `Ball::pending_restart_station`.
        if let Some((player_id, station)) = field.ball.pending_restart_station.take() {
            if let Some(idx) = field.player_index(player_id) {
                field.players[idx].set_piece_station = Some(station);
            }
        }

        // Corner dead-ball set-up: put both sides into the shape a corner
        // is actually played in (see `Ball::pending_corner_teleports` and
        // `CornerShape`).
        //
        // ⚠ **The positions are NOT written any more.** The comment this
        // replaces said there was no stoppage in the sim to walk into the
        // shape during, and nobody could cover the ground inside the 50 ms
        // between the award and the cross. Both were true when the corner
        // was awarded, the ball placed on the arc and the taker teleported
        // onto it, all on one tick. The corner now waits for its taker to
        // go and fetch the ball and carry it to the flag — several seconds
        // — so the stoppage exists, and the twenty walk into the shape
        // under `CornerHold` exactly as they do in the thirty seconds
        // before a real one. Writing the positions on top of that is the
        // last of the corner's three teleports.
        if !field.ball.pending_corner_teleports.is_empty() {
            #[cfg(feature = "match-logs")]
            if !CornerWalk::armed() {
                tc::PlayerTeleportCensus::note_firing(tc::PSITE_CORNER_STATION);
            }
            let stations = std::mem::take(&mut field.ball.pending_corner_teleports);
            for station in stations {
                if let Some(idx) = field.player_index(station.player_id) {
                    let p = &mut field.players[idx];
                    if !CornerWalk::armed() {
                        #[cfg(feature = "match-logs")]
                        tc::PlayerTeleportCensus::note(
                            tc::PSITE_CORNER_STATION,
                            p.position,
                            station.position,
                        );
                        // `OF_CORNER_WALK=off`: written, not walked.
                        p.position = station.position;
                        p.velocity = Vector3::zeros();
                    }
                    if station.role == CornerRole::BoxAttacker {
                        // The pushed-up centre-back is the one station that
                        // forces a state instead of pinning a position: he
                        // may have been in any defensive state when the
                        // corner was won, and not all of them carry the
                        // entry hook, so this is what guarantees he attacks
                        // the delivery. `transition_to` resets in_state_time
                        // so the run starts at entry.
                        //
                        // Deliberately NO station for him — `AttackingCorner`
                        // already owns where he stands AND when he leaves it
                        // to attack the ball, and pinning him too would fight
                        // that state's own attack/hold blend.
                        p.transition_to(
                            PlayerState::Defender(DefenderState::AttackingCorner),
                            TransitionSource::SetPiece,
                        );
                    } else {
                        // Everyone else walks to his station and stays on
                        // it for the life of the corner. Without the pin
                        // the shape unravels inside a second: a midfielder
                        // heading for his own six-yard box reads the next
                        // tick as ordinary open play and turns round.
                        p.set_piece_station = Some(station.position);
                    }
                }
            }
        }

        Self::clear_expired_corner_stations(field);
    }

    /// Box census at the instant the shape goes up — one sample per
    /// corner, at a moment that cannot be anything but a corner.
    ///
    /// Its sibling in `resolve_corner_contest` samples the same thing when
    /// the delivery is airborne, which also proves the shape SURVIVED to
    /// the cross — but that resolver fires on the first airborne,
    /// ownerless tick with a live `Corner` origin, and the origin outlives
    /// the set piece. A ball hooked up two seconds later, by then at the
    /// other end, is sampled as if it were the delivery. That is what puts
    /// the occasional 1-defender reading in the delivery census, and it is
    /// a measurement artefact rather than a deserted goalmouth. Reading
    /// both is what tells the two apart.
    /// The box census, on the tick the kick becomes takeable and only
    /// then.
    ///
    /// ⚠ **Called from the tick loop, not from the drain.** The drain runs
    /// twice per full tick — once after `play_ball` and once after
    /// dispatch — so a census inside it counted every corner twice; and it
    /// has to be reached from the LIGHT tick as well, where the ball is
    /// updated but the players only move, or half the sample is lost.
    ///
    /// It also has to fire at the KICK, not at the award. Those used to be
    /// the same tick. They are now several seconds apart with the walk-in
    /// in between, and read at the award it would report the open-play
    /// shape the corner was won from — which is the very thing
    /// `CornerShape` exists to replace — and report it as a success.
    #[cfg(feature = "match-logs")]
    fn note_corner_setup_box_if_taken(field: &MatchField) {
        if field
            .ball
            .corner_shape
            .is_some_and(|s| s.live_tick == Some(field.ball.current_tick_cached))
        {
            Self::note_corner_setup_box(field);
        }
    }

    #[cfg(feature = "match-logs")]
    fn note_corner_setup_box(field: &MatchField) {
        let Some(shape) = field.ball.corner_shape else {
            return;
        };
        let Some(taker) = field.players.iter().find(|p| p.id == shape.taker_id) else {
            return;
        };
        let attacking_side = taker.side;
        let field_height = field.size.height as f32;
        // The corner is taken at a flag, so the defended goal is the near one.
        let goal_x = if field.ball.position.x < field.size.width as f32 * 0.5 {
            0.0
        } else {
            field.size.width as f32
        };
        let (mut defenders, mut attackers) = (0u32, 0u32);
        for p in field.players.iter() {
            if p.is_sent_off
                || p.tactical_position.current_position.is_goalkeeper()
                || !CornerShape::is_in_penalty_area(p.position, goal_x, field_height)
            {
                continue;
            }
            if p.side == attacking_side {
                attackers += 1;
            } else {
                defenders += 1;
            }
        }
        SetPieceDiag::note_corner_setup_box(defenders, attackers);
        // ⚠ **Per-player probe, `OF_CORNER_PROBE=1`.** The aggregate says
        // how many are in the box; only this says WHY the ones that are
        // not are not — and the two answers need opposite fixes. It is
        // what solved the walked corner: every stationed man read
        // `st 3u him=IN station=IN` (standing on his station, arrived),
        // while the two pushed-up centre-backs read `NO-STATION
        // Defender(Covering)`. Nobody was slow; two men had left.
        //
        // Kept because the corner shape has three independent ways to
        // fail — a station outside the box, a man who never arrives, and
        // a man whose state walked him out of it — and the box count
        // cannot tell them apart.
        if std::env::var("OF_CORNER_PROBE").is_ok() {
            let mut line = String::new();
            for p in field.players.iter() {
                if p.side != attacking_side || p.tactical_position.current_position.is_goalkeeper()
                {
                    continue;
                }
                let inbox = CornerShape::is_in_penalty_area(p.position, goal_x, field_height);
                let where_he_is = if inbox { "IN" } else { "out" };
                match p.set_piece_station {
                    Some(s) => line.push_str(&format!(
                        " [{} st {:.0}u him={where_he_is} station={}]",
                        p.id,
                        (p.position - s).xy().magnitude(),
                        if CornerShape::is_in_penalty_area(s, goal_x, field_height) {
                            "IN"
                        } else {
                            "out"
                        }
                    )),
                    None => line.push_str(&format!(
                        " [{} NO-STATION {:?} {where_he_is}]",
                        p.id, p.state
                    )),
                }
            }
            println!("CORNERPROBE att_in_box={attackers}{line}");
        }
    }

    /// Release the corner shape once the corner is over.
    ///
    /// Three ways out, and each covers a case the others miss.
    ///
    /// **First contact** is the honest end of a set piece: the moment
    /// anybody heads, blocks, claims or clears the delivery, the corner is
    /// over and what follows is open play. The taker's own touch is
    /// stamped at the award, on the same tick the stations are armed, so
    /// any strictly later touch is somebody else on the ball.
    ///
    /// **The restart origin leaving `Corner`** is what a clean reception
    /// clears, and what every other corner-aware read in the engine keys
    /// off. Kept because it is the condition the rest of the corner code
    /// agrees on.
    ///
    /// **The deadline** is what stops the other two deadlocking. Both are
    /// events that require somebody to reach the ball, and the pin is what
    /// keeps everybody standing still — so a delivery cleared out of the
    /// box, or one that sails over everyone, satisfies neither and the pin
    /// never lifts. Measured before the deadline landed: a mean **held
    /// shape of about seven seconds per corner**, against a corner that is
    /// over in one or two.
    fn clear_expired_corner_stations(field: &mut MatchField) {
        let Some(shape) = field.ball.corner_shape else {
            // ⚠ **NO CORNER, SO NOBODY MAY HOLD A STATION — and this used
            // to be a bare `return`.**
            //
            // It was safe for exactly as long as the corner was the only
            // restart with a `take_from`: the carry leg writes
            // `pending_restart_station`, that is the sole producer of
            // `MatchPlayer::set_piece_station`, and this function was the
            // sole consumer. Every restart has a carry leg now
            // ([`RunOff`](crate::r#match::engine::ball::ball::RunOff)), so
            // a throw-in or a goal kick stamps a station and nothing on
            // the pitch ever takes it off again — not the half-time reset,
            // not the goal reset.
            //
            // It then lies dormant until the next CORNER, because
            // `CornerHold::apply` bails on `pass_origin_restart != Corner`
            // — a guard exactly the wrong way round for this. Measured:
            // the keeper carries a goal kick in from `(6, 199.8)`, keeps
            // that station, and on the next corner `hold_weight` returns
            // 1.0 for a man 270 u from the flag, replacing his velocity
            // outright and walking him 12 m off his line onto an hour-old
            // goal-kick spot while the cross comes in.
            //
            // Sweeping is the whole fix: a station exists only while a
            // corner shape is live, or for the one man carrying a dead
            // ball back to its spot right now.
            let carrier = field
                .ball
                .awaiting_restart
                .filter(|restart| restart.carrying)
                .map(|restart| restart.taker_id);
            for player in field.players.iter_mut() {
                if Some(player.id) != carrier {
                    player.set_piece_station = None;
                }
            }
            return;
        };
        // **Still being set up.** The taker is fetching the ball or
        // carrying it to the arc, so the kick has not been taken — and
        // neither of the two release conditions can be reached from here:
        // first contact needs somebody to touch a ball nobody may touch,
        // and the ball's last toucher is still whoever put it out, which
        // reads as "first contact" the moment the test is applied. The
        // shape holds for the whole walk-in, which is the only time the
        // players are actually walking into it. `AwaitedRestart` carries
        // its own timeout, so this cannot deadlock behind a taker who
        // never arrives.
        if field
            .ball
            .awaiting_restart
            .is_some_and(|r| r.origin == PassOriginRestart::Corner)
        {
            return;
        }
        let held = field
            .ball
            .current_tick_cached
            .saturating_sub(shape.armed_tick);
        // The taker is the only man who may touch the ball without ending
        // the set piece: the award stamps him as last toucher, and so does
        // his own delivery (a cross is a deliberate kick). Anybody else on
        // it is first contact.
        let only_the_taker_has_touched_it = field.ball.last_touch_player_id == Some(shape.taker_id);
        let corner_still_live = field.ball.pass_origin_restart == PassOriginRestart::Corner
            && only_the_taker_has_touched_it;
        if corner_still_live && held < Self::CORNER_SHAPE_MAX_TICKS {
            return;
        }
        // `corner_still_live` here means nothing ended it — the deadline
        // did. The harness prints that share so the ceiling stays visible
        // as a backstop rather than quietly becoming the rule.
        #[cfg(feature = "match-logs")]
        SetPieceDiag::note_corner_shape_held(held, corner_still_live);
        for p in field.players.iter_mut() {
            p.set_piece_station = None;
        }
        field.ball.corner_shape = None;
    }

    /// Longest a corner shape may pin anybody, in engine ticks (10 ms
    /// each), counted from the award.
    ///
    /// Sized off what a corner physically takes: the cross leaves the
    /// taker 50 ms after the award, the flight from the flag to the far
    /// post is 35 m at ~20 m/s, and the aerial contest resolves the
    /// instant the ball is airborne. Two and a half seconds covers all of
    /// it with room over, and nothing beyond it is still a corner —
    /// whatever is happening by then is open play with the restart flag
    /// left up.
    const CORNER_SHAPE_MAX_TICKS: u64 = 250;

    /// Apex of a corner delivery, in metres. A normal in-swinger: 5 m up
    /// puts about 1.7 s between the strike and the header, which is what
    /// a real one takes and comfortably inside
    /// [`CORNER_SHAPE_MAX_TICKS`](Self::CORNER_SHAPE_MAX_TICKS) so the
    /// set-piece shape holds for the whole flight.
    const CORNER_APEX: f32 = 5.0;

    /// Apex of an open-play cross, in metres. Shorter than a corner
    /// because it is played from further forward and has to beat a moving
    /// line rather than a set one.
    const CROSS_APEX: f32 = 4.0;

    /// How far short of the winner a corner is aimed, in units.
    const CORNER_DROP_BEHIND: f32 = 2.0;

    /// The same for an open-play cross. 1.2u (15 cm) sits inside every
    /// role's heading reach, including the midfielder's 2.0u, which the
    /// corner's own 2.0u sits exactly on the boundary of.
    const CROSS_DROP_BEHIND: f32 = 1.2;

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
        // A short corner and a cutback to the edge are played on the floor:
        // there is no ball into the box to attack, so the discrete aerial
        // contest must not fire and the move simply plays out as open play.
        //
        // Until the routine was wired through, EVERY corner resolved as an
        // aerial contest whatever routine had been chosen — which is why
        // `pick_corner_routine` could be called and its answer thrown away
        // without changing a single outcome.
        if matches!(
            ball.pending_corner_routine,
            Some(CornerRoutine::Short) | Some(CornerRoutine::EdgeCutback)
        ) {
            field.ball.corner_contest_resolved = true;
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
        // Who the defending header actually falls to, so the cleared-behind
        // branch can hook it from where he is standing rather than from the
        // corner flag the ball has not left yet.
        let mut best_def: Option<usize> = None;
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
                    best_def = Some(i);
                }
            }
        }

        // Box census, taken here because this is the one place that runs
        // exactly once per corner at the instant the delivery is in the
        // air — so it sees both the set-up AND whether the shape survived
        // to the cross. Counted over the real penalty area rather than the
        // contest's 135u radius: "in the box" has to mean the box, or the
        // number cannot be compared with the real one (8-10 defenders).
        #[cfg(feature = "match-logs")]
        {
            let field_height = context.field_size.height as f32;
            let (mut def_in_box, mut att_in_box) = (0u32, 0u32);
            for p in field.players.iter() {
                if p.is_sent_off
                    || p.tactical_position.current_position.is_goalkeeper()
                    || !CornerShape::is_in_penalty_area(p.position, attacked_goal.x, field_height)
                {
                    continue;
                }
                if p.team_id == att_team {
                    att_in_box += 1;
                } else {
                    def_in_box += 1;
                }
            }
            SetPieceDiag::note_corner_box(def_in_box, att_in_box);
        }

        let (att_idx, att_score) = match best_att {
            Some(v) => v,
            None => {
                field.ball.corner_contest_resolved = true;
                return;
            }
        };

        // Base eased 0.36 → 0.31 in the 2026-08 state-repair
        // recalibration. 0.36 was set while the loose-ball override could
        // still yank the winning header off the dropped ball mid-attempt;
        // headers are committed actions now and complete every time, so
        // the same win rate converts to ~35% more corner goals (DEF
        // corner headers on goal 536 → 708 per 200 matches, DEF goal
        // share 14.5% → 18.6% against the real ~10%).
        //
        // Delivery scale: the ball that arrives is the other half of the
        // contest, and it was missing entirely — the two duellists and the
        // keeper decided everything, so a dead-ball specialist's whipped
        // corner and a centre-half's hopeful clip produced identical
        // chances.
        //
        // ⚠ MULTIPLICATIVE, and it has to be. For an evenly-matched box
        // the expression below lands NEGATIVE before the clamp (0.100
        // − gk_command·0.18 with gk_command ≈ 0.6 is −0.008), so the 0.04
        // floor is what most corners actually return. An *additive*
        // delivery term centred on the population mean therefore does not
        // cancel out: the below-average half is swallowed by the floor
        // while the above-average half escapes it, and the contest
        // ratchets upward — measured at +30% attacker wins with a
        // correctly-centred additive term. Scaling instead keeps the sign,
        // so a poor delivery makes an already-floored corner more negative
        // (still floored) and only corners with a real aerial edge move at
        // all, in both directions.
        let delivery_scale =
            (field.ball.pending_corner_delivery / CORNER_DELIVERY_REFERENCE).clamp(0.55, 1.45);
        // Routine: where the ball is put changes how cleanly it can be
        // met. The penalty spot is the classic — most time to attack it
        // and the keeper furthest from it. Near post is a flick, harder to
        // time; far post gives the keeper the whole flight to read it.
        let routine_scale = match field.ball.pending_corner_routine {
            Some(CornerRoutine::NearPost) => 0.95,
            Some(CornerRoutine::FarPost) => 0.92,
            _ => 1.00,
        };
        let att_win = ((0.100 + (att_score - best_def_score) * 0.50 - gk_command * 0.18)
            * delivery_scale
            * routine_scale)
            .clamp(0.04, 0.36);

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
            //
            // Restated for the metres-per-tick vertical axis: −0.02 m/tick
            // is 2 m/s of descent, which walks the ball down through the
            // [1.4, 2.5] heading band over ~40 ticks, and 0.12 u/tick of
            // goalward drift keeps it inside the 6u header reach for all of
            // them. The old (−0.35, 1.8) pair was written when the vertical
            // axis carried unit-scale speeds and would now fall through the
            // band in three ticks while drifting 60u out of reach.
            //
            // ⚠ **The ball is no longer WRITTEN onto his head.** All of the
            // above still happens; it happens when the delivery gets
            // there. The cross now actually flies the twenty-five metres
            // from the flag — see [`Self::deliver_to_winner`] and
            // [`AerialDelivery`] — which is the corner half of the "ball
            // teleports" report. `CORNER_APEX` is a normal in-swinger:
            // 5 m up, about 1.7 s in the air, comfortably inside
            // `CORNER_SHAPE_MAX_TICKS` so the shape still holds for the
            // whole flight.
            Self::deliver_to_winner(
                field,
                att_idx,
                attacked_goal,
                taker,
                Self::CORNER_DROP_BEHIND,
                Self::CORNER_APEX,
                true,
                false,
            );
        } else if let Some(clearer) = best_def {
            // **The repeat corner.** The defending side wins the header,
            // and the man it falls to — standing in his own six-yard area
            // with the ball already across him — hooks it over his own
            // byline instead of trying to turn it upfield.
            //
            // This is the sibling of the same branch in
            // `resolve_cross_contest`, on the same curve and the same
            // window, and the corner contest was the one that never had
            // it: a delivery the attackers did not win simply flew on to
            // its aim point untouched, so **a corner in this engine could
            // never produce another corner**. Real football does that
            // constantly — it is why sides win three and four in a row —
            // and the corner-source census had the whole "defender puts a
            // delivery behind" family at 4% of supply against a real ~35%.
            //
            // ⚠ This branch was the bigger half of the corner teleport,
            // not the attacking one — `att_win` is clamped at 0.36, so
            // most corners come here. It used to hook the ball behind
            // FROM THE CLEARER'S FEET while the ball was still at the
            // flag, which wrote it the full width of the box in one tick.
            // Now the delivery flies to him and is hooked when it
            // arrives, through the same [`AerialDelivery`] machinery the
            // attacking branch uses.
            let from = field.players[clearer].position;
            if Self::heads_it_behind(from, attacked_goal, field.size.width as f32, context) {
                Self::deliver_to_winner(
                    field,
                    clearer,
                    attacked_goal,
                    taker,
                    Self::CORNER_DROP_BEHIND,
                    Self::CORNER_APEX,
                    false,
                    false,
                );
            }
        }
        // Otherwise the cross plays out — the keeper claims or a defender
        // clears it upfield (the realistic majority outcome).

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
        // Back to "an ordinary delivery" so a stale specialist stamp can't
        // leak into the next corner (or into an open-play cross contest
        // that reads the same field).
        field.ball.pending_corner_delivery = CORNER_DELIVERY_REFERENCE;

        field.ball.corner_contest_resolved = true;
    }

    /// Sample the DEFENDING side's shape while it is actually defending.
    ///
    /// Two questions, both of which an aggregate over a whole match
    /// answers wrongly because most of a match is spent attacking:
    ///
    /// * **Is the back line a rigid body?** `depth spread` is the range of
    ///   the back four along the goal-to-goal axis. A real back four
    ///   staggers — the cover defender drops behind the presser, the far
    ///   full-back tucks in — so 25-65u (3-8 m) is normal. Near zero means
    ///   four players sharing one depth target and sliding as one.
    /// * **Does anybody meet the attacker?** For every opponent inside our
    ///   defensive third, how far away is the nearest defender. Real
    ///   marking distances in a settled block are 2-6 m on the ball side.
    ///
    /// Sampled every 25 ticks (quarter-second) and only while an opponent
    /// carries the ball in our half, so the numbers describe defending
    /// rather than an average dominated by possession.
    #[cfg(feature = "match-logs")]
    pub(super) fn sample_defensive_shape(field: &MatchField, context: &MatchContext) {
        use crate::mid_run_diag::DefenceDiag;

        const SAMPLE_INTERVAL_TICKS: u64 = 25;
        /// A defender this close is contesting the attacker. 24u = 3 m.
        const MARKED_RADIUS: f32 = 24.0;

        if context.current_tick() % SAMPLE_INTERVAL_TICKS != 0 {
            return;
        }
        let Some(carrier) = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
        else {
            return;
        };
        let attacking_team = carrier.team_id;
        let Some(defending_side) = field
            .players
            .iter()
            .find(|p| p.team_id != attacking_team)
            .and_then(|p| p.side)
        else {
            return;
        };
        let field_width = field.size.width as f32;
        let own_goal_x = match defending_side {
            PlayerSide::Left => 0.0,
            PlayerSide::Right => field_width,
        };
        // Only sample while the ball is in the defending side's half —
        // otherwise "the back line" is a pressing line and the marking
        // question is meaningless.
        if (field.ball.position.x - own_goal_x).abs() > field_width * 0.5 {
            return;
        }

        // ── Back-line shape ──────────────────────────────────────────
        let mut xs = [0.0f32; 8];
        let mut ys = [0.0f32; 8];
        let mut n = 0usize;
        for p in field.players.iter() {
            if p.team_id == attacking_team || n == xs.len() {
                continue;
            }
            let pos = p.tactical_position.current_position;
            // `is_defender()` is the POSITION GROUP, and that group holds
            // `DefensiveMidfielder` (see `position_group`). A DM sits ten
            // to fifteen metres in front of the back four on purpose, so
            // including him put that gap into `max_x - min_x`
            // permanently: this printed 17.5 m of "back-line depth
            // spread" against a real-back-four reference of 3-8 m and
            // could never have reached it, whatever the defenders did.
            // Two rounds of shape work were measured against that number
            // before it was checked. The back LINE is the back four.
            if !pos.is_defender()
                || pos.is_goalkeeper()
                || matches!(pos, PlayerPositionType::DefensiveMidfielder)
            {
                continue;
            }
            // …and the man who has gone to the ball is not part of the
            // shape. Somebody always leaves the line to engage — measured
            // at 25% of the back line at the moment a shot is struck —
            // and with four defenders that is one man permanently 15 m
            // upfield, which puts 15 m into `max_x - min_x` on its own.
            // The number could therefore never approach its own
            // real-football reference no matter how the line behaved, and
            // it did not move across two rounds of shape work for exactly
            // that reason. What the shape constraint governs, and what
            // this should report, is the spread of the defenders actually
            // holding shape.
            if matches!(
                p.state,
                PlayerState::Defender(
                    DefenderState::Pressing
                        | DefenderState::Tackling
                        | DefenderState::TakeBall
                        | DefenderState::Intercepting
                )
            ) {
                continue;
            }
            xs[n] = p.position.x;
            ys[n] = p.position.y;
            n += 1;
        }
        if n >= 3 {
            let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
            for &x in &xs[..n] {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
            }
            // Insertion-sort the lateral positions so "adjacent pair" is
            // meaningful rather than roster order.
            for i in 1..n {
                let v = ys[i];
                let mut j = i;
                while j > 0 && ys[j - 1] > v {
                    ys[j] = ys[j - 1];
                    j -= 1;
                }
                ys[j] = v;
            }
            let mut max_gap = 0.0f32;
            for w in ys[..n].windows(2) {
                max_gap = max_gap.max(w[1] - w[0]);
            }
            DefenceDiag::note_shape(max_x - min_x, max_gap);
        }

        // ── Is anybody meeting the attackers? ────────────────────────
        let third = field_width / 3.0;
        for a in field.players.iter() {
            if a.team_id != attacking_team {
                continue;
            }
            if a.tactical_position.current_position.is_goalkeeper() {
                continue;
            }
            if (a.position.x - own_goal_x).abs() > third {
                continue;
            }
            let mut nearest = f32::MAX;
            for d in field.players.iter() {
                if d.team_id == attacking_team
                    || d.tactical_position.current_position.is_goalkeeper()
                {
                    continue;
                }
                nearest = nearest.min((d.position - a.position).magnitude());
            }
            if nearest.is_finite() {
                DefenceDiag::note_attacker(nearest, nearest > MARKED_RADIUS);
            }
        }

        // ── The marking duel itself ──────────────────────────────────
        //
        // The measure above is every defender against every attacker, and
        // it is dominated by sheer density: the goal-side recovery rule
        // puts bodies near attackers whether or not anybody is marking
        // them, so it cannot tell you whether marking is being BEATEN.
        // This one reads the assigned pairs only — the distance between a
        // marker and the man he was actually given — which is the number
        // the evasion work moves or does not.
        let plan = context.defence_plan_for_team(
            field
                .players
                .iter()
                .find(|p| p.team_id != attacking_team)
                .map(|p| p.team_id)
                .unwrap_or(attacking_team),
        );
        if plan.active {
            for d in field.players.iter() {
                let Some(target) = plan.mark_of(d.id) else {
                    continue;
                };
                if let Some(man) = field.players.iter().find(|p| p.id == target) {
                    let pos = man.tactical_position.current_position;
                    let line = if pos.is_forward() {
                        2
                    } else if pos.is_midfielder() {
                        1
                    } else {
                        0
                    };
                    // Is the assigned marker in a state that actually
                    // HONOURS the assignment? The plan hands out duties
                    // to the whole unit, but only two states read them —
                    // `Marking` for a defender and `Guarding` for a
                    // midfielder — so a marker in any other state is
                    // carrying a duty nothing acts on, and tuning the
                    // marking distance reaches none of those ticks. Same
                    // failure shape as the back line's shape code living
                    // in `HoldingLine`.
                    // 0 marking, 1 playing the ball (legitimate), 2
                    // pressing/covering, 3 running/recovering, 4 idle.
                    // Only bucket 4 — and most of 3 — is a duty nobody
                    // is acting on.
                    let bucket = match d.state {
                        PlayerState::Defender(DefenderState::Marking)
                        | PlayerState::Midfielder(MidfielderState::Guarding) => 0,
                        PlayerState::Defender(
                            DefenderState::Tackling
                            | DefenderState::Intercepting
                            | DefenderState::TakeBall
                            | DefenderState::Clearing
                            | DefenderState::Heading
                            | DefenderState::Passing,
                        )
                        | PlayerState::Midfielder(
                            MidfielderState::Tackling
                            | MidfielderState::Intercepting
                            | MidfielderState::TakeBall
                            | MidfielderState::Heading
                            | MidfielderState::Passing,
                        ) => 1,
                        PlayerState::Defender(
                            DefenderState::Pressing | DefenderState::Covering,
                        )
                        | PlayerState::Midfielder(MidfielderState::Pressing) => 2,
                        PlayerState::Defender(
                            DefenderState::Running
                            | DefenderState::Returning
                            | DefenderState::TrackingBack,
                        )
                        | PlayerState::Midfielder(
                            MidfielderState::Running | MidfielderState::Returning,
                        ) => 3,
                        _ => 4,
                    };
                    DefenceDiag::note_duel((d.position - man.position).magnitude(), line, bucket);
                }
            }
        }
    }

    /// **What happens to the man who has the ball in our box.**
    ///
    /// The shape sampler above measures where defenders STAND. This one
    /// measures what they DO about a carrier who is already among them —
    /// the question behind "he runs around the penalty area surrounded by
    /// defenders and nobody tries to take it off him".
    ///
    /// Every challenge in the engine, from every state and every role,
    /// funnels through the same three gates before an attempt is rolled:
    /// the per-player tackle cooldown (`can_attempt_tackle`), the duel
    /// gate (`TackleEngagement::may_engage_carrier`), and the commitment
    /// roll (`TackleDecision`). A defender stopped by any of them looks
    /// identical from the stands and identical in the aggregate stats —
    /// he is next to the carrier, doing nothing. Bucketing the pairs by
    /// which gate stopped them is the only way to tell which one is
    /// binding.
    ///
    /// Sampled every tick, because a challenge is a moment: a
    /// quarter-second sampler would miss most of the `Tackling` ticks it
    /// is looking for.
    #[cfg(feature = "match-logs")]
    pub(super) fn sample_duel_gates(field: &MatchField, context: &MatchContext) {
        use crate::r#match::common_states::TackleEngagement;
        use crate::mid_run_diag::DuelDiag;

        /// Close enough to be in the picture the report describes. 24u = 3 m.
        const SURROUND_RADIUS: f32 = 24.0;

        let Some(carrier) = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
        else {
            return;
        };
        if carrier.tactical_position.current_position.is_goalkeeper() {
            return;
        }
        let attacking_team = carrier.team_id;
        let Some(defending) = field.players.iter().find(|p| p.team_id != attacking_team) else {
            return;
        };
        let defending_team = defending.team_id;
        let Some(defending_side) = defending.side else {
            return;
        };
        let plan = context.defence_plan_for_team(defending_team);
        let presser = plan.presser();
        // The referee's own test — the ball inside the area the defending
        // side is protecting. Same question `PenaltyRisk::applies` asks.
        let in_box = context
            .penalty_area(defending_side == PlayerSide::Left)
            .contains(&field.ball.position);

        let mut bodies = 0u64;
        let mut contested = false;
        // Nearest defender to the carrier, for the closing census below.
        let mut nearest: Option<(f32, &MatchPlayer)> = None;
        for d in field.players.iter() {
            if d.team_id == attacking_team || d.tactical_position.current_position.is_goalkeeper() {
                continue;
            }
            DuelDiag::note_cooldown(!d.can_attempt_tackle());
            let gap = (d.position - carrier.position).magnitude();
            if nearest.is_none_or(|(best, _)| gap < best) {
                nearest = Some((gap, d));
            }
            if gap <= SURROUND_RADIUS {
                bodies += 1;
            }
            let challenging = matches!(
                d.state,
                PlayerState::Defender(DefenderState::Tackling)
                    | PlayerState::Midfielder(MidfielderState::Tackling)
                    | PlayerState::Forward(ForwardState::Tackling)
            );
            if challenging {
                // An attempt is only ever rolled inside `CONTACT`. Where the
                // rest of them are standing is the difference between a
                // defence that declines its challenges and one that never
                // reaches them.
                DuelDiag::note_reach(if gap <= TackleEngagement::CONTACT {
                    0
                } else if gap <= TackleEngagement::COMMIT {
                    1
                } else if gap <= TackleEngagement::DISENGAGE {
                    2
                } else {
                    3
                });
            }
            if challenging && gap <= SURROUND_RADIUS {
                contested = true;
            }
            if gap > TackleEngagement::COMMIT {
                continue;
            }
            // Ordered as the gates are, so the first large bucket is the
            // binding one.
            let bucket = if challenging {
                0
            } else if !d.can_attempt_tackle() {
                1
            } else if presser.is_some_and(|p| p != d.id) {
                2
            } else {
                3
            };
            DuelDiag::note_gate(bucket, in_box);
        }
        if in_box {
            DuelDiag::note_box_carry(bodies, contested);
        }

        // ── IS HE ACTUALLY GETTING ANY CLOSER? ────────────────────────
        //
        // See the note on `mid_run_diag::CLOSE_SAMPLES`. Everything above
        // buckets a defender by whether he is ALLOWED to challenge; this
        // asks whether the man nearest the carrier is converging on him or
        // merely travelling alongside, which is the difference the report
        // is about and which no other counter here can see.
        //
        // Only sampled while the carrier is genuinely moving — a defender
        // holding his ground against a man shielding the ball is
        // jockeying, and counting that as a failure to close would bury
        // the signal under correct defending.
        let carrier_v = carrier.velocity;
        let carrier_speed = carrier_v.magnitude();
        if carrier_speed > 0.05 {
            if let Some((gap, d)) = nearest {
                if gap > 0.5 && gap <= 200.0 {
                    let to_carrier = (carrier.position - d.position) / gap;
                    // Closing rate: how fast the gap shrinks. The carrier's
                    // own motion counts — running at a man who is running
                    // away is not closing on him.
                    let rate = (d.velocity - carrier_v).dot(&to_carrier);
                    let d_speed = d.velocity.magnitude();
                    let align = if d_speed > 0.01 {
                        d.velocity.dot(&carrier_v) / (d_speed * carrier_speed)
                    } else {
                        0.0
                    };
                    let own_goal_x = if defending_side == PlayerSide::Left {
                        0.0
                    } else {
                        context.field_size.width as f32
                    };
                    // ── GOAL-SIDE SHADOWING IS NOT THE DEFECT ─────────
                    //
                    // A defender jockeying a carrier who is running at
                    // goal retreats in front of him: same heading, gap
                    // held, closing rate ~0. That is textbook defending
                    // and the naive "same heading and not closing" test
                    // counts every second of it as a failure — which is
                    // why the first cut of this census read a flat 50%
                    // whatever was changed underneath it.
                    //
                    // What the report describes is the other one: a
                    // defender LEVEL WITH or BEHIND the man, matching his
                    // speed shoulder to shoulder, with the goal open past
                    // him. So the parallel test is restricted to a
                    // defender who is not goal-side — the ball is already
                    // nearer his goal than he is.
                    let goal_side =
                        (d.position.x - own_goal_x).abs() < (carrier.position.x - own_goal_x).abs();
                    let parallel = align > 0.5 && rate < carrier_speed * 0.10 && !goal_side;
                    let gaining = rate > carrier_speed * 0.10;
                    let deep = (carrier.position.x - own_goal_x).abs()
                        < context.field_size.width as f32 / 3.0;
                    let state = match d.state {
                        PlayerState::Defender(DefenderState::Tackling)
                        | PlayerState::Midfielder(MidfielderState::Tackling)
                        | PlayerState::Forward(ForwardState::Tackling) => 0,
                        PlayerState::Defender(DefenderState::Pressing)
                        | PlayerState::Midfielder(MidfielderState::Pressing)
                        | PlayerState::Forward(ForwardState::Pressing) => 1,
                        PlayerState::Defender(DefenderState::Marking)
                        | PlayerState::Midfielder(MidfielderState::Guarding) => 2,
                        PlayerState::Defender(DefenderState::Covering) => 3,
                        PlayerState::Defender(DefenderState::Running)
                        | PlayerState::Midfielder(MidfielderState::Running)
                        | PlayerState::Forward(ForwardState::Running) => 4,
                        PlayerState::Defender(DefenderState::HoldingLine) => 5,
                        PlayerState::Defender(DefenderState::TrackingBack) => 6,
                        _ => 7,
                    };
                    DuelDiag::note_closing(rate, align, gap, deep, parallel, gaining, state);
                }
            }
        }
    }

    /// IS HE RUNNING AT THE BALL, OR JUST ALONGSIDE IT?
    ///
    /// The sibling of the closing census in
    /// [`sample_duel_gates`](Self::sample_duel_gates), for the half of
    /// the game that one cannot see. It samples only while the ball is
    /// LOOSE — which is exactly when it bails out, because a `TakeBall`
    /// state exists only while nobody owns the ball.
    ///
    /// See `mid_run_diag::CHASE_SAMPLES` for what `lead` means and why it
    /// is the quantity that separates an interception from a stern chase.
    #[cfg(feature = "match-logs")]
    pub(super) fn sample_loose_chase(field: &MatchField) {
        use crate::mid_run_diag::ChaseDiag;

        if field.ball.current_owner.is_some() {
            return;
        }
        let ball_v = Vector3::new(field.ball.velocity.x, field.ball.velocity.y, 0.0);
        let ball_speed = ball_v.magnitude();
        // A ball that is barely moving cannot be run alongside, and the
        // lead of a stationary target is undefined. 0.05 u/tick is
        // 6 cm/s — a ball at rest in everything but the last decimal.
        if ball_speed < 0.05 {
            return;
        }
        let ball_dir = ball_v / ball_speed;
        let ball_pos = Vector3::new(field.ball.position.x, field.ball.position.y, 0.0);

        for p in field.players.iter() {
            let line = match p.state {
                PlayerState::Defender(DefenderState::TakeBall) => 0,
                PlayerState::Midfielder(MidfielderState::TakeBall) => 1,
                PlayerState::Forward(ForwardState::TakeBall) => 2,
                PlayerState::Goalkeeper(GoalkeeperState::TakeBall) => 3,
                _ => continue,
            };
            let flat = Vector3::new(p.position.x, p.position.y, 0.0);
            let to_ball = ball_pos - flat;
            let gap = to_ball.magnitude();
            // Inside a stride the geometry is degenerate — the aim point
            // and the ball are the same place whatever the model does.
            if gap < 2.0 {
                continue;
            }
            let to_ball_dir = to_ball / gap;

            let p_v = Vector3::new(p.velocity.x, p.velocity.y, 0.0);
            let p_speed = p_v.magnitude();
            // Standing still is not a chase; it is a different defect and
            // averaging it in would mute this one.
            if p_speed < 0.02 {
                continue;
            }
            let p_dir = p_v / p_speed;

            let rate = (p_v - ball_v).dot(&to_ball_dir);
            let align = p_dir.dot(&ball_dir);
            // Cross-track aim: strip the part of his heading that points
            // AT the ball, and ask how much of what is left runs with the
            // ball's travel. Zero is a man pointed at where the ball is.
            let lead = (p_dir - to_ball_dir * p_dir.dot(&to_ball_dir)).dot(&ball_dir);

            ChaseDiag::note(rate, lead, align, gap, ball_speed, line);
        }
    }
    /// Discrete OPEN-PLAY cross contest — the sibling of
    /// [`resolve_corner_contest`](Self::resolve_corner_contest), and for
    /// the same reason.
    ///
    /// A lofted cross is aimed at a patch of the box, not at a pair of
    /// feet, so it cannot be settled the way a pass is. Three engine
    /// facts made that impossible before this existed: `try_intercept`
    /// declines any ball above 2.5 m, the receiver claim declines above
    /// 2.8 m, and the in-flight window reserves the delivery for one
    /// named receiver for its entire flight. The result was that an
    /// aerial cross was a private transaction between the crosser and one
    /// teammate that no defender, second attacker or keeper could touch.
    ///
    /// So the engine resolves ONE skill-weighted contest the moment the
    /// delivery is over the box: the best attacking header against the
    /// best defending header, with the keeper's command of his area able
    /// to take the ball off both of them. The winner gets the ball
    /// dropped on their head and strikes it through the NORMAL shot /
    /// save pipeline, so goals, shots, xG and saves all credit through
    /// the paths they already use — no bespoke scoring route.
    ///
    /// Win rates are deliberately low. Real football completes roughly a
    /// quarter of open-play crosses, and only a fraction of those become
    /// attempts, which is why crossing is a low-percentage way to attack
    /// even though every team does it.
    pub(super) fn resolve_cross_contest(field: &mut MatchField, context: &mut MatchContext) {
        let ball = &field.ball;
        if ball.cross_contest_resolved {
            return;
        }
        // Only once the delivery has left the crosser and is genuinely in
        // the air. A cross still at his feet is a set-up, not a contest.
        if ball.current_owner.is_some() {
            return;
        }
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::CROSS_CONTEST_SEEN.fetch_add(1, Ordering::Relaxed);

        // Resolve at the point the ball is actually attackable — head
        // height on the way DOWN. Above that it is still travelling; below
        // it, the ordinary reception path has it.
        //
        // Widening this band to 5.0 m was tried, on the theory that the
        // ordinary receiver claim (which starts at 2.8 m, and resolves
        // EARLIER in the tick than this does) was pre-empting the duel.
        // It moved contests from 3.9 to 4.7 a match — inside run-to-run
        // noise — and was reverted, because the diagnosis was wrong.
        //
        // What actually happens: of ~14 lofted deliveries a match, ~12.6
        // are CORNER kicks, and `resolve_corner_contest` runs first in
        // `game_tick_inner` and ends by calling
        // `clear_pending_pass_metadata`, which disarms this contest —
        // correctly, since a corner is its business. Only 2-3 open-play
        // crosses a match exist for this contest to resolve. The gap is
        // crossing VOLUME, not this window. See `CrossDiag`.
        const CONTEST_CEILING: f32 = 2.9;
        const CONTEST_FLOOR: f32 = 1.5;
        if ball.position.z > CONTEST_CEILING
            || ball.position.z < CONTEST_FLOOR
            || ball.velocity.z > 0.0
        {
            #[cfg(feature = "match-logs")]
            CrossDiag::note_reject(if ball.position.z > CONTEST_CEILING {
                0
            } else if ball.velocity.z > 0.0 {
                2
            } else {
                1
            });
            return;
        }

        let cross_type = ball.pending_cross_type;
        let crosser = ball.previous_owner;
        let Some(att_team) = crosser
            .and_then(|id| field.players.iter().find(|p| p.id == id))
            .map(|p| p.team_id)
        else {
            field.ball.cross_contest_resolved = true;
            return;
        };

        // The goal being attacked is the one the crossing team shoots at.
        let gl = context.goal_positions.left;
        let gr = context.goal_positions.right;
        let ball_pos = ball.position;
        let attacked_goal = if (ball_pos - gl).magnitude() < (ball_pos - gr).magnitude() {
            gl
        } else {
            gr
        };
        // Not a box delivery — let it play out as an ordinary ball.
        if (ball_pos - attacked_goal).magnitude() > 200.0 {
            #[cfg(feature = "match-logs")]
            CrossDiag::note_reject(3);
            return;
        }

        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::CROSS_CONTEST_FIRED.fetch_add(1, Ordering::Relaxed);

        let minute = (context.total_match_time / 60_000) as u32;

        // Only players who can actually get to the ball contest it. 34u is
        // ~4.3 m — a stride and a jump, which is the real radius of an
        // aerial challenge, not the whole penalty area.
        const CONTEST_RADIUS: f32 = 34.0;

        let mut best_att: Option<(usize, f32)> = None;
        let mut best_def_score = 0.0_f32;
        let mut defenders_contesting = 0u32;
        let mut gk_command = 0.0_f32;
        let mut gk_idx: Option<usize> = None;

        for (i, p) in field.players.iter().enumerate() {
            let gap = (p.position - ball_pos).magnitude();
            let is_gk = p.tactical_position.current_position.is_goalkeeper();
            // The keeper commands a wider zone than an outfielder — that
            // is the whole point of coming for a cross.
            let reach = if is_gk { 58.0 } else { CONTEST_RADIUS };
            if gap > reach {
                continue;
            }
            if p.team_id == att_team {
                if is_gk {
                    continue;
                }
                let s = sc::aerial_outfield_attacker(p, minute);
                if best_att.map_or(true, |(_, bs)| s > bs) {
                    best_att = Some((i, s));
                }
            } else if is_gk {
                let raw = (p.skills.goalkeeping.command_of_area * 0.6
                    + p.skills.goalkeeping.aerial_reach * 0.4)
                    / 20.0;
                // Distance decay — a keeper on his line does not command
                // a ball at the back post.
                gk_command = raw * (1.0 - gap / 58.0).clamp(0.0, 1.0);
                gk_idx = Some(i);
            } else {
                defenders_contesting += 1;
                let s = sc::aerial_outfield_defender(p, minute);
                if s > best_def_score {
                    best_def_score = s;
                }
            }
        }

        // Nobody attacking it — the delivery just runs through, which is
        // what a bad cross does.
        let Some((att_idx, att_score)) = best_att else {
            field.ball.cross_contest_resolved = true;
            return;
        };

        // An unmarked header is rare; an empty box is not a free goal
        // either, because the keeper is still there.
        let def_score = if defenders_contesting == 0 {
            0.30
        } else {
            // Each extra body in the challenge makes it harder to get a
            // clean contact, independent of the best defender's quality.
            best_def_score + (defenders_contesting.saturating_sub(1) as f32) * 0.06
        };

        // A whipped or driven ball is harder for a keeper to claim and
        // easier for an attacker to attack; a floated one hangs long
        // enough for the defence to set. This is the payoff for modelling
        // the delivery mix at all — the numbers live on `CrossType` so the
        // contest and the crosser's own risk estimate read one source.
        let type_edge = cross_type.map(CrossType::contest_edge).unwrap_or(0.0);
        let gk_claim_edge = cross_type.map(CrossType::keeper_claim_scale).unwrap_or(1.0);

        // Keeper first: he either takes it off everyone or he doesn't come.
        let gk_claim = (gk_command * 0.55 * gk_claim_edge).clamp(0.0, 0.45);
        if gk_idx.is_some() && context.rng.bernoulli(gk_claim) {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::CROSS_CONTEST_GK.fetch_add(1, Ordering::Relaxed);
            // Leave the ball live and low in front of the keeper — his own
            // claim/catch model in the GK state machine takes it from
            // here, so the save/gather accounting stays on one path.
            //
            // ⚠ **Brought DOWN, not put down.** This used to be
            // `b.position.z = 0.6`, and a cross the keeper comes for is
            // two to three metres up — so the ball fell as much as 2.4 m
            // in a single 10 ms tick with its x/y untouched. On the
            // whole-tick relocation census that was the entire residue of
            // the `cross_contest` row: 1.3 a match, and **every one of
            // them purely vertical**, which is the axis a replay shows
            // most plainly. Height is the one axis `flight_diag` has
            // never measured — its `StageProbe` is `sqrt(dx² + dy²)` — so
            // this had no counter until now.
            //
            // A descent rate instead of a height gets the ball to the
            // same place in an eighth of a second, which is a keeper
            // taking the pace off a cross rather than the ball blinking.
            let b = &mut field.ball;
            /// Ticks the ball takes to come down to the keeper's hands.
            /// 12 (0.12 s) is fast enough that his claim model sees a low
            /// ball on the same approach it always did, and slow enough
            /// that the descent is drawn.
            const SETTLE_TICKS: f32 = 12.0;
            const CLAIM_HEIGHT: f32 = 0.6;
            let drop = ((b.position.z - CLAIM_HEIGHT) / SETTLE_TICKS).max(0.0);
            b.velocity = Vector3::new(b.velocity.x * 0.25, b.velocity.y * 0.25, -drop);
            b.pass_target_player_id = None;
            b.clear_pending_pass_metadata();
            b.cross_contest_resolved = true;
            return;
        }

        // Attacker vs defender. Base is low because most crosses are
        // headed clear — the spread comes from the aerial mismatch.
        let att_win = (0.26 + (att_score - def_score) * 0.55 + type_edge).clamp(0.05, 0.62);

        if context.rng.bernoulli(att_win) {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::CROSS_CONTEST_WON.fetch_add(1, Ordering::Relaxed);
            // Drop the ball onto the winner's head, moving goalward, and
            // hold it in the heading band long enough for their state
            // machine to strike it. Same kinematics as the corner contest:
            // z 2.5 sits one tick above the intercept window, -0.02 m/tick
            // walks down through the [1.5, 2.5] band over ~40 ticks, and
            // 0.12 u/tick of drift keeps it inside header reach for all of
            // them — so ANY winner's state machine gets a valid tick, not
            // just one that happened to already be in a heading state.
            // The winner is forced into his heading state — "not all of
            // them carry the entry hook", and leaving the transition to
            // chance is why the contest could be won 307 times and produce
            // zero headers. That is still true; what changed is WHEN. The
            // transition now rides on the delivery and fires when the ball
            // reaches him, because a heading state does not survive the
            // 1.5 s the ball is now in the air. See
            // `AerialDelivery::force_heading`.
            //
            // The cross flies to him rather than being written onto his
            // head — the same change, and the same reasons, as the corner
            // contest above. This one moved the ball a mean of 1.1 m
            // against the corner's 25 m, but it fires on every lofted
            // cross rather than on corners alone, and 80% of its
            // relocations were a VERTICAL snap: the ball dropping to
            // 2.5 m from wherever the delivery had climbed to, which is
            // the most visible axis there is.
            Self::deliver_to_winner(
                field,
                att_idx,
                attacked_goal,
                crosser,
                Self::CROSS_DROP_BEHIND,
                Self::CROSS_APEX,
                true,
                true,
            );
        } else {
            // Headed clear. This is the majority outcome and it is what
            // feeds the second-ball phase — but a defensive header is a
            // full-blooded clearance, not a nudge: it goes 20-30 m and
            // lands OUTSIDE the area. A short one just drops the ball back
            // into the box for a rebound shot, which is a cheap way to
            // manufacture chances that never existed.
            //
            // Solved rather than picked, because the vertical axis is in
            // METRES and a hand-written z reads as a sane number while
            // meaning something absurd: the first draft of this used
            // `0.28`, which is a 40 m apex. Ask for the apex and let the
            // shared ballistics helper produce the launch speed, then size
            // the horizontal component to the range the arc can carry.
            // …but not always UPFIELD. A defender meeting a ball that is
            // already across him, six yards out, cannot turn it round —
            // he puts it behind, and concedes the corner he can defend
            // instead of the chance he cannot.
            //
            // This branch is the majority outcome of every cross in the
            // engine and it could only ever clear away from goal, so
            // **defenders never conceded corners**: before it, the only
            // real supplier was the keeper parrying, at 3.4 a match.
            //
            // ⚠ THE TARGET IT WAS SIZED AGAINST WAS TWICE THE REAL ONE.
            // "corners ran at ~10.8 against a real ~21, and the endline
            // split was 25% corners against ~62% real" — both of those
            // reference figures came from reading the per-MATCH corner
            // average (~10.4) as a per-TEAM one. A real match has ~10.4
            // corners and ~16 goal kicks: ~40% corners, which is what the
            // engine measures today. So this branch was aimed at roughly
            // double the corners football actually produces, and its
            // `BEHIND_AT_LINE` share should be read in that light before
            // anybody raises it further.
            if Self::heads_it_behind(ball_pos, attacked_goal, field.size.width as f32, context) {
                Self::hook_it_behind(field, ball_pos, attacked_goal);
                field.ball.cross_contest_resolved = true;
                return;
            }

            const CLEAR_RANGE_UNITS: f32 = 210.0; // ~26 m
            const CLEAR_APEX_METRES: f32 = 6.0;
            let vz = Ball::launch_speed_for_apex(CLEAR_APEX_METRES);
            let hang = Ball::hang_ticks(vz).max(1.0);
            let speed = CLEAR_RANGE_UNITS / hang;

            let clear_dir = (ball_pos - attacked_goal)
                .try_normalize(0.01)
                .unwrap_or_else(|| Vector3::new(1.0, 0.0, 0.0));
            // Headers are cleared toward the touchline, not straight back
            // down the middle where the attack came from.
            let lateral = if ball_pos.y >= attacked_goal.y {
                1.0
            } else {
                -1.0
            };
            let dir = Vector3::new(
                clear_dir.x + lateral * 0.15,
                clear_dir.y + lateral * 0.55,
                0.0,
            )
            .try_normalize(0.01)
            .unwrap_or(clear_dir);

            let b = &mut field.ball;
            // ⚠ No height write. This used to be `b.position.z = 2.2`,
            // which is a snap of up to 0.7 m on the one axis a replay
            // shows most plainly — and it is redundant: the guard at the
            // top of this function only lets the contest fire on a ball
            // already inside `[CONTEST_FLOOR, CONTEST_CEILING]` and
            // already coming down, so it is at heading height by
            // construction. He heads it from where it is.
            b.velocity = Vector3::new(dir.x * speed, dir.y * speed, vz);
            b.current_owner = None;
            b.flags.in_flight_state = 1;
        }

        // The contest IS the resolution of the delivery — drop the stale
        // aim so the nominal target can't auto-claim the dropped ball
        // through the receiver-priority radius, exactly as the corner
        // contest does.
        field.ball.pass_target_player_id = None;
        field.ball.clear_pending_pass_metadata();
        field.ball.cross_contest_resolved = true;
    }

    /// Does this defensive header go BEHIND for a corner rather than
    /// upfield? See the call site in [`resolve_cross_contest`].
    ///
    /// Depth decides it, because depth is what removes the option: a
    /// header met on the edge of the area can be sent anywhere, one met
    /// on the six-yard line with the ball travelling across you can only
    /// go one way. The share rises steeply as the goal line approaches
    /// and is zero outside the area, so ordinary defensive headers in and
    /// around the box still play the ball out as they always did.
    /// Put the ball over the defender's own byline, wide of the post.
    ///
    /// The other half of [`heads_it_behind`](Self::heads_it_behind) and of
    /// the corner contest's cleared branch: once the decision is taken,
    /// both need the same hooked, high, short trajectory, and both need it
    /// to finish OUTSIDE the posts — a clearance across the face of goal
    /// is an own goal, not a clearance.
    /// Send a decided aerial contest's ball to the man who won it — by
    /// flying it there, not by writing it onto his head.
    ///
    /// # The teleport this replaces
    ///
    /// Both contests used to finish with `b.position = winner_pos - dir *
    /// n`. Measured over 40 matches at level 14 with the whole-tick
    /// relocation census, `resolve_corner_contest` alone was **1.9
    /// relocations a match at a mean of 25 m, every one of them large
    /// enough for a replay to show** — the largest thing left in the
    /// engine moving the ball with no flight under it that is not a
    /// restart placing a dead ball on its spot. That is the "the ball
    /// teleports on corners" report, exactly.
    ///
    /// The duel stays where it was. What changes is that its result is
    /// now delivered by [`Ball::ballistic_launch_arriving_at`], which
    /// solves the arc that puts the ball on the winner's head at
    /// `arrival_height` **on the way down**, and the outcome is applied
    /// when the ball gets there. See [`AerialDelivery`].
    ///
    /// `behind` is how far short of the winner the ball is aimed, in
    /// units — the two contests use different values and the difference
    /// is load-bearing (a midfielder's heading reach is 2.0u, which the
    /// corner's own 2.0u drop sat exactly on the boundary of).
    fn deliver_to_winner(
        field: &mut MatchField,
        winner_idx: usize,
        attacked_goal: Vector3<f32>,
        previous_owner: Option<u32>,
        behind: f32,
        apex: f32,
        outcome_is_header: bool,
        force_heading: bool,
    ) {
        /// Head height, in metres. One tick above the intercept window,
        /// which is what the corner path's own comment sized it at.
        const HEADING_HEIGHT: f32 = 2.5;
        /// Ticks of slack past the solved flight before the delivery is
        /// abandoned. Half a second: the winner is running while the ball
        /// is in the air, so the arrival test has to tolerate him being a
        /// stride from where the arc was solved to.
        const GRACE_TICKS: u64 = 50;

        let winner_pos = field.players[winner_idx].position;
        let winner_id = field.players[winner_idx].id;
        let to_goal = attacked_goal - winner_pos;
        let dir = if to_goal.magnitude() > 0.01 {
            to_goal.normalize()
        } else {
            Vector3::new(1.0, 0.0, 0.0)
        };
        let target = Vector3::new(
            winner_pos.x - dir.x * behind,
            winner_pos.y - dir.y * behind,
            HEADING_HEIGHT,
        );
        // The calibrated hang, unchanged: −0.02 m/tick walks the ball down
        // through the [1.4, 2.5] heading band over ~40 ticks and 0.12
        // u/tick of goalward drift keeps it inside the 6u header reach for
        // all of them, so ANY winner's state machine gets a valid tick.
        let outcome = if outcome_is_header {
            AerialOutcome::Header {
                drift: Vector3::new(dir.x * 0.12, dir.y * 0.12, -0.02),
            }
        } else {
            AerialOutcome::HookedBehind {
                attacked_goal,
                field_height: field.size.height as f32,
            }
        };

        let b = &mut field.ball;
        b.current_owner = None;
        b.previous_owner = previous_owner;
        if outcome_is_header {
            // Every heading state reads this to take a clean-contact roll
            // instead of re-deciding the duel. Set at the strike rather
            // than on arrival because the winner's own state machine uses
            // it to decide to go and attack the ball in the first place.
            b.aerial_contest_winner = Some(winner_id);
        }

        match Ball::ballistic_launch_arriving_at(b.position, target, apex) {
            Some((velocity, ticks)) => {
                #[cfg(feature = "match-logs")]
                tc::TeleportCensus::note_delivery_armed(ticks);
                b.velocity = velocity;
                // Hold the loose-ball machinery off for the whole flight:
                // `in_flight_state > 0` is what keeps `check_ball_ownership`
                // from handing a travelling delivery to whoever is nearest.
                b.flags.in_flight_state = ticks as usize + GRACE_TICKS as usize;
                b.aerial_delivery = Some(AerialDelivery {
                    winner_id,
                    target,
                    outcome,
                    arrival_height: HEADING_HEIGHT,
                    deadline_tick: b.current_tick_cached + ticks as u64 + GRACE_TICKS,
                    force_heading,
                });
            }
            None => {
                // The ball is already standing on the target — there is no
                // arc to solve and nothing to fly. Apply the outcome now;
                // the "relocation" is under a unit.
                b.velocity = match outcome {
                    AerialOutcome::Header { drift } => drift,
                    AerialOutcome::HookedBehind {
                        attacked_goal,
                        field_height,
                    } => Ball::hook_behind_velocity(b.position, attacked_goal, field_height),
                };
                b.flags.in_flight_state = 1;
                // There is no delivery to carry the heading transition, so
                // it is stashed straight away — the arrival is now.
                if force_heading {
                    b.pending_aerial_strike = Some(winner_id);
                }
            }
        }
    }

    fn hook_it_behind(field: &mut MatchField, from: Vector3<f32>, attacked_goal: Vector3<f32>) {
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::HEADED_BEHIND_FIRED.fetch_add(1, Ordering::Relaxed);
        let field_height = field.size.height as f32;
        // The geometry lives on `Ball` so this and the arrival of an
        // `AerialDelivery` that resolved to `HookedBehind` strike the
        // same clearance. See `Ball::hook_behind_velocity`.
        let velocity = Ball::hook_behind_velocity(from, attacked_goal, field_height);

        let b = &mut field.ball;
        // ⚠ NO POSITION WRITE. The only caller left passes the BALL's own
        // position as `from` (`resolve_cross_contest`'s cleared branch),
        // so the header happens where the ball is. The corner contest used
        // to pass the CLEARER's position with the ball still at the flag,
        // which wrote it the width of the box in one tick; that path now
        // flies the delivery to him first.
        b.velocity = velocity;
        b.current_owner = None;
        b.flags.in_flight_state = 1;
        b.pass_target_player_id = None;
        b.clear_pending_pass_metadata();
    }

    fn heads_it_behind(
        ball_pos: Vector3<f32>,
        attacked_goal: Vector3<f32>,
        field_width: f32,
        context: &mut MatchContext,
    ) -> bool {
        /// Outside this there is always a way out. 130u ≈ 16 m.
        const BEHIND_DEPTH: f32 = 130.0;
        /// Share that goes behind when the header is right on the line.
        const BEHIND_AT_LINE: f32 = 0.55;

        let depth = (ball_pos.x - attacked_goal.x).abs();
        if depth > BEHIND_DEPTH || field_width <= 0.0 {
            return false;
        }
        // 1.0 on the goal line, 0 at the edge of the window.
        let urgency = 1.0 - depth / BEHIND_DEPTH;
        context.rng.bernoulli(BEHIND_AT_LINE * urgency * urgency)
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
            #[cfg(feature = "match-logs")]
            save_accounting_stats::PENDING_LOST_NO_PLAYER
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        };
        let keeper_team = field.players[keeper_idx].team_id;
        let shooter_team = field.players[shooter_idx].team_id;
        if keeper_team == shooter_team {
            #[cfg(feature = "match-logs")]
            save_accounting_stats::PENDING_LOST_SAME_TEAM
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        let shot_xg = field.ball.last_shot_xgot;
        // ── Make the save VISIBLE ─────────────────────────────────────
        //
        // The physics save resolves a shot entirely inside ball physics:
        // it changes ball state, credits the stats, and never touches the
        // keeper's state machine. So he made ~86 saves a match while
        // `Goalkeeper: Diving` sat below 0.25% of his ticks — the ball
        // simply stopped at a standing man, which is the "he doesn't
        // catch anything, he just sits on it" report.
        //
        // Put him into the state the save actually demanded. `reach_ratio`
        // is how far he had to stretch, and it is already the quantity the
        // save model scores, so the state and the physics agree about how
        // hard the save was rather than rolling it twice.
        {
            /// Beyond this he has left his feet. 0.22 of full stretch is
            /// roughly a step and a reach — anything past that is a dive,
            /// which is most saves a keeper makes. Only the ball hit
            /// straight at him is taken standing.
            const DIVE_STRETCH: f32 = 0.22;
            let reach = field.ball.pending_save_reach;
            let held = field.ball.current_owner == Some(keeper_id);
            let next = if reach >= DIVE_STRETCH {
                // Full-stretch — he goes to ground whether he holds it or
                // pushes it away.
                GoalkeeperState::Diving
            } else if held {
                // Straight at him and gathered: a clean catch.
                GoalkeeperState::Catching
            } else {
                // Straight at him and NOT held — he got something behind
                // it and the rebound is live. That is a parry, and
                // `Punching` is the state for it; `PreparingForSave`
                // would say he is still waiting for a shot he has already
                // stopped.
                GoalkeeperState::Punching
            };
            // …UNLESS HE IS ALREADY DOING IT.
            //
            // A keeper who left his feet during the flight (see
            // `KeeperShotDive`) is ALREADY in `Diving` when the ball
            // reaches him, and `transition_to` RESETS `in_state_time` — so
            // re-issuing the same state here restarted his dive timer at
            // the moment of contact and pinned him to the floor for another
            // full dive on top of the one he had just made. It also
            // double-counted him in the action census, which is how the
            // dive count came out at more than twice the number of saves.
            //
            // This site exists to make an INVISIBLE save visible. When the
            // save is already visible it has nothing to add, and the
            // crediting below carries on exactly as before.
            let already = field.players[keeper_idx].state == PlayerState::Goalkeeper(next);
            if !already {
                #[cfg(feature = "match-logs")]
                {
                    crate::mid_run_diag::KeeperSweepDiag::note_exit(match next {
                        GoalkeeperState::Diving => 1,
                        GoalkeeperState::Catching => 3,
                        _ => 4,
                    });
                    // …and into the action census as well. This site does
                    // not go through `PlayerMatchState::process`, so the
                    // counters there never see it — leaving the physics
                    // save, which is where most of a keeper's dives come
                    // from, out of the one table that reports how often he
                    // dives.
                    crate::mid_run_diag::KeeperActionDiag::note(match next {
                        GoalkeeperState::Diving => 0,
                        GoalkeeperState::Punching => 2,
                        _ => usize::MAX,
                    });
                }
                let gk = &mut field.players[keeper_idx];
                gk.transition_to(
                    PlayerState::Goalkeeper(next),
                    TransitionSource::EventHandler,
                );
            }
        }
        field.ball.pending_save_reach = 0.0;
        // Read the outcome BEFORE resetting it — the accounting block at the
        // bottom of this function needs it, and resetting here first is why
        // the table kept reporting `parry 0` while the parry branch was
        // demonstrably firing 3662 times per 200 matches.
        let save_site = field.ball.pending_save_site;
        field.ball.pending_save_site = 1;
        let _ = save_site; // only read by the `match-logs` accounting below
        {
            let gk = &mut field.players[keeper_idx];
            // The GK denied a shot worth `shot_xg` xG — books the save,
            // the shot faced, and both xG ledgers in one call so they
            // cannot drift apart (see `note_shot_faced`).
            gk.statistics.note_shot_faced(shot_xg, true);
        }
        field.players[shooter_idx].memory.credit_shot_on_target();
        // Shot has resolved (saved). Drop the metadata so any
        // subsequent goal / save event can't double-credit.
        field.ball.clear_shot_metadata();
        field.ball.pending_error_to_shot_player_id = None;
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            // Book it under the outcome the physics actually produced —
            // catch, or either flavour of parry. This used to hard-code the
            // "catch" bucket because the outcome wasn't carried across, so
            // the table read `parry 0` and looked like parried shots were
            // never credited at all. See `Ball::pending_save_site`.
            let site = (save_site as usize).min(save_accounting_stats::SITE_LABELS.len() - 1);
            save_accounting_stats::SAVES_CREDITED[site].fetch_add(1, Ordering::Relaxed);
            save_accounting_stats::ON_TARGET_PAIRED[site].fetch_add(1, Ordering::Relaxed);
            // `note_shot_faced` was called above, so this column has to move
            // with it — the physics path used to leave it behind, which is
            // why `shots_faced` matched `saves` only by accident.
            save_accounting_stats::SHOTS_FACED_INC[site].fetch_add(1, Ordering::Relaxed);
            save_accounting_stats::PENDING_DELIVERED.fetch_add(1, Ordering::Relaxed);
        }
    }
}
