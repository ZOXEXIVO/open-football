use crate::club::{PersonBehaviourState, Player, PlayerPositionType, Staff};
use crate::Team;

#[derive(Debug, Clone)]
pub struct Tactics {
    pub tactic_type: MatchTacticType,
    pub selected_reason: TacticSelectionReason,
    pub formation_strength: f32, // 0.0 to 1.0 indicating how well this formation suits the team
}

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub enum TacticSelectionReason {
    CoachPreference,
    TeamComposition,
    OpponentCounter,
    GameSituation,
    Default,
}

impl Tactics {
    pub fn new(tactic_type: MatchTacticType) -> Self {
        Tactics {
            tactic_type,
            selected_reason: TacticSelectionReason::Default,
            formation_strength: 0.5,
        }
    }

    pub fn with_reason(
        tactic_type: MatchTacticType,
        reason: TacticSelectionReason,
        strength: f32,
    ) -> Self {
        Tactics {
            tactic_type,
            selected_reason: reason,
            formation_strength: strength.clamp(0.0, 1.0),
        }
    }

    pub fn positions(&self) -> &[PlayerPositionType; 11] {
        let (_, positions) = TACTICS_POSITIONS
            .iter()
            .find(|(positioning, _)| *positioning == self.tactic_type)
            .unwrap_or(&TACTICS_POSITIONS[0]);

        positions
    }

    pub fn defender_count(&self) -> usize {
        self.positions()
            .iter()
            .filter(|pos| pos.is_defender())
            .count()
    }

    pub fn midfielder_count(&self) -> usize {
        self.positions()
            .iter()
            .filter(|pos| pos.is_midfielder())
            .count()
    }

    pub fn forward_count(&self) -> usize {
        self.positions()
            .iter()
            .filter(|pos| pos.is_forward())
            .count()
    }

    pub fn formation_description(&self) -> String {
        format!(
            "{}-{}-{}",
            self.defender_count(),
            self.midfielder_count(),
            self.forward_count()
        )
    }

    pub fn is_attacking(&self) -> bool {
        self.forward_count() >= 3 || (self.forward_count() == 2 && self.midfielder_count() <= 3)
    }

    pub fn is_defensive(&self) -> bool {
        self.defender_count() >= 5 || (self.defender_count() == 4 && self.midfielder_count() >= 5)
    }

    pub fn is_high_pressing(&self) -> bool {
        true
    }

    pub fn tactical_style(&self) -> TacticalStyle {
        match self.tactic_type {
            MatchTacticType::T442
            | MatchTacticType::T442Diamond
            | MatchTacticType::T442DiamondWide => TacticalStyle::Balanced,
            MatchTacticType::T433 | MatchTacticType::T343 => TacticalStyle::Attacking,
            MatchTacticType::T451 | MatchTacticType::T4141 => TacticalStyle::Defensive,
            MatchTacticType::T352 => TacticalStyle::WingPlay,
            MatchTacticType::T4231 | MatchTacticType::T4312 => TacticalStyle::Possession,
            MatchTacticType::T442Narrow => TacticalStyle::Compact,
            MatchTacticType::T4411 => TacticalStyle::Counterattack,
            MatchTacticType::T1333 => TacticalStyle::Experimental,
            MatchTacticType::T4222 => TacticalStyle::WidePlay,
        }
    }

    /// Calculate how well this tactic suits the available players
    pub fn calculate_formation_fitness(&self, players: &[&Player]) -> f32 {
        let required_positions = self.positions();
        let mut fitness_score = 0.0;
        let mut total_positions = 0.0;

        for required_pos in required_positions.iter() {
            let best_player_fitness = players
                .iter()
                .filter(|p| p.positions().contains(required_pos))
                .map(|p| self.calculate_player_position_fitness(p, required_pos))
                .fold(0.0f32, |acc, x| acc.max(x));

            fitness_score += best_player_fitness;
            total_positions += 1.0;
        }

        if total_positions > 0.0 {
            fitness_score / total_positions
        } else {
            0.0
        }
    }

    fn calculate_player_position_fitness(
        &self,
        player: &Player,
        position: &PlayerPositionType,
    ) -> f32 {
        let position_level = player.positions.get_level(*position) as f32 / 20.0; // Normalize to 0-1
        let overall_ability = player.player_attributes.current_ability as f32 / 200.0; // Normalize to 0-1
        let match_readiness = player.skills.physical.match_readiness / 20.0; // Normalize to 0-1

        // Weight the factors
        (position_level * 0.5) + (overall_ability * 0.3) + (match_readiness * 0.2)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum TacticalStyle {
    Attacking,
    Defensive,
    Balanced,
    Possession,
    Counterattack,
    WingPlay,
    WidePlay,
    Compact,
    Experimental,
}

// Include the TACTICS_POSITIONS array from the previous implementation
pub const TACTICS_POSITIONS: &[(MatchTacticType, [PlayerPositionType; 11])] = &[
    (
        MatchTacticType::T442,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
        ],
    ),
    (
        MatchTacticType::T433,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardCenter,
            PlayerPositionType::ForwardRight,
        ],
    ),
    (
        MatchTacticType::T451,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::Striker,
        ],
    ),
    (
        MatchTacticType::T4231,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::DefensiveMidfielder,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::AttackingMidfielderLeft,
            PlayerPositionType::AttackingMidfielderCenter,
            PlayerPositionType::AttackingMidfielderRight,
            PlayerPositionType::Striker,
        ],
    ),
    (
        MatchTacticType::T352,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenter,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::WingbackLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::WingbackRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
        ],
    ),
    // Add more formations as needed...
];

#[derive(Copy, Debug, Eq, PartialEq, PartialOrd, Clone, Hash)]
pub enum MatchTacticType {
    T442,
    T433,
    T451,
    T4231,
    T352,
    T442Diamond,
    T442DiamondWide,
    T442Narrow,
    T4141,
    T4411,
    T343,
    T1333,
    T4312,
    T4222,
}

impl MatchTacticType {
    pub fn all() -> Vec<MatchTacticType> {
        vec![
            MatchTacticType::T442,
            MatchTacticType::T433,
            MatchTacticType::T451,
            MatchTacticType::T4231,
            MatchTacticType::T352,
            MatchTacticType::T442Diamond,
            MatchTacticType::T442DiamondWide,
            MatchTacticType::T442Narrow,
            MatchTacticType::T4141,
            MatchTacticType::T4411,
            MatchTacticType::T343,
            MatchTacticType::T1333,
            MatchTacticType::T4312,
            MatchTacticType::T4222,
        ]
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            MatchTacticType::T442 => "4-4-2",
            MatchTacticType::T433 => "4-3-3",
            MatchTacticType::T451 => "4-5-1",
            MatchTacticType::T4231 => "4-2-3-1",
            MatchTacticType::T352 => "3-5-2",
            MatchTacticType::T442Diamond => "4-4-2 Diamond",
            MatchTacticType::T442DiamondWide => "4-4-2 Diamond Wide",
            MatchTacticType::T442Narrow => "4-4-2 Narrow",
            MatchTacticType::T4141 => "4-1-4-1",
            MatchTacticType::T4411 => "4-4-1-1",
            MatchTacticType::T343 => "3-4-3",
            MatchTacticType::T1333 => "1-3-3-3",
            MatchTacticType::T4312 => "4-3-1-2",
            MatchTacticType::T4222 => "4-2-2-2",
        }
    }
}

pub struct TacticsSelector;

impl TacticsSelector {
    /// Main method to select the best tactic for a team
    pub fn select(team: &Team, coach: &Staff) -> Tactics {
        let available_players: Vec<&Player> = team
            .players
            .players()
            .into_iter()
            .filter(|p| p.is_ready_for_match())
            .collect();

        if available_players.len() < 11 {
            // Emergency: not enough players, use simple formation
            return Tactics::with_reason(
                MatchTacticType::T442,
                TacticSelectionReason::Default,
                0.3,
            );
        }

        // Evaluate multiple selection strategies
        let strategies = vec![
            Self::select_by_coach_preference(coach, &available_players),
            Self::select_by_team_composition(&available_players),
            Self::select_by_player_quality(&available_players),
        ];

        // Choose the best strategy result
        strategies
            .into_iter()
            .max_by(|a, b| {
                a.formation_strength
                    .partial_cmp(&b.formation_strength)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_else(|| {
                Tactics::with_reason(MatchTacticType::T442, TacticSelectionReason::Default, 0.5)
            })
    }

    /// Select tactic based on coach attributes and behavior
    fn select_by_coach_preference(coach: &Staff, players: &[&Player]) -> Tactics {
        let tactical_knowledge = coach.staff_attributes.knowledge.tactical_knowledge;
        let attacking_coaching = coach.staff_attributes.coaching.attacking;
        let defending_coaching = coach.staff_attributes.coaching.defending;

        let preferred_tactic = match coach.behaviour.state {
            PersonBehaviourState::Poor => {
                // Conservative, simple formation
                if tactical_knowledge < 10 {
                    MatchTacticType::T442
                } else {
                    MatchTacticType::T451
                }
            }
            PersonBehaviourState::Normal => Self::select_balanced_by_coaching_style(
                attacking_coaching,
                defending_coaching,
                tactical_knowledge,
            ),
            PersonBehaviourState::Good => Self::select_advanced_by_coaching_expertise(
                attacking_coaching,
                defending_coaching,
                tactical_knowledge,
            ),
        };

        let tactic = Tactics::new(preferred_tactic);
        let strength =
            tactic.calculate_formation_fitness(players) * Self::coach_confidence_multiplier(coach);

        Tactics::with_reason(
            preferred_tactic,
            TacticSelectionReason::CoachPreference,
            strength,
        )
    }

    fn select_balanced_by_coaching_style(
        attacking: u8,
        defending: u8,
        tactical: u8,
    ) -> MatchTacticType {
        let attack_def_diff = attacking as i16 - defending as i16;

        match (attack_def_diff, tactical) {
            (diff, tact) if diff > 5 && tact >= 12 => MatchTacticType::T433,
            (diff, tact) if diff < -5 && tact >= 12 => MatchTacticType::T451,
            (_, tact) if tact >= 15 => MatchTacticType::T4231,
            (_, tact) if tact >= 10 => MatchTacticType::T442,
            _ => MatchTacticType::T442,
        }
    }

    fn select_advanced_by_coaching_expertise(
        attacking: u8,
        defending: u8,
        tactical: u8,
    ) -> MatchTacticType {
        if tactical >= 18 {
            // Master tactician - can use complex formations
            if attacking >= 16 {
                MatchTacticType::T343
            } else if defending >= 16 {
                MatchTacticType::T352
            } else {
                MatchTacticType::T4312
            }
        } else if tactical >= 15 {
            // Experienced - advanced but proven formations
            if attacking > defending + 3 {
                MatchTacticType::T433
            } else if defending > attacking + 3 {
                MatchTacticType::T4141
            } else {
                MatchTacticType::T4231
            }
        } else {
            // Good but not exceptional
            MatchTacticType::T442Diamond
        }
    }

    /// Select tactic based on available player composition
    fn select_by_team_composition(players: &[&Player]) -> Tactics {
        let position_analysis = Self::analyze_team_composition(players);

        let selected_tactic = Self::match_formation_to_composition(&position_analysis);
        let tactic = Tactics::new(selected_tactic);
        let strength = tactic.calculate_formation_fitness(players);

        Tactics::with_reason(
            selected_tactic,
            TacticSelectionReason::TeamComposition,
            strength,
        )
    }

    fn analyze_team_composition(players: &[&Player]) -> TeamCompositionAnalysis {
        let mut analysis = TeamCompositionAnalysis::new();

        for player in players {
            for position in player.positions() {
                let quality = player.player_attributes.current_ability as f32 / 200.0;

                match position {
                    pos if pos.is_defender() => {
                        analysis.defender_quality += quality;
                        analysis.defender_count += 1;
                    }
                    pos if pos.is_midfielder() => {
                        analysis.midfielder_quality += quality;
                        analysis.midfielder_count += 1;
                    }
                    pos if pos.is_forward() => {
                        analysis.forward_quality += quality;
                        analysis.forward_count += 1;
                    }
                    PlayerPositionType::Goalkeeper => {
                        analysis.goalkeeper_quality += quality;
                        analysis.goalkeeper_count += 1;
                    }
                    _ => {}
                }
            }
        }

        // Calculate averages
        if analysis.defender_count > 0 {
            analysis.defender_quality /= analysis.defender_count as f32;
        }
        if analysis.midfielder_count > 0 {
            analysis.midfielder_quality /= analysis.midfielder_count as f32;
        }
        if analysis.forward_count > 0 {
            analysis.forward_quality /= analysis.forward_count as f32;
        }

        analysis
    }

    fn match_formation_to_composition(analysis: &TeamCompositionAnalysis) -> MatchTacticType {
        // Determine strongest area
        let def_strength =
            analysis.defender_quality * (analysis.defender_count as f32 / 6.0).min(1.0);
        let mid_strength =
            analysis.midfielder_quality * (analysis.midfielder_count as f32 / 6.0).min(1.0);
        let att_strength =
            analysis.forward_quality * (analysis.forward_count as f32 / 4.0).min(1.0);

        if att_strength > def_strength + 0.15 && att_strength > mid_strength + 0.1 {
            // Strong attack
            if analysis.forward_count >= 4 {
                MatchTacticType::T433
            } else {
                MatchTacticType::T4231
            }
        } else if def_strength > att_strength + 0.15 && def_strength > mid_strength + 0.1 {
            // Strong defense
            if analysis.defender_count >= 6 {
                MatchTacticType::T352
            } else {
                MatchTacticType::T451
            }
        } else if mid_strength > 0.7 {
            // Strong midfield
            MatchTacticType::T4312
        } else {
            // Balanced
            MatchTacticType::T442
        }
    }

    /// Select tactic based on individual player quality and fitness
    fn select_by_player_quality(players: &[&Player]) -> Tactics {
        // Test multiple formations and pick the one with best player fit
        let candidate_tactics = vec![
            MatchTacticType::T442,
            MatchTacticType::T433,
            MatchTacticType::T451,
            MatchTacticType::T4231,
            MatchTacticType::T352,
        ];

        let mut best_tactic = MatchTacticType::T442;
        let mut best_strength = 0.0;

        for tactic_type in candidate_tactics {
            let tactic = Tactics::new(tactic_type);
            let strength = tactic.calculate_formation_fitness(players);

            if strength > best_strength {
                best_strength = strength;
                best_tactic = tactic_type;
            }
        }

        Tactics::with_reason(
            best_tactic,
            TacticSelectionReason::TeamComposition,
            best_strength,
        )
    }

    /// Select counter tactic against specific opponent formation
    pub fn select_counter_tactic(
        opponent_tactic: &MatchTacticType,
        our_players: &[&Player],
    ) -> Tactics {
        let counter_tactic = match opponent_tactic {
            // Counter attacking formations with defensive setups
            MatchTacticType::T433 | MatchTacticType::T343 => MatchTacticType::T451,
            // Counter defensive formations with attacking ones
            MatchTacticType::T451 | MatchTacticType::T4141 => MatchTacticType::T433,
            // Counter possession-based with pressing
            MatchTacticType::T4231 | MatchTacticType::T4312 => MatchTacticType::T442Diamond,
            // Counter narrow with wide
            MatchTacticType::T442Narrow => MatchTacticType::T442DiamondWide,
            // Counter wide with compact
            MatchTacticType::T442DiamondWide => MatchTacticType::T442Narrow,
            // Default counter
            _ => MatchTacticType::T442,
        };

        let tactic = Tactics::new(counter_tactic);
        let strength = tactic.calculate_formation_fitness(our_players) * 0.9; // Slight penalty for reactive approach

        Tactics::with_reason(
            counter_tactic,
            TacticSelectionReason::OpponentCounter,
            strength,
        )
    }

    /// Select tactics based on game situation
    pub fn select_situational_tactic(
        current_tactic: &MatchTacticType,
        is_home: bool,
        score_difference: i8,
        minutes_played: u8,
        players: &[&Player],
    ) -> Option<Tactics> {
        let new_tactic = match (score_difference, minutes_played) {
            // Desperately need goals
            (diff, min) if diff < -1 && min > 75 => Some(MatchTacticType::T343),
            (diff, min) if diff < 0 && min > 70 => Some(MatchTacticType::T433),

            // Protecting a lead
            (diff, min) if diff > 1 && min > 80 => Some(MatchTacticType::T451),
            (diff, min) if diff > 0 && min > 75 => Some(MatchTacticType::T4141),

            // First half adjustments
            (diff, min) if diff < -1 && min < 30 && is_home => Some(MatchTacticType::T4231),

            _ => None,
        };

        if let Some(tactic_type) = new_tactic {
            if tactic_type != *current_tactic {
                let tactic = Tactics::new(tactic_type);
                let strength = tactic.calculate_formation_fitness(players) * 0.8; // Penalty for mid-game change
                return Some(Tactics::with_reason(
                    tactic_type,
                    TacticSelectionReason::GameSituation,
                    strength,
                ));
            }
        }

        None
    }

    fn coach_confidence_multiplier(coach: &Staff) -> f32 {
        let base_confidence = match coach.behaviour.state {
            PersonBehaviourState::Poor => 0.7,
            PersonBehaviourState::Normal => 1.0,
            PersonBehaviourState::Good => 1.2,
        };

        let tactical_bonus =
            (coach.staff_attributes.knowledge.tactical_knowledge as f32 / 20.0) * 0.3;
        let experience_bonus = (coach.staff_attributes.mental.determination as f32 / 20.0) * 0.2;

        (base_confidence + tactical_bonus + experience_bonus).clamp(0.5, 1.5)
    }
}

#[derive(Debug)]
struct TeamCompositionAnalysis {
    goalkeeper_count: u8,
    goalkeeper_quality: f32,
    defender_count: u8,
    defender_quality: f32,
    midfielder_count: u8,
    midfielder_quality: f32,
    forward_count: u8,
    forward_quality: f32,
}

impl TeamCompositionAnalysis {
    fn new() -> Self {
        Self {
            goalkeeper_count: 0,
            goalkeeper_quality: 0.0,
            defender_count: 0,
            defender_quality: 0.0,
            midfielder_count: 0,
            midfielder_quality: 0.0,
            forward_count: 0,
            forward_quality: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::fullname::FullName;
    use crate::PersonAttributes;
    fn create_test_player(id: u32, position: PlayerPositionType, ability: u8) -> Player {
        use crate::club::player::builder::PlayerBuilder;
        use crate::club::player::*;

        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".to_string(), "Player".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(1995, 1, 1).unwrap())
            .country_id(1)
            .skills(PlayerSkills::default())
            .attributes(PersonAttributes::default())
            .player_attributes(PlayerAttributes {
                current_ability: ability,
                potential_ability: ability + 10,
                condition: 10000,
                ..Default::default()
            })
            .contract(None)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 18,
                }],
            })
            .build()
            .expect("Failed to build test player")
    }

    #[test]
    fn test_formation_fitness_calculation() {
        let players = vec![
            create_test_player(1, PlayerPositionType::Goalkeeper, 150),
            create_test_player(2, PlayerPositionType::DefenderLeft, 140),
            create_test_player(3, PlayerPositionType::MidfielderCenter, 160),
            create_test_player(4, PlayerPositionType::ForwardCenter, 170),
        ];

        let player_refs: Vec<&Player> = players.iter().collect();
        let tactic = Tactics::new(MatchTacticType::T442);

        let fitness = tactic.calculate_formation_fitness(&player_refs);
        assert!(fitness > 0.0 && fitness <= 1.0);
    }

    #[test]
    fn test_tactical_selection_by_composition() {
        // Create a team with strong attackers
        let players = vec![
            create_test_player(1, PlayerPositionType::ForwardCenter, 180),
            create_test_player(2, PlayerPositionType::ForwardLeft, 175),
            create_test_player(3, PlayerPositionType::ForwardRight, 170),
            create_test_player(4, PlayerPositionType::MidfielderCenter, 140),
        ];

        let player_refs: Vec<&Player> = players.iter().collect();
        let result = TacticsSelector::select_by_team_composition(&player_refs);

        // Should prefer attacking formation for strong forwards
        assert!(result.is_attacking() || matches!(result.tactic_type, MatchTacticType::T4231));
    }

    #[test]
    fn test_counter_tactic_selection() {
        let players = vec![create_test_player(1, PlayerPositionType::Goalkeeper, 150)];
        let player_refs: Vec<&Player> = players.iter().collect();

        let counter = TacticsSelector::select_counter_tactic(&MatchTacticType::T433, &player_refs);
        assert_eq!(counter.tactic_type, MatchTacticType::T451);
        assert_eq!(
            counter.selected_reason,
            TacticSelectionReason::OpponentCounter
        );
    }
}
