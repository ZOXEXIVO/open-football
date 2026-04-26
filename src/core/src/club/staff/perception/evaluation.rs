use crate::{Player, TeamType};
use crate::utils::DateUtils;
use chrono::NaiveDate;

use super::bias::PlayerBias;
use super::state::CoachDecisionState;
use super::utils::date_to_week;

impl CoachDecisionState {
    /// Lens-weighted skill composite using the coach's perception lens
    pub(super) fn lens_skill_composite(&self, player: &Player) -> f32 {
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
    pub(super) fn init_bias(&self, player_id: u32) -> PlayerBias {
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
            ..PlayerBias::default()
        }
    }

    /// Compute visibility for a player based on team type and context
    pub(super) fn compute_visibility(&self, player: &Player, team_type: &TeamType) -> f32 {
        let base = match team_type {
            TeamType::Main => 0.85,
            TeamType::B | TeamType::Second | TeamType::Reserve | TeamType::U23 | TeamType::U20 | TeamType::U21 => {
                0.5 + self.profile.youth_preference * 0.15
            }
            TeamType::U18 | TeamType::U19 => {
                0.2 + self.profile.youth_preference * 0.3
            }
        };

        let recency_bonus = if player.player_attributes.days_since_last_match <= 7 { 0.15 } else { 0.0 };

        let weeks_in = self.impressions.get(&player.id)
            .map(|imp| imp.weeks_in_squad)
            .unwrap_or(0);
        let familiarity_bonus = (weeks_in as f32 / 52.0).min(0.15);

        let appearances = player.statistics.played + player.statistics.played_subs;
        let performance_bonus = if appearances >= 3 {
            let rating_visibility = ((player.statistics.average_rating - 6.5) * 0.06).clamp(0.0, 0.12);
            let motm_visibility = (player.statistics.player_of_the_match as f32 * 0.04).min(0.08);
            rating_visibility + motm_visibility
        } else {
            0.0
        };

        (base + recency_bonus + familiarity_bonus + performance_bonus).clamp(0.1, 1.0)
    }

    /// Perceived quality: lens-weighted, bias-shifted, sticky noise, attitude bleed
    pub fn perceived_quality(&self, player: &Player, date: NaiveDate) -> f32 {
        let week = date_to_week(date);

        let (quality_offset, visibility, first_imp, anchored, perception_drift,
             overreaction_mag, overreaction_active) = self.impressions.get(&player.id)
            .map(|imp| (
                imp.bias.quality_offset, imp.bias.visibility, imp.bias.first_impression,
                imp.bias.anchored, imp.bias.perception_drift, imp.bias.overreaction_magnitude,
                imp.bias.overreaction_timer > 0,
            ))
            .unwrap_or((0.0, 1.0, 0.0, false, 0.0, 0.0, false));

        let noise_scale = 1.0 - self.profile.judging_accuracy;
        let jitter = self.profile.perception_noise(player.id, week) * noise_scale * 0.8;
        let visibility_amplifier = (1.0 / visibility.max(0.1)).min(2.5);
        let noise = (perception_drift + jitter) * visibility_amplifier;

        let skill_composite = self.lens_skill_composite(player);

        let position_level = player.positions.positions.iter()
            .map(|p| p.level).max().unwrap_or(0) as f32;
        let position_contribution = position_level * (1.0 - self.profile.tactical_blindness * 0.5);

        let base = skill_composite * 0.75 + position_contribution * 0.25;
        let biased_base = base + quality_offset;

        let raw_form_bonus = if player.statistics.played + player.statistics.played_subs > 3 {
            (player.statistics.average_rating - 6.5).clamp(-1.5, 1.5)
        } else {
            0.0
        };
        let form_bonus = raw_form_bonus * (1.0 + self.profile.recency_bias * 0.8);

        let attitude_bleed = {
            let visible_effort = (player.skills.mental.work_rate + player.skills.mental.determination) / 2.0;
            (visible_effort - 10.0) * self.profile.attitude_weight * 0.15
        };

        let rep_bias = (player.player_attributes.world_reputation as f32 / 10000.0).clamp(0.0, 0.5);

        let anchor_pull = if anchored {
            let observation = biased_base + form_bonus + rep_bias + noise + attitude_bleed;
            (first_imp - observation) * self.profile.stubbornness * 0.15
        } else {
            0.0
        };

        let overreaction_effect = if overreaction_active { overreaction_mag * 0.5 } else { 0.0 };

        let condition = (player.player_attributes.condition_percentage() as f32 / 100.0).clamp(0.5, 1.0);

        (biased_base + form_bonus + rep_bias + noise + anchor_pull + attitude_bleed + overreaction_effect) * condition
    }

    /// Match readiness: coach-specific — low intuition coaches misread fitness
    pub fn match_readiness(&self, player: &Player) -> f32 {
        if player.player_attributes.is_injured || player.player_attributes.is_banned {
            return 0.0;
        }

        let condition = player.player_attributes.condition_percentage() as f32 / 100.0;
        let fitness = player.player_attributes.fitness as f32 / 10000.0;

        let days_since = player.player_attributes.days_since_last_match as f32;
        let sharpness = if days_since <= 3.0 { 1.0 }
            else if days_since <= 7.0 { 0.95 }
            else if days_since <= 14.0 { 0.85 }
            else if days_since <= 28.0 { 0.70 }
            else { 0.55 };

        let physical_readiness = player.skills.physical.match_readiness / 20.0;

        let raw_readiness = (condition * 0.35 + fitness.clamp(0.0, 1.0) * 0.25
            + sharpness * 0.25 + physical_readiness * 0.15).clamp(0.0, 1.0);

        let intuition = self.profile.readiness_intuition;
        let noise_scale = (1.0 - intuition) * 0.25;
        let noise = self.profile.perception_noise(player.id,
            self.current_week.wrapping_add(0xFE57)) * noise_scale;

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
            0..=15 => 1.5, 16..=17 => 2.5, 18 => 3.0,
            19..=20 => 2.0, 21..=22 => 1.0, _ => 0.0,
        };

        let attitude = (player.attributes.professionalism + player.skills.mental.determination) / 2.0;
        let attitude_bonus = (attitude - 10.0).clamp(-1.0, 2.0) * 0.5;

        let noise_scale = 1.0 - self.profile.potential_accuracy;
        let noise = self.profile.perception_noise(player.id, week.wrapping_add(7919)) * noise_scale * 3.0;

        let youth_bias = self.profile.youth_preference * 1.5;

        let height_bonus = if player.player_attributes.height >= 185 {
            self.profile.physical_bias_youth * 1.5
        } else if player.player_attributes.height <= 170 {
            -self.profile.physical_bias_youth * 0.8
        } else {
            0.0
        };

        let pace_bonus = ((player.skills.physical.pace - 12.0).max(0.0) / 8.0)
            * self.profile.physical_bias_youth * 1.5;

        let hidden_quality = (player.skills.mental.composure + player.skills.mental.decisions
            + player.skills.mental.anticipation + player.skills.technical.first_touch
            + player.skills.technical.technique) / 5.0;
        let hidden_perception = hidden_quality * self.profile.potential_accuracy;
        let hidden_bonus = (hidden_perception - quality * 0.3) * 0.3;

        let spotlight_hash = self.profile.coach_seed
            .wrapping_mul(player.id).wrapping_add(week.wrapping_mul(13)).wrapping_add(0xCC01);
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

    /// Training impression: blends visible effort with actual training performance
    pub fn training_impression(&self, player: &Player) -> f32 {
        let professionalism = player.attributes.professionalism;
        let determination = player.skills.mental.determination;
        let work_rate = player.skills.mental.work_rate;

        let aw = self.profile.attitude_weight;
        let prof_w = 0.40 - aw * 0.10;
        let det_w = 0.30;
        let wr_w = 0.30 + aw * 0.10;
        let visible_effort = professionalism * prof_w + determination * det_w + work_rate * wr_w;

        let actual_performance = player.training.training_performance;

        let actual_weight = 0.30 + self.profile.judging_accuracy * 0.40;
        let effort_weight = 1.0 - actual_weight;
        let base = actual_performance * actual_weight + visible_effort * effort_weight;

        let skill_signal = {
            let flair = player.skills.mental.flair;
            let technique = player.skills.technical.technique;
            ((flair + technique) / 2.0 - 10.0) * (1.0 - aw) * 0.08
        };

        let noise_scale = 1.0 - self.profile.judging_accuracy;
        let noise = self.profile.perception_noise(player.id,
            self.current_week.wrapping_add(42)) * noise_scale * 1.5;

        (base + skill_signal + noise).clamp(1.0, 20.0)
    }
}
