use crate::club::team::behaviour::{PlayerRelationshipChangeResult, TeamBehaviourResult};
use crate::{Player, PlayerCollection, StaffCollection};

pub struct TeamBehaviour;

impl TeamBehaviour {
    pub fn simulate(
        players: &mut PlayerCollection,
        _staffs: &mut StaffCollection,
    ) -> TeamBehaviourResult {
        let mut result = TeamBehaviourResult::new();

        for i in 0..players.players.len() {
            for j in i + 1..players.players.len() {
                let player_i = &players.players[i];
                let player_j = &players.players[j];

                let temperament_i = player_i.attributes.temperament;
                let temperament_j = player_j.attributes.temperament;

                if temperament_i > 0.5 && temperament_j > 0.5 {
                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_i.id,
                            to_player_id: player_j.id,
                            relationship_change: calculate_player_happiness(player_i),
                        });

                    result
                        .players
                        .relationship_result
                        .push(PlayerRelationshipChangeResult {
                            from_player_id: player_j.id,
                            to_player_id: player_i.id,
                            relationship_change: calculate_player_happiness(player_j),
                        });
                }
            }
        }

        result
    }
}

fn calculate_player_happiness(player: &Player) -> f32 {
    let mut happiness = 0.0;

    happiness += player
        .contract
        .as_ref()
        .map(|c| c.salary as f32 / 100.0)
        .unwrap_or(0.0);

    happiness += player.statistics.played as f32 / 20.0;

    happiness += player.statistics.goals as f32 / 10.0;

    happiness += player.attributes.temperament / 100.0;

    happiness = happiness.min(1.0).max(-1.0);

    happiness
}

#[allow(dead_code)]
pub struct PlayerBehaviour {
    pub players: PlayerBehaviourResult,
}

#[allow(dead_code)]
pub struct PlayerBehaviourResult {
    pub players: Vec<Player>,
}
