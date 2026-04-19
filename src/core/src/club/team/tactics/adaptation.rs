use crate::r#match::EnhancedTacticsSelector;
use crate::{Player, RecommendationPriority, TacticSelectionReason, Tactics, TacticsSelector, Team};
use log::{debug, info};

impl Team {
    /// Adaptive tactics during a match based on game state
    pub fn adapt_tactics_during_match_enhanced(
        &mut self,
        score_difference: i8,
        minutes_played: u8,
        is_home: bool,
        team_morale: f32,
    ) -> Option<Tactics> {
        let current_tactic = &self.tactics().tactic_type;
        let available_players: Vec<&Player> = self
            .players
            .players()
            .into_iter()
            .filter(|p| p.is_ready_for_match())
            .collect();

        // Use enhanced contextual selection
        let staff = self.staffs.head_coach();
        let recent_results = vec![]; // This would come from match history

        let suggested_tactics = EnhancedTacticsSelector::select_contextual_tactics(
            self,
            staff,
            &recent_results,
            team_morale,
        );

        // Override with situational tactics if needed
        if let Some(situational_tactics) = TacticsSelector::select_situational_tactic(
            current_tactic,
            is_home,
            score_difference,
            minutes_played,
            &available_players,
        ) {
            info!(
                "Adapting tactics due to match situation: {} -> {}",
                current_tactic.display_name(),
                situational_tactics.tactic_type.display_name()
            );
            return Some(situational_tactics);
        }

        // Check if suggested tactics are significantly different
        if suggested_tactics.tactic_type != *current_tactic {
            let fitness_current = self
                .tactics()
                .calculate_formation_fitness(&available_players);
            let fitness_suggested =
                suggested_tactics.calculate_formation_fitness(&available_players);

            if fitness_suggested > fitness_current + 0.1 {
                // Significant improvement threshold
                info!(
                    "Switching tactics for better formation fitness: {:.2} -> {:.2}",
                    fitness_current, fitness_suggested
                );
                return Some(suggested_tactics);
            }
        }

        None
    }

    /// Run comprehensive tactical analysis during team simulation
    pub fn run_tactical_analysis(
        &mut self,
    ) -> crate::club::team::tactics::decision::TacticalDecisionResult {
        let decisions =
            crate::club::team::tactics::decision::TacticalDecisionEngine::make_tactical_decisions(
                self,
            );

        // Apply formation change if recommended with high confidence
        if let Some(ref change) = decisions.formation_change {
            if change.confidence > 0.75 {
                info!(
                    "Implementing formation change: {} -> {} ({})",
                    change.from.map(|f| f.display_name()).unwrap_or("None"),
                    change.to.display_name(),
                    change.reason
                );

                self.tactics = Some(Tactics::with_reason(
                    change.to,
                    TacticSelectionReason::TeamComposition,
                    change.confidence,
                ));
            }
        }

        // Log important recommendations
        for rec in &decisions.recommendations {
            match rec.priority {
                RecommendationPriority::High | RecommendationPriority::Critical => {
                    log::warn!(
                        "[{}] {}: {}",
                        if rec.priority == RecommendationPriority::Critical {
                            "CRITICAL"
                        } else {
                            "HIGH"
                        },
                        format!("{:?}", rec.category),
                        rec.description
                    );
                }
                _ => {
                    debug!(
                        "[{:?}] {}: {}",
                        rec.priority,
                        format!("{:?}", rec.category),
                        rec.description
                    );
                }
            }
        }

        decisions
    }

    /// Method to adapt tactics during a match
    pub fn adapt_tactics_during_match(
        &mut self,
        score_difference: i8,
        minutes_played: u8,
        is_home: bool,
    ) -> Option<Tactics> {
        let current_tactic = &self.tactics().tactic_type;
        let available_players: Vec<&Player> = self
            .players
            .players()
            .into_iter()
            .filter(|p| p.is_ready_for_match())
            .collect();

        TacticsSelector::select_situational_tactic(
            current_tactic,
            is_home,
            score_difference,
            minutes_played,
            &available_players,
        )
    }
}
