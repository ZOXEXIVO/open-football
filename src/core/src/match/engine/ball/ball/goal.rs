//! Out-of-play resolution: actual goals, over-the-bar goal kicks,
//! and wide-of-goal corner / goal kick decisions. The wide-of-goal
//! flow stages the set-piece teleport via `pending_set_piece_teleport`
//! since the ball can't move other players' positions itself.

use super::Ball;
use crate::r#match::PassOriginRestart;
use crate::r#match::ball::events::{BallEvent, BallGoalEventMetadata, GoalSide};
use crate::r#match::engine::corner_shape::{CornerShape, CornerShapeHold};
use crate::r#match::engine::goal::GOAL_WIDTH;
use crate::r#match::engine::set_pieces::{
    CornerRoutine, pick_corner_routine, score_corner_routines, score_corner_taker,
};
use crate::r#match::events::EventCollection;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchContext, MatchPlayer, PlayerSide};
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::EndlineCensus;
use nalgebra::Vector3;
use std::cmp::Ordering;

impl Ball {
    /// Aerial match-up for a delivery into the box, as a 0..1 score where
    /// 0.5 is parity, >0.5 means the attacking side has the better heads.
    ///
    /// Reads the three best aerial threats against the three best aerial
    /// defenders rather than averaging the whole XI — a corner is contested
    /// by the big men who go up for it, and averaging in eight players who
    /// will never jump reports every side as identically average, which is
    /// what makes a routine chooser fed by it pick the same routine forever.
    fn box_aerial_advantage(
        players: &[MatchPlayer],
        attacking_side: PlayerSide,
        minute: u32,
    ) -> f32 {
        let mut att: Vec<f32> = Vec::with_capacity(10);
        let mut def: Vec<f32> = Vec::with_capacity(10);
        for p in players {
            if p.is_sent_off || p.tactical_position.current_position.is_goalkeeper() {
                continue;
            }
            match p.side {
                Some(s) if s == attacking_side => att.push(sc::aerial_outfield_attacker(p, minute)),
                Some(_) => def.push(sc::aerial_outfield_defender(p, minute)),
                None => {}
            }
        }
        let best_three = |mut v: Vec<f32>| -> f32 {
            if v.is_empty() {
                return 0.5;
            }
            v.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
            let n = v.len().min(3);
            v.iter().take(n).sum::<f32>() / n as f32
        };
        (0.5 + (best_three(att) - best_three(def))).clamp(0.0, 1.0)
    }

    // `pub(crate)` rather than `pub(super)` so the goal / celebration
    // integration tests can drive a goal directly. The alternative is
    // standing up a full `GameTickContext` to reach it through `update`,
    // which tests the tick orchestration rather than the goal.
    pub(crate) fn check_goal(&mut self, context: &MatchContext, result: &mut EventCollection) {
        // Guard: don't detect another goal if one was already scored this
        // tick, and none at all while the ball is still in the goal from the
        // last one. `goal_scored` alone used to be enough because the ball
        // was teleported to the centre spot on the same tick; it now lives
        // in the net until the restart, sitting permanently inside
        // `is_goal`, so the durable marker is the one that has to gate this.
        if self.goal_scored || self.in_net.is_some() {
            return;
        }

        // Don't detect goals when ball is attached to a player (ball follows owner).
        // Goals only happen when the ball crosses the line freely (shot, deflection, etc.).
        // This prevents defenders "carrying" the ball into their own goal via boundary clamping.
        if self.current_owner.is_some() {
            return;
        }

        if let Some(goal_side) = context.goal_positions.is_goal(self.position) {
            // Prefer current_owner (e.g. player carrying ball into goal)
            // Fall back to previous_owner (e.g. shooter or passer whose ball went in)
            if let Some(goalscorer) = self.current_owner.or(self.previous_owner) {
                let Some(player) = context.players.by_id(goalscorer) else {
                    return;
                };
                let is_auto_goal = match player.side {
                    Some(PlayerSide::Left) => goal_side == GoalSide::Home,
                    Some(PlayerSide::Right) => goal_side == GoalSide::Away,
                    _ => false,
                };

                // Require a recent shot or a live shot-target. Without
                // this, passes that happen to roll across the goal line
                // (receiver missed, ball trajectory drifted) credit the
                // passer with a goal — which was producing 10-15 "goals"
                // per match per team that never involved a Shoot event.
                // Real football treats those as out-of-bounds → goal
                // kick, not a goal. Exception: auto-goal path skips this
                // check, because an own goal happens via touch, not a
                // shot by the credited player.
                if !is_auto_goal {
                    let current_tick = context.current_tick();
                    let recent_shot = context
                        .players
                        .by_id(goalscorer)
                        .map(|p| {
                            p.memory.shots_taken > 0
                                && current_tick.saturating_sub(p.memory.last_shot_tick) < 300
                        })
                        .unwrap_or(false);
                    let shot_in_flight = self.cached_shot_target.is_some();
                    // Whether the BALL was struck as a shot recently, no
                    // matter who has touched it since. The two tests above
                    // both ask about the player being credited, and that
                    // is the wrong subject once anyone else intervenes: a
                    // keeper who gets a hand to a shot becomes the ball's
                    // `previous_owner`, has taken no shot himself, and
                    // clears `cached_shot_target` on the way — so a
                    // parried or deflected effort crossing the line failed
                    // both and was waved away. Measured at 2604 refused
                    // goals per 300 matches, 34% of every shot taken.
                    // 400 ticks (~4 s) comfortably covers a strike, a
                    // deflection and the ball rolling over the line.
                    let ball_came_from_a_shot = self.last_shot_struck_tick > 0
                        && current_tick.saturating_sub(self.last_shot_struck_tick) < 400;
                    if !recent_shot && !shot_in_flight && !ball_came_from_a_shot {
                        #[cfg(feature = "match-logs")]
                        super::ownership::reception_diag::GOAL_REJECTED
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        // Not a shot — treat as ball out of play, not a goal.
                        return;
                    }

                    // Indirect free-kick rule: the kick itself can't
                    // produce a goal. If the ball came from an
                    // IndirectFreeKick origin and only the taker has
                    // touched it since (no second player), the goal
                    // must not stand. We approximate "no second touch"
                    // by checking that the taker is the SOLE recent
                    // passer. If anyone else is in `recent_passers`,
                    // somebody has taken a touch and a goal is legal.
                    if self.pass_origin_restart == PassOriginRestart::IndirectFreeKick {
                        let any_second_touch = self
                            .recent_passers
                            .iter()
                            .any(|e| e.player_id != goalscorer);
                        if !any_second_touch {
                            // Reject: ball stays live, but no goal.
                            return;
                        }
                    }
                }

                // Deflection fix: if this would be an own goal but the player only just
                // touched the ball (deflection/failed save), credit the goal to the
                // previous owner (the attacker who actually shot) instead.
                // A genuine own goal requires the defender to have had meaningful possession.
                let (final_scorer, final_is_auto_goal) =
                    if is_auto_goal && self.ownership_duration < 30 {
                        // Check if previous_owner is from the opposing team (the attacker)
                        let attacker = if self.current_owner == Some(goalscorer) {
                            self.previous_owner
                        } else {
                            // goalscorer came from previous_owner, check recent_passers
                            self.recent_passers
                                .iter()
                                .rev()
                                .find(|e| e.player_id != goalscorer)
                                .map(|e| e.player_id)
                        };

                        if let Some(attacker_id) = attacker {
                            if let Some(attacker_player) = context.players.by_id(attacker_id) {
                                // Verify attacker is from the other team
                                let attacker_would_score = match attacker_player.side {
                                    Some(PlayerSide::Left) => goal_side != GoalSide::Home,
                                    Some(PlayerSide::Right) => goal_side != GoalSide::Away,
                                    _ => false,
                                };
                                if attacker_would_score {
                                    // Credit the attacker — this was a deflection, not a real own goal
                                    (attacker_id, false)
                                } else {
                                    (goalscorer, true)
                                }
                            } else {
                                (goalscorer, true)
                            }
                        } else {
                            (goalscorer, true)
                        }
                    } else {
                        (goalscorer, is_auto_goal)
                    };

                // Find the assist provider. `assist_for_goal` enforces the
                // teammate / same-possession / recency rules — see its doc
                // comment. An own goal never carries an assist.
                let assist_player_id = if !final_is_auto_goal {
                    context.players.by_id(final_scorer).and_then(|scorer| {
                        self.assist_for_goal(final_scorer, scorer.team_id, context.current_tick())
                    })
                } else {
                    None
                };

                let goal_event_metadata = BallGoalEventMetadata {
                    side: goal_side,
                    goalscorer_player_id: final_scorer,
                    assist_player_id,
                    auto_goal: final_is_auto_goal,
                };

                result.add_ball_event(BallEvent::Goal(goal_event_metadata));

                // Hand the ball to the netting rather than teleporting it to
                // the centre spot. It keeps the pace it crossed the line
                // with, stretches the mesh, and settles in the goal — see
                // `net.rs` for why the ball used to appear to stop dead on
                // the line. The restart puts it back on the centre spot
                // once the celebration is over.
                self.enter_net(goal_side, final_scorer, final_is_auto_goal);
            }

            // Determine which side should kick off (the conceding team)
            // Home goal (x=0) = Left side conceded → Left kicks off
            // Away goal (x=field_width) = Right side conceded → Right kicks off
            self.kickoff_team_side = match goal_side {
                GoalSide::Home => Some(PlayerSide::Left),
                GoalSide::Away => Some(PlayerSide::Right),
            };

            self.goal_scored = true;
            // No scorer could be credited (a goal with no owner and no
            // previous owner — a ball that crossed the line off nobody).
            // There is no goal event and no celebration to run, so the ball
            // still needs the old behaviour: straight back to the centre.
            if self.in_net.is_none() {
                self.reset();
            }
        }
    }

    /// Where the defending side restarts from after the ball goes out
    /// over their own goal line.
    ///
    /// Both goal-kick sites used to place the ball on ONE point —
    /// `(±50, goal_positions.left.y)` — for every restart at either end,
    /// which is the single spot in front of the goal a replay shows the
    /// ball blinking to after every wide or skied shot. It is also not
    /// how a goal kick is taken: the ball is put down in the goal area on
    /// the side it went out. Reading `left.y` for the RIGHT-hand goal was
    /// wrong on its own terms too — the two centres are built from
    /// `height as f32 / 2.0` and `(height / 2) as f32`, so they differ by
    /// half a unit on an odd-height pitch.
    ///
    /// Carrying the exit point through keeps the spot continuous in where
    /// the ball actually left the pitch, so it varies the way the real
    /// thing does without spending a roll on it.
    fn goal_kick_spot(&self, side: GoalSide, goal_center_y: f32, exit_y: f32) -> Vector3<f32> {
        // Goal area: 5.5 m deep, 9.16 m either side of the goal centre.
        // At 0.125 m/unit that is 44u and 73u — the 50u depth used here
        // puts the ball just outside the six-yard line, where keepers
        // actually tee it up.
        const GOAL_AREA_HALF_WIDTH: f32 = 73.0;
        const GOAL_KICK_DEPTH: f32 = 50.0;
        let x = match side {
            GoalSide::Home => GOAL_KICK_DEPTH,
            GoalSide::Away => self.field_width - GOAL_KICK_DEPTH,
        };
        let offset = (exit_y - goal_center_y).clamp(-GOAL_AREA_HALF_WIDTH, GOAL_AREA_HALF_WIDTH);
        Vector3::new(x, goal_center_y + offset * 0.75, 0.0)
    }

    /// Ball crossed goal line within goal width but above crossbar — goal kick.
    /// Place ball near the 6-yard box and give it to the defending goalkeeper.
    pub(super) fn check_over_goal(
        &mut self,
        context: &mut MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // A ball already in the goal is not a ball going out of play — see
        // `Ball::in_net`. Without this, one settling against the roof
        // netting reads as a skied shot and awards a goal kick.
        if self.goal_scored || self.in_net.is_some() {
            return;
        }

        let over_side = match context.goal_positions.is_over_goal(self.position) {
            Some(side) => side,
            None => return,
        };

        #[cfg(feature = "match-logs")]
        if self.cached_shot_target.is_some() {
            super::ownership::reception_diag::SHOT_OVER
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        // Determine which side's goalkeeper defends this goal
        // GoalSide::Home = left goal (x=0) → defended by PlayerSide::Left
        // GoalSide::Away = right goal (x=field_width) → defended by PlayerSide::Right
        let defending_side = match over_side {
            GoalSide::Home => PlayerSide::Left,
            GoalSide::Away => PlayerSide::Right,
        };

        // Find the goalkeeper on the defending side
        if let Some(gk) = players.iter().find(|p| {
            p.side == Some(defending_side) && p.tactical_position.current_position.is_goalkeeper()
        }) {
            // Place the ball in the goal area, on the side it went out.
            let goal_center_y = match over_side {
                GoalSide::Home => context.goal_positions.left.y,
                GoalSide::Away => context.goal_positions.right.y,
            };
            let spot = self.goal_kick_spot(over_side, goal_center_y, self.position.y);
            self.position = spot;
            self.velocity = Vector3::zeros();

            // Give ball to goalkeeper
            let gk_id = gk.id;
            let gk_team = gk.team_id;
            self.current_owner = Some(gk_id);
            self.previous_owner = None;
            self.ownership_duration = 0;
            self.claim_cooldown = 30; // Protection so no one steals immediately
            self.flags.in_flight_state = 30;
            self.pass_target_player_id = None;
            // Clear the shot target — the shot ended (above the bar) and
            // is now resolved as a goal kick. Without this clear, the
            // GK's eventual ClearBall event hits gk_clearing_shot with
            // a stale `cached_shot_target=Some`, false-crediting a save
            // for a shot that never reached the keeper.
            self.cached_shot_target = None;
            self.recent_passers.clear();
            // Dead ball: drop every open-play window in one place so a
            // pass that was live when the ball went out cannot survive the
            // restart and swallow the restart pass.
            self.clear_open_play_metadata();
            self.pass_origin_restart = PassOriginRestart::GoalKick;
            self.offside_snapshot = None;
            self.record_touch(gk_id, gk_team, self.current_tick_cached, true);

            events.add_ball_event(BallEvent::Claimed(gk_id));
        }
    }

    /// Ball crossed the endline (x <= 0 or x >= field_width) but OUTSIDE the goal posts.
    /// In real football this is a goal kick OR a corner kick — depending on
    /// which team last touched the ball.
    pub(super) fn check_wide_of_goal(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // The ball is in the net. It is past the endline and between the
        // posts, which is precisely the shape this resolver exists to catch
        // — and it must not, because a ball that has gone IN is not a ball
        // that has gone OUT. Ahead of everything else in here: the corner /
        // goal-kick decision below would otherwise fire on every tick of
        // every celebration.
        if self.goal_scored || self.in_net.is_some() {
            return;
        }

        let field_width = context.field_size.width as f32;
        let goal_half_width = GOAL_WIDTH;

        // Which endline, if either, the ball is past.
        let (crossed_side, goal_center_y) = if self.position.x <= 0.0 {
            (Some(GoalSide::Home), context.goal_positions.left.y)
        } else if self.position.x >= field_width {
            (Some(GoalSide::Away), context.goal_positions.right.y)
        } else {
            (None, 0.0)
        };

        let side = match crossed_side {
            Some(s) => s,
            None => return,
        };

        // Between the posts this used to return, on the assumption that
        // `check_goal` / `check_over_goal` had it covered. They do not
        // cover the case between them. `check_goal` REFUSES a ball that
        // crosses the line with no shot behind it — a cross carrying
        // through the six-yard box, a through-ball hit too hard — and
        // returns without restarting play; `check_over_goal` only takes
        // balls above the bar. Measured at 22 refused balls a match.
        //
        // Nothing downstream owned them either, so
        // `check_boundary_collision` had the last word: it clamped the
        // ball back to x = ±10 and zeroed its velocity, leaving it dead
        // in the goalmouth a metre off the line for whoever got there
        // first. That is the ball "appearing at a single point in front
        // of the goal", and play never restarted from any of them.
        //
        // A ball wholly over the goal line is out of play whatever its
        // height, so the corner-or-goal-kick resolver below is the right
        // answer for all of them. Restricted to a LOOSE ball: the
        // position clamp can pin a keeper on his own goal line, and a
        // keeper standing there with it in his gloves is holding the
        // ball, not putting it out.
        let outside_posts = (self.position.y - goal_center_y).abs() > goal_half_width;
        if !outside_posts && self.current_owner.is_some() {
            return;
        }

        #[cfg(feature = "match-logs")]
        if self.cached_shot_target.is_some() {
            super::ownership::reception_diag::SHOT_WIDE
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        let defending_side = match side {
            GoalSide::Home => PlayerSide::Left,
            GoalSide::Away => PlayerSide::Right,
        };
        let attacking_side = match defending_side {
            PlayerSide::Left => PlayerSide::Right,
            PlayerSide::Right => PlayerSide::Left,
        };

        // Decide corner vs goal kick from the last player who TOUCHED the
        // ball. If the defending team put it out, it's a corner for the
        // attacking team. Use `last_touch_player_id` (the true last contact,
        // maintained by `record_touch` on every control change / block /
        // save) rather than `previous_owner` (the last OWNER). They differ
        // exactly on a DEFLECTION: when a defender blocks/parries/clears a
        // shot out, `previous_owner` is still the attacking SHOOTER, so the
        // ball was wrongly given as a goal kick — which is the dominant
        // reason the engine ran ~0.5 corners/match vs ~10 real. Falls back
        // to the owner when no touch is recorded.
        let last_toucher_side: Option<PlayerSide> = self
            .last_touch_player_id
            .or(self.previous_owner)
            .or(self.current_owner)
            .and_then(|pid| players.iter().find(|p| p.id == pid))
            .and_then(|p| p.side);

        let is_corner = last_toucher_side == Some(defending_side);

        // Endline census — total crossings and how they split. A drop in
        // corners can mean either "fewer balls go out behind" or "the same
        // number go out but off the attacker", and the two have opposite
        // fixes. The corner count alone cannot distinguish them.
        #[cfg(feature = "match-logs")]
        {
            let last_state = self
                .last_touch_player_id
                .and_then(|pid| players.iter().find(|p| p.id == pid))
                .map(|p| p.state.compact_id())
                .unwrap_or(0);
            EndlineCensus::note(is_corner, last_state);
            // …and when it is a goal kick, how far did the ball run after
            // the last man touched it, and what was he doing? A pass
            // struck too hard and a clearance hammered out are the same
            // count and completely different bugs.
            if !is_corner {
                let run = (self.position - self.last_touch_position).magnitude();
                let toucher_state = self
                    .last_touch_player_id
                    .and_then(|pid| players.iter().find(|p| p.id == pid))
                    .map(|p| p.state.compact_id())
                    .unwrap_or(0);
                // Was this a MISSED SHOT rather than a pass? The last
                // toucher being the last shooter is the test. The states
                // above cannot answer it: they are read now, and a shooter
                // is back in an off-ball state long before his shot
                // reaches the line.
                let was_shot = self.last_shot_shooter_id.is_some()
                    && self.last_shot_shooter_id == self.last_touch_player_id;
                let speed = Vector3::new(self.velocity.x, self.velocity.y, 0.0).magnitude();
                EndlineCensus::note_goal_kick_run(toucher_state, run, was_shot, speed);
            }
        }

        if is_corner {
            // Attacking team gets a corner. Place ball at the nearest corner
            // flag and hand it to the attacking team's best corner taker.
            let corner_x = match side {
                GoalSide::Home => 2.0,
                GoalSide::Away => field_width - 2.0,
            };
            let field_height = context.field_size.height as f32;
            // Pick the near corner based on where the ball went out
            let near_top = self.position.y < field_height * 0.5;
            let corner_y = if near_top { 2.0 } else { field_height - 2.0 };

            // Find the attacking team's designated corner taker via the
            // shared `score_corner_taker` blend (corners 0.45 / crossing
            // 0.30 / technique 0.15 / vision 0.10), restricted to players
            // currently on the pitch.
            //
            // This used to be a local `crossing*0.6 + technique*0.3 +
            // corners*0.1`, which put a player's actual corner attribute
            // at a *tenth* of the decision — so the best crosser took every
            // corner and a dedicated corner specialist essentially never
            // took his own. `corners` reached nothing else in the engine,
            // which made it the one attribute a player could max out for
            // no in-match effect whatsoever.
            let score_taker = |p: &MatchPlayer| {
                score_corner_taker(
                    p.skills.technical.corners,
                    p.skills.technical.crossing,
                    p.skills.technical.technique,
                    p.skills.mental.vision,
                )
            };
            let taker = players
                .iter()
                .filter(|p| {
                    p.side == Some(attacking_side)
                        && !p.is_sent_off
                        && !p.tactical_position.current_position.is_goalkeeper()
                })
                .max_by(|a, b| {
                    score_taker(a)
                        .partial_cmp(&score_taker(b))
                        .unwrap_or(Ordering::Equal)
                });

            if let Some(taker) = taker {
                let taker_id = taker.id;
                let taker_team = taker.team_id;
                self.position.x = corner_x;
                self.position.y = corner_y;
                self.position.z = 0.0;
                self.velocity = Vector3::zeros();

                self.current_owner = Some(taker_id);
                self.previous_owner = None;
                self.ownership_duration = 0;
                self.claim_cooldown = 30;
                self.flags.in_flight_state = 30;
                self.pass_target_player_id = None;
                self.recent_passers.clear();
                // Same as goal-kick restart: clear stale shot target so
                // the eventual clearance/distribution doesn't false-credit
                // a phantom save (see check_over_goal for the full bug
                // explanation).
                self.cached_shot_target = None;
                // Dead ball: drop every open-play window in one place so a
                // pass that was live when the ball went out cannot survive the
                // restart and swallow the restart pass.
                self.clear_open_play_metadata();
                self.pass_origin_restart = PassOriginRestart::Corner;
                // Pick the corner routine. The five routine scores used to
                // be five hardcoded constants, which meant the routine was
                // decided by the repeat-blocker alone: identical for a
                // Premier League set-piece side and a pub team, in a gale
                // and in still air, 3-0 up and 0-1 down. `score_corner_routines`
                // is the intended model and it reads the taker's corners /
                // crossing, the aerial match-up in the box, the keeper's
                // command of his area, the score state and the weather.
                let minute = (context.total_match_time / 60_000) as u32;
                let is_home_attacking = taker_team == context.field_home_team_id;
                let (att_goals, def_goals) = if is_home_attacking {
                    (context.score.home_team.get(), context.score.away_team.get())
                } else {
                    (context.score.away_team.get(), context.score.home_team.get())
                };
                let late = minute >= 70;
                let chasing_late = late && att_goals < def_goals;
                let protecting_lead = late && att_goals > def_goals;

                let aerial_advantage = Self::box_aerial_advantage(players, attacking_side, minute);
                let gk_aerial = players
                    .iter()
                    .find(|p| {
                        p.side != Some(attacking_side)
                            && p.tactical_position.current_position.is_goalkeeper()
                    })
                    .map(|gk| {
                        ((gk.skills.goalkeeping.command_of_area * 0.55
                            + gk.skills.goalkeeping.aerial_reach * 0.45)
                            / 20.0)
                            .clamp(0.0, 1.0)
                    })
                    .unwrap_or(0.5);

                let scores = score_corner_routines(
                    taker.skills.technical.corners,
                    taker.skills.technical.crossing,
                    aerial_advantage,
                    gk_aerial,
                    chasing_late,
                    protecting_lead,
                    &context.environment,
                );
                let chosen_routine =
                    pick_corner_routine(&scores, &context.set_piece_history, is_home_attacking);
                self.pending_corner_routine = Some(chosen_routine);
                // Stamp the taker's delivery quality so `resolve_corner_contest`
                // can weigh the ball that actually arrives. Without it the
                // contest reads only the two aerial duellists and the keeper —
                // an elite dead-ball specialist and a centre-half hitting the
                // first man produced identical corners.
                self.pending_corner_delivery = sc::set_piece_delivery(taker, minute);
                #[cfg(feature = "match-logs")]
                {
                    use crate::mid_run_diag::SetPieceDiag;
                    use std::sync::atomic::Ordering;
                    crate::mid_run_diag::CORNERS_AWARDED.fetch_add(1, Ordering::Relaxed);
                    let slot = match chosen_routine {
                        CornerRoutine::NearPost => 0,
                        CornerRoutine::PenaltySpot => 1,
                        CornerRoutine::FarPost => 2,
                        CornerRoutine::Short => 3,
                        CornerRoutine::EdgeCutback => 4,
                    };
                    SetPieceDiag::note_corner(slot, self.pending_corner_delivery);
                }
                self.offside_snapshot = None;
                self.record_touch(taker_id, taker_team, self.current_tick_cached, true);

                events.add_ball_event(BallEvent::Claimed(taker_id));
                // Teleport the taker onto the ball so `move_to`'s
                // distance check doesn't immediately null ownership
                // on the next tick. The ball struct only has a &[MatchPlayer]
                // here — record the teleport and let the engine apply
                // it when it has &mut field.players.
                self.pending_set_piece_teleport = Some((taker_id, self.position));

                // Dead-ball set-up. In real football both sides spend the
                // thirty-to-sixty-second stoppage walking into a shape: the
                // defending side brings ten men back, the attacking side
                // loads the box and leaves a short option by the flag. The
                // sim has no stoppage — the cross is struck 50 ms after the
                // corner is awarded — so the shape has to be placed
                // directly, and until it was, the "corner shape" was
                // whatever open-play positions the twenty-two happened to
                // be standing in when the ball went behind. After a
                // counter that ended with a defender hooking it out, that
                // routinely meant the DEFENDING SIDE HAD NOBODY BUT THE
                // KEEPER IN ITS OWN BOX.
                //
                // `CornerShape::plan` owns the geometry and the
                // who-stands-where; the engine drains the plan in
                // `apply_pending_set_piece_teleport`, which is the layer
                // that has `&mut field.players`.
                let goal_x = match side {
                    GoalSide::Home => 0.0,
                    GoalSide::Away => field_width,
                };
                // Arm the discrete aerial contest for this corner: it fires
                // once, the instant the cross is struck (see engine.rs
                // resolve_corner_contest).
                self.corner_contest_resolved = false;
                self.corner_shape = Some(CornerShapeHold {
                    armed_tick: self.current_tick_cached,
                    taker_id,
                });
                self.pending_corner_teleports = CornerShape::plan(
                    players,
                    attacking_side,
                    taker_id,
                    goal_x,
                    self.position,
                    field_width,
                    field_height,
                );

                return;
            }
            // If no eligible outfielder was found, fall through to goal kick
        }

        // Goal kick: give ball to defending goalkeeper
        if let Some(gk) = players.iter().find(|p| {
            p.side == Some(defending_side) && p.tactical_position.current_position.is_goalkeeper()
        }) {
            let gk_id = gk.id;
            let gk_team = gk.team_id;

            let spot = self.goal_kick_spot(side, goal_center_y, self.position.y);
            self.position = spot;
            self.velocity = Vector3::zeros();

            self.current_owner = Some(gk_id);
            self.previous_owner = None;
            self.ownership_duration = 0;
            self.claim_cooldown = 30;
            self.flags.in_flight_state = 30;
            self.pass_target_player_id = None;
            self.recent_passers.clear();
            // See check_over_goal for full rationale — clear the shot
            // target so the eventual GK clearance can't false-credit a
            // save for a shot that ended out of play.
            self.cached_shot_target = None;
            // Dead ball: drop every open-play window in one place so a
            // pass that was live when the ball went out cannot survive the
            // restart and swallow the restart pass.
            self.clear_open_play_metadata();
            self.pass_origin_restart = PassOriginRestart::GoalKick;
            self.offside_snapshot = None;
            self.record_touch(gk_id, gk_team, self.current_tick_cached, true);

            events.add_ball_event(BallEvent::Claimed(gk_id));
            // Same as corner kick: put the GK onto the ball so the
            // distance check in `move_to` doesn't immediately null
            // ownership because the GK was ~35 units away at the goal
            // line when the ball crossed the end line.
            self.pending_set_piece_teleport = Some((gk_id, self.position));
        }
    }
}
