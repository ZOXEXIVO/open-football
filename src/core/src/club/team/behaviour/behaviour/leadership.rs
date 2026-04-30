//! Captain / leadership-driven passes: dressing-room mediation, captain
//! morale propagation, leader-to-teammate influence on relationships,
//! and the playing-time jealousy sweep (which sits here because it's a
//! status-of-the-pecking-order signal).

use super::TeamBehaviour;
use crate::club::team::behaviour::{PlayerRelationshipChangeResult, TeamBehaviourResult};
use crate::{ChangeType, Player, PlayerCollection};

impl TeamBehaviour {
    /// A respected captain mediates dressing-room conflicts. For each
    /// pair of teammates whose relationship has crossed below a friction
    /// threshold, the captain's leadership + professionalism is converted
    /// into a small healing nudge applied to both directions of the
    /// pair. A weak / controversial captain does nothing here.
    ///
    /// Sits alongside the morale-spread captain pass — that one moves
    /// captain mood, this one moves teammate-to-teammate relationships.
    pub(super) fn process_captain_mediation(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
        // Reuse the captain identification logic — highest leadership
        // among 10+ leadership players, weighted by reputation.
        let captain = players
            .players
            .iter()
            .filter(|p| p.skills.mental.leadership >= 12.0)
            .filter(|p| p.attributes.professionalism >= 13.0)
            .max_by(|a, b| {
                let sa = a.skills.mental.leadership * 1.0
                    + a.attributes.professionalism * 0.6
                    + a.attributes.loyalty * 0.4;
                let sb = b.skills.mental.leadership * 1.0
                    + b.attributes.professionalism * 0.6
                    + b.attributes.loyalty * 0.4;
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            });

        let Some(captain) = captain else { return };
        let captain_id = captain.id;

        // Mediation strength: 0..1, scales the per-pair healing nudge.
        let leadership = captain.skills.mental.leadership;
        let prof = captain.attributes.professionalism;
        let temperament = captain.attributes.temperament;
        let strength =
            ((leadership / 20.0) * 0.5 + (prof / 20.0) * 0.3 + (temperament / 20.0) * 0.2)
                .clamp(0.0, 1.0);

        if strength < 0.4 {
            return;
        }

        // Find broken pairs (relationship level <= -25) and emit a
        // small symmetric positive nudge. Cap to a few pairs per week
        // so a single captain doesn't carpet-bomb the whole squad.
        const MAX_MEDIATIONS: usize = 4;
        let mut emitted = 0;
        'outer: for i in 0..players.players.len() {
            let a = &players.players[i];
            if a.id == captain_id {
                continue;
            }
            for j in (i + 1)..players.players.len() {
                if emitted >= MAX_MEDIATIONS {
                    break 'outer;
                }
                let b = &players.players[j];
                if b.id == captain_id {
                    continue;
                }
                let level_ab = a.relations.get_player(b.id).map(|r| r.level).unwrap_or(0.0);
                if level_ab > -25.0 {
                    continue;
                }
                // Mediation effectiveness depends on how each party
                // feels about the captain. If either of them dislikes
                // the captain (relation level <= -20), the intervention
                // lands with half the force; if both already respect
                // the captain (level >= 30) it lands a quarter harder.
                let a_to_cap = a
                    .relations
                    .get_player(captain_id)
                    .map(|r| r.level)
                    .unwrap_or(0.0);
                let b_to_cap = b
                    .relations
                    .get_player(captain_id)
                    .map(|r| r.level)
                    .unwrap_or(0.0);
                let captain_relation_mult = if a_to_cap <= -20.0 || b_to_cap <= -20.0 {
                    0.5
                } else if a_to_cap >= 30.0 && b_to_cap >= 30.0 {
                    1.25
                } else {
                    1.0
                };
                // Healing nudge sized by mediation strength and how
                // bad the relationship is (worse → more visible
                // intervention, but still small per week).
                let intensity = ((-level_ab - 25.0) / 75.0).clamp(0.0, 1.0);
                let nudge =
                    ((strength * 0.4 + intensity * 0.2) * captain_relation_mult).clamp(0.0, 0.6);
                if nudge < 0.05 {
                    continue;
                }
                result
                    .players
                    .relationship_result
                    .push(PlayerRelationshipChangeResult {
                        from_player_id: a.id,
                        to_player_id: b.id,
                        relationship_change: nudge,
                        change_type: ChangeType::PersonalSupport,
                    });
                result
                    .players
                    .relationship_result
                    .push(PlayerRelationshipChangeResult {
                        from_player_id: b.id,
                        to_player_id: a.id,
                        relationship_change: nudge,
                        change_type: ChangeType::PersonalSupport,
                    });
                emitted += 1;
            }
        }
    }

    /// Captain = highest `leadership + influence` on the squad. Their
    /// mood leaks out to teammates: ~±2 morale points/week based on how
    /// happy the captain is relative to neutral 50. Sits on top of the
    /// existing `process_leadership_influence` pass (which only moves
    /// relationship numbers, not morale).
    pub(super) fn process_captain_morale_propagation(players: &mut PlayerCollection) {
        // Pick the captain by compound score. Don't fall back to anyone
        // with <10 leadership — a weak captain shouldn't propagate.
        let captain_id_opt = players
            .players
            .iter()
            .filter(|p| p.skills.mental.leadership >= 10.0)
            .max_by(|a, b| {
                let sa = a.skills.mental.leadership * 1.0
                    + a.attributes.loyalty * 0.5
                    + a.player_attributes.current_reputation as f32 / 2000.0;
                let sb = b.skills.mental.leadership * 1.0
                    + b.attributes.loyalty * 0.5
                    + b.player_attributes.current_reputation as f32 / 2000.0;
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.id);

        let captain_id = match captain_id_opt {
            Some(id) => id,
            None => return,
        };

        let captain_morale = match players.find(captain_id) {
            Some(c) => c.happiness.morale,
            None => return,
        };

        // Delta: captain at 50 morale → 0 effect. At 80 → +0.6, at 20 → -0.6.
        // Leadership scales the magnitude (a 20-leadership captain hits 2x
        // a 10-leadership captain).
        let captain_leadership = players
            .find(captain_id)
            .map(|c| c.skills.mental.leadership)
            .unwrap_or(10.0);
        let leadership_scale = (captain_leadership / 20.0).clamp(0.0, 1.0);
        let base_delta = (captain_morale - 50.0) * 0.02; // -1..1
        let delta = base_delta * leadership_scale; // -1..1 scaled

        if delta.abs() < 0.05 {
            return;
        }

        for player in players.players.iter_mut() {
            if player.id == captain_id {
                continue;
            }
            // Per-teammate sway — a captain someone respects lands harder,
            // someone the squad mistrusts lands weaker (or slightly
            // negative on a good-mood captain). Maps level [-100,100] →
            // multiplier [0.3, 1.5].
            let relation_mult = player
                .relations
                .get_player(captain_id)
                .map(|r| 0.9 + (r.level / 100.0).clamp(-0.6, 0.6))
                .unwrap_or(1.0);
            player.happiness.adjust_morale(delta * relation_mult);
        }
    }

    /// High leadership players influence team morale and relationships
    pub(super) fn process_leadership_influence(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
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
                            change_type: ChangeType::MentorshipBond,
                        });
                }
            }
        }
    }

    /// Playing time jealousy
    pub(super) fn process_playing_time_jealousy(
        players: &PlayerCollection,
        result: &mut TeamBehaviourResult,
    ) {
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
                    let change_type = if jealousy_factor > 0.0 {
                        ChangeType::TrainingBonding
                    } else {
                        ChangeType::CompetitionRivalry
                    };

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: jealousy_factor,
                            change_type: change_type.clone(),
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: jealousy_factor,
                            change_type,
                        });
                }
            }
        }
    }
}
