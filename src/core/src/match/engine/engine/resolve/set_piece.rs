//! **Set pieces the engine has to place** — the pending teleport the ball
//! layer stages when a restart rewrites ownership, and the corner shape's
//! station lifecycle: arming it, probing the box under `match-logs`, and
//! sweeping it once the corner is over.
//!
//! The ball cannot move other players, so it records the intent and this
//! file drains it on the next tick, when `&mut field.players` is in hand.

use crate::r#match::PassOriginRestart;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::ball::ball::CornerWalk;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::teleport as tc;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::corner_shape::CornerShape;
use crate::r#match::engine::corner_shape::{CornerDeadline, CornerRole};
use crate::r#match::engine::engine::*;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::SetPieceDiag;
use nalgebra::Vector3;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// Corner kicks and goal kicks rewrite ball ownership inside `ball.update`,
    /// but ball.rs only has `&[MatchPlayer]` — it can't teleport the designated
    /// taker to the ball. Instead it stashes the teleport intent on the Ball;
    /// we drain it here, now that we have `&mut field.players`. Without this,
    /// the ball sits at the corner flag / goal area with ownership assigned
    /// to a player 30-200 units away, and `move_to`'s 15-unit distance check
    /// nulls ownership on the very next tick — ball stalls for seconds.
    pub(in crate::r#match::engine::engine) fn apply_pending_set_piece_teleport(
        field: &mut MatchField,
    ) {
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
                let attacking_corner =
                    field.ball.pass_origin_restart == crate::r#match::PassOriginRestart::Corner;
                let p = &mut field.players[idx];
                let next = if p.tactical_position.current_position.is_forward() {
                    PlayerState::Forward(ForwardState::Heading)
                } else if p.tactical_position.current_position.is_midfielder() {
                    PlayerState::Midfielder(MidfielderState::Heading)
                } else if attacking_corner {
                    // A defender who won the ATTACKING corner header goes
                    // to the state that shoots, not the one that clears.
                    PlayerState::Defender(DefenderState::AttackingCorner)
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
    pub(in crate::r#match::engine::engine) fn note_corner_setup_box_if_taken(field: &MatchField) {
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
        // **He is still standing over it.** The kick has not been taken,
        // so the delivery's clock has not started: keep re-stamping the
        // deadline's origin onto now, exactly as the go-live re-stamp in
        // `AwaitedRestart::take_from` does for the walk-in.
        //
        // The deadline is sized off what a corner PHYSICALLY takes once
        // the ball is in the air — the flight and the moment somebody
        // attacks it — and the seconds a taker spends set over a dead ball
        // are a stoppage, which is precisely when the other twenty-one are
        // meant to be walking into the shape. Measured before this: the
        // shape was released by the deadline on 81% of corners in the
        // fourth tier and 56% in the first, i.e. the box emptied before
        // the cross was met in most corners at every level of the pyramid.
        //
        // Bounded, because "he never strikes it" has to terminate. Past
        // the set-up ceiling the corner is not a corner any more and falls
        // through to the release below.
        //
        // ⚠ It has to be the ORIGINAL dead ball he is still standing over,
        // not a ball that came back to him. A short corner played to a
        // team-mate and returned would otherwise re-pin the whole shape
        // for another set-up window, in the middle of open play.
        let now = field.ball.current_tick_cached;
        // The taker is the only man who may touch the ball without ending
        // the set piece: the award stamps him as last toucher, and so does
        // his own delivery (a cross is a deliberate kick). Anybody else on
        // it is first contact.
        let only_the_taker_has_touched_it = field.ball.last_touch_player_id == Some(shape.taker_id);
        let corner_still_live = field.ball.pass_origin_restart == PassOriginRestart::Corner
            && only_the_taker_has_touched_it;
        if CornerDeadline::armed()
            && corner_still_live
            && field.ball.current_owner == Some(shape.taker_id)
        {
            let since_live = now.saturating_sub(shape.live_tick.unwrap_or(shape.armed_tick));
            if since_live < CornerDeadline::SETUP_MAX_TICKS {
                if let Some(s) = field.ball.corner_shape.as_mut() {
                    s.armed_tick = now;
                }
                return;
            }
        }
        let held = now.saturating_sub(shape.armed_tick);
        let deadline = if CornerDeadline::armed() {
            shape.deadline_ticks
        } else {
            CornerDeadline::CALIBRATION_TICKS
        };
        if corner_still_live && held < deadline {
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
}
