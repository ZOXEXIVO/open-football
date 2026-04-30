//! Reputation-driven and longer-arc relationship effects: star-status
//! admiration / tension, veteran-to-youth mentorship, contract
//! satisfaction / jealousy, injury sympathy from teammates, and bonds
//! between national-team compatriots.

use super::TeamBehaviour;
use crate::club::team::behaviour::{PlayerRelationshipChangeResult, TeamBehaviourResult};
use crate::context::GlobalContext;
use crate::{ChangeType, Person, Player, PlayerCollection};
use std::collections::HashMap;

impl TeamBehaviour {
    /// Reputation dynamics: star players command respect or create tension
    /// High-reputation players are admired by professional teammates but
    /// resented by ambitious players who feel overshadowed
    pub(super) fn process_reputation_dynamics(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let rep_i = player_i.player_attributes.current_reputation as f32;
                let rep_j = player_j.player_attributes.current_reputation as f32;
                let rep_diff = (rep_i - rep_j).abs();

                if rep_diff < 500.0 {
                    // Similar reputation: mutual professional respect
                    let max_rep = rep_i.max(rep_j);
                    let respect_bonus = 0.03 * (max_rep / 10000.0).clamp(0.1, 1.0);

                    if respect_bonus > 0.005 {
                        result
                            .players
                            .relationship_result
                            .push(PlayerRelationshipChangeResult {
                                from_player_id: player_i.id,
                                to_player_id: player_j.id,
                                relationship_change: respect_bonus,
                                change_type: ChangeType::ReputationAdmiration,
                            });
                        result
                            .players
                            .relationship_result
                            .push(PlayerRelationshipChangeResult {
                                from_player_id: player_j.id,
                                to_player_id: player_i.id,
                                relationship_change: respect_bonus,
                                change_type: ChangeType::ReputationAdmiration,
                            });
                    }
                } else if rep_diff > 3000.0 {
                    // Large reputation gap: admiration or resentment
                    let (star, lesser) = if rep_i > rep_j {
                        (player_i, player_j)
                    } else {
                        (player_j, player_i)
                    };
                    let (star_id, lesser_id) = (star.id, lesser.id);

                    // Lesser player's reaction depends on personality
                    let admiration = (lesser.attributes.sportsmanship / 20.0) * 0.1
                        + (lesser.attributes.professionalism / 20.0) * 0.05;

                    let resentment = if lesser.attributes.ambition > 14.0 {
                        (lesser.attributes.ambition - 14.0) / 6.0 * -0.08
                    } else {
                        0.0
                    };

                    let lesser_to_star = admiration + resentment;

                    if lesser_to_star.abs() > 0.01 {
                        let change_type = if lesser_to_star > 0.0 {
                            ChangeType::ReputationAdmiration
                        } else {
                            ChangeType::ReputationTension
                        };

                        result
                            .players
                            .relationship_result
                            .push(PlayerRelationshipChangeResult {
                                from_player_id: lesser_id,
                                to_player_id: star_id,
                                relationship_change: lesser_to_star,
                                change_type,
                            });
                    }

                    // Star player's reaction: professional stars are approachable,
                    // controversial stars create tension
                    let star_to_lesser = if star.attributes.professionalism > 14.0 {
                        0.04 * (star.attributes.professionalism / 20.0)
                    } else if star.attributes.controversy > 14.0 {
                        -0.06 * (star.attributes.controversy / 20.0)
                    } else {
                        0.0
                    };

                    if star_to_lesser.abs() > 0.01 {
                        let change_type = if star_to_lesser > 0.0 {
                            ChangeType::PersonalSupport
                        } else {
                            ChangeType::ReputationTension
                        };

                        result
                            .players
                            .relationship_result
                            .push(PlayerRelationshipChangeResult {
                                from_player_id: star_id,
                                to_player_id: lesser_id,
                                relationship_change: star_to_lesser,
                                change_type,
                            });
                    }
                } else if rep_diff > 1000.0 {
                    // Moderate reputation gap: small professional respect toward higher-rep player
                    let (higher_id, lower_id) = if rep_i > rep_j {
                        (player_i.id, player_j.id)
                    } else {
                        (player_j.id, player_i.id)
                    };

                    let respect = 0.02 * (rep_i.max(rep_j) / 10000.0).clamp(0.1, 1.0);

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: lower_id,
                            to_player_id: higher_id,
                            relationship_change: respect,
                            change_type: ChangeType::ReputationAdmiration,
                        });
                }
            }
        }
    }

    /// Mentorship dynamics: experienced veterans mentor young players
    /// based on reputation, leadership, age gap, and position compatibility
    pub(super) fn process_mentorship_dynamics(
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
                if age_diff < 5 {
                    continue;
                }

                let (veteran, youth) = if age_i > age_j {
                    (player_i, player_j)
                } else {
                    (player_j, player_i)
                };
                let (vet_age, youth_age) = if age_i > age_j {
                    (age_i, age_j)
                } else {
                    (age_j, age_i)
                };

                // Veteran must be 28+ and youth must be under 24
                if vet_age < 28 || youth_age > 23 {
                    continue;
                }

                // Mentorship potential factors:
                // - Veteran's leadership and experience
                // - Veteran's reputation (experienced, respected players mentor better)
                // - Youth's adaptability (how well they receive mentorship)
                // - Position compatibility (same position = more relevant)
                let leadership_factor = (veteran.skills.mental.leadership / 20.0).clamp(0.0, 1.0);
                let rep_factor =
                    (veteran.player_attributes.current_reputation as f32 / 10000.0).clamp(0.0, 1.0);
                let adaptability_factor = (youth.attributes.adaptability / 20.0).clamp(0.0, 1.0);

                let same_position = veteran.position() == youth.position();
                let position_bonus = if same_position { 1.5 } else { 1.0 };

                // Professionalism of the veteran matters
                let professionalism_factor =
                    (veteran.attributes.professionalism / 20.0).clamp(0.0, 1.0);

                let mentorship_strength = leadership_factor
                    * (0.3 + rep_factor * 0.4 + professionalism_factor * 0.3)
                    * adaptability_factor
                    * position_bonus
                    * 0.12;

                if mentorship_strength > 0.015 {
                    // Youth admires and learns from veteran
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: youth.id,
                            to_player_id: veteran.id,
                            relationship_change: mentorship_strength,
                            change_type: ChangeType::MentorshipBond,
                        });

                    // Veteran gains satisfaction from mentoring (slightly less)
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: veteran.id,
                            to_player_id: youth.id,
                            relationship_change: mentorship_strength * 0.6,
                            change_type: ChangeType::MentorshipBond,
                        });
                }
            }
        }
    }

    pub(super) fn process_contract_satisfaction(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let (jealousy_i_to_j, jealousy_j_to_i) =
                    Self::calculate_contract_jealousy(player_i, player_j);

                if jealousy_i_to_j.abs() > 0.02 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: jealousy_i_to_j,
                            change_type: ChangeType::PersonalConflict,
                        });
                }

                if jealousy_j_to_i.abs() > 0.02 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: jealousy_j_to_i,
                            change_type: ChangeType::PersonalConflict,
                        });
                }
            }
        }
    }

    pub(super) fn process_injury_sympathy(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
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
                                change_type: ChangeType::PersonalSupport,
                            });
                    }
                }
            }
        }
    }

    pub(super) fn process_international_duty_bonds(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
        let mut country_groups: HashMap<u32, Vec<&Player>> = HashMap::new();

        for player in &players.players {
            country_groups
                .entry(player.country_id)
                .or_default()
                .push(player);
        }

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
                                    change_type: ChangeType::TrainingBonding,
                                },
                            );

                            result.players.relationship_result.push(
                                PlayerRelationshipChangeResult {
                                    from_player_id: country_players[j].id,
                                    to_player_id: country_players[i].id,
                                    relationship_change: bond_strength,
                                    change_type: ChangeType::TrainingBonding,
                                },
                            );
                        }
                    }
                }
            }
        }
    }
}
