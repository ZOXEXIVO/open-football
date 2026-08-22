//! **Team quality → tactical correction.** The adjustments `refresh`
//! lays on top of the raw [`signals`](super::signals): a weak back line
//! drops its own line, a side that cannot press does not press hot, a
//! side that can genuinely play out buys recycle time, and an organised
//! leader damps its own panic.
//!
//! Every input here is a composite from
//! [`TeamSkillAggregates`](super::inputs::TeamSkillAggregates), and every
//! one of them is read through the divisional shift first — see
//! `MatchStandard` — because the gates below are ABSOLUTE thresholds.

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

    /// Damping factor for "panic" tactical shape changes. Returns a
    /// multiplier in [0.85, 1.05] — high concentration / teamwork
    /// damps the panic response (the leader holds shape better);
    /// poor organisation slightly amplifies it (the leader rushes
    /// clearances and gives the ball back). Spec: 0.85..1.05.
    pub fn protect_lead_damping(concentration_teamwork_avg: f32) -> f32 {
        let q = concentration_teamwork_avg.clamp(0.0, 1.0);
        if q >= 0.50 {
            // Above 0.50: scale down toward 0.85 at q=1.0.
            let damp = (q - 0.50) / 0.50 * 0.15;
            (1.0 - damp).clamp(0.85, 1.05)
        } else {
            // Below 0.50: very slight amplification — a poorly-organised
            // side that's leading can play wilder than they should.
            let amp = (0.50 - q) / 0.50 * 0.05;
            (1.0 + amp).clamp(0.85, 1.05)
        }
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

    /// Bias delta when chasing a result. Sides with the actual
    /// attacking quality to convert get a small tempo / risk lift —
    /// poor sides chasing late don't, since rushing wouldn't help
    /// them. Spec: bounded to ±0.05.
    pub(crate) fn attacking_chase_lift(attacking_quality: f32, minute: f32) -> f32 {
        // Only bites in the back half of the match (chasing late, not
        // hunting in minute 12).
        let late = ((minute - 60.0).max(0.0) / 30.0).clamp(0.0, 1.0);
        // Skill gate: only sides above 0.55 attacking_quality see the
        // lift; weaker sides stay disciplined.
        let q = attacking_quality.clamp(0.0, 1.0);
        if q <= 0.55 {
            return 0.0;
        }
        // 0.55 → 0; 1.00 → 0.05 max at minute 90.
        let raw = (q - 0.55) / 0.45 * 0.05;
        (raw * late).clamp(0.0, 0.05)
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
    fn protect_lead_damping_in_band() {
        // Spec range 0.85..1.05.
        assert!((TeamTacticalState::protect_lead_damping(0.50) - 1.0).abs() < 1e-6);
        // Elite organisation: max 15% damp.
        let elite = TeamTacticalState::protect_lead_damping(1.0);
        assert!((elite - 0.85).abs() < 1e-6);
        // Poor organisation: slight amplification.
        let poor = TeamTacticalState::protect_lead_damping(0.0);
        assert!((poor - 1.05).abs() < 1e-6);
        // Always inside [0.85, 1.05].
        for q_int in 0..=20 {
            let q = q_int as f32 / 20.0;
            let v = TeamTacticalState::protect_lead_damping(q);
            assert!(v >= 0.85 - 1e-6 && v <= 1.05 + 1e-6, "q={q} v={v}");
        }
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

    #[test]
    fn attacking_chase_lift_zero_for_weak_side() {
        // Weak attack chasing late shouldn't get a free pass.
        assert_eq!(TeamTacticalState::attacking_chase_lift(0.40, 85.0), 0.0);
    }

    #[test]
    fn attacking_chase_lift_grows_late_for_strong_side() {
        let early = TeamTacticalState::attacking_chase_lift(0.85, 30.0);
        let late = TeamTacticalState::attacking_chase_lift(0.85, 88.0);
        assert!(late > early);
        // Spec cap: bounded to 0.05.
        assert!(late <= 0.0501);
    }
}
