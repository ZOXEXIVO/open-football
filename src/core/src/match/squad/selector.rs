use crate::club::{PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::{Player, Tactics, Team};
use log::{debug, warn};
use std::borrow::Borrow;

pub struct SquadSelector;

const DEFAULT_SQUAD_SIZE: usize = 11;
const DEFAULT_BENCH_SIZE: usize = 7;

pub struct PlayerSelectionResult {
    pub main_squad: Vec<MatchPlayer>,
    pub substitutes: Vec<MatchPlayer>,
}

#[derive(Debug, Clone)]
struct PlayerRating {
    player_id: u32,
    rating: f32,
    position_fitness: f32,
    overall_ability: f32,
}

impl SquadSelector {
    pub fn select(team: &Team, staff: &Staff) -> PlayerSelectionResult {
        let current_tactics = team.tactics();

        // Filter available players (not injured, not banned)
        let available_players: Vec<&Player> = team
            .players
            .players()
            .iter()
            .filter(|&&p| !p.player_attributes.is_injured && !p.player_attributes.is_banned)
            .map(|p| *p)
            .collect();

        debug!("Available players for selection: {}", available_players.len());

        if available_players.len() < DEFAULT_SQUAD_SIZE {
            warn!("Not enough available players for full squad: {}", available_players.len());
        }

        // Select main squad based on tactics
        let main_squad = Self::select_main_squad_optimized(
            team.id,
            &available_players,
            staff,
            current_tactics.borrow(),
        );

        // Filter out selected main squad players for substitutes selection
        let remaining_players: Vec<&Player> = available_players
            .iter()
            .filter(|&player| !main_squad.iter().any(|mp| mp.id == player.id))
            .map(|&p| p)
            .collect();

        // Select substitutes
        let substitutes = Self::select_substitutes_optimized(
            team.id,
            &remaining_players,
            staff,
            current_tactics.borrow(),
        );

        debug!("Selected squad - Main: {}, Subs: {}", main_squad.len(), substitutes.len());

        PlayerSelectionResult {
            main_squad,
            substitutes,
        }
    }

    /// Optimized main squad selection that properly uses tactics
    fn select_main_squad_optimized(
        team_id: u32,
        available_players: &[&Player],
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let mut squad: Vec<MatchPlayer> = Vec::with_capacity(DEFAULT_SQUAD_SIZE);
        let mut used_players: Vec<u32> = Vec::new();

        // Get required positions from tactics
        let required_positions = tactics.positions();

        debug!("Formation: {} requires positions: {:?}",
               tactics.formation_description(),
               required_positions);

        // For each required position, find the best available player
        for (position_index, &required_position) in required_positions.iter().enumerate() {
            if let Some(best_player) = Self::find_best_player_for_position(
                available_players,
                &used_players,
                required_position,
                staff,
                tactics,
            ) {
                squad.push(MatchPlayer::from_player(
                    team_id,
                    best_player,
                    required_position,
                    position_index < DEFAULT_SQUAD_SIZE,
                ));
                used_players.push(best_player.id);

                debug!("Selected {} for position {} ({})",
                       best_player.full_name,
                       required_position.get_short_name(),
                       Self::calculate_player_rating_for_position(best_player, staff, required_position, tactics));
            } else {
                warn!("No suitable player found for position: {}", required_position.get_short_name());
            }
        }

        // Fill remaining spots if we don't have 11 players yet
        while squad.len() < DEFAULT_SQUAD_SIZE && squad.len() < available_players.len() {
            if let Some(best_remaining) = Self::find_best_remaining_player(
                available_players,
                &used_players,
                staff,
                tactics,
            ) {
                // Assign them to their best position within the formation
                let best_position = Self::find_best_position_for_player(best_remaining, tactics);

                squad.push(MatchPlayer::from_player(
                    team_id,
                    best_remaining,
                    best_position,
                    true,
                ));
                used_players.push(best_remaining.id);
            } else {
                break;
            }
        }

        squad
    }

    /// Optimized substitute selection focusing on tactical flexibility
    fn select_substitutes_optimized(
        team_id: u32,
        remaining_players: &[&Player],
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let mut substitutes: Vec<MatchPlayer> = Vec::with_capacity(DEFAULT_BENCH_SIZE);
        let mut used_players: Vec<u32> = Vec::new();

        // Prioritize substitute selection:
        // 1. Backup goalkeeper (if not already selected)
        if let Some(backup_gk) = remaining_players
            .iter()
            .filter(|p| p.positions.is_goalkeeper() && !used_players.contains(&p.id))
            .max_by(|a, b| {
                Self::calculate_player_rating_for_position(a, staff, PlayerPositionType::Goalkeeper, tactics)
                    .partial_cmp(&Self::calculate_player_rating_for_position(b, staff, PlayerPositionType::Goalkeeper, tactics))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
            substitutes.push(MatchPlayer::from_player(
                team_id,
                backup_gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_players.push(backup_gk.id);
        }

        // 2. Versatile players who can cover multiple positions
        let mut versatile_players: Vec<(&Player, f32)> = remaining_players
            .iter()
            .filter(|p| !used_players.contains(&p.id))
            .map(|&player| {
                let versatility_score = Self::calculate_versatility_score(player, tactics);
                (player, versatility_score)
            })
            .collect();

        versatile_players.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // 3. Add most versatile players to bench
        for (player, _score) in versatile_players.iter().take(DEFAULT_BENCH_SIZE - substitutes.len()) {
            if !used_players.contains(&player.id) {
                let best_position = Self::find_best_position_for_player(player, tactics);
                substitutes.push(MatchPlayer::from_player(
                    team_id,
                    player,
                    best_position,
                    false,
                ));
                used_players.push(player.id);
            }
        }

        // 4. Fill remaining spots with best available players
        while substitutes.len() < DEFAULT_BENCH_SIZE && substitutes.len() + used_players.len() < remaining_players.len() {
            if let Some(best_remaining) = Self::find_best_remaining_player(
                remaining_players,
                &used_players,
                staff,
                tactics,
            ) {
                let best_position = Self::find_best_position_for_player(best_remaining, tactics);
                substitutes.push(MatchPlayer::from_player(
                    team_id,
                    best_remaining,
                    best_position,
                    false,
                ));
                used_players.push(best_remaining.id);
            } else {
                break;
            }
        }

        substitutes
    }

    /// Find the best player for a specific position
    fn find_best_player_for_position<'p>(
        available_players: &'p [&Player],
        used_players: &[u32],
        position: PlayerPositionType,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Option<&'p Player> {
        available_players
            .iter()
            .filter(|p| !used_players.contains(&p.id))
            .filter(|p| p.positions.has_position(position) || position == PlayerPositionType::Goalkeeper && p.positions.is_goalkeeper())
            .max_by(|a, b| {
                Self::calculate_player_rating_for_position(a, staff, position, tactics)
                    .partial_cmp(&Self::calculate_player_rating_for_position(b, staff, position, tactics))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
    }

    /// Find the best remaining player regardless of position
    fn find_best_remaining_player<'p>(
        available_players: &'p [&Player],
        used_players: &[u32],
        staff: &Staff,
        tactics: &Tactics,
    ) -> Option<&'p Player> {
        available_players
            .iter()
            .filter(|p| !used_players.contains(&p.id))
            .max_by(|a, b| {
                let rating_a = Self::calculate_overall_player_value(a, staff, tactics);
                let rating_b = Self::calculate_overall_player_value(b, staff, tactics);
                rating_a.partial_cmp(&rating_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
    }

    /// Find the best position for a player within the current tactics
    fn find_best_position_for_player(player: &Player, tactics: &Tactics) -> PlayerPositionType {
        let mut best_position = player.position(); // Default to primary position
        let mut best_rating = 0.0;

        for &position in tactics.positions() {
            if player.positions.has_position(position) {
                let position_level = player.positions.get_level(position) as f32;
                if position_level > best_rating {
                    best_rating = position_level;
                    best_position = position;
                }
            }
        }

        best_position
    }

    /// Calculate player rating for a specific position considering tactics
    pub fn calculate_player_rating_for_position(
        player: &Player,
        staff: &Staff,
        position: PlayerPositionType,
        tactics: &Tactics,
    ) -> f32 {
        let mut rating = 0.0;

        // Position proficiency (most important factor)
        let position_level = player.positions.get_level(position) as f32;
        rating += position_level * 0.4; // 40% weight

        // Physical condition
        rating += (player.player_attributes.condition as f32 / 10000.0) * 20.0 * 0.25; // 25% weight

        // Overall ability
        rating += player.player_attributes.current_ability as f32 / 200.0 * 20.0 * 0.2; // 20% weight

        // Tactical fit for formation
        rating += Self::calculate_tactical_fit(player, position, tactics) * 0.1; // 10% weight

        // Staff relationship bonus
        if staff.relations.is_favorite_player(player.id) {
            rating += 2.0;
        }

        // Reputation bonus (small factor)
        rating += (player.player_attributes.world_reputation as f32 / 10000.0) * 0.05; // 5% weight

        rating
    }

    /// Calculate how well a player fits tactically in a position
    fn calculate_tactical_fit(player: &Player, position: PlayerPositionType, tactics: &Tactics) -> f32 {
        let mut fit_score = 10.0; // Base score

        // Tactical style bonuses
        match tactics.tactical_style() {
            crate::TacticalStyle::Attacking => {
                if position.is_forward() || position == PlayerPositionType::AttackingMidfielderCenter {
                    fit_score += player.skills.technical.finishing * 0.1;
                    fit_score += player.skills.mental.off_the_ball * 0.1;
                }
            }
            crate::TacticalStyle::Defensive => {
                if position.is_defender() || position == PlayerPositionType::DefensiveMidfielder {
                    fit_score += player.skills.technical.tackling * 0.1;
                    fit_score += player.skills.mental.positioning * 0.1;
                }
            }
            crate::TacticalStyle::Possession => {
                fit_score += player.skills.technical.passing * 0.08;
                fit_score += player.skills.mental.vision * 0.08;
            }
            crate::TacticalStyle::Counterattack => {
                if position.is_forward() || position.is_midfielder() {
                    fit_score += player.skills.physical.pace * 0.1;
                    fit_score += player.skills.mental.off_the_ball * 0.08;
                }
            }
            crate::TacticalStyle::WingPlay | crate::TacticalStyle::WidePlay => {
                if position == PlayerPositionType::WingbackLeft
                    || position == PlayerPositionType::WingbackRight
                    || position == PlayerPositionType::MidfielderLeft
                    || position == PlayerPositionType::MidfielderRight {
                    fit_score += player.skills.technical.crossing * 0.1;
                    fit_score += player.skills.physical.pace * 0.08;
                }
            }
            _ => {} // No specific bonuses for other styles
        }

        fit_score
    }

    /// Calculate overall player value considering multiple factors
    fn calculate_overall_player_value(player: &Player, staff: &Staff, tactics: &Tactics) -> f32 {
        let mut value = 0.0;

        // Find best position for this player in current tactics
        let best_position = Self::find_best_position_for_player(player, tactics);
        value += Self::calculate_player_rating_for_position(player, staff, best_position, tactics);

        // Add versatility bonus
        value += Self::calculate_versatility_score(player, tactics) * 0.1;

        value
    }

    /// Calculate how versatile a player is (can play multiple positions)
    fn calculate_versatility_score(player: &Player, tactics: &Tactics) -> f32 {
        let tactics_positions = tactics.positions();
        let player_positions = player.positions();

        let covered_positions = tactics_positions
            .iter()
            .filter(|&&pos| player_positions.contains(&pos))
            .count();

        // Score based on how many tactical positions they can cover
        match covered_positions {
            0 => 0.0,
            1 => 1.0,
            2 => 3.0,
            3 => 6.0,
            4 => 10.0,
            _ => 15.0,
        }
    }

    /// Legacy method for backward compatibility
    pub fn select_main_squad(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        Self::select_main_squad_optimized(team_id, players, staff, tactics)
    }

    /// Legacy method for backward compatibility
    pub fn select_substitutes(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        Self::select_substitutes_optimized(team_id, players, staff, tactics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IntegerUtils, MatchTacticType, PlayerCollection, PlayerGenerator,
        StaffCollection, TeamReputation, TeamType, TrainingSchedule
        ,
    };
    use chrono::{NaiveTime, Utc};

    #[test]
    fn test_squad_selection_respects_formation() {
        let team = generate_test_team();
        let staff = generate_test_staff();

        let result = SquadSelector::select(&team, &staff);

        // Should select exactly 11 main squad players
        assert_eq!(result.main_squad.len(), 11);

        // Should have substitutes
        assert!(result.substitutes.len() > 0);
        assert!(result.substitutes.len() <= DEFAULT_BENCH_SIZE);

        // All positions in formation should be covered
        let tactics = team.tactics();
        let formation_positions = tactics.positions();
        assert_eq!(result.main_squad.len(), formation_positions.len());
    }

    #[test]
    fn test_tactical_fit_calculation() {
        let player = generate_attacking_player();
        let tactics = crate::Tactics::new(MatchTacticType::T433); // More attacking formation

        let fit = SquadSelector::calculate_tactical_fit(&player, PlayerPositionType::Striker, &tactics);

        assert!(fit > 10.0); // Should be above base score due to attacking bonuses
    }

    // Helper functions for tests
    fn generate_test_team() -> Team {
        let mut team = Team::new(
            1,
            1,
            1,
            "Test Team".to_string(),
            "test-team".to_string(),
            TeamType::Main,
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            ),
            TeamReputation::new(100, 100, 100),
            PlayerCollection::new(generate_test_players()),
            StaffCollection::new(Vec::new()),
        );

        team.tactics = Some(crate::Tactics::new(MatchTacticType::T442));
        team
    }

    fn generate_test_staff() -> crate::Staff {
        crate::StaffStub::default()
    }

    fn generate_test_players() -> Vec<crate::Player> {
        let mut players = Vec::new();

        // Generate players for each position
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
            for _ in 0..3 { // 3 players per position
                let level = IntegerUtils::random(15, 20) as u8;
                let player = PlayerGenerator::generate(1, Utc::now().date_naive(), position, level);
                players.push(player);
            }
        }

        players
    }

    fn generate_versatile_player() -> crate::Player {
        // Create a player that can play multiple positions
        PlayerGenerator::generate(1, Utc::now().date_naive(), PlayerPositionType::MidfielderCenter, 18)
    }

    fn generate_attacking_player() -> crate::Player {
        PlayerGenerator::generate(1, Utc::now().date_naive(), PlayerPositionType::Striker, 18)
    }
}