//! The eighteen-yard box as a rectangle, derived from the field size so
//! it scales with whatever pitch dimensions the match was built with.

use super::match_context::MatchContext;
use nalgebra::Vector3;

impl MatchContext {
    pub fn penalty_area(&self, is_home_team: bool) -> PenaltyArea {
        let field_width = self.field_size.width as f32;
        let field_height = self.field_size.height as f32;
        let scale = field_width / 105.0; // Field units per real meter
        let penalty_area_width = 40.32 * scale; // 40.32m wide (centered on goal)
        let penalty_area_depth = 16.5 * scale; // 16.5m deep from goal line

        if is_home_team {
            PenaltyArea::new(
                Vector3::new(0.0, (field_height - penalty_area_width) / 2.0, 0.0),
                Vector3::new(
                    penalty_area_depth,
                    (field_height + penalty_area_width) / 2.0,
                    0.0,
                ),
            )
        } else {
            PenaltyArea::new(
                Vector3::new(
                    field_width - penalty_area_depth,
                    (field_height - penalty_area_width) / 2.0,
                    0.0,
                ),
                Vector3::new(field_width, (field_height + penalty_area_width) / 2.0, 0.0),
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PenaltyArea {
    pub min: Vector3<f32>,
    pub max: Vector3<f32>,
}

impl PenaltyArea {
    pub fn new(min: Vector3<f32>, max: Vector3<f32>) -> Self {
        PenaltyArea { min, max }
    }

    pub fn contains(&self, point: &Vector3<f32>) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }
}
