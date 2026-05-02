//! Out-of-play resolution: actual goals, over-the-bar goal kicks,
//! and wide-of-goal corner / goal kick decisions. The wide-of-goal
//! flow stages the set-piece teleport via `pending_set_piece_teleport`
//! since the ball can't move other players' positions itself.

use super::Ball;
use crate::r#match::ball::events::{BallEvent, BallGoalEventMetadata, GoalSide};
use crate::r#match::engine::goal::GOAL_WIDTH;
use crate::r#match::events::EventCollection;
use crate::r#match::{MatchContext, MatchPlayer, PlayerSide};
use nalgebra::Vector3;

impl Ball {
    pub(super) fn check_goal(&mut self, context: &MatchContext, result: &mut EventCollection) {
        // Guard: don't detect another goal if one was already scored this tick
        if self.goal_scored {
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
                    if !recent_shot && !shot_in_flight {
                        // Not a shot — treat as ball out of play, not a goal.
                        return;
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
                                .find(|&&id| id != goalscorer)
                                .copied()
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

                // Find assist provider: most recent passer who isn't the goalscorer
                let assist_player_id = if !final_is_auto_goal {
                    self.recent_passers
                        .iter()
                        .rev()
                        .find(|&&id| id != final_scorer)
                        .copied()
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
            }

            // Determine which side should kick off (the conceding team)
            // Home goal (x=0) = Left side conceded → Left kicks off
            // Away goal (x=field_width) = Right side conceded → Right kicks off
            self.kickoff_team_side = match goal_side {
                GoalSide::Home => Some(PlayerSide::Left),
                GoalSide::Away => Some(PlayerSide::Right),
            };

            self.goal_scored = true;
            self.reset();
        }
    }

    /// Ball crossed goal line within goal width but above crossbar — goal kick.
    /// Place ball near the 6-yard box and give it to the defending goalkeeper.
    pub(super) fn check_over_goal(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        let over_side = match context.goal_positions.is_over_goal(self.position) {
            Some(side) => side,
            None => return,
        };

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
            // Place ball at the 6-yard area in front of the goal
            let goal_kick_x = match over_side {
                GoalSide::Home => 50.0, // ~6 yards from left goal line
                GoalSide::Away => self.field_width - 50.0,
            };

            self.position.x = goal_kick_x;
            self.position.y = context.goal_positions.left.y; // Center of goal
            self.position.z = 0.0;
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
            self.pass_origin_restart = crate::r#match::PassOriginRestart::GoalKick;
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
        let field_width = context.field_size.width as f32;
        let goal_half_width = GOAL_WIDTH;

        // Check left endline
        let crossed_side = if self.position.x <= 0.0 {
            let goal_center_y = context.goal_positions.left.y;
            // Only trigger if OUTSIDE the goal posts (inside is handled by check_goal/check_over_goal)
            if self.position.y < goal_center_y - goal_half_width
                || self.position.y > goal_center_y + goal_half_width
            {
                Some(GoalSide::Home)
            } else {
                None
            }
        } else if self.position.x >= field_width {
            let goal_center_y = context.goal_positions.right.y;
            if self.position.y < goal_center_y - goal_half_width
                || self.position.y > goal_center_y + goal_half_width
            {
                Some(GoalSide::Away)
            } else {
                None
            }
        } else {
            None
        };

        let side = match crossed_side {
            Some(s) => s,
            None => return,
        };

        let defending_side = match side {
            GoalSide::Home => PlayerSide::Left,
            GoalSide::Away => PlayerSide::Right,
        };
        let attacking_side = match defending_side {
            PlayerSide::Left => PlayerSide::Right,
            PlayerSide::Right => PlayerSide::Left,
        };

        // Decide corner vs goal kick from the last player who touched the
        // ball. If the defending team put it out, it's a corner for the
        // attacking team. Unknown last-touch defaults to goal kick.
        let last_toucher_side: Option<PlayerSide> = self
            .previous_owner
            .or(self.current_owner)
            .and_then(|pid| players.iter().find(|p| p.id == pid))
            .and_then(|p| p.side);

        let is_corner = last_toucher_side == Some(defending_side);

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

            // Find the attacking team's designated corner taker — score by
            // (crossing, technique, corners) like SetPieceSetup::choose, but
            // restricted to players currently on the pitch.
            let taker = players
                .iter()
                .filter(|p| {
                    p.side == Some(attacking_side)
                        && !p.tactical_position.current_position.is_goalkeeper()
                })
                .max_by(|a, b| {
                    let sa = a.skills.technical.crossing * 0.6
                        + a.skills.technical.technique * 0.3
                        + a.skills.technical.corners * 0.1;
                    let sb = b.skills.technical.crossing * 0.6
                        + b.skills.technical.technique * 0.3
                        + b.skills.technical.corners * 0.1;
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
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
                self.pass_origin_restart = crate::r#match::PassOriginRestart::Corner;
                self.offside_snapshot = None;
                self.record_touch(taker_id, taker_team, self.current_tick_cached, true);

                events.add_ball_event(BallEvent::Claimed(taker_id));
                // Teleport the taker onto the ball so `move_to`'s
                // distance check doesn't immediately null ownership
                // on the next tick. The ball struct only has a &[MatchPlayer]
                // here — record the teleport and let the engine apply
                // it when it has &mut field.players.
                self.pending_set_piece_teleport = Some((taker_id, self.position));
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
            let goal_kick_x = match side {
                GoalSide::Home => 50.0,
                GoalSide::Away => field_width - 50.0,
            };

            self.position.x = goal_kick_x;
            self.position.y = context.goal_positions.left.y;
            self.position.z = 0.0;
            self.velocity = Vector3::zeros();

            self.current_owner = Some(gk_id);
            self.previous_owner = None;
            self.ownership_duration = 0;
            self.claim_cooldown = 30;
            self.flags.in_flight_state = 30;
            self.pass_target_player_id = None;
            self.recent_passers.clear();
            self.pass_origin_restart = crate::r#match::PassOriginRestart::GoalKick;
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
