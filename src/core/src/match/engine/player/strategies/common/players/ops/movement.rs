use crate::r#match::{MatchPlayerLite, StateProcessingContext};
use nalgebra::Vector3;

/// Operations for movement and space-finding
pub struct MovementOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

impl<'p> MovementOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        MovementOperationsImpl { ctx }
    }

    /// Find space to dribble into
    pub fn find_dribbling_space(&self) -> Option<Vector3<f32>> {
        let player_pos = self.ctx.player.position;
        let goal_direction = (self.ctx.player().opponent_goal_position() - player_pos).normalize();

        // Check multiple angles for space
        let angles = [-45.0f32, -30.0, 0.0, 30.0, 45.0];

        for angle_deg in angles.iter() {
            let angle_rad = angle_deg.to_radians();
            let cos_a = angle_rad.cos();
            let sin_a = angle_rad.sin();

            // Rotate goal direction by angle
            let check_direction = Vector3::new(
                goal_direction.x * cos_a - goal_direction.y * sin_a,
                goal_direction.x * sin_a + goal_direction.y * cos_a,
                0.0,
            );

            let check_position = player_pos + check_direction * 15.0;

            // Check if this direction is clear
            let opponents_in_path = self
                .ctx
                .players()
                .opponents()
                .all()
                .filter(|opp| {
                    let to_opp = opp.position - player_pos;
                    let dist = to_opp.magnitude();
                    let dot = to_opp.normalize().dot(&check_direction);

                    dist < 20.0 && dot > 0.7
                })
                .count();

            if opponents_in_path == 0 {
                return Some(check_position);
            }
        }

        None
    }

    /// Find the best gap in opponent defense
    pub fn find_best_gap_in_defense(&self) -> Option<Vector3<f32>> {
        let player_pos = self.ctx.player.position;
        let goal_pos = self.ctx.player().opponent_goal_position();

        let opponents: Vec<MatchPlayerLite> = self
            .ctx
            .players()
            .opponents()
            .nearby(100.0)
            .filter(|opp| {
                // Only consider opponents between player and goal
                let to_goal = goal_pos - player_pos;
                let to_opp = opp.position - player_pos;
                to_goal.normalize().dot(&to_opp.normalize()) > 0.5
            })
            .collect();

        if opponents.len() < 2 {
            return None;
        }

        // Find largest gap
        let mut best_gap = None;
        let mut best_gap_size = 0.0;

        for i in 0..opponents.len() {
            for j in i + 1..opponents.len() {
                let gap_center = (opponents[i].position + opponents[j].position) * 0.5;
                let gap_size = (opponents[i].position - opponents[j].position).magnitude();

                if gap_size > best_gap_size && gap_size > 20.0 {
                    best_gap_size = gap_size;
                    best_gap = Some(gap_center);
                }
            }
        }

        best_gap
    }

    /// Find optimal attacking path considering opponents
    pub fn find_optimal_attacking_path(&self) -> Option<Vector3<f32>> {
        let player_pos = self.ctx.player.position;
        let goal_pos = self.ctx.player().opponent_goal_position();

        // Look for gaps in defense
        if let Some(gap) = self.find_best_gap_in_defense() {
            return Some(gap);
        }

        // Try to move toward goal while avoiding opponents
        let to_goal = goal_pos - player_pos;
        let goal_direction = to_goal.normalize();

        // Check if direct path is clear
        if !self
            .ctx
            .players()
            .opponents()
            .nearby(30.0)
            .any(|opp| {
                let to_opp = opp.position - player_pos;
                let dot = to_opp.normalize().dot(&goal_direction);
                dot > 0.8 && to_opp.magnitude() < 40.0
            })
        {
            return Some(player_pos + goal_direction * 50.0);
        }

        None
    }

    /// Calculate support run position for a ball holder
    pub fn calculate_support_run_position(&self, holder_pos: Vector3<f32>) -> Vector3<f32> {
        let player_pos = self.ctx.player.position;
        let field_height = self.ctx.context.field_size.height as f32;

        // Determine player's role based on position
        let is_central = (player_pos.y - field_height / 2.0).abs() < field_height * 0.2;

        if is_central {
            self.calculate_central_support_position(holder_pos)
        } else {
            self.calculate_wide_support_position(holder_pos)
        }
    }

    /// Calculate wide support position (for wingers)
    fn calculate_wide_support_position(&self, holder_pos: Vector3<f32>) -> Vector3<f32> {
        let player_pos = self.ctx.player.position;
        let field_height = self.ctx.context.field_size.height as f32;

        // Stay wide and ahead of ball
        let target_y = if player_pos.y < field_height / 2.0 {
            field_height * 0.1 // Left wing
        } else {
            field_height * 0.9 // Right wing
        };

        // Stay ahead of ball carrier (increased distance to prevent clustering)
        let target_x = match self.ctx.player.side {
            Some(crate::r#match::PlayerSide::Left) => holder_pos.x + 80.0,
            Some(crate::r#match::PlayerSide::Right) => holder_pos.x - 80.0,
            None => holder_pos.x,
        };

        Vector3::new(target_x, target_y, 0.0)
    }

    /// Calculate central support position (for central players)
    fn calculate_central_support_position(&self, holder_pos: Vector3<f32>) -> Vector3<f32> {
        let field_height = self.ctx.context.field_size.height as f32;
        let player_pos = self.ctx.player.position;

        // Move into space between defenders (increased distance)
        let target_x = match self.ctx.player.side {
            Some(crate::r#match::PlayerSide::Left) => holder_pos.x + 90.0,
            Some(crate::r#match::PlayerSide::Right) => holder_pos.x - 90.0,
            None => holder_pos.x,
        };

        // Gentle pull toward center
        let center_pull = (field_height / 2.0 - player_pos.y) * 0.2;
        let target_y = player_pos.y + center_pull;

        Vector3::new(
            target_x,
            target_y.clamp(field_height * 0.3, field_height * 0.7),
            0.0,
        )
    }

    /// Check if player is congested near boundary
    pub fn is_congested_near_boundary(&self) -> bool {
        let field_width = self.ctx.context.field_size.width as f32;
        let field_height = self.ctx.context.field_size.height as f32;
        let pos = self.ctx.player.position;

        let near_boundary = pos.x < 20.0
            || pos.x > field_width - 20.0
            || pos.y < 20.0
            || pos.y > field_height - 20.0;

        if !near_boundary {
            return false;
        }

        // Count all nearby players (teammates + opponents)
        let nearby_teammates = self.ctx.players().teammates().nearby(15.0).count();
        let nearby_opponents = self.ctx.players().opponents().nearby(15.0).count();
        let total_nearby = nearby_teammates + nearby_opponents;

        // If 3 or more players nearby (congestion)
        total_nearby >= 3
    }

    /// Calculate better passing position to escape pressure
    pub fn calculate_better_passing_position(&self) -> Option<Vector3<f32>> {
        let player_pos = self.ctx.player.position;
        let goal_pos = self.ctx.ball().direction_to_own_goal();

        // Find positions of nearby opponents creating pressure
        let nearby_opponents: Vec<MatchPlayerLite> =
            self.ctx.players().opponents().nearby(15.0).collect();

        if nearby_opponents.is_empty() {
            return None;
        }

        // Calculate average position of pressing opponents
        let avg_opponent_pos = nearby_opponents
            .iter()
            .fold(Vector3::zeros(), |acc, p| acc + p.position)
            / nearby_opponents.len() as f32;

        // Calculate direction away from pressure and perpendicular to goal line
        let away_from_pressure = (player_pos - avg_opponent_pos).normalize();
        let to_goal = (goal_pos - player_pos).normalize();

        // Create a movement perpendicular to goal line
        let perpendicular = Vector3::new(-to_goal.y, to_goal.x, 0.0).normalize();

        // Blend the two directions (more weight to away from pressure)
        let direction = (away_from_pressure * 0.7 + perpendicular * 0.3).normalize();

        // Move slightly in the calculated direction
        Some(player_pos + direction * 5.0)
    }
}
