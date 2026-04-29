//! Personality-aware scaling helpers used by happiness emit sites.
//!
//! Centralised here so emit sites stay declarative — a fan-criticism
//! site reads "amplified by reputation, dampened by professionalism"
//! rather than redoing the math inline. Each helper returns a positive
//! multiplier (no sign flip — the caller already knows the polarity of
//! the underlying event from the catalog).

/// Reputation amplifier. Higher-profile players feel fan/media events
/// more — their name gets carried on banners, not buried on page 14.
/// Returns ~1.0 at low reputation up to ~1.5 at the top of the scale.
#[inline]
pub fn reputation_amplifier(current_reputation: i16) -> f32 {
    let r = (current_reputation as f32 / 10_000.0).clamp(0.0, 1.0);
    1.0 + 0.5 * r
}

/// Pressure / big-match amplifier. Cup nights, derbies, decisive
/// moments hit harder for `important_matches` and `pressure` players
/// who live for those occasions. Returns 1.0 at neutral 10/10, up
/// to ~1.4 at 20/20, down to ~0.8 at 0/0.
#[inline]
pub fn pressure_amplifier(important_matches: f32, pressure: f32) -> f32 {
    let im = important_matches.clamp(0.0, 20.0) / 20.0;
    let pr = pressure.clamp(0.0, 20.0) / 20.0;
    let avg = (im + pr) * 0.5;
    // Map [0, 1] → [0.8, 1.4]
    0.8 + avg * 0.6
}

/// Criticism amplifier — provocative personalities (high `controversy`
/// or `temperament`) react more strongly to fan/media negativity.
/// Returns 1.0 at neutral 10/10, ~1.5 at 20/20.
#[inline]
pub fn criticism_amplifier(controversy: f32, temperament: f32) -> f32 {
    let c = controversy.clamp(0.0, 20.0) / 20.0;
    // Low temperament = more reactive (gets in the player's head).
    let t_inv = 1.0 - (temperament.clamp(0.0, 20.0) / 20.0);
    // Map both factors into [0, 1] then blend 60/40 controversy/temp.
    let blended = c * 0.6 + t_inv * 0.4;
    0.75 + blended * 0.75
}

/// Professionalism dampener — high-pro players brush off fan/media
/// noise more readily. Returns 1.0 at 0 professionalism down to ~0.6
/// at 20.
#[inline]
pub fn criticism_dampener(professionalism: f32) -> f32 {
    let p = professionalism.clamp(0.0, 20.0) / 20.0;
    1.0 - 0.4 * p
}

/// Ambition amplifier for upward / career-defining events (trophies,
/// continental qualification, dream moves). Returns 0.85 at zero
/// ambition, 1.0 at neutral 10, ~1.3 at 20.
#[inline]
pub fn ambition_amplifier(ambition: f32) -> f32 {
    let a = ambition.clamp(0.0, 20.0) / 20.0;
    0.85 + a * 0.45
}

/// Loyalty amplifier for club-bonded events (promotion celebration
/// for a long-serving player feels stronger). Mild — 0.9 to 1.2.
#[inline]
pub fn loyalty_amplifier(loyalty: f32) -> f32 {
    let l = loyalty.clamp(0.0, 20.0) / 20.0;
    0.9 + l * 0.3
}

/// Age amplifier for trophy-style events. Veterans (30+) treasure
/// silverware they may never win again; the very young get a smaller
/// kick because they expect more chances. Returns 0.8 (under 21),
/// 1.0 (22-29), 1.2 (30+).
#[inline]
pub fn veteran_amplifier(age: u8) -> f32 {
    if age >= 30 {
        1.2
    } else if age >= 22 {
        1.0
    } else {
        0.8
    }
}
