use crate::r#match::{MatchField, MatchPlayer, VectorExtensions};
use log::{debug};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

const MAX_DISTANCE: f32 = 999.0;

#[derive(Debug)]
pub struct PlayerDistanceClosure {
    pub distances: BinaryHeap<PlayerDistanceItem>,
}

#[derive(Debug)]
pub struct PlayerDistanceItem {
    pub player_from_id: u32,
    pub player_from_team: u32,
    pub player_to_id: u32,
    pub player_to_team: u32,
    pub distance: f32,
}

impl From<&MatchField> for PlayerDistanceClosure {
    fn from(field: &MatchField) -> Self {
        let n = field.players.len();
        let capacity = (n * (n - 1)) / 2;

        let mut distances = BinaryHeap::with_capacity(capacity);

        for outer_player in &field.players {
            for inner_player in &field.players {
                if outer_player.id == inner_player.id {
                    continue;
                }

                let distance = outer_player.position.distance_to(&inner_player.position);

                distances.push(PlayerDistanceItem {
                    player_from_id: outer_player.id,
                    player_from_team: outer_player.team_id,
                    player_to_id: inner_player.id,
                    player_to_team: inner_player.team_id,
                    distance,
                });
            }
        }

        PlayerDistanceClosure { distances }
    }
}

impl PlayerDistanceClosure {
    pub fn get(&self, player_from_id: u32, player_to_id: u32) -> f32 {
        if player_from_id == player_to_id {
            debug!(
                "player {} and {} are the same",
                player_from_id, player_to_id
            );
            return 0.0;
        }

        self.distances
            .iter()
            .find(|distance| {
                (distance.player_from_id == player_from_id && distance.player_to_id == player_to_id)
                    || (distance.player_from_id == player_to_id && distance.player_to_id == player_from_id)
            })
            .map(|dist| dist.distance)
            .unwrap_or_else(|| {
                MAX_DISTANCE  // Required for subs
            })
    }

    pub fn get_collisions(&self, max_distance: f32) -> Vec<&PlayerDistanceItem> {
        self.distances
            .iter()
            .filter(|&p| p.distance < max_distance)
            .collect()
    }

    pub fn teammates<'t>(
        &'t self,
        player_id: u32,
        min_distance: f32,
        max_distance: f32,
    ) -> impl Iterator<Item = (u32, f32)> + 't {
        self.distances
            .iter()
            .filter(move |p| p.distance >= min_distance && p.distance <= max_distance)
            .filter_map(move |item| {
                if item.player_from_id == item.player_to_id
                {
                    return None;
                }
                
                if item.player_from_id == player_id && item.player_from_team == item.player_to_team
                {
                    return Some((item.player_to_id, item.distance));
                }

                if item.player_to_id == player_id && item.player_from_team == item.player_to_team {
                    return Some((item.player_from_id, item.distance));
                }

                None
            })
    }

    pub fn opponents<'t>(
        &'t self,
        player_id: u32,
        distance: f32,
    ) -> impl Iterator<Item = (u32, f32)> + 't {
        self.distances
            .iter()
            .filter(move |p| p.distance <= distance)
            .filter_map(move |item| {
                if item.player_from_id == item.player_to_id
                {
                    return None;
                }
                
                if item.player_from_id == player_id && item.player_from_team != item.player_to_team
                {
                    return Some((item.player_to_id, item.distance));
                }

                if item.player_to_id == player_id && item.player_from_team != item.player_to_team {
                    return Some((item.player_from_id, item.distance));
                }

                None
            })
    }
}

impl Eq for PlayerDistanceItem {}

impl PartialEq<PlayerDistanceItem> for PlayerDistanceItem {
    fn eq(&self, other: &Self) -> bool {
        self.player_from_id == other.player_from_id
            && self.player_from_team == other.player_from_team
            && self.player_to_id == other.player_to_id
            && self.player_to_team == other.player_to_team
            && self.distance == other.distance
    }
}

impl PartialOrd<Self> for PlayerDistanceItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PlayerDistanceItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance
            .partial_cmp(&other.distance)
            .unwrap_or(Ordering::Equal)
    }
}
