//! Match-owned deterministic RNG.
//!
//! Wraps `rand::rngs::StdRng` so engine code can take a `&MatchRng`
//! instead of reaching for the global thread-local `rand::rng()`. The
//! engine seeds it from a fixture/match seed; replaying a match with
//! the same seed reproduces the same RNG stream from this RNG.
//!
//! All match-critical decision sites draw from this RNG (substitution
//! timing, shootout, foul cards / advantage / call gating, corner
//! aerial contest, passing / shooting / save / first-touch / tackle
//! rolls, every player state). The only `rand::rng()` site is
//! `MatchRng::from_entropy()`, which derives a single seed at context
//! construction so live production matches stay non-deterministic
//! without a global RNG inside the tick loop.
use rand::distr::uniform::{SampleRange, SampleUniform};
use rand::{RngExt, SeedableRng, rngs::StdRng};
use std::cell::RefCell;

/// Deterministic, seedable RNG owned by `MatchContext`.
///
/// Player-state code holds `&MatchContext` (not `&mut`) — the engine's
/// dispatcher hands every state an immutable view so multiple states
/// can be evaluated against the same world snapshot. That makes a
/// plain `&mut self` RNG hostile to the existing access pattern. We
/// use `RefCell<StdRng>` so a state can draw a roll via
/// `context.rng.unit_f32()` without taking `&mut MatchContext`. The
/// borrow is held only for the body of each helper call (one
/// `random()` / `random_range()` invocation), so there is no
/// dynamic-borrow conflict between states.
///
/// Construction:
///   * `MatchRng::from_seed(seed)` — engine/test entry point.
///   * `MatchRng::from_entropy()` — production default when no fixture
///     seed is available. Pulls one seed from the OS RNG so live
///     production matches remain non-deterministic (today's behaviour)
///     but a future caller can pin a seed without API churn.
pub struct MatchRng {
    inner: RefCell<StdRng>,
    seed: u64,
}

impl MatchRng {
    /// Build a seeded RNG. Two matches built with the same seed will
    /// emit identical sequences from their own helpers.
    pub fn from_seed(seed: u64) -> Self {
        // Fan the u64 seed into the 32-byte StdRng seed via a simple
        // splitmix64 expansion. Avoids pulling in a hashing crate just
        // to derive a wider seed.
        let mut bytes = [0u8; 32];
        let mut state = seed;
        for chunk in bytes.chunks_exact_mut(8) {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            chunk.copy_from_slice(&z.to_le_bytes());
        }
        MatchRng {
            inner: RefCell::new(StdRng::from_seed(bytes)),
            seed,
        }
    }

    /// Production default — derives a seed from the OS RNG so live
    /// matches stay non-deterministic. The seed itself is retained so
    /// failed matches could in principle be replayed by saving it.
    pub fn from_entropy() -> Self {
        // Tap the thread-local RNG just once to derive the persistent
        // seed. After construction, no further global-RNG use occurs
        // for this match (provided callers prefer the owned facade).
        let seed: u64 = rand::rng().random();
        Self::from_seed(seed)
    }

    /// Returns the seed used to construct this RNG. Logged on
    /// post-match diagnostics so an unexpected result can be reproduced.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Uniform `[0, 1)` sample — direct replacement for
    /// `rand::random::<f32>()`. Takes `&self` so callers holding
    /// `&MatchContext` can use it.
    #[inline]
    pub fn unit_f32(&self) -> f32 {
        self.inner.borrow_mut().random::<f32>()
    }

    /// Uniform `[low, high)` integer in the given range — direct
    /// replacement for `rng.random_range(low..high)`.
    #[inline]
    pub fn range_u64(&self, low: u64, high: u64) -> u64 {
        self.inner.borrow_mut().random_range(low..high)
    }

    /// Uniform i32 range. Convenience for the few sites that already
    /// build `random_range(-N..N)` ranges by hand.
    #[inline]
    pub fn range_i32(&self, low: i32, high: i32) -> i32 {
        self.inner.borrow_mut().random_range(low..high)
    }

    /// Uniform f32 in `[low, high)`.
    #[inline]
    pub fn range_f32(&self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit_f32()
    }

    /// Symmetric jitter: uniform in `[center - half_width, center + half_width)`.
    ///
    /// A zero (or negative) `half_width` yields no jitter and returns
    /// `center` exactly. This is the load-bearing case: engine code derives
    /// the half-width from skill as `(1.0 - skill) * k`, which collapses to
    /// `0.0` for a max-skill player (heading 20, or passing/technique/vision
    /// all maxed). Feeding `center..center` to `random_range` panics with
    /// "cannot sample empty range" — that crashed national-competition
    /// matches, which field the highest-quality players. The non-degenerate
    /// branch draws via the same generic `random_range`, so the RNG stream
    /// and distribution stay identical to a hand-written
    /// `random_range(center - half_width..center + half_width)`.
    #[inline]
    pub fn jitter(&self, center: f32, half_width: f32) -> f32 {
        if half_width > f32::EPSILON {
            self.random_range(center - half_width..center + half_width)
        } else {
            center
        }
    }

    /// Generic range — drop-in for `rng.random_range(low..high)` so
    /// callers don't have to pick between `range_f32` / `range_i32`
    /// / `range_u64` based on the inferred type. Borrows the inner
    /// RNG only for the call.
    #[inline]
    pub fn random_range<T, R>(&self, range: R) -> T
    where
        T: SampleUniform,
        R: SampleRange<T>,
    {
        self.inner.borrow_mut().random_range(range)
    }

    /// Generic single draw — drop-in for `rng.random::<T>()`. The
    /// borrow is held only for the call. Common type instantiations
    /// (u32 noise, f32 unit, f64 unit, bool) reuse the same inner
    /// `StdRng` so the seeded stream stays coherent.
    #[inline]
    pub fn random<T>(&self) -> T
    where
        rand::distr::StandardUniform: rand::distr::Distribution<T>,
    {
        self.inner.borrow_mut().random::<T>()
    }

    /// Bernoulli trial — `true` with probability `p` (clamped to [0,1]).
    #[inline]
    pub fn bernoulli(&self, p: f32) -> bool {
        self.unit_f32() < p.clamp(0.0, 1.0)
    }

    /// Run a closure with the underlying `StdRng`. For the rare site
    /// that needs to pass `&mut R: Rng` into a `rand` helper (e.g.
    /// shuffling). The borrow is held only for the closure body.
    pub fn with_inner<R>(&self, f: impl FnOnce(&mut StdRng) -> R) -> R {
        f(&mut self.inner.borrow_mut())
    }
}

impl Default for MatchRng {
    fn default() -> Self {
        Self::from_entropy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_emits_same_stream() {
        let a = MatchRng::from_seed(0xC0FFEE);
        let b = MatchRng::from_seed(0xC0FFEE);
        for _ in 0..256 {
            assert_eq!(a.unit_f32().to_bits(), b.unit_f32().to_bits());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let a = MatchRng::from_seed(1);
        let b = MatchRng::from_seed(2);
        let mut diverged = false;
        for _ in 0..256 {
            if a.unit_f32() != b.unit_f32() {
                diverged = true;
                break;
            }
        }
        assert!(diverged, "two distinct seeds must diverge within 256 draws");
    }

    #[test]
    fn range_and_bernoulli_are_deterministic() {
        let a = MatchRng::from_seed(42);
        let b = MatchRng::from_seed(42);
        for _ in 0..64 {
            assert_eq!(a.range_u64(0, 100), b.range_u64(0, 100));
            assert_eq!(a.bernoulli(0.5), b.bernoulli(0.5));
        }
    }

    #[test]
    fn unit_f32_stays_in_unit_interval() {
        let r = MatchRng::from_seed(7);
        for _ in 0..1024 {
            let v = r.unit_f32();
            assert!((0.0..1.0).contains(&v), "out of band: {v}");
        }
    }

    #[test]
    fn seed_is_exposed_for_diagnostics() {
        let r = MatchRng::from_seed(0xDEADBEEF);
        assert_eq!(r.seed(), 0xDEADBEEF);
    }

    #[test]
    fn jitter_zero_half_width_returns_center_without_panicking() {
        // A max-skill player produces `(1.0 - skill) * k == 0.0`, which used
        // to feed `random_range(center..center)` and panic with "cannot
        // sample empty range" (crashed national-competition matches).
        let r = MatchRng::from_seed(123);
        assert_eq!(r.jitter(1.0, 0.0), 1.0);
        assert_eq!(r.jitter(0.0, 0.0), 0.0);
        // Negative half-widths (e.g. `overall_quality` rounding past 1.0)
        // must also be inert rather than producing a reversed range.
        assert_eq!(r.jitter(2.5, -0.1), 2.5);
    }

    #[test]
    fn jitter_stays_within_symmetric_band() {
        let r = MatchRng::from_seed(99);
        for _ in 0..1024 {
            let v = r.jitter(1.0, 0.35);
            assert!((0.65..1.35).contains(&v), "out of band: {v}");
        }
    }
}
