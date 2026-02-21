use crate::club::team::coach_perception::{
    CoachDecisionState, RecentMoveType, seeded_decision, sigmoid_probability,
};
use crate::utils::DateUtils;
use crate::{PlayerSquadStatus, PlayerStatusType, Team};
use chrono::NaiveDate;

use super::legacy;

pub struct DemotionEvaluator;

impl DemotionEvaluator {
    pub fn evaluate(
        teams: &[Team],
        main_idx: usize,
        coach_state: Option<&CoachDecisionState>,
        date: NaiveDate,
    ) -> Vec<u32> {
        let main_team = &teams[main_idx];
        let state = match coach_state {
            Some(s) => s,
            None => return legacy::legacy_identify_demotions(main_team, date),
        };

        let players = &main_team.players.players;
        let squad_size = players.len();
        let mut demotions = Vec::new();

        if players.is_empty() {
            return demotions;
        }

        let profile = &state.profile;
        let emotional_heat = state.emotional_heat;

        let avg_quality: f32 = players
            .iter()
            .map(|p| {
                state
                    .impressions
                    .get(&p.id)
                    .map(|imp| imp.perceived_quality)
                    .unwrap_or_else(|| state.perceived_quality(p, date))
            })
            .sum::<f32>()
            / squad_size as f32;

        for player in players {
            let statuses = player.statuses.get();

            // Administrative demotions stay deterministic
            if statuses.contains(&PlayerStatusType::Lst) {
                demotions.push(player.id);
                continue;
            }
            if statuses.contains(&PlayerStatusType::Loa) {
                demotions.push(player.id);
                continue;
            }
            if let Some(ref contract) = player.contract {
                if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                    demotions.push(player.id);
                    continue;
                }
            }

            // Inertia protection
            if state.is_protected(
                player.id,
                &[
                    RecentMoveType::PromotedToFirst,
                    RecentMoveType::RecalledFromReserves,
                    RecentMoveType::YouthPromoted,
                    RecentMoveType::SwappedIn,
                ],
            ) {
                continue;
            }

            let perceived = state
                .impressions
                .get(&player.id)
                .map(|imp| imp.perceived_quality)
                .unwrap_or_else(|| state.perceived_quality(player, date));

            let coach_trust = state
                .impressions
                .get(&player.id)
                .map(|imp| imp.coach_trust)
                .unwrap_or(5.0);

            let (sunk_cost, disappointments) = state
                .impressions
                .get(&player.id)
                .map(|imp| (imp.bias.sunk_cost, imp.bias.disappointments))
                .unwrap_or((0.0, 0));

            // Staleness blindspot: coach hasn't observed this player recently
            let observation_staleness = state.impressions.get(&player.id)
                .map(|imp| state.current_week.saturating_sub(imp.bias.last_observation_week))
                .unwrap_or(0);
            let staleness_blindness = if observation_staleness > 4 {
                1.0 - (observation_staleness.min(12) as f32 / 24.0)
            } else {
                1.0
            };

            let age = DateUtils::age(player.birth_date, date);

            // --- Patience snap: emotional coach turns on disappointing player ---
            if emotional_heat > 0.4 && disappointments >= 2 && squad_size > 18 {
                let snap_intensity = (emotional_heat - 0.3) * (disappointments as f32 / 4.0);
                let snap_prob = (snap_intensity * 0.4).clamp(0.0, 0.5);
                let snap_seed = profile.coach_seed
                    .wrapping_mul(player.id)
                    .wrapping_add(state.current_week.wrapping_mul(0xBAAD));
                if seeded_decision(snap_prob, snap_seed) {
                    let player_group = player.position().position_group();
                    let others = players.iter()
                        .filter(|p| {
                            p.id != player.id
                                && p.position().position_group() == player_group
                                && !p.player_attributes.is_injured
                                && !demotions.contains(&p.id)
                        })
                        .count();
                    if others >= 2 {
                        demotions.push(player.id);
                        continue;
                    }
                }
            }

            // Hot prospects / youngsters below average
            if let Some(ref contract) = player.contract {
                if matches!(
                    contract.squad_status,
                    PlayerSquadStatus::HotProspectForTheFuture
                        | PlayerSquadStatus::DecentYoungster
                ) {
                    let youth_protection = profile.youth_preference * 1.5;
                    let gap = avg_quality - perceived - youth_protection;
                    let steepness = 1.5 - profile.conservatism * 0.5;
                    let prob = sigmoid_probability(gap - 1.0, steepness);
                    let trust_factor = 1.0 - (coach_trust / 10.0) * 0.3;
                    let sunk_cost_factor = 1.0 - (sunk_cost / 10.0) * 0.4;
                    let disappointment_factor = if disappointments >= 3 { 1.3 } else { 1.0 };
                    let final_prob = prob * trust_factor * sunk_cost_factor
                        * disappointment_factor * staleness_blindness;

                    if squad_size > 20 {
                        let seed = profile.coach_seed
                            .wrapping_mul(player.id)
                            .wrapping_add(state.current_week);
                        if seeded_decision(final_prob, seed) {
                            demotions.push(player.id);
                            continue;
                        }
                    }
                }
            }

            // Players significantly below squad average
            if squad_size > 20 {
                let gap_required = 3.0 + profile.conservatism * 1.5;
                let gap = avg_quality - perceived;

                let youth_modifier = if age <= 22 {
                    profile.youth_preference * 1.0
                } else {
                    0.0
                };

                let steepness = 1.0 - profile.conservatism * 0.3;
                let prob = sigmoid_probability(gap - gap_required - youth_modifier, steepness);
                let trust_factor = 1.0 - (coach_trust / 10.0) * 0.3;
                let sunk_cost_factor = 1.0 - (sunk_cost / 10.0) * 0.4;
                let disappointment_factor = if disappointments >= 3 { 1.3 } else { 1.0 };
                let final_prob = prob * trust_factor * sunk_cost_factor
                    * disappointment_factor * staleness_blindness;

                let seed = profile.coach_seed
                    .wrapping_mul(player.id)
                    .wrapping_add(state.current_week.wrapping_mul(3));
                if seeded_decision(final_prob, seed) {
                    let player_group = player.position().position_group();
                    let others_in_position = players
                        .iter()
                        .filter(|p| {
                            p.id != player.id
                                && p.position().position_group() == player_group
                                && !p.player_attributes.is_injured
                                && !demotions.contains(&p.id)
                        })
                        .count();
                    if others_in_position >= 2 {
                        demotions.push(player.id);
                        continue;
                    }
                }
            }
        }

        // Force demote if squad > 25
        let remaining = squad_size - demotions.len();
        if remaining > 25 {
            let excess = remaining - 25;
            let mut candidates: Vec<_> = players
                .iter()
                .filter(|p| !demotions.contains(&p.id))
                .map(|p| {
                    let q = state
                        .impressions
                        .get(&p.id)
                        .map(|imp| imp.perceived_quality)
                        .unwrap_or_else(|| state.perceived_quality(p, date));
                    (p.id, q)
                })
                .collect();
            candidates.sort_by(|a, b| {
                a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
            });
            for (id, _) in candidates.into_iter().take(excess) {
                demotions.push(id);
            }
        }

        demotions
    }

    /// Mandatory administrative demotions only: Lst (transfer-listed) and Loa (leave of absence).
    /// These bypass the trigger system because they are not coaching decisions.
    /// NotNeeded and performance-based demotions go through the trigger-gated weekly review.
    pub fn identify_administrative_demotions(main_team: &Team) -> Vec<u32> {
        main_team
            .players
            .players
            .iter()
            .filter_map(|player| {
                let statuses = player.statuses.get();
                if statuses.contains(&PlayerStatusType::Lst)
                    || statuses.contains(&PlayerStatusType::Loa)
                {
                    return Some(player.id);
                }
                None
            })
            .collect()
    }
}
