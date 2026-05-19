//! CPU feature detection. Centralised so the rest of the codebase
//! doesn't have to reach into the match engine for "is SIMD available
//! on this host" questions.

/// `true` when the running CPU advertises AVX2. Backed by
/// `std::is_x86_feature_detected!`, which caches the result after the
/// first call — the dispatch cost on x86_64 is one load + one bit test.
/// Always `false` on non-x86_64 targets.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn avx2_available() -> bool {
    std::is_x86_feature_detected!("avx2")
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn avx2_available() -> bool {
    false
}
