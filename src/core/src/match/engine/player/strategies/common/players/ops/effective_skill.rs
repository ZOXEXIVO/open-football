//! Fatigue-aware effective-skill helper.
//!
//! Real footballers don't just slow down when tired — their first touch
//! gets heavy, decisions get rushed, pressing arrives late, and explosive
//! actions (sprint, jump, dive) lose more than steady-state ones. This
//! helper takes a base skill value (1–20) and returns the *effective*
//! value after applying:
//!
//!   1. Match-condition multipliers, banded by category (technical /
//!      mental / explosive). Below 30% condition the explosive penalty
//!      reaches 32%, while technical only drops 22% and mental 18%.
//!   2. Stamina + natural_fitness mitigation: elite-fitness players
//!      recover up to 35% of the fatigue penalty.
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

/// Internal: fatigue-band multipliers per category. Returned values are
/// the **effective fraction** of the base skill (1.00 = no penalty).
fn band_multipliers(condition_pct: f32, category: SkillCategory) -> f32 {
    // condition_pct in [0.0, 1.0]
    let p = condition_pct.clamp(0.0, 1.0);
    if p >= 0.80 {
        return 1.00;
    }
    let (tech, mental, expl) = if p >= 0.65 {
        (0.97, 0.98, 0.96)
    } else if p >= 0.45 {
        (0.92, 0.94, 0.88)
    } else if p >= 0.30 {
        (0.86, 0.88, 0.78)
    } else {
        (0.78, 0.82, 0.68)
    };
    match category {
        SkillCategory::Technical => tech,
        SkillCategory::Mental => mental,
        SkillCategory::Explosive => expl,
    }
}

/// Compute the per-player fatigue-mitigation score in [0.0, 1.0]. Players
/// with elite stamina and natural_fitness recover up to ~35% of the
/// penalty; baseline players almost none.
fn mitigation_score(player: &MatchPlayer) -> f32 {
    let stamina = (player.skills.physical.stamina / 20.0).clamp(0.0, 1.0);
    let nat_fit = (player.skills.physical.natural_fitness / 20.0).clamp(0.0, 1.0);
    (stamina * 0.55 + nat_fit * 0.45).clamp(0.0, 1.0)
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
    // Mitigate the penalty by recovering up to 35% of the lost fraction.
    let mitigation = mitigation_score(player);
    let recovered = 1.0 - (1.0 - band) * (1.0 - mitigation * 0.35);
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
}
