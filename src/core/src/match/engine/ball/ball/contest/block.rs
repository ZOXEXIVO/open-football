//! **The block** — a defender's body in the shot's corridor between the
//! striker and the goal line.

use crate::PlayerFieldPositionGroup;
use crate::r#match::ball::events::BallEvent;
use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::engine::ball::ball::contest::contact::ContactInPlace;
use crate::r#match::engine::goal::GOAL_WIDTH;
use crate::r#match::events::EventCollection;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext as EffSkillCtx, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchContext, MatchPlayer, PassOriginRestart, PlayerSide};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

impl Ball {
    /// Shot-block check. Runs only when the ball is a shot in flight
    /// (has a cached goal-line target). A defender whose body is in
    /// the shot's corridor between the current ball position and the
    /// goal line has a skill-weighted chance to block — the ball
    /// deflects to a loose state rather than reaching the keeper.
    /// Real football blocks ~6-10% of shots; we aim for that band.
    ///
    /// Distinct from `try_intercept`:
    /// - Intercept: ≤ 2.5u radius, pass-targeted; tiny per-tick chance
    /// - Block:     ≤ 4u radius, shot-targeted; higher per-event chance
    /// Both are scoped to unowned balls with `in_flight_state > 0`.
    pub fn try_block_shot(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // Only live shots — no cache means no shot in flight, no block.
        let shot_target = match self.cached_shot_target {
            Some(t) => t,
            None => return,
        };
        if self.current_owner.is_some() || self.flags.in_flight_state == 0 {
            return;
        }
        // A block already won, waiting for the ball to arrive at the man who
        // won it — see `ShotTarget::blocked_by`.
        if let Some((blocker_id, outcome_roll)) = shot_target.blocked_by {
            let reached = players
                .iter()
                .find(|p| p.id == blocker_id)
                .map(|p| {
                    (p.position.x - self.position.x).hypot(p.position.y - self.position.y)
                        <= Self::BLOCK_REACH
                })
                .unwrap_or(false);
            if reached {
                self.resolve_block(blocker_id, outcome_roll, context, players, events);
            }
            return;
        }
        // One shot, one roll — see `ShotTarget::block_rolled`.
        if shot_target.block_rolled {
            return;
        }
        #[cfg(feature = "match-logs")]
        crate::r#match::engine::ball::ball::diagnostics::block_diag::SHOTS_SEEN
            .fetch_add(1, Ordering::Relaxed);
        // Ball above defender reach. This read `> 2.0` and the comment
        // called it "chest height" — but 1u is 0.125 m, so the bar was
        // 25 CENTIMETRES. Anything above ankle height was unblockable,
        // which excluded 23% of all shot-ticks outright. A defender
        // blocks with whatever he can get in the way, up to a raised
        // boot or a head: 16u is 2 m.
        // 2.2 m — a defender's raised-arm reach. Was 16.0, which on a
        // vertical axis measured in metres put the block ceiling above the
        // stands; it never rejected anything.
        const MAX_BLOCK_HEIGHT: f32 = 2.2;
        if self.position.z > MAX_BLOCK_HEIGHT {
            #[cfg(feature = "match-logs")]
            crate::r#match::engine::ball::ball::diagnostics::block_diag::TOO_HIGH
                .fetch_add(1, Ordering::Relaxed);
            return;
        }

        let shooter_team = match self.previous_owner {
            Some(prev_id) => players.iter().find(|p| p.id == prev_id).map(|p| p.team_id),
            None => return,
        };
        let shooter_team = match shooter_team {
            Some(t) => t,
            None => return,
        };

        // Defender must be in the shot's path: between the ball and
        // the goal line, in the corridor defined by the shot direction.
        let ball_velocity_2d =
            (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y).sqrt();
        if ball_velocity_2d < 0.5 {
            return; // Ball has stopped / nearly — not a live shot.
        }
        let shot_dir_x = self.velocity.x / ball_velocity_2d;
        let shot_dir_y = self.velocity.y / ball_velocity_2d;

        // Block window. Widened from 30u lookahead + 4u corridor so
        // defenders near the shot line have a real chance to get a
        // leg/body in. Real football blocks ~18-22% of shots (2-3 per
        // team per match from ~13 shots); the engine emits ~0.01 blocks
        // per defender per match.
        //
        // ⚠ That gap is NOT this window. Measured with `block_diag`
        // (2026-08, n=400 at L14): of 246k shot-ticks reaching the
        // check, 28% are above blocking height and **0.1% ever find a
        // defender in the lane at all** — so the roll below almost never
        // gets to happen. Widening the lookahead to 120u (15m, the
        // distance shots are really taken from) and the corridor to 16u
        // (2m, a committed lunge rather than a standing body) moved
        // candidates from 0.0% to 0.1% and blocks not at all. Defenders
        // are simply not between the ball and the goal while a shot is
        // in flight, which is a positioning property of the engine and a
        // separate piece of work from the block model. Both constants
        // are therefore left where they were rather than carrying an
        // unmeasured widening for no benefit.
        // Widened 40/7 → 90/13 once the 25cm height bar above was lifted.
        // The earlier attempt recorded in this comment measured no gain,
        // but it was made while that bar silently threw away every ball
        // above ankle height, so the corridor was never the thing being
        // tested. 90u is 11 m — the range over which a defender can still
        // get across to a shot — and 13u is 1.6 m, a committed lunge or
        // slide rather than a standing body.
        const BLOCK_LOOKAHEAD: f32 = 90.0;
        const BLOCK_CORRIDOR: f32 = 16.0;

        let mut best_blocker: Option<u32> = None;
        let mut best_chance: f32 = 0.0;

        for player in players {
            // Only opposing outfielders block (GK save pipeline handles
            // shots that reach the line; a GK blocking a shot at 5u
            // out is already Catching/Diving).
            if player.team_id == shooter_team {
                continue;
            }
            if player.tactical_position.current_position.position_group()
                == PlayerFieldPositionGroup::Goalkeeper
            {
                continue;
            }

            #[cfg(feature = "match-logs")]
            crate::r#match::engine::ball::ball::diagnostics::block_diag::OPP_SEEN
                .fetch_add(1, Ordering::Relaxed);

            // Project defender position onto the shot line.
            let dx = player.position.x - self.position.x;
            let dy = player.position.y - self.position.y;
            let projection = dx * shot_dir_x + dy * shot_dir_y;
            // Must be ahead of the ball along the shot line, within
            // the lookahead window. 1u minimum so a defender level
            // with the ball (who's already been passed) doesn't count.
            if projection < 1.0 {
                #[cfg(feature = "match-logs")]
                crate::r#match::engine::ball::ball::diagnostics::block_diag::BEHIND_BALL
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }
            if projection > BLOCK_LOOKAHEAD {
                #[cfg(feature = "match-logs")]
                crate::r#match::engine::ball::ball::diagnostics::block_diag::BEYOND_LOOKAHEAD
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }
            // Perpendicular distance to the line.
            let perp =
                (dx - projection * shot_dir_x).powi(2) + (dy - projection * shot_dir_y).powi(2);
            let perp_dist = perp.sqrt();
            #[cfg(feature = "match-logs")]
            {
                crate::r#match::engine::ball::ball::diagnostics::block_diag::IN_WINDOW
                    .fetch_add(1, Ordering::Relaxed);
                crate::r#match::engine::ball::ball::diagnostics::block_diag::PERP_SUM_X100
                    .fetch_add((perp_dist * 100.0) as u64, Ordering::Relaxed);
            }
            if perp_dist > BLOCK_CORRIDOR {
                #[cfg(feature = "match-logs")]
                crate::r#match::engine::ball::ball::diagnostics::block_diag::OUTSIDE_CORRIDOR
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }

            // Skill mix: bravery (willingness to step into shot),
            // positioning (read the angle), anticipation (read the
            // cue), jumping/agility (get the body in the way), plus
            // tackling (stretching / last-ditch leg out). Weighted
            // toward mental attributes since shot-blocking is 70%
            // reading the shooter's body shape. Routed through
            // `effective_skill` so a tired defender blocks worse.
            let block_minute = sc::minute_from_ticks(self.current_tick_cached);
            let block_tech = EffSkillCtx::technical(block_minute);
            let block_mental = EffSkillCtx::mental(block_minute);
            let block_expl = EffSkillCtx::explosive(block_minute);
            let bravery = effective_skill(player, player.skills.mental.bravery, block_mental);
            let positioning =
                effective_skill(player, player.skills.mental.positioning, block_mental);
            let anticipation =
                effective_skill(player, player.skills.mental.anticipation, block_mental);
            let agility = effective_skill(player, player.skills.physical.agility, block_expl);
            let tackling = effective_skill(player, player.skills.technical.tackling, block_tech);
            let skill_factor = (bravery * 0.25
                + positioning * 0.25
                + anticipation * 0.25
                + agility * 0.15
                + tackling * 0.10)
                / 20.0;

            // Line factor — closer to the ball is better because the
            // defender's body is actually in the way. Farther along the
            // line means the shot has had time to rise / dip / move.
            let line_factor = 1.0 - (projection / BLOCK_LOOKAHEAD) * 0.4;
            // Perp factor — right on the line is best. Steeper fall-off
            // than before (0.5 from center → basically full chance;
            // 1.0 from edge → 60% chance) so wings-of-corridor still
            // produce blocks at meaningful rates.
            let perp_factor = 1.0 - (perp_dist / BLOCK_CORRIDOR) * 0.5;
            // Fast shots are harder to get in front of — but reaction
            // reflexes matter too. Elite defender reads the shape and
            // steps a tick earlier.
            let speed_penalty = 1.0 / (1.0 + ball_velocity_2d * 0.10);

            // Base multiplier 0.55 (was 0.35) — elite defenders
            // (skill_factor ≈ 0.85) at a good angle now block at
            // 30-40% chance, matching the real "closed-down striker
            // gets the ball blocked" rate.
            let chance = skill_factor * line_factor * perp_factor * speed_penalty * 0.95;

            if chance > best_chance {
                best_chance = chance;
                best_blocker = Some(player.id);
            }
        }

        // RNG threshold instead of deterministic cutoff: a 30% block
        // chance still allows the shot through 70% of the time, which
        // is what we want — defenders block but don't always block.
        //
        // Latch BEFORE rolling so a shot that survives the best-placed
        // defender is not re-offered to him (or to a worse one) on the
        // next tick.
        if best_blocker.is_some() {
            #[cfg(feature = "match-logs")]
            crate::r#match::engine::ball::ball::diagnostics::block_diag::CANDIDATES
                .fetch_add(1, Ordering::Relaxed);
            if let Some(t) = self.cached_shot_target.as_mut() {
                t.block_rolled = true;
            }
        }
        let blocker_id = match best_blocker {
            Some(id) if context.rng.unit_f32() < best_chance.clamp(0.03, 0.70) => id,
            _ => return,
        };
        #[cfg(feature = "match-logs")]
        crate::r#match::engine::ball::ball::diagnostics::block_diag::FIRED
            .fetch_add(1, Ordering::Relaxed);

        // The outcome roll is drawn HERE, on the tick the block was won, so
        // the shared RNG stream is untouched by the deferral below.
        let outcome_roll = context.rng.unit_f32();

        // **He has won the block; the ball still has to get to him.**
        //
        // The candidate window reaches 90u (11 m) up the shot line, so the
        // roll can succeed long before the two meet. Deflecting on this tick
        // turned the ball round in mid-flight with the defender still eleven
        // metres away, which is the "the ball bounced off nothing" report on
        // the same axis as the keeper's. Commit and wait: the rate is
        // decided here, the contact happens where the body is.
        let gap_now = players
            .iter()
            .find(|p| p.id == blocker_id)
            .map(|p| (p.position.x - self.position.x).hypot(p.position.y - self.position.y))
            .unwrap_or(f32::MAX);
        if gap_now > Self::BLOCK_REACH {
            if let Some(t) = self.cached_shot_target.as_mut() {
                t.blocked_by = Some((blocker_id, outcome_roll));
            }
            return;
        }

        self.resolve_block(blocker_id, outcome_roll, context, players, events);
    }

    /// How close the blocker has to be for the contact to be his. 16u is
    /// 2 m — the same [`BLOCK_CORRIDOR`](Self::BLOCK_REACH) the candidate
    /// search calls "a committed lunge or a slide rather than a standing
    /// body", which is the engine's own statement of a defender's reach.
    const BLOCK_REACH: f32 = 16.0;

    // ⚠ "A DEFENDER STRETCHING TO CUT OUT A LOW BALL PUTS IT BEHIND" WAS
    // IMPLEMENTED HERE AND REMOVED — MEASURED AT 0.04 A MATCH.
    //
    // It is correct football and the obvious hole in the corner-source
    // census (the whole "defender puts a delivery behind" family is fed
    // only by SHOTS and AIRBORNE deliveries; the low ball across the face
    // of goal has no path). It fires on nothing because the situation
    // does not arise: interceptions cluster AT the defensive line, which
    // sits ~119u (14.9 m) from its own goal, and almost no live low ball
    // reaches the last twelve metres to be cut out at all.
    //
    // Widening the depth gate to the edge of the area would make it fire
    // — and would be wrong, because a defender sixteen metres out has the
    // whole pitch behind him and no reason to concede a corner. The gate
    // is right and the input is missing. Same shape of answer as the
    // `Defender: Clearing` hook, which was also correct and also dead.
    // See `corner_supply_root_cause`.

    /// Turn a won block into a deflection, at the blocker.
    ///
    /// `outcome_roll` was drawn when the block was won — see
    /// [`ShotTarget::blocked_by`] for why it is carried rather than redrawn.
    fn resolve_block(
        &mut self,
        blocker_id: u32,
        outcome_roll: f32,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        let ball_velocity_2d =
            (self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y).sqrt();
        if ball_velocity_2d < 0.3 {
            // It has died on the way to him — there is nothing left to
            // deflect, and the loose-ball machinery owns it now.
            self.cached_shot_target = None;
            return;
        }
        let shot_dir_x = self.velocity.x / ball_velocity_2d;
        let shot_dir_y = self.velocity.y / ball_velocity_2d;

        // Outcome distribution. Real blocks rarely produce clean
        // possession — they produce loose balls, deflections wide for a
        // corner, sideways skips, or (rarely) deflections back into
        // danger. The previous deterministic ownership flow over-credited
        // defenders.
        let blocker = match players.iter().find(|p| p.id == blocker_id) {
            Some(p) => p,
            None => return,
        };
        let blocker_pos = blocker.position;
        let blocker_team = blocker.team_id;
        let blocker_side = blocker.side;
        let composure = (blocker.skills.mental.composure / 20.0).clamp(0.0, 1.0);
        let technique = (blocker.skills.technical.technique / 20.0).clamp(0.0, 1.0);
        let ball_speed_low_bonus = if ball_velocity_2d < 2.0 { 0.06 } else { 0.0 };
        // Taking the ball cleanly off a block means having it at your
        // feet, so it is only available to a defender the ball actually
        // reached — see the position note below. Blocking at a stretch
        // from range always leaves a loose ball. Without this the
        // controlled branch could hand ownership to a man 11 m away, and
        // `move_to` would drop it again on the next tick as an
        // unreachable owner.
        let blocker_gap = (blocker_pos.x - self.position.x).hypot(blocker_pos.y - self.position.y);
        let blocker_in_reach = blocker_gap <= crate::r#match::engine::ball::ball::CONTROL_DISTANCE;
        let controlled_block_prob = if blocker_in_reach {
            (0.06 + composure * 0.05 + technique * 0.04 + ball_speed_low_bonus).clamp(0.06, 0.30)
        } else {
            0.0
        };

        // Deflection direction: away from the shot line, with a random ±45° spread.
        let angle: f32 = (context.rng.unit_f32() - 0.5) * 1.56;
        let rev_x = -shot_dir_x * angle.cos() - (-shot_dir_y) * angle.sin();
        let rev_y = -shot_dir_x * angle.sin() + (-shot_dir_y) * angle.cos();
        let tick = self.current_tick_cached;

        // ⚠ THE CORNER SHARE HERE IS DELIBERATELY FLAT, and depth-scaling
        // it is a mistake worth documenting because the idea is tempting.
        //
        // A defender who HOOKS a delivery behind is choosing to, and the
        // closer to his own line he is the less choice he has — that is
        // why `heads_it_behind` in the cross contest scales with depth. A
        // BLOCKED SHOT is not a choice at all: the ball keeps most of its
        // goalward momentum through a small deflection, so it carries over
        // the byline from sixteen metres about as readily as from six. The
        // thing that decides is the deflection angle, which the spread
        // above already draws.
        //
        // Scaling it on depth was measured against this engine's own
        // geometry and would have made it worse, not better: blocks land
        // at ~119u from the blocker's line (`at the strike` in the shot
        // census), so an `urgency²` curve over the penalty area puts
        // almost every block on the floor of the curve and would have cut
        // block-fed corners from 0.74 a match to near zero.
        let roll = outcome_roll;
        let p_controlled = controlled_block_prob;
        let p_corner = p_controlled + 0.23;
        let p_safe = p_corner + 0.23;
        let p_loose = p_safe + 0.40; // ~40% loose central rebound
        // remainder ~14% → unlucky deflection toward goal (slows but stays live)

        // A block happens where the BALL is, not where the defender is.
        //
        // This used to be an unconditional `self.position = blocker_pos`,
        // and the block window reaches 90u ahead and 16u across — so a
        // successful block could pick the ball up in mid-flight and set
        // it down eleven metres away inside a single tick. That is the
        // "ball suddenly somewhere else for no reason" report exactly:
        // there is no kick, no bounce and no carry to explain it, the
        // ball is simply elsewhere on the next frame.
        //
        // The ball is now guaranteed to be inside `BLOCK_REACH` of him
        // (`blocked_by` waits for it), so the two really are meeting; what
        // is left is the difference between a ball at his feet and one he
        // reaches with a stretched leg or a slide. `CONTROL_DISTANCE` is
        // the engine's existing answer to "close enough to have played it",
        // and beyond it the ball still deflects from its own position.
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::SaveContactDiag::note(
            3,
            blocker_gap,
            self.position.z,
            blocker_pos.x - self.position.x,
            blocker_pos.y - self.position.y,
        );
        // ⚠ **Neither axis is written any more.**
        //
        // Horizontally, this is the same argument as `secure_ball_for` and
        // `apply_failed_first_touch`, and it lands the same way: the block
        // happens where the ball is. `blocker_in_reach` is
        // `CONTROL_DISTANCE`, so the write was up to 1.5 m of ball moving
        // with nothing touching it, and where it mattered — the clean
        // block, where the defender takes possession — `move_to` draws the
        // ball to him at 1.5 u/tick anyway, well inside
        // `MAX_OWNER_TRACK_DISTANCE`. `blocker_in_reach` now decides only
        // the *controlled* branch, which is what its name says.
        //
        // Vertically, `position.z = 0.0` dropped a ball from as high as
        // `MAX_BLOCK_HEIGHT` (2.2 m) onto the grass in one tick. Its
        // sibling `try_save_shot` has captured `contact_z` and restored it
        // per branch for exactly this reason since the save-contact work;
        // the block path never got the same treatment. The deflection
        // branches below now `sink_to_ground` instead of zeroing `vz`, and
        // the controlled branch needs nothing — the owner's `carry_toward`
        // has it.
        if !ContactInPlace::armed() {
            if blocker_in_reach {
                self.position = blocker_pos;
            }
            self.position.z = 0.0;
        }
        self.previous_owner = self.current_owner.or(self.previous_owner);
        self.pass_target_player_id = None;
        self.cached_shot_target = None;
        self.record_touch(blocker_id, blocker_team, tick, false);
        self.offside_snapshot = None;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;
        // Dedicated Blocked event so the block credit can't leak into a
        // separate Intercepted that happens to share the same tick — the
        // ordering of events in `EventCollection` is no longer load-
        // bearing for stat correctness.
        let block_position = self.position;
        events.add_ball_event(BallEvent::Blocked(blocker_id, block_position));

        if roll < p_controlled {
            // Clean block — defender gets the ball at his feet.
            self.velocity = Vector3::zeros();
            self.current_owner = Some(blocker_id);
            self.flags.in_flight_state = 0;
            self.claim_cooldown = 25;
            events.add_ball_event(BallEvent::Intercepted(
                blocker_id,
                self.previous_owner,
                false,
            ));
            return;
        }

        // Deflection branches below leave the ball loose (no owner) and
        // do NOT emit `Intercepted` — block credit was already booked
        // via the dedicated `Blocked` event above. Emitting `Intercepted`
        // here would double-credit (interception + block), and worse,
        // its `ClaimBall` follow-up would force ownership onto a
        // defender who in physics terms hasn't actually picked the ball
        // up. Possession is decided by whoever claims the loose ball
        // next, not by the block itself.
        if roll < p_corner {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::BLOCK_CORNER_FIRED.fetch_add(1, Ordering::Relaxed);
            // Deflection out for a corner — push the ball past the
            // defender's OWN byline and WIDE OF THE POST (toward the corner
            // flag) so the endline resolver awards a corner (defender = last
            // toucher → corner for the attackers). Aiming merely at the
            // byline (the old ±1.2 y nudge) left a central block crossing
            // BETWEEN the posts → goal kick / own goal, so blocks almost
            // never became corners (engine ran ~0.5 corners/match vs ~10
            // real). The ball must finish outside `center ± GOAL_WIDTH`.
            let endline_x = match blocker_side {
                Some(PlayerSide::Left) => 0.0_f32,
                Some(PlayerSide::Right) => self.field_width,
                None => {
                    if self.position.x < self.field_width * 0.5 {
                        0.0
                    } else {
                        self.field_width
                    }
                }
            };
            let center_y = self.field_height * 0.5;
            // Deflect toward the touchline the ball is already drifting to
            // (sign of the reverse-deflection y), past the post.
            let to_top = if rev_y.abs() > 0.01 {
                rev_y < 0.0
            } else {
                self.position.y < center_y
            };
            let wide_y = if to_top {
                (center_y - GOAL_WIDTH - self.field_height * 0.05).max(2.0)
            } else {
                (center_y + GOAL_WIDTH + self.field_height * 0.05).min(self.field_height - 2.0)
            };
            let dx = endline_x - self.position.x;
            let dy = wide_y - self.position.y;
            let dist = (dx * dx + dy * dy).sqrt().max(1.0);
            let speed = (ball_velocity_2d * 0.6).clamp(3.0, 6.0);
            self.velocity.x = (dx / dist) * speed;
            self.velocity.y = (dy / dist) * speed;
            self.settle_or_flatten();
            self.current_owner = None;
            self.flags.in_flight_state = 30;
            // Hold off re-claims so the deflection crosses the byline before
            // a covering defender grabs it back (else it never becomes a
            // corner — the whole point of this branch).
            self.claim_cooldown = 16;
            return;
        }

        if roll < p_safe {
            // Safe sideways deflection — perpendicular skip away from
            // both goals. Loose ball; either team can recover.
            let safe_speed = (ball_velocity_2d * 0.35).clamp(1.5, 3.5);
            // Rotate shot direction 90° (sign chosen by random) to skip sideways.
            let sign = if context.rng.unit_f32() < 0.5 {
                -1.0
            } else {
                1.0
            };
            self.velocity.x = -shot_dir_y * sign * safe_speed;
            self.velocity.y = shot_dir_x * sign * safe_speed;
            self.settle_or_flatten();
            self.current_owner = None;
            self.flags.in_flight_state = 25;
            self.claim_cooldown = 0;
            return;
        }

        if roll < p_loose {
            // Loose central rebound — ball trickles in front of the
            // defender, often producing a second-ball contest. Arms the
            // rebound window (team shot-spacing exemption) so the
            // second ball can actually be struck. The blocker is the
            // last player the ball came off — recording him as previous
            // owner makes the ATTACKERS the intercept-eligible side
            // during the flight window (the spill is the defender's
            // touch, not the shooter's pass), restoring the two-sided
            // second-ball race.
            self.last_rebound_tick = tick;
            self.previous_owner = Some(blocker_id);
            let loose_speed = (ball_velocity_2d * 0.30).clamp(1.0, 2.8);
            self.velocity.x = rev_x * loose_speed;
            self.velocity.y = rev_y * loose_speed;
            self.settle_or_flatten();
            self.current_owner = None;
            self.flags.in_flight_state = 20;
            self.claim_cooldown = 0;
            return;
        }

        // Unlucky deflection: ball loses pace but keeps drifting toward
        // goal. The shot flag is already cleared, so the keeper save
        // pipeline won't credit a phantom save — but the ball is still
        // live and can be a tap-in opportunity. Arms the rebound window;
        // blocker booked as previous owner (see the loose branch above).
        self.last_rebound_tick = tick;
        self.previous_owner = Some(blocker_id);
        let unlucky_speed = (ball_velocity_2d * 0.50).clamp(1.5, 3.5);
        self.velocity.x = shot_dir_x * unlucky_speed * 0.7;
        self.velocity.y = shot_dir_y * unlucky_speed * 0.7;
        self.settle_or_flatten();
        self.current_owner = None;
        self.flags.in_flight_state = 25;
        self.claim_cooldown = 0;
    }
}
