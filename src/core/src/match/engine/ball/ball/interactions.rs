//! Ball-vs-defender interactions during in-flight passes and shots:
//! interception, shot-block, and goalkeeper save. Each runs only on
//! unowned balls with `in_flight_state > 0` so routine possession
//! play isn't disturbed.

use super::Ball;
use crate::PlayerFieldPositionGroup;
use crate::r#match::ball::events::BallEvent;
use crate::r#match::engine::goal::GOAL_WIDTH;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::player::events::players::save_accounting_stats;
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
    /// Opposing players near the ball's flight path can intercept passes.
    /// Interception chance depends on tackling, anticipation, positioning skills
    /// and proximity to the ball's trajectory.
    pub fn try_intercept(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // `context` is held even when this site does not currently
        // draw from `context.rng` so future calibration / env-modifier
        // wiring (slide tackle range, sliding_tackle_success) lands
        // without changing the signature again.
        let _ = context;
        // Only intercept unowned balls that are in flight (active pass)
        if self.current_owner.is_some() || self.flags.in_flight_state == 0 {
            return;
        }

        // Don't intercept aerial balls above player reach
        if self.position.z > 2.5 {
            return;
        }

        // Need to know who passed to determine the opposing team
        let passer_team = match self.previous_owner {
            Some(prev_id) => players.iter().find(|p| p.id == prev_id).map(|p| p.team_id),
            None => return,
        };
        let passer_team = match passer_team {
            Some(t) => t,
            None => return,
        };

        // Ball velocity determines the interception corridor width
        let ball_speed_sq = self.velocity.x * self.velocity.x + self.velocity.y * self.velocity.y;
        if ball_speed_sq < 1.0 {
            return; // Ball too slow, normal claiming handles it
        }

        // Interception reach in game units. Field is 840u = 105m, so 1u =
        // 0.125m. Old 2.5u left average defenders mathematically
        // unable to intercept (max score 0.039 vs 0.04 threshold). 5u
        // produced ~0.1 interceptions/team/match — defenders within
        // the radius hit ~0.025 chance, below the 0.035 threshold for
        // anyone but the closest, fastest, best-positioned. 6.5u
        // (~0.8m — a stretch-extension radius for the planted leg) and
        // a slightly higher base coefficient produces ~10
        // interceptions/team/match (real-football band) without the
        // intercept→snap→re-pass loops the previous 8u radius caused.
        const INTERCEPT_RADIUS: f32 = 5.5;
        const INTERCEPT_RADIUS_SQ: f32 = INTERCEPT_RADIUS * INTERCEPT_RADIUS;

        let mut best_interceptor: Option<u32> = None;
        let mut best_chance: f32 = 0.0;

        for player in players {
            // Only opposing team players can intercept
            if player.team_id == passer_team {
                continue;
            }

            // Don't let the pass target's team intercept their own pass target
            if Some(player.id) == self.pass_target_player_id {
                continue;
            }

            // Distance from player to ball
            let dx = player.position.x - self.position.x;
            let dy = player.position.y - self.position.y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq > INTERCEPT_RADIUS_SQ {
                continue;
            }

            // Base chance: dedicated `interception` composite — anticipation,
            // positioning, concentration, marking, etc. routed through
            // `effective_skill` so fatigue applies. Drop-in replacement for
            // the legacy 4-skill average; magnitude lands in the same band
            // (0..1). Minute derived from the cached tick (10ms ticks).
            let minute = sc::minute_from_ticks(self.current_tick_cached);
            let skill_factor = sc::interception(player, minute);

            // Proximity factor: closer = higher chance (1.0 at 0m, 0.3 at max radius)
            let dist = dist_sq.sqrt();
            let proximity_factor = 1.0 - (dist / INTERCEPT_RADIUS) * 0.7;

            // Fast passes are harder to intercept — penalty coefficient
            // moderated from 0.10 (which made 7 u/tick passes 41% harder
            // than slow ones) back toward a lighter slope.
            let speed_penalty = 1.0 / (1.0 + ball_speed_sq.sqrt() * 0.06);

            // Per-tick interception chance. The 0.13 coefficient with
            // the 0.035 threshold mathematically excluded average
            // defenders (skill 0.5 × proximity 0.65 × speed 0.6 ≈
            // 0.025 per the old radius), so observed interceptions
            // were ~0.1/team/match vs real ~10/team. 0.16 (with the
            // bumped 5.5u radius and lowered 0.030 threshold) brings
            // an average-positioned defender to ~0.038 (above
            // threshold), and an elite defender at point-blank to
            // ~0.07, while still leaving peripheral or off-the-pace
            // defenders below the bar. Population per-team
            // interceptions land near 12–13/match.
            let chance = skill_factor * proximity_factor * speed_penalty * 0.16;

            if chance > best_chance {
                best_chance = chance;
                best_interceptor = Some(player.id);
            }
        }

        // Deterministic threshold. Avg defender (skill 0.5) at 60% of
        // reach with a typical pass score ~0.040 — just above the bar —
        // so most in-path defenders qualify, but peripheral ones don't.
        //
        // Stat-line note: population interceptions/team/match measures
        // ~120 vs the ~10 real-football target. ~3× of that comes from
        // the flight-protection extension (40→120 ticks) tripling the
        // per-pass intercept window; the rest from pass-volume inflation
        // (~1000 attempts/team vs real ~500). Raising the threshold to
        // suppress this inflated goals/match dramatically (it acts as a
        // population-wide pass-success governor), so the cosmetic stat
        // inflation is accepted in favour of in-band goals + draws.
        if best_chance > 0.030 {
            if let Some(interceptor_id) = best_interceptor {
                // Snap the ball to the interceptor and zero the
                // velocity. Before this, velocity was just scaled to
                // Zeroing velocity + handing ownership to the defender
                // prevents the old "own-goal after intercept" bug without
                // needing to teleport the ball. `move_to` will track the
                // ball toward its new owner at 1.5 u/tick over the next
                // 2-3 ticks, so visually the ball decelerates into the
                // defender's feet instead of jumping instantly from its
                // flight path onto the defender — which was visible to
                // the user as "ball appearing on another player without
                // moving".
                //
                // OG risk is fully handled by `self.velocity = zeros()`:
                // a stationary ball can't roll past the 15u owner-drop
                // threshold, so it can't cross the goal line unowned.
                let _ = interceptor_id; // no teleport, keep position as-is
                self.current_owner = Some(interceptor_id);
                self.pass_target_player_id = None;
                self.flags.in_flight_state = 0;
                self.claim_cooldown = 15;
                self.velocity = Vector3::zeros();
                self.position.z = 0.0;
                // Interception ends any in-flight shot — a defender taking
                // control downfield extinguishes the shot. Without this,
                // the next time the keeper grabs a moving ball from an
                // opponent (a long pass that loops to them), the stale
                // shot flag credits a phantom save and inflates the
                // saves/on-target ratio above 100%.
                self.cached_shot_target = None;
                let interceptor_team = players
                    .iter()
                    .find(|p| p.id == interceptor_id)
                    .map(|p| p.team_id)
                    .unwrap_or(0);
                let tick = self.current_tick_cached;
                self.record_touch(interceptor_id, interceptor_team, tick, true);
                self.offside_snapshot = None;
                self.pass_origin_restart = PassOriginRestart::OpenPlay;
                events.add_ball_event(BallEvent::Intercepted(interceptor_id, self.previous_owner));
            }
        }
    }

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
        let _shot_target = match self.cached_shot_target {
            Some(t) => t,
            None => return,
        };
        if self.current_owner.is_some() || self.flags.in_flight_state == 0 {
            return;
        }
        // Ball above defender reach — aerial shots aren't blocked at
        // chest height, only grounders and waist-high strikes.
        if self.position.z > 2.0 {
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
        // leg/body in — previously many "close-but-not-perfect"
        // positions fell just outside the corridor and the shot flew
        // through unopposed. Real football blocks ~18-22% of shots
        // (2-3 blocks per team per match from ~13 shots); we were
        // below that with the tight window.
        const BLOCK_LOOKAHEAD: f32 = 40.0; // was 30u
        const BLOCK_CORRIDOR: f32 = 7.0; // was 4u — body + stretched leg

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

            // Project defender position onto the shot line.
            let dx = player.position.x - self.position.x;
            let dy = player.position.y - self.position.y;
            let projection = dx * shot_dir_x + dy * shot_dir_y;
            // Must be ahead of the ball along the shot line, within
            // the lookahead window. 1u minimum so a defender level
            // with the ball (who's already been passed) doesn't count.
            if projection < 1.0 || projection > BLOCK_LOOKAHEAD {
                continue;
            }
            // Perpendicular distance to the line.
            let perp =
                (dx - projection * shot_dir_x).powi(2) + (dy - projection * shot_dir_y).powi(2);
            let perp_dist = perp.sqrt();
            if perp_dist > BLOCK_CORRIDOR {
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
            let chance = skill_factor * line_factor * perp_factor * speed_penalty * 0.55;

            if chance > best_chance {
                best_chance = chance;
                best_blocker = Some(player.id);
            }
        }

        // RNG threshold instead of deterministic cutoff: a 30% block
        // chance still allows the shot through 70% of the time, which
        // is what we want — defenders block but don't always block.
        let blocker_id = match best_blocker {
            Some(id) if context.rng.unit_f32() < best_chance.clamp(0.03, 0.38) => id,
            _ => return,
        };

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
        let controlled_block_prob =
            (0.06 + composure * 0.05 + technique * 0.04 + ball_speed_low_bonus).clamp(0.06, 0.30);

        // Deflection direction: away from the shot line, with a random ±45° spread.
        let angle: f32 = (context.rng.unit_f32() - 0.5) * 1.56;
        let rev_x = -shot_dir_x * angle.cos() - (-shot_dir_y) * angle.sin();
        let rev_y = -shot_dir_x * angle.sin() + (-shot_dir_y) * angle.cos();
        let tick = self.current_tick_cached;

        let roll = context.rng.unit_f32();
        let p_controlled = controlled_block_prob;
        let p_corner = p_controlled + 0.23;
        let p_safe = p_corner + 0.23;
        let p_loose = p_safe + 0.40; // ~40% loose central rebound
        // remainder ~14% → unlucky deflection toward goal (slows but stays live)

        self.position = blocker_pos;
        self.position.z = 0.0;
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
            events.add_ball_event(BallEvent::Intercepted(blocker_id, self.previous_owner));
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
            self.velocity.z = 0.0;
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
            self.velocity.z = 0.0;
            self.current_owner = None;
            self.flags.in_flight_state = 25;
            self.claim_cooldown = 0;
            return;
        }

        if roll < p_loose {
            // Loose central rebound — ball trickles in front of the
            // defender, often producing a second-ball contest.
            let loose_speed = (ball_velocity_2d * 0.30).clamp(1.0, 2.8);
            self.velocity.x = rev_x * loose_speed;
            self.velocity.y = rev_y * loose_speed;
            self.velocity.z = 0.0;
            self.current_owner = None;
            self.flags.in_flight_state = 20;
            self.claim_cooldown = 0;
            return;
        }

        // Unlucky deflection: ball loses pace but keeps drifting toward
        // goal. The shot flag is already cleared, so the keeper save
        // pipeline won't credit a phantom save — but the ball is still
        // live and can be a tap-in opportunity.
        let unlucky_speed = (ball_velocity_2d * 0.50).clamp(1.5, 3.5);
        self.velocity.x = shot_dir_x * unlucky_speed * 0.7;
        self.velocity.y = shot_dir_y * unlucky_speed * 0.7;
        self.velocity.z = 0.0;
        self.current_owner = None;
        self.flags.in_flight_state = 25;
        self.claim_cooldown = 0;
    }

    /// Goalkeeper save check. Runs during shot flight: when the ball
    /// approaches the goal line and the defending keeper's body is
    /// within reach of the shot's trajectory, roll a skill-weighted
    /// save. The keeper state machine's `is_catch_successful` path
    /// timed saves to player-state ticks that didn't line up with the
    /// ball's physics step — saves fired too early or too late, and
    /// shots past the keeper cleared into the net. A physics-level
    /// save runs every ball tick with fresh ball position and commits
    /// the ball to the keeper at the moment of contact.
    pub fn try_save_shot(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        let shot_target = match self.cached_shot_target {
            Some(t) => t,
            None => return,
        };
        if self.current_owner.is_some() || self.flags.in_flight_state == 0 {
            return;
        }

        // Ball well over the bar — not a save situation.
        if self.position.z > 2.8 {
            return;
        }

        // Only consider the shot once it's close to the goal line —
        // the save resolves at the moment of contact. Distance in
        // x-units the ball will cover in a single tick determines the
        // window: we check within ~2 ticks of arrival.
        let (goal_x, goal_y) = match shot_target.defending_side {
            PlayerSide::Left => (context.goal_positions.left.x, context.goal_positions.left.y),
            PlayerSide::Right => (
                context.goal_positions.right.x,
                context.goal_positions.right.y,
            ),
        };

        // Reject balls that have already crossed the goal line. Using
        // `.abs()` below meant a shot 2u behind the goal at goal_y+15
        // still satisfied "close to goal line" and "moving toward goal"
        // and got saved out of thin air — the visible bug: ball flies
        // past the goal, then teleports into the keeper's hands. Once
        // the ball is past the line (goal or goal kick, depending on Y),
        // the shot is over.
        let past_goal_line = match shot_target.defending_side {
            PlayerSide::Left => self.position.x < goal_x,
            PlayerSide::Right => self.position.x > goal_x,
        };
        if past_goal_line {
            #[cfg(feature = "match-logs")]
            save_accounting_stats::SAVE_TICKS_PAST_GOAL_LINE.fetch_add(1, Ordering::Relaxed);
            self.cached_shot_target = None;
            return;
        }

        let dist_to_goal_x = (self.position.x - goal_x).abs();
        let ball_vx = self.velocity.x.abs().max(0.5);
        if dist_to_goal_x > ball_vx * 2.5 {
            #[cfg(feature = "match-logs")]
            save_accounting_stats::SAVE_TICKS_OUT_OF_REACH.fetch_add(1, Ordering::Relaxed);
            return;
        }
        #[cfg(feature = "match-logs")]
        save_accounting_stats::SAVE_TICKS_REACHED.fetch_add(1, Ordering::Relaxed);

        // Ball must still be traveling toward that goal line.
        let moving_toward_goal = match shot_target.defending_side {
            PlayerSide::Left => self.velocity.x < -0.2,
            PlayerSide::Right => self.velocity.x > 0.2,
        };
        if !moving_toward_goal {
            return;
        }

        // Ball must be within goal width (else it's wide and the
        // post / out-of-play handler catches it).
        if (self.position.y - goal_y).abs() > GOAL_WIDTH + 1.0 {
            return;
        }

        // Find the defending keeper.
        let keeper = players.iter().find(|p| {
            p.side == Some(shot_target.defending_side)
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
                && !p.is_sent_off
        });
        let keeper = match keeper {
            Some(k) => k,
            None => return,
        };

        // Route through `effective_skill` so a tired keeper has worse
        // reach / handling / reflexes than a fresh one. Routing minute
        // is taken from `MatchContext::total_match_time`.
        let minute_for_effective = sc::minute_from_ms(context.total_match_time);
        let tech_ctx = EffSkillCtx::technical(minute_for_effective);
        let mental_ctx = EffSkillCtx::mental(minute_for_effective);
        let expl_ctx = EffSkillCtx::explosive(minute_for_effective);
        let handling = effective_skill(keeper, keeper.skills.goalkeeping.handling, tech_ctx);
        let reflexes = effective_skill(keeper, keeper.skills.goalkeeping.reflexes, tech_ctx);
        let agility = effective_skill(keeper, keeper.skills.physical.agility, expl_ctx);
        // Concentration acts on the catch / parry split — focused
        // keepers catch cleaner, distracted ones parry into danger.
        let concentration = effective_skill(keeper, keeper.skills.mental.concentration, mental_ctx);
        let scaled_handling = ((handling - 1.0) / 19.0).max(0.0);
        let scaled_reflexes = ((reflexes - 1.0) / 19.0).max(0.0);
        let scaled_agility = ((agility - 1.0) / 19.0).max(0.0);
        let scaled_concentration = ((concentration - 1.0) / 19.0).max(0.0);

        // Diving reach in game units. Field is 840u = 105m, so 1u = 0.126m
        // (half-goal 29u = 3.66m matches real 3.66m). Every keeper, even a
        // youth-level one, can physically dive across most of the goal
        // — skill determines whether they *catch* the ball, not whether
        // they can reach it. The previous 10u floor made corner shots
        // literally unreachable for weak keepers, so blowouts in youth
        // leagues (hnd=1, ref=1) pushed matches to 10+ goals. New reach:
        //   skills 1   → 20u (2.5m, standing dive — can touch the post)
        //   skills 10  → 26u (3.25m, covers most of the goal)
        //   skills 20  → 32u (4.0m, elite full-stretch — beyond the post)
        let reach = 20.0 + scaled_agility * 8.0 + scaled_reflexes * 4.0;
        let lateral_error = (keeper.position.y - shot_target.goal_line_y).abs();
        if lateral_error > reach {
            return;
        }

        // Base save chance. Centered shot ~0.88; full-stretch ~0.30.
        // Skill handles the rest; this curve is purely geometry.
        let reach_ratio = (lateral_error / reach).clamp(0.0, 1.0);
        let base = 0.88 - reach_ratio * reach_ratio * 0.58;

        // Shot-speed penalty — elite shots beat keepers more often.
        let ball_speed = self.velocity.norm();
        let speed_excess = (ball_speed - 3.0).max(0.0);
        let speed_penalty = (speed_excess * 0.08 * (1.0 - scaled_reflexes * 0.5)).min(0.40);

        // Skill multiplier. Floor 0.72 so a 1.0-skill keeper still saves
        // ~60% of centred shots (real weak keepers save 55-65% overall).
        // Old `0.6 + skill*0.5` gave a 40% floor, pushing youth matches
        // with hnd=1.0 / ref=1.0 GKs into 10-goal blowouts. At 0.72 →
        // 1.07, skill matters (10-pt skill gap = ~30% save-rate gap)
        // but weak keepers can't single-handedly lose 13-4.
        //
        // The composite blend (`gk_shot_stopping`) feeds reflexes,
        // handling, agility, positioning, concentration, anticipation
        // and one_on_ones through `effective_skill` so a tired keeper
        // late in the match plays worse — drop-in replacement for the
        // legacy 3-skill blend, magnitude tuned to the same band.
        let skill = sc::gk_shot_stopping(keeper, minute_for_effective);
        // Per-tick save rate. This save check runs every tick the ball
        // is within reach of the goal line, AND the GK state-machine
        // (Catching, Diving) runs its OWN per-tick save roll. Both
        // compound across the 5-15 ticks of shot flight.
        //
        // Calibration target: per-shot conversion ~12-15% (real Opta
        // ~12% of shots are goals, ~33% on target with 30-35% of those
        // being saves). Earlier `0.45 + 0.35*skill` clamped at 0.55
        // produced top-scorer rates of 1.5+ goals/match — too generous.
        // New `0.55 + 0.40*skill` clamped at 0.68 lifts the per-tick
        // floor for any GK on the pitch (a Reflexes-5 keeper still
        // makes routine saves) and raises the ceiling so elite GKs
        // stop the centred power shots they're paid to stop. Skill
        // gap stays 10pt → ~30% save-rate gap.
        // Trimmed `0.55 + skill*0.40` → `0.50 + skill*0.30` after
        // dev_match audits showed population save% at 79% vs real ~67%
        // AND elite-keeper save% at extreme skill gaps inflated to ~91%
        // (real ~75-80%), driving the gap-9+ upset rate to 0% vs real
        // ~9%. The previous 0.36 coefficient on the skill term was the
        // dominant lever crushing weak-team conversion. Pulling it to
        // 0.30 narrows the strong-vs-weak save spread while leaving the
        // equal-skill baseline (skill_mult ≈ 0.65 at skill 0.5) in the
        // same band as before.
        let skill_mult = 0.52 + skill * 0.32;
        // Environment shifts keeper handling — heavy rain spills more,
        // wind on cross-claims has a subtler effect (the keeper still
        // sets feet under a regular shot).
        let env_mod = context.environment.modifiers();
        let env_handling_delta = env_mod.goalkeeper_handling;
        let save_prob =
            ((base - speed_penalty) * skill_mult + env_handling_delta).clamp(0.05, 0.55);

        #[cfg(feature = "match-logs")]
        save_accounting_stats::SAVE_PHYSICS_FIRED.fetch_add(1, Ordering::Relaxed);

        if context.rng.unit_f32() >= save_prob {
            return; // Keeper beaten — shot goes on.
        }
        #[cfg(feature = "match-logs")]
        save_accounting_stats::SAVE_PHYSICS_PASSED.fetch_add(1, Ordering::Relaxed);

        // Save outcome distribution. Catch / safe parry / dangerous
        // parry / corner — the previous code always caught.
        //   catch_prob   = 0.12 + handling*0.26 + positioning*0.10
        //                  + concentration*0.06
        //                  - shot_power*0.18 - reach_stretch*0.18
        //   safe_parry   = 0.20 + reflexes*0.10 + handling*0.07 + agility*0.05
        //                  + concentration*0.04
        //   dangerous    = remainder
        // Concentration shifts the split toward catch/safe parry: a
        // focused keeper does NOT spill the ball back into danger.
        let positioning = (effective_skill(keeper, keeper.skills.mental.positioning, mental_ctx)
            / 20.0)
            .clamp(0.0, 1.0);
        let shot_power_norm = (ball_speed / 8.0).clamp(0.0, 1.0);
        let reach_stretch = reach_ratio;
        let catch_prob =
            (0.12 + scaled_handling * 0.26 + positioning * 0.10 + scaled_concentration * 0.06
                - shot_power_norm * 0.18
                - reach_stretch * 0.18)
                .clamp(0.04, 0.62);
        let safe_parry_prob = (0.20
            + scaled_reflexes * 0.10
            + scaled_handling * 0.07
            + scaled_agility * 0.05
            + scaled_concentration * 0.04)
            .clamp(0.12, 0.52);

        let keeper_id = keeper.id;
        let keeper_pos = keeper.position;
        let keeper_team = keeper.team_id;
        let keeper_side = keeper.side;

        let outcome_roll = context.rng.unit_f32();
        let p_catch = catch_prob;
        let p_safe = (catch_prob + safe_parry_prob).min(0.92);

        self.position.z = 0.0;
        self.previous_owner = self.current_owner.or(self.previous_owner);
        self.pass_target_player_id = None;
        // Stage the save credit before clearing the shot target. This
        // marker is consumed by the event-dispatch step so the GK earns
        // a save in the stats sheet and the shooter's on-target count
        // increments. Without this, the physics save changes ball state
        // (catch/parry) but bypasses the state-machine save events that
        // were the only path crediting saves — leaving ~90% of resolved
        // shots stat-less.
        if let Some(shooter_id) = self.previous_owner {
            self.pending_save_credit = Some((keeper_id, shooter_id));
        }
        self.cached_shot_target = None;
        let tick = self.current_tick_cached;
        self.offside_snapshot = None;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;

        if outcome_roll < p_catch {
            // Clean catch — keeper holds.
            self.position = keeper_pos;
            self.position.z = 0.0;
            self.velocity = Vector3::zeros();
            self.current_owner = Some(keeper_id);
            self.flags.in_flight_state = 0;
            self.claim_cooldown = 200;
            self.record_touch(keeper_id, keeper_team, tick, true);
            events.add_ball_event(BallEvent::Claimed(keeper_id));
            return;
        }

        if outcome_roll < p_safe {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::SAVE_PARRY_FIRED.fetch_add(1, Ordering::Relaxed);
            // Parried OUT for a corner. The outcome is already decided, so
            // resolve it POSITIONALLY — place the ball just past the byline,
            // wide of the post — rather than driving it there by velocity.
            // The velocity approach half-failed: the keeper sits on the goal
            // line, so the ball only reached the post (y±GOAL_WIDTH) by the
            // time it crossed x=0, landing borderline → ~half fell inside
            // for a goal kick. Placing it out (outside `goal_y ± GOAL_WIDTH`,
            // a few units past x=0) makes the endline resolver award the
            // corner reliably next tick (keeper = last toucher → corner for
            // the attackers; save already booked via `pending_save_credit`).
            let goal_y_for_side = match keeper_side {
                Some(PlayerSide::Left) => context.goal_positions.left.y,
                Some(PlayerSide::Right) => context.goal_positions.right.y,
                None => self.position.y,
            };
            let to_top = self.position.y < goal_y_for_side;
            self.position.x = match keeper_side {
                Some(PlayerSide::Left) => -3.0,
                Some(PlayerSide::Right) => self.field_width + 3.0,
                None => self.position.x,
            };
            self.position.y = if to_top {
                (goal_y_for_side - GOAL_WIDTH - 10.0).max(3.0)
            } else {
                (goal_y_for_side + GOAL_WIDTH + 10.0).min(self.field_height - 3.0)
            };
            self.position.z = 0.0;
            self.velocity = Vector3::zeros();
            self.current_owner = None;
            self.flags.in_flight_state = 0;
            self.claim_cooldown = 30;
            self.record_touch(keeper_id, keeper_team, tick, false);
            // NB: do NOT emit Intercepted here — its ClaimBall follow-up
            // forces ownership onto the keeper, which CANCELS the corner
            // (the ball must stay loose and cross out). The save is already
            // booked via `pending_save_credit`, and `record_touch` marks the
            // keeper as last toucher so the endline resolver awards the
            // corner to the attackers.
            return;
        }

        // Dangerous parry — ball spills off the keeper's hands.
        // Real goalkeepers under pressure push the ball toward the side
        // they're already diving, not back into the central goalmouth
        // where the attacking team gets a free tap-in. The previous
        // ±15u y-spread around the ball position landed ~50% of parries
        // in the six-yard tap-in lane.
        let drop_distance = 12.0 + context.rng.unit_f32() * 18.0;
        let drop_x = match keeper_side {
            Some(PlayerSide::Left) => keeper_pos.x + drop_distance,
            Some(PlayerSide::Right) => keeper_pos.x - drop_distance,
            None => keeper_pos.x,
        };
        // Outward y-bias: push the ball *away* from the goal centre. If
        // the ball was already lateral, push further laterally; for
        // central shots, pick a random side and push 14-30u outward.
        let goal_center_y = match keeper_side {
            Some(PlayerSide::Left) => context.goal_positions.left.y,
            Some(PlayerSide::Right) => context.goal_positions.right.y,
            None => self.field_height * 0.5,
        };
        let outward_sign = if (self.position.y - goal_center_y).abs() < 1.0 {
            if context.rng.unit_f32() < 0.5 {
                -1.0
            } else {
                1.0
            }
        } else {
            (self.position.y - goal_center_y).signum()
        };
        let outward_offset = (14.0 + context.rng.unit_f32() * 16.0) * outward_sign;
        let drop_y = self.position.y + outward_offset + (context.rng.unit_f32() - 0.5) * 10.0;
        let drop_y = drop_y.clamp(0.0, self.field_height);
        let drop_x = drop_x.clamp(0.0, self.field_width);
        let dx = drop_x - self.position.x;
        let dy = drop_y - self.position.y;
        let dist = (dx * dx + dy * dy).sqrt().max(1.0);
        let parry_speed = 3.5_f32;
        self.velocity.x = (dx / dist) * parry_speed;
        self.velocity.y = (dy / dist) * parry_speed;
        self.velocity.z = 0.0;
        self.current_owner = None;
        self.flags.in_flight_state = 30;
        self.claim_cooldown = 0;
        self.record_touch(keeper_id, keeper_team, tick, false);
        events.add_ball_event(BallEvent::Intercepted(keeper_id, self.previous_owner));
    }
}
