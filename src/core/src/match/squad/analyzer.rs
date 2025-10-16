use crate::club::{PlayerPositionType, Staff};
use crate::r#match::MatchResult;
use crate::{MatchTacticType, Player, TacticSelectionReason, TacticalStyle, Tactics, Team};
use log::info;
use std::collections::HashMap;

/// Enhanced tactical analysis for squad selection
pub struct TacticalSquadAnalyzer;

impl TacticalSquadAnalyzer {
    /// Analyze team composition and suggest best formation
    pub fn suggest_optimal_formation(team: &Team, staff: &Staff) -> Option<MatchTacticType> {
        let available_players: Vec<&Player> = team
            .players
            .players()
            .iter()
            .filter(|&&p| !p.player_attributes.is_injured && !p.player_attributes.is_banned)
            .map(|p| *p)
            .collect();

        if available_players.len() < 11 {
            return None;
        }

        let composition = Self::analyze_squad_composition(&available_players);
        let coach_preference = Self::analyze_coach_preferences(staff);

        Self::determine_best_formation(composition, coach_preference)
    }

    /// Analyze the strengths and weaknesses of available players
    fn analyze_squad_composition(players: &[&Player]) -> SquadComposition {
        let mut composition = SquadComposition::new();

        for &player in players {
            // Analyze by position groups
            for position in player.positions() {
                let quality = Self::calculate_player_quality(player);

                match position {
                    pos if pos.is_goalkeeper() => {
                        composition.goalkeeper_quality.push(quality);
                    }
                    pos if pos.is_defender() => {
                        composition.defender_quality.push(quality);
                        if matches!(pos, PlayerPositionType::DefenderLeft | PlayerPositionType::DefenderRight) {
                            composition.fullback_quality += quality;
                        }
                    }
                    pos if pos.is_midfielder() => {
                        composition.midfielder_quality.push(quality);
                        if matches!(pos, PlayerPositionType::DefensiveMidfielder) {
                            composition.defensive_mid_quality += quality;
                        }
                        if matches!(pos, PlayerPositionType::AttackingMidfielderCenter |
                                        PlayerPositionType::AttackingMidfielderLeft |
                                        PlayerPositionType::AttackingMidfielderRight) {
                            composition.attacking_mid_quality += quality;
                        }
                    }
                    pos if pos.is_forward() => {
                        composition.forward_quality.push(quality);
                    }
                    _ => {}
                }
            }

            // Analyze specific attributes
            composition.pace_merchants += if player.skills.physical.pace > 15.0 { 1 } else { 0 };
            composition.technical_players += if player.skills.technical.technique > 15.0 { 1 } else { 0 };
            composition.physical_players += if player.skills.physical.strength > 15.0 { 1 } else { 0 };
            composition.creative_players += if player.skills.mental.vision > 15.0 { 1 } else { 0 };
        }

        composition.finalize_analysis();
        composition
    }

    /// Analyze coach preferences and tactical knowledge
    fn analyze_coach_preferences(staff: &Staff) -> CoachPreferences {
        CoachPreferences {
            tactical_knowledge: staff.staff_attributes.knowledge.tactical_knowledge,
            attacking_preference: staff.staff_attributes.coaching.attacking,
            defending_preference: staff.staff_attributes.coaching.defending,
            prefers_youth: staff.staff_attributes.coaching.working_with_youngsters > 12,
            conservative_approach: staff.behaviour.is_poor(),
        }
    }

    /// Determine the best formation based on analysis
    fn determine_best_formation(
        composition: SquadComposition,
        coach_prefs: CoachPreferences,
    ) -> Option<MatchTacticType> {
        let mut formation_scores = HashMap::new();

        // Score each formation based on squad composition
        for formation in MatchTacticType::all() {
            let score = Self::score_formation_fit(&formation, &composition, &coach_prefs);
            formation_scores.insert(formation, score);
        }

        // Return the highest scoring formation
        formation_scores
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(formation, _)| formation)
    }

    /// Score how well a formation fits the squad
    fn score_formation_fit(
        formation: &MatchTacticType,
        composition: &SquadComposition,
        coach_prefs: &CoachPreferences,
    ) -> f32 {
        let tactics = Tactics::new(*formation);
        let mut score = 0.0;

        // Base fitness score (how well players fit positions)
        score += tactics.calculate_formation_fitness(&composition.get_top_players()) * 0.4;

        // Coach preference bonus
        score += Self::coach_formation_preference_bonus(formation, coach_prefs) * 0.3;

        // Tactical style bonuses based on squad strengths
        match tactics.tactical_style() {
            TacticalStyle::Attacking => {
                score += composition.average_forward_quality() * 0.15;
                score += composition.attacking_mid_quality * 0.1;
            }
            TacticalStyle::Defensive => {
                score += composition.average_defender_quality() * 0.15;
                score += composition.defensive_mid_quality * 0.1;
            }
            TacticalStyle::Possession => {
                score += (composition.technical_players as f32 / 11.0) * 3.0;
                score += (composition.creative_players as f32 / 11.0) * 2.0;
            }
            TacticalStyle::Counterattack => {
                score += (composition.pace_merchants as f32 / 11.0) * 3.0;
                score += composition.average_forward_quality() * 0.1;
            }
            TacticalStyle::WingPlay | TacticalStyle::WidePlay => {
                score += composition.fullback_quality * 0.2;
                score += (composition.pace_merchants as f32 / 11.0) * 2.0;
            }
            _ => {}
        }

        score
    }

    /// Calculate coach formation preference bonus
    fn coach_formation_preference_bonus(
        formation: &MatchTacticType,
        coach_prefs: &CoachPreferences,
    ) -> f32 {
        let mut bonus = 0.0;

        // Tactical knowledge allows for more complex formations
        let complexity_bonus = match formation {
            MatchTacticType::T442 | MatchTacticType::T433 => {
                // Simple formations - any coach can use
                1.0
            }
            MatchTacticType::T4231 | MatchTacticType::T352 => {
                // Moderate complexity
                if coach_prefs.tactical_knowledge >= 12 { 1.2 } else { 0.8 }
            }
            MatchTacticType::T343 | MatchTacticType::T4312 => {
                // High complexity
                if coach_prefs.tactical_knowledge >= 15 { 1.5 } else { 0.6 }
            }
            _ => {
                // Very complex formations
                if coach_prefs.tactical_knowledge >= 18 { 2.0 } else { 0.4 }
            }
        };

        bonus += complexity_bonus;

        // Conservative coaches prefer proven formations
        if coach_prefs.conservative_approach {
            bonus += match formation {
                MatchTacticType::T442 | MatchTacticType::T451 => 0.5,
                _ => 0.0,
            };
        }

        // Attacking/Defending preference
        let attack_def_ratio = coach_prefs.attacking_preference as f32 / coach_prefs.defending_preference as f32;

        if attack_def_ratio > 1.2 {
            // Attacking coach
            bonus += match formation {
                MatchTacticType::T433 | MatchTacticType::T343 | MatchTacticType::T4231 => 0.3,
                _ => 0.0,
            };
        } else if attack_def_ratio < 0.8 {
            // Defensive coach
            bonus += match formation {
                MatchTacticType::T451 | MatchTacticType::T352 | MatchTacticType::T4141 => 0.3,
                _ => 0.0,
            };
        }

        bonus
    }

    /// Calculate overall player quality
    fn calculate_player_quality(player: &Player) -> f32 {
        let technical_avg = player.skills.technical.average();
        let mental_avg = player.skills.mental.average();
        let physical_avg = player.skills.physical.average();
        let condition_factor = player.player_attributes.condition_percentage() as f32 / 100.0;

        ((technical_avg + mental_avg + physical_avg) / 3.0) * condition_factor
    }
}

/// Analysis of squad composition
#[derive(Debug)]
pub struct SquadComposition {
    pub goalkeeper_quality: Vec<f32>,
    pub defender_quality: Vec<f32>,
    pub midfielder_quality: Vec<f32>,
    pub forward_quality: Vec<f32>,

    pub fullback_quality: f32,
    pub defensive_mid_quality: f32,
    pub attacking_mid_quality: f32,

    pub pace_merchants: u8,
    pub technical_players: u8,
    pub physical_players: u8,
    pub creative_players: u8,

    // Computed averages
    avg_gk_quality: f32,
    avg_def_quality: f32,
    avg_mid_quality: f32,
    avg_fwd_quality: f32,
}

impl SquadComposition {
    fn new() -> Self {
        SquadComposition {
            goalkeeper_quality: Vec::new(),
            defender_quality: Vec::new(),
            midfielder_quality: Vec::new(),
            forward_quality: Vec::new(),
            fullback_quality: 0.0,
            defensive_mid_quality: 0.0,
            attacking_mid_quality: 0.0,
            pace_merchants: 0,
            technical_players: 0,
            physical_players: 0,
            creative_players: 0,
            avg_gk_quality: 0.0,
            avg_def_quality: 0.0,
            avg_mid_quality: 0.0,
            avg_fwd_quality: 0.0,
        }
    }

    fn finalize_analysis(&mut self) {
        self.avg_gk_quality = self.goalkeeper_quality.iter().sum::<f32>() / self.goalkeeper_quality.len().max(1) as f32;
        self.avg_def_quality = self.defender_quality.iter().sum::<f32>() / self.defender_quality.len().max(1) as f32;
        self.avg_mid_quality = self.midfielder_quality.iter().sum::<f32>() / self.midfielder_quality.len().max(1) as f32;
        self.avg_fwd_quality = self.forward_quality.iter().sum::<f32>() / self.forward_quality.len().max(1) as f32;
    }

    pub fn average_goalkeeper_quality(&self) -> f32 { self.avg_gk_quality }
    pub fn average_defender_quality(&self) -> f32 { self.avg_def_quality }
    pub fn average_midfielder_quality(&self) -> f32 { self.avg_mid_quality }
    pub fn average_forward_quality(&self) -> f32 { self.avg_fwd_quality }

    /// Get a representative set of top players for formation fitness calculation
    fn get_top_players(&self) -> Vec<&Player> {
        // This would need to be implemented to return actual player references
        // For now, return empty vec as this is used in a fitness calculation that
        // we're approximating with the averages above
        Vec::new()
    }
}

/// Coach tactical preferences
#[derive(Debug)]
pub struct CoachPreferences {
    pub tactical_knowledge: u8,
    pub attacking_preference: u8,
    pub defending_preference: u8,
    pub prefers_youth: bool,
    pub conservative_approach: bool,
}

/// Enhanced tactics selector that considers team dynamics
pub struct EnhancedTacticsSelector;

impl EnhancedTacticsSelector {
    /// Select tactics considering recent performance and team mood
    pub fn select_contextual_tactics(
        team: &Team,
        staff: &Staff,
        recent_results: &[MatchResult],
        team_morale: f32,
    ) -> Tactics {
        let base_tactics = TacticalSquadAnalyzer::suggest_optimal_formation(team, staff)
            .unwrap_or(MatchTacticType::T442);

        let mut tactics = Tactics::new(base_tactics);

        // Adjust based on recent performance
        if let Some(adjusted) = Self::adjust_for_recent_form(&tactics, recent_results) {
            tactics = adjusted;
        }

        // Adjust based on team morale
        tactics = Self::adjust_for_morale(tactics, team_morale);

        info!("Selected tactics: {} ({})",
              tactics.formation_description(),
              tactics.tactic_type.display_name());

        tactics
    }

    /// Adjust tactics based on recent match results
    fn adjust_for_recent_form(
        current_tactics: &Tactics,
        recent_results: &[MatchResult],
    ) -> Option<Tactics> {
        if recent_results.len() < 3 {
            return None;
        }

        let losses = recent_results.iter()
            .take(5) // Last 5 games
            .filter(|result| {
                result.score.home_team.get() < result.score.away_team.get() ||
                    result.score.away_team.get() < result.score.home_team.get()
            })
            .count();

        // If struggling, become more defensive
        if losses >= 3 {
            let defensive_formation = match current_tactics.tactic_type {
                MatchTacticType::T433 => MatchTacticType::T451,
                MatchTacticType::T4231 => MatchTacticType::T4141,
                _ => return None,
            };

            Some(Tactics::with_reason(
                defensive_formation,
                TacticSelectionReason::GameSituation,
                0.8,
            ))
        } else {
            None
        }
    }

    /// Adjust tactics based on team morale
    fn adjust_for_morale(mut tactics: Tactics, morale: f32) -> Tactics {
        // High morale = more attacking
        if morale > 0.7 {
            tactics.formation_strength *= 1.1;
        }
        // Low morale = more conservative
        else if morale < 0.3 {
            tactics.formation_strength *= 0.9;
        }

        tactics
    }
}