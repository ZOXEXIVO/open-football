use crate::{Player, Staff, TeamType};
use crate::club::staff::staff::CoachingStyle;
use crate::utils::DateUtils;
use chrono::NaiveDate;
use std::collections::HashMap;

// ─── RecentMove ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecentMoveType {
    DemotedToReserves,
    RecalledFromReserves,
    PromotedToFirst,
    YouthPromoted,
    SwappedIn,
    SwappedOut,
}

#[derive(Debug, Clone, Copy)]
pub struct RecentMove {
    pub move_type: RecentMoveType,
    pub week: u32,
}

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
    pub fn from_style_and_staff(style: &CoachingStyle, coaching: &crate::club::staff::attributes::StaffCoaching) -> Self {
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

// ─── PlayerBias ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PlayerBias {
    pub quality_offset: f32,
    pub visibility: f32,
    pub sunk_cost: f32,
    pub first_impression: f32,
    pub anchored: bool,
    pub disappointments: u8,
    pub perception_drift: f32,
    pub last_observation_week: u32,
    pub overreaction_timer: u8,
    pub overreaction_magnitude: f32,
}

impl Default for PlayerBias {
    fn default() -> Self {
        PlayerBias {
            quality_offset: 0.0,
            visibility: 1.0,
            sunk_cost: 0.0,
            first_impression: 0.0,
            anchored: false,
            disappointments: 0,
            perception_drift: 0.0,
            last_observation_week: 0,
            overreaction_timer: 0,
            overreaction_magnitude: 0.0,
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

        // Readiness intuition: fitness coaches read match readiness better
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

        // Attitude weight: how much visible effort biases quality perception
        let attitude_base = match staff.coaching_style {
            CoachingStyle::Authoritarian => 0.8,
            CoachingStyle::Tactical => 0.5,
            CoachingStyle::Democratic => 0.4,
            CoachingStyle::Transformational => 0.3,
            CoachingStyle::LaissezFaire => 0.2,
        };
        let attitude_weight = (attitude_base * (0.5 + discipline_norm * 0.5)).clamp(0.1, 0.9);

        // Tactical blindness: how much position value is misread
        let tact_style_mod = match staff.coaching_style {
            CoachingStyle::Tactical => -0.2,
            CoachingStyle::Authoritarian => 0.1,
            CoachingStyle::LaissezFaire => 0.15,
            _ => 0.0,
        };
        let tactical_blindness = (1.0 - coaching.tactical as f32 / 20.0 * 0.7
            + tact_style_mod)
            .clamp(0.0, 0.8);

        // Recency bias: how much recent form dominates long-term view
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

        // Emotional volatility: how strongly coach overreacts to events
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
        let hash = self
            .coach_seed
            .wrapping_mul(2654435761)
            .wrapping_add(player_id.wrapping_mul(2246822519))
            .wrapping_add(salt.wrapping_mul(3266489917));
        let hash = hash ^ (hash >> 16);
        let hash = hash.wrapping_mul(0x45d9f3b);
        let hash = hash ^ (hash >> 16);
        (hash & 0xFFFF) as f32 / 32768.0 - 1.0
    }
}

// ─── PlayerImpression ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PlayerImpression {
    pub player_id: u32,
    pub perceived_quality: f32,
    pub match_readiness: f32,
    pub coach_trust: f32,
    pub potential_impression: f32,
    pub training_impression: f32,
    pub last_updated: NaiveDate,
    pub weeks_in_squad: u16,
    pub recent_move: Option<RecentMove>,
    pub bias: PlayerBias,
    pub prev_red_cards: u8,
    pub prev_goals: u16,
    pub prev_avg_rating: f32,
}

impl PlayerImpression {
    pub fn new(player_id: u32, date: NaiveDate) -> Self {
        PlayerImpression {
            player_id,
            perceived_quality: 0.0,
            match_readiness: 0.0,
            coach_trust: 5.0,
            potential_impression: 0.0,
            training_impression: 0.0,
            last_updated: date,
            weeks_in_squad: 0,
            recent_move: None,
            bias: PlayerBias::default(),
            prev_red_cards: 0,
            prev_goals: 0,
            prev_avg_rating: 0.0,
        }
    }
}

// ─── CoachDecisionState ──────────────────────────────────────────────

#[derive(Debug)]
pub struct CoachDecisionState {
    pub profile: CoachProfile,
    pub impressions: HashMap<u32, PlayerImpression>,
    pub coach_id: u32,
    pub current_week: u32,
    pub squad_satisfaction: f32,
    pub weeks_since_last_change: u32,
    pub trigger_pressure: f32,
    pub emotional_heat: f32,
}

impl CoachDecisionState {
    pub fn new(staff: &Staff, date: NaiveDate) -> Self {
        CoachDecisionState {
            profile: CoachProfile::from_staff(staff),
            impressions: HashMap::new(),
            coach_id: staff.id,
            current_week: date_to_week(date),
            squad_satisfaction: 0.5,
            weeks_since_last_change: 0,
            trigger_pressure: 0.0,
            emotional_heat: 0.0,
        }
    }

    /// Lens-weighted skill composite using the coach's perception lens
    fn lens_skill_composite(&self, player: &Player) -> f32 {
        let lens = &self.profile.perception_lens;
        let t = &player.skills.technical;
        let m = &player.skills.mental;
        let p = &player.skills.physical;

        let atk_tech = (t.finishing + t.dribbling + t.crossing + t.first_touch + t.technique + t.long_shots) / 6.0;
        let def_tech = (t.tackling + t.marking + t.heading + t.passing) / 4.0;
        let tech_score = atk_tech * lens.attacking_focus + def_tech * (1.0 - lens.attacking_focus);

        let creative_mental = (m.flair + m.vision + m.composure + m.decisions + m.anticipation) / 5.0;
        let discipline_mental = (m.work_rate + m.determination + m.positioning + m.teamwork + m.concentration) / 5.0;
        let mental_score = creative_mental * lens.creativity_focus + discipline_mental * (1.0 - lens.creativity_focus);

        let explosive = (p.pace + p.acceleration + p.strength + p.jumping) / 4.0;
        let endurance = (p.stamina + p.natural_fitness + p.agility + p.balance) / 4.0;
        let physical_score = explosive * lens.physicality_focus + endurance * (1.0 - lens.physicality_focus);

        tech_score * lens.technical_weight + mental_score * lens.mental_weight + physical_score * lens.physical_weight
    }

    /// Initialize bias for a new player encounter
    fn init_bias(&self, player_id: u32) -> PlayerBias {
        // Quality offset hash
        let hash = self.profile.coach_seed
            .wrapping_mul(2654435761)
            .wrapping_add(player_id.wrapping_mul(2246822519))
            .wrapping_add(0xB1A5u32.wrapping_mul(3266489917));
        let hash = hash ^ (hash >> 16);
        let hash = hash.wrapping_mul(0x45d9f3b);
        let hash = hash ^ (hash >> 16);
        let raw = (hash & 0xFFFF) as f32 / 32768.0 - 1.0;
        let magnitude = (1.0 - self.profile.judging_accuracy) * 2.0;
        let quality_offset = raw * magnitude;

        // Perception drift hash (different seed ordering)
        let drift_hash = self.profile.coach_seed
            .wrapping_mul(3266489917)
            .wrapping_add(player_id.wrapping_mul(2654435761))
            .wrapping_add(0xDF71u32.wrapping_mul(2246822519));
        let drift_hash = drift_hash ^ (drift_hash >> 16);
        let drift_hash = drift_hash.wrapping_mul(0x45d9f3b);
        let drift_hash = drift_hash ^ (drift_hash >> 16);
        let drift_raw = (drift_hash & 0xFFFF) as f32 / 32768.0 - 1.0;
        let perception_drift = drift_raw * (1.0 - self.profile.judging_accuracy) * 1.5;

        PlayerBias {
            quality_offset,
            perception_drift,
            visibility: 1.0,
            sunk_cost: 0.0,
            first_impression: 0.0,
            anchored: false,
            disappointments: 0,
            last_observation_week: 0,
            overreaction_timer: 0,
            overreaction_magnitude: 0.0,
        }
    }

    /// Compute visibility for a player based on team type and context
    fn compute_visibility(&self, player: &Player, team_type: &TeamType) -> f32 {
        let base = match team_type {
            TeamType::Main => 0.85,
            TeamType::B | TeamType::U23 | TeamType::U21 => {
                0.5 + self.profile.youth_preference * 0.15
            }
            TeamType::U18 | TeamType::U19 => {
                0.2 + self.profile.youth_preference * 0.3
            }
        };

        let recency_bonus = if player.player_attributes.days_since_last_match <= 7 {
            0.15
        } else {
            0.0
        };

        let weeks_in = self.impressions.get(&player.id)
            .map(|imp| imp.weeks_in_squad)
            .unwrap_or(0);
        let familiarity_bonus = (weeks_in as f32 / 52.0).min(0.15);

        (base + recency_bonus + familiarity_bonus).clamp(0.1, 1.0)
    }

    /// Perceived quality: lens-weighted, bias-shifted, sticky noise, attitude bleed
    pub fn perceived_quality(&self, player: &Player, date: NaiveDate) -> f32 {
        let week = date_to_week(date);

        // Get bias info if available
        let (quality_offset, visibility, first_imp, anchored, perception_drift,
             overreaction_mag, overreaction_active) = self.impressions.get(&player.id)
            .map(|imp| (
                imp.bias.quality_offset, imp.bias.visibility, imp.bias.first_impression,
                imp.bias.anchored, imp.bias.perception_drift, imp.bias.overreaction_magnitude,
                imp.bias.overreaction_timer > 0,
            ))
            .unwrap_or((0.0, 1.0, 0.0, false, 0.0, 0.0, false));

        // Noise: sticky drift + small weekly jitter (NOT memoryless)
        let noise_scale = 1.0 - self.profile.judging_accuracy;
        let jitter = self.profile.perception_noise(player.id, week) * noise_scale * 0.8;
        let visibility_amplifier = (1.0 / visibility.max(0.1)).min(2.5);
        let noise = (perception_drift + jitter) * visibility_amplifier;

        // Lens-weighted skill composite
        let skill_composite = self.lens_skill_composite(player);

        // Position mastery (dampened by tactical blindness)
        let position_level = player
            .positions
            .positions
            .iter()
            .map(|p| p.level)
            .max()
            .unwrap_or(0) as f32;
        let position_contribution = position_level * (1.0 - self.profile.tactical_blindness * 0.5);

        let base = skill_composite * 0.75 + position_contribution * 0.25;

        // Bias offset
        let biased_base = base + quality_offset;

        // Form bonus (amplified by recency bias — some coaches live by recent results)
        let raw_form_bonus = if player.statistics.played + player.statistics.played_subs > 3 {
            (player.statistics.average_rating - 6.5).clamp(-1.5, 1.5)
        } else {
            0.0
        };
        let form_bonus = raw_form_bonus * (1.0 + self.profile.recency_bias * 0.8);

        // Attitude bleed: visible effort bleeds into quality perception
        let attitude_bleed = {
            let visible_effort = (player.skills.mental.work_rate
                + player.skills.mental.determination) / 2.0;
            (visible_effort - 10.0) * self.profile.attitude_weight * 0.15
        };

        // Reputation bias
        let rep_bias = (player.player_attributes.world_reputation as f32 / 10000.0).clamp(0.0, 0.5);

        // Anchoring pull for stubborn coaches
        let anchor_pull = if anchored {
            let observation = biased_base + form_bonus + rep_bias + noise + attitude_bleed;
            (first_imp - observation) * self.profile.stubbornness * 0.15
        } else {
            0.0
        };

        // Overreaction distortion during emotional episodes
        let overreaction_effect = if overreaction_active {
            overreaction_mag * 0.5
        } else {
            0.0
        };

        // Condition factor
        let condition = (player.player_attributes.condition_percentage() as f32 / 100.0).clamp(0.5, 1.0);

        (biased_base + form_bonus + rep_bias + noise + anchor_pull + attitude_bleed + overreaction_effect) * condition
    }

    /// Match readiness: now coach-specific — low intuition coaches misread fitness
    pub fn match_readiness(&self, player: &Player) -> f32 {
        if player.player_attributes.is_injured {
            return 0.0;
        }
        if player.player_attributes.is_banned {
            return 0.0;
        }

        let condition = player.player_attributes.condition_percentage() as f32 / 100.0;
        let fitness = player.player_attributes.fitness as f32 / 10000.0;

        let days_since = player.player_attributes.days_since_last_match as f32;
        let sharpness = if days_since <= 3.0 {
            1.0
        } else if days_since <= 7.0 {
            0.95
        } else if days_since <= 14.0 {
            0.85
        } else if days_since <= 28.0 {
            0.70
        } else {
            0.55
        };

        let physical_readiness = player.skills.physical.match_readiness / 20.0;

        let raw_readiness = (condition * 0.35 + fitness.clamp(0.0, 1.0) * 0.25
            + sharpness * 0.25 + physical_readiness * 0.15)
            .clamp(0.0, 1.0);

        // Coach's reading of readiness is imperfect
        let intuition = self.profile.readiness_intuition;
        let noise_scale = (1.0 - intuition) * 0.25;
        let noise = self.profile.perception_noise(player.id,
            self.current_week.wrapping_add(0xFE57)) * noise_scale;

        // Low-intuition coaches overweight visible physicality ("he looks strong, he's ready")
        let physicality_illusion = if intuition < 0.5 {
            let physical_look = (player.skills.physical.strength + player.skills.physical.pace) / 40.0;
            (physical_look - 0.5) * (0.5 - intuition) * 0.3
        } else {
            0.0
        };

        ((raw_readiness + noise + physicality_illusion).clamp(0.0, 1.0)) * 20.0
    }

    /// Youth potential impression with physical bias and spotlight moments
    pub fn potential_impression(&self, player: &Player, date: NaiveDate) -> f32 {
        let quality = self.perceived_quality(player, date);
        let age = DateUtils::age(player.birth_date, date);
        let week = date_to_week(date);

        let age_bonus = match age {
            0..=15 => 1.5,
            16..=17 => 2.5,
            18 => 3.0,
            19..=20 => 2.0,
            21..=22 => 1.0,
            _ => 0.0,
        };

        let attitude =
            (player.attributes.professionalism + player.skills.mental.determination) / 2.0;
        let attitude_bonus = (attitude - 10.0).clamp(-1.0, 2.0) * 0.5;

        let noise_scale = 1.0 - self.profile.potential_accuracy;
        let noise = self.profile.perception_noise(player.id, week.wrapping_add(7919)) * noise_scale * 3.0;

        let youth_bias = self.profile.youth_preference * 1.5;

        // Height bonus: tall = "looks like a player" to physically-biased coaches
        let height_bonus = if player.player_attributes.height >= 185 {
            self.profile.physical_bias_youth * 1.5
        } else if player.player_attributes.height <= 170 {
            -self.profile.physical_bias_youth * 0.8
        } else {
            0.0
        };

        // Pace/strength bonus for physical coaches
        let pace_bonus = ((player.skills.physical.pace - 12.0).max(0.0) / 8.0)
            * self.profile.physical_bias_youth * 1.5;

        // Hidden traits only visible proportional to potential_accuracy
        let hidden_quality = (player.skills.mental.composure
            + player.skills.mental.decisions
            + player.skills.mental.anticipation
            + player.skills.technical.first_touch
            + player.skills.technical.technique) / 5.0;
        let hidden_perception = hidden_quality * self.profile.potential_accuracy;
        let hidden_bonus = (hidden_perception - quality * 0.3) * 0.3;

        // Spotlight moment: random chance coach suddenly notices this youth
        let spotlight_hash = self.profile.coach_seed
            .wrapping_mul(player.id)
            .wrapping_add(week.wrapping_mul(13))
            .wrapping_add(0xCC01);
        let spotlight_hash = spotlight_hash ^ (spotlight_hash >> 16);
        let spotlight_hash = spotlight_hash.wrapping_mul(0x45d9f3b);
        let spotlight_roll = (spotlight_hash & 0xFFFF) as f32 / 65536.0;
        let spotlight_chance = 0.05 + self.profile.youth_preference * 0.10;
        let spotlight_bonus = if spotlight_roll < spotlight_chance {
            3.0 + self.profile.emotional_volatility * 2.0
        } else {
            0.0
        };

        quality + age_bonus + attitude_bonus + noise + youth_bias
            + height_bonus + pace_bonus + hidden_bonus + spotlight_bonus
    }

    /// Training impression: style-dependent weighting of visible effort vs skill signals
    pub fn training_impression(&self, player: &Player) -> f32 {
        let professionalism = player.attributes.professionalism;
        let determination = player.skills.mental.determination;
        let work_rate = player.skills.mental.work_rate;

        // Attitude-sensitive coaches overvalue visible effort in training
        let aw = self.profile.attitude_weight;
        let prof_w = 0.40 - aw * 0.10;
        let det_w = 0.30;
        let wr_w = 0.30 + aw * 0.10;
        let base_effort = professionalism * prof_w + determination * det_w + work_rate * wr_w;

        // Non-attitude coaches pick up on skill signals in training
        let skill_signal = {
            let flair = player.skills.mental.flair;
            let technique = player.skills.technical.technique;
            ((flair + technique) / 2.0 - 10.0) * (1.0 - aw) * 0.08
        };

        let base = base_effort + skill_signal;

        // Time-varying noise (not constant across weeks!)
        let noise_scale = 1.0 - self.profile.judging_accuracy;
        let noise = self.profile.perception_noise(player.id,
            self.current_week.wrapping_add(42)) * noise_scale * 1.5;

        (base + noise).clamp(1.0, 20.0)
    }

    /// Update or create impression. Visibility-based skipping, sticky drift,
    /// overreaction mechanics, trust decay, emotional heat accumulation.
    pub fn update_impression(&mut self, player: &Player, date: NaiveDate, team_type: &TeamType) {
        // Pre-compute everything that reads &self before mutable entry borrow
        let new_quality = self.perceived_quality(player, date);
        let new_readiness = self.match_readiness(player);
        let new_potential = self.potential_impression(player, date);
        let new_training = self.training_impression(player);
        let visibility = self.compute_visibility(player, team_type);
        let initial_bias = self.init_bias(player.id);

        // Cache profile values for use while impression is borrowed
        let volatility = self.profile.emotional_volatility;
        let negativity_bias = self.profile.negativity_bias;
        let judging_accuracy = self.profile.judging_accuracy;
        let confirmation_bias = self.profile.confirmation_bias;
        let trust_in_decisions = self.profile.trust_in_decisions;
        let current_week = self.current_week;
        let coach_seed = self.profile.coach_seed;
        let stubbornness = self.profile.stubbornness;

        // --- Mutable borrow of impressions starts here ---
        let impression = self
            .impressions
            .entry(player.id)
            .or_insert_with(|| {
                let mut imp = PlayerImpression::new(player.id, date);
                imp.bias = initial_bias;
                imp.prev_red_cards = player.statistics.red_cards;
                imp.prev_goals = player.statistics.goals;
                imp.prev_avg_rating = player.statistics.average_rating;
                imp
            });

        // Always update visibility
        impression.bias.visibility = visibility;

        // --- Visibility-based observation skip ---
        // First encounter (quality == 0.0) always gets full observation
        let is_first_encounter = impression.perceived_quality == 0.0;
        if !is_first_encounter {
            let skip_prob = if *team_type == TeamType::Main {
                0.0 // always observe first team
            } else {
                ((1.0 - visibility) * 0.6).clamp(0.0, 0.7)
            };
            let skip_seed = coach_seed
                .wrapping_mul(player.id)
                .wrapping_add(current_week.wrapping_mul(0xA77E));
            if seeded_decision(skip_prob, skip_seed) {
                // Not observed: trust decays, impression stays stale
                impression.coach_trust = (impression.coach_trust - 0.05).clamp(0.0, 10.0);
                impression.weeks_in_squad = impression.weeks_in_squad.saturating_add(1);
                impression.last_updated = date;
                if impression.bias.overreaction_timer > 0 {
                    impression.bias.overreaction_timer -= 1;
                }
                impression.bias.sunk_cost *= 0.95;
                return;
            }
        }

        // Mark as observed
        impression.bias.last_observation_week = current_week;

        // --- First impression anchoring ---
        if !impression.bias.anchored {
            impression.bias.first_impression = new_quality;
            impression.bias.anchored = true;
        }

        // --- Event detection with overreaction ---
        let mut heat_delta: f32 = 0.0;

        // Red cards
        if player.statistics.red_cards > impression.prev_red_cards {
            impression.bias.quality_offset = (impression.bias.quality_offset - 0.5 * negativity_bias).clamp(-3.0, 3.0);
            impression.bias.disappointments = impression.bias.disappointments.saturating_add(1);
            // Overreaction: volatile coaches flip out
            impression.bias.overreaction_timer = (3.0 + volatility * 3.0) as u8;
            impression.bias.overreaction_magnitude = -1.5 * volatility;
            impression.coach_trust = (impression.coach_trust - 1.5 * volatility).clamp(0.0, 10.0);
            heat_delta += 0.15 * volatility;
        }

        // Average rating dropped significantly
        if impression.prev_avg_rating > 0.0
            && player.statistics.average_rating < impression.prev_avg_rating - 0.5
        {
            impression.bias.quality_offset = (impression.bias.quality_offset - 0.3 * negativity_bias).clamp(-3.0, 3.0);
            if volatility > 0.4 {
                impression.bias.overreaction_timer = impression.bias.overreaction_timer
                    .max((2.0 + volatility * 2.0) as u8);
                impression.bias.overreaction_magnitude = (impression.bias.overreaction_magnitude
                    - 0.8 * volatility).clamp(-3.0, 3.0);
            }
            heat_delta += 0.10 * volatility;
        }

        // Goals scored (significant increase)
        if player.statistics.goals > impression.prev_goals + 2 {
            impression.bias.quality_offset = (impression.bias.quality_offset + 0.3).clamp(-3.0, 3.0);
            impression.bias.overreaction_timer = impression.bias.overreaction_timer
                .max((2.0 + volatility) as u8);
            impression.bias.overreaction_magnitude = (impression.bias.overreaction_magnitude
                + 1.0 * volatility).clamp(-3.0, 3.0);
        }

        // Poor behaviour
        if player.behaviour.is_poor() {
            impression.bias.quality_offset = (impression.bias.quality_offset - 0.4 * negativity_bias).clamp(-3.0, 3.0);
            impression.bias.disappointments = impression.bias.disappointments.saturating_add(1);
            impression.bias.overreaction_timer = (2.0 + volatility * 2.0) as u8;
            impression.bias.overreaction_magnitude = -1.0 * volatility;
            heat_delta += 0.10 * volatility;
        }

        // Average rating excellent
        if player.statistics.average_rating > 7.5
            && player.statistics.played + player.statistics.played_subs > 3
        {
            impression.bias.quality_offset = (impression.bias.quality_offset + 0.3).clamp(-3.0, 3.0);
        }

        // Update stat snapshots
        impression.prev_red_cards = player.statistics.red_cards;
        impression.prev_goals = player.statistics.goals;
        impression.prev_avg_rating = player.statistics.average_rating;

        // --- Overreaction timer countdown ---
        if impression.bias.overreaction_timer > 0 {
            impression.bias.overreaction_timer -= 1;
            // During overreaction, quality offset drifts toward overreaction direction
            impression.bias.quality_offset = (impression.bias.quality_offset
                + impression.bias.overreaction_magnitude * 0.15).clamp(-3.0, 3.0);
        } else {
            impression.bias.overreaction_magnitude *= 0.5; // decay when timer off
        }

        // --- Perception drift: slow random walk (sticky correlated noise) ---
        let drift_noise = perception_noise_raw(coach_seed, player.id,
            current_week.wrapping_add(0xDFFF)) * 0.12 * (1.0 - judging_accuracy);
        impression.bias.perception_drift = (impression.bias.perception_drift * 0.97 + drift_noise)
            .clamp(-2.0, 2.0);

        // --- Bias drift: 2% decay toward zero ---
        impression.bias.quality_offset *= 0.98;
        let offset_noise = perception_noise_raw(coach_seed, player.id,
            current_week.wrapping_add(0xD01F)) * 0.05 * (1.0 - judging_accuracy);
        impression.bias.quality_offset = (impression.bias.quality_offset + offset_noise).clamp(-3.0, 3.0);

        // --- Sunk cost decay ---
        impression.bias.sunk_cost *= 0.95;

        // --- Trust ceiling based on disappointments ---
        let trust_ceiling = 7.0 - impression.bias.disappointments.min(4) as f32;

        if impression.perceived_quality == 0.0 {
            // First observation: take raw values
            impression.perceived_quality = new_quality;
            impression.match_readiness = new_readiness;
            impression.potential_impression = new_potential;
            impression.training_impression = new_training;
        } else {
            // --- Asymmetric blend ---
            let base_blend = trust_in_decisions * 0.6;
            let delta = new_quality - impression.perceived_quality;

            let direction_matches = (delta > 0.0) == (impression.perceived_quality >= impression.bias.first_impression);
            let conf_shift = if direction_matches {
                -confirmation_bias * 0.15
            } else {
                confirmation_bias * 0.15
            };

            let neg_shift = if delta < 0.0 {
                -negativity_bias * 0.1
            } else {
                negativity_bias * 0.05
            };

            let vis_dampening = (1.0 - visibility) * 0.2;

            let old_weight = (base_blend + conf_shift + neg_shift + vis_dampening).clamp(0.15, 0.90);
            let new_weight = 1.0 - old_weight;

            impression.perceived_quality =
                impression.perceived_quality * old_weight + new_quality * new_weight;
            impression.match_readiness = new_readiness;
            impression.potential_impression =
                impression.potential_impression * old_weight + new_potential * new_weight;
            impression.training_impression =
                impression.training_impression * old_weight + new_training * new_weight;
        }

        // --- Trust: no longer monotonic. Grows when observed, decays when stale ---
        impression.coach_trust = (impression.coach_trust + 0.1).clamp(0.0, trust_ceiling);
        impression.weeks_in_squad = impression.weeks_in_squad.saturating_add(1);
        impression.last_updated = date;

        // Decay recent_move after protection window (sunk_cost extends)
        if let Some(ref mv) = impression.recent_move {
            let weeks_since = current_week.saturating_sub(mv.week);
            let base_protection = (4.0 * stubbornness).max(2.0) as u32;
            let sunk_cost_extension = (impression.bias.sunk_cost * 0.5) as u32;
            let protection_weeks = base_protection + sunk_cost_extension;
            if weeks_since > protection_weeks {
                impression.recent_move = None;
            }
        }

        // --- Accumulate emotional heat on state (disjoint field from impressions) ---
        self.emotional_heat = (self.emotional_heat + heat_delta).clamp(0.0, 1.0);
    }

    /// Record a move for inertia tracking, updating sunk cost
    pub fn record_move(&mut self, player_id: u32, move_type: RecentMoveType, date: NaiveDate) {
        let week = date_to_week(date);
        let impression = self
            .impressions
            .entry(player_id)
            .or_insert_with(|| PlayerImpression::new(player_id, date));
        impression.recent_move = Some(RecentMove { move_type, week });

        match move_type {
            RecentMoveType::DemotedToReserves | RecentMoveType::SwappedOut => {
                impression.coach_trust = (impression.coach_trust - 1.0).clamp(0.0, 10.0);
                impression.bias.sunk_cost = (impression.bias.sunk_cost - 1.0).max(0.0);
            }
            RecentMoveType::PromotedToFirst
            | RecentMoveType::RecalledFromReserves
            | RecentMoveType::YouthPromoted
            | RecentMoveType::SwappedIn => {
                impression.coach_trust = (impression.coach_trust + 0.5).clamp(0.0, 10.0);
                impression.bias.sunk_cost = (impression.bias.sunk_cost + 2.0).min(10.0);
            }
        }
    }

    /// Check if a player has a recent move providing protection from reversal
    pub fn is_protected(&self, player_id: u32, protecting_moves: &[RecentMoveType]) -> bool {
        if let Some(impression) = self.impressions.get(&player_id) {
            if let Some(ref mv) = impression.recent_move {
                if protecting_moves.contains(&mv.move_type) {
                    let weeks_since = self.current_week.saturating_sub(mv.week);
                    let base_protection = (4.0 * self.profile.stubbornness).max(2.0) as u32;
                    let sunk_cost_extension = (impression.bias.sunk_cost * 0.5) as u32;
                    let protection_weeks = base_protection + sunk_cost_extension;
                    return weeks_since <= protection_weeks;
                }
            }
        }
        false
    }

    /// Get cached impression for a player
    pub fn get_impression(&self, player_id: u32) -> Option<&PlayerImpression> {
        self.impressions.get(&player_id)
    }
}

// ─── Utility functions ───────────────────────────────────────────────

/// Standalone noise function usable without borrowing CoachProfile
fn perception_noise_raw(coach_seed: u32, player_id: u32, salt: u32) -> f32 {
    let hash = coach_seed
        .wrapping_mul(2654435761)
        .wrapping_add(player_id.wrapping_mul(2246822519))
        .wrapping_add(salt.wrapping_mul(3266489917));
    let hash = hash ^ (hash >> 16);
    let hash = hash.wrapping_mul(0x45d9f3b);
    let hash = hash ^ (hash >> 16);
    (hash & 0xFFFF) as f32 / 32768.0 - 1.0
}

pub fn sigmoid_probability(x: f32, steepness: f32) -> f32 {
    1.0 / (1.0 + (-x * steepness).exp())
}

pub fn seeded_decision(probability: f32, seed: u32) -> bool {
    if probability >= 1.0 {
        return true;
    }
    if probability <= 0.0 {
        return false;
    }
    let hash = seed
        .wrapping_mul(2654435761)
        .wrapping_add(0xdeadbeef);
    let hash = hash ^ (hash >> 16);
    let hash = hash.wrapping_mul(0x45d9f3b);
    let hash = hash ^ (hash >> 16);
    let roll = (hash & 0xFFFF) as f32 / 65536.0;
    roll < probability
}

pub fn date_to_week(date: NaiveDate) -> u32 {
    let epoch = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
    let days = date.signed_duration_since(epoch).num_days();
    (days / 7).max(0) as u32
}
