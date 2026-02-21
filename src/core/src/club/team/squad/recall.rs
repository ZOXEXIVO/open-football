use crate::club::team::coach_perception::CoachDecisionState;
use crate::{
    ContractType, Player, PlayerFieldPositionGroup, PlayerSquadStatus, PlayerStatusType, Team,
};
use chrono::NaiveDate;

use super::legacy;

pub struct RecallEvaluator;

impl RecallEvaluator {
    pub fn evaluate(
        teams: &[Team],
        main_idx: usize,
        reserve_idx: usize,
        coach_state: Option<&CoachDecisionState>,
        date: NaiveDate,
    ) -> Vec<u32> {
        let main_team = &teams[main_idx];
        let reserve_team = &teams[reserve_idx];
        let main_players = &main_team.players.players;
        let reserve_players = &reserve_team.players.players;
        let mut recalls = Vec::new();

        if reserve_players.is_empty() {
            return recalls;
        }

        let state = match coach_state {
            Some(s) => s,
            None => return legacy::legacy_identify_recalls(main_team, reserve_team, date, &[]),
        };

        let profile = &state.profile;

        // Eligible candidates: healthy, not on loan, available
        let mut candidates: Vec<&Player> = reserve_players
            .iter()
            .filter(|p| {
                let statuses = p.statuses.get();
                !statuses.contains(&PlayerStatusType::Lst)
                    && !statuses.contains(&PlayerStatusType::Loa)
                    && !p.player_attributes.is_injured
                    && !matches!(
                        p.contract.as_ref().map(|c| &c.contract_type),
                        Some(ContractType::Loan)
                    )
                    && p.player_attributes.condition_percentage() > 40
                    && !matches!(
                        p.contract.as_ref().map(|c| &c.squad_status),
                        Some(PlayerSquadStatus::NotNeeded)
                    )
            })
            .collect();

        // Score each candidate
        let recall_score = |p: &Player| -> f32 {
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
            let trust = state
                .impressions
                .get(&p.id)
                .map(|imp| imp.coach_trust)
                .unwrap_or(5.0);

            let status_bonus = match p.contract.as_ref().map(|c| &c.squad_status) {
                Some(PlayerSquadStatus::KeyPlayer) => 3.0,
                Some(PlayerSquadStatus::FirstTeamRegular) => 2.0,
                Some(PlayerSquadStatus::FirstTeamSquadRotation) => 1.0,
                Some(PlayerSquadStatus::MainBackupPlayer) => 0.5,
                Some(PlayerSquadStatus::HotProspectForTheFuture) => {
                    0.3 + profile.youth_preference * 1.0
                }
                Some(PlayerSquadStatus::DecentYoungster) => {
                    0.1 + profile.youth_preference * 0.5
                }
                _ => 0.0,
            };

            perceived * 0.4 + readiness * 0.3 + (trust / 10.0) * 3.0 * 0.15
                + status_bonus * 0.15
        };

        candidates.sort_by(|a, b| {
            recall_score(b)
                .partial_cmp(&recall_score(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Compute quality threshold: worst first-team player quality
        // Any reserve player above this belongs in the first team
        let main_qualities: Vec<f32> = main_players
            .iter()
            .map(|p| {
                state
                    .impressions
                    .get(&p.id)
                    .map(|imp| imp.perceived_quality)
                    .unwrap_or_else(|| state.perceived_quality(p, date))
            })
            .collect();

        let min_main_quality = main_qualities
            .iter()
            .cloned()
            .fold(f32::INFINITY, f32::min);

        // Phase 1: Recall all quality players that belong in the first team
        for candidate in &candidates {
            let perceived = state
                .impressions
                .get(&candidate.id)
                .map(|imp| imp.perceived_quality)
                .unwrap_or_else(|| state.perceived_quality(candidate, date));

            // Player is better than the worst first-team player — recall
            if perceived >= min_main_quality {
                if !recalls.contains(&candidate.id) {
                    recalls.push(candidate.id);
                }
            }
        }

        // Phase 2: Fill position needs
        let available_main: Vec<&Player> = main_players
            .iter()
            .filter(|p| !p.player_attributes.is_injured)
            .collect();

        let count_by_group = |group: PlayerFieldPositionGroup| -> usize {
            available_main
                .iter()
                .filter(|p| p.position().position_group() == group)
                .count()
        };

        let gk_count = count_by_group(PlayerFieldPositionGroup::Goalkeeper);
        let def_count = count_by_group(PlayerFieldPositionGroup::Defender);
        let mid_count = count_by_group(PlayerFieldPositionGroup::Midfielder);
        let fwd_count = count_by_group(PlayerFieldPositionGroup::Forward);

        let tactics = main_team.tactics();
        let positions = tactics.positions();
        let def_need = positions.iter().filter(|p| p.is_defender()).count() + 1;
        let mid_need = positions.iter().filter(|p| p.is_midfielder()).count() + 1;
        let fwd_need = positions.iter().filter(|p| p.is_forward()).count() + 1;

        let position_needs = [
            (PlayerFieldPositionGroup::Goalkeeper, gk_count, 2usize),
            (PlayerFieldPositionGroup::Defender, def_count, def_need),
            (PlayerFieldPositionGroup::Midfielder, mid_count, mid_need),
            (PlayerFieldPositionGroup::Forward, fwd_count, fwd_need),
        ];

        for (group, count, min) in &position_needs {
            if *count < *min {
                let needed = min - count;
                let mut recalled = 0;
                for candidate in &candidates {
                    if recalled >= needed {
                        break;
                    }
                    if candidate.position().position_group() == *group
                        && !recalls.contains(&candidate.id)
                    {
                        recalls.push(candidate.id);
                        recalled += 1;
                    }
                }
            }
        }

        // Phase 3: First team should have at least 18 players
        let current_main_size = main_players.len() + recalls.len();
        if current_main_size < 18 {
            let needed = 18 - current_main_size;
            let mut recalled = 0;
            for candidate in &candidates {
                if recalled >= needed {
                    break;
                }
                if !recalls.contains(&candidate.id) {
                    recalls.push(candidate.id);
                    recalled += 1;
                }
            }
        }

        recalls
    }
}
