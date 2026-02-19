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
        self.ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| (opp.position - teammate.position).magnitude() < marking_distance)
            .count()
            >= 1
    }

    /// Check if a teammate is heavily marked (multiple opponents or very close marking)
    pub fn is_teammate_heavily_marked(&self, teammate: &MatchPlayerLite) -> bool {
        let marking_distance = 8.0;
        let close_marking_distance = 3.0;

        let markers = self
            .ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| (opp.position - teammate.position).magnitude() < marking_distance)
            .count();

        let close_markers = self
            .ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| (opp.position - teammate.position).magnitude() < close_marking_distance)
            .count();

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
        let close_opponents = self.pressing_opponents_count(10.0) as f32;
        let medium_opponents = self.pressing_opponents_count(20.0) as f32;
        let far_opponents = self.pressing_opponents_count(30.0) as f32;

        // Weight closer opponents more heavily
        let intensity = (close_opponents * 0.5 + medium_opponents * 0.3 + far_opponents * 0.2) / 3.0;

        intensity.min(1.0)
    }

    /// Check if there's space around the player (inverse of pressure)
    pub fn has_space_around(&self, min_distance: f32) -> bool {
        !self.ctx.players().opponents().exists(min_distance)
    }
}
