//! Deterministic random stream for player generation.
//!
//! ODB hydration seeds one stream per record id so the same database record
//! always hydrates to the same player in every new game (attribute profile,
//! archetype, PA band roll, body metrics — everything). Procedural generation
//! seeds from OS entropy and keeps its historical unseeded behaviour.

/// Splitmix64-based generator. Small, fast, and statistically fine for
/// attribute jitter; nearby seeds (sequential player ids) produce
/// uncorrelated streams because every draw scrambles the counter.
pub struct HydrationRng {
    state: u64,
}

impl HydrationRng {
    /// Deterministic stream for a known identity (e.g. an ODB player id).
    pub fn from_seed(seed: u64) -> Self {
        let mut rng = HydrationRng { state: seed };
        // One warm-up draw so trivially small seeds (id 1, 2, …) don't hand
        // their raw value to the first consumer.
        rng.next_u64();
        rng
    }

    /// Entropy-seeded stream for procedural generation, where cross-save
    /// stability is not wanted.
    pub fn from_entropy() -> Self {
        HydrationRng {
            state: rand::random::<u64>(),
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform f32 in [0, 1).
    pub fn f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) / (1u64 << 24) as f32
    }

    fn f64_unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64) / (1u64 << 53) as f64
    }

    /// Uniform integer in [min, max) — same half-open convention as
    /// `IntegerUtils::random`, so call sites translate one-to-one.
    pub fn int_range(&mut self, min: i32, max: i32) -> i32 {
        if max <= min {
            return min;
        }
        min + (self.f64_unit() * ((max - min) as f64)) as i32
    }

    /// Uniform f32 in [min, max).
    pub fn float_range(&mut self, min: f32, max: f32) -> f32 {
        min + self.f32() * (max - min)
    }

    /// Standard normal (mean 0, std 1) via Box-Muller.
    pub fn normal(&mut self) -> f32 {
        let u1 = self.f32().max(1e-10);
        let u2 = self.f32();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = HydrationRng::from_seed(42);
        let mut b = HydrationRng::from_seed(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = HydrationRng::from_seed(1);
        let mut b = HydrationRng::from_seed(2);
        let same = (0..32).filter(|_| a.next_u64() == b.next_u64()).count();
        assert_eq!(same, 0, "adjacent seeds must not correlate");
    }

    #[test]
    fn ranges_respect_bounds() {
        let mut rng = HydrationRng::from_seed(7);
        for _ in 0..1000 {
            let v = rng.int_range(3, 10);
            assert!((3..10).contains(&v), "int_range out of bounds: {v}");
            let f = rng.f32();
            assert!((0.0..1.0).contains(&f), "f32 out of bounds: {f}");
            let fr = rng.float_range(2.0, 5.0);
            assert!((2.0..5.0).contains(&fr), "float_range out of bounds: {fr}");
        }
        assert_eq!(rng.int_range(5, 5), 5, "empty range returns min");
    }
}
