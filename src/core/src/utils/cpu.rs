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

/// `true` when the running CPU advertises ARM NEON. On AArch64 (Apple
/// Silicon, modern ARM servers) NEON is part of the mandatory ARMv8-A
/// baseline, so this is a compile-time constant. Always `false` on
/// non-AArch64 targets.
#[cfg(target_arch = "aarch64")]
#[inline]
pub fn neon_available() -> bool {
    true
}

#[cfg(not(target_arch = "aarch64"))]
#[inline]
pub fn neon_available() -> bool {
    false
}

/// Human-readable name of the SIMD path the distance-matrix kernel will
/// take on this host: `"AVX2"` on x86_64 with AVX2, `"NEON"` on AArch64,
/// `"scalar"` otherwise. Match-engine dispatch uses the same ordering.
#[inline]
pub fn simd_kernel_name() -> &'static str {
    if avx2_available() {
        "AVX2"
    } else if neon_available() {
        "NEON"
    } else {
        "scalar"
    }
}
