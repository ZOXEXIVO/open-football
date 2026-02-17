use crate::club::team::behaviour::TeamBehaviour;
use crate::context::GlobalContext;
use crate::r#match::{
    EnhancedTacticsSelector, MatchPlayer, MatchSquad, SquadSelector,
};
use crate::shared::CurrencyValue;
use crate::{MatchHistory, MatchTacticType, Player, PlayerCollection, RecommendationPriority, StaffCollection, Tactics, TacticsSelector, TeamReputation, TeamResult, TeamTraining, TrainingSchedule, TransferItem, Transfers};
use log::{debug, info};
use std::borrow::Cow;
use std::str::FromStr;
use crate::club::team::builder::TeamBuilder;

#[derive(Debug, PartialEq)]
pub enum TeamType {
    Main = 0,
    B = 1,
    U18 = 2,
    U19 = 3,
    U21 = 4,
    U23 = 5,
}

#[derive(Debug)]
pub struct Team {
    pub id: u32,
    pub league_id: u32,
    pub club_id: u32,
    pub name: String,
    pub slug: String,

    pub team_type: TeamType,
    pub tactics: Option<Tactics>,

    pub players: PlayerCollection,
    pub staffs: StaffCollection,

    pub behaviour: TeamBehaviour,

    pub reputation: TeamReputation,
    pub training_schedule: TrainingSchedule,
    pub transfer_list: Transfers,
    pub match_history: MatchHistory,
}

impl Team {
    pub fn builder() -> TeamBuilder {
        TeamBuilder::new()
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> TeamResult {
        let date = ctx.simulation.date.date();

        // Staff responsible for outgoing transfers evaluates squad
        let players_refs: Vec<&Player> = self.players.players();
        let staff_transfer_list = self.staffs.evaluate_outgoing_transfers(&players_refs, date);

        let result = TeamResult::new(
            self.id,
            self.players.simulate(ctx.with_player(None)),
            self.staffs.simulate(ctx.with_staff(None)),
            self.behaviour
                .simulate(&mut self.players, &mut self.staffs, ctx.with_team(self.id)),
            TeamTraining::train(self, ctx.simulation.date),
            staff_transfer_list,
        );

        if self.tactics.is_none() {
            self.tactics = Some(TacticsSelector::select(self, self.staffs.head_coach()));
        };

        if self.training_schedule.is_default {
            //let coach = self.staffs.head_coach();
        }

        result
    }

    pub fn players(&self) -> Vec<&Player> {
        self.players.players()
    }

    pub fn add_player_to_transfer_list(&mut self, player_id: u32, value: CurrencyValue) {
        self.transfer_list.add(TransferItem {
            player_id,
            amount: value,
        })
    }

    pub fn get_week_salary(&self) -> u32 {
        self.players
            .players
            .iter()
            .filter_map(|p| p.contract.as_ref())
            .map(|c| c.salary)
            .chain(
                self.staffs
                    .staffs
                    .iter()
                    .filter_map(|p| p.contract.as_ref())
                    .map(|c| c.salary),
            )
            .sum()
    }

    /// Enhanced get_match_squad that uses improved tactical analysis
    pub fn get_enhanced_match_squad(&self) -> MatchSquad {
        let head_coach = self.staffs.head_coach();
        
        // Step 2: Use enhanced squad selection
        let squad_result = SquadSelector::select(self, head_coach);

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

    /// Adaptive tactics during a match based on game state
    pub fn adapt_tactics_during_match_enhanced(
        &mut self,
        score_difference: i8,
        minutes_played: u8,
        is_home: bool,
        team_morale: f32,
    ) -> Option<Tactics> {
        let current_tactic = &self.tactics().tactic_type;
        let available_players: Vec<&crate::Player> = self
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
                    crate::TacticSelectionReason::TeamComposition,
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

    pub fn tactics(&self) -> Cow<'_, Tactics> {
        if let Some(tactics) = &self.tactics {
            Cow::Borrowed(tactics)
        } else {
            Cow::Owned(Tactics::new(MatchTacticType::T442))
        }
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

impl FromStr for TeamType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Main" => Ok(TeamType::Main),
            "B" => Ok(TeamType::B),
            "U18" => Ok(TeamType::U18),
            "U19" => Ok(TeamType::U19),
            "U21" => Ok(TeamType::U21),
            "U23" => Ok(TeamType::U23),
            _ => Err(format!("'{}' is not a valid value for WSType", s)),
        }
    }
}
