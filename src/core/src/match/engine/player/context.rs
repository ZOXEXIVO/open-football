use crate::r#match::position::{PlayerFieldPosition, VectorExtensions};
use crate::r#match::{BallSide, MatchField, MatchPlayer, Space, SphereCollider};
use nalgebra::Vector3;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub struct GameTickContext {
    pub object_positions: MatchObjectsPositions,
    pub ball: BallMetadata,
    pub space: Space<SphereCollider>,
}

impl GameTickContext {
    pub fn new(field: &MatchField) -> Self {
        let mut space = Space::new();

        // Add ball collider
        let ball_radius = 0.11; // Assuming the ball radius is 0.11 meters (size 5 football)
        let ball_collider = SphereCollider {
            center: field.ball.position,
            radius: ball_radius,
            player: None,
        };
        space.add_collider(ball_collider);

        // Add player colliders
        for player in &field.players {
            let player_radius = 0.5; // Assuming the player radius is 0.5 meters
            let player_collider = SphereCollider {
                center: player.position,
                radius: player_radius,
                player: Some(player.clone()),
            };
            space.add_collider(player_collider);
        }

        GameTickContext {
            ball: BallMetadata::from_field(field),
            object_positions: MatchObjectsPositions::from(field),
            space,
        }
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum PlayerDistanceFromStartPosition {
    Small,
    Medium,
    Big,
}

pub struct MatchObjectsPositions {
    pub ball_position: Vector3<f32>,
    pub ball_velocity: Vector3<f32>,
    pub players_positions: PlayerPositionsClosure,
    pub player_distances: PlayerDistanceClosure,
}

impl MatchObjectsPositions {
    pub fn from(field: &MatchField) -> Self {
        let positions: Vec<PlayerFieldPosition> = field
            .players
            .iter()
            .map(|p| PlayerFieldPosition {
                player_id: p.id,
                side: p.side.unwrap(),
                position: p.position,
            })
            .collect();

        // fill distances

        let mut distances = PlayerDistanceClosure::new();

        field
            .players
            .iter()
            .enumerate()
            .for_each(|(i, outer_player)| {
                field.players.iter().skip(i + 1).for_each(|inner_player| {
                    let distance = outer_player.position.distance_to(&inner_player.position);

                    distances.add(
                        outer_player.id,
                        outer_player.team_id,
                        outer_player.position,
                        inner_player.id,
                        inner_player.team_id,
                        inner_player.position,
                        distance,
                    );

                    distances.add(
                        inner_player.id,
                        inner_player.team_id,
                        inner_player.position,
                        outer_player.id,
                        outer_player.team_id,
                        outer_player.position,
                        distance,
                    );
                });
            });

        MatchObjectsPositions {
            ball_position: field.ball.position,
            ball_velocity: field.ball.velocity,
            players_positions: PlayerPositionsClosure::new(positions),
            player_distances: distances,
        }
    }
}

pub struct BallMetadata {
    pub side: BallSide,
    pub is_owned: bool,
    pub current_owner: Option<u32>,
    pub last_owner: Option<u32>,
}

impl BallMetadata {
    pub fn from_field(field: &MatchField) -> Self {
        BallMetadata {
            side: Self::calculate_side(field),
            is_owned: field.ball.current_owner.is_some(),
            current_owner: field.ball.current_owner,
            last_owner: field.ball.previous_owner,
        }
    }

    fn calculate_side(field: &MatchField) -> BallSide {
        if field.ball.position.x < field.ball.center_field_position {
            return BallSide::Left;
        }

        BallSide::Right
    }
}

pub struct PlayerPositionsClosure {
    pub items: Vec<PlayerFieldPosition>,
}

impl PlayerPositionsClosure {
    pub fn new(players_positions: Vec<PlayerFieldPosition>) -> Self {
        PlayerPositionsClosure {
            items: players_positions,
        }
    }

    pub fn get_player_position(&self, player_id: u32) -> Option<Vector3<f32>> {
        self.items
            .iter()
            .find(|p| p.player_id == player_id)
            .map(|p| p.position)
    }
}

pub struct PlayerDistanceClosure {
    pub distances: BinaryHeap<PlayerDistanceItem>,
}

pub struct PlayerDistanceItem {
    pub player_from_id: u32,
    pub player_from_team: u32,
    pub player_from_position: Vector3<f32>,

    pub player_to_id: u32,
    pub player_to_team: u32,
    pub player_to_position: Vector3<f32>,

    pub distance: f32,
}

impl PlayerDistanceClosure {
    pub fn new() -> Self {
        PlayerDistanceClosure {
            distances: BinaryHeap::with_capacity(50),
        }
    }

    pub fn add(
        &mut self,
        player_from_id: u32,
        player_from_team: u32,
        player_from_position: Vector3<f32>,
        player_to_id: u32,
        player_to_team: u32,
        player_to_position: Vector3<f32>,
        distance: f32,
    ) {
        self.distances.push(PlayerDistanceItem {
            player_from_id,
            player_from_team,
            player_from_position,
            player_to_id,
            player_to_team,
            player_to_position,
            distance,
        })
    }

    pub fn get(&self, player_from_id: u32, player_to_id: u32) -> Option<f32> {
        self.distances
            .iter()
            .find(|distance| {
                (distance.player_from_id == player_from_id && distance.player_to_id == player_to_id)
                    || (distance.player_from_id == player_to_id
                        && distance.player_to_id == player_from_id)
            })
            .map(|dist| dist.distance)
    }

    pub fn get_collisions(&self, max_distance: f32) -> Vec<&PlayerDistanceItem> {
        self.distances
            .iter()
            .filter(|&p| p.distance < max_distance)
            .collect()
    }

    pub fn teammates<'t>(&'t self, player: &'t MatchPlayer, distance: f32) -> impl Iterator<Item = (u32, f32)> + 't {
        self.distances
            .iter()
            .filter(move |p| p.distance <= distance)
            .filter(|item| {
                item.player_from_id == player.id
                    && item.player_from_team == item.player_to_team
                    && item.player_from_id != item.player_to_id
            })
            .map(|item| (item.player_to_id, item.distance))
    }

    pub fn opponents<'t>(&'t self, player: &'t MatchPlayer, distance: f32) -> impl Iterator<Item = (u32, f32)> + 't {
        self.distances
            .iter()
            .filter(move |p| p.distance <= distance)
            .filter(|item| {
                item.player_from_id == player.id
                    && item.player_from_team != item.player_to_team
                    && item.player_from_id != item.player_to_id
            })
            .map(|item| (item.player_to_id, item.distance))
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
