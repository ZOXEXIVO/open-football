use crate::utils::random::engine;

pub struct IntegerUtils;

impl IntegerUtils {
    #[inline]
    pub fn random(min: i32, max: i32) -> i32 {
        let random_val: f64 = engine::gen_f64();
        min + (random_val * ((max - min) as f64)) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_sequential_returns_different_numbers() {
        let random_int_1 = IntegerUtils::random(0, 1000);
        let random_int_2 = IntegerUtils::random(0, 1000);

        assert_ne!(random_int_1, random_int_2);
    }
}
