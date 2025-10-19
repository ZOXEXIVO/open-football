use crate::club::team::behaviour::{PlayerRelationshipChangeResult, TeamBehaviourResult};
use crate::context::GlobalContext;
use crate::utils::FloatUtils;
use crate::{Person, PersonBehaviourState, Player, PlayerCollection, PlayerPositionType, PlayerRelation, StaffCollection};
use chrono::{Datelike, NaiveDateTime};
use log::{debug, info};

#[derive(Debug)]
pub struct TeamBehaviour {
    last_full_update: Option<NaiveDateTime>,
    last_minor_update: Option<NaiveDateTime>,
}

impl Default for TeamBehaviour {
    fn default() -> Self {
        Self::new()
    }
}

impl TeamBehaviour {
    pub fn new() -> Self {
        TeamBehaviour {
            last_full_update: None,
            last_minor_update: None,
        }
    }

    /// Main simulate function that decides what type of update to run
    pub fn simulate(
        &mut self,
        players: &mut PlayerCollection,
        staffs: &mut StaffCollection,
        ctx: GlobalContext<'_>,
    ) -> TeamBehaviourResult {
        let current_time = ctx.simulation.date;

        // Determine update frequency based on simulation settings
        let should_run_full = self.should_run_full_update(current_time);
        let should_run_minor = self.should_run_minor_update(current_time);

        if should_run_full {
            info!("🏟️  Running FULL team behaviour update at {}", current_time);
            self.last_full_update = Some(current_time);
            self.run_full_behaviour_simulation(players, staffs, ctx)
        } else if should_run_minor {
            debug!("⚽ Running minor team behaviour update at {}", current_time);
            self.last_minor_update = Some(current_time);
            self.run_minor_behaviour_simulation(players, staffs, ctx)
        } else {
            debug!("⏸️  Skipping team behaviour update at {}", current_time);
            TeamBehaviourResult::new()
        }
    }

    /// Determine if we should run a full behaviour update
    fn should_run_full_update(&self, current_time: NaiveDateTime) -> bool {
        match self.last_full_update {
            None => true, // First run
            Some(last) => {
                let days_since = current_time.signed_duration_since(last).num_days();

                // Run full update every week (7 days)
                days_since >= 7 ||

                    // Or if it's a match day (more interactions)
                    current_time.weekday() == chrono::Weekday::Sat ||
                    current_time.weekday() == chrono::Weekday::Sun ||

                    // Or beginning of month (team meetings, etc.)
                    current_time.day() == 1
            }
        }
    }

    fn should_run_minor_update(&self, current_time: NaiveDateTime) -> bool {
        match self.last_minor_update {
            None => true, // First run
            Some(last) => {
                let days_since = current_time.signed_duration_since(last).num_days();

                // Run minor updates every 2-3 days
                days_since >= 2 ||

                    // Or during training intensive periods (Tuesday to Thursday)
                    matches!(current_time.weekday(),
                    chrono::Weekday::Tue | chrono::Weekday::Wed | chrono::Weekday::Thu)
            }
        }
    }

    /// Full comprehensive behaviour simulation
    fn run_full_behaviour_simulation(
        &self,
        players: &mut PlayerCollection,
        _staffs: &mut StaffCollection,
        ctx: GlobalContext<'_>,
    ) -> TeamBehaviourResult {
        info!("🔄 Processing comprehensive team dynamics...");

        let mut result = TeamBehaviourResult::new();

        // Log team state before processing
        Self::log_team_state(players, "BEFORE full update");

        // Process all interaction types
        Self::process_position_group_dynamics(players, &mut result);
        Self::process_age_group_dynamics(players, &mut result);
        Self::process_performance_based_relationships(players, &mut result);
        Self::process_personality_conflicts(players, &mut result);
        Self::process_leadership_influence(players, &mut result);
        Self::process_playing_time_jealousy(players, &mut result);

        // Additional full-update only processes
        Self::process_contract_satisfaction(players, &mut result, &ctx);
        Self::process_injury_sympathy(players, &mut result, &ctx);
        Self::process_international_duty_bonds(players, &mut result, &ctx);

        info!(
            "✅ Full team behaviour update complete - {} relationship changes",
            result.players.relationship_result.len()
        );

        result
    }

    /// Lighter, more frequent behaviour updates
    fn run_minor_behaviour_simulation(
        &self,
        players: &mut PlayerCollection,
        _staffs: &mut StaffCollection,
        ctx: GlobalContext<'_>,
    ) -> TeamBehaviourResult {
        debug!("🔄 Processing minor team dynamics...");

        let mut result = TeamBehaviourResult::new();

        // Only process most dynamic relationships for minor updates
        Self::process_daily_interactions(players, &mut result, &ctx);
        Self::process_mood_changes(players, &mut result, &ctx);
        Self::process_recent_performance_reactions(players, &mut result);

        // Log results if there are changes
        if !result.players.relationship_result.is_empty() {
            debug!(
                "✅ Minor team behaviour update complete - {} relationship changes",
                result.players.relationship_result.len()
            );
        }

        result
    }

    /// Log current team relationship state
    fn log_team_state(players: &PlayerCollection, context: &str) {
        debug!(
            "📊 Team State {}: {} players",
            context,
            players.players.len()
        );

        let mut happy_players = 0;
        let mut unhappy_players = 0;
        let mut neutral_players = 0;

        for player in &players.players {
            let happiness = Self::calculate_player_happiness(player);
            if happiness > 0.2 {
                happy_players += 1;
            } else if happiness < -0.2 {
                unhappy_players += 1;
            } else {
                neutral_players += 1;
            }
        }

        info!(
            "😊 Happy: {} | 😐 Neutral: {} | 😠 Unhappy: {}",
            happy_players, neutral_players, unhappy_players
        );
    }

    fn process_daily_interactions(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        debug!("🗣️  Processing daily player interactions...");

        // Random small interactions between players who already know each other
        for i in 0..players.players.len().min(10) {
            // Limit for performance
            for j in i + 1..players.players.len().min(10) {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                // Check if they already have a relationship
                if let Some(existing_relationship) = player_i.relations.get_player(player_j.id) {
                    // Small random fluctuations in existing relationships
                    let daily_change = Self::calculate_daily_interaction_change(
                        player_i,
                        player_j,
                        existing_relationship,
                        ctx,
                    );

                    if daily_change.abs() > 0.005 {
                        result
                            .players
                            .relationship_result
                            .push(PlayerRelationshipChangeResult {
                                from_player_id: player_i.id,
                                to_player_id: player_j.id,
                                relationship_change: daily_change,
                            });
                    }
                }
            }
        }
    }

    /// Process mood-based relationship changes
    fn process_mood_changes(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
        debug!("😔 Processing player mood changes...");

        for player in &players.players {
            let current_happiness = Self::calculate_player_happiness(player);

            // Very unhappy players affect team morale
            if current_happiness < -0.5 {
                for other_player in &players.players {
                    if player.id != other_player.id {
                        let mood_impact =
                            Self::calculate_mood_spread(player, other_player, current_happiness);

                        if mood_impact.abs() > 0.01 {
                            result.players.relationship_result.push(
                                PlayerRelationshipChangeResult {
                                    from_player_id: other_player.id,
                                    to_player_id: player.id,
                                    relationship_change: mood_impact,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    /// Process reactions to recent match performances
    fn process_recent_performance_reactions(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        debug!("⚽ Processing recent performance reactions...");

        // This would ideally check recent match results, but for now simulate
        // reactions to recent stat changes
        for player in &players.players {
            // Players who scored recently get temporary popularity boost
            if player.statistics.goals > 0 && player.position().is_forward() {
                let popularity_boost = 0.05;

                for other_player in &players.players {
                    if player.id != other_player.id {
                        result
                            .players
                            .relationship_result
                            .push(PlayerRelationshipChangeResult {
                                from_player_id: other_player.id,
                                to_player_id: player.id,
                                relationship_change: popularity_boost,
                            });
                    }
                }
            }
        }
    }

    // ========== ADDITIONAL FULL UPDATE PROCESSES ==========

    /// Process contract satisfaction effects on relationships
    fn process_contract_satisfaction(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
        debug!("💰 Processing contract satisfaction effects...");

        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let contract_jealousy = Self::calculate_contract_jealousy(player_i, player_j);

                if contract_jealousy.abs() > 0.02 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: contract_jealousy,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: contract_jealousy,
                        });
                }
            }
        }
    }

    /// Process injury sympathy and support
    fn process_injury_sympathy(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
        debug!("🏥 Processing injury sympathy...");

        for injured_player in players
            .players
            .iter()
            .filter(|p| p.player_attributes.is_injured)
        {
            for other_player in &players.players {
                if injured_player.id != other_player.id {
                    let sympathy = Self::calculate_injury_sympathy(injured_player, other_player);

                    if sympathy > 0.01 {
                        result
                            .players
                            .relationship_result
                            .push(PlayerRelationshipChangeResult {
                                from_player_id: other_player.id,
                                to_player_id: injured_player.id,
                                relationship_change: sympathy,
                            });
                    }
                }
            }
        }
    }

    /// Process international duty bonding
    fn process_international_duty_bonds(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
        debug!("🌍 Processing international duty bonds...");

        // Group players by country
        use std::collections::HashMap;
        let mut country_groups: HashMap<u32, Vec<&Player>> = HashMap::new();

        for player in &players.players {
            country_groups
                .entry(player.country_id)
                .or_default()
                .push(player);
        }

        // Players from same country bond
        for (_, country_players) in country_groups {
            if country_players.len() > 1 {
                for i in 0..country_players.len() {
                    for j in i + 1..country_players.len() {
                        let bond_strength = Self::calculate_national_team_bond(
                            country_players[i],
                            country_players[j],
                        );

                        if bond_strength > 0.01 {
                            result.players.relationship_result.push(
                                PlayerRelationshipChangeResult {
                                    from_player_id: country_players[i].id,
                                    to_player_id: country_players[j].id,
                                    relationship_change: bond_strength,
                                },
                            );

                            result.players.relationship_result.push(
                                PlayerRelationshipChangeResult {
                                    from_player_id: country_players[j].id,
                                    to_player_id: country_players[i].id,
                                    relationship_change: bond_strength,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    // ========== CALCULATION HELPER FUNCTIONS ==========

    fn calculate_daily_interaction_change(
        player_a: &Player,
        player_b: &Player,
        existing_relationship: &PlayerRelation,  // Changed from f32 to &PlayerRelation
        _ctx: &GlobalContext<'_>,
    ) -> f32 {
        // Extract the relationship level from the PlayerRelation struct
        let relationship_level = existing_relationship.level;

        // Small random fluctuations based on personalities
        let temperament_factor =
            (player_a.attributes.temperament + player_b.attributes.temperament) / 40.0;
        let base_change = FloatUtils::random(-0.02, 0.02) * temperament_factor;

        // Additional factors from the enhanced relationship data
        let trust_factor = existing_relationship.trust / 100.0;
        let friendship_factor = existing_relationship.friendship / 100.0;

        // Relationship decay/improvement tendency
        if relationship_level > 50.0 {
            // Very good relationships tend to stay stable but can still improve slightly
            // Trust and friendship help maintain good relationships
            let stability_bonus = (trust_factor * 0.3 + friendship_factor * 0.2) * base_change;
            base_change * 0.5 + stability_bonus
        } else if relationship_level < -50.0 {
            // Very bad relationships might improve over time
            // Low trust makes improvement harder
            let improvement_chance = base_change + 0.01 * (1.0 - trust_factor);
            improvement_chance
        } else {
            // Neutral relationships are more volatile
            // Professional respect can stabilize neutral relationships
            let professional_factor = existing_relationship.professional_respect / 100.0;
            base_change * (1.0 - professional_factor * 0.3)
        }
    }

    fn calculate_daily_interaction_change_simple(
        player_a: &Player,
        player_b: &Player,
        existing_relationship_level: f32,  // Just the level value
        _ctx: &GlobalContext<'_>,
    ) -> f32 {
        // Small random fluctuations based on personalities
        let temperament_factor =
            (player_a.attributes.temperament + player_b.attributes.temperament) / 40.0;
        let base_change = FloatUtils::random(-0.02, 0.02) * temperament_factor;

        // Relationship decay/improvement tendency
        if existing_relationship_level > 50.0 {
            // Very good relationships tend to stay stable
            base_change * 0.5
        } else if existing_relationship_level < -50.0 {
            // Very bad relationships might improve over time
            base_change + 0.01
        } else {
            base_change
        }
    }

    fn calculate_mood_spread(
        unhappy_player: &Player,
        other_player: &Player,
        happiness: f32,
    ) -> f32 {
        // Unhappy players with high leadership spread negativity more
        let influence = (unhappy_player.skills.mental.leadership / 20.0) * happiness.abs() * 0.1;

        // Players with high professionalism resist negative influence
        let resistance = other_player.attributes.professionalism / 20.0;

        influence * (1.0 - resistance).max(0.0)
    }

    fn calculate_contract_jealousy(player_a: &Player, player_b: &Player) -> f32 {
        let salary_a = player_a.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        let salary_b = player_b.contract.as_ref().map(|c| c.salary).unwrap_or(0);

        if salary_a == 0 || salary_b == 0 {
            return 0.0; // Can't compare without contracts
        }

        let salary_ratio = salary_a as f32 / salary_b as f32;

        if salary_ratio > 2.0 || salary_ratio < 0.5 {
            // Large salary differences create jealousy
            let jealousy_factor =
                (player_a.attributes.ambition + player_b.attributes.ambition) / 40.0;
            -0.1 * jealousy_factor
        } else {
            0.0
        }
    }

    fn calculate_injury_sympathy(_injured_player: &Player, other_player: &Player) -> f32 {
        // More empathetic players show more sympathy
        let empathy = other_player.attributes.sportsmanship / 20.0;

        // Team players show more concern
        let team_spirit = other_player.skills.mental.teamwork / 20.0;

        (empathy + team_spirit) * 0.1
    }

    fn calculate_national_team_bond(player_a: &Player, player_b: &Player) -> f32 {
        // International experience creates stronger bonds
        let int_experience_a =
            (player_a.player_attributes.international_apps as f32 / 50.0).min(1.0);
        let int_experience_b =
            (player_b.player_attributes.international_apps as f32 / 50.0).min(1.0);

        (int_experience_a + int_experience_b) * 0.05
    }

    // Include all the previous helper functions from the earlier implementation
    // (calculate_competition_factor, calculate_synergy_factor, etc.)
    // ... [Previous helper functions would go here] ...

    fn calculate_player_happiness(player: &Player) -> f32 {
        let mut happiness = 0.0;

        // Contract satisfaction
        happiness += player
            .contract
            .as_ref()
            .map(|c| (c.salary as f32 / 10000.0).min(1.0))
            .unwrap_or(-0.5);

        // Playing time satisfaction
        if player.statistics.played > 20 {
            happiness += 0.3;
        } else if player.statistics.played > 10 {
            happiness += 0.1;
        } else {
            happiness -= 0.2;
        }

        // Performance satisfaction
        let goals_ratio = player.statistics.goals as f32 / player.statistics.played.max(1) as f32;
        if player.position().is_forward() && goals_ratio > 0.5 {
            happiness += 0.2;
        } else if !player.position().is_forward() && goals_ratio > 0.3 {
            happiness += 0.15;
        }

        // Personality factors
        happiness += (player.attributes.professionalism - 10.0) / 100.0;
        happiness -= (player.attributes.controversy - 10.0) / 50.0;

        // Behavior state affects happiness
        match player.behaviour.state {
            PersonBehaviourState::Good => happiness += 0.2,
            PersonBehaviourState::Poor => happiness -= 0.3,
            PersonBehaviourState::Normal => {}
        }

        happiness.clamp(-1.0, 1.0)
    }

    // pub fn simulate(
    //     players: &mut PlayerCollection,
    //     _staffs: &mut StaffCollection,
    // ) -> TeamBehaviourResult {
    //     let mut result = TeamBehaviourResult::new();
    //
    //     // Process different types of interactions
    //     Self::process_position_group_dynamics(players, &mut result);
    //     Self::process_age_group_dynamics(players, &mut result);
    //     Self::process_performance_based_relationships(players, &mut result);
    //     Self::process_personality_conflicts(players, &mut result);
    //     Self::process_leadership_influence(players, &mut result);
    //     Self::process_playing_time_jealousy(players, &mut result);
    //
    //     result
    // }

    /// Players in similar positions tend to have more complex relationships
    /// due to competition for spots
    fn process_position_group_dynamics(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let position_i = player_i.position();
                let position_j = player_j.position();

                // Same position = competition (negative relationship)
                if position_i == position_j {
                    let competition_factor = Self::calculate_competition_factor(player_i, player_j);

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: -competition_factor,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: -competition_factor,
                        });
                }
                // Complementary positions (e.g., defender + midfielder) = positive
                else if Self::are_complementary_positions(&position_i, &position_j) {
                    let synergy_factor = Self::calculate_synergy_factor(player_i, player_j);

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: synergy_factor,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: synergy_factor,
                        });
                }
            }
        }
    }

    /// Age groups naturally form bonds - young players stick together,
    /// veterans mentor youth, but sometimes clash with middle-aged players
    fn process_age_group_dynamics(players: &PlayerCollection, result: &mut TeamBehaviourResult) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                // Calculate ages (using a fixed date for consistency)
                let current_date = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
                let age_i = player_i.age(current_date);
                let age_j = player_j.age(current_date);

                let age_diff = (age_i as i32 - age_j as i32).abs();
                let relationship_change =
                    Self::calculate_age_relationship_factor(age_i, age_j, age_diff);

                if relationship_change.abs() > 0.01 {
                    // Only process significant changes
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change,
                        });
                }
            }
        }
    }

    /// Players with similar performance levels respect each other,
    /// while big performance gaps can create tension
    fn process_performance_based_relationships(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let performance_i = Self::calculate_player_performance_rating(player_i);
                let performance_j = Self::calculate_player_performance_rating(player_j);

                let performance_diff = (performance_i - performance_j).abs();
                let relationship_change = Self::calculate_performance_relationship_factor(
                    performance_i,
                    performance_j,
                    performance_diff,
                );

                if relationship_change.abs() > 0.01 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change,
                        });
                }
            }
        }
    }

    /// Some personality combinations just don't work well together
    fn process_personality_conflicts(players: &PlayerCollection, result: &mut TeamBehaviourResult) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let conflict_factor = Self::calculate_personality_conflict(player_i, player_j);

                if conflict_factor.abs() > 0.02 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: conflict_factor,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: conflict_factor,
                        });
                }
            }
        }
    }

    /// High leadership players influence team morale and relationships
    fn process_leadership_influence(players: &PlayerCollection, result: &mut TeamBehaviourResult) {
        // Find leaders (high leadership skill)
        let leaders: Vec<&Player> = players
            .players
            .iter()
            .filter(|p| p.skills.mental.leadership > 15.0)
            .collect();

        for leader in leaders {
            for player in &players.players {
                if leader.id == player.id {
                    continue;
                }

                let influence = Self::calculate_leadership_influence(leader, player);

                if influence.abs() > 0.01 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player.id,
                            to_player_id: leader.id,
                            relationship_change: influence,
                        });
                }
            }
        }
    }

    /// Players with similar playing time are more likely to bond,
    /// while those with very different playing time might be jealous
    fn process_playing_time_jealousy(players: &PlayerCollection, result: &mut TeamBehaviourResult) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let playing_time_i = player_i.statistics.played;
                let playing_time_j = player_j.statistics.played;

                let jealousy_factor = Self::calculate_playing_time_jealousy(
                    playing_time_i,
                    playing_time_j,
                    player_i,
                    player_j,
                );

                if jealousy_factor.abs() > 0.01 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: jealousy_factor,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: jealousy_factor,
                        });
                }
            }
        }
    }

    // Helper functions for calculations

    fn calculate_competition_factor(player_a: &Player, player_b: &Player) -> f32 {
        let ability_diff = (player_a.player_attributes.current_ability as f32
            - player_b.player_attributes.current_ability as f32)
            .abs();

        // More similar abilities = more competition
        let competition_base = 0.3 - (ability_diff / 100.0);

        // Ambition increases competition
        let ambition_factor = (player_a.attributes.ambition + player_b.attributes.ambition) / 40.0;

        (competition_base * ambition_factor).max(0.0).min(0.5)
    }

    fn calculate_synergy_factor(player_a: &Player, player_b: &Player) -> f32 {
        // Players with good teamwork create positive relationships
        let teamwork_factor =
            (player_a.skills.mental.teamwork + player_b.skills.mental.teamwork) / 40.0;

        // Professional players work well together
        let professionalism_factor =
            (player_a.attributes.professionalism + player_b.attributes.professionalism) / 40.0;

        (teamwork_factor * professionalism_factor * 0.2).min(0.3)
    }

    fn are_complementary_positions(pos_a: &PlayerPositionType, pos_b: &PlayerPositionType) -> bool {
        use PlayerPositionType::*;

        match (pos_a, pos_b) {
            // Defenders and midfielders work well together
            (
                DefenderCenter | DefenderLeft | DefenderRight,
                MidfielderCenter | MidfielderLeft | MidfielderRight | DefensiveMidfielder,
            ) => true,

            // Midfielders and forwards complement each other
            (
                MidfielderCenter | MidfielderLeft | MidfielderRight | AttackingMidfielderCenter,
                Striker | ForwardLeft | ForwardRight | ForwardCenter,
            ) => true,

            // Reverse combinations
            (
                MidfielderCenter | MidfielderLeft | MidfielderRight | DefensiveMidfielder,
                DefenderCenter | DefenderLeft | DefenderRight,
            ) => true,

            (
                Striker | ForwardLeft | ForwardRight | ForwardCenter,
                MidfielderCenter | MidfielderLeft | MidfielderRight | AttackingMidfielderCenter,
            ) => true,

            _ => false,
        }
    }

    fn calculate_age_relationship_factor(age_a: u8, age_b: u8, age_diff: i32) -> f32 {
        match (age_a, age_b) {
            // Both young (16-22) - natural bonding
            (16..=22, 16..=22) if age_diff <= 3 => FloatUtils::random(0.1, 0.25),

            // Young and experienced (30+) - mentorship potential
            (16..=22, 30..) | (30.., 16..=22) => {
                // But depends on personalities
                FloatUtils::random(-0.1, 0.2)
            }

            // Prime age players (23-29) - competitive tension
            (23..=29, 23..=29) if age_diff <= 2 => FloatUtils::random(-0.15, 0.1),

            // Large age gaps in prime years - respect or resentment
            _ if age_diff > 8 => FloatUtils::random(-0.2, 0.15),

            // Similar ages in general - slight positive
            _ if age_diff <= 2 => FloatUtils::random(0.0, 0.1),

            _ => 0.0,
        }
    }

    fn calculate_player_performance_rating(player: &Player) -> f32 {
        let goals_factor =
            (player.statistics.goals as f32 / (player.statistics.played.max(1) as f32)) * 10.0;
        let assists_factor =
            (player.statistics.assists as f32 / (player.statistics.played.max(1) as f32)) * 5.0;
        let appearance_factor = (player.statistics.played as f32 / 30.0).min(1.0) * 5.0;
        let rating_factor = player.statistics.average_rating;

        (goals_factor + assists_factor + appearance_factor + rating_factor) / 4.0
    }

    fn calculate_performance_relationship_factor(perf_a: f32, perf_b: f32, diff: f32) -> f32 {
        if diff < 1.0 {
            // Similar performance - mutual respect
            FloatUtils::random(0.05, 0.2)
        } else if diff > 3.0 {
            // Large performance gap - could create tension or admiration
            if perf_a > perf_b {
                // Higher performer might look down
                FloatUtils::random(-0.1, 0.05)
            } else {
                // Lower performer might be jealous or inspired
                FloatUtils::random(-0.15, 0.1)
            }
        } else {
            0.0
        }
    }

    fn calculate_personality_conflict(player_a: &Player, player_b: &Player) -> f32 {
        // High controversy players clash with professional players
        let controversy_clash = if player_a.attributes.controversy > 15.0
            && player_b.attributes.professionalism > 15.0
            || player_b.attributes.controversy > 15.0 && player_a.attributes.professionalism > 15.0
        {
            -0.3
        } else {
            0.0
        };

        // High temperament players might clash
        let temperament_clash =
            if player_a.attributes.temperament > 18.0 && player_b.attributes.temperament > 18.0 {
                FloatUtils::random(-0.2, -0.05)
            } else {
                0.0
            };

        // Different behavioral states can cause friction
        let behavior_clash = match (&player_a.behaviour.state, &player_b.behaviour.state) {
            (PersonBehaviourState::Poor, PersonBehaviourState::Good)
            | (PersonBehaviourState::Good, PersonBehaviourState::Poor) => -0.15,
            _ => 0.0,
        };

        // Loyalty and professionalism create bonds
        let positive_traits =
            if player_a.attributes.loyalty > 15.0 && player_b.attributes.loyalty > 15.0 {
                0.1
            } else {
                0.0
            };

        controversy_clash + temperament_clash + behavior_clash + positive_traits
    }

    fn calculate_leadership_influence(leader: &Player, player: &Player) -> f32 {
        let leadership_strength = leader.skills.mental.leadership / 20.0;

        // Good leaders positively influence well-behaved players
        let influence = match player.behaviour.state {
            PersonBehaviourState::Good => leadership_strength * 0.2,
            PersonBehaviourState::Normal => leadership_strength * 0.1,
            PersonBehaviourState::Poor => {
                // Poor behavior players might resist or benefit from leadership
                if player.attributes.professionalism > 10.0 {
                    leadership_strength * 0.15 // Can be helped
                } else {
                    -leadership_strength * 0.1 // Resists authority
                }
            }
        };

        influence
    }

    fn calculate_playing_time_jealousy(
        time_a: u16,
        time_b: u16,
        player_a: &Player,
        player_b: &Player,
    ) -> f32 {
        let time_diff = (time_a as i32 - time_b as i32).abs();

        if time_diff < 3 {
            // Similar playing time - bond over shared experience
            return FloatUtils::random(0.05, 0.15);
        }

        if time_diff > 10 {
            let ambition_factor =
                (player_a.attributes.ambition + player_b.attributes.ambition) / 40.0;

            // High ambition players get jealous of more playing time
            if time_a < time_b && player_a.attributes.ambition > 15.0 {
                return -0.2 * ambition_factor;
            } else if time_b < time_a && player_b.attributes.ambition > 15.0 {
                return -0.2 * ambition_factor;
            }
        }

        0.0
    }
}
