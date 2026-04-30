//! Pair-wise full-update dynamics: position-group competition / synergy,
//! age-group bonding, performance-based admiration vs envy, and
//! personality conflicts.

use super::TeamBehaviour;
use crate::club::team::behaviour::{PlayerRelationshipChangeResult, TeamBehaviourResult};
use crate::context::GlobalContext;
use crate::{ChangeType, Person, PlayerCollection};

impl TeamBehaviour {
    /// Players in similar positions compete; complementary positions bond
    pub(super) fn process_position_group_dynamics(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let position_i = player_i.position();
                let position_j = player_j.position();

                if position_i == position_j {
                    // Halved here because `process_unit_partnerships`
                    // already emits the high-signal same-position rivalry
                    // for this exact pair (with depth-chart asymmetry,
                    // ambition gating, etc.). Keeping a half-strength
                    // legacy contribution lets coarse signals (CA gap,
                    // reputation amplification) still register without
                    // double-counting the headline rivalry.
                    let competition_factor =
                        Self::calculate_competition_factor(player_i, player_j) * 0.5;

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: -competition_factor,
                            change_type: ChangeType::CompetitionRivalry,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: -competition_factor,
                            change_type: ChangeType::CompetitionRivalry,
                        });
                } else if Self::are_complementary_positions(&position_i, &position_j) {
                    let synergy_factor = Self::calculate_synergy_factor(player_i, player_j);

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: synergy_factor,
                            change_type: ChangeType::TrainingBonding,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: synergy_factor,
                            change_type: ChangeType::TrainingBonding,
                        });
                }
            }
        }
    }

    /// Age groups form bonds - young players stick together, veterans mentor youth
    pub(super) fn process_age_group_dynamics(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        let current_date = ctx.simulation.date.date();

        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let age_i = player_i.age(current_date);
                let age_j = player_j.age(current_date);

                let age_diff = (age_i as i32 - age_j as i32).abs();
                let relationship_change =
                    Self::calculate_age_relationship_factor(age_i, age_j, age_diff);

                if relationship_change.abs() > 0.01 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change,
                            change_type: ChangeType::NaturalProgression,
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change,
                            change_type: ChangeType::NaturalProgression,
                        });
                }
            }
        }
    }

    /// Performance-based relationships using stats and reputation
    pub(super) fn process_performance_based_relationships(
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
                    player_i,
                    player_j,
                );

                if relationship_change.abs() > 0.01 {
                    let change_type = if relationship_change > 0.0 {
                        ChangeType::MatchCooperation
                    } else {
                        ChangeType::CompetitionRivalry
                    };

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change,
                            change_type: change_type.clone(),
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change,
                            change_type,
                        });
                }
            }
        }
    }

    /// Personality conflicts
    pub(super) fn process_personality_conflicts(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let conflict_factor = Self::calculate_personality_conflict(player_i, player_j);

                if conflict_factor.abs() > 0.02 {
                    let change_type = if conflict_factor > 0.0 {
                        ChangeType::PersonalSupport
                    } else {
                        ChangeType::PersonalConflict
                    };

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: conflict_factor,
                            change_type: change_type.clone(),
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: conflict_factor,
                            change_type,
                        });
                }
            }
        }
    }
}
