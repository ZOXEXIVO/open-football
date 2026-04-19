use crate::club::staff::attributes::StaffCoaching;
use crate::club::staff::staff::CoachingStyle;
use crate::Staff;

// ─── PerceptionLens ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PerceptionLens {
    pub technical_weight: f32,
    pub mental_weight: f32,
    pub physical_weight: f32,
    pub attacking_focus: f32,
    pub creativity_focus: f32,
    pub physicality_focus: f32,
}

impl PerceptionLens {
    pub fn from_style_and_staff(style: &CoachingStyle, coaching: &StaffCoaching) -> Self {
        let (tech_w, mental_w, phys_w, atk_focus, creat_focus, phys_focus) = match style {
            CoachingStyle::Tactical         => (0.30, 0.45, 0.25, 0.5, 0.3, 0.4),
            CoachingStyle::Authoritarian    => (0.30, 0.40, 0.30, 0.4, 0.2, 0.7),
            CoachingStyle::Transformational => (0.40, 0.35, 0.25, 0.6, 0.8, 0.3),
            CoachingStyle::Democratic       => (0.35, 0.35, 0.30, 0.5, 0.5, 0.5),
            CoachingStyle::LaissezFaire     => (0.35, 0.30, 0.35, 0.6, 0.7, 0.6),
        };

        let atk_def_ratio = if coaching.attacking + coaching.defending > 0 {
            coaching.attacking as f32 / (coaching.attacking as f32 + coaching.defending as f32)
        } else {
            0.5
        };
        let atk_shift = (atk_def_ratio - 0.5) * 0.3;
        let tech_shift = (coaching.technical as f32 / 20.0 - 0.5) * 0.2;
        let phys_shift = (coaching.fitness as f32 / 20.0 - 0.5) * 0.2;

        PerceptionLens {
            technical_weight: tech_w,
            mental_weight: mental_w,
            physical_weight: phys_w,
            attacking_focus: (atk_focus + atk_shift).clamp(0.0, 1.0),
            creativity_focus: (creat_focus + tech_shift).clamp(0.0, 1.0),
            physicality_focus: (phys_focus + phys_shift).clamp(0.0, 1.0),
        }
    }
}

// ─── CoachProfile ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CoachProfile {
    pub judging_accuracy: f32,
    pub potential_accuracy: f32,
    pub risk_tolerance: f32,
    pub stubbornness: f32,
    pub trust_in_decisions: f32,
    pub youth_preference: f32,
    pub conservatism: f32,
    pub coach_seed: u32,
    pub perception_lens: PerceptionLens,
    pub confirmation_bias: f32,
    pub negativity_bias: f32,
    pub physical_bias_youth: f32,
    pub readiness_intuition: f32,
    pub attitude_weight: f32,
    pub tactical_blindness: f32,
    pub recency_bias: f32,
    pub emotional_volatility: f32,
}

impl CoachProfile {
    pub fn from_staff(staff: &Staff) -> Self {
        let mental = &staff.staff_attributes.mental;
        let knowledge = &staff.staff_attributes.knowledge;
        let coaching = &staff.staff_attributes.coaching;

        let style_adaptability_bonus = match staff.coaching_style {
            CoachingStyle::LaissezFaire => 0.1,
            CoachingStyle::Democratic => 0.05,
            CoachingStyle::Transformational => 0.05,
            CoachingStyle::Tactical => -0.05,
            CoachingStyle::Authoritarian => -0.1,
        };

        let style_youth_bonus = match staff.coaching_style {
            CoachingStyle::Transformational => 0.1,
            CoachingStyle::Democratic => 0.05,
            CoachingStyle::LaissezFaire => 0.0,
            CoachingStyle::Tactical => -0.05,
            CoachingStyle::Authoritarian => -0.05,
        };

        let adaptability_norm = mental.adaptability as f32 / 20.0;
        let determination_norm = mental.determination as f32 / 20.0;
        let discipline_norm = mental.discipline as f32 / 20.0;
        let man_management_norm = mental.man_management as f32 / 20.0;

        let conf_style_mod = match staff.coaching_style {
            CoachingStyle::Authoritarian => 0.1,
            CoachingStyle::Democratic => -0.05,
            _ => 0.0,
        };
        let confirmation_bias = (determination_norm * 0.4
            + (1.0 - adaptability_norm) * 0.6
            + conf_style_mod)
            .clamp(0.0, 1.0);

        let neg_style_mod = match staff.coaching_style {
            CoachingStyle::Authoritarian => 0.15,
            CoachingStyle::Transformational => -0.1,
            _ => 0.0,
        };
        let negativity_bias = (discipline_norm * 0.5
            + (1.0 - man_management_norm) * 0.5
            + neg_style_mod)
            .clamp(0.0, 1.0);

        let phys_style_base = match staff.coaching_style {
            CoachingStyle::Authoritarian => 0.7,
            CoachingStyle::Tactical => 0.3,
            CoachingStyle::Transformational => 0.2,
            CoachingStyle::Democratic => 0.4,
            CoachingStyle::LaissezFaire => 0.5,
        };
        let physical_bias_youth = (phys_style_base
            * (1.0 - coaching.working_with_youngsters as f32 / 20.0 * 0.5))
            .clamp(0.1, 0.9);

        let readiness_style_bonus = match staff.coaching_style {
            CoachingStyle::Tactical => 0.1,
            CoachingStyle::Authoritarian => 0.05,
            CoachingStyle::LaissezFaire => -0.1,
            _ => 0.0,
        };
        let readiness_intuition = (coaching.fitness as f32 / 20.0 * 0.5
            + coaching.mental as f32 / 20.0 * 0.3
            + knowledge.judging_player_ability as f32 / 20.0 * 0.2
            + readiness_style_bonus)
            .clamp(0.1, 1.0);

        let attitude_base = match staff.coaching_style {
            CoachingStyle::Authoritarian => 0.8,
            CoachingStyle::Tactical => 0.5,
            CoachingStyle::Democratic => 0.4,
            CoachingStyle::Transformational => 0.3,
            CoachingStyle::LaissezFaire => 0.2,
        };
        let attitude_weight = (attitude_base * (0.5 + discipline_norm * 0.5)).clamp(0.1, 0.9);

        let tact_style_mod = match staff.coaching_style {
            CoachingStyle::Tactical => -0.2,
            CoachingStyle::Authoritarian => 0.1,
            CoachingStyle::LaissezFaire => 0.15,
            _ => 0.0,
        };
        let tactical_blindness = (1.0 - coaching.tactical as f32 / 20.0 * 0.7
            + tact_style_mod)
            .clamp(0.0, 0.8);

        let recency_style_mod = match staff.coaching_style {
            CoachingStyle::LaissezFaire => 0.1,
            CoachingStyle::Tactical => -0.1,
            CoachingStyle::Authoritarian => 0.05,
            _ => 0.0,
        };
        let recency_bias = ((1.0 - knowledge.judging_player_ability as f32 / 20.0) * 0.5
            + (1.0 - man_management_norm) * 0.3
            + recency_style_mod)
            .clamp(0.1, 0.9);

        let vol_style_mod = match staff.coaching_style {
            CoachingStyle::Authoritarian => 0.15,
            CoachingStyle::Transformational => -0.1,
            CoachingStyle::Democratic => -0.05,
            _ => 0.0,
        };
        let emotional_volatility = ((1.0 - man_management_norm) * 0.4
            + determination_norm * 0.3
            + (1.0 - adaptability_norm) * 0.2
            + vol_style_mod)
            .clamp(0.1, 0.9);

        let perception_lens = PerceptionLens::from_style_and_staff(&staff.coaching_style, coaching);

        CoachProfile {
            judging_accuracy: (knowledge.judging_player_ability as f32 / 20.0).clamp(0.0, 1.0),
            potential_accuracy: (knowledge.judging_player_potential as f32 / 20.0).clamp(0.0, 1.0),
            risk_tolerance: (adaptability_norm + style_adaptability_bonus).clamp(0.0, 1.0),
            stubbornness: (determination_norm * 0.6 + discipline_norm * 0.4).clamp(0.0, 1.0),
            trust_in_decisions: determination_norm.clamp(0.0, 1.0),
            youth_preference: (coaching.working_with_youngsters as f32 / 20.0 + style_youth_bonus)
                .clamp(0.0, 1.0),
            conservatism: ((1.0 - adaptability_norm) * 0.6 + discipline_norm * 0.4).clamp(0.0, 1.0),
            coach_seed: staff.id,
            perception_lens,
            confirmation_bias,
            negativity_bias,
            physical_bias_youth,
            readiness_intuition,
            attitude_weight,
            tactical_blindness,
            recency_bias,
            emotional_volatility,
        }
    }

    /// Hash-based noise in [-1.0, 1.0]. Deterministic per coach+player+salt.
    pub fn perception_noise(&self, player_id: u32, salt: u32) -> f32 {
        super::utils::perception_noise_raw(self.coach_seed, player_id, salt)
    }
}
