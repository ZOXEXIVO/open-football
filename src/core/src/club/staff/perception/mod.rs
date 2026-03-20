pub mod bias;
mod evaluation;
pub mod profile;
pub mod utils;

// Re-export key types at module level
pub use bias::{PlayerBias, PlayerImpression, RecentMove, RecentMoveType};
pub use profile::{CoachProfile, PerceptionLens};
pub use utils::{date_to_week, seeded_decision, sigmoid_probability};

// CoachDecisionState is defined here (the "state" module) because it orchestrates
// evaluation + bias + profile. Evaluation methods are in evaluation.rs as `impl CoachDecisionState`.

use crate::{Player, Staff, TeamType};
use chrono::NaiveDate;
use std::collections::HashMap;

use utils::perception_noise_raw;

/// Namespace alias for use in `impl` blocks in other files
pub(crate) mod state {
    pub type CoachDecisionState = super::CoachDecisionState;
}

#[derive(Debug, Clone)]
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

    /// Update or create impression. Visibility-based skipping, sticky drift,
    /// overreaction mechanics, trust decay, emotional heat accumulation.
    pub fn update_impression(&mut self, player: &Player, date: NaiveDate, team_type: &TeamType) {
        let new_quality = self.perceived_quality(player, date);
        let new_readiness = self.match_readiness(player);
        let new_potential = self.potential_impression(player, date);
        let new_training = self.training_impression(player);
        let visibility = self.compute_visibility(player, team_type);
        let initial_bias = self.init_bias(player.id);

        let volatility = self.profile.emotional_volatility;
        let negativity_bias = self.profile.negativity_bias;
        let judging_accuracy = self.profile.judging_accuracy;
        let confirmation_bias = self.profile.confirmation_bias;
        let trust_in_decisions = self.profile.trust_in_decisions;
        let current_week = self.current_week;
        let coach_seed = self.profile.coach_seed;
        let stubbornness = self.profile.stubbornness;

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

        impression.bias.visibility = visibility;

        // Visibility-based observation skip
        let is_first_encounter = impression.perceived_quality == 0.0;
        if !is_first_encounter {
            let skip_prob = if *team_type == TeamType::Main {
                0.0
            } else {
                ((1.0 - visibility) * 0.6).clamp(0.0, 0.7)
            };
            let skip_seed = coach_seed
                .wrapping_mul(player.id)
                .wrapping_add(current_week.wrapping_mul(0xA77E));
            if seeded_decision(skip_prob, skip_seed) {
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

        impression.bias.last_observation_week = current_week;

        // First impression anchoring
        if !impression.bias.anchored {
            impression.bias.first_impression = new_quality;
            impression.bias.anchored = true;
        }

        // Event detection with overreaction
        let mut heat_delta: f32 = 0.0;

        if player.statistics.red_cards > impression.prev_red_cards {
            impression.bias.quality_offset = (impression.bias.quality_offset - 0.5 * negativity_bias).clamp(-3.0, 3.0);
            impression.bias.disappointments = impression.bias.disappointments.saturating_add(1);
            impression.bias.overreaction_timer = (3.0 + volatility * 3.0) as u8;
            impression.bias.overreaction_magnitude = -1.5 * volatility;
            impression.coach_trust = (impression.coach_trust - 1.5 * volatility).clamp(0.0, 10.0);
            heat_delta += 0.15 * volatility;
        }

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

        if player.statistics.goals > impression.prev_goals + 2 {
            impression.bias.quality_offset = (impression.bias.quality_offset + 0.3).clamp(-3.0, 3.0);
            impression.bias.overreaction_timer = impression.bias.overreaction_timer
                .max((2.0 + volatility) as u8);
            impression.bias.overreaction_magnitude = (impression.bias.overreaction_magnitude
                + 1.0 * volatility).clamp(-3.0, 3.0);
        }

        if player.behaviour.is_poor() {
            impression.bias.quality_offset = (impression.bias.quality_offset - 0.4 * negativity_bias).clamp(-3.0, 3.0);
            impression.bias.disappointments = impression.bias.disappointments.saturating_add(1);
            impression.bias.overreaction_timer = (2.0 + volatility * 2.0) as u8;
            impression.bias.overreaction_magnitude = -1.0 * volatility;
            heat_delta += 0.10 * volatility;
        }

        if player.statistics.average_rating > 7.5
            && player.statistics.played + player.statistics.played_subs > 3
        {
            impression.bias.quality_offset = (impression.bias.quality_offset + 0.3).clamp(-3.0, 3.0);
        }

        impression.prev_red_cards = player.statistics.red_cards;
        impression.prev_goals = player.statistics.goals;
        impression.prev_avg_rating = player.statistics.average_rating;

        // Overreaction timer countdown
        if impression.bias.overreaction_timer > 0 {
            impression.bias.overreaction_timer -= 1;
            impression.bias.quality_offset = (impression.bias.quality_offset
                + impression.bias.overreaction_magnitude * 0.15).clamp(-3.0, 3.0);
        } else {
            impression.bias.overreaction_magnitude *= 0.5;
        }

        // Perception drift
        let drift_noise = perception_noise_raw(coach_seed, player.id,
            current_week.wrapping_add(0xDFFF)) * 0.12 * (1.0 - judging_accuracy);
        impression.bias.perception_drift = (impression.bias.perception_drift * 0.97 + drift_noise)
            .clamp(-2.0, 2.0);

        // Bias decay
        impression.bias.quality_offset *= 0.98;
        let offset_noise = perception_noise_raw(coach_seed, player.id,
            current_week.wrapping_add(0xD01F)) * 0.05 * (1.0 - judging_accuracy);
        impression.bias.quality_offset = (impression.bias.quality_offset + offset_noise).clamp(-3.0, 3.0);

        impression.bias.sunk_cost *= 0.95;

        // Trust ceiling
        let trust_ceiling = 7.0 - impression.bias.disappointments.min(4) as f32;

        if impression.perceived_quality == 0.0 {
            impression.perceived_quality = new_quality;
            impression.match_readiness = new_readiness;
            impression.potential_impression = new_potential;
            impression.training_impression = new_training;
        } else {
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

        impression.coach_trust = (impression.coach_trust + 0.1).clamp(0.0, trust_ceiling);
        impression.weeks_in_squad = impression.weeks_in_squad.saturating_add(1);
        impression.last_updated = date;

        // Decay recent_move after protection window
        if let Some(ref mv) = impression.recent_move {
            let weeks_since = current_week.saturating_sub(mv.week);
            let base_protection = (4.0 * stubbornness).max(2.0) as u32;
            let sunk_cost_extension = (impression.bias.sunk_cost * 0.5) as u32;
            let protection_weeks = base_protection + sunk_cost_extension;
            if weeks_since > protection_weeks {
                impression.recent_move = None;
            }
        }

        self.emotional_heat = (self.emotional_heat + heat_delta).clamp(0.0, 1.0);
    }

    /// Record a move for inertia tracking
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
