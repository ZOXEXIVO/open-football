//! Pressure-aware decision modifier — what we informally call "panic".
//!
//! When a player is under pressure their decision-making changes shape:
//! pass-decision time shrinks, long clearance becomes more attractive
//! for defenders/GK, miscontrol probability rises, risky central passes
//! drop sharply unless composure / vision are elite. The helper here
//! wraps that into a single `pressure_score` in [0.0, ~1.5] (clamped at
//! the call site) plus a panic-clear trigger for defensive thirds.

use crate::r#match::MatchPlayer;
use crate::r#match::engine::player::strategies::common::players::ops::effective_skill::{
    ActionContext, effective_skill,
};
use nalgebra::Vector3;

#[derive(Debug, Clone, Copy, Default)]
pub struct PressureContext {
    /// Distance to nearest opponent in pitch units.
    pub nearest_defender: f32,
    /// Whether a second defender is within 8 units.
    pub second_within_8u: bool,
    /// True when player is facing their own goal (back to opp goal).
    pub facing_own_goal: bool,
    /// True when player is squeezed against the touchline.
    pub touchline_trap: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct PressureResult {
    pub score: f32,
    pub miscontrol_bias: f32,
    pub long_clearance_bias: f32,
    pub safe_pass_bias: f32,
}

/// Compute the player's current pressure score and the derived biases.
pub fn pressure_score(player: &MatchPlayer, ctx: PressureContext, minute: u32) -> PressureResult {
    let mut s = 0.0;
    if ctx.nearest_defender <= 3.0 {
        s += 0.45;
    } else if ctx.nearest_defender <= 6.0 {
        s += 0.28;
    } else if ctx.nearest_defender <= 10.0 {
        s += 0.12;
    }
    if ctx.second_within_8u {
        s += 0.20;
    }
    if ctx.facing_own_goal {
        s += 0.12;
    }
    if ctx.touchline_trap {
        s += 0.10;
    }

    let mental_ctx = ActionContext::mental(minute);
    let composure = effective_skill(player, player.skills.mental.composure, mental_ctx) / 20.0;
    let cond = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
    s += (1.0 - composure) * 0.12;
    s += (1.0 - cond) * 0.08;

    let score = s.clamp(0.0, 1.5);
    PressureResult {
        score,
        miscontrol_bias: (score * 0.30).min(0.45),
        long_clearance_bias: (score * 0.55).min(0.85),
        safe_pass_bias: (score * 0.45).min(0.70),
    }
}

/// Decide whether a defender / GK should panic-clear. Triggers when the
/// player is in their own defensive third, pressure_score > 0.62, and
/// no safe pass with score > 0.55 exists. Returns the chosen direction
/// (toward touchline or wide channel).
pub fn should_panic_clear(
    pressure: &PressureResult,
    in_own_third: bool,
    best_safe_pass_score: f32,
) -> bool {
    in_own_third && pressure.score > 0.62 && best_safe_pass_score < 0.55
}

/// Pick a panic-clear direction: toward the nearer touchline (so the
/// ball goes out / into a channel rather than central where the
/// opponent can immediately counter). Returns a unit vector in the
/// pitch-XY plane.
pub fn panic_clear_direction(player_pos: Vector3<f32>, field_height: f32) -> Vector3<f32> {
    let mid_y = field_height * 0.5;
    let toward_touchline_y = if player_pos.y < mid_y { -1.0 } else { 1.0 };
    // Mostly y-axis push but with some forward bias so the ball at
    // least leaves the danger area rather than going behind the line.
    Vector3::new(0.4, toward_touchline_y, 0.0).normalize()
}

/// Quality of the resulting clearance, in [0.0, 1.0]. Mixes kicking /
/// passing / technique / decisions / condition. Used by callers to
/// decide how far / how accurate the clearance is.
pub fn clearance_quality(player: &MatchPlayer, minute: u32) -> f32 {
    let tech_ctx = ActionContext::technical(minute);
    let mental_ctx = ActionContext::mental(minute);
    let s = &player.skills;
    let kicking = effective_skill(player, s.goalkeeping.kicking.max(s.technical.passing), tech_ctx);
    let passing = effective_skill(player, s.technical.passing, tech_ctx);
    let technique = effective_skill(player, s.technical.technique, tech_ctx);
    let decisions = effective_skill(player, s.mental.decisions, mental_ctx);
    let cond = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);

    let blend = kicking * 0.30 + passing * 0.25 + technique * 0.20 + decisions * 0.15;
    let q = (blend / 20.0) * (0.85 + 0.15 * cond);
    q.clamp(0.0, 1.0)
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

    fn make(composure: f32, condition: i16) -> MatchPlayer {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = condition;
        let mut skills = PlayerSkills::default();
        skills.mental.composure = composure;
        skills.physical.stamina = 14.0;
        skills.physical.natural_fitness = 14.0;
        let player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::DefenderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &player, PlayerPositionType::DefenderCenter, false)
    }

    #[test]
    fn close_pressure_increases_score() {
        let p = make(12.0, 9000);
        let calm = pressure_score(
            &p,
            PressureContext {
                nearest_defender: 15.0,
                ..Default::default()
            },
            45,
        );
        let close = pressure_score(
            &p,
            PressureContext {
                nearest_defender: 2.5,
                second_within_8u: true,
                ..Default::default()
            },
            45,
        );
        assert!(close.score > calm.score + 0.4);
    }

    #[test]
    fn low_composure_raises_pressure() {
        let calm = make(18.0, 9000);
        let nervous = make(6.0, 9000);
        let ctx = PressureContext {
            nearest_defender: 4.0,
            ..Default::default()
        };
        let calm_score = pressure_score(&calm, ctx, 45).score;
        let nervous_score = pressure_score(&nervous, ctx, 45).score;
        assert!(nervous_score > calm_score);
    }

    #[test]
    fn panic_clear_triggers_in_own_third_under_pressure() {
        let p = make(8.0, 4000);
        let pr = pressure_score(
            &p,
            PressureContext {
                nearest_defender: 2.0,
                second_within_8u: true,
                facing_own_goal: true,
                touchline_trap: true,
            },
            85,
        );
        assert!(should_panic_clear(&pr, true, 0.4));
        assert!(!should_panic_clear(&pr, false, 0.4));
        assert!(!should_panic_clear(&pr, true, 0.7));
    }
}
