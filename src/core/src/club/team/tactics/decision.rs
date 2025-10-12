use crate::r#match::{SquadSelector, TacticalSquadAnalyzer};
use crate::{Player, Team};

pub struct TacticalDecisionEngine;

impl TacticalDecisionEngine {
    /// Make comprehensive tactical decisions for a team
    pub fn make_tactical_decisions(team: &mut Team) -> TacticalDecisionResult {
        let head_coach = team.staffs.head_coach();
        let mut decisions = TacticalDecisionResult::new();

        // 1. Formation Analysis
        if let Some(optimal_formation) = TacticalSquadAnalyzer::suggest_optimal_formation(team, head_coach) {
            let current_formation = team.tactics.as_ref().map(|t| t.tactic_type);

            if current_formation != Some(optimal_formation) {
                decisions.formation_change = Some(FormationChange {
                    from: current_formation,
                    to: optimal_formation,
                    reason: "Squad composition analysis".to_string(),
                    confidence: 0.8,
                });
            }
        }

        // 2. Squad Selection Analysis
        let squad_result = SquadSelector::select(team, head_coach);
        decisions.squad_analysis = Self::analyze_squad_selection(&squad_result, team);

        // 3. Tactical Recommendations
        decisions.recommendations = Self::generate_tactical_recommendations(team, head_coach);

        decisions
    }

    /// Analyze the quality of squad selection
    fn analyze_squad_selection(
        squad_result: &crate::r#match::squad::PlayerSelectionResult,
        team: &Team,
    ) -> SquadAnalysis {
        let tactics = team.tactics();
        let mut analysis = SquadAnalysis::new();

        // Calculate average ratings for main squad
        let mut total_rating = 0.0;
        let mut position_mismatches = 0;

        for match_player in &squad_result.main_squad {
            let players = team.players.players();

            let player = players
                .iter()
                .find(|p| p.id == match_player.id)
                .unwrap();

            let position = match_player.tactical_position.current_position;
            let rating = SquadSelector::calculate_player_rating_for_position(
                player,
                team.staffs.head_coach(),
                position,
                &tactics
            );

            total_rating += rating;

            // Check for position mismatches
            if !player.positions.has_position(position) {
                position_mismatches += 1;
                analysis.warnings.push(format!(
                    "{} playing out of position at {}",
                    player.full_name,
                    position.get_short_name()
                ));
            }
        }

        analysis.average_rating = total_rating / squad_result.main_squad.len() as f32;
        analysis.position_mismatches = position_mismatches;
        analysis.formation_fitness = tactics.calculate_formation_fitness(&team.players.players());

        // Analyze bench strength
        analysis.bench_quality = Self::calculate_bench_quality(&squad_result.substitutes, team);

        analysis
    }

    /// Calculate the quality of substitutes
    fn calculate_bench_quality(
        substitutes: &[crate::r#match::MatchPlayer],
        team: &Team,
    ) -> f32 {
        if substitutes.is_empty() {
            return 0.0;
        }

        let mut total_quality = 0.0;
        for sub in substitutes {
            if let Some(player) = team.players.players().iter().find(|p| p.id == sub.id) {
                let quality = (player.skills.technical.average() +
                    player.skills.mental.average() +
                    player.skills.physical.average()) / 3.0;
                total_quality += quality;
            }
        }

        total_quality / substitutes.len() as f32
    }

    /// Generate tactical recommendations for the team
    fn generate_tactical_recommendations(team: &Team, staff: &crate::Staff) -> Vec<TacticalRecommendation> {
        let mut recommendations = Vec::new();

        // Check if coach tactical knowledge matches formation complexity
        let current_tactics = team.tactics();
        let coach_knowledge = staff.staff_attributes.knowledge.tactical_knowledge;

        match current_tactics.tactic_type {
            crate::MatchTacticType::T4312 | crate::MatchTacticType::T343 if coach_knowledge < 15 => {
                recommendations.push(TacticalRecommendation {
                    priority: RecommendationPriority::High,
                    category: RecommendationCategory::Formation,
                    description: "Current formation may be too complex for coach's tactical knowledge. Consider simpler formation like 4-4-2 or 4-3-3.".to_string(),
                    suggested_action: Some("Change to 4-4-2".to_string()),
                });
            }
            _ => {}
        }

        // Check for player-position mismatches
        let available_players: Vec<&Player> = team.players.players()
            .iter()
            .filter(|p| !p.player_attributes.is_injured && !p.player_attributes.is_banned)
            .map(|p| *p)
            .collect();

        let formation_fitness = current_tactics.calculate_formation_fitness(&available_players);
        if formation_fitness < 0.6 {
            recommendations.push(TacticalRecommendation {
                priority: RecommendationPriority::Medium,
                category: RecommendationCategory::SquadSelection,
                description: format!("Formation fitness is low ({:.2}). Consider formation change or player acquisitions.", formation_fitness),
                suggested_action: Some("Analyze alternative formations".to_string()),
            });
        }

        // Check bench depth
        let bench_players = available_players.len() - 11;
        if bench_players < 7 {
            recommendations.push(TacticalRecommendation {
                priority: RecommendationPriority::Medium,
                category: RecommendationCategory::SquadDepth,
                description: format!("Limited bench options ({} substitutes available). Squad depth may be insufficient.", bench_players),
                suggested_action: Some("Consider loan or transfer signings".to_string()),
            });
        }

        recommendations
    }
}

/// Result of tactical decision analysis
#[derive(Debug)]
pub struct TacticalDecisionResult {
    pub formation_change: Option<FormationChange>,
    pub squad_analysis: SquadAnalysis,
    pub recommendations: Vec<TacticalRecommendation>,
}

impl TacticalDecisionResult {
    fn new() -> Self {
        TacticalDecisionResult {
            formation_change: None,
            squad_analysis: SquadAnalysis::new(),
            recommendations: Vec::new(),
        }
    }
}

/// Suggested formation change
#[derive(Debug)]
pub struct FormationChange {
    pub from: Option<crate::MatchTacticType>,
    pub to: crate::MatchTacticType,
    pub reason: String,
    pub confidence: f32,
}

/// Analysis of squad selection quality
#[derive(Debug)]
pub struct SquadAnalysis {
    pub average_rating: f32,
    pub formation_fitness: f32,
    pub position_mismatches: u8,
    pub bench_quality: f32,
    pub warnings: Vec<String>,
}

impl SquadAnalysis {
    fn new() -> Self {
        SquadAnalysis {
            average_rating: 0.0,
            formation_fitness: 0.0,
            position_mismatches: 0,
            bench_quality: 0.0,
            warnings: Vec::new(),
        }
    }
}

/// Tactical recommendation for team improvement
#[derive(Debug)]
pub struct TacticalRecommendation {
    pub priority: RecommendationPriority,
    pub category: RecommendationCategory,
    pub description: String,
    pub suggested_action: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum RecommendationPriority {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug)]
pub enum RecommendationCategory {
    Formation,
    SquadSelection,
    SquadDepth,
    TacticalStyle,
    PlayerDevelopment,
}
