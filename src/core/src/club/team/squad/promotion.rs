use crate::club::team::coach_perception::{
    CoachDecisionState, seeded_decision, sigmoid_probability,
};
use crate::utils::DateUtils;
use crate::Team;
use chrono::NaiveDate;

use super::legacy;

pub struct YouthPromotionEvaluator;

impl YouthPromotionEvaluator {
    pub fn evaluate(
        teams: &[Team],
        main_idx: usize,
        youth_idx: usize,
        coach_state: Option<&CoachDecisionState>,
        date: NaiveDate,
    ) -> Vec<u32> {
        let main_team = &teams[main_idx];
        let youth_team = &teams[youth_idx];
        let main_size = main_team.players.players.len();
        let mut promotions = Vec::new();

        let state = match coach_state {
            Some(s) => s,
            None => return legacy::legacy_identify_youth_promotions(main_team, youth_team, date),
        };

        let profile = &state.profile;

        let promotion_ceiling = (18.0 + profile.youth_preference * 4.0) as usize;
        if main_size >= promotion_ceiling {
            return promotions;
        }

        let needed = promotion_ceiling - main_size;

        let avg_perceived: f32 = if main_team.players.players.is_empty() {
            10.0
        } else {
            main_team
                .players
                .players
                .iter()
                .map(|p| {
                    state
                        .impressions
                        .get(&p.id)
                        .map(|imp| imp.perceived_quality)
                        .unwrap_or_else(|| state.perceived_quality(p, date))
                })
                .sum::<f32>()
                / main_team.players.players.len() as f32
        };

        // Philosophy bonus: youth-loving coaches with thin squads lower the bar
        let philosophy_bonus = if profile.youth_preference > 0.6 && main_size < 20 {
            (profile.youth_preference - 0.5) * 3.0
        } else {
            0.0
        };

        let threshold = avg_perceived - 2.0 - profile.risk_tolerance * 2.0 - philosophy_bonus;

        // Promotion candidates (spotlight is already in potential_impression)
        let mut candidates: Vec<(u32, f32)> = youth_team
            .players
            .players
            .iter()
            .filter_map(|p| {
                let age = DateUtils::age(p.birth_date, date);
                if age < 16 || p.player_attributes.is_injured
                    || p.player_attributes.condition_percentage() <= 40
                {
                    return None;
                }

                let potential = state
                    .impressions
                    .get(&p.id)
                    .map(|imp| imp.potential_impression)
                    .unwrap_or_else(|| state.potential_impression(p, date));

                let quality = state
                    .impressions
                    .get(&p.id)
                    .map(|imp| imp.perceived_quality)
                    .unwrap_or_else(|| state.perceived_quality(p, date));

                let training = state
                    .impressions
                    .get(&p.id)
                    .map(|imp| imp.training_impression)
                    .unwrap_or_else(|| state.training_impression(p));

                let score = potential * 0.4 + quality * 0.3 + training * 0.3;

                let steepness = 1.0 + profile.risk_tolerance * 0.5;
                let prob = sigmoid_probability(score - threshold, steepness);

                let seed = profile.coach_seed
                    .wrapping_mul(p.id)
                    .wrapping_add(state.current_week.wrapping_mul(7));

                if seeded_decision(prob, seed) {
                    Some((p.id, score))
                } else {
                    None
                }
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });

        for (id, _) in candidates.into_iter().take(needed) {
            promotions.push(id);
        }

        promotions
    }
}
