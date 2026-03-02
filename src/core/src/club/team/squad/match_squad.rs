use crate::r#match::{MatchPlayer, MatchSquad, SelectionContext, SquadSelector};
use crate::{Player, Tactics, TacticsSelector, Team};

impl Team {
    /// Get match squad using rotation — prioritizes players who haven't played recently.
    /// Used for friendly/development leagues where all players need game time.
    pub fn get_rotation_match_squad(&self) -> MatchSquad {
        let head_coach = self.staffs.head_coach();

        let squad_result = SquadSelector::select_for_rotation(self, head_coach);

        let final_tactics = self
            .tactics
            .as_ref()
            .cloned()
            .unwrap_or_else(|| TacticsSelector::select(self, head_coach));

        self.validate_squad_selection(&squad_result, &final_tactics);

        MatchSquad {
            team_id: self.id,
            team_name: self.name.clone(),
            tactics: final_tactics,
            main_squad: squad_result.main_squad,
            substitutes: squad_result.substitutes,
            captain_id: self.select_captain(),
            vice_captain_id: self.select_vice_captain(),
            penalty_taker_id: self.select_penalty_taker(),
            free_kick_taker_id: self.select_free_kick_taker(),
        }
    }

    /// Get match squad using rotation with supplementary players from other club teams.
    /// Ensures non-main teams always have enough players for a full squad.
    pub fn get_rotation_match_squad_with_reserves(&self, reserve_players: &[&Player], ctx: &SelectionContext) -> MatchSquad {
        let head_coach = self.staffs.head_coach();

        let squad_result =
            SquadSelector::select_for_rotation_with_context(self, head_coach, reserve_players, ctx);

        let final_tactics = self
            .tactics
            .as_ref()
            .cloned()
            .unwrap_or_else(|| TacticsSelector::select(self, head_coach));

        self.validate_squad_selection(&squad_result, &final_tactics);

        MatchSquad {
            team_id: self.id,
            team_name: self.name.clone(),
            tactics: final_tactics,
            main_squad: squad_result.main_squad,
            substitutes: squad_result.substitutes,
            captain_id: self.select_captain(),
            vice_captain_id: self.select_vice_captain(),
            penalty_taker_id: self.select_penalty_taker(),
            free_kick_taker_id: self.select_free_kick_taker(),
        }
    }

    /// Enhanced get_match_squad that uses improved tactical analysis
    /// Accepts optional reserve players that can be selected for the match squad
    pub fn get_enhanced_match_squad(&self, reserve_players: &[&Player], ctx: &SelectionContext) -> MatchSquad {
        let head_coach = self.staffs.head_coach();

        // Use squad selection with reserve pool
        let squad_result = SquadSelector::select_with_context(self, head_coach, reserve_players, ctx);

        // Step 3: Create match squad with selected tactics
        let final_tactics = self
            .tactics
            .as_ref()
            .cloned()
            .unwrap_or_else(|| TacticsSelector::select(self, head_coach));

        // Step 5: Validate squad selection
        self.validate_squad_selection(&squad_result, &final_tactics);

        MatchSquad {
            team_id: self.id,
            team_name: self.name.clone(),
            tactics: final_tactics,
            main_squad: squad_result.main_squad,
            substitutes: squad_result.substitutes,
            captain_id: self.select_captain(),
            vice_captain_id: self.select_vice_captain(),
            penalty_taker_id: self.select_penalty_taker(),
            free_kick_taker_id: self.select_free_kick_taker(),
        }
    }

    fn validate_squad_selection(
        &self,
        squad_result: &crate::r#match::squad::PlayerSelectionResult,
        tactics: &Tactics,
    ) {
        let formation_positions = tactics.positions();

        if squad_result.main_squad.len() != formation_positions.len() {
            log::warn!(
                "Squad size mismatch: got {} players for {} positions",
                squad_result.main_squad.len(),
                formation_positions.len()
            );
        }

        let mut position_coverage = std::collections::HashMap::new();
        for match_player in &squad_result.main_squad {
            let pos = match_player.tactical_position.current_position;
            *position_coverage.entry(pos).or_insert(0) += 1;
        }

        for &required_pos in formation_positions {
            if !position_coverage.contains_key(&required_pos) {
                log::warn!(
                    "No player selected for required position: {}",
                    required_pos.get_short_name()
                );
            }
        }
    }

    /// Select team captain based on leadership and experience
    fn select_captain(&self) -> Option<MatchPlayer> {
        self.players
            .players()
            .iter()
            .filter(|p| !p.player_attributes.is_injured && !p.player_attributes.is_banned)
            .max_by(|a, b| {
                let leadership_a = a.skills.mental.leadership;
                let leadership_b = b.skills.mental.leadership;
                let experience_a = a.player_attributes.international_apps;
                let experience_b = b.player_attributes.international_apps;

                (leadership_a + experience_a as f32 / 10.0)
                    .partial_cmp(&(leadership_b + experience_b as f32 / 10.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| MatchPlayer::from_player(self.id, p, p.position(), false))
    }

    /// Select vice captain
    fn select_vice_captain(&self) -> Option<MatchPlayer> {
        // Similar logic to captain but exclude current captain
        // Implementation would be similar to select_captain
        None
    }

    /// Select penalty taker based on penalty taking skill and composure
    fn select_penalty_taker(&self) -> Option<MatchPlayer> {
        self.players
            .players()
            .iter()
            .filter(|p| !p.player_attributes.is_injured && !p.player_attributes.is_banned)
            .max_by(|a, b| {
                let penalty_skill_a = a.skills.technical.penalty_taking + a.skills.mental.composure;
                let penalty_skill_b = b.skills.technical.penalty_taking + b.skills.mental.composure;

                penalty_skill_a
                    .partial_cmp(&penalty_skill_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| MatchPlayer::from_player(self.id, p, p.position(), false))
    }

    /// Select free kick taker based on free kick skill and technique
    fn select_free_kick_taker(&self) -> Option<MatchPlayer> {
        self.players
            .players()
            .iter()
            .filter(|p| !p.player_attributes.is_injured && !p.player_attributes.is_banned)
            .max_by(|a, b| {
                let fk_skill_a = a.skills.technical.free_kicks + a.skills.technical.technique;
                let fk_skill_b = b.skills.technical.free_kicks + b.skills.technical.technique;

                fk_skill_a
                    .partial_cmp(&fk_skill_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| MatchPlayer::from_player(self.id, p, p.position(), false))
    }
}
