use crate::r#match::{CoachInstruction, GamePhase, MatchCoach, PlayerSide, StateProcessingContext, TeamTacticalState};
use crate::{PlayerFieldPositionGroup, Tactics};
use nalgebra::Vector3;

pub struct TeamOperationsImpl<'b> {
    ctx: &'b StateProcessingContext<'b>,
}

impl<'b> TeamOperationsImpl<'b> {
    pub fn new(ctx: &'b StateProcessingContext<'b>) -> Self {
        TeamOperationsImpl { ctx }
    }
}

impl<'b> TeamOperationsImpl<'b> {
    pub fn tactics(&self) -> &Tactics {
        match self.ctx.player.side {
            Some(PlayerSide::Left) => &self.ctx.context.tactics.left,
            Some(PlayerSide::Right) => &self.ctx.context.tactics.right,
            None => {
                panic!("unknown player side")
            }
        }
    }

    /// Get the coach's current instruction for this player's team
    pub fn coach_instruction(&self) -> CoachInstruction {
        self.coach().instruction
    }

    /// Get the coach state for this player's team
    pub fn coach(&self) -> &MatchCoach {
        self.ctx.context.coach_for_team(self.ctx.player.team_id)
    }

    /// Team-level tactical state for this player's side (phase,
    /// possession timers, defensive line height). All eleven players on
    /// a team read the same value — that's the point, it's the shared
    /// context that pulls their individual decisions into a coherent
    /// pattern. Recomputed every 10 sim ticks.
    pub fn tactical(&self) -> &TeamTacticalState {
        self.ctx.context.tactical_for_team(self.ctx.player.team_id)
    }

    /// Shortcut — most branching just needs the phase.
    pub fn phase(&self) -> GamePhase {
        self.tactical().phase
    }

    /// Is the attack ready to progress? True when at least one of our
    /// forwards is in a genuine scoring threat — close to goal, in space,
    /// and facing a clear lane. When FALSE, defenders and midfielders
    /// should hold possession and recycle rather than force a forward
    /// pass: there's nobody to pass TO whose reception would lead to a
    /// shot, so a risky forward ball will just turn over possession and
    /// become the opponent's next attack.
    pub fn is_attack_ready(&self) -> bool {
        let ctx = self.ctx;
        let goal_pos = ctx.player().opponent_goal_position();
        // Scan our forwards / attacking midfielders — any one of them
        // positioned within ~35m of goal in open space means the attack
        // is a legitimate threat.
        ctx.players().teammates().all()
            .filter(|t| {
                t.tactical_positions.is_forward()
                    || t.tactical_positions.is_midfielder()
            })
            .any(|t| {
                let to_goal = (t.position - goal_pos).magnitude();
                if to_goal > 70.0 {
                    return false;
                }
                // Check space around the forward — an opponent within
                // 8u means they're marked and not a live shooting threat.
                let marked_closely = ctx.tick_context.grid
                    .opponents(t.id, 8.0).count() > 0;
                !marked_closely
            })
    }

    /// Should the team play in "possession mode" right now — i.e. slow
    /// down, recycle the ball, reject risky forward passes? Real football
    /// triggers:
    ///   1. **Just won the ball.** Stabilize for a few seconds before
    ///      looking to attack — avoids the rushed long-ball turnover
    ///      that re-loses possession within a breath of winning it.
    ///   2. **Leading in the match.** Already captured by
    ///      `prefer_possession()` on the coach (WasteTime / ParkTheBus
    ///      / SlowDown); we union it here so one helper covers all
    ///      realistic triggers.
    ///   3. **Team is tired.** A fatigued squad keeps the ball to rest.
    ///   4. **Attack not ready.** No forward in a shooting threat — no
    ///      point pushing forward into nothing.
    ///   5. **Late in a competitive match.** Final 10 minutes, any
    ///      team ahead or drawing plays safer than earlier.
    pub fn should_play_possession(&self) -> bool {
        let ctx = self.ctx;
        let coach = self.coach_instruction();
        if coach.prefer_possession() {
            return true;
        }

        // (1) Just won the ball — stabilize window. The coach tracks
        // `last_possession_gain_tick`; for the first ~300 ticks (3 s)
        // after winning, keep possession rather than counter-rushing.
        let current_tick = ctx.context.current_tick();
        let ticks_since_gain = current_tick.saturating_sub(self.coach().last_possession_gain_tick);
        if ticks_since_gain < 300 && ctx.team().is_control_ball() {
            return true;
        }

        // (3) Team tired — use average condition of outfielders within
        // 250u of the ball (cheap proxy for "our involved players").
        let mut total = 0u32;
        let mut count = 0u32;
        for t in ctx.players().teammates().nearby(250.0) {
            if let Some(p) = ctx.context.players.by_id(t.id) {
                total += p.player_attributes.condition_percentage();
                count += 1;
            }
        }
        if count > 0 && (total / count) < 50 {
            return true;
        }

        // (5) Late game + drawing or leading — even Normal-instruction
        // teams slow down in the last 10 min when they don't need goals.
        let half_ms = crate::r#match::engine::engine::MATCH_HALF_TIME_MS as f32;
        let full_ms = half_ms * 2.0;
        let match_progress = (ctx.context.total_match_time as f32 / full_ms).clamp(0.0, 1.0);
        if match_progress > 0.88 && self.score_diff() >= 0 {
            return true;
        }

        // (4) Attack not set up downfield.
        !self.is_attack_ready()
    }

    /// Game-management intensity from this team's perspective. Rises
    /// when we are leading, late in the game, and/or the weaker side —
    /// drives safe-pass / hold-the-ball / don't-shoot behaviour.
    pub fn game_management_intensity(&self) -> f32 {
        self.tactical().game_management_intensity
    }

    /// Whether the team's coach allows shooting right now (team-level cooldown)
    pub fn can_shoot(&self) -> bool {
        let current_tick = self.ctx.context.current_tick();
        self.coach().can_shoot(current_tick)
    }

    /// Score difference from this team's perspective (positive = leading)
    pub fn score_diff(&self) -> i8 {
        let home_goals = self.ctx.context.score.home_team.get() as i8;
        let away_goals = self.ctx.context.score.away_team.get() as i8;
        if self.ctx.player.team_id == self.ctx.context.field_home_team_id {
            home_goals - away_goals
        } else {
            away_goals - home_goals
        }
    }

    pub fn is_control_ball(&self) -> bool {
        let current_player_team_id = self.ctx.player.team_id;

        // First check: if a player from player's team has the ball
        if let Some(owner_id) = self.ctx.ball().owner_id() {
            if let Some(ball_owner) = self.ctx.context.players.by_id(owner_id) {
                return ball_owner.team_id == current_player_team_id;
            }
        }

        // Second check: if previous owner was from player's team
        if let Some(prev_owner_id) = self.ctx.ball().previous_owner_id() {
            if let Some(prev_ball_owner) = self.ctx.context.players.by_id(prev_owner_id) {
                if prev_ball_owner.team_id == current_player_team_id {
                    // Check if the ball is still heading in a favorable direction for the team
                    // or if a teammate is clearly going to get it
                    let ball_velocity = self.ctx.tick_context.positions.ball.velocity;

                    // If ball has significant velocity and is heading toward opponent's goal
                    if ball_velocity.norm_squared() > 1.0 {
                        // Determine which way is "forward" based on team side
                        let forward_direction = match self.ctx.player.side {
                            Some(PlayerSide::Left) => Vector3::new(1.0, 0.0, 0.0),  // Left team attacks right
                            Some(PlayerSide::Right) => Vector3::new(-1.0, 0.0, 0.0), // Right team attacks left
                            None => Vector3::new(0.0, 0.0, 0.0),
                        };

                        // If ball is moving forward or toward a teammate
                        let dot_product = ball_velocity.normalize().dot(&forward_direction);
                        if dot_product > 0.1 {
                            return true;
                        }

                        // If a teammate is clearly going for the ball and is close
                        if self.is_teammate_chasing_ball() {
                            return true;
                        }
                    }
                }
            }
        }

        // If we get here, we need to check if any player from our team
        // is closer to the ball than any opponent
        let ball_pos = self.ctx.tick_context.positions.ball.position;

        let closest_teammate_dist_sq = self.ctx.players().teammates().all()
            .map(|p| (p.position - ball_pos).norm_squared())
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let closest_opponent_dist_sq = self.ctx.players().opponents().all()
            .map(|p| (p.position - ball_pos).norm_squared())
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // If a teammate is significantly closer to the ball than any opponent
        // 0.7 distance ratio = 0.49 squared ratio
        if let (Some(team_sq), Some(opp_sq)) = (closest_teammate_dist_sq, closest_opponent_dist_sq) {
            if team_sq < opp_sq * 0.49 {
                return true;
            }
        }

        false
    }

    /// Check if team has JUST lost possession (previous owner was teammate, ball now with opponent).
    /// Used for counter-pressing triggers — defenders immediately press instead of retreating.
    pub fn has_just_lost_possession(&self) -> bool {
        // Already have ball — haven't lost it
        if self.is_control_ball() {
            return false;
        }

        // Check if previous owner was from our team
        if let Some(prev_id) = self.ctx.tick_context.ball.last_owner {
            if let Some(prev_player) = self.ctx.context.players.by_id(prev_id) {
                if prev_player.team_id == self.ctx.player.team_id {
                    // Previous owner was us, now we don't control ball
                    // Only counts as "just lost" if within ~300ms
                    let new_ownership = self.ctx.tick_context.ball.ownership_duration;
                    return new_ownership < 30;
                }
            }
        }

        false
    }

    pub fn is_leading(&self) -> bool {
        !self.is_loosing()
    }

    pub fn is_loosing(&self) -> bool {
        if self.ctx.player.team_id == self.ctx.context.score.home_team.team_id {
            self.ctx.context.score.home_team < self.ctx.context.score.away_team
        } else {
            self.ctx.context.score.away_team < self.ctx.context.score.home_team
        }
    }

    pub fn is_teammate_chasing_ball(&self) -> bool {
        let ball_position = self.ctx.tick_context.positions.ball.position;

        self.ctx
            .players()
            .teammates()
            .all()
            .any(|player| {
                // Check if player is heading toward the ball
                let player_position = self.ctx.tick_context.positions.players.position(player.id);
                let player_velocity = self.ctx.tick_context.positions.players.velocity(player.id);

                if player_velocity.norm_squared() < 0.01 {
                    return false;
                }

                let direction_to_ball = (ball_position - player_position).normalize();
                let player_direction = player_velocity.normalize();
                let dot_product = direction_to_ball.dot(&player_direction);

                // Player is moving toward the ball (use norm_squared for distance comparison)
                dot_product > 0.85 &&
                    (ball_position - player_position).norm_squared() <
                        (ball_position - self.ctx.player.position).norm_squared() * 1.44 // 1.2^2
            })
    }

    // Determine if this player is the best positioned to chase the ball
    pub fn is_best_player_to_chase_ball(&self) -> bool {
        let ball_position = self.ctx.tick_context.positions.ball.position;

        // Don't chase the ball if a teammate already has it
        if let Some(owner_id) = self.ctx.ball().owner_id() {
            if let Some(owner) = self.ctx.context.players.by_id(owner_id) {
                if owner.team_id == self.ctx.player.team_id {
                    return false;
                }
            }
        }

        // Score for current player (use norm_squared to avoid sqrt)
        let player_dist_sq = (ball_position - self.ctx.player.position).norm_squared();
        let player_score = {
            let skills = &self.ctx.player.skills;
            let pace_factor = skills.physical.pace / 20.0;
            let acceleration_factor = skills.physical.acceleration / 20.0;
            let position_factor = match self.ctx.player.tactical_position.current_position.position_group() {
                PlayerFieldPositionGroup::Forward => 1.2,
                PlayerFieldPositionGroup::Midfielder => 1.1,
                PlayerFieldPositionGroup::Defender => 0.9,
                PlayerFieldPositionGroup::Goalkeeper => 0.5,
            };
            let ability = pace_factor * acceleration_factor * position_factor * 0.5 + 0.5;
            player_dist_sq / (ability * ability)
        };

        let threshold = player_score * 0.64; // 0.8^2

        // Compare against teammates
        !self.ctx
            .players()
            .teammates()
            .all()
            .any(|teammate| {
                let dist_sq = (ball_position - teammate.position).norm_squared();
                // Quick distance check
                if dist_sq > player_dist_sq {
                    return false;
                }

                let skills = match self.ctx.context.players.by_id(teammate.id) {
                    Some(p) => &p.skills,
                    None => return false,
                };

                let pace_factor = skills.physical.pace / 20.0;
                let acceleration_factor = skills.physical.acceleration / 20.0;
                let position_factor = match teammate.tactical_positions.position_group() {
                    PlayerFieldPositionGroup::Forward => 1.2,
                    PlayerFieldPositionGroup::Midfielder => 1.1,
                    PlayerFieldPositionGroup::Defender => 0.9,
                    PlayerFieldPositionGroup::Goalkeeper => 0.5,
                };

                let ability = pace_factor * acceleration_factor * position_factor * 0.5 + 0.5;
                let score = dist_sq / (ability * ability);
                score < threshold
            })
    }
}
