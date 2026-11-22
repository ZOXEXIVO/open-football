use crate::Staff;
use crate::club::staff::CoachingStyle;
use crate::club::staff::StaffCoaching;
use crate::club::staff::StaffKnowledge;
use crate::club::staff::StaffMental;

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
            CoachingStyle::Tactical => (0.30, 0.45, 0.25, 0.5, 0.3, 0.4),
            CoachingStyle::Authoritarian => (0.30, 0.40, 0.30, 0.4, 0.2, 0.7),
            CoachingStyle::Transformational => (0.40, 0.35, 0.25, 0.6, 0.8, 0.3),
            CoachingStyle::Democratic => (0.35, 0.35, 0.30, 0.5, 0.5, 0.5),
            CoachingStyle::LaissezFaire => (0.35, 0.30, 0.35, 0.6, 0.7, 0.6),
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

/// How a coach decides, read off his mind and his style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecisionTemperament {
    pub risk_tolerance: f32,
    pub stubbornness: f32,
    pub conservatism: f32,
    pub emotional_volatility: f32,
}

#[derive(Debug, Clone)]
pub struct CoachProfile {
    pub judging_accuracy: f32,
    pub potential_accuracy: f32,
    pub risk_tolerance: f32,
    pub stubbornness: f32,
    pub trust_in_decisions: f32,
    pub youth_preference: f32,
    pub conservatism: f32,
    /// Man-management / motivation, normalised 0..1. A blend of the staff's
    /// `man_management` and `motivating` attributes. A high value coach
    /// integrates fringe / young players more readily — they trust him to
    /// keep the dressing room settled while a kid finds his feet. Feeds the
    /// future-aware youth-pathway factor.
    pub man_management: f32,
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
    /// A lens that distorts nothing, for a state not yet bound to a
    /// coach. Never used to judge anybody — [`CoachDecisionState::bind`]
    /// replaces it with the real man's lens before the first read.
    pub fn neutral() -> Self {
        CoachProfile {
            judging_accuracy: 0.5,
            potential_accuracy: 0.5,
            risk_tolerance: 0.5,
            stubbornness: 0.5,
            trust_in_decisions: 0.5,
            youth_preference: 0.5,
            conservatism: 0.5,
            man_management: 0.5,
            coach_seed: 0,
            perception_lens: PerceptionLens {
                technical_weight: 0.35,
                mental_weight: 0.35,
                physical_weight: 0.30,
                attacking_focus: 0.5,
                creativity_focus: 0.5,
                physicality_focus: 0.5,
            },
            confirmation_bias: 0.5,
            negativity_bias: 0.5,
            physical_bias_youth: 0.5,
            readiness_intuition: 0.5,
            attitude_weight: 0.5,
            tactical_blindness: 0.5,
            recency_bias: 0.5,
            emotional_volatility: 0.5,
        }
    }

    pub fn from_staff(staff: &Staff) -> Self {
        let mental = &staff.staff_attributes.mental;
        let knowledge = &staff.staff_attributes.knowledge;
        let coaching = &staff.staff_attributes.coaching;
        let style = staff.coaching_style;

        let adaptability_norm = mental.adaptability as f32 / 20.0;
        let determination_norm = mental.determination as f32 / 20.0;
        let discipline_norm = mental.discipline as f32 / 20.0;
        let man_management_norm = mental.man_management as f32 / 20.0;

        let confirmation_bias =
            Self::confirmation_bias(determination_norm, adaptability_norm, style);
        let negativity_bias = Self::negativity_bias(discipline_norm, man_management_norm, style);
        let physical_bias_youth = Self::physical_bias_youth(coaching, style);
        let readiness_intuition = Self::readiness_intuition(coaching, knowledge, style);
        let attitude_weight = Self::attitude_weight(style, discipline_norm);
        let tactical_blindness = Self::tactical_blindness(coaching, style);
        let recency_bias = Self::recency_bias(knowledge, man_management_norm, style);

        let temperament = Self::temperament_of(mental, style);
        let perception_lens = PerceptionLens::from_style_and_staff(&style, coaching);

        CoachProfile {
            judging_accuracy: (knowledge.judging_player_ability as f32 / 20.0).clamp(0.0, 1.0),
            potential_accuracy: (knowledge.judging_player_potential as f32 / 20.0).clamp(0.0, 1.0),
            risk_tolerance: temperament.risk_tolerance,
            stubbornness: temperament.stubbornness,
            trust_in_decisions: determination_norm.clamp(0.0, 1.0),
            youth_preference: (coaching.working_with_youngsters as f32 / 20.0
                + Self::style_youth_bonus(style))
            .clamp(0.0, 1.0),
            conservatism: temperament.conservatism,
            man_management: (man_management_norm * 0.6 + mental.motivating as f32 / 20.0 * 0.4)
                .clamp(0.0, 1.0),
            coach_seed: staff.id,
            perception_lens,
            confirmation_bias,
            negativity_bias,
            physical_bias_youth,
            readiness_intuition,
            attitude_weight,
            tactical_blindness,
            recency_bias,
            emotional_volatility: temperament.emotional_volatility,
        }
    }

    /// How much a style adds to the preference for working with young
    /// players.
    fn style_youth_bonus(style: CoachingStyle) -> f32 {
        match style {
            CoachingStyle::Transformational => 0.1,
            CoachingStyle::Democratic => 0.05,
            CoachingStyle::LaissezFaire => 0.0,
            CoachingStyle::Tactical => -0.05,
            CoachingStyle::Authoritarian => -0.05,
        }
    }

    /// How much a coach reads new evidence as confirmation of what he
    /// already believed.
    fn confirmation_bias(
        determination_norm: f32,
        adaptability_norm: f32,
        style: CoachingStyle,
    ) -> f32 {
        let style_mod = match style {
            CoachingStyle::Authoritarian => 0.1,
            CoachingStyle::Democratic => -0.05,
            _ => 0.0,
        };
        (determination_norm * 0.4 + (1.0 - adaptability_norm) * 0.6 + style_mod).clamp(0.0, 1.0)
    }

    /// How heavily bad news weighs against good news.
    fn negativity_bias(
        discipline_norm: f32,
        man_management_norm: f32,
        style: CoachingStyle,
    ) -> f32 {
        let style_mod = match style {
            CoachingStyle::Authoritarian => 0.15,
            CoachingStyle::Transformational => -0.1,
            _ => 0.0,
        };
        (discipline_norm * 0.5 + (1.0 - man_management_norm) * 0.5 + style_mod).clamp(0.0, 1.0)
    }

    /// How strongly a coach judges young players through their body
    /// rather than through what they can already do.
    fn physical_bias_youth(coaching: &StaffCoaching, style: CoachingStyle) -> f32 {
        let style_base = match style {
            CoachingStyle::Authoritarian => 0.7,
            CoachingStyle::Tactical => 0.3,
            CoachingStyle::Transformational => 0.2,
            CoachingStyle::Democratic => 0.4,
            CoachingStyle::LaissezFaire => 0.5,
        };
        (style_base * (1.0 - coaching.working_with_youngsters as f32 / 20.0 * 0.5)).clamp(0.1, 0.9)
    }

    /// How well a coach can tell whether a player is ready today.
    fn readiness_intuition(
        coaching: &StaffCoaching,
        knowledge: &StaffKnowledge,
        style: CoachingStyle,
    ) -> f32 {
        let style_bonus = match style {
            CoachingStyle::Tactical => 0.1,
            CoachingStyle::Authoritarian => 0.05,
            CoachingStyle::LaissezFaire => -0.1,
            _ => 0.0,
        };
        (coaching.fitness as f32 / 20.0 * 0.5
            + coaching.mental as f32 / 20.0 * 0.3
            + knowledge.judging_player_ability as f32 / 20.0 * 0.2
            + style_bonus)
            .clamp(0.1, 1.0)
    }

    /// How much weight a coach gives a player's attitude.
    fn attitude_weight(style: CoachingStyle, discipline_norm: f32) -> f32 {
        let base = match style {
            CoachingStyle::Authoritarian => 0.8,
            CoachingStyle::Tactical => 0.5,
            CoachingStyle::Democratic => 0.4,
            CoachingStyle::Transformational => 0.3,
            CoachingStyle::LaissezFaire => 0.2,
        };
        (base * (0.5 + discipline_norm * 0.5)).clamp(0.1, 0.9)
    }

    /// How much of the tactical picture a coach misses.
    fn tactical_blindness(coaching: &StaffCoaching, style: CoachingStyle) -> f32 {
        let style_mod = match style {
            CoachingStyle::Tactical => -0.2,
            CoachingStyle::Authoritarian => 0.1,
            CoachingStyle::LaissezFaire => 0.15,
            _ => 0.0,
        };
        (1.0 - coaching.tactical as f32 / 20.0 * 0.7 + style_mod).clamp(0.0, 0.8)
    }

    /// How much the most recent evidence outweighs the rest.
    fn recency_bias(
        knowledge: &StaffKnowledge,
        man_management_norm: f32,
        style: CoachingStyle,
    ) -> f32 {
        let style_mod = match style {
            CoachingStyle::LaissezFaire => 0.1,
            CoachingStyle::Tactical => -0.1,
            CoachingStyle::Authoritarian => 0.05,
            _ => 0.0,
        };
        ((1.0 - knowledge.judging_player_ability as f32 / 20.0) * 0.5
            + (1.0 - man_management_norm) * 0.3
            + style_mod)
            .clamp(0.1, 0.9)
    }

    /// How a coach of this mind and this style decides: the four readings
    /// the profile and the match's coach card share.
    pub fn temperament_of(mental: &StaffMental, style: CoachingStyle) -> DecisionTemperament {
        let adaptability = mental.adaptability as f32 / 20.0;
        let determination = mental.determination as f32 / 20.0;
        let discipline = mental.discipline as f32 / 20.0;
        let man_management = mental.man_management as f32 / 20.0;
        let style_adaptability_bonus = match style {
            CoachingStyle::LaissezFaire => 0.1,
            CoachingStyle::Democratic => 0.05,
            CoachingStyle::Transformational => 0.05,
            CoachingStyle::Tactical => -0.05,
            CoachingStyle::Authoritarian => -0.1,
        };
        let vol_style_mod = match style {
            CoachingStyle::Authoritarian => 0.15,
            CoachingStyle::Transformational => -0.1,
            CoachingStyle::Democratic => -0.05,
            _ => 0.0,
        };
        DecisionTemperament {
            risk_tolerance: (adaptability + style_adaptability_bonus).clamp(0.0, 1.0),
            stubbornness: (determination * 0.6 + discipline * 0.4).clamp(0.0, 1.0),
            conservatism: ((1.0 - adaptability) * 0.6 + discipline * 0.4).clamp(0.0, 1.0),
            emotional_volatility: ((1.0 - man_management) * 0.4
                + determination * 0.3
                + (1.0 - adaptability) * 0.2
                + vol_style_mod)
                .clamp(0.1, 0.9),
        }
    }

    /// Hash-based noise in [-1.0, 1.0]. Deterministic per coach+player+salt.
    pub fn perception_noise(&self, player_id: u32, salt: u32) -> f32 {
        super::noise::PerceptionNoise::raw(self.coach_seed, player_id, salt)
    }

    // ── Coach-decision reaction weights ────────────────────────────────
    //
    // The persistent coach memory / coach decision engine reads these
    // helpers to size *how strongly* the coach reacts to evidence. The
    // formulas live here so the engine reads as orchestration and the
    // tuning surface stays in one place.

    /// How strongly the coach reacts to recent form evidence (0..1+).
    /// A negativity-biased and recency-biased coach reacts harder; a
    /// stubborn / high-judging coach reacts less.
    ///
    /// Used by [`AssessmentMath`] to size the form-pressure adjustment.
    pub fn form_reaction_weight(&self) -> f32 {
        let recency = self.recency_bias;
        let negativity = self.negativity_bias;
        let stubborn_damp = (1.0 - self.stubbornness * 0.4).clamp(0.6, 1.0);
        let acc_damp = (1.0 - self.judging_accuracy * 0.3).clamp(0.7, 1.0);
        ((recency * 0.5 + negativity * 0.4 + 0.3) * stubborn_damp * acc_damp).clamp(0.2, 1.4)
    }

    /// How strongly the coach reacts to trust-delta evidence (0..1+).
    /// Authoritarian / negativity-biased coaches react harder.
    pub fn trust_reaction_weight(&self) -> f32 {
        ((self.attitude_weight + self.negativity_bias * 0.5 + 0.3) * 0.8).clamp(0.3, 1.3)
    }

    /// How patient the coach is with development minutes (0..1+).
    /// High youth_preference + high man_management + transformational
    /// style → patient. Conservative + low man_management → impatient.
    pub fn development_patience(&self) -> f32 {
        ((self.youth_preference * 0.4 + self.man_management * 0.4 + 0.2) * 1.05).clamp(0.2, 1.4)
    }

    /// Multiplier applied to single-bad-game pressure (0..1, closer to
    /// 0 = stronger dampening). A high-judging-accuracy, high-man-
    /// management coach sees through one bad game; a high-recency,
    /// high-negativity coach lets it through unchecked.
    /// A manager of ordinary habits hands the gloves back for the final.
    pub const NEUTRAL_CUP_REACH: f32 = 0.85;

    /// How far into a cup he lets his deputy keep the gloves, as a share of
    /// the bracket: 1.0 is the final. A manager who manages men and rotates
    /// lets him finish what he started; a conservative one takes the shirt
    /// back for the late rounds.
    pub fn deputy_cup_reach(&self) -> f32 {
        (0.45 + 0.75 * (0.6 * self.man_management + 0.4 * self.rotation_tendency())).clamp(0.0, 1.0)
    }

    /// How readily he rotates, 0..1: less the more conservative he is and
    /// the better he judges who his best players are.
    pub fn rotation_tendency(&self) -> f32 {
        (0.8 - self.conservatism * 0.5 - self.judging_accuracy * 0.3).clamp(0.0, 1.0)
    }

    pub fn one_bad_game_dampener(&self) -> f32 {
        let damp = self.judging_accuracy * 0.4
            + self.man_management * 0.3
            + (1.0 - self.recency_bias) * 0.2
            + (1.0 - self.emotional_volatility) * 0.1;
        // Map 0..1 onto a tightening dampener: high damp → small mult.
        (1.0 - damp * 0.7).clamp(0.3, 1.0)
    }
}
