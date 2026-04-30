use crate::r#match::{MatchPlayerLite, PlayerSide, StateProcessingContext};
use nalgebra::Vector3;

/// Operations for defensive positioning and tactics
pub struct DefensiveOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

/// Role a defender plays relative to the current ball carrier.
///
/// Computed per-tick from geometry (no flag storage), so when the
/// ball carrier moves or the primary presser is dribbled past, roles
/// reassign naturally — the old cover becomes the new primary, the
/// beaten primary drops into help or hold. Every defender computes
/// its own role consistently because the ranking is deterministic
/// (distance to carrier + player-id tiebreak).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefensiveRole {
    /// Closest defender to the ball carrier — engage (press/tackle).
    Primary,
    /// Second-closest — hold a safety position on the line between
    /// ball carrier and own goal, ready to step up if Primary is beaten.
    Cover,
    /// There's a dangerous unmarked non-carrier opponent within reach —
    /// pick them up (close pass lane / mark shadow runner).
    Help,
    /// No immediate individual responsibility — maintain shape.
    Hold,
}

const THREAT_SCAN_DISTANCE: f32 = 100.0;
const DANGEROUS_RUN_SPEED: f32 = 2.0;
const DANGEROUS_RUN_ANGLE: f32 = 0.5;

// Coordination constants
const ENGAGEMENT_DISTANCE: f32 = 20.0; // Distance at which a defender is considered "engaging" an opponent
#[allow(dead_code)]
const MIN_MARKING_SEPARATION: f32 = 15.0; // Minimum distance between defenders marking different opponents

// Role assignment thresholds
const COVER_MAX_DISTANCE: f32 = 45.0; // Too far from carrier to be useful as cover
const HELP_SCAN_RADIUS: f32 = 28.0; // Range in which Help defender looks for a pass option
const COVER_GOAL_SIDE_OFFSET: f32 = 12.0; // Cover sits this far goal-side of carrier

impl<'p> DefensiveOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        DefensiveOperationsImpl { ctx }
    }

    /// Scan for opponents making dangerous runs toward goal
    pub fn scan_for_dangerous_runs(&self) -> Option<MatchPlayerLite> {
        self.scan_for_dangerous_runs_with_distance(THREAT_SCAN_DISTANCE)
    }

    /// Scan for dangerous runs with custom scan distance
    pub fn scan_for_dangerous_runs_with_distance(
        &self,
        scan_distance: f32,
    ) -> Option<MatchPlayerLite> {
        let own_goal_position = self.ctx.ball().direction_to_own_goal();

        // Find the closest opponent making a dangerous run — no intermediate Vec
        self.ctx
            .players()
            .opponents()
            .nearby(scan_distance)
            .filter(|opp| {
                let velocity = opp.velocity(self.ctx);
                let speed = velocity.norm();

                // Must be moving at significant speed
                if speed < DANGEROUS_RUN_SPEED {
                    return false;
                }

                // Check if running toward our goal
                let to_goal = (own_goal_position - opp.position).normalize();
                let velocity_dir = velocity.normalize();
                let alignment = velocity_dir.dot(&to_goal);

                if alignment < DANGEROUS_RUN_ANGLE {
                    return false;
                }

                // Check if in attacking position (closer to our goal than most defenders)
                let defender_x = self.ctx.player.position.x;
                let is_in_dangerous_position =
                    if own_goal_position.x < self.ctx.context.field_size.width as f32 / 2.0 {
                        opp.position.x < defender_x + 20.0 // Attacker is ahead or close
                    } else {
                        opp.position.x > defender_x - 20.0
                    };

                if !is_in_dangerous_position {
                    return false;
                }

                // Skip runners already being engaged by a teammate
                // defender. Without this filter, two defenders both
                // identify the same runner and both mark them,
                // double-teaming while secondary runners go free.
                if self.is_opponent_being_engaged(opp) {
                    return false;
                }

                true
            })
            .min_by(|a, b| {
                let dist_a = a.distance(self.ctx);
                let dist_b = b.distance(self.ctx);
                dist_a.partial_cmp(&dist_b).unwrap()
            })
    }

    /// Find the opponent defensive line position — single-pass, zero allocation
    pub fn find_defensive_line(&self) -> f32 {
        let (sum, count, min_x, max_x) = self
            .ctx
            .players()
            .opponents()
            .all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| p.position.x)
            .fold((0.0f32, 0u32, f32::MAX, f32::MIN), |(s, c, mn, mx), x| {
                (s + x, c + 1, mn.min(x), mx.max(x))
            });

        if count == 0 {
            return self.ctx.context.field_size.width as f32 / 2.0;
        }

        match self.ctx.player.side {
            Some(PlayerSide::Left) => max_x,
            Some(PlayerSide::Right) => min_x,
            None => sum / count as f32,
        }
    }

    /// Find the own team's defensive line position — single-pass, zero allocation
    pub fn find_own_defensive_line(&self) -> f32 {
        let (sum, count) = self
            .ctx
            .players()
            .teammates()
            .defenders()
            .map(|p| p.position.x)
            .fold((0.0f32, 0u32), |(s, c), x| (s + x, c + 1));

        if count == 0 {
            self.ctx.context.field_size.width as f32 / 2.0
        } else {
            sum / count as f32
        }
    }

    /// Check if there's exploitable space behind opponent defense
    pub fn check_space_behind_defense(&self, defensive_line: f32) -> bool {
        let player_x = self.ctx.player.position.x;

        match self.ctx.player.side {
            Some(PlayerSide::Left) => {
                // Space exists if defensive line is high and there's room behind
                defensive_line < self.ctx.context.field_size.width as f32 * 0.7
                    && player_x < defensive_line + 20.0
            }
            Some(PlayerSide::Right) => {
                defensive_line > self.ctx.context.field_size.width as f32 * 0.3
                    && player_x > defensive_line - 20.0
            }
            None => false,
        }
    }

    /// Check if player is the last defender
    pub fn is_last_defender(&self) -> bool {
        self.ctx
            .players()
            .teammates()
            .defenders()
            .all(|d| match self.ctx.player.side {
                Some(PlayerSide::Left) => d.position.x >= self.ctx.player.position.x,
                Some(PlayerSide::Right) => d.position.x <= self.ctx.player.position.x,
                None => false,
            })
    }

    /// Check if should hold defensive line — single-pass, zero allocation
    pub fn should_hold_defensive_line(&self) -> bool {
        let (sum, count) = self
            .ctx
            .players()
            .teammates()
            .defenders()
            .map(|d| d.position.x)
            .fold((0.0f32, 0u32), |(s, c), x| (s + x, c + 1));

        if count == 0 {
            return false;
        }

        let avg_defender_x = sum / count as f32;

        (self.ctx.player.position.x - avg_defender_x).abs() < 5.0
            && self.ctx.ball().distance() > 200.0
            && !self.ctx.team().is_control_ball()
    }

    /// Check if should mark an opponent
    pub fn should_mark_opponent(&self, opponent: &MatchPlayerLite, marking_distance: f32) -> bool {
        let distance = (opponent.position - self.ctx.player.position).magnitude();

        if distance > marking_distance {
            return false;
        }

        // Mark if opponent is in dangerous position
        let opponent_distance_to_goal =
            (opponent.position - self.ctx.ball().direction_to_own_goal()).magnitude();
        let own_distance_to_goal =
            (self.ctx.player.position - self.ctx.ball().direction_to_own_goal()).magnitude();

        // Opponent is closer to our goal than we are
        opponent_distance_to_goal < own_distance_to_goal + 10.0
    }

    /// Get the nearest opponent to mark
    pub fn find_opponent_to_mark(&self, max_distance: f32) -> Option<MatchPlayerLite> {
        self.ctx
            .players()
            .opponents()
            .nearby(max_distance)
            .filter(|opp| self.should_mark_opponent(opp, max_distance))
            .min_by(|a, b| {
                let dist_a = a.distance(self.ctx);
                let dist_b = b.distance(self.ctx);
                dist_a.partial_cmp(&dist_b).unwrap()
            })
    }

    /// True when this attacker is sitting beyond the opposing defensive
    /// line AND their team doesn't have possession — i.e. stranded in an
    /// offside position with nothing to do. Forwards use this to trigger
    /// a drop back; otherwise they camp near the opponent goal, the ball
    /// gets cleared upfield, and every subsequent pass reaches them
    /// offside. Needs an `OFFSIDE_MARGIN` so a forward holding the line
    /// on the shoulder of the last defender isn't forced to retreat
    /// unnecessarily — only steps back when clearly stranded.
    pub fn is_stranded_offside(&self) -> bool {
        if self.ctx.team().is_control_ball() {
            return false;
        }
        const OFFSIDE_MARGIN: f32 = 8.0; // ~1m, the shoulder-of-last-defender tolerance
        let line = self.find_defensive_line();
        let my_x = self.ctx.player.position.x;
        match self.ctx.player.side {
            Some(PlayerSide::Left) => my_x > line + OFFSIDE_MARGIN,
            Some(PlayerSide::Right) => my_x < line - OFFSIDE_MARGIN,
            None => false,
        }
    }

    /// The opposing team has the ball in our defensive third — every
    /// defender, regardless of fatigue/position/current state, must drop
    /// every passive duty (Resting, Returning, Guarding) and engage.
    /// Without this, the passive-state transition logic kept defenders
    /// stuck "returning to position" or "resting until 90% stamina"
    /// while red attackers were literally inside the penalty box.
    pub fn is_defensive_crisis(&self) -> bool {
        if !self.ctx.ball().on_own_side() {
            return false;
        }
        if self.ctx.players().opponents().with_ball().next().is_none() {
            return false;
        }
        let own_goal = self.ctx.ball().direction_to_own_goal();
        let ball_to_goal = (self.ctx.tick_context.positions.ball.position - own_goal).magnitude();
        // Defensive third distance from own goal — third of the field length.
        let third = self.ctx.context.field_size.width as f32 / 3.0;
        ball_to_goal < third
    }

    /// Box emergency — the ball is being carried by an opponent
    /// INSIDE our penalty area. Every defender close enough to make a
    /// difference should abandon their shape-holding duties and engage
    /// immediately. Previously defenders stuck to Cover/Hold roles and
    /// let attackers dribble through the 18-yard box unopposed; real
    /// football: once the ball is in your box, position and coordination
    /// stop mattering — stop the shot NOW.
    ///
    /// Returns `true` if THIS defender should break shape and press the
    /// carrier. Two nearest defenders engage; the rest stay put so we
    /// don't leave the other side of the box open.
    pub fn is_box_emergency_for_me(&self) -> bool {
        if !self.ctx.ball().in_own_penalty_area() {
            return false;
        }
        let Some(carrier) = self.ctx.players().opponents().with_ball().next() else {
            return false;
        };
        let my_id = self.ctx.player.id;
        let my_dist = (self.ctx.player.position - carrier.position).magnitude();
        // Only the two closest defenders engage. The rest hold shape
        // so the box isn't emptied. Rank by distance, tiebreak on id.
        let closer_defenders = self
            .ctx
            .players()
            .teammates()
            .defenders()
            .filter(|d| d.id != my_id)
            .filter(|d| {
                let dist = (d.position - carrier.position).magnitude();
                dist < my_dist || (dist == my_dist && d.id < my_id)
            })
            .count();
        closer_defenders < 2
    }

    /// Attacker is approaching our penalty area and is in our zone —
    /// step out to meet them BEFORE they reach the box. Real football:
    /// a deep block doesn't mean waiting motionless at the edge of the
    /// 6-yard line; defenders hold a line at the 18-yard box and step
    /// to the carrier as they cross some trigger distance.
    pub fn should_step_up_to_meet_attacker(&self) -> bool {
        let Some(carrier) = self.ctx.players().opponents().with_ball().next() else {
            return false;
        };
        let own_goal = self.ctx.ball().direction_to_own_goal();
        let carrier_to_goal = (carrier.position - own_goal).magnitude();
        let field_width = self.ctx.context.field_size.width as f32;
        // Trigger zone: carrier within ~20% of field length from our goal
        // (the edge of our defensive third, approaching the box).
        let trigger = field_width * 0.22;
        if carrier_to_goal > trigger {
            return false;
        }
        // Only the closest defender to the carrier steps up — others hold
        // shape and track secondary runners.
        let my_id = self.ctx.player.id;
        let my_dist = (self.ctx.player.position - carrier.position).magnitude();
        let closer = self
            .ctx
            .players()
            .teammates()
            .defenders()
            .filter(|d| d.id != my_id)
            .any(|d| (d.position - carrier.position).magnitude() < my_dist);
        !closer
    }

    /// Calculate optimal covering position
    pub fn calculate_covering_position(&self) -> Vector3<f32> {
        let goal_position = self.ctx.ball().direction_to_own_goal();
        let ball_position = self.ctx.tick_context.positions.ball.position;

        // Position between ball and goal
        let to_ball = ball_position - goal_position;
        let covering_distance = to_ball.magnitude() * 0.3; // 30% of the way from goal to ball

        goal_position + to_ball.normalize() * covering_distance
    }

    /// Check if in dangerous defensive position (near own goal)
    pub fn in_dangerous_position(&self) -> bool {
        let distance_to_goal =
            (self.ctx.player.position - self.ctx.ball().direction_to_own_goal()).magnitude();
        let danger_threshold = self.ctx.context.field_size.width as f32 * 0.15; // 15% of field width

        distance_to_goal < danger_threshold
    }

    // ==================== DEFENDER COORDINATION ====================

    /// Check if an opponent is already being engaged by a teammate defender
    /// Returns true if another defender is closer AND actively moving toward them
    pub fn is_opponent_being_engaged(&self, opponent: &MatchPlayerLite) -> bool {
        let my_distance = (self.ctx.player.position - opponent.position).magnitude();

        self.ctx
            .players()
            .teammates()
            .defenders()
            .filter(|d| d.id != self.ctx.player.id)
            .any(|teammate| {
                let teammate_distance = (teammate.position - opponent.position).magnitude();

                // Teammate must be within engagement distance
                if teammate_distance > ENGAGEMENT_DISTANCE {
                    return false;
                }

                // Teammate must be closer than us
                if teammate_distance >= my_distance {
                    return false;
                }

                // Teammate must be actively moving toward this opponent (not just standing nearby)
                let teammate_velocity = teammate.velocity(self.ctx);
                let speed = teammate_velocity.norm();

                if speed > 0.5 {
                    let to_opponent = (opponent.position - teammate.position).normalize();
                    let velocity_dir = teammate_velocity.normalize();
                    let alignment = velocity_dir.dot(&to_opponent);

                    // Actively pursuing: moving toward opponent with reasonable alignment
                    alignment > 0.3
                } else {
                    // Stationary but very close — counts as engaged (physically blocking)
                    teammate_distance < 8.0
                }
            })
    }

    /// Assign a defensive role to this defender relative to the current
    /// ball carrier. Single source of truth for defender coordination —
    /// every role decision flows from this. Recomputed each tick, so
    /// role changes with geometry (e.g. primary gets dribbled past →
    /// old cover promotes to primary next tick).
    pub fn defensive_role_for_ball_carrier(&self) -> DefensiveRole {
        let Some(ball_carrier) = self.ctx.players().opponents().with_ball().next() else {
            return DefensiveRole::Hold;
        };

        let my_id = self.ctx.player.id;
        let my_dist = (self.ctx.player.position - ball_carrier.position).magnitude();

        // Rank among defender teammates by distance to ball carrier.
        // Tiebreak on id so every defender agrees on the ranking.
        let rank = self
            .ctx
            .players()
            .teammates()
            .defenders()
            .filter(|d| d.id != my_id)
            .filter(|d| {
                let dist = (d.position - ball_carrier.position).magnitude();
                dist < my_dist || (dist == my_dist && d.id < my_id)
            })
            .count();

        match rank {
            0 => DefensiveRole::Primary,
            1 if my_dist < COVER_MAX_DISTANCE => DefensiveRole::Cover,
            _ => {
                let carrier_id = ball_carrier.id;
                let has_help_target = self
                    .ctx
                    .players()
                    .opponents()
                    .nearby(HELP_SCAN_RADIUS)
                    .any(|opp| opp.id != carrier_id && !self.is_opponent_being_engaged(&opp));
                if has_help_target {
                    DefensiveRole::Help
                } else {
                    DefensiveRole::Hold
                }
            }
        }
    }

    /// Target position for a Cover-role defender: on the line between
    /// ball carrier and own goal, `COVER_GOAL_SIDE_OFFSET` units behind
    /// the carrier. If Primary is beaten, this defender is the next
    /// body between attacker and goal.
    pub fn cover_target_position(&self) -> Option<Vector3<f32>> {
        let ball_carrier = self.ctx.players().opponents().with_ball().next()?;
        let own_goal = self.ctx.ball().direction_to_own_goal();
        let to_goal = own_goal - ball_carrier.position;
        let to_goal_dist = to_goal.magnitude();
        if to_goal_dist < 0.1 {
            return Some(own_goal);
        }
        let dir = to_goal / to_goal_dist;
        Some(ball_carrier.position + dir * COVER_GOAL_SIDE_OFFSET)
    }

    /// Most dangerous non-ball-carrier opponent within Help scan radius
    /// that isn't already being engaged. Target for a Help-role defender.
    pub fn find_help_target(&self) -> Option<MatchPlayerLite> {
        let ball_carrier_id = self.ctx.players().opponents().with_ball().next()?.id;
        let own_goal = self.ctx.ball().direction_to_own_goal();
        self.ctx
            .players()
            .opponents()
            .nearby(HELP_SCAN_RADIUS)
            .filter(|opp| opp.id != ball_carrier_id && !self.is_opponent_being_engaged(opp))
            .max_by(|a, b| {
                let score_a = self.calculate_opponent_danger_score(a, own_goal);
                let score_b = self.calculate_opponent_danger_score(b, own_goal);
                score_a.partial_cmp(&score_b).unwrap()
            })
    }

    /// Find an unmarked dangerous opponent that this defender should cover
    /// Returns the most dangerous opponent not already engaged by a teammate — zero allocation
    pub fn find_unmarked_opponent(&self, max_distance: f32) -> Option<MatchPlayerLite> {
        let own_goal = self.ctx.ball().direction_to_own_goal();

        self.ctx
            .players()
            .opponents()
            .nearby(max_distance)
            .filter(|opp| !self.is_opponent_being_engaged(opp))
            .max_by(|a, b| {
                let score_a = self.calculate_opponent_danger_score(a, own_goal);
                let score_b = self.calculate_opponent_danger_score(b, own_goal);
                score_a.partial_cmp(&score_b).unwrap()
            })
    }

    /// Calculate danger score for an opponent
    fn calculate_opponent_danger_score(
        &self,
        opponent: &MatchPlayerLite,
        own_goal: Vector3<f32>,
    ) -> f32 {
        let mut score = 0.0;

        // Distance to our goal (closer = more dangerous)
        let distance_to_goal = (opponent.position - own_goal).magnitude();
        score += (500.0 - distance_to_goal.min(500.0)) / 5.0;

        // Has the ball
        if opponent.has_ball(self.ctx) {
            score += 150.0;
        }

        // Running toward goal
        let velocity = opponent.velocity(self.ctx);
        if velocity.norm() > 2.0 {
            let to_goal = (own_goal - opponent.position).normalize();
            let alignment = velocity.normalize().dot(&to_goal);
            if alignment > 0.5 {
                score += alignment * 50.0;
            }
        }

        // Close to ball (potential receiver)
        let ball_distance =
            (opponent.position - self.ctx.tick_context.positions.ball.position).magnitude();
        score += (100.0 - ball_distance.min(100.0)) / 2.0;

        score
    }

    /// Check if engaging this opponent would leave a dangerous space uncovered
    pub fn would_leave_space_uncovered(&self, target_opponent: &MatchPlayerLite) -> bool {
        let own_goal = self.ctx.ball().direction_to_own_goal();
        let my_pos = self.ctx.player.position;

        // Find other dangerous opponents near our current position
        let dangerous_nearby = self
            .ctx
            .players()
            .opponents()
            .nearby(40.0)
            .filter(|opp| {
                opp.id != target_opponent.id && {
                    let distance_to_goal = (opp.position - own_goal).magnitude();
                    let my_distance_to_goal = (my_pos - own_goal).magnitude();
                    // Opponent is in a more dangerous position than where we are
                    distance_to_goal < my_distance_to_goal
                }
            })
            .count();

        // If there are multiple dangerous opponents nearby and no other defender covering
        if dangerous_nearby >= 2 {
            // Check if other defenders are available to cover
            let covering_defenders = self
                .ctx
                .players()
                .teammates()
                .defenders()
                .filter(|d| d.id != self.ctx.player.id && (d.position - my_pos).magnitude() < 50.0)
                .count();

            return covering_defenders < dangerous_nearby;
        }

        false
    }

    /// Get the number of defenders currently engaging opponents within a distance
    pub fn count_engaging_defenders(&self, radius: f32) -> usize {
        self.ctx
            .players()
            .teammates()
            .defenders()
            .filter(|d| {
                d.id != self.ctx.player.id && {
                    // Check if this defender is close to any opponent
                    self.ctx
                        .players()
                        .opponents()
                        .nearby(ENGAGEMENT_DISTANCE)
                        .any(|opp| (d.position - opp.position).magnitude() < radius)
                }
            })
            .count()
    }
}
