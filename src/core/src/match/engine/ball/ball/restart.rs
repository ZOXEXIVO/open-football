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

use super::{AwaitedRestart, Ball};
use crate::PlayerFieldPositionGroup;
use crate::r#match::PassOriginRestart;
use crate::r#match::ball::events::BallEvent;
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
        // still in the goal from the last one.
        if self.goal_scored || self.in_net.is_some() {
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

        // Inset the throw-in slightly inside the touchline so subsequent
        // physics doesn't immediately re-trigger a boundary cross.
        let field_width = context.field_size.width as f32;
        let throw_x = self.position.x.clamp(2.0, field_width - 2.0);
        let throw_y = if crossed_top { 2.0 } else { field_height - 2.0 };
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

        self.position = throw_pos;
        self.velocity = Vector3::zeros();

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

        self.awaiting_restart = Some(AwaitedRestart {
            taker_id: thrower_id,
            spot: throw_pos,
            origin: PassOriginRestart::ThrowIn,
            awarded_tick: context.current_tick(),
        });
        // Send him. `TakeMe` is the engine's existing "go and get the
        // ball" signal (`MatchPlayer::run_for_ball`), so the taker runs
        // there under ordinary AI and ordinary stamina rather than being
        // placed there by a set-piece teleport.
        events.add_ball_event(BallEvent::TakeMe(thrower_id));
    }

    /// One tick of a dead ball waiting for its taker.
    ///
    /// Pins the ball on the line, hands it over the moment he arrives, and
    /// falls back to the old teleport if he never does. Runs BEFORE
    /// ownership so nothing can claim a ball that is out of play.
    pub(crate) fn tick_awaited_restart(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        let Some(await_state) = self.awaiting_restart else {
            return;
        };

        // The ball is out of play: it does not move and it is nobody's,
        // whatever else happened this tick.
        self.position = await_state.spot;
        self.velocity = Vector3::zeros();
        self.current_owner = None;

        let Some(taker) = players.iter().find(|p| p.id == await_state.taker_id) else {
            // Substituted or sent off between the award and now. Drop the
            // restart and let the ordinary loose-ball logic have it rather
            // than waiting for a man who is not on the pitch.
            self.awaiting_restart = None;
            return;
        };

        let range = (taker.position - await_state.spot).magnitude();
        let waited = context
            .current_tick()
            .saturating_sub(await_state.awarded_tick);
        let arrived = range <= AwaitedRestart::REACH;
        if !arrived && waited < AwaitedRestart::PATIENCE_TICKS {
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
            #[cfg(feature = "match-logs")]
            RestartCensus::note_throw_in_teleport();
            self.pending_set_piece_teleport = Some((await_state.taker_id, await_state.spot));
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
