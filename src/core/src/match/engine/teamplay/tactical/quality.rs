//! **Team quality → tactical correction.** The adjustments `refresh`
//! lays on top of the raw [`signals`](super::signals): a weak back line
//! drops its own line, a side that cannot press does not press hot, a
//! side that can genuinely play out buys recycle time, and an organised
//! leader damps its own panic.
//!
//! Every input here is a composite from
//! [`TeamSkillAggregates`](super::inputs::TeamSkillAggregates) of squads
//! already measured against the standard of the match — see
//! `MatchStandard::measure` — because the gates below are ABSOLUTE
//! thresholds.

use crate::r#match::engine::teamplay::tactical::team_state::TeamTacticalState;

impl TeamTacticalState {
    /// Skill-adjust the build-up patience by the team's actual passing
    /// execution composite. Above 0.65 we lift patience (the side can
    /// genuinely play through pressure); below 0.45 we drop it (we
    /// should look for direct outlets instead of holding the ball).
    pub(crate) fn skill_adjusted_build_up_patience(base: f32, build_up_quality: f32) -> f32 {
        let q = build_up_quality.clamp(0.0, 1.0);
        let delta = if q >= 0.65 {
            // 0.65 → 0, 1.00 → +0.15
            (q - 0.65) / 0.35 * 0.15
        } else if q <= 0.45 {
            // 0.45 → 0, 0.00 → -0.20
            -(0.45 - q) / 0.45 * 0.20
        } else {
            0.0
        };
        (base + delta).clamp(0.0, 1.0)
    }

    /// Drop the defensive line by up to 0.08 of pitch width when
    /// `defensive_quality` is poor. Returns absolute units to subtract
    /// from the home line / add to the away line.
    pub(crate) fn line_height_drop(defensive_quality: f32, field_width: f32) -> f32 {
        let q = defensive_quality.clamp(0.0, 1.0);
        if q >= 0.55 {
            return 0.0;
        }
        // 0.55 → 0, 0.00 → 0.08 of field_width.
        let factor = (0.55 - q) / 0.55 * 0.08;
        field_width * factor
    }

    /// Reduce press intensity by up to 35% when `press_quality` < 0.45.
    pub(crate) fn press_skill_adjustment(press_intensity: f32, press_quality: f32) -> f32 {
        let q = press_quality.clamp(0.0, 1.0);
        if q >= 0.45 {
            return press_intensity;
        }
        let deficit = (0.45 - q) / 0.45; // 0..1
        let mult = (1.0 - deficit * 0.35).max(0.65);
        (press_intensity * mult).clamp(0.0, 1.0)
    }

    /// Sweeper-keeper line lift: a high `gk_quality` lets the back
    /// line push higher because the keeper covers space behind. Capped
    /// at 0.02 of pitch width so this is a tweak, not a takeover.
    pub(crate) fn gk_line_lift(gk_quality: f32, field_width: f32) -> f32 {
        let q = gk_quality.clamp(0.0, 1.0);
        if q <= 0.55 {
            return 0.0;
        }
        // 0.55 → 0, 1.00 → 0.02 of field_width.
        ((q - 0.55) / 0.45 * 0.02).max(0.0) * field_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ──────────────────────────────────────────────────────────────────
    // Skill-composite-aware adjustments (line height, press
    // sustainability, build-up patience, lead protection damping).
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn line_height_drop_zero_when_strong_back_line() {
        // defensive_quality >= 0.55 means the back line is good enough
        // to hold a high line — no drop.
        let drop = TeamTacticalState::line_height_drop(0.70, 840.0);
        assert_eq!(drop, 0.0);
    }

    #[test]
    fn line_height_drop_grows_as_defense_weakens() {
        let mid = TeamTacticalState::line_height_drop(0.30, 840.0);
        let weak = TeamTacticalState::line_height_drop(0.10, 840.0);
        assert!(weak > mid);
        // Spec: max ~0.08 of pitch.
        let worst = TeamTacticalState::line_height_drop(0.0, 840.0);
        assert!(worst <= 840.0 * 0.0801);
        assert!(worst >= 840.0 * 0.07);
    }

    #[test]
    fn press_skill_adjustment_no_op_when_strong() {
        let pressed = TeamTacticalState::press_skill_adjustment(0.80, 0.75);
        assert!((pressed - 0.80).abs() < 1e-6);
    }

    #[test]
    fn press_skill_adjustment_drops_for_weak_side() {
        let strong = TeamTacticalState::press_skill_adjustment(0.80, 0.70);
        let weak = TeamTacticalState::press_skill_adjustment(0.80, 0.10);
        assert!(weak < strong);
        // Spec: at most 35% reduction.
        assert!(weak >= 0.80 * 0.65 - 1e-4);
    }

    #[test]
    fn skill_adjusted_build_up_patience_lifts_for_quality_sides() {
        let neutral = TeamTacticalState::skill_adjusted_build_up_patience(0.50, 0.50);
        let high = TeamTacticalState::skill_adjusted_build_up_patience(0.50, 0.90);
        let low = TeamTacticalState::skill_adjusted_build_up_patience(0.50, 0.20);
        assert_eq!(neutral, 0.50);
        assert!(high > neutral);
        assert!(low < neutral);
    }

    #[test]
    fn gk_line_lift_zero_when_average() {
        assert_eq!(TeamTacticalState::gk_line_lift(0.50, 840.0), 0.0);
    }

    #[test]
    fn gk_line_lift_caps_at_two_percent_width() {
        let lift = TeamTacticalState::gk_line_lift(1.0, 840.0);
        assert!(lift <= 840.0 * 0.0201);
        assert!(lift >= 840.0 * 0.019);
    }
}
