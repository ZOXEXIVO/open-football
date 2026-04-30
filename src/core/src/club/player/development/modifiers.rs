//! Independent multipliers applied on top of the base age-curve rate:
//! personality, match experience, official-vs-friendly weighting, average
//! rating, per-skill gap to ceiling, competition quality, decline
//! protection, workload (condition + jadedness), and match readiness.
//!
//! Also defines [`FitnessState`], the body-state gate that decides whether
//! development happens at all.

/// State of the player's body for the purposes of weekly development.
/// Drives whether growth happens at all and at what intensity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitnessState {
    /// No injury and no recovery: full development.
    Fit,
    /// Coming back from an injury. No growth, no decline either: the body
    /// is busy healing.
    Recovering,
    /// Currently injured. Skip the development tick entirely.
    Injured,
}

pub(super) fn personality_multiplier(
    professionalism: f32,
    ambition: f32,
    determination: f32,
    work_rate: f32,
) -> f32 {
    let weighted =
        professionalism * 0.40 + ambition * 0.25 + determination * 0.20 + work_rate * 0.15;
    // Map 0-20 -> 0.4-1.6
    let norm = weighted / 20.0;
    0.4 + norm * 1.2
}

// ── Match-experience multiplier ─────────────────────────────────────────
//
// Counts both official and friendly appearances. Official matches have full
// weight; friendly appearances contribute at only 20% because the competitive
// intensity and development stimulus is much lower. Loaning a young player
// for 30 league games is far more impactful than 30 U20 games.
pub(super) fn match_experience_multiplier(
    started: u16,
    sub_apps: u16,
    friendly_started: u16,
    friendly_subs: u16,
) -> f32 {
    let official = started as f32 + sub_apps as f32 * 0.4;
    let friendly = (friendly_started as f32 + friendly_subs as f32 * 0.4) * 0.2;
    let effective = official + friendly;
    (0.70 + effective * 0.020).min(1.40)
}

// ── Official match bonus ────────────────────────────────────────────────
//
// Competitive (official league/cup) matches develop players significantly
// faster than friendlies or youth-team games due to higher pressure,
// intensity, and stakes.
//
// Range: 0.75 (only friendlies) -> 1.0 (no games) -> 1.30 (only official)
pub(super) fn official_match_bonus(official_games: u16, friendly_games: u16) -> f32 {
    let total = official_games + friendly_games;
    if total == 0 {
        return 1.0;
    }
    let official_ratio = official_games as f32 / total as f32;
    0.75 + official_ratio * 0.55
}

pub(super) fn rating_multiplier(avg_rating: f32, total_games: u16) -> f32 {
    if total_games == 0 {
        return 1.0;
    }
    (1.0 + (avg_rating - 7.0) * 0.10).clamp(0.85, 1.25)
}

// ── Potential gap factor ────────────────────────────────────────────────
//
// Per-skill: how far this skill is from its ceiling. Skills near their
// ceiling barely grow. Skills far below grow fast.
pub(super) fn skill_gap_factor(current_skill: f32, skill_ceiling: f32) -> f32 {
    if skill_ceiling <= current_skill || skill_ceiling <= 1.0 {
        return 0.05;
    }
    let gap_ratio = (skill_ceiling - current_skill) / skill_ceiling;
    // Sqrt curve: stays high for longer, drops sharply near ceiling.
    (gap_ratio * 2.0).sqrt().clamp(0.1, 1.5)
}

// ── Competition quality multiplier ──────────────────────────────────────
//
// Players in stronger leagues develop faster: better opposition, higher
// tactical demands, greater physical intensity. A player getting 30 apps
// in a semi-pro division grows slower than one getting 30 apps in La Liga.
pub(super) fn competition_quality_multiplier(league_reputation: u16) -> f32 {
    if league_reputation == 0 {
        return 0.75;
    }
    let normalized = (league_reputation as f32 / 10000.0).clamp(0.0, 1.0);
    (0.70 + normalized * 0.45).clamp(0.70, 1.15)
}

pub(super) fn decline_protection(natural_fitness: f32, professionalism: f32) -> f32 {
    let nf_norm = natural_fitness / 20.0;
    let pr_norm = professionalism / 20.0;
    let protection = nf_norm * 0.50 + pr_norm * 0.50;
    1.0 - protection * 0.50
}

/// Multiplier applied to *growth* rates based on the player's chronic
/// workload state. A drained player learns less even when they show up.
///
/// `condition_pct` is 0..100, `jadedness` is 0..10000.
pub(super) fn workload_growth_modifier(condition_pct: u32, jadedness: i16) -> f32 {
    let cond = (condition_pct as f32 / 100.0).clamp(0.0, 1.0);
    // Very low condition (<40%) drags hardest; full condition is neutral.
    let cond_mult = (0.55 + cond * 0.45).clamp(0.55, 1.0);

    let jad = (jadedness.max(0) as f32 / 10000.0).clamp(0.0, 1.0);
    // No jadedness = neutral; max jadedness blunts growth ~35%.
    let jad_mult = (1.0 - jad * 0.35).clamp(0.65, 1.0);

    (cond_mult * jad_mult).clamp(0.40, 1.0)
}

/// Multiplier applied to *decline* rates (only used when the per-tick
/// roll is negative). A burned-out player decays a little faster.
pub(super) fn workload_decline_amplifier(condition_pct: u32, jadedness: i16) -> f32 {
    let cond = (condition_pct as f32 / 100.0).clamp(0.0, 1.0);
    let jad = (jadedness.max(0) as f32 / 10000.0).clamp(0.0, 1.0);
    // Up to +25% decline for chronically tired/jaded players.
    1.0 + (1.0 - cond) * 0.10 + jad * 0.15
}

/// Match readiness (0-20) feeds match-driven growth a small extra push.
/// A player kept match-sharp benefits more from the same minutes than one
/// who's been out of the rhythm.
pub(super) fn match_readiness_multiplier(match_readiness: f32) -> f32 {
    let mr = (match_readiness / 20.0).clamp(0.0, 1.0);
    // 0 readiness -> 0.90, 20 -> 1.10
    0.90 + mr * 0.20
}
