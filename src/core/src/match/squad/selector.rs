use crate::club::{PlayerFieldPositionGroup, PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::{Player, PlayerStatusType, Tactics, Team};
use log::{debug, warn};
use std::borrow::Borrow;

pub struct SquadSelector;

const DEFAULT_SQUAD_SIZE: usize = 11;
const DEFAULT_BENCH_SIZE: usize = 7;

pub struct PlayerSelectionResult {
    pub main_squad: Vec<MatchPlayer>,
    pub substitutes: Vec<MatchPlayer>,
}

impl SquadSelector {
    pub fn select(team: &Team, staff: &Staff) -> PlayerSelectionResult {
        Self::select_with_reserves(team, staff, &[])
    }

    /// Select squad with additional reserve/youth players available for selection
    pub fn select_with_reserves(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
    ) -> PlayerSelectionResult {
        let tactics = team.tactics();

        // Collect all available players (first team + reserves), not injured, not banned
        let mut available: Vec<&Player> = team
            .players
            .players()
            .iter()
            .filter(|&&p| {
                !p.player_attributes.is_injured
                    && !p.player_attributes.is_banned
                    && !p.statuses.get().contains(&PlayerStatusType::Lst)
                    && !p.statuses.get().contains(&PlayerStatusType::Loa)
            })
            .copied()
            .collect();

        for &rp in reserve_players {
            if !rp.player_attributes.is_injured
                && !rp.player_attributes.is_banned
                && !rp.statuses.get().contains(&PlayerStatusType::Lst)
                && !rp.statuses.get().contains(&PlayerStatusType::Loa)
                && !available.iter().any(|p| p.id == rp.id)
            {
                available.push(rp);
            }
        }

        let outfield_count = available.iter().filter(|p| !Self::is_goalkeeper_player(p)).count();
        let gk_count = available.len() - outfield_count;

        if available.len() < DEFAULT_SQUAD_SIZE {
            warn!(
                "Squad selection for team {}: only {} available ({} outfield, {} GK, {} reserves offered)",
                team.name, available.len(), outfield_count, gk_count, reserve_players.len()
            );
        } else {
            debug!(
                "Squad selection: {} available ({} outfield, {} GK, {} reserves)",
                available.len(), outfield_count, gk_count, reserve_players.len()
            );
        }

        // Select starting 11
        let main_squad =
            Self::select_starting_eleven(team.id, &available, staff, tactics.borrow());

        // Remaining pool for substitutes
        let remaining: Vec<&Player> = available
            .iter()
            .filter(|p| !main_squad.iter().any(|mp| mp.id == p.id))
            .copied()
            .collect();

        let substitutes =
            Self::select_substitutes(team.id, &remaining, staff, tactics.borrow());

        debug!(
            "Final squad: {} starters, {} subs",
            main_squad.len(),
            substitutes.len()
        );

        PlayerSelectionResult {
            main_squad,
            substitutes,
        }
    }

    // ========== STARTING 11 SELECTION ==========

    /// Select the best starting 11 for the given formation.
    ///
    /// Algorithm:
    /// 1. Always pick the best goalkeeper first
    /// 2. For each tactical position, score ALL available players using position group
    ///    compatibility (not exact position match) combined with condition/readiness/ability
    /// 3. Greedy assignment: fill each slot with the highest-scoring unused player
    /// 4. If any slots remain unfilled, use the best available player regardless of position
    fn select_starting_eleven(
        team_id: u32,
        available: &[&Player],
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let mut squad: Vec<MatchPlayer> = Vec::with_capacity(DEFAULT_SQUAD_SIZE);
        let mut used_ids: Vec<u32> = Vec::new();
        let required = tactics.positions();

        // STEP 1: Goalkeeper — must always be filled
        if let Some(gk) = Self::pick_best_goalkeeper(available, &used_ids) {
            squad.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        } else {
            warn!("No goalkeeper found at all — picking any player as GK");
            if let Some(any) = Self::pick_best_unused(available, &used_ids) {
                squad.push(MatchPlayer::from_player(
                    team_id,
                    any,
                    PlayerPositionType::Goalkeeper,
                    false,
                ));
                used_ids.push(any.id);
            }
        }

        // STEP 2: Fill each outfield position using position-group scoring
        for &pos in required.iter() {
            if pos == PlayerPositionType::Goalkeeper {
                continue;
            }

            let target_group = pos.position_group();

            // Score all unused players for this position
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| !Self::is_goalkeeper_player(p)) // don't pick GKs for outfield
                .max_by(|a, b| {
                    let sa = Self::score_player_for_slot(a, pos, target_group, staff, tactics);
                    let sb = Self::score_player_for_slot(b, pos, target_group, staff, tactics);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();

            if let Some(player) = best {
                squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
        }

        // STEP 3: Fill any remaining slots with best available outfield players
        while squad.len() < DEFAULT_SQUAD_SIZE {
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| !Self::is_goalkeeper_player(p))
                .max_by(|a, b| {
                    let sa = Self::overall_quality(a, staff, tactics);
                    let sb = Self::overall_quality(b, staff, tactics);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = Self::best_tactical_position(player, tactics);
                    squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        // STEP 4: LAST RESORT — use ANY remaining player (even GKs as outfield)
        // Better to play with 11 including a GK in outfield than to play with fewer
        while squad.len() < DEFAULT_SQUAD_SIZE {
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    let sa = a.player_attributes.current_ability;
                    let sb = b.player_attributes.current_ability;
                    sa.cmp(&sb)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = Self::best_tactical_position(player, tactics);
                    warn!("Emergency fill: using {} (GK) as outfield player", player.full_name);
                    squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break, // truly no more players
            }
        }

        if squad.len() < DEFAULT_SQUAD_SIZE {
            warn!(
                "Could only select {} of 11 starting players",
                squad.len()
            );
        }

        squad
    }

    // ========== SUBSTITUTE SELECTION ==========

    fn select_substitutes(
        team_id: u32,
        remaining: &[&Player],
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let mut subs: Vec<MatchPlayer> = Vec::with_capacity(DEFAULT_BENCH_SIZE);
        let mut used_ids: Vec<u32> = Vec::new();

        // 1. Backup goalkeeper (always first on the bench)
        if let Some(gk) = Self::pick_best_goalkeeper(remaining, &used_ids) {
            subs.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        }

        // 2. Fill remaining bench spots with best overall outfield players
        //    Prefer positional variety: try to get at least one defender, midfielder, forward
        for target_group in &[
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ] {
            if subs.len() >= DEFAULT_BENCH_SIZE {
                break;
            }
            // Check if we already have someone from this group on bench
            let has_group = subs.iter().any(|s| {
                s.tactical_position.current_position.position_group() == *target_group
            });
            if has_group {
                continue;
            }

            // Pick the best available from this group
            let best = remaining
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| p.position().position_group() == *target_group)
                .max_by(|a, b| {
                    let sa = Self::overall_quality(a, staff, tactics);
                    let sb = Self::overall_quality(b, staff, tactics);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();

            if let Some(player) = best {
                let pos = Self::best_tactical_position(player, tactics);
                subs.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
        }

        // 3. Fill remaining spots with the best available players by quality
        while subs.len() < DEFAULT_BENCH_SIZE {
            let best = remaining
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    let sa = Self::overall_quality(a, staff, tactics);
                    let sb = Self::overall_quality(b, staff, tactics);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = Self::best_tactical_position(player, tactics);
                    subs.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        subs
    }

    // ========== SCORING ==========

    /// Score a player for a specific tactical slot.
    /// Uses position group compatibility instead of exact position match.
    fn score_player_for_slot(
        player: &Player,
        slot_position: PlayerPositionType,
        slot_group: PlayerFieldPositionGroup,
        staff: &Staff,
        tactics: &Tactics,
    ) -> f32 {
        let mut score: f32 = 0.0;

        // 1. Position fit (35% weight) — based on group compatibility
        let position_fit = Self::position_fit_score(player, slot_position, slot_group);
        score += position_fit * 0.35;

        // 2. Physical condition (20% weight)
        let condition = (player.player_attributes.condition as f32 / 10000.0).clamp(0.0, 1.0);
        score += condition * 20.0 * 0.20;

        // 3. Match readiness (15% weight)
        let readiness = (player.skills.physical.match_readiness / 20.0).clamp(0.0, 1.0);
        score += readiness * 20.0 * 0.15;

        // 4. Current ability (25% weight)
        let ability = player.player_attributes.current_ability as f32 / 200.0;
        score += ability * 20.0 * 0.25;

        // 5. Staff favourite bonus
        if staff.relations.is_favorite_player(player.id) {
            score += 1.0;
        }

        // 6. Tactical style fit (small bonus)
        score += Self::tactical_style_bonus(player, slot_position, tactics) * 0.05;

        score
    }

    /// Calculate how well a player fits a target position.
    /// Returns 0..20 score.
    fn position_fit_score(
        player: &Player,
        slot_position: PlayerPositionType,
        slot_group: PlayerFieldPositionGroup,
    ) -> f32 {
        // Check exact position match first (any level)
        let exact_level = player.positions.get_level(slot_position);
        if exact_level > 0 {
            return exact_level as f32; // 1..20
        }

        // Check if player has any position in the same group
        let player_group = player.position().position_group();

        if player_group == slot_group {
            // Same group but different specific position — good fit
            // Use their primary position level with a penalty
            let primary_level = player
                .positions
                .positions
                .iter()
                .map(|p| p.level)
                .max()
                .unwrap_or(0);
            return primary_level as f32 * 0.7; // 70% of their best level
        }

        // Adjacent group compatibility
        let adjacent = matches!(
            (player_group, slot_group),
            (PlayerFieldPositionGroup::Defender, PlayerFieldPositionGroup::Midfielder)
                | (PlayerFieldPositionGroup::Midfielder, PlayerFieldPositionGroup::Defender)
                | (PlayerFieldPositionGroup::Midfielder, PlayerFieldPositionGroup::Forward)
                | (PlayerFieldPositionGroup::Forward, PlayerFieldPositionGroup::Midfielder)
        );

        if adjacent {
            let primary_level = player
                .positions
                .positions
                .iter()
                .map(|p| p.level)
                .max()
                .unwrap_or(0);
            return primary_level as f32 * 0.4; // 40% — can play but not ideal
        }

        // Completely wrong group (e.g. defender as forward)
        1.0 // minimal score — only as last resort
    }

    /// Overall quality score for a player (used for bench selection and general ranking)
    fn overall_quality(player: &Player, staff: &Staff, tactics: &Tactics) -> f32 {
        let ability = player.player_attributes.current_ability as f32 / 200.0 * 20.0;
        let condition = (player.player_attributes.condition as f32 / 10000.0).clamp(0.0, 1.0) * 20.0;
        let readiness = (player.skills.physical.match_readiness / 20.0).clamp(0.0, 1.0) * 20.0;
        let primary_level = player
            .positions
            .positions
            .iter()
            .map(|p| p.level)
            .max()
            .unwrap_or(0) as f32;

        let mut score = ability * 0.30 + condition * 0.20 + readiness * 0.15 + primary_level * 0.30;

        if staff.relations.is_favorite_player(player.id) {
            score += 1.0;
        }

        // Small bonus for matching a tactical position
        let best_pos = Self::best_tactical_position(player, tactics);
        if player.positions.get_level(best_pos) > 0 {
            score += 0.5;
        }

        score
    }

    /// Tactical style bonus for a player in a given position
    fn tactical_style_bonus(
        player: &Player,
        position: PlayerPositionType,
        tactics: &Tactics,
    ) -> f32 {
        let mut bonus = 0.0;

        match tactics.tactical_style() {
            crate::TacticalStyle::Attacking => {
                if position.is_forward()
                    || position == PlayerPositionType::AttackingMidfielderCenter
                {
                    bonus += player.skills.technical.finishing * 0.1;
                    bonus += player.skills.mental.off_the_ball * 0.1;
                }
            }
            crate::TacticalStyle::Defensive => {
                if position.is_defender() || position == PlayerPositionType::DefensiveMidfielder {
                    bonus += player.skills.technical.tackling * 0.1;
                    bonus += player.skills.mental.positioning * 0.1;
                }
            }
            crate::TacticalStyle::Possession => {
                bonus += player.skills.technical.passing * 0.08;
                bonus += player.skills.mental.vision * 0.08;
            }
            crate::TacticalStyle::Counterattack => {
                if position.is_forward() || position.is_midfielder() {
                    bonus += player.skills.physical.pace * 0.1;
                    bonus += player.skills.mental.off_the_ball * 0.08;
                }
            }
            crate::TacticalStyle::WingPlay | crate::TacticalStyle::WidePlay => {
                if position == PlayerPositionType::WingbackLeft
                    || position == PlayerPositionType::WingbackRight
                    || position == PlayerPositionType::MidfielderLeft
                    || position == PlayerPositionType::MidfielderRight
                {
                    bonus += player.skills.technical.crossing * 0.1;
                    bonus += player.skills.physical.pace * 0.08;
                }
            }
            _ => {}
        }

        bonus
    }

    // ========== HELPERS ==========

    /// Check if a player is primarily a goalkeeper (using raw position data, not level threshold)
    fn is_goalkeeper_player(player: &Player) -> bool {
        player
            .positions
            .positions
            .iter()
            .any(|p| p.position == PlayerPositionType::Goalkeeper)
    }

    /// Pick the best goalkeeper from available players
    fn pick_best_goalkeeper<'p>(
        available: &[&'p Player],
        used_ids: &[u32],
    ) -> Option<&'p Player> {
        available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| Self::is_goalkeeper_player(p))
            .max_by(|a, b| {
                let score_a = Self::goalkeeper_score(a);
                let score_b = Self::goalkeeper_score(b);
                score_a
                    .partial_cmp(&score_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
    }

    /// Score a goalkeeper — ability + condition + GK level
    fn goalkeeper_score(player: &Player) -> f32 {
        let gk_level = player.positions.get_level(PlayerPositionType::Goalkeeper) as f32;
        let ability = player.player_attributes.current_ability as f32 / 200.0 * 20.0;
        let condition = (player.player_attributes.condition as f32 / 10000.0).clamp(0.0, 1.0) * 20.0;
        let readiness =
            (player.skills.physical.match_readiness / 20.0).clamp(0.0, 1.0) * 20.0;

        gk_level * 0.30 + ability * 0.25 + condition * 0.25 + readiness * 0.20
    }

    /// Pick the best unused player by overall quality
    fn pick_best_unused<'p>(
        available: &[&'p Player],
        used_ids: &[u32],
    ) -> Option<&'p Player> {
        available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .max_by(|a, b| {
                let sa = a.player_attributes.current_ability;
                let sb = b.player_attributes.current_ability;
                sa.cmp(&sb)
            })
            .copied()
    }

    /// Find the best tactical position for a player within the formation
    fn best_tactical_position(player: &Player, tactics: &Tactics) -> PlayerPositionType {
        let player_group = player.position().position_group();

        // First: find exact match in formation
        for &pos in tactics.positions() {
            if player.positions.get_level(pos) > 0 {
                return pos;
            }
        }

        // Second: find same position group in formation
        for &pos in tactics.positions() {
            if pos.position_group() == player_group && pos != PlayerPositionType::Goalkeeper {
                return pos;
            }
        }

        // Third: find any outfield position in formation
        for &pos in tactics.positions() {
            if pos != PlayerPositionType::Goalkeeper {
                return pos;
            }
        }

        // Fallback to player's natural position
        player.position()
    }

    // ========== PUBLIC API (backward compatibility) ==========

    /// Calculate player rating for a specific position (used by substitution engine)
    pub fn calculate_player_rating_for_position(
        player: &Player,
        staff: &Staff,
        position: PlayerPositionType,
        tactics: &Tactics,
    ) -> f32 {
        let group = position.position_group();
        Self::score_player_for_slot(player, position, group, staff, tactics)
    }

    /// Legacy method
    pub fn select_main_squad(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        Self::select_starting_eleven(team_id, players, staff, tactics)
    }

    /// Legacy method
    pub fn select_substitutes_legacy(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        Self::select_substitutes(team_id, players, staff, tactics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IntegerUtils, MatchTacticType, PlayerCollection, PlayerGenerator, StaffCollection,
        TeamBuilder, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{NaiveTime, Utc};

    #[test]
    fn test_squad_selection_always_produces_11() {
        let team = generate_test_team();
        let staff = generate_test_staff();

        let result = SquadSelector::select(&team, &staff);

        assert_eq!(result.main_squad.len(), 11);
        assert!(!result.substitutes.is_empty());
        assert!(result.substitutes.len() <= DEFAULT_BENCH_SIZE);
    }

    #[test]
    fn test_squad_always_has_goalkeeper() {
        let team = generate_test_team();
        let staff = generate_test_staff();

        let result = SquadSelector::select(&team, &staff);

        let has_gk = result.main_squad.iter().any(|p| {
            p.tactical_position.current_position == PlayerPositionType::Goalkeeper
        });
        assert!(has_gk, "Starting 11 must always have a goalkeeper");
    }

    #[test]
    fn test_squad_no_duplicate_players() {
        let team = generate_test_team();
        let staff = generate_test_staff();

        let result = SquadSelector::select(&team, &staff);

        let mut all_ids: Vec<u32> = result.main_squad.iter().map(|p| p.id).collect();
        all_ids.extend(result.substitutes.iter().map(|p| p.id));

        let unique_count = {
            let mut sorted = all_ids.clone();
            sorted.sort();
            sorted.dedup();
            sorted.len()
        };
        assert_eq!(all_ids.len(), unique_count, "No player should appear twice");
    }

    #[test]
    fn test_position_group_matching() {
        // A DefenderCenter player should be able to fill a DefenderCenterLeft slot
        let score = SquadSelector::position_fit_score(
            &generate_defender_center(),
            PlayerPositionType::DefenderCenterLeft,
            PlayerFieldPositionGroup::Defender,
        );
        assert!(score > 5.0, "Same-group player should score well: {}", score);
    }

    // ========== Test helpers ==========

    fn generate_test_team() -> Team {
        let mut team = TeamBuilder::new()
            .id(1)
            .league_id(Some(1))
            .club_id(1)
            .name("Test Team".to_string())
            .slug("test-team".to_string())
            .team_type(TeamType::Main)
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            ))
            .reputation(TeamReputation::new(100, 100, 100))
            .players(PlayerCollection::new(generate_test_players()))
            .staffs(StaffCollection::new(Vec::new()))
            .tactics(Some(Tactics::new(MatchTacticType::T442)))
            .build()
            .expect("Failed to build test team");

        team.tactics = Some(Tactics::new(MatchTacticType::T442));
        team
    }

    fn generate_test_staff() -> Staff {
        crate::StaffStub::default()
    }

    fn generate_test_players() -> Vec<Player> {
        let mut players = Vec::new();

        for &position in &[
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenter,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::Striker,
        ] {
            for _ in 0..3 {
                let level = IntegerUtils::random(15, 20) as u8;
                let player =
                    PlayerGenerator::generate(1, Utc::now().date_naive(), position, level);
                players.push(player);
            }
        }

        players
    }

    fn generate_defender_center() -> Player {
        PlayerGenerator::generate(
            1,
            Utc::now().date_naive(),
            PlayerPositionType::DefenderCenter,
            18,
        )
    }
}
