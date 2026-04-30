use crate::r#match::{MatchPlayerLite, StateProcessingContext};

/// Operations for assessing pressure situations on the field
pub struct PressureOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

impl<'p> PressureOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        PressureOperationsImpl { ctx }
    }

    /// Check if player is under immediate pressure (at least one opponent within 1m)
    pub fn is_under_immediate_pressure(&self) -> bool {
        self.is_under_immediate_pressure_with_distance(5.0)
    }

    /// Check if player is under immediate pressure with custom distance
    pub fn is_under_immediate_pressure_with_distance(&self, distance: f32) -> bool {
        self.ctx.players().opponents().exists(distance)
    }

    /// Check if player is under heavy pressure (multiple opponents within 1m)
    pub fn is_under_heavy_pressure(&self) -> bool {
        self.is_under_heavy_pressure_with_params(3.0, 2)
    }

    /// Check if player is under heavy pressure with custom parameters
    pub fn is_under_heavy_pressure_with_params(&self, distance: f32, threshold: usize) -> bool {
        self.pressing_opponents_count(distance) >= threshold
    }

    /// Count pressing opponents within distance
    pub fn pressing_opponents_count(&self, distance: f32) -> usize {
        self.ctx.players().opponents().nearby(distance).count()
    }

    /// Check if a teammate is marked by opponents
    pub fn is_teammate_marked(&self, teammate: &MatchPlayerLite, marking_distance: f32) -> bool {
        // Use pre-computed distances: opponents of teammate = our players near them,
        // but we need opponents near teammate, so from teammate's POV our team are opponents
        self.ctx
            .tick_context
            .grid
            .opponents(teammate.id, marking_distance)
            .count()
            >= 1
    }

    /// Check if a teammate is heavily marked (multiple opponents or very close marking)
    pub fn is_teammate_heavily_marked(&self, teammate: &MatchPlayerLite) -> bool {
        // Single scan at max distance, bucket by distance
        let mut markers = 0;
        let mut close_markers = 0;
        for (_id, dist) in self.ctx.tick_context.grid.opponents(teammate.id, 8.0) {
            markers += 1;
            if dist <= 3.0 {
                close_markers += 1;
            }
        }

        markers >= 2 || (markers >= 1 && close_markers > 0)
    }

    /// Get the closest pressing opponent
    pub fn closest_pressing_opponent(&self, max_distance: f32) -> Option<MatchPlayerLite> {
        self.ctx
            .players()
            .opponents()
            .nearby(max_distance)
            .min_by(|a, b| {
                let dist_a = a.distance(self.ctx);
                let dist_b = b.distance(self.ctx);
                dist_a.partial_cmp(&dist_b).unwrap()
            })
    }

    /// Calculate pressure intensity (0.0 = no pressure, 1.0 = extreme pressure)
    pub fn pressure_intensity(&self) -> f32 {
        // Single scan at max distance, bucket by distance
        let mut close: f32 = 0.0;
        let mut medium: f32 = 0.0;
        let mut far: f32 = 0.0;
        for (_id, dist) in self
            .ctx
            .tick_context
            .grid
            .opponents(self.ctx.player.id, 30.0)
        {
            far += 1.0;
            if dist <= 20.0 {
                medium += 1.0;
            }
            if dist <= 10.0 {
                close += 1.0;
            }
        }

        // Weight closer opponents more heavily
        let intensity = (close * 0.5 + medium * 0.3 + far * 0.2) / 3.0;
        intensity.min(1.0)
    }

    /// Check if there's space around the player (inverse of pressure)
    pub fn has_space_around(&self, min_distance: f32) -> bool {
        !self.ctx.players().opponents().exists(min_distance)
    }
}
