//! Restart resolution: throw-ins on the touchlines and the lifetime
//! management of the in-flight offside snapshot. Endline restarts
//! (corner / goal kick) live in `goal.rs` because they share machinery
//! with the actual goal check.
//!
//! Real football rule the ball is restarted by the team that did NOT
//! last touch the ball. If we have no record of a touch (kickoff /
//! genuine bug) we fall back to the team currently holding ownership;
//! if even that is missing, we leave the boundary inset as the safety
//! net for `check_boundary_collision`.

use super::runoff::ExitAxis;
use super::{AwaitedRestart, Ball, RunOff};
use crate::PlayerFieldPositionGroup;
use crate::r#match::PassOriginRestart;
use crate::r#match::ball::events::BallEvent;
use crate::r#match::engine::corner_shape::CornerShape;
use crate::r#match::events::EventCollection;
use crate::r#match::{MatchContext, MatchPlayer, PlayerSide};
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::RestartCensus;
use nalgebra::Vector3;

impl Ball {
    /// How often the taker is re-sent for a ball he has not reached yet,
    /// in engine ticks. 40 = 0.4 s: often enough that losing him to
    /// another state costs a fraction of a second, rare enough that it is
    /// not a signal every tick.
    const NUDGE_TICKS: u64 = 40;

    /// Touchline check: if the ball crossed y<=0 or y>=field_height, set
    /// up a throw-in for the team that did NOT last touch it. Routes
    /// through `pending_set_piece_teleport` like corners / goal kicks so
    /// the engine can place the thrower onto the ball after the tick.
    pub(crate) fn check_throw_in(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // Already resolved this tick (goal scored, etc.), or the ball is
        // still in the goal from the last one — or it is already out of
        // play and running out past the line, in which case awarding it
        // again on every tick of the run-out would keep resetting the
        // patience clock. See [`RunOff`](super::RunOff).
        if self.goal_scored || self.in_net.is_some() || self.awaiting_restart.is_some() {
            return;
        }

        // ⚠ **A ball somebody is HOLDING is not a ball that has gone out.**
        //
        // This never needed saying while the player clamp pinned everybody
        // inside the rectangle: the only way an owned ball reached `y <= 0`
        // was an owner standing exactly on the touchline, and even that
        // handed a throw-in to the other side against a man who was still
        // dribbling. It needs saying now, because the taker of a restart is
        // allowed out into the run-off — so a thrower who has just picked
        // the ball up two metres past the line would award a throw-in
        // against his own team, once per tick, all the way back in.
        //
        // Same rule `check_wide_of_goal` states for the byline.
        if self.current_owner.is_some() {
            return;
        }
        let field_height = context.field_size.height as f32;
        let crossed_top = self.position.y <= 0.0;
        let crossed_bottom = self.position.y >= field_height;
        if !crossed_top && !crossed_bottom {
            return;
        }

        // Last toucher's side decides which team gets the throw-in.
        let last_toucher_side = self
            .last_touch_player_id
            .or(self.previous_owner)
            .or(self.current_owner)
            .and_then(|pid| players.iter().find(|p| p.id == pid))
            .and_then(|p| p.side);

        let throwing_side = match last_toucher_side {
            Some(PlayerSide::Left) => PlayerSide::Right,
            Some(PlayerSide::Right) => PlayerSide::Left,
            None => return, // Safety net: let boundary_collision handle it.
        };

        // **Where it crossed, not where it now is.** Law 15 takes the throw
        // from the point the ball crossed the touchline, and this resolver
        // runs immediately after `move_to`, so `self.position` is that
        // point plus whatever was left of the tick — up to a metre down
        // the line on a hard-struck ball. `line_crossing` solves the tick
        // back; it declines the cases where that reconstruction is not
        // valid and this falls back to the position, as it always did.
        //
        // Inset two units inside the line so no boundary test can read the
        // spot itself as out.
        let field_width = context.field_size.width as f32;
        let touchline = if crossed_top { 0.0 } else { field_height };
        let crossed_at = self
            .line_crossing(ExitAxis::Touchline, touchline)
            .map(|p| p.x)
            .unwrap_or(self.position.x);
        let inset = AwaitedRestart::SPOT_INSET;
        let throw_x = crossed_at.clamp(inset, field_width - inset);
        let throw_y = if crossed_top {
            inset
        } else {
            field_height - inset
        };
        let throw_pos = Vector3::new(throw_x, throw_y, 0.0);

        let thrower = ThrowIn::pick_thrower(players, throwing_side, throw_pos);
        let thrower_id = match thrower {
            Some(id) => id,
            None => return,
        };

        // Sampled BEFORE the mutations below, all of which rewrite the very
        // fields the census is asking about — `record_touch` in particular
        // stamps this tick, which would make every gap read as zero.
        #[cfg(feature = "match-logs")]
        {
            // Do the two answers to "who put it out" agree? They are the
            // last TOUCH and the last OWNER, and when they disagree the
            // award is decided by whichever one happens to be non-null —
            // see the note on the fallback chain above.
            let owner_side = self
                .previous_owner
                .or(self.current_owner)
                .and_then(|pid| players.iter().find(|p| p.id == pid))
                .and_then(|p| p.side);
            let disagree = owner_side.is_some_and(|s| Some(s) != last_toucher_side);
            let since_last = context.current_tick().saturating_sub(self.last_touch_tick);
            let walk = players
                .iter()
                .find(|p| p.id == thrower_id)
                .map(|p| (p.position - throw_pos).magnitude())
                .unwrap_or(0.0);
            RestartCensus::note_throw_in(disagree, since_last, walk);
        }

        // **The ball is NOT written onto the spot.** It has just crossed
        // the touchline and it goes on going — out across the run-off
        // until the hoardings stop it — and the thrower fetches it from
        // wherever that leaves it and brings it back to `throw_pos`. See
        // [`RunOff`]; `OF_RUN_OUT=off` restores the snap.
        let runs_out = RunOff::armed();
        #[cfg(feature = "match-logs")]
        super::frame_trace::FrameTrace::note(format!(
            "check_throw_in: THROW-IN, ball ({:.1}, {:.1}, {:.2}) {} ({:.1}, {:.1}) taker {thrower_id}",
            self.position.x,
            self.position.y,
            self.position.z,
            if runs_out {
                "runs out; taken from"
            } else {
                "->"
            },
            throw_pos.x,
            throw_pos.y
        ));
        if !runs_out {
            self.position = throw_pos;
            self.velocity = Vector3::zeros();
        }

        // OUT OF PLAY, and nobody's. The taker has to go and get it — see
        // [`AwaitedRestart`]. Handing him the ball here and teleporting him
        // onto it is what made a player appear on the touchline out of
        // nowhere on every throw-in in the match.
        self.previous_owner = self.current_owner;
        self.current_owner = None;
        self.ownership_duration = 0;
        self.claim_cooldown = 0;
        self.flags.in_flight_state = 0;
        self.pass_target_player_id = None;
        self.recent_passers.clear();
        self.cached_shot_target = None;
        self.offside_snapshot = None;
        // Dead ball — see `clear_open_play_metadata`.
        self.clear_open_play_metadata();
        self.pass_origin_restart = PassOriginRestart::ThrowIn;
        // A carry that was still running when the ball went out ends HERE,
        // at the touchline. The whole ball update is skipped while the
        // restart waits (see `Ball::update`), so without this the carry
        // stays open for the seconds the taker spends walking and is then
        // reported as having ended on the tick play resumed.
        self.tick_carry_tracker(events);

        // While the ball runs out, `spot` is provisional and follows it;
        // `take_from` is the touchline point the throw is legally taken
        // from and never moves. Switched off, the ball is already on that
        // point and there is nothing to fetch it from — the old
        // single-leg shape, unchanged.
        #[cfg(feature = "match-logs")]
        if runs_out {
            RestartCensus::note_run_out_begun();
        }
        self.awaiting_restart = Some(AwaitedRestart {
            take_from: runs_out.then_some(throw_pos),
            settled: !runs_out,
            carrying: false,
            taker_id: thrower_id,
            spot: if runs_out { self.position } else { throw_pos },
            origin: PassOriginRestart::ThrowIn,
            awarded_tick: context.current_tick(),
            patience_ticks: AwaitedRestart::PATIENCE_TICKS,
            settled_tick: None,
        });
        // Send him. `TakeMe` is the engine's existing "go and get the
        // ball" signal (`MatchPlayer::run_for_ball`), so the taker runs
        // there under ordinary AI and ordinary stamina rather than being
        // placed there by a set-piece teleport.
        events.add_ball_event(BallEvent::TakeMe(thrower_id));
    }

    /// One tick of a dead ball waiting for its taker.
    ///
    /// Pins the ball where it lies, hands it over the moment he arrives,
    /// and falls back to the old teleport if he never does. Runs BEFORE
    /// ownership so nothing can claim a ball that is out of play.
    ///
    /// Which goal a corner taken from `spot` is attacking, as an x
    /// coordinate. The kick is taken from the byline of the goal being
    /// attacked, so the nearer end is the answer.
    fn corner_attacked_goal_x(&self, spot: Vector3<f32>) -> f32 {
        if spot.x < self.field_width * 0.5 {
            0.0
        } else {
            self.field_width
        }
    }

    /// Attackers standing in the penalty area, excluding the taker and
    /// the goalkeeper. The census the corner set-up waits on.
    fn attackers_in_the_box(
        players: &[MatchPlayer],
        attacking_team: u32,
        taker_id: u32,
        goal_x: f32,
        field_height: f32,
    ) -> usize {
        players
            .iter()
            .filter(|p| {
                p.team_id == attacking_team
                    && p.id != taker_id
                    && !p.is_sent_off
                    && !p.tactical_position.current_position.is_goalkeeper()
                    && CornerShape::is_in_penalty_area(p.position, goal_x, field_height)
            })
            .count()
    }

    /// A restart with an [`AwaitedRestart::take_from`] — the corner, and
    /// only the corner — runs the same thing TWICE: he fetches the ball
    /// from wherever it went out, and then carries it to the arc.
    pub(crate) fn tick_awaited_restart(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        let Some(mut await_state) = self.awaiting_restart else {
            return;
        };
        // Denominator for the dead-ball census: "twelve refused touches a
        // match" means nothing without how long the ball is dead for.
        #[cfg(feature = "match-logs")]
        RestartCensus::note_dead_ball_tick();

        let Some(taker) = players.iter().find(|p| p.id == await_state.taker_id) else {
            // Substituted or sent off between the award and now. Drop the
            // restart and let the ordinary loose-ball logic have it rather
            // than waiting for a man who is not on the pitch.
            //
            // ⚠ **But not while the ball is off the pitch.** Handing a
            // loose ball back used to be harmless because it was lying on
            // the restart spot, two units INSIDE the line. It can now be
            // four metres beyond the hoardings ([`RunOff`]), where the
            // pitch clamp on every other player forbids anyone from ever
            // reaching it — the ball would sit out there, unowned and
            // untouchable, until a stall detector eventually kicked it
            // away. Put it on the spot the restart was awarded at, which
            // is the same backstop a taker who never arrives gets.
            let spot = await_state.take_from.unwrap_or(await_state.spot);
            if RunOff::armed() {
                self.position.x = spot.x;
                self.position.y = spot.y;
                self.velocity = Vector3::zeros();
                self.spin = Vector3::zeros();
            }
            self.awaiting_restart = None;
            return;
        };

        // ── STILL RUNNING OUT ────────────────────────────────────────
        //
        // The ball crossed the line on the tick the restart was awarded
        // and it has not finished travelling. It is out of play from that
        // instant — nobody may touch it, nothing may claim it — but it is
        // emphatically not stationary, and pinning it here is exactly the
        // reported *"the ball stops on the line"*. So the ordinary physics
        // runs, the perimeter bounds it, and only when it comes to rest
        // does the pin below take over. See [`RunOff`].
        //
        // The taker was sent at the award and is already on his way, so
        // the run-out costs the restart almost nothing: he is walking
        // across the pitch while the ball is crossing the last four
        // metres of it.
        if !await_state.settled {
            let now = context.current_tick();
            if !self.tick_run_out(&mut await_state, now) {
                self.current_owner = None;
                // Keep sending him — the ball is moving, so a chaser who
                // reached where it WAS has to be told again. Same 40-tick
                // cadence as the wait below.
                if now.saturating_sub(await_state.awarded_tick) % Self::NUDGE_TICKS == 0 {
                    events.add_ball_event(BallEvent::TakeMe(await_state.taker_id));
                }
                self.awaiting_restart = Some(await_state);
                return;
            }
            // It has just stopped. The patience was set at the award
            // against a spot the ball was never on — recompute it against
            // the walk he actually has, or a keeper who has to come four
            // metres further than the award assumed runs out of clock for
            // a reason that has nothing to do with him.
            let left = (taker.position - await_state.spot).magnitude();
            let ceiling = if await_state.origin == PassOriginRestart::Corner {
                AwaitedRestart::corner_patience_for(left)
            } else {
                AwaitedRestart::patience_for(left)
            };
            await_state.patience_ticks = now
                .saturating_sub(await_state.awarded_tick)
                .saturating_add(ceiling);
            // ⚠ **Written back HERE, not at the end.** The pin below
            // returns without storing `await_state` — it never had to,
            // because nothing above it used to mutate the restart — so
            // leaving the write to the fall-through loses both the settle
            // and the new bound, and the ball re-settles from scratch on
            // every subsequent tick. `settled` would never be true in the
            // ball's own copy and the taker's arrival would never be
            // tested.
            self.awaiting_restart = Some(await_state);
        }

        // The ball is out of play: it does not move by itself and it is
        // nobody's, whatever else happened this tick. On the second leg of
        // a corner it moves with the man CARRYING it, which is the one
        // case where a dead ball travels — and it travels at his pace,
        // under his feet, rather than being written onto the arc.
        //
        // …the height settles either way. A throw-in's ball is already on
        // the deck, but an offside is flagged the moment the receiver
        // reaches the ball and that can be at chest height, so writing the
        // spot's `z` in would drop it a couple of metres in one tick — the
        // teleport this whole mechanism exists to avoid, on the one axis
        // where it is always visible. 0.10 m/tick is a ball dying on the
        // grass.
        const SETTLE_RATE: f32 = 0.10;
        /// Where a carried dead ball rides, in metres — `Ball::carry_height`'s
        /// own figure for a ball in a player's hands.
        ///
        /// A restart is fetched BY HAND: the thrower picks the ball up off
        /// the run-off, the keeper carries it in for the goal kick, the
        /// corner taker walks it to the flag. Left settling to the spot's
        /// zero the ball rode along the grass at his feet for the whole
        /// carry, which the viewer draws as a man dribbling rather than
        /// one carrying — `Actors::in_his_hands` wants it between 0.85 and
        /// 1.45 m before it will put it in his arms.
        const CARRY_HEIGHT: f32 = 1.15;
        /// How fast the ball closes the last stride onto the man picking it
        /// up, in units per tick — `Ball::move_to`'s own `BALL_TRACK_SPEED`,
        /// which is what carries a ball along with its owner.
        ///
        /// ⚠ **The carry used to start by writing the ball onto him**, and
        /// [`AwaitedRestart::REACH`] is 12 u, so it started with a jump of
        /// up to a metre and a half. Caught in a recorded track: the ball
        /// lay against the hoardings at x = −36.8 and the next sample had
        /// it at −27.6 with the keeper. He is *bending down to pick it up*
        /// — the last teleport in a sequence that exists to remove them.
        const PICKUP_RATE: f32 = 1.5;
        let height = if await_state.carrying {
            let to_taker = Vector3::new(
                taker.position.x - self.position.x,
                taker.position.y - self.position.y,
                0.0,
            );
            let gap = to_taker.norm();
            if gap > PICKUP_RATE {
                let step = to_taker * (PICKUP_RATE / gap);
                self.position.x += step.x;
                self.position.y += step.y;
            } else {
                self.position.x = taker.position.x;
                self.position.y = taker.position.y;
            }
            CARRY_HEIGHT
        } else {
            self.position.x = await_state.spot.x;
            self.position.y = await_state.spot.y;
            await_state.spot.z
        };
        self.position.z += (height - self.position.z).clamp(-SETTLE_RATE, SETTLE_RATE);
        self.velocity = Vector3::zeros();
        self.current_owner = None;

        let range = (taker.position - await_state.spot).magnitude();
        let waited = context
            .current_tick()
            .saturating_sub(await_state.awarded_tick);
        // ⚠ **A carried ball is handed over only once its carrier is back
        // ON the pitch.**
        //
        // `REACH` is 1.5 m and deliberately generous — the taker is
        // steered by behaviours that settle beside a point rather than
        // landing on it. That is harmless for the fetch, whose spot is
        // wherever the ball lies; it is not harmless for the carry, whose
        // spot is two units inside the line. A man walking in from the
        // run-off satisfies `range <= REACH` while still 1.3 m the wrong
        // side of it, and the ball is at his feet — so the restart would
        // be TAKEN from outside the pitch, and the first thing the throw
        // or the kick did was cross the line again from the wrong side and
        // be awarded to the opposition. He is a stride away and walking;
        // waiting for that stride costs a tenth of a second.
        let clearance = AwaitedRestart::IN_PLAY_CLEARANCE;
        let on_the_pitch = !await_state.carrying
            || (taker.position.x >= clearance
                && taker.position.x <= self.field_width - clearance
                && taker.position.y >= clearance
                && taker.position.y <= self.field_height - clearance);
        let arrived = range <= AwaitedRestart::REACH && on_the_pitch;

        // **The corner waits for the box.**
        //
        // He is on the arc with the ball at his feet and every other
        // restart would be taken now. A corner is not ready: five runners
        // have to arrive from the other half, and taking it the moment the
        // TAKER was ready is the single reason the walked corner was left
        // switched off — the attacking box at the delivery measured 3.5
        // against a placed corner's 5.4 and a real 5-7.
        //
        // ⚠ **Ahead of the patience gate, and it resets the clock.**
        // `patience_ticks` bounds how long he may take to REACH the ball;
        // standing over it is not taking too long, it is the restart
        // working. Put after the gate instead, the wait ran the same
        // counter down and the backstop teleport fired on a man already
        // on the spot: measured **9.5 timed-out legs a match at a mean of
        // 0.8 m still to go**, which is the signature of a bound expiring
        // under somebody standing still rather than of a walk that failed.
        //
        // See [`AwaitedRestart::settled_tick`].
        if arrived && await_state.take_from.is_none() {
            if await_state.origin == PassOriginRestart::Corner {
                let now = context.current_tick();
                let settled_at = *await_state.settled_tick.get_or_insert(now);
                let holding = now.saturating_sub(settled_at);
                let box_full = Self::attackers_in_the_box(
                    players,
                    taker.team_id,
                    await_state.taker_id,
                    self.corner_attacked_goal_x(await_state.spot),
                    self.field_height,
                ) >= AwaitedRestart::CORNER_BOX_TARGET;
                let ceiling = holding >= AwaitedRestart::CORNER_SETUP_CEILING;
                if !box_full && !ceiling {
                    // No `TakeMe`: he is standing over the ball, not
                    // running at it, and the nudge would restart a chase
                    // he has already finished.
                    await_state.awarded_tick = now;
                    self.awaiting_restart = Some(await_state);
                    return;
                }
                #[cfg(feature = "match-logs")]
                RestartCensus::note_corner_setup_wait(holding, ceiling && !box_full);
            }
        }

        if !arrived && waited < await_state.patience_ticks {
            // Keep sending him. `run_for_ball` is a no-op for a player who
            // is already chasing, so this cannot restart his chase clock;
            // what it does do is pick him up again if something else — a
            // reset, a shape recall — pulled him out of `TakeBall` on the
            // way, which would otherwise leave the ball waiting for a man
            // who has stopped coming.
            if waited % Self::NUDGE_TICKS == 0 {
                events.add_ball_event(BallEvent::TakeMe(await_state.taker_id));
            }
            return;
        }

        if !arrived {
            // He never got there. The teleport is the backstop, not the
            // behaviour — a restart that never happens stalls the match.
            //
            // A restart that still had a leg to run skips it: the point is
            // to get play going again, and a carry he has not started is
            // not going to run better than the fetch he just failed.
            let final_spot = await_state.take_from.take().unwrap_or(await_state.spot);
            #[cfg(feature = "match-logs")]
            {
                RestartCensus::note_restart_teleport();
                RestartCensus::note_restart_timeout_state(taker.state.compact_id());
                if await_state.origin == PassOriginRestart::GoalKick {
                    RestartCensus::note_goal_kick_teleport(range);
                }
                if await_state.origin == PassOriginRestart::Corner {
                    RestartCensus::note_corner_leg_timeout(range);
                }
            }
            self.pending_set_piece_teleport = Some((await_state.taker_id, final_spot));
            await_state.spot = final_spot;
            self.position.x = final_spot.x;
            self.position.y = final_spot.y;
        } else if let Some(take_from) = await_state.take_from.take() {
            // **He has reached the ball, and the kick is taken somewhere
            // else.** Second leg: he picks it up and walks it to the arc,
            // and the ball goes with him because he is holding it.
            await_state.carrying = true;
            await_state.spot = take_from;
            await_state.awarded_tick = context.current_tick();
            let carry = (taker.position - take_from).magnitude();
            await_state.patience_ticks = AwaitedRestart::corner_patience_for(carry);
            self.awaiting_restart = Some(await_state);
            // His touch, from here on. `clear_expired_corner_stations`
            // releases the shape when anybody OTHER than the taker touches
            // the ball, so the stamp has to be his before the walk starts.
            self.record_touch(
                await_state.taker_id,
                taker.team_id,
                context.current_tick(),
                true,
            );
            // ⚠ **Nothing else can steer him now.** `run_for_ball` sends a
            // man to the ball and the ball is at his feet, so he reads as
            // arrived and stands still; `CornerHold` releases anybody
            // within four metres of it for the same reason. The station is
            // what walks him to the flag — see `CornerHold::hold_weight`.
            self.pending_restart_station = Some((await_state.taker_id, take_from));
            events.add_ball_event(BallEvent::TakeMe(await_state.taker_id));
            #[cfg(feature = "match-logs")]
            RestartCensus::note_restart_carry(carry);
            return;
        }
        #[cfg(feature = "match-logs")]
        RestartCensus::note_restart_taken(waited, arrived);

        self.awaiting_restart = None;
        // Play restarts HERE. The stall detector measures how long the
        // ball has sat in one place, and it was paused rather than reset
        // for the whole wait — so without this the ticks the taker spent
        // walking are charged to whoever receives the throw.
        self.stall_anchor_pos = self.position;
        self.stall_anchor_tick = 0;
        self.current_owner = Some(await_state.taker_id);
        self.ownership_duration = 0;
        self.claim_cooldown = 45;
        self.flags.in_flight_state = 20;
        self.pass_origin_restart = await_state.origin;
        self.record_touch(
            await_state.taker_id,
            taker.team_id,
            context.current_tick(),
            true,
        );
        // **A corner only becomes a corner here** — the taker on the arc
        // with the ball at his feet, several seconds after it was awarded.
        // Both of these used to be armed at the award because that WAS the
        // same tick.
        if await_state.origin == PassOriginRestart::Corner {
            if let Some(shape) = self.corner_shape.as_mut() {
                // The deadline runs from the kick, not from the award, or
                // the walk-in spends it and the shape drops before the
                // cross — see `CornerShapeHold::armed_tick`.
                shape.armed_tick = context.current_tick();
                shape.live_tick = Some(context.current_tick());
            }
            self.corner_contest_resolved = false;
        }
        events.add_ball_event(BallEvent::Claimed(await_state.taker_id));
    }

    /// Drop the offside snapshot once its lifetime expires. Real-world
    /// passes that don't reach a receiver should end the offside
    /// pretence — anything older than ~220 ticks is stale.
    pub(super) fn expire_offside_snapshot(&mut self, context: &MatchContext) {
        const OFFSIDE_LIFETIME_TICKS: u64 = 220;
        if let Some(snap) = self.offside_snapshot {
            let now = context.current_tick();
            if now.saturating_sub(snap.set_tick) > OFFSIDE_LIFETIME_TICKS {
                self.offside_snapshot = None;
                if self.pass_origin_restart != PassOriginRestart::OpenPlay {
                    self.pass_origin_restart = PassOriginRestart::OpenPlay;
                }
            }
        }
    }
}

/// **A dead ball may not be touched, by anybody, including its taker.**
///
/// [`Ball::tick_awaited_restart`] pins the ball on the spot and strips its
/// owner every tick, and [`Ball::update`] returns immediately afterwards —
/// so nothing inside the ball's own update can move a ball that is out of
/// play. Nothing OUTSIDE it knew the rule at all.
///
/// The player layer reaches the ball through its own events, which run
/// after `play_ball` in the same tick (see `TickEngine::game_tick_inner`),
/// and every one of them writes the ball directly: `GainBall` and
/// `BallOwnerChange` route to `secure_ball_for`, which SNAPS THE BALL TO
/// THE PLAYER'S FEET. So a player standing near the spot took the ball off
/// the line, `tick_awaited_restart` yanked it back on the next tick, and
/// the cycle repeated for as long as the restart waited.
///
/// Traced on a throw-in awarded by the corner flag (`dev_match gather`,
/// `OF_FRAME_TRACE=gather,miss`): the defending keeper was 1.2 m from the
/// spot and took the ball nine times in 120 ticks, the ball jumping 1.1 m
/// to his feet and 1.1 m back on alternate ticks.
///
/// ```text
///  270389  801.51 543.00  own -      catch    WAIT   <- pinned on the touchline
/// *270390  802.74 534.01  own G200   clear    WAIT   <- GainBall: snapped to him
/// *270391  801.51 543.00  own -      clear    WAIT   <- pinned again
/// ```
///
/// That is the reported *"the ball bounces off the side of the field and
/// then appears in the goalkeeper's hands"* verbatim, and it is a
/// GOALKEEPER problem in practice for a mundane reason: he is the one
/// player whose states chase a ball he is merely standing next to, and
/// dead balls near his own goal are the ones he is standing next to.
///
/// Enforced in two places on purpose, because that is the split that has
/// held elsewhere in this engine:
/// * `PlayerEventDispatcher::dispatch` refuses every ball-touching event —
///   the backstop, and the reason the artefact cannot come back through a
///   route nobody has thought of. Same allow-list shape as the `in_net`
///   guard directly above it, and for the same reason.
/// * [`KeeperBallClaim::is_favourite`](crate::r#match::goalkeepers::states::
///   common::KeeperBallClaim) refuses to call a dead ball his, so he does
///   not walk at it in the first place. Without that half he still shuttles
///   between `Catching`, `Clearing` and `Standing` beside a ball he can
///   never have — the state churn is the same shape the sweep-limit and
///   catch-release bands were widened to remove.
///
/// The taker is NOT exempt. He acquires the ball through
/// `tick_awaited_restart` the moment he arrives, and that is the only
/// legitimate route: letting him take it through an event would teleport
/// the ball to him from wherever he had got to, which is the whole
/// artefact [`AwaitedRestart`] exists to remove.
///
/// `OF_DEAD_BALL=off` restores the old behaviour as the A/B.
pub struct DeadBall;

impl DeadBall {
    /// False when `OF_DEAD_BALL=off` — a dead ball is touchable again,
    /// exactly as it used to be.
    pub fn armed() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            std::env::var("OF_DEAD_BALL")
                .map(|v| v != "off" && v != "0")
                .unwrap_or(true)
        })
    }

    /// The man a pending restart is waiting for, or `None` when the ball is
    /// in play (or the guard is switched off).
    ///
    /// Takes the taker rather than the ball so both sides can use it: the
    /// engine passes `ball.awaiting_restart.map(|r| r.taker_id)` and the
    /// player states pass `BallMetadata::restart_taker`, which is the same
    /// value copied into their view of the tick.
    #[inline]
    pub fn taker(restart_taker: Option<u32>) -> Option<u32> {
        if !Self::armed() {
            return None;
        }
        restart_taker
    }

    /// Is the ball out of play right now?
    #[inline]
    pub fn is_dead(restart_taker: Option<u32>) -> bool {
        Self::taker(restart_taker).is_some()
    }
}

/// **Whether a corner is walked or placed.**
///
/// The corner used to be three teleports on one tick: the ball jumped to
/// the arc (a mean 220u, **27.5 m** — the largest relocation left in the
/// engine), the taker was teleported onto it, and `CornerShape` wrote all
/// twenty other players into the set-piece shape. Then the cross was
/// struck 50 ms later.
///
/// Armed, the taker fetches the ball from wherever it went out and carries
/// it to the arc ([`AwaitedRestart::take_from`]), which takes several
/// seconds — and those seconds are the stoppage the shape needed all
/// along, so the twenty walk into it under `CornerHold` instead of being
/// written there.
///
/// # ⚠ ON BY DEFAULT since 2026-08-21. `OF_CORNER_WALK=off` restores the
/// placed corner.
///
/// It was off, and the reasons were measured and real: the attacking box
/// at the delivery read **3.0 against a placed corner's 5.4**, and **44% of
/// corners never completed the fetch**, falling back to the backstop
/// teleport this exists to remove. Four separate faults were behind those
/// two numbers, none of them the walk itself. Each is documented at its own
/// site; together they are why the trade has changed:
///
/// * **The corner arc was inset 2 u, not [`AwaitedRestart::SPOT_INSET`].**
///   Its spot sat exactly on `IN_PLAY_CLEARANCE`, so the arrival gate
///   refused a taker standing on it. That is the whole of the "44% never
///   complete the fetch": 6.98 legs a match timed out **a mean of 0.3 m
///   short**. Now 0.13/match. See `Ball::check_wide_of_goal`.
/// * **`CornerHold` imposed a velocity without an effort floor**, so the
///   twenty were steered to their stations at the speed cap of whatever
///   low-intensity state they were drifting in — `Standing` is `Recovery`,
///   0.12 of top speed. See `CornerHold::apply`.
/// * **`is_team_attacking_corner` answered FALSE during the whole set-up**,
///   because it asks who last touched the ball and the last toucher at a
///   corner is by definition an opponent. `DefenderAttackingCornerState`
///   bails to `Returning` on that, so the two pushed-up centre-backs turned
///   round and ran back the tick the corner was awarded. Invisible while
///   corners were placed — they were teleported into the box 50 ms later.
/// * **The kick was taken the moment the TAKER was ready**, not when the
///   box was. See [`AwaitedRestart::settled_tick`].
///
/// Measured after all four, **3 runs × 100 matches an arm**:
///
/// | per match | placed | walked |
/// |---|---|---|
/// | goals | 2.56 | 2.60 |
/// | shots / team | 12.83 | 12.90 |
/// | saves / on-target | 67.3% | 66.5% |
/// | corners | 7.57 | 7.30 |
/// | box at the kick, defenders | 7.0 | 7.0 |
/// | box at the kick, **attackers** | **5.43** | **4.93** |
///
/// Everything but the last row is inside run-to-run noise (the placed arm's
/// own corner rate spans 7.2-8.1 across its three runs). The last row is the
/// honest cost: half a body.
///
/// **What it buys.** A placed corner writes the ball 16.4 m onto the arc,
/// teleports the taker onto it, and writes **seventeen other players a mean
/// of 30.3 m each** — worst case 84 m — all on one tick. Measured over 12
/// matches that is ~4,950 m of visible relocation per match against the
/// ball's own ~270 m for the entire rest of the engine. The corner is not
/// merely the largest teleport in the match; it is larger than everything
/// else put together, by a factor of eighteen, and the ball is 2.5% of it.
///
/// Half an attacker in the box is worth that — and the 5.43 the placed arm
/// posts is not a quality the simulation earned, it is seventeen men being
/// written into position.
/// The A/B arm for walking the taker to a free kick or a penalty.
///
/// The foul restart was the last one in the engine still handing its taker
/// the ball on the spot and teleporting him onto it — every other restart
/// (throw-in, goal kick, corner, offside free kick) awards an
/// [`AwaitedRestart`] and sends him to walk. Armed, so does this one.
///
/// It carries its own switch because it is the one restart conversion that
/// changes how long the ball is DEAD, and dead time is upstream of fouls,
/// shots and the save rate. `OF_FOUL_WALK=off` restores the teleport.
pub struct FoulWalk;

impl FoulWalk {
    /// False only when `OF_FOUL_WALK=off`.
    pub fn armed() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            std::env::var("OF_FOUL_WALK")
                .map(|v| v != "off" && v != "0")
                .unwrap_or(true)
        })
    }
}

pub struct CornerWalk;

impl CornerWalk {
    /// False only when `OF_CORNER_WALK=off` — the ball, the taker and the
    /// shape are all placed again, exactly as they used to be.
    pub fn armed() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            std::env::var("OF_CORNER_WALK")
                .map(|v| v != "off" && v != "0")
                .unwrap_or(true)
        })
    }
}

/// Throw-ins: who takes them, and what the taker's `long_throws`
/// attribute actually buys him. Consumed by
/// `PassEvaluator::long_throw_target` when deciding whether a ball into
/// the box is on and who is inside range of it.
pub struct ThrowIn;

impl ThrowIn {
    /// Below this the attribute is not a long-throw specialism, it is
    /// just an ordinary throw-in.
    const SPECIALIST_BAR: f32 = 14.0;

    /// Delivery range in field units, as `(min, max)`.
    ///
    /// 140u baseline + up to 180u extra at long_throws=20 — at this
    /// engine's 1u ≈ 0.125 m that is 17.5 m for an ordinary thrower
    /// rising to ~40 m for a specialist, which is the real band (Rory
    /// Delap's were ~35-40 m). The previous 35u/70u pair was written
    /// against a ~0.5 m/unit field and reads as 4-9 m today: shorter
    /// than a square ball to the corner flag, so no throw could ever
    /// have reached a target in the box through it.
    ///
    /// Minimum useful range 12u — anything shorter is a recycle pass.
    pub fn range(long_throws_skill: f32) -> (f32, f32) {
        let scaled = (long_throws_skill / 20.0).clamp(0.0, 1.0);
        let max_range = 140.0 + scaled * 180.0;
        (12.0, max_range)
    }

    /// Whether the player can deliver a "long throw into the box" — only
    /// in the attacking third, and only with a genuine long-throw skill.
    pub fn can_reach_box(long_throws_skill: f32, in_attacking_third: bool) -> bool {
        in_attacking_third && long_throws_skill >= Self::SPECIALIST_BAR
    }

    /// Score-based thrower selection.
    ///   0.50 distance (closer to the throw point is better)
    ///   0.18 position fit (fullback / wing-back / wide mid preferred)
    ///   0.16 long_throws scaled
    ///   0.08 decisions scaled
    ///   0.05 technique scaled
    ///   0.03 strength scaled
    ///
    /// ⚠ **Distance carries half of it now, and it did not used to.** At
    /// 0.35 against a 0.25 position fit, a centre-back twenty-five metres
    /// away beat a winger standing on the line — which cost nothing while
    /// the taker was TELEPORTED to the ball, and costs several seconds of
    /// match now that he has to walk (see [`AwaitedRestart`]). Whoever the
    /// ball went out next to takes it, unless somebody with a real
    /// long throw is also close.
    fn pick_thrower(
        players: &[MatchPlayer],
        throwing_side: PlayerSide,
        throw_pos: Vector3<f32>,
    ) -> Option<u32> {
        let mut best: Option<(u32, f32)> = None;
        // Furthest-search radius — a player 200u from the touchline isn't
        // realistically the thrower. We still score them; this just bounds
        // the normalisation.
        const MAX_REASONABLE_DISTANCE: f32 = 200.0;

        for p in players {
            if p.side != Some(throwing_side) {
                continue;
            }
            if p.is_sent_off {
                continue;
            }
            if p.tactical_position.current_position.position_group()
                == PlayerFieldPositionGroup::Goalkeeper
            {
                continue;
            }
            let dx = p.position.x - throw_pos.x;
            let dy = p.position.y - throw_pos.y;
            let dist = (dx * dx + dy * dy).sqrt();
            let dist_score = (1.0 - (dist / MAX_REASONABLE_DISTANCE).min(1.0)).max(0.0);

            // Position fit: defenders + midfielders get a small bump for
            // throw-ins (fullbacks / wide mids most often take throws). The
            // exact PlayerPositionType subdivision (LB/RB/LM/RM) lives one
            // crate boundary away, so we lean on the position-group buckets
            // that are visible here.
            let position_fit = match p.tactical_position.current_position.position_group() {
                PlayerFieldPositionGroup::Defender => 1.0,
                PlayerFieldPositionGroup::Midfielder => 0.8,
                PlayerFieldPositionGroup::Forward => 0.5,
                PlayerFieldPositionGroup::Goalkeeper => 0.0,
            };

            let long_throws = (p.skills.technical.long_throws / 20.0).clamp(0.0, 1.0);
            let decisions = (p.skills.mental.decisions / 20.0).clamp(0.0, 1.0);
            let technique = (p.skills.technical.technique / 20.0).clamp(0.0, 1.0);
            let strength = (p.skills.physical.strength / 20.0).clamp(0.0, 1.0);

            let score = dist_score * 0.50
                + position_fit * 0.18
                + long_throws * 0.16
                + decisions * 0.08
                + technique * 0.05
                + strength * 0.03;

            let candidate = (p.id, score);
            match best {
                None => best = Some(candidate),
                Some((_, best_score)) if score > best_score => best = Some(candidate),
                _ => {}
            }
        }
        best.map(|(id, _)| id)
    }
}

#[cfg(test)]
mod throw_in_tests {
    use super::ThrowIn;

    #[test]
    fn range_scales_with_the_attribute_and_reaches_the_box() {
        let (min_ordinary, max_ordinary) = ThrowIn::range(6.0);
        let (min_specialist, max_specialist) = ThrowIn::range(20.0);
        assert_eq!(min_ordinary, min_specialist, "the floor is a recycle pass");
        assert!(max_specialist > max_ordinary);
        // A specialist has to out-reach the 18-yard line (132u) from the
        // touchline, or `long_throw_target` can never find anybody and
        // the whole long-throw path is unreachable again.
        assert!(
            max_specialist > 132.0,
            "specialist range {max_specialist}u cannot reach the box"
        );
    }

    #[test]
    fn box_throw_needs_both_the_third_and_the_attribute() {
        assert!(!ThrowIn::can_reach_box(20.0, false), "own half");
        assert!(!ThrowIn::can_reach_box(9.0, true), "no long-throw skill");
        assert!(ThrowIn::can_reach_box(16.0, true));
    }
}
