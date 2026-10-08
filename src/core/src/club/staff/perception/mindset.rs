//! What a staff member believes about a player's state of mind.
//!
//! The player's assurance, belief and nerves are his own and are never
//! read here. A coach sees where a man has been playing, whether things
//! have been going wrong for him, how his matches have been going and how
//! much he goes for — and reads his head from that, as well as his own
//! eye allows and no better than his sample does.

use super::utils::perception_noise_raw;
use crate::Staff;
use crate::club::staff::mind::organs::judgements::CoachMemory;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MindsetEstimate {
    /// The standard of football he is believed to be used to.
    pub perceived_assurance: f32,
    /// −1..1 — how confident he is believed to be.
    pub perceived_confidence: f32,
    /// 0..1 — how much the staff member would stake on the read. Zero
    /// when he has never seen him play.
    pub certainty: f32,
}

impl MindsetEstimate {
    /// Nothing seen, nothing claimed: the side's own standard, a neutral
    /// head, and no certainty at all.
    pub fn silent(team_standard: f32) -> Self {
        MindsetEstimate {
            perceived_assurance: team_standard,
            perceived_confidence: 0.0,
            certainty: 0.0,
        }
    }
}

pub struct MindsetEstimator;

impl MindsetEstimator {
    /// Matches seen before the read is as firm as his eye allows.
    const FULL_LOOK: f32 = 10.0;
    const ASSURANCE_WIDTH: f32 = 0.06;
    const CONFIDENCE_WIDTH: f32 = 0.35;
    /// What one recent costly match and one rating point say about him.
    const ERROR_WEIGHT: f32 = 0.25;
    const RATING_WEIGHT: f32 = 0.30;
    const NEUTRAL_RATING: f32 = 6.6;
    const SALT: u32 = 0x4D49_4E44;

    /// `memory` is the staff member's dossier on the player, `team_standard`
    /// the standard of the side he belongs to, `week` the read's week.
    pub fn estimate(
        staff: &Staff,
        memory: Option<&CoachMemory>,
        team_standard: f32,
        player_id: u32,
        week: u32,
    ) -> MindsetEstimate {
        let Some(m) = memory.filter(|m| m.matches_observed > 0) else {
            return MindsetEstimate::silent(team_standard);
        };
        let knowledge = &staff.staff_attributes.knowledge;
        let judging = (knowledge.judging_player_ability as f32 / 20.0).clamp(0.0, 1.0);
        let looks = (m.matches_observed as f32 / Self::FULL_LOOK).clamp(0.0, 1.0);
        let width = (1.2 - judging) * (1.0 - 0.5 * looks);
        let noise = |salt: u32| perception_noise_raw(staff.id, player_id, Self::SALT + salt + week * 2);

        let assurance = m.observed_standard.unwrap_or(team_standard)
            + noise(0) * Self::ASSURANCE_WIDTH * width;
        let confidence = -(m.recent_errors() as f32) * Self::ERROR_WEIGHT
            + (m.recent_rating_ema - Self::NEUTRAL_RATING) * Self::RATING_WEIGHT
            + noise(1) * Self::CONFIDENCE_WIDTH * width;
        MindsetEstimate {
            perceived_assurance: assurance,
            perceived_confidence: confidence.clamp(-1.0, 1.0),
            certainty: looks * (0.5 + 0.5 * judging),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::staff::StaffStub;
    use crate::club::staff::mind::organs::judgements::{CoachMatchObservation, CoachMemoryStore};
    use crate::club::staff::perception::CoachProfile;
    use chrono::NaiveDate;

    fn observation(player_id: u32, standard: f32, errors: u16, day: i64) -> CoachMatchObservation {
        CoachMatchObservation {
            player_id,
            effective_rating: 6.6,
            minutes_played: 90,
            is_starter: true,
            match_importance: 0.5,
            is_cup: false,
            is_derby: false,
            is_continental: false,
            goals: 0,
            assists: 0,
            errors_leading_to_goal: errors,
            errors_leading_to_shot: errors,
            keeper_claims: 2,
            standard_of_football: standard,
            yellow_cards: 0,
            red_cards: 0,
            team_won: true,
            was_substituted_early: false,
            role_fit: 1.0,
            professionalism_signal: 0.5,
            date: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap() + chrono::Duration::days(day * 7),
        }
    }

    fn judge(judging: u8) -> Staff {
        let mut staff = StaffStub::build();
        staff.id = 7;
        staff.staff_attributes.knowledge.judging_player_ability = judging;
        staff
    }

    #[test]
    fn a_coach_remembers_where_a_man_played_and_what_went_wrong() {
        let mut store = CoachMemoryStore::new();
        let profile = CoachProfile::from_staff(&judge(12));
        store.observe(&observation(5, 0.70, 0, 0), &profile);
        store.observe(&observation(5, 0.70, 1, 1), &profile);
        let m = store.get(5).unwrap();
        assert!((m.observed_standard.unwrap() - 0.70).abs() < 1e-6);
        assert_eq!(m.recent_errors(), 1);
        assert!(m.claims_ema > 0.0);
    }

    #[test]
    fn nothing_seen_nothing_claimed() {
        let est = MindsetEstimator::estimate(&judge(15), None, 0.62, 5, 100);
        assert_eq!(est, MindsetEstimate::silent(0.62));
        assert_eq!(est.certainty, 0.0);
    }

    #[test]
    fn errors_are_noticed() {
        let profile = CoachProfile::from_staff(&judge(12));
        let mut clean = CoachMemoryStore::new();
        let mut shaky = CoachMemoryStore::new();
        for day in 0..4 {
            clean.observe(&observation(5, 0.66, 0, day), &profile);
            shaky.observe(&observation(5, 0.66, 1, day), &profile);
        }
        let staff = judge(12);
        let calm = MindsetEstimator::estimate(&staff, clean.get(5), 0.66, 5, 200);
        let rattled = MindsetEstimator::estimate(&staff, shaky.get(5), 0.66, 5, 200);
        assert!(rattled.perceived_confidence < calm.perceived_confidence);
    }

    #[test]
    fn a_better_judge_reads_him_more_accurately() {
        let profile = CoachProfile::from_staff(&judge(12));
        let (good, poor) = (judge(19), judge(3));
        let mut good_err = 0.0;
        let mut poor_err = 0.0;
        for player_id in 1..400u32 {
            let mut store = CoachMemoryStore::new();
            for day in 0..3 {
                store.observe(&observation(player_id, 0.64, 0, day), &profile);
            }
            let m = store.get(player_id);
            good_err += (MindsetEstimator::estimate(&good, m, 0.6, player_id, 9).perceived_assurance
                - 0.64)
                .abs();
            poor_err += (MindsetEstimator::estimate(&poor, m, 0.6, player_id, 9).perceived_assurance
                - 0.64)
                .abs();
        }
        assert!(good_err < poor_err);
    }
}
