//! The coach's running impressions of the squad — the second of the two
//! per-player stores this organ holds.
//!
//! Moved here from `staff/perception/` for the reason §2.6 of
//! `docs/staff_mind.md` gives: a coach's read of a player is one thing,
//! and it should live in one place. `staff::perception::CoachDecisionState`
//! is kept as a re-export, so every existing path resolves unchanged.
//!
//! Its evaluation half — `perceived_quality`, `match_readiness`,
//! `potential_impression`, `training_impression` — came with it, as
//! [`impression_lens`]: splitting one type's inherent impl across two
//! folders would have been worse than leaving it where it was, and it
//! would have forced five `pub(super)` internals open to the crate.
//!
//! What stays in `staff/perception/` is the **lens** — `CoachProfile`,
//! `PerceptionLens`, `PlayerBias`, `PlayerImpression` and the
//! estimators. That is what this state reads *through*; this file is
//! what he concluded and is carrying around.
//!
//! [`impression_lens`]: super::impression_lens

use crate::club::staff::perception::{
    CoachProfile, PerceptionNoise, PlayerImpression, RecentMove, RecentMoveType,
};
use crate::{Player, Staff, TeamType};
use chrono::NaiveDate;
use rustc_hash::FxHashMap;

/// What the coach observed this week, before it is blended with what he
/// already believed.
struct PlayerReadings {
    quality: f32,
    readiness: f32,
    potential: f32,
    training: f32,
}

#[derive(Debug, Clone)]
pub struct CoachDecisionState {
    pub profile: CoachProfile,
    pub impressions: FxHashMap<u32, PlayerImpression>,
    pub coach_id: u32,
    pub current_week: u32,
    pub squad_satisfaction: f32,
    pub weeks_since_last_change: u32,
    pub trigger_pressure: f32,
    pub emotional_heat: f32,
}

impl Default for CoachDecisionState {
    fn default() -> Self {
        Self::unbound()
    }
}

impl CoachDecisionState {
    /// A state that belongs to nobody yet.
    ///
    /// Every `Staff` carries one — a physio's stays like this forever,
    /// and a coach's is bound the first time he takes a dugout. The
    /// profile is a placeholder until [`Self::bind`] fills it in from
    /// the man himself.
    pub fn unbound() -> Self {
        CoachDecisionState {
            profile: CoachProfile::neutral(),
            impressions: FxHashMap::default(),
            coach_id: 0,
            current_week: 0,
            squad_satisfaction: 0.5,
            weeks_since_last_change: 0,
            trigger_pressure: 0.0,
            emotional_heat: 0.0,
        }
    }

    pub fn new(staff: &Staff, date: NaiveDate) -> Self {
        CoachDecisionState {
            profile: CoachProfile::from_staff(staff),
            impressions: FxHashMap::default(),
            coach_id: staff.id,
            current_week: PerceptionNoise::week(date),
            squad_satisfaction: 0.5,
            weeks_since_last_change: 0,
            trigger_pressure: 0.0,
            emotional_heat: 0.0,
        }
    }

    /// Bind this state to the coach it belongs to, deriving his lens.
    ///
    /// Idempotent after the first call for a given man: the profile is
    /// a pure function of attributes that never drift after generation,
    /// so re-deriving it is free and re-binding is a no-op. What it is
    /// *not* is a reset — the impressions, the heat and the pressure
    /// are exactly what the plan wants to survive a change of club.
    pub fn bind(&mut self, coach_id: u32, profile: CoachProfile, date: NaiveDate) {
        self.coach_id = coach_id;
        self.profile = profile;
        self.current_week = PerceptionNoise::week(date);
    }

    /// Has this state ever been bound to a coach?
    #[inline]
    pub fn is_bound(&self) -> bool {
        self.coach_id != 0
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

        // Track perception against the regressed average so coach
        // overreaction (delta vs prev_avg_rating) and quality bumps
        // aren't driven by small-sample raw noise.
        let pos_group = player.position().position_group();
        let regressed_avg = player.statistics.average_rating_realistic(pos_group);

        let impression = self.impressions.entry(player.id).or_insert_with(|| {
            let mut imp = PlayerImpression::new(player.id, date);
            imp.bias = initial_bias;
            imp.prev_red_cards = player.statistics.red_cards;
            imp.prev_goals = player.statistics.goals;
            imp.prev_avg_rating = regressed_avg;
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
            if PerceptionNoise::seeded_decision(skip_prob, skip_seed) {
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

        let heat_delta = Self::observe_match_events(
            impression,
            player,
            regressed_avg,
            volatility,
            negativity_bias,
        );
        Self::apply_bias_drift(
            impression,
            coach_seed,
            player.id,
            current_week,
            judging_accuracy,
        );

        // Trust ceiling
        let trust_ceiling = 7.0 - impression.bias.disappointments.min(4) as f32;

        let readings = PlayerReadings {
            quality: new_quality,
            readiness: new_readiness,
            potential: new_potential,
            training: new_training,
        };
        if impression.perceived_quality == 0.0 {
            impression.perceived_quality = readings.quality;
            impression.match_readiness = readings.readiness;
            impression.potential_impression = readings.potential;
            impression.training_impression = readings.training;
        } else {
            Self::blend_observations(
                impression,
                readings,
                trust_in_decisions,
                confirmation_bias,
                negativity_bias,
                visibility,
            );
        }

        impression.coach_trust = (impression.coach_trust + 0.1).clamp(0.0, trust_ceiling);
        impression.weeks_in_squad = impression.weeks_in_squad.saturating_add(1);
        impression.last_updated = date;

        Self::expire_recent_move(impression, current_week, stubbornness);
        self.emotional_heat = (self.emotional_heat + heat_delta).clamp(0.0, 1.0);
    }

    /// Apply the week's results to one player's impression, returning the
    /// emotional heat the events added.
    fn observe_match_events(
        impression: &mut PlayerImpression,
        player: &Player,
        regressed_avg: f32,
        volatility: f32,
        negativity_bias: f32,
    ) -> f32 {
        // Event detection with overreaction
        let mut heat_delta: f32 = 0.0;

        if player.statistics.red_cards > impression.prev_red_cards {
            impression.bias.quality_offset =
                (impression.bias.quality_offset - 0.5 * negativity_bias).clamp(-3.0, 3.0);
            impression.bias.disappointments = impression.bias.disappointments.saturating_add(1);
            impression.bias.overreaction_timer = (3.0 + volatility * 3.0) as u8;
            impression.bias.overreaction_magnitude = -1.5 * volatility;
            impression.coach_trust = (impression.coach_trust - 1.5 * volatility).clamp(0.0, 10.0);
            heat_delta += 0.15 * volatility;
        }

        if impression.prev_avg_rating > 0.0 && regressed_avg < impression.prev_avg_rating - 0.5 {
            impression.bias.quality_offset =
                (impression.bias.quality_offset - 0.3 * negativity_bias).clamp(-3.0, 3.0);
            if volatility > 0.4 {
                impression.bias.overreaction_timer = impression
                    .bias
                    .overreaction_timer
                    .max((2.0 + volatility * 2.0) as u8);
                impression.bias.overreaction_magnitude =
                    (impression.bias.overreaction_magnitude - 0.8 * volatility).clamp(-3.0, 3.0);
            }
            heat_delta += 0.10 * volatility;
        }

        if player.statistics.goals > impression.prev_goals + 2 {
            impression.bias.quality_offset =
                (impression.bias.quality_offset + 0.3).clamp(-3.0, 3.0);
            impression.bias.overreaction_timer = impression
                .bias
                .overreaction_timer
                .max((2.0 + volatility) as u8);
            impression.bias.overreaction_magnitude =
                (impression.bias.overreaction_magnitude + 1.0 * volatility).clamp(-3.0, 3.0);
        }

        if player.behaviour.is_poor() {
            impression.bias.quality_offset =
                (impression.bias.quality_offset - 0.4 * negativity_bias).clamp(-3.0, 3.0);
            impression.bias.disappointments = impression.bias.disappointments.saturating_add(1);
            impression.bias.overreaction_timer = (2.0 + volatility * 2.0) as u8;
            impression.bias.overreaction_magnitude = -volatility;
            heat_delta += 0.10 * volatility;
        }

        // "Clearly performing well" bump: regressed > 7.5 means the
        // player is genuinely outperforming the league-neutral baseline
        // by enough margin to register with the coach even after the
        // sample-size discount. The 4-app floor remains as a basic
        // observability gate.
        if regressed_avg > 7.5 && player.statistics.played + player.statistics.played_subs > 3 {
            impression.bias.quality_offset =
                (impression.bias.quality_offset + 0.3).clamp(-3.0, 3.0);
        }

        impression.prev_red_cards = player.statistics.red_cards;
        impression.prev_goals = player.statistics.goals;
        impression.prev_avg_rating = regressed_avg;
        heat_delta
    }

    /// The counters that move on their own every week: overreaction
    /// cooling, perception drift and bias decay.
    fn apply_bias_drift(
        impression: &mut PlayerImpression,
        coach_seed: u32,
        player_id: u32,
        current_week: u32,
        judging_accuracy: f32,
    ) {
        // Overreaction timer countdown
        if impression.bias.overreaction_timer > 0 {
            impression.bias.overreaction_timer -= 1;
            impression.bias.quality_offset = (impression.bias.quality_offset
                + impression.bias.overreaction_magnitude * 0.15)
                .clamp(-3.0, 3.0);
        } else {
            impression.bias.overreaction_magnitude *= 0.5;
        }

        // Perception drift
        let drift_noise =
            PerceptionNoise::raw(coach_seed, player_id, current_week.wrapping_add(0xDFFF))
                * 0.12
                * (1.0 - judging_accuracy);
        impression.bias.perception_drift =
            (impression.bias.perception_drift * 0.97 + drift_noise).clamp(-2.0, 2.0);

        // Bias decay
        impression.bias.quality_offset *= 0.98;
        let offset_noise =
            PerceptionNoise::raw(coach_seed, player_id, current_week.wrapping_add(0xD01F))
                * 0.05
                * (1.0 - judging_accuracy);
        impression.bias.quality_offset =
            (impression.bias.quality_offset + offset_noise).clamp(-3.0, 3.0);

        impression.bias.sunk_cost *= 0.95;
    }

    /// Blend the new observations into what the coach already believed.
    fn blend_observations(
        impression: &mut PlayerImpression,
        readings: PlayerReadings,
        trust_in_decisions: f32,
        confirmation_bias: f32,
        negativity_bias: f32,
        visibility: f32,
    ) {
        let base_blend = trust_in_decisions * 0.6;
        let delta = readings.quality - impression.perceived_quality;

        let direction_matches =
            (delta > 0.0) == (impression.perceived_quality >= impression.bias.first_impression);
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
            impression.perceived_quality * old_weight + readings.quality * new_weight;
        impression.match_readiness = readings.readiness;
        impression.potential_impression =
            impression.potential_impression * old_weight + readings.potential * new_weight;
        impression.training_impression =
            impression.training_impression * old_weight + readings.training * new_weight;
    }

    /// Decay a recent move after its protection window.
    fn expire_recent_move(impression: &mut PlayerImpression, current_week: u32, stubbornness: f32) {
        if let Some(ref mv) = impression.recent_move {
            let weeks_since = current_week.saturating_sub(mv.week);
            let base_protection = (4.0 * stubbornness).max(2.0) as u32;
            let sunk_cost_extension = (impression.bias.sunk_cost * 0.5) as u32;
            let protection_weeks = base_protection + sunk_cost_extension;
            if weeks_since > protection_weeks {
                impression.recent_move = None;
            }
        }
    }

    /// Record a move for inertia tracking
    pub fn record_move(&mut self, player_id: u32, move_type: RecentMoveType, date: NaiveDate) {
        let week = PerceptionNoise::week(date);
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
        if let Some(impression) = self.impressions.get(&player_id)
            && let Some(ref mv) = impression.recent_move
            && protecting_moves.contains(&mv.move_type)
        {
            let weeks_since = self.current_week.saturating_sub(mv.week);
            let base_protection = (4.0 * self.profile.stubbornness).max(2.0) as u32;
            let sunk_cost_extension = (impression.bias.sunk_cost * 0.5) as u32;
            let protection_weeks = base_protection + sunk_cost_extension;
            return weeks_since <= protection_weeks;
        }
        false
    }

    /// Get cached impression for a player
    pub fn get_impression(&self, player_id: u32) -> Option<&PlayerImpression> {
        self.impressions.get(&player_id)
    }
}
