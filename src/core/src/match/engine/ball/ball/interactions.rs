//! Ball-vs-defender interactions during in-flight passes and shots:
//! interception, shot-block, and goalkeeper save. Each runs only on
//! unowned balls with `in_flight_state > 0` so routine possession
//! play isn't disturbed.

use super::Ball;
use crate::PlayerFieldPositionGroup;
use crate::r#match::ball::events::BallEvent;
use crate::r#match::engine::goal::GOAL_WIDTH;
use crate::r#match::events::EventCollection;
use crate::r#match::{MatchContext, MatchPlayer, PlayerSide};
use nalgebra::Vector3;

impl Ball {
    /// Opposing players near the ball's flight path can intercept passes.
    /// Interception chance depends on tackling, anticipation, positioning skills
    /// and proximity to the ball's trajectory.
    pub fn try_intercept(&mut self, players: &[MatchPlayer], events: &mut EventCollection) {
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
        // 0.125m. Old 2.5u = 0.31m left average defenders mathematically
        // unable to intercept (max score 0.039 vs 0.04 threshold). First
        // pass at 8u was too generous — per-tick chance ~0.05 over 3 ticks
        // of ball-brush gave ~40% cumulative rate per pass across 2-3
        // defenders, well above real football's ~15%. Produced constant
        // intercept→snap→claim-cooldown→re-pass cycles that the user
        // observed as "ball uncontrolled 80% of match". 5u (~0.6m — a
        // defender's leg-extension radius) strikes the realistic balance.
        const INTERCEPT_RADIUS: f32 = 5.0;
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

            // Calculate interception probability from player skills
            let tackling = player.skills.technical.tackling;
            let anticipation = player.skills.mental.anticipation;
            let positioning = player.skills.mental.positioning;
            let concentration = player.skills.mental.concentration;

            // Base chance: average of key defensive skills (0-20 scale → 0-1)
            let skill_factor =
                (tackling + anticipation + positioning + concentration) / (4.0 * 20.0);

            // Proximity factor: closer = higher chance (1.0 at 0m, 0.3 at max radius)
            let dist = dist_sq.sqrt();
            let proximity_factor = 1.0 - (dist / INTERCEPT_RADIUS) * 0.7;

            // Fast passes are harder to intercept — penalty coefficient
            // moderated from 0.10 (which made 7 u/tick passes 41% harder
            // than slow ones) back toward a lighter slope.
            let speed_penalty = 1.0 / (1.0 + ball_speed_sq.sqrt() * 0.06);

            // Per-tick interception chance. Tuned so average defenders
            // well-positioned can intercept (original 0.08 made that
            // mathematically impossible against the 0.04 threshold) but
            // cumulative per-pass rate lands near real-football ~15%
            // rather than 40%+.
            let chance = skill_factor * proximity_factor * speed_penalty * 0.13;

            if chance > best_chance {
                best_chance = chance;
                best_interceptor = Some(player.id);
            }
        }

        // Deterministic threshold. Avg defender (skill 0.5) at 60% of
        // reach with a typical pass score ~0.040 — just above the bar —
        // so most in-path defenders qualify, but peripheral ones don't.
        if best_chance > 0.035 {
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
                self.pass_origin_restart =
                    crate::r#match::PassOriginRestart::OpenPlay;
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
    pub fn try_block_shot(&mut self, players: &[MatchPlayer], events: &mut EventCollection) {
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
            // reading the shooter's body shape.
            let bravery = player.skills.mental.bravery;
            let positioning = player.skills.mental.positioning;
            let anticipation = player.skills.mental.anticipation;
            let agility = player.skills.physical.agility;
            let tackling = player.skills.technical.tackling;
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
            Some(id) if rand::random::<f32>() < best_chance.clamp(0.03, 0.38) => id,
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
        let angle: f32 = (rand::random::<f32>() - 0.5) * 1.56;
        let rev_x = -shot_dir_x * angle.cos() - (-shot_dir_y) * angle.sin();
        let rev_y = -shot_dir_x * angle.sin() + (-shot_dir_y) * angle.cos();
        let tick = self.current_tick_cached;

        let roll = rand::random::<f32>();
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
        self.pass_origin_restart = crate::r#match::PassOriginRestart::OpenPlay;

        if roll < p_controlled {
            // Clean block — defender gets the ball at his feet.
            self.velocity = Vector3::zeros();
            self.current_owner = Some(blocker_id);
            self.flags.in_flight_state = 0;
            self.claim_cooldown = 25;
            events.add_ball_event(BallEvent::Intercepted(blocker_id, self.previous_owner));
            return;
        }

        if roll < p_corner {
            // Deflection wide — push the ball toward the defender's own
            // endline so it crosses out of play. The endline resolver
            // will then award a corner (defender = last toucher, side
            // matches → corner for attackers).
            let endline_x = match blocker_side {
                Some(crate::r#match::PlayerSide::Left) => 0.0_f32,
                Some(crate::r#match::PlayerSide::Right) => self.field_width,
                None => self.position.x + rev_x * 8.0,
            };
            let dx = endline_x - self.position.x;
            let dist = dx.abs().max(1.0);
            let speed = (ball_velocity_2d * 0.6).clamp(2.0, 5.0);
            self.velocity.x = (dx / dist) * speed;
            // Slight outward y-component so the ball goes wide of the post.
            self.velocity.y = if rand::random::<f32>() < 0.5 { -1.0 } else { 1.0 } * 1.2;
            self.velocity.z = 0.0;
            self.current_owner = None;
            self.flags.in_flight_state = 30;
            self.claim_cooldown = 0;
            events.add_ball_event(BallEvent::Intercepted(blocker_id, self.previous_owner));
            return;
        }

        if roll < p_safe {
            // Safe sideways deflection — perpendicular skip away from
            // both goals. Loose ball; either team can recover.
            let safe_speed = (ball_velocity_2d * 0.35).clamp(1.5, 3.5);
            // Rotate shot direction 90° (sign chosen by random) to skip sideways.
            let sign = if rand::random::<f32>() < 0.5 { -1.0 } else { 1.0 };
            self.velocity.x = -shot_dir_y * sign * safe_speed;
            self.velocity.y = shot_dir_x * sign * safe_speed;
            self.velocity.z = 0.0;
            self.current_owner = None;
            self.flags.in_flight_state = 25;
            self.claim_cooldown = 0;
            events.add_ball_event(BallEvent::Intercepted(blocker_id, self.previous_owner));
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
            events.add_ball_event(BallEvent::Intercepted(blocker_id, self.previous_owner));
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
        events.add_ball_event(BallEvent::Intercepted(blocker_id, self.previous_owner));
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
            self.cached_shot_target = None;
            return;
        }

        let dist_to_goal_x = (self.position.x - goal_x).abs();
        let ball_vx = self.velocity.x.abs().max(0.5);
        if dist_to_goal_x > ball_vx * 2.5 {
            return;
        }

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

        let handling = keeper.skills.goalkeeping.handling;
        let reflexes = keeper.skills.goalkeeping.reflexes;
        let agility = keeper.skills.physical.agility;
        let scaled_handling = ((handling - 1.0) / 19.0).max(0.0);
        let scaled_reflexes = ((reflexes - 1.0) / 19.0).max(0.0);
        let scaled_agility = ((agility - 1.0) / 19.0).max(0.0);

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
        let skill = scaled_handling * 0.4 + scaled_reflexes * 0.4 + scaled_agility * 0.2;
        // Per-tick save rate. This save check runs every tick the ball
        // is within reach of the goal line, AND the GK state-machine
        // (Catching, Diving) runs its OWN per-tick save roll. Both
        // compound across the 5-15 ticks of shot flight, so per-tick
        // rates need to be calibrated to give a CUMULATIVE ~67% save
        // rate (real-world). Old skill_mult 0.72-1.07 + cap 0.96 left
        // the cumulative chance >95% for any centred shot. New
        // calibration: skill_mult 0.45-0.80, cap 0.55, so a centred
        // shot (base 0.88) at average skill (mult 0.6) lands per-tick
        // at 0.50, cumulative ~75% across the contact window — the
        // catching-state rolls add another tick and drop the leak rate
        // toward the realistic 30%.
        let skill_mult = 0.45 + skill * 0.35;
        let save_prob = ((base - speed_penalty) * skill_mult).clamp(0.05, 0.55);

        if rand::random::<f32>() >= save_prob {
            return; // Keeper beaten — shot goes on.
        }

        // Save outcome distribution. Catch / safe parry / dangerous
        // parry / corner — the previous code always caught.
        //   catch_prob   = 0.12 + handling*0.28 + positioning*0.12
        //                  - shot_power*0.18 - reach_stretch*0.18
        //   safe_parry   = 0.20 + reflexes*0.12 + handling*0.08 + agility*0.05
        //   dangerous    = remainder
        let positioning = (keeper.skills.mental.positioning / 20.0).clamp(0.0, 1.0);
        let shot_power_norm = (ball_speed / 8.0).clamp(0.0, 1.0);
        let reach_stretch = reach_ratio;
        let catch_prob = (0.12 + scaled_handling * 0.28 + positioning * 0.12
            - shot_power_norm * 0.18
            - reach_stretch * 0.18)
            .clamp(0.04, 0.62);
        let safe_parry_prob = (0.20 + scaled_reflexes * 0.12 + scaled_handling * 0.08
            + scaled_agility * 0.05)
            .clamp(0.12, 0.52);

        let keeper_id = keeper.id;
        let keeper_pos = keeper.position;
        let keeper_team = keeper.team_id;
        let keeper_side = keeper.side;

        let outcome_roll = rand::random::<f32>();
        let p_catch = catch_prob;
        let p_safe = (catch_prob + safe_parry_prob).min(0.92);

        self.position.z = 0.0;
        self.previous_owner = self.current_owner.or(self.previous_owner);
        self.pass_target_player_id = None;
        self.cached_shot_target = None;
        let tick = self.current_tick_cached;
        self.offside_snapshot = None;
        self.pass_origin_restart = crate::r#match::PassOriginRestart::OpenPlay;

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
            // Safe parry — palmed wide for a corner OR over the bar.
            // Push the ball toward the keeper's own endline outside the
            // post so the endline resolver awards a corner.
            let endline_x = match keeper_side {
                Some(crate::r#match::PlayerSide::Left) => -1.0_f32,
                Some(crate::r#match::PlayerSide::Right) => self.field_width + 1.0,
                None => self.position.x,
            };
            let dx = endline_x - self.position.x;
            let dist = dx.abs().max(1.0);
            let parry_speed = 4.0_f32;
            self.velocity.x = (dx / dist) * parry_speed;
            // Sideways spread so it goes wide of the post.
            let goal_y_for_side = match keeper_side {
                Some(crate::r#match::PlayerSide::Left) => context.goal_positions.left.y,
                Some(crate::r#match::PlayerSide::Right) => context.goal_positions.right.y,
                None => self.position.y,
            };
            let sign = if self.position.y < goal_y_for_side {
                -1.0
            } else {
                1.0
            };
            self.velocity.y = sign * 2.5;
            self.velocity.z = 0.0;
            self.current_owner = None;
            self.flags.in_flight_state = 30;
            self.claim_cooldown = 0;
            self.record_touch(keeper_id, keeper_team, tick, false);
            // Save was successful but ball is still loose — emit
            // Intercepted so the rating helper still sees the keeper
            // touched the ball, and the endline resolver awards the
            // corner on the next tick.
            events.add_ball_event(BallEvent::Intercepted(keeper_id, self.previous_owner));
            return;
        }

        // Dangerous parry — ball drops 8-28u from goal, often central.
        let drop_distance = 8.0 + rand::random::<f32>() * 20.0;
        let drop_x = match keeper_side {
            Some(crate::r#match::PlayerSide::Left) => keeper_pos.x + drop_distance,
            Some(crate::r#match::PlayerSide::Right) => keeper_pos.x - drop_distance,
            None => keeper_pos.x,
        };
        let drop_y = self.position.y
            + (rand::random::<f32>() - 0.5) * 30.0;
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
