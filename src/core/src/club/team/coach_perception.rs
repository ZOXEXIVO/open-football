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

        // Shift attacking_focus by attacking vs defending ratio
        let atk_def_ratio = if coaching.attacking + coaching.defending > 0 {
            coaching.attacking as f32 / (coaching.attacking as f32 + coaching.defending as f32)
        } else {
            0.5
        };
        let atk_shift = (atk_def_ratio - 0.5) * 0.3;

        // Shift creativity_focus by technical coaching
        let tech_shift = (coaching.technical as f32 / 20.0 - 0.5) * 0.2;

        // Shift physicality_focus by fitness coaching
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

        // Confirmation bias: stubborn coaches resist contradicting evidence
        let conf_style_mod = match staff.coaching_style {
            CoachingStyle::Authoritarian => 0.1,
            CoachingStyle::Democratic => -0.05,
            _ => 0.0,
        };
        let confirmation_bias = (determination_norm * 0.4
            + (1.0 - adaptability_norm) * 0.6
            + conf_style_mod)
            .clamp(0.0, 1.0);

        // Negativity bias: bad events weighted more than good
        let neg_style_mod = match staff.coaching_style {
            CoachingStyle::Authoritarian => 0.15,
            CoachingStyle::Transformational => -0.1,
            _ => 0.0,
        };
        let negativity_bias = (discipline_norm * 0.5
            + (1.0 - man_management_norm) * 0.5
            + neg_style_mod)
            .clamp(0.0, 1.0);

        // Physical bias for youth scouting
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

        let perception_lens = PerceptionLens::from_style_and_staff(&staff.coaching_style, coaching);

        CoachProfile {
            judging_accuracy: (knowledge.judging_player_ability as f32 / 20.0).clamp(0.0, 1.0),
            potential_accuracy: (knowledge.judging_player_potential as f32 / 20.0).clamp(0.0, 1.0),
            risk_tolerance: (adaptability_norm + style_adaptability_bonus).clamp(0.0, 1.0),
            stubbornness: ((determination_norm * 0.6 + discipline_norm * 0.4)).clamp(0.0, 1.0),
            trust_in_decisions: determination_norm.clamp(0.0, 1.0),
            youth_preference: (coaching.working_with_youngsters as f32 / 20.0 + style_youth_bonus)
                .clamp(0.0, 1.0),
            conservatism: ((1.0 - adaptability_norm) * 0.6 + discipline_norm * 0.4).clamp(0.0, 1.0),
            coach_seed: staff.id,
            perception_lens,
            confirmation_bias,
            negativity_bias,
            physical_bias_youth,
        }
    }

    /// Hash-based noise in [-1.0, 1.0] per coach+player+week. Deterministic.
    pub fn perception_noise(&self, player_id: u32, date_week: u32) -> f32 {
        let hash = self
            .coach_seed
            .wrapping_mul(2654435761)
            .wrapping_add(player_id.wrapping_mul(2246822519))
            .wrapping_add(date_week.wrapping_mul(3266489917));
        let hash = hash ^ (hash >> 16);
        let hash = hash.wrapping_mul(0x45d9f3b);
        let hash = hash ^ (hash >> 16);
        // Map to [-1.0, 1.0]
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
        }
    }

    /// Compute lens-weighted skill composite using the coach's perception lens
    fn lens_skill_composite(&self, player: &Player) -> f32 {
        let lens = &self.profile.perception_lens;
        let t = &player.skills.technical;
        let m = &player.skills.mental;
        let p = &player.skills.physical;

        // Technical: blend attacking-tech and defensive-tech by attacking_focus
        let atk_tech = (t.finishing + t.dribbling + t.crossing + t.first_touch + t.technique + t.long_shots) / 6.0;
        let def_tech = (t.tackling + t.marking + t.heading + t.passing) / 4.0;
        let tech_score = atk_tech * lens.attacking_focus + def_tech * (1.0 - lens.attacking_focus);

        // Mental: blend creative-mental and discipline-mental by creativity_focus
        let creative_mental = (m.flair + m.vision + m.composure + m.decisions + m.anticipation) / 5.0;
        let discipline_mental = (m.work_rate + m.determination + m.positioning + m.teamwork + m.concentration) / 5.0;
        let mental_score = creative_mental * lens.creativity_focus + discipline_mental * (1.0 - lens.creativity_focus);

        // Physical: blend explosive and endurance by physicality_focus
        let explosive = (p.pace + p.acceleration + p.strength + p.jumping) / 4.0;
        let endurance = (p.stamina + p.natural_fitness + p.agility + p.balance) / 4.0;
        let physical_score = explosive * lens.physicality_focus + endurance * (1.0 - lens.physicality_focus);

        tech_score * lens.technical_weight + mental_score * lens.mental_weight + physical_score * lens.physical_weight
    }

    /// Initialize bias for a new player encounter
    fn init_bias(&self, player_id: u32) -> PlayerBias {
        // Seed from coach_seed + player_id + magic constant
        let hash = self.profile.coach_seed
            .wrapping_mul(2654435761)
            .wrapping_add(player_id.wrapping_mul(2246822519))
            .wrapping_add(0xB1A5u32.wrapping_mul(3266489917));
        let hash = hash ^ (hash >> 16);
        let hash = hash.wrapping_mul(0x45d9f3b);
        let hash = hash ^ (hash >> 16);

        // Map to [-1.0, 1.0] then scale by inaccuracy
        let raw = (hash & 0xFFFF) as f32 / 32768.0 - 1.0;
        let magnitude = (1.0 - self.profile.judging_accuracy) * 2.0;
        let quality_offset = raw * magnitude;

        PlayerBias {
            quality_offset,
            visibility: 1.0,
            sunk_cost: 0.0,
            first_impression: 0.0,
            anchored: false,
            disappointments: 0,
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

    /// Evaluate perceived quality: lens-weighted skill composite + bias + form + reputation + condition
    pub fn perceived_quality(&self, player: &Player, date: NaiveDate) -> f32 {
        let week = date_to_week(date);

        // Get bias info if available
        let (quality_offset, visibility, first_imp, anchored) = self.impressions.get(&player.id)
            .map(|imp| (imp.bias.quality_offset, imp.bias.visibility, imp.bias.first_impression, imp.bias.anchored))
            .unwrap_or((0.0, 1.0, 0.0, false));

        // Noise scaled by 1/visibility (invisible players = noisier)
        let noise_scale = 1.0 - self.profile.judging_accuracy;
        let visibility_noise_scale = noise_scale / visibility.max(0.1);
        let noise = self.profile.perception_noise(player.id, week) * visibility_noise_scale * 2.5;

        // Lens-weighted skill composite
        let skill_composite = self.lens_skill_composite(player);

        // Position mastery
        let position_level = player
            .positions
            .positions
            .iter()
            .map(|p| p.level)
            .max()
            .unwrap_or(0) as f32;

        let base = skill_composite * 0.75 + position_level * 0.25;

        // Add bias offset before condition scaling
        let biased_base = base + quality_offset;

        // Form bonus
        let form_bonus = if player.statistics.played + player.statistics.played_subs > 3 {
            (player.statistics.average_rating - 6.5).clamp(-1.5, 1.5)
        } else {
            0.0
        };

        // Reputation bias (small weight from world reputation)
        let rep_bias = (player.player_attributes.world_reputation as f32 / 10000.0).clamp(0.0, 0.5);

        // Anchoring pull for stubborn coaches
        let anchor_pull = if anchored {
            let observation = biased_base + form_bonus + rep_bias + noise;
            (first_imp - observation) * self.profile.stubbornness * 0.15
        } else {
            0.0
        };

        // Condition factor
        let condition = (player.player_attributes.condition_percentage() as f32 / 100.0).clamp(0.5, 1.0);

        (biased_base + form_bonus + rep_bias + noise + anchor_pull) * condition
    }

    /// Match readiness: condition + match sharpness + days-since-last-match curve + injury/ban penalties
    pub fn match_readiness(&self, player: &Player) -> f32 {
        if player.player_attributes.is_injured {
            return 0.0;
        }
        if player.player_attributes.is_banned {
            return 0.0;
        }

        let condition = player.player_attributes.condition_percentage() as f32 / 100.0;
        let fitness = player.player_attributes.fitness as f32 / 10000.0;

        // Match sharpness: players who haven't played recently lose sharpness
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

        // Physical readiness from match_readiness skill
        let physical_readiness = player.skills.physical.match_readiness / 20.0;

        (condition * 0.35 + fitness.clamp(0.0, 1.0) * 0.25 + sharpness * 0.25 + physical_readiness * 0.15)
            .clamp(0.0, 1.0)
            * 20.0 // Scale to ~0-20 to match quality scale
    }

    /// Youth potential impression with physical bias for youth scouting
    pub fn potential_impression(&self, player: &Player, date: NaiveDate) -> f32 {
        let quality = self.perceived_quality(player, date);
        let age = DateUtils::age(player.birth_date, date);
        let week = date_to_week(date);

        // Age bonus: younger = more perceived potential
        let age_bonus = match age {
            0..=15 => 1.5,
            16..=17 => 2.5,
            18 => 3.0,
            19..=20 => 2.0,
            21..=22 => 1.0,
            _ => 0.0,
        };

        // Attitude: professional, determined youth look more promising
        let attitude =
            (player.attributes.professionalism + player.skills.mental.determination) / 2.0;
        let attitude_bonus = (attitude - 10.0).clamp(-1.0, 2.0) * 0.5;

        // Potential noise (scales with inaccuracy)
        let noise_scale = 1.0 - self.profile.potential_accuracy;
        let noise = self.profile.perception_noise(player.id, week.wrapping_add(7919)) * noise_scale * 3.0;

        // Youth preference bias: coaches who love youth see more potential
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

        // Hidden traits (composure, decisions, anticipation, technique, first_touch)
        // Only visible proportional to potential_accuracy
        let hidden_quality = (player.skills.mental.composure
            + player.skills.mental.decisions
            + player.skills.mental.anticipation
            + player.skills.technical.first_touch
            + player.skills.technical.technique) / 5.0;
        let hidden_perception = hidden_quality * self.profile.potential_accuracy;
        // Blend: reduce reliance on raw quality, add hidden perception
        let hidden_bonus = (hidden_perception - quality * 0.3) * 0.3;

        quality + age_bonus + attitude_bonus + noise + youth_bias + height_bonus + pace_bonus + hidden_bonus
    }

    /// Training impression: professionalism + determination + work rate with accuracy noise
    pub fn training_impression(&self, player: &Player) -> f32 {
        let professionalism = player.attributes.professionalism;
        let determination = player.skills.mental.determination;
        let work_rate = player.skills.mental.work_rate;

        let base = professionalism * 0.4 + determination * 0.35 + work_rate * 0.25;

        let noise_scale = 1.0 - self.profile.judging_accuracy;
        let noise = self.profile.perception_noise(player.id, 42) * noise_scale * 1.5;

        (base + noise).clamp(1.0, 20.0)
    }

    /// Update or create impression for a player, blending new perceptions with old.
    /// Now accepts team_type for visibility computation and includes bias mechanics.
    pub fn update_impression(&mut self, player: &Player, date: NaiveDate, team_type: &TeamType) {
        let new_quality = self.perceived_quality(player, date);
        let new_readiness = self.match_readiness(player);
        let new_potential = self.potential_impression(player, date);
        let new_training = self.training_impression(player);

        let visibility = self.compute_visibility(player, team_type);
        let initial_bias = self.init_bias(player.id);

        let impression = self
            .impressions
            .entry(player.id)
            .or_insert_with(|| {
                let mut imp = PlayerImpression::new(player.id, date);
                imp.bias = initial_bias;
                // Initialize stat snapshots
                imp.prev_red_cards = player.statistics.red_cards;
                imp.prev_goals = player.statistics.goals;
                imp.prev_avg_rating = player.statistics.average_rating;
                imp
            });

        // Update visibility
        impression.bias.visibility = visibility;

        // --- First impression anchoring ---
        if !impression.bias.anchored {
            impression.bias.first_impression = new_quality;
            impression.bias.anchored = true;
        }

        // --- Event detection: compare current stats to snapshot ---
        let negativity_bias = self.profile.negativity_bias;

        // Red cards
        if player.statistics.red_cards > impression.prev_red_cards {
            impression.bias.quality_offset = (impression.bias.quality_offset - 0.5 * negativity_bias).clamp(-3.0, 3.0);
            impression.bias.disappointments = impression.bias.disappointments.saturating_add(1);
        }

        // Average rating dropped significantly
        if impression.prev_avg_rating > 0.0
            && player.statistics.average_rating < impression.prev_avg_rating - 0.5
        {
            impression.bias.quality_offset = (impression.bias.quality_offset - 0.3 * negativity_bias).clamp(-3.0, 3.0);
        }

        // Goals scored (significant increase)
        if player.statistics.goals > impression.prev_goals + 2 {
            impression.bias.quality_offset = (impression.bias.quality_offset + 0.3).clamp(-3.0, 3.0);
        }

        // Poor behaviour
        if player.behaviour.is_poor() {
            impression.bias.quality_offset = (impression.bias.quality_offset - 0.4 * negativity_bias).clamp(-3.0, 3.0);
            impression.bias.disappointments = impression.bias.disappointments.saturating_add(1);
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

        // --- Bias drift: 2% decay toward zero + small random walk ---
        impression.bias.quality_offset *= 0.98;
        let inaccuracy = 1.0 - self.profile.judging_accuracy;
        let drift_noise = self.profile.perception_noise(player.id, self.current_week.wrapping_add(0xD01F)) * 0.05 * inaccuracy;
        impression.bias.quality_offset = (impression.bias.quality_offset + drift_noise).clamp(-3.0, 3.0);

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
            // --- Asymmetric blend with confirmation bias, negativity bias, visibility ---
            let base_blend = self.profile.trust_in_decisions * 0.6;
            let delta = new_quality - impression.perceived_quality;

            // Confirmation bias: confirming evidence absorbed faster
            let direction_matches = (delta > 0.0) == (impression.perceived_quality >= impression.bias.first_impression);
            let conf_shift = if direction_matches {
                -self.profile.confirmation_bias * 0.15
            } else {
                self.profile.confirmation_bias * 0.15
            };

            // Negativity bias: negative changes absorbed faster
            let neg_shift = if delta < 0.0 {
                -negativity_bias * 0.1
            } else {
                negativity_bias * 0.05
            };

            // Visibility dampening: low visibility = slower updates
            let vis_dampening = (1.0 - visibility) * 0.2;

            let old_weight = (base_blend + conf_shift + neg_shift + vis_dampening).clamp(0.15, 0.90);
            let new_weight = 1.0 - old_weight;

            impression.perceived_quality =
                impression.perceived_quality * old_weight + new_quality * new_weight;
            impression.match_readiness = new_readiness; // Always current
            impression.potential_impression =
                impression.potential_impression * old_weight + new_potential * new_weight;
            impression.training_impression =
                impression.training_impression * old_weight + new_training * new_weight;
        }

        // Build trust over time, but cap at trust ceiling
        impression.coach_trust = (impression.coach_trust + 0.1).clamp(0.0, trust_ceiling);
        impression.weeks_in_squad = impression.weeks_in_squad.saturating_add(1);
        impression.last_updated = date;

        // Decay recent_move after protection window (sunk_cost extends protection)
        if let Some(ref mv) = impression.recent_move {
            let weeks_since = self.current_week.saturating_sub(mv.week);
            let base_protection = (4.0 * self.profile.stubbornness).max(2.0) as u32;
            let sunk_cost_extension = (impression.bias.sunk_cost * 0.5) as u32;
            let protection_weeks = base_protection + sunk_cost_extension;
            if weeks_since > protection_weeks {
                impression.recent_move = None;
            }
        }
    }

    /// Record a move for inertia tracking, updating sunk cost
    pub fn record_move(&mut self, player_id: u32, move_type: RecentMoveType, date: NaiveDate) {
        let week = date_to_week(date);
        let impression = self
            .impressions
            .entry(player_id)
            .or_insert_with(|| PlayerImpression::new(player_id, date));
        impression.recent_move = Some(RecentMove { move_type, week });

        // Adjust trust and sunk cost based on move type
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

    /// Get cached impression for a player (or default)
    pub fn get_impression(&self, player_id: u32) -> Option<&PlayerImpression> {
        self.impressions.get(&player_id)
    }
}

// ─── Utility functions ───────────────────────────────────────────────

/// Soft threshold: maps distance-from-threshold to probability in [0,1].
/// Positive x = more likely to trigger. Steepness controls how sharp the transition is.
pub fn sigmoid_probability(x: f32, steepness: f32) -> f32 {
    1.0 / (1.0 + (-x * steepness).exp())
}

/// Deterministic coin flip from hash. Same inputs = same result. No rand::random().
pub fn seeded_decision(probability: f32, seed: u32) -> bool {
    if probability >= 1.0 {
        return true;
    }
    if probability <= 0.0 {
        return false;
    }
    // Hash the seed to get a uniform-ish value in [0, 1)
    let hash = seed
        .wrapping_mul(2654435761)
        .wrapping_add(0xdeadbeef);
    let hash = hash ^ (hash >> 16);
    let hash = hash.wrapping_mul(0x45d9f3b);
    let hash = hash ^ (hash >> 16);
    let roll = (hash & 0xFFFF) as f32 / 65536.0;
    roll < probability
}

/// Convert a NaiveDate to a week number (weeks since epoch-ish)
pub fn date_to_week(date: NaiveDate) -> u32 {
    let epoch = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
    let days = date.signed_duration_since(epoch).num_days();
    (days / 7).max(0) as u32
}
