use crate::utils::DateUtils;
use crate::{
    ContractType, Player, PlayerFieldPositionGroup, PlayerSquadStatus, PlayerStatusType, Team,
};
use chrono::NaiveDate;

pub fn legacy_estimate_player_quality(player: &Player) -> f32 {
    let tech = player.skills.technical.average();
    let mental = player.skills.mental.average();
    let physical = player.skills.physical.average();
    let skill_composite = tech * 0.40 + mental * 0.35 + physical * 0.25;
    let position_level = player.positions.positions.iter()
        .map(|p| p.level).max().unwrap_or(0) as f32;
    let base = skill_composite * 0.75 + position_level * 0.25;
    let form_bonus = if player.statistics.played + player.statistics.played_subs > 3 {
        (player.statistics.average_rating - 6.5).clamp(-1.5, 1.5)
    } else {
        0.0
    };
    let noise = ((player.id.wrapping_mul(2654435761)) >> 24) as f32 / 128.0 - 1.0;
    base + form_bonus + noise
}

fn legacy_estimate_youth_potential(player: &Player, date: NaiveDate) -> f32 {
    let quality = legacy_estimate_player_quality(player);
    let age = DateUtils::age(player.birth_date, date);
    let age_bonus = match age {
        0..=15 => 1.5, 16..=17 => 2.5, 18 => 3.0, 19..=20 => 2.0, 21..=22 => 1.0, _ => 0.0,
    };
    let attitude = (player.attributes.professionalism + player.skills.mental.determination) / 2.0;
    let attitude_bonus = (attitude - 10.0).clamp(-1.0, 2.0) * 0.5;
    quality + age_bonus + attitude_bonus
}

fn legacy_recall_priority_score(player: &Player) -> f32 {
    let quality = legacy_estimate_player_quality(player);
    let status_bonus = match player.contract.as_ref().map(|c| &c.squad_status) {
        Some(PlayerSquadStatus::KeyPlayer) => 3.0,
        Some(PlayerSquadStatus::FirstTeamRegular) => 2.0,
        Some(PlayerSquadStatus::FirstTeamSquadRotation) => 1.0,
        Some(PlayerSquadStatus::MainBackupPlayer) => 0.5,
        Some(PlayerSquadStatus::HotProspectForTheFuture) => 0.3,
        Some(PlayerSquadStatus::DecentYoungster) => 0.1,
        Some(PlayerSquadStatus::NotNeeded) => -5.0,
        _ => 0.0,
    };
    let condition = (player.player_attributes.condition_percentage() as f32 / 100.0).clamp(0.3, 1.0);
    (quality + status_bonus) * condition
}

pub fn legacy_identify_demotions(main_team: &Team, _date: NaiveDate) -> Vec<u32> {
    let players = &main_team.players.players;
    let squad_size = players.len();
    let mut demotions = Vec::new();
    if players.is_empty() { return demotions; }

    let avg_quality: f32 = players.iter()
        .map(|p| legacy_estimate_player_quality(p))
        .sum::<f32>() / squad_size as f32;

    for player in players {
        let statuses = player.statuses.get();
        if statuses.contains(&PlayerStatusType::Lst) { demotions.push(player.id); continue; }
        if statuses.contains(&PlayerStatusType::Loa) { demotions.push(player.id); continue; }
        if let Some(ref contract) = player.contract {
            if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                demotions.push(player.id); continue;
            }
        }
        if let Some(ref contract) = player.contract {
            if matches!(contract.squad_status,
                PlayerSquadStatus::HotProspectForTheFuture | PlayerSquadStatus::DecentYoungster
            ) {
                let quality = legacy_estimate_player_quality(player);
                if quality < avg_quality - 1.0 && squad_size > 20 {
                    demotions.push(player.id); continue;
                }
            }
        }
        if squad_size > 20 {
            let quality = legacy_estimate_player_quality(player);
            if quality < avg_quality - 3.0 {
                let player_group = player.position().position_group();
                let others = players.iter()
                    .filter(|p| p.id != player.id
                        && p.position().position_group() == player_group
                        && !p.player_attributes.is_injured)
                    .count();
                if others >= 2 { demotions.push(player.id); continue; }
            }
        }
    }

    let remaining = squad_size - demotions.len();
    if remaining > 25 {
        let excess = remaining - 25;
        let mut candidates: Vec<_> = players.iter()
            .filter(|p| !demotions.contains(&p.id))
            .map(|p| (p.id, legacy_estimate_player_quality(p)))
            .collect();
        candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        for (id, _) in candidates.into_iter().take(excess) { demotions.push(id); }
    }
    demotions
}

pub fn legacy_identify_recalls(
    main_team: &Team, reserve_team: &Team, _date: NaiveDate, excluded_ids: &[u32],
) -> Vec<u32> {
    let main_players = &main_team.players.players;
    let reserve_players = &reserve_team.players.players;
    let mut recalls = Vec::new();
    if reserve_players.is_empty() { return recalls; }

    let mut candidates: Vec<&Player> = reserve_players.iter()
        .filter(|p| {
            let statuses = p.statuses.get();
            !statuses.contains(&PlayerStatusType::Lst)
                && !statuses.contains(&PlayerStatusType::Loa)
                && !p.player_attributes.is_injured
                && !matches!(p.contract.as_ref().map(|c| &c.contract_type), Some(ContractType::Loan))
                && p.player_attributes.condition_percentage() > 40
                && !excluded_ids.contains(&p.id)
                && !matches!(p.contract.as_ref().map(|c| &c.squad_status), Some(PlayerSquadStatus::NotNeeded))
        }).collect();
    candidates.sort_by(|a, b| {
        legacy_recall_priority_score(b).partial_cmp(&legacy_recall_priority_score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Quality threshold: worst first-team player
    let min_main_quality = main_players.iter()
        .map(|p| legacy_estimate_player_quality(p))
        .fold(f32::INFINITY, f32::min);

    // Phase 1: Recall all quality players that belong in the first team
    for candidate in &candidates {
        let quality = legacy_estimate_player_quality(candidate);
        if quality >= min_main_quality && !recalls.contains(&candidate.id) {
            recalls.push(candidate.id);
        }
    }

    // Phase 2: Fill position needs
    let available_main: Vec<&Player> = main_players.iter()
        .filter(|p| !p.player_attributes.is_injured).collect();
    let count_by_group = |group: PlayerFieldPositionGroup| -> usize {
        available_main.iter().filter(|p| p.position().position_group() == group).count()
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
                if recalled >= needed { break; }
                if candidate.position().position_group() == *group && !recalls.contains(&candidate.id) {
                    recalls.push(candidate.id); recalled += 1;
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
            if recalled >= needed { break; }
            if !recalls.contains(&candidate.id) { recalls.push(candidate.id); recalled += 1; }
        }
    }

    recalls
}

pub fn legacy_identify_youth_promotions(main_team: &Team, youth_team: &Team, date: NaiveDate) -> Vec<u32> {
    let main_size = main_team.players.players.len();
    let mut promotions = Vec::new();
    if main_size >= 18 { return promotions; }
    let needed = 18 - main_size;

    let avg_quality: f32 = if main_team.players.players.is_empty() { 10.0 } else {
        main_team.players.players.iter()
            .map(|p| legacy_estimate_player_quality(p)).sum::<f32>()
            / main_team.players.players.len() as f32
    };

    let mut candidates: Vec<&Player> = youth_team.players.players.iter()
        .filter(|p| {
            let age = DateUtils::age(p.birth_date, date);
            let quality = legacy_estimate_player_quality(p);
            let youth_potential = legacy_estimate_youth_potential(p, date);
            age >= 16 && !p.player_attributes.is_injured
                && p.player_attributes.condition_percentage() > 40
                && (quality >= avg_quality - 2.0 || youth_potential > avg_quality + 2.0)
        }).collect();
    candidates.sort_by(|a, b| {
        legacy_estimate_youth_potential(b, date)
            .partial_cmp(&legacy_estimate_youth_potential(a, date))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for candidate in candidates.into_iter().take(needed) { promotions.push(candidate.id); }
    promotions
}

pub fn legacy_identify_ability_swaps(main_team: &Team, reserve_team: &Team, _date: NaiveDate) -> Vec<(u32, u32)> {
    const SWAP_THRESHOLD: f32 = 2.0;
    let mut swaps = Vec::new();
    let mut used_main = Vec::new();
    let mut used_reserve = Vec::new();

    let reserve_candidates: Vec<&Player> = reserve_team.players.players.iter()
        .filter(|p| {
            let st = p.statuses.get();
            !p.player_attributes.is_injured && !p.player_attributes.is_banned
                && !st.contains(&PlayerStatusType::Lst) && !st.contains(&PlayerStatusType::Loa)
                && !matches!(p.contract.as_ref().map(|c| &c.contract_type), Some(ContractType::Loan))
                && p.player_attributes.condition_percentage() > 50
        }).collect();

    for group in &[
        PlayerFieldPositionGroup::Goalkeeper, PlayerFieldPositionGroup::Defender,
        PlayerFieldPositionGroup::Midfielder, PlayerFieldPositionGroup::Forward,
    ] {
        let mut main_group: Vec<&Player> = main_team.players.players.iter()
            .filter(|p| p.position().position_group() == *group
                && !used_main.contains(&p.id)
                && !p.statuses.get().contains(&PlayerStatusType::Lst))
            .collect();
        main_group.sort_by(|a, b| {
            legacy_estimate_player_quality(a).partial_cmp(&legacy_estimate_player_quality(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut res_group: Vec<&&Player> = reserve_candidates.iter()
            .filter(|p| p.position().position_group() == *group && !used_reserve.contains(&p.id))
            .collect();
        res_group.sort_by(|a, b| {
            legacy_estimate_player_quality(b).partial_cmp(&legacy_estimate_player_quality(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for main_p in &main_group {
            if res_group.is_empty() { break; }
            let best_res = res_group[0];
            if legacy_estimate_player_quality(best_res)
                > legacy_estimate_player_quality(main_p) + SWAP_THRESHOLD
            {
                swaps.push((main_p.id, best_res.id));
                used_main.push(main_p.id); used_reserve.push(best_res.id);
                res_group.remove(0);
            } else { break; }
        }
    }
    swaps
}
