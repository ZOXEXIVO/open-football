use crate::r#match::{MatchPlayerLite, PlayerSide, StateProcessingContext};
use nalgebra::Vector3;

/// Operations for defensive positioning and tactics
pub struct DefensiveOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

const THREAT_SCAN_DISTANCE: f32 = 70.0;
const DANGEROUS_RUN_SPEED: f32 = 3.0;
const DANGEROUS_RUN_ANGLE: f32 = 0.7;

// Coordination constants
const ENGAGEMENT_DISTANCE: f32 = 25.0; // Distance at which a defender is considered "engaging" an opponent
const MIN_MARKING_SEPARATION: f32 = 15.0; // Minimum distance between defenders marking different opponents

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

        // Find opponents making dangerous runs within extended range
        let dangerous_runners: Vec<MatchPlayerLite> = self
            .ctx
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
                let is_in_dangerous_position = if own_goal_position.x
                    < self.ctx.context.field_size.width as f32 / 2.0
                {
                    opp.position.x < defender_x + 20.0 // Attacker is ahead or close
                } else {
                    opp.position.x > defender_x - 20.0
                };

                alignment >= DANGEROUS_RUN_ANGLE && is_in_dangerous_position
            })
            .collect();

        if dangerous_runners.is_empty() {
            return None;
        }

        // Return the closest dangerous runner
        dangerous_runners
            .iter()
            .min_by(|a, b| {
                let dist_a = a.distance(self.ctx);
                let dist_b = b.distance(self.ctx);
                dist_a.partial_cmp(&dist_b).unwrap()
            })
            .copied()
    }

    /// Find the opponent defensive line position
    pub fn find_defensive_line(&self) -> f32 {
        let defenders: Vec<f32> = self
            .ctx
            .players()
            .opponents()
            .all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| match self.ctx.player.side {
                Some(PlayerSide::Left) => p.position.x,
                Some(PlayerSide::Right) => p.position.x,
                None => p.position.x,
            })
            .collect();

        if defenders.is_empty() {
            self.ctx.context.field_size.width as f32 / 2.0
        } else {
            // Return the position of the last defender
            match self.ctx.player.side {
                Some(PlayerSide::Left) => defenders.iter().fold(f32::MIN, |a, &b| a.max(b)),
                Some(PlayerSide::Right) => defenders.iter().fold(f32::MAX, |a, &b| a.min(b)),
                None => defenders.iter().sum::<f32>() / defenders.len() as f32,
            }
        }
    }

    /// Find the own team's defensive line position
    pub fn find_own_defensive_line(&self) -> f32 {
        let defenders: Vec<f32> = self
            .ctx
            .players()
            .teammates()
            .defenders()
            .map(|p| match self.ctx.player.side {
                Some(PlayerSide::Left) => p.position.x,
                Some(PlayerSide::Right) => p.position.x,
                None => p.position.x,
            })
            .collect();

        if defenders.is_empty() {
            self.ctx.context.field_size.width as f32 / 2.0
        } else {
            defenders.iter().sum::<f32>() / defenders.len() as f32
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

    /// Check if should hold defensive line
    pub fn should_hold_defensive_line(&self) -> bool {
        let ball_ops = self.ctx.ball();

        let defenders: Vec<MatchPlayerLite> =
            self.ctx.players().teammates().defenders().collect();

        if defenders.is_empty() {
            return false;
        }

        let avg_defender_x =
            defenders.iter().map(|d| d.position.x).sum::<f32>() / defenders.len() as f32;

        (self.ctx.player.position.x - avg_defender_x).abs() < 5.0
            && ball_ops.distance() > 200.0
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
        let distance_to_goal = (self.ctx.player.position - self.ctx.ball().direction_to_own_goal())
            .magnitude();
        let danger_threshold = self.ctx.context.field_size.width as f32 * 0.15; // 15% of field width

        distance_to_goal < danger_threshold
    }

    // ==================== DEFENDER COORDINATION ====================

    /// Check if an opponent is already being engaged by a teammate defender
    /// Returns true if another defender is closer to the opponent and moving toward them
    pub fn is_opponent_being_engaged(&self, opponent: &MatchPlayerLite) -> bool {
        let my_distance = (self.ctx.player.position - opponent.position).magnitude();

        self.ctx
            .players()
            .teammates()
            .defenders()
            .filter(|d| d.id != self.ctx.player.id)
            .any(|teammate| {
                let teammate_distance = (teammate.position - opponent.position).magnitude();

                // Teammate is closer and within engagement distance
                if teammate_distance < my_distance && teammate_distance < ENGAGEMENT_DISTANCE {
                    return true;
                }

                // Teammate is moving toward this opponent
                let teammate_velocity = teammate.velocity(self.ctx);
                if teammate_velocity.norm() > 1.0 {
                    let to_opponent = (opponent.position - teammate.position).normalize();
                    let velocity_dir = teammate_velocity.normalize();
                    let alignment = velocity_dir.dot(&to_opponent);

                    // Teammate is actively pursuing this opponent
                    if alignment > 0.7 && teammate_distance < my_distance * 1.2 {
                        return true;
                    }
                }

                false
            })
    }

    /// Check if this defender is the best positioned to engage a specific opponent
    /// Considers distance, angle to goal coverage, and whether others are already engaged
    pub fn is_best_defender_for_opponent(&self, opponent: &MatchPlayerLite) -> bool {
        let my_distance = (self.ctx.player.position - opponent.position).magnitude();
        let own_goal = self.ctx.ball().direction_to_own_goal();

        // Calculate how well positioned we are to cut off the goal angle
        let my_goal_coverage = self.calculate_goal_coverage_score(opponent, own_goal);

        // Check all other defenders
        for teammate in self.ctx.players().teammates().defenders() {
            if teammate.id == self.ctx.player.id {
                continue;
            }

            let teammate_distance = (teammate.position - opponent.position).magnitude();
            let teammate_goal_coverage = {
                let to_opponent = opponent.position - teammate.position;
                let opponent_to_goal = own_goal - opponent.position;
                let alignment = to_opponent.normalize().dot(&opponent_to_goal.normalize());
                alignment.max(0.0) * 0.3 + (1.0 / (teammate_distance + 1.0)) * 0.7
            };

            // Significant advantage threshold - 20% better positioning
            if teammate_distance < my_distance * 0.8 {
                return false;
            }

            // Teammate has much better goal coverage
            if teammate_goal_coverage > my_goal_coverage * 1.3 && teammate_distance < my_distance * 1.1 {
                return false;
            }
        }

        true
    }

    /// Calculate how well positioned a defender is to cover the goal against an opponent
    fn calculate_goal_coverage_score(&self, opponent: &MatchPlayerLite, goal: Vector3<f32>) -> f32 {
        let to_opponent = opponent.position - self.ctx.player.position;
        let opponent_to_goal = goal - opponent.position;
        let distance = to_opponent.magnitude();

        // How well aligned we are between opponent and goal
        let alignment = to_opponent.normalize().dot(&opponent_to_goal.normalize());
        let alignment_score = alignment.max(0.0);

        // Closer is better
        let distance_score = 1.0 / (distance + 1.0);

        alignment_score * 0.3 + distance_score * 0.7
    }

    /// Find an unmarked dangerous opponent that this defender should cover
    /// Returns the best opponent to mark that isn't already being engaged by a teammate
    pub fn find_unmarked_opponent(&self, max_distance: f32) -> Option<MatchPlayerLite> {
        let own_goal = self.ctx.ball().direction_to_own_goal();

        let mut candidates: Vec<(MatchPlayerLite, f32)> = self
            .ctx
            .players()
            .opponents()
            .nearby(max_distance)
            .filter(|opp| !self.is_opponent_being_engaged(opp))
            .map(|opp| {
                let danger_score = self.calculate_opponent_danger_score(&opp, own_goal);
                (opp, danger_score)
            })
            .collect();

        // Sort by danger score descending
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // Return the most dangerous unmarked opponent
        candidates.into_iter().next().map(|(opp, _)| opp)
    }

    /// Calculate danger score for an opponent
    fn calculate_opponent_danger_score(&self, opponent: &MatchPlayerLite, own_goal: Vector3<f32>) -> f32 {
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
        let ball_distance = (opponent.position - self.ctx.tick_context.positions.ball.position).magnitude();
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
                .filter(|d| {
                    d.id != self.ctx.player.id
                        && (d.position - my_pos).magnitude() < 50.0
                })
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
