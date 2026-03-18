use nalgebra::Vector3;
use crate::r#match::MatchField;

pub struct Space {
    colliders: Vec<SphereCollider>,
}

impl From<&MatchField> for Space {
    fn from(field: &MatchField) -> Self {
        let mut space = Space::new();

        // Add ball collider
        let ball_collider = SphereCollider {
            center: field.ball.position,
            radius: 0.11,
            player_id: None,
        };

        space.add_collider(ball_collider);

        // Add player colliders (no cloning — just store position + id)
        for player in &field.players {
            let player_collider = SphereCollider {
                center: player.position,
                radius: 0.5,
                player_id: Some(player.id),
            };
            space.add_collider(player_collider);
        }

        space
    }
}

impl Space {
    pub fn new() -> Self {
        Space {
            colliders: Vec::with_capacity(30),
        }
    }

    pub fn add_collider(&mut self, collider: SphereCollider) {
        self.colliders.push(collider);
    }

    pub fn update(&mut self, field: &MatchField) {
        self.colliders.clear();
        self.colliders.push(SphereCollider {
            center: field.ball.position,
            radius: 0.11,
            player_id: None,
        });
        for player in &field.players {
            self.colliders.push(SphereCollider {
                center: player.position,
                radius: 0.5,
                player_id: Some(player.id),
            });
        }
    }

    pub fn cast_ray(
        &self,
        origin: Vector3<f32>,
        direction: Vector3<f32>,
        max_distance: f32,
        include_players: bool,
    ) -> Option<RaycastHit<SphereCollider>> {
        let mut closest_hit: Option<RaycastHit<SphereCollider>> = None;
        let mut closest_distance = max_distance;

        for collider in &self.colliders {
            if collider.is_player() && !include_players {
                continue;
            }

            if let Some(intersection) = collider.intersect_ray(origin, direction) {
                let distance = (intersection - origin).magnitude();

                if distance < closest_distance {
                    closest_distance = distance;
                    closest_hit = Some(RaycastHit {
                        collider: collider.clone(),
                        _point: intersection,
                        _normal: collider.normal(intersection),
                        _distance: distance,
                    });
                }
            }
        }

        closest_hit
    }
}

pub struct RaycastHit<T: Collider> {
    pub collider: T,
    _point: Vector3<f32>,
    _normal: Vector3<f32>,
    _distance: f32,
}

pub trait Collider: Clone {
    fn intersect_ray(&self, origin: Vector3<f32>, direction: Vector3<f32>) -> Option<Vector3<f32>>;
    fn normal(&self, point: Vector3<f32>) -> Vector3<f32>;
    fn is_player(&self) -> bool;
}

#[derive(Clone)]
pub struct SphereCollider {
    pub center: Vector3<f32>,
    pub radius: f32,
    pub player_id: Option<u32>,
}

impl Collider for SphereCollider {
    fn intersect_ray(&self, origin: Vector3<f32>, direction: Vector3<f32>) -> Option<Vector3<f32>> {
        let oc = origin - self.center;
        let a = direction.dot(&direction);
        let b = 2.0 * oc.dot(&direction);
        let c = oc.dot(&oc) - self.radius * self.radius;
        let discriminant = b * b - 4.0 * a * c;

        if discriminant < 0.0 {
            None
        } else {
            let t1 = (-b - discriminant.sqrt()) / (2.0 * a);
            let t2 = (-b + discriminant.sqrt()) / (2.0 * a);

            if t1 >= 0.0 && t2 >= 0.0 {
                let t = t1.min(t2);
                Some(origin + t * direction)
            } else if t1 >= 0.0 {
                Some(origin + t1 * direction)
            } else if t2 >= 0.0 {
                Some(origin + t2 * direction)
            } else {
                None
            }
        }
    }

    fn normal(&self, point: Vector3<f32>) -> Vector3<f32> {
        (point - self.center).normalize()
    }

    fn is_player(&self) -> bool {
        self.player_id.is_some()
    }
}
