//! Daily / minor-update passes: small per-tick relationship changes,
//! mood spread from unhappy stars, post-transfer squad integration, and
//! recent-form popularity reactions.

use super::TeamBehaviour;
use crate::club::team::behaviour::{PlayerRelationshipChangeResult, TeamBehaviourResult};
use crate::context::GlobalContext;
use crate::{ChangeType, HappinessEventType, PlayerCollection};

impl TeamBehaviour {
    pub(super) fn process_daily_interactions(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        for i in 0..players.players.len().min(10) {
            for j in i + 1..players.players.len().min(10) {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                if let Some(existing_relationship) = player_i.relations.get_player(player_j.id) {
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
                                change_type: ChangeType::NaturalProgression,
                            });
                    }
                }
            }
        }
    }

    pub(super) fn process_mood_changes(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
        _ctx: &GlobalContext<'_>,
    ) {
        for player in &players.players {
            let current_happiness = Self::calculate_player_happiness(player);

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
                                    change_type: ChangeType::PersonalConflict,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    /// Generate squad integration events: "settled into squad" or "feeling isolated".
    /// Runs weekly. Also generates "dressing room speech" from team leaders.
    pub(super) fn process_squad_integration(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let sim_date = ctx.simulation.date.date();

        // Collect teammate IDs for relationship lookups
        let teammate_ids: Vec<u32> = players.iter().map(|p| p.id).collect();

        for player in players.iter_mut() {
            // Integration events for recent transfers (first 90 days)
            let is_recent = player.last_transfer_date
                .map(|d| (sim_date - d).num_days() < 90)
                .unwrap_or(false);

            if is_recent {
                // Count positive relationships with current teammates
                let positive_count = teammate_ids.iter()
                    .filter(|&&tid| tid != player.id)
                    .filter(|&&tid| player.relations.get_player(tid)
                        .map(|r| r.level > 20.0).unwrap_or(false))
                    .count();

                let has_any_relation = teammate_ids.iter()
                    .any(|&tid| tid != player.id && player.relations.get_player(tid).is_some());

                // ~10% weekly chance, biased by relationship count.
                // Cooldowns prevent the same player firing back-to-back
                // weekly — settling-in is a slow process; isolation
                // shouldn't tick every fortnight while the player is
                // already adjusting elsewhere.
                if positive_count >= 3 && rand::random::<f32>() < 0.10 {
                    // Long cooldown so a single transfer spell yields at
                    // most one settling-in event — the previous 21-day
                    // window let it fire ~4× per 90-day adaptation period
                    // and read as event-feed spam. Happiness is cleared
                    // on every transfer, so the cooldown effectively
                    // resets at the next club.
                    player.happiness.add_event_with_cooldown(
                        HappinessEventType::SettledIntoSquad,
                        2.0,
                        365,
                    );
                } else if !has_any_relation && rand::random::<f32>() < 0.08 {
                    player.happiness.add_event_with_cooldown(
                        HappinessEventType::FeelingIsolated,
                        -1.5,
                        14,
                    );
                }
            }
        }
    }

    pub(super) fn process_recent_performance_reactions(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        for player in &players.players {
            // Require actual appearances and a notable goal ratio, not just goals > 0.
            // This prevents a single early-season goal from generating boosts all year.
            if player.statistics.played == 0 || !player.position().is_forward() {
                continue;
            }

            let goals_per_game =
                player.statistics.goals as f32 / player.statistics.played as f32;

            if goals_per_game > 0.25 {
                let rep_factor = (player.player_attributes.current_reputation as f32 / 10000.0)
                    .clamp(0.1, 1.0);
                // Scale down for minor-update frequency (~every 2 days vs weekly full update)
                let popularity_boost = (0.03 + 0.04 * rep_factor) * 0.25;

                for other_player in &players.players {
                    if player.id != other_player.id {
                        result
                            .players
                            .relationship_result
                            .push(PlayerRelationshipChangeResult {
                                from_player_id: other_player.id,
                                to_player_id: player.id,
                                relationship_change: popularity_boost,
                                change_type: ChangeType::MatchCooperation,
                            });
                    }
                }
            }
        }
    }
}
