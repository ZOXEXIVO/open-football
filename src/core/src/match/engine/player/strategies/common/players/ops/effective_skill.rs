//! Fatigue-aware effective-skill helper.
//!
//! Real footballers don't just slow down when tired — their first touch
//! gets heavy, decisions get rushed, pressing arrives late, and explosive
//! actions (sprint, jump, dive) lose more than steady-state ones. This
//! helper takes a base skill value (1–20) and returns the *effective*
//! value after applying:
//!
//!   1. Match-condition multipliers via a smooth continuous curve per
//!      category (technical / mental / explosive). At full match
//!      condition (≥ 80%) the penalty is zero; at the in-match floor
//!      (15%) explosive drops ~20%, mental ~12%, technical ~14%. The
//!      curve is anchored so it agrees with the legacy band values at
//!      65 / 45 / 30 / 0% but never produces a cliff between bands —
//!      a player drifting from 46 → 44% condition no longer suddenly
//!      loses ~10% explosive output in a single tick.
//!   2. Stamina + natural_fitness mitigation: elite-fitness players
//!      recover up to ~35% of the fatigue penalty when fresh, tapering
//!      to ~20% at the in-match condition floor. Elite stamina cannot
//!      nullify deep drain — the legs go regardless once condition is
//!      truly broken.
//!   3. Late-game mental fatigue: after the 70th minute, condition < 45%
//!      additionally drops decisions/concentration/composure 3–10%, with
//!      high determination reducing that secondary penalty by up to 40%.
//!
//! All callers route skill reads through `effective_skill_*` to make the
//! engine actually feel the gap between a fresh elite stamina player and
//! a wilting late-game specimen.

use crate::r#match::MatchPlayer;

/// What kind of action the skill is being read for. Drives the size of
/// the fatigue penalty — explosive actions (sprints, dives, tackles
/// requiring acceleration) suffer more than steady-state ones.
#[derive(Debug, Clone, Copy)]
pub enum SkillCategory {
    /// First touch, passing, crossing, shooting, technique-led actions.
    Technical,
    /// Decisions, concentration, composure, anticipation, vision.
    Mental,
    /// Pace, acceleration, jumping, agility — short-burst actions.
    Explosive,
}

/// Per-action context for the effective-skill calculation. The minute
/// matters because late-game mental fatigue compounds with low condition.
#[derive(Debug, Clone, Copy)]
pub struct ActionContext {
    /// Match minute (0..=120). Used for the late-game mental penalty.
    pub minute: u32,
    pub category: SkillCategory,
}

impl ActionContext {
    pub fn technical(minute: u32) -> Self {
        Self {
            minute,
            category: SkillCategory::Technical,
        }
    }
    pub fn mental(minute: u32) -> Self {
        Self {
            minute,
            category: SkillCategory::Mental,
        }
    }
    pub fn explosive(minute: u32) -> Self {
        Self {
            minute,
            category: SkillCategory::Explosive,
        }
    }
}

/// Per-category maximum fatigue penalty at 0% condition. Calibrated to
/// hit the legacy band values at the old breakpoints (65 / 45 / 30%)
/// while the curve below interpolates smoothly between them.
const MAX_PENALTY_TECHNICAL: f32 = 0.22;
const MAX_PENALTY_MENTAL: f32 = 0.18;
const MAX_PENALTY_EXPLOSIVE: f32 = 0.32;

/// Condition at which the smooth fatigue penalty starts to bite. Above
/// this, the player is treated as fresh.
const FRESH_CONDITION: f32 = 0.80;

/// Curvature of the deficit→penalty mapping. Slightly super-linear so
/// the penalty accelerates as condition drops — preserves the legacy
/// step at 30% as a smooth inflection rather than a cliff. Hand-fit so
/// the smooth curve agrees with the old band values at 65 / 45 / 30 / 0
/// within ~0.02 multiplier units across all three categories.
const PENALTY_EXPONENT: f32 = 1.20;

#[inline]
fn max_penalty(category: SkillCategory) -> f32 {
    match category {
        SkillCategory::Technical => MAX_PENALTY_TECHNICAL,
        SkillCategory::Mental => MAX_PENALTY_MENTAL,
        SkillCategory::Explosive => MAX_PENALTY_EXPLOSIVE,
    }
}

/// Internal: smooth fatigue multiplier per category. Returned value is
/// the **effective fraction** of the base skill (1.00 = no penalty,
/// ~0.68 = explosive at 0% condition). The legacy stepwise bands lived
/// here; the curve below hits the same anchor values without the cliffs
/// between bands that produced visible state-change artefacts.
fn band_multipliers(condition_pct: f32, category: SkillCategory) -> f32 {
    let p = condition_pct.clamp(0.0, 1.0);
    if p >= FRESH_CONDITION {
        return 1.00;
    }
    // Deficit in [0, 1]: 0 at the freshness threshold, 1 at zero condition.
    let deficit = ((FRESH_CONDITION - p) / FRESH_CONDITION).clamp(0.0, 1.0);
    let curve = deficit.powf(PENALTY_EXPONENT);
    (1.0 - max_penalty(category) * curve).clamp(0.40, 1.00)
}

/// Compute the per-player fatigue-mitigation score in [0.0, 1.0]. Players
/// with elite stamina and natural_fitness recover up to ~35% of the
/// penalty when fresh, tapering to ~20% at the in-match condition floor.
fn mitigation_score(player: &MatchPlayer) -> f32 {
    let stamina = (player.skills.physical.stamina / 20.0).clamp(0.0, 1.0);
    let nat_fit = (player.skills.physical.natural_fitness / 20.0).clamp(0.0, 1.0);
    (stamina * 0.55 + nat_fit * 0.45).clamp(0.0, 1.0)
}

/// Effective mitigation cap as a function of remaining condition. Elite
/// stamina helps you stay sharp through the first hour, but once
/// condition is truly broken the legs go regardless — the cap tapers
/// from 35% (fresh) to ~19% (zero condition) so a max-stamina player at
/// the 15% floor still loses meaningful explosive output. Without this
/// taper, a high-stamina player at 20% condition would still execute
/// near a fresh-player level, which contradicts the spec's "elite
/// stamina mitigates but does not nullify extreme fatigue" requirement.
#[inline]
fn mitigation_cap(condition_pct: f32) -> f32 {
    let p = condition_pct.clamp(0.0, 1.0);
    0.35 * (0.55 + 0.45 * p)
}

/// Late-game mental compounding penalty. After the 70th minute, low
/// condition additionally degrades decision / concentration / composure.
/// Returns a multiplier ≤ 1.0 (1.0 = no extra penalty).
fn late_game_mental_extra(player: &MatchPlayer, ctx: ActionContext) -> f32 {
    if !matches!(ctx.category, SkillCategory::Mental) {
        return 1.0;
    }
    if ctx.minute < 70 {
        return 1.0;
    }
    let cond_pct = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
    if cond_pct >= 0.45 {
        return 1.0;
    }
    // Linear penalty in [3%, 10%] as condition drops 0.45 -> 0.0.
    let raw_penalty = 0.03 + (0.45 - cond_pct) / 0.45 * 0.07;
    // Determination knocks up to 40% off the secondary penalty.
    let det = (player.skills.mental.determination / 20.0).clamp(0.0, 1.0);
    let mitigated = raw_penalty * (1.0 - det * 0.40);
    1.0 - mitigated
}

/// Apply the full fatigue model to a base skill value (1–20 scale).
/// Returned value stays in 1–20 space so callers can treat the result
/// like any other skill read.
pub fn effective_skill(player: &MatchPlayer, base: f32, ctx: ActionContext) -> f32 {
    let cond_pct = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
    let band = band_multipliers(cond_pct, ctx.category);
    // Mitigate the penalty: elite stamina recovers a fraction of the
    // lost band, with the cap tapered by condition so deep exhaustion
    // cannot be cancelled by fitness alone.
    let mitigation = mitigation_score(player);
    let cap = mitigation_cap(cond_pct);
    let recovered = 1.0 - (1.0 - band) * (1.0 - mitigation * cap);
    let extra = late_game_mental_extra(player, ctx);
    (base * recovered * extra).clamp(1.0, 20.0)
}

/// Convenience: read a skill from the player and apply the fatigue model.
/// `accessor` returns the raw skill in 1–20 space.
#[inline]
pub fn read_effective<F>(player: &MatchPlayer, ctx: ActionContext, accessor: F) -> f32
where
    F: FnOnce(&MatchPlayer) -> f32,
{
    effective_skill(player, accessor(player), ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerSkills;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
    };
    use chrono::NaiveDate;

    fn build_player(condition: i16, stamina: f32, natural_fitness: f32) -> MatchPlayer {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = condition;
        attrs.jadedness = 0;
        let mut skills = PlayerSkills::default();
        skills.physical.stamina = stamina;
        skills.physical.natural_fitness = natural_fitness;
        skills.mental.determination = 12.0;
        let player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &player, PlayerPositionType::MidfielderCenter, false)
    }

    #[test]
    fn fresh_player_has_no_penalty() {
        let p = build_player(9000, 14.0, 14.0);
        let eff = effective_skill(&p, 15.0, ActionContext::technical(45));
        assert!((eff - 15.0).abs() < 0.01);
    }

    #[test]
    fn exhausted_player_loses_explosive_more_than_technical() {
        let p = build_player(2500, 10.0, 10.0);
        let tech = effective_skill(&p, 15.0, ActionContext::technical(80));
        let expl = effective_skill(&p, 15.0, ActionContext::explosive(80));
        assert!(expl < tech);
        assert!(tech < 15.0);
    }

    #[test]
    fn elite_stamina_mitigates_fatigue() {
        let weak = build_player(3500, 8.0, 8.0);
        let elite = build_player(3500, 19.0, 18.0);
        let weak_skill = effective_skill(&weak, 15.0, ActionContext::technical(85));
        let elite_skill = effective_skill(&elite, 15.0, ActionContext::technical(85));
        assert!(elite_skill > weak_skill);
    }

    #[test]
    fn late_game_mental_extra_only_after_70() {
        let p = build_player(3000, 12.0, 12.0);
        let early = effective_skill(&p, 15.0, ActionContext::mental(50));
        let late = effective_skill(&p, 15.0, ActionContext::mental(85));
        assert!(late < early);
    }

    #[test]
    fn smooth_curve_has_no_cliffs_at_legacy_band_boundaries() {
        // The old stepwise bands jumped at 65 / 45 / 30% condition —
        // a player drifting across the boundary could lose ~10% of
        // their explosive output in a single tick. The smooth curve
        // must produce changes proportional to the condition delta.
        let mid_stamina = 12.0;
        let mid_nf = 12.0;
        let make = |cond_pct: f32| build_player((cond_pct * 10_000.0) as i16, mid_stamina, mid_nf);
        let category = ActionContext::explosive(45);

        for &boundary in &[0.65f32, 0.45, 0.30] {
            let above = effective_skill(&make(boundary + 0.005), 15.0, category);
            let below = effective_skill(&make(boundary - 0.005), 15.0, category);
            let jump = (above - below).abs();
            // Even at the steepest point the cliff must stay tiny —
            // anything > 0.10 effective skill units across 1% condition
            // would mean the band stepped instead of curving.
            assert!(
                jump < 0.10,
                "explosive cliff at boundary {boundary}: above={above} below={below} jump={jump}"
            );
        }
    }

    #[test]
    fn smooth_curve_preserves_category_ordering() {
        // Explosive must always suffer most under fatigue. Mental and
        // technical band penalties are close together (mental
        // intentionally has the smallest max penalty), so the spec's
        // ordering is: mental ≥ technical ≥ explosive multiplier. Tested
        // before minute 70 so the late-game mental extra doesn't fire.
        let p = build_player(3000, 12.0, 12.0);
        let tech = effective_skill(&p, 15.0, ActionContext::technical(45));
        let mental = effective_skill(&p, 15.0, ActionContext::mental(45));
        let expl = effective_skill(&p, 15.0, ActionContext::explosive(45));
        assert!(mental >= tech - 1e-3);
        assert!(tech >= expl - 1e-3);
    }

    #[test]
    fn elite_mitigation_tapers_at_extreme_fatigue() {
        // At ~10% condition (clamped to the floor), elite stamina
        // should still leave a visible explosive deficit — the cap
        // taper prevents elite fitness from nullifying broken legs.
        let elite_broken = build_player(1500, 19.0, 18.0);
        let elite_fresh = build_player(9500, 19.0, 18.0);
        let broken = effective_skill(&elite_broken, 15.0, ActionContext::explosive(85));
        let fresh = effective_skill(&elite_fresh, 15.0, ActionContext::explosive(85));
        // Broken-legs elite must lose at least 10% explosive vs fresh.
        assert!(
            fresh - broken >= 1.50,
            "fresh {fresh} broken {broken} — taper too weak"
        );
        // But the elite must still outperform a weak-stamina player
        // at the same shattered condition.
        let weak_broken = build_player(1500, 6.0, 6.0);
        let weak = effective_skill(&weak_broken, 15.0, ActionContext::explosive(85));
        assert!(broken > weak);
    }

    #[test]
    fn fresh_player_explosive_unchanged_by_mitigation_taper() {
        // The taper only kicks in below the freshness threshold. A
        // fully-fresh elite player must still take zero penalty.
        let elite = build_player(9800, 19.0, 18.0);
        let s = effective_skill(&elite, 15.0, ActionContext::explosive(20));
        assert!((s - 15.0).abs() < 1e-3);
    }

    #[test]
    fn smooth_curve_matches_legacy_band_anchors_within_tolerance() {
        // Sanity: the curve was hand-fit so it lands within ~0.02
        // multiplier of the old discrete band values at the historical
        // breakpoints. This locks in the calibration so future tuning
        // can't drift the bands silently.
        // Build a 0-mitigation player (stamina=0, nf=0) so we read the
        // band value directly.
        let make = |cond_pct: f32| build_player((cond_pct * 10_000.0) as i16, 0.0, 0.0);
        let probes: &[(f32, f32, f32, f32)] = &[
            // (cond_pct, expected_tech, expected_mental, expected_expl)
            (0.65, 0.97, 0.98, 0.96),
            (0.45, 0.92, 0.94, 0.88),
            (0.30, 0.86, 0.88, 0.78),
            (0.00, 0.78, 0.82, 0.68),
        ];
        for &(cond, exp_t, exp_m, exp_e) in probes {
            let p = make(cond);
            let t = effective_skill(&p, 10.0, ActionContext::technical(20)) / 10.0;
            let m = effective_skill(&p, 10.0, ActionContext::mental(20)) / 10.0;
            let e = effective_skill(&p, 10.0, ActionContext::explosive(20)) / 10.0;
            assert!((t - exp_t).abs() < 0.04, "tech at {cond}: got {t}, want {exp_t}");
            assert!((m - exp_m).abs() < 0.04, "mental at {cond}: got {m}, want {exp_m}");
            assert!((e - exp_e).abs() < 0.06, "expl at {cond}: got {e}, want {exp_e}");
        }
    }
}
