//! Ball-vs-defender interactions during in-flight passes and shots:
//! interception, shot-block, and goalkeeper save. Each runs only on
//! unowned balls with `in_flight_state > 0` so routine possession
//! play isn't disturbed.

use super::Ball;
use crate::r#match::ball::events::BallEvent;
use crate::r#match::engine::goal::GOAL_WIDTH;
use crate::r#match::events::EventCollection;
use crate::r#match::{MatchContext, MatchPlayer, PlayerSide};
use crate::PlayerFieldPositionGroup;
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
            let skill_factor = (tackling + anticipation + positioning + concentration) / (4.0 * 20.0);

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
        let shot_target = match self.cached_shot_target {
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
        const BLOCK_CORRIDOR: f32 = 7.0;   // was 4u — body + stretched leg

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
            let perp = (dx - projection * shot_dir_x).powi(2)
                + (dy - projection * shot_dir_y).powi(2);
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

        // Threshold lowered from 0.08 → 0.05 so partial-line defenders
        // still occasionally block. A 5% per-attempt rate with ~13
        // shots/team gives ~0.65 additional blocks/team/match from
        // the tail — matches real football's "sliding block" variance.
        if best_chance > 0.05 {
            if let Some(blocker_id) = best_blocker {
                // Ball deflects off the defender.
                //
                // Critical correction from the first version of this
                // code: previously we just scaled velocity by 0.2 and
                // made the blocker owner. That kept the ball moving
                // toward the defender's own goal at ~1.1 u/tick — and
                // since the ball owner can drift up to 15u from the
                // ball before ownership drops, a block inside the
                // penalty box would see the ball escape ownership and
                // cross the goal line as an unowned ball → registered
                // as a goal from the attacker's side. That produced
                // matches like "9 goals from 6 shots" in real-data
                // sims and was the source of the remaining blowout
                // outliers.
                //
                // Fix: snap the ball to the blocker's position and
                // reverse it along the shot direction. Real deflected
                // shots rebound away from the defender's goal, not
                // through it. Small random angle + outward component
                // so the ball doesn't reliably come back on the same
                // line.
                if let Some(blocker) = players.iter().find(|p| p.id == blocker_id) {
                    self.position = blocker.position;
                    self.position.z = 0.0;
                }
                let reverse_speed = ball_velocity_2d * 0.25;
                // Random deflection: ±45° off the reverse direction.
                let angle: f32 = (rand::random::<f32>() - 0.5) * 1.56;
                let rev_x = -shot_dir_x * angle.cos() - (-shot_dir_y) * angle.sin();
                let rev_y = -shot_dir_x * angle.sin() + (-shot_dir_y) * angle.cos();
                self.velocity.x = rev_x * reverse_speed;
                self.velocity.y = rev_y * reverse_speed;
                self.velocity.z = 0.0;

                self.previous_owner = self.current_owner.or(self.previous_owner);
                self.current_owner = Some(blocker_id);
                self.pass_target_player_id = None;
                self.flags.in_flight_state = 0;
                self.claim_cooldown = 20;
                self.cached_shot_target = None;
                events.add_ball_event(BallEvent::Intercepted(blocker_id, self.previous_owner));
                let _ = shot_target;
            }
        }
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
            PlayerSide::Right => (context.goal_positions.right.x, context.goal_positions.right.y),
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

        // Save made. Stop the ball, hand it to the keeper, clear the
        // shot flag, and emit BallEvent::Claimed. The save is silent
        // at this layer — handle_caught_ball_event won't credit
        // because cached_shot_target is gone — but the GK Catching
        // state running a per-tick save roll BEFORE this physics-level
        // save fires already credits saves through the gated handler.
        // try_save_shot is the safety net that catches shots the
        // state-machine catch missed.
        self.position.z = 0.0;
        self.velocity = Vector3::zeros();
        self.previous_owner = self.current_owner.or(self.previous_owner);
        self.current_owner = Some(keeper.id);
        self.pass_target_player_id = None;
        self.flags.in_flight_state = 0;
        self.cached_shot_target = None;
        self.claim_cooldown = 200;
        events.add_ball_event(BallEvent::Claimed(keeper.id));
    }
}
