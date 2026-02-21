use crate::club::team::coach_perception::{
    CoachDecisionState, RecentMoveType, seeded_decision, sigmoid_probability,
};
use crate::{
    ContractType, Player, PlayerFieldPositionGroup, PlayerStatusType, Team,
};
use chrono::NaiveDate;

use super::legacy;

pub struct AbilitySwapEvaluator;

impl AbilitySwapEvaluator {
    pub fn evaluate(
        teams: &[Team],
        main_idx: usize,
        reserve_idx: usize,
        coach_state: Option<&CoachDecisionState>,
        date: NaiveDate,
    ) -> Vec<(u32, u32)> {
        let main_team = &teams[main_idx];
        let reserve_team = &teams[reserve_idx];

        let state = match coach_state {
            Some(s) => s,
            None => return legacy::legacy_identify_ability_swaps(main_team, reserve_team, date),
        };

        let profile = &state.profile;

        let max_swaps = (2.0 * (1.0 - profile.conservatism * 0.5)).ceil() as usize;

        // Emotional urgency lowers swap threshold (panic changes)
        let emotional_urgency = state.emotional_heat * 0.5;
        let swap_threshold = (1.5 + profile.conservatism * 1.5 - emotional_urgency).max(0.5);

        let mut swaps = Vec::new();
        let mut used_main = Vec::new();
        let mut used_reserve = Vec::new();

        let reserve_candidates: Vec<&Player> = reserve_team
            .players
            .players
            .iter()
            .filter(|p| {
                let st = p.statuses.get();
                !p.player_attributes.is_injured
                    && !p.player_attributes.is_banned
                    && !st.contains(&PlayerStatusType::Lst)
                    && !st.contains(&PlayerStatusType::Loa)
                    && !matches!(
                        p.contract.as_ref().map(|c| &c.contract_type),
                        Some(ContractType::Loan)
                    )
                    && p.player_attributes.condition_percentage() > 50
            })
            .collect();

        let swap_score = |p: &Player| -> f32 {
            let perceived = state
                .impressions
                .get(&p.id)
                .map(|imp| imp.perceived_quality)
                .unwrap_or_else(|| state.perceived_quality(p, date));
            let readiness = state
                .impressions
                .get(&p.id)
                .map(|imp| imp.match_readiness)
                .unwrap_or_else(|| state.match_readiness(p));
            perceived * 0.7 + readiness * 0.3
        };

        for group in &[
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ] {
            if swaps.len() >= max_swaps {
                break;
            }

            let mut main_group: Vec<&Player> = main_team
                .players
                .players
                .iter()
                .filter(|p| {
                    p.position().position_group() == *group
                        && !used_main.contains(&p.id)
                        && !p.statuses.get().contains(&PlayerStatusType::Lst)
                        && !state.is_protected(
                            p.id,
                            &[
                                RecentMoveType::SwappedIn,
                                RecentMoveType::RecalledFromReserves,
                                RecentMoveType::YouthPromoted,
                            ],
                        )
                })
                .collect();
            main_group.sort_by(|a, b| {
                swap_score(a)
                    .partial_cmp(&swap_score(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut res_group: Vec<&&Player> = reserve_candidates
                .iter()
                .filter(|p| {
                    p.position().position_group() == *group && !used_reserve.contains(&p.id)
                })
                .collect();
            res_group.sort_by(|a, b| {
                swap_score(b)
                    .partial_cmp(&swap_score(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            for main_p in &main_group {
                if res_group.is_empty() || swaps.len() >= max_swaps {
                    break;
                }
                let best_res = res_group[0];
                let main_score = swap_score(main_p);
                let res_score = swap_score(best_res);
                let gap = res_score - main_score;

                let steepness = 1.0 + (1.0 - profile.conservatism) * 0.5;
                let prob = sigmoid_probability(gap - swap_threshold, steepness);

                let seed = profile.coach_seed
                    .wrapping_mul(main_p.id)
                    .wrapping_add(best_res.id)
                    .wrapping_add(state.current_week.wrapping_mul(11));

                if seeded_decision(prob, seed) {
                    swaps.push((main_p.id, best_res.id));
                    used_main.push(main_p.id);
                    used_reserve.push(best_res.id);
                    res_group.remove(0);
                } else {
                    break;
                }
            }
        }

        swaps
    }
}
