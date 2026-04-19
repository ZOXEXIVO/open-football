use crate::utils::random::engine;

pub struct FloatUtils;

impl FloatUtils {
    #[inline]
    pub fn random(min: f32, max: f32) -> f32 {
        let random_val: f32 = engine::gen_f32();
        min + (random_val * (max - min))
    }
}
