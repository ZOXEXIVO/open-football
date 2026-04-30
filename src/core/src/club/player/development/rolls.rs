//! Deterministic roll source for the development tick.
//!
//! Production code uses [`ThreadRolls`] which forwards to `rand::random()`.
//! Tests use [`FixedRolls`] (every roll returns the same value) so a
//! development tick produces stable, inspectable output.

/// Source of uniform random numbers in `[0.0, 1.0)`. Implementations are
/// expected to be cheap and stateful (each call advances the stream).
pub trait RollSource {
    fn roll_unit(&mut self) -> f32;
}

/// Default production roll source backed by the thread-local RNG.
pub struct ThreadRolls;

impl RollSource for ThreadRolls {
    #[inline]
    fn roll_unit(&mut self) -> f32 {
        rand::random::<f32>()
    }
}

/// Roll source that returns the same value on every call. Useful when
/// tests want to pin the per-skill roll to either the lower or upper
/// edge of the age-curve band.
#[derive(Debug, Clone, Copy)]
pub struct FixedRolls(pub f32);

impl RollSource for FixedRolls {
    #[inline]
    fn roll_unit(&mut self) -> f32 {
        self.0
    }
}
