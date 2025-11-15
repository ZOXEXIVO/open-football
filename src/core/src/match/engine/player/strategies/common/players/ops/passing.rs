use crate::r#match::{MatchPlayerLite, PassEvaluator, PlayerSide, StateProcessingContext};
use nalgebra::Vector3;

/// Operations for passing decision-making
pub struct PassingOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

impl<'p> PassingOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        PassingOperationsImpl { ctx }
    }

    /// Check if a pass direction is forward (toward opponent goal)
    pub fn is_forward_pass(&self, from_pos: &Vector3<f32>, to_pos: &Vector3<f32>) -> bool {
        match self.ctx.player.side {
            Some(PlayerSide::Left) => to_pos.x > from_pos.x,
            Some(PlayerSide::Right) => to_pos.x < from_pos.x,
            None => false,
        }
    }

    /// Check if passing to teammate would be a forward pass
    pub fn is_forward_pass_to(&self, teammate: &MatchPlayerLite) -> bool {
        self.is_forward_pass(&self.ctx.player.position, &teammate.position)
    }

    /// Find a safe pass option when under pressure
    pub fn find_safe_pass_option(&self) -> Option<MatchPlayerLite> {
        self.find_safe_pass_option_with_distance(50.0)
    }

    /// Find a safe pass option with custom max distance
    pub fn find_safe_pass_option_with_distance(&self, max_distance: f32) -> Option<MatchPlayerLite> {
        let teammates = self.ctx.players().teammates();

        // Prioritize closest teammates with clear passing lanes
        let safe_options: Vec<MatchPlayerLite> = teammates
            .nearby(max_distance)
            .filter(|t| {
                self.ctx.player().has_clear_pass(t.id)
                    && !self.is_teammate_under_pressure(t)
            })
            .collect();

        // Find the safest option by direction and pressure
        safe_options.into_iter().min_by(|a, b| {
            // Compare how "away from danger" the pass would be
            let a_safety = self.calculate_pass_safety(a);
            let b_safety = self.calculate_pass_safety(b);
            b_safety
                .partial_cmp(&a_safety)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// Find the best pass option using the PassEvaluator
    pub fn find_best_pass_option(&self) -> Option<(MatchPlayerLite, &'static str)> {
        PassEvaluator::find_best_pass_option(self.ctx, 300.0)
    }

    /// Find the best pass option with custom max distance
    pub fn find_best_pass_option_with_distance(&self, max_distance: f32) -> Option<(MatchPlayerLite, &'static str)> {
        PassEvaluator::find_best_pass_option(self.ctx, max_distance)
    }

    /// Calculate how safe a pass would be based on direction and receiver situation
    pub fn calculate_pass_safety(&self, target: &MatchPlayerLite) -> f32 {
        // Get vectors for calculations
        let pass_vector = target.position - self.ctx.player.position;
        let to_own_goal = self.ctx.ball().direction_to_own_goal() - self.ctx.player.position;

        // Calculate how much this pass moves away from own goal (higher is better)
        let pass_away_from_goal = -(pass_vector.normalize().dot(&to_own_goal.normalize()));

        // Calculate space around target player
        let space_factor = 1.0
            - (self
                .ctx
                .players()
                .opponents()
                .all()
                .filter(|o| (o.position - target.position).magnitude() < 10.0)
                .count() as f32
                * 0.2)
                .min(0.8);

        // Return combined safety score
        pass_away_from_goal + space_factor
    }

    /// Check if there are safe passing options available
    pub fn has_safe_passing_option(&self, teammates: &[MatchPlayerLite]) -> bool {
        teammates.iter().any(|teammate| {
            let has_clear_lane = self.ctx.player().has_clear_pass(teammate.id);
            let not_marked = !self.is_teammate_under_pressure(teammate);

            has_clear_lane && not_marked
        })
    }

    /// Check if a teammate is under pressure
    pub fn is_teammate_under_pressure(&self, teammate: &MatchPlayerLite) -> bool {
        self.ctx
            .players()
            .opponents()
            .all()
            .filter(|o| (o.position - teammate.position).magnitude() < 10.0)
            .count()
            >= 1
    }

    /// Find any teammate as a last resort option
    pub fn find_any_teammate(&self) -> Option<MatchPlayerLite> {
        self.find_any_teammate_with_distance(200.0)
    }

    /// Find any teammate with custom max distance
    pub fn find_any_teammate_with_distance(&self, max_distance: f32) -> Option<MatchPlayerLite> {
        // Get the closest teammate regardless of quality
        self.ctx
            .players()
            .teammates()
            .nearby(max_distance)
            .min_by(|a, b| {
                let dist_a = (a.position - self.ctx.player.position).magnitude();
                let dist_b = (b.position - self.ctx.player.position).magnitude();
                dist_a
                    .partial_cmp(&dist_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Check for forward passes to better positioned teammates
    pub fn has_forward_pass_to_better_teammate(
        &self,
        teammates: &[MatchPlayerLite],
        current_distance_to_goal: f32,
    ) -> bool {
        let player_pos = self.ctx.player.position;

        teammates.iter().any(|teammate| {
            // Must be a forward pass direction
            let is_forward = self.is_forward_pass(&player_pos, &teammate.position);

            if !is_forward {
                return false;
            }

            // Teammate must be much closer to goal
            let teammate_distance =
                (teammate.position - self.ctx.player().opponent_goal_position()).magnitude();
            let is_much_closer = teammate_distance < current_distance_to_goal * 0.6;
            let not_heavily_marked = !self.is_teammate_heavily_marked(teammate);
            let has_clear_lane = self.ctx.player().has_clear_pass(teammate.id);

            is_much_closer && not_heavily_marked && has_clear_lane
        })
    }

    /// Check if teammate is heavily marked
    pub fn is_teammate_heavily_marked(&self, teammate: &MatchPlayerLite) -> bool {
        let marking_distance = 8.0;
        let close_distance = 3.0;

        let markers = self
            .ctx
            .players()
            .opponents()
            .all()
            .filter(|o| (o.position - teammate.position).magnitude() < marking_distance)
            .count();

        let close_markers = self
            .ctx
            .players()
            .opponents()
            .all()
            .filter(|o| (o.position - teammate.position).magnitude() < close_distance)
            .count();

        markers >= 2 || (markers >= 1 && close_markers > 0)
    }

    /// Check if there's a teammate in a dangerous attacking position
    pub fn has_teammate_in_dangerous_position(
        &self,
        teammates: &[MatchPlayerLite],
        current_distance_to_goal: f32,
    ) -> bool {
        teammates.iter().any(|teammate| {
            let teammate_distance =
                (teammate.position - self.ctx.player().opponent_goal_position()).magnitude();

            // Check if teammate is in a good attacking position
            let in_attacking_position = teammate_distance < current_distance_to_goal * 1.1;

            // Check if teammate is in free space
            let in_free_space = self
                .ctx
                .players()
                .opponents()
                .all()
                .filter(|opp| (opp.position - teammate.position).magnitude() < 12.0)
                .count()
                < 2;

            // Check if teammate is making a forward run
            let teammate_velocity = self
                .ctx
                .tick_context
                .positions
                .players
                .velocity(teammate.id);
            let making_run = teammate_velocity.magnitude() > 2.0 && {
                let to_goal = self.ctx.player().opponent_goal_position() - teammate.position;
                teammate_velocity.normalize().dot(&to_goal.normalize()) > 0.5
            };

            let has_clear_pass = self.ctx.player().has_clear_pass(teammate.id);

            has_clear_pass && in_attacking_position && (in_free_space || making_run)
        })
    }

    /// Check for any good passing option (balanced assessment)
    pub fn has_good_passing_option(&self, teammates: &[MatchPlayerLite]) -> bool {
        teammates.iter().any(|teammate| {
            let has_clear_lane = self.ctx.player().has_clear_pass(teammate.id);
            let has_space = self
                .ctx
                .players()
                .opponents()
                .all()
                .filter(|opp| (opp.position - teammate.position).magnitude() < 10.0)
                .count()
                < 2;

            // Prefer forward passes
            let is_forward_pass = self.is_forward_pass_to(teammate);

            has_clear_lane && has_space && is_forward_pass
        })
    }
}
