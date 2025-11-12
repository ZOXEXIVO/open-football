use crate::r#match::{MatchPlayerLite, PlayerSide, StateProcessingContext};
use crate::PlayerFieldPositionGroup;
use nalgebra::Vector3;

/// Operations for defensive positioning and tactics
pub struct DefensiveOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

const THREAT_SCAN_DISTANCE: f32 = 70.0;
const DANGEROUS_RUN_SPEED: f32 = 3.0;
const DANGEROUS_RUN_ANGLE: f32 = 0.7;

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
}
