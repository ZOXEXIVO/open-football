use crate::r#match::player::strategies::processor::{StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct CommonInjuredState {}

impl StateProcessingHandler for CommonInjuredState {
    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }
}
