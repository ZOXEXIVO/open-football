//! Age-based growth/decline rates per skill category, plus per-skill peak
//! age offsets.
//!
//! Curve shape:
//!   Physical:    very limited growth 0-15 -> rapid 16-22 -> plateau 23-27 ->
//!                noticeable decline 28-30 -> steep 31+
//!   Technical:   modest growth 0-15 -> rapid 16-20 -> moderate 21-26 ->
//!                plateau 27-29 -> slow decline 30+
//!   Mental:      slow growth 0-15 -> steady 16-32 -> very slow decline 33+
//!   Goalkeeping: very limited 0-15 -> later peak (28-33) -> slower decline.
//!
//! Under-16 bands have been deliberately *lowered* relative to the original
//! tuning. The realism intent: pre-pubescent and early-puberty bodies do
//! not absorb senior-grade development from the age curve alone — even
//! before the maturity multiplier, the raw curve barely moves. This shifts
//! the dominant growth signal for very young players away from "they
//! played senior matches, the curve fired" and toward "they trained
//! consistently in age-appropriate sessions".

use super::skills_array::*;

/// Returns a base development rate per week. Positive = growth, negative
/// = decline. The pair is `(min_rate, max_rate)`; the per-tick value is
/// rolled uniformly inside that band.
pub(super) fn base_weekly_rate(age: u8, cat: SkillCategory) -> (f32, f32) {
    match cat {
        SkillCategory::Physical => match age {
            0..=13 => (0.001, 0.005),
            14..=15 => (0.003, 0.010),
            16..=17 => (0.015, 0.035),
            18..=19 => (0.010, 0.025),
            20..=22 => (0.006, 0.015),
            23..=27 => (0.002, 0.008),
            28..=29 => (-0.003, 0.003),
            30..=31 => (-0.008, -0.001),
            32..=33 => (-0.012, -0.003),
            _ => (-0.018, -0.005),
        },
        SkillCategory::Technical => match age {
            0..=13 => (0.005, 0.015),
            14..=15 => (0.010, 0.028),
            16..=17 => (0.040, 0.100),
            18..=19 => (0.035, 0.080),
            20..=22 => (0.020, 0.050),
            23..=26 => (0.010, 0.028),
            27..=29 => (0.003, 0.012),
            30..=32 => (-0.006, 0.003),
            33..=35 => (-0.012, -0.002),
            _ => (-0.018, -0.004),
        },
        SkillCategory::Mental => match age {
            0..=13 => (0.003, 0.010),
            14..=15 => (0.006, 0.018),
            16..=17 => (0.025, 0.060),
            18..=19 => (0.022, 0.055),
            20..=22 => (0.018, 0.045),
            23..=26 => (0.012, 0.030),
            27..=29 => (0.008, 0.020),
            30..=32 => (0.005, 0.015),
            33..=35 => (0.002, 0.008),
            _ => (-0.003, 0.003),
        },
        SkillCategory::Goalkeeping => match age {
            0..=13 => (0.002, 0.008),
            14..=15 => (0.005, 0.014),
            16..=17 => (0.030, 0.070),
            18..=19 => (0.025, 0.060),
            20..=22 => (0.020, 0.050),
            23..=26 => (0.015, 0.035),
            27..=29 => (0.010, 0.025),
            30..=33 => (0.004, 0.015),
            34..=36 => (-0.002, 0.005),
            _ => (-0.008, -0.001),
        },
    }
}

/// Per-skill offset (in years) applied to the player's age before
/// looking up the age curve. Positive offsets shift the peak later
/// (e.g. leadership, command of area); negative offsets shift it
/// earlier (e.g. pace, agility).
pub(super) fn individual_peak_offset(idx: usize) -> i8 {
    match idx {
        SK_PACE | SK_ACCELERATION => -1,
        SK_AGILITY | SK_BALANCE => -1,
        SK_STRENGTH | SK_JUMPING => 1,
        SK_STAMINA => 0,
        SK_NATURAL_FITNESS => 2,
        SK_LEADERSHIP | SK_COMPOSURE => 3,
        SK_DECISIONS | SK_VISION | SK_POSITIONING => 2,
        SK_ANTICIPATION => 1,
        SK_FLAIR | SK_DRIBBLING => -1,
        // GK: experience-based skills peak later
        SK_GK_COMMAND_OF_AREA | SK_GK_COMMUNICATION => 3,
        SK_GK_ONE_ON_ONES | SK_GK_RUSHING_OUT => 2,
        SK_GK_HANDLING | SK_GK_PUNCHING => 1,
        // GK: reflexes/aerial reach are more physical, peak earlier
        SK_GK_REFLEXES | SK_GK_AERIAL_REACH => -1,
        _ => 0,
    }
}
