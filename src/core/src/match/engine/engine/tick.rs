use super::phase_prof::PhaseProf;
use super::*;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::engine::player::events::players::FoulResolver;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::passing::CrossType;
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
            tick_ctx.update_for_goalkeeper_shot(field, &context.players);
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

        for player in field.players.iter_mut().filter(|p| !p.is_sent_off) {
            // Move first, then clamp — see `MatchPlayer::update`.
            player.move_to();
            player.check_boundary_collision(context);
            #[cfg(feature = "match-logs")]
            player.trace_motion(context, ball_pos, ball_vel);
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

        let t = prof_on.then(Instant::now);
        Self::play_ball(field, context, tick_ctx, events);
        Self::apply_pending_set_piece_teleport(field);
        Self::apply_pending_save_credit(field);
        Self::resolve_corner_contest(field, context);
        Self::resolve_cross_contest(field, context);
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

        #[cfg(feature = "match-logs")]
        Self::sample_defensive_shape(field, context);

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

        // Base eased 0.36 → 0.31 in the 2026-08 state-repair
        // recalibration. 0.36 was set while the loose-ball override could
        // still yank the winning header off the dropped ball mid-attempt;
        // headers are committed actions now and complete every time, so
        // the same win rate converts to ~35% more corner goals (DEF
        // corner headers on goal 536 → 708 per 200 matches, DEF goal
        // share 14.5% → 18.6% against the real ~10%).
        let att_win =
            (0.100 + (att_score - best_def_score) * 0.50 - gk_command * 0.18).clamp(0.04, 0.36);

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
            //
            // Restated for the metres-per-tick vertical axis: −0.02 m/tick
            // is 2 m/s of descent, which walks the ball down through the
            // [1.4, 2.5] heading band over ~40 ticks, and 0.12 u/tick of
            // goalward drift keeps it inside the 6u header reach for all of
            // them. The old (−0.35, 1.8) pair was written when the vertical
            // axis carried unit-scale speeds and would now fall through the
            // band in three ticks while drifting 60u out of reach.
            let b = &mut field.ball;
            b.position = Vector3::new(winner_pos.x - dir.x * 2.0, winner_pos.y - dir.y * 2.0, 2.5);
            b.velocity = Vector3::new(dir.x * 0.12, dir.y * 0.12, -0.02);
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
            if !pos.is_defender() || pos.is_goalkeeper() {
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
                    DefenceDiag::note_duel((d.position - man.position).magnitude(), line);
                }
            }
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
        if ball.position.z > 2.9 || ball.position.z < 1.5 || ball.velocity.z > 0.0 {
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
            let b = &mut field.ball;
            b.position.z = 0.6;
            b.velocity = Vector3::new(b.velocity.x * 0.25, b.velocity.y * 0.25, 0.0);
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
            let winner_pos = field.players[att_idx].position;
            let to_goal = attacked_goal - winner_pos;
            let dir = if to_goal.magnitude() > 0.01 {
                to_goal.normalize()
            } else {
                Vector3::new(1.0, 0.0, 0.0)
            };
            let winner_id = field.players[att_idx].id;
            // Force the winner into his heading state, exactly as the
            // corner set-up forces `AttackingCorner` and for the same
            // documented reason: "not all of them carry the entry hook".
            // A cross winner may have been in any of a dozen states when
            // the delivery arrived, and the running-state entry gate that
            // would normally hand off to `Heading` has its own range and
            // ball-angle conditions. Leaving the transition to chance is
            // why the contest could be won 307 times and produce zero
            // headers — the contest decided a duel nobody then took.
            {
                let winner = &mut field.players[att_idx];
                let next = if winner.tactical_position.current_position.is_forward() {
                    PlayerState::Forward(ForwardState::Heading)
                } else if winner.tactical_position.current_position.is_midfielder() {
                    PlayerState::Midfielder(MidfielderState::Heading)
                } else {
                    PlayerState::Defender(DefenderState::Heading)
                };
                winner.transition_to(next, TransitionSource::EventHandler);
            }
            let b = &mut field.ball;
            // 1.2u (15 cm) behind the winner — inside every role's heading
            // reach, including the midfielder's 2.0u, which the corner
            // contest's 2.0u drop sits exactly on the boundary of.
            b.position = Vector3::new(winner_pos.x - dir.x * 1.2, winner_pos.y - dir.y * 1.2, 2.5);
            b.velocity = Vector3::new(dir.x * 0.12, dir.y * 0.12, -0.02);
            b.current_owner = None;
            b.previous_owner = crosser;
            b.flags.in_flight_state = 1;
            // The contest already decided this duel — tell the winner's
            // heading state not to roll it again.
            b.aerial_contest_winner = Some(winner_id);
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
            b.position.z = 2.2;
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
        let shot_xg = field.ball.last_shot_xgot;
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
            // Re-use the "catch" site bucket — physics-save outcomes are
            // catches, parries, and dangerous parries indistinguishably
            // from the stats viewpoint. The save_pipeline counters above
            // already separate them at the physics layer.
            save_accounting_stats::SAVES_CREDITED[1].fetch_add(1, Ordering::Relaxed);
            save_accounting_stats::ON_TARGET_PAIRED[1].fetch_add(1, Ordering::Relaxed);
        }
    }
}
