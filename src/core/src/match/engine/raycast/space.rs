use crate::r#match::MatchField;
use nalgebra::Vector3;

const MAX_COLLIDERS: usize = 24; // 1 ball + 22 players + 1 spare

pub struct Space {
    colliders: [SphereCollider; MAX_COLLIDERS],
    len: usize,
}

impl From<&MatchField> for Space {
    fn from(field: &MatchField) -> Self {
        let mut space = Space::new();

        // Add ball collider
        space.push(SphereCollider {
            center: field.ball.position,
            radius: 0.11,
            player_id: None,
        });

        // Add player colliders
        for player in &field.players {
            space.push(SphereCollider {
                center: player.position,
                radius: 0.5,
                player_id: Some(player.id),
            });
        }

        space
    }
}

impl Space {
    pub fn new() -> Self {
        Space {
            colliders: [SphereCollider::EMPTY; MAX_COLLIDERS],
            len: 0,
        }
    }

    #[inline]
    fn push(&mut self, collider: SphereCollider) {
        debug_assert!(self.len < MAX_COLLIDERS);
        self.colliders[self.len] = collider;
        self.len += 1;
    }

    pub fn add_collider(&mut self, collider: SphereCollider) {
        self.push(collider);
    }

    pub fn update(&mut self, field: &MatchField) {
        // Update positions in-place — structure (len, radii, player_ids) doesn't change
        if self.len > 0 {
            self.colliders[0].center = field.ball.position;
            for (i, player) in field.players.iter().enumerate() {
                self.colliders[i + 1].center = player.position;
            }
        } else {
            // First call or after reset — full rebuild
            self.push(SphereCollider {
                center: field.ball.position,
                radius: 0.11,
                player_id: None,
            });
            for player in &field.players {
                self.push(SphereCollider {
                    center: player.position,
                    radius: 0.5,
                    player_id: Some(player.id),
                });
            }
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

        for i in 0..self.len {
            let collider = &self.colliders[i];

            if collider.is_player() && !include_players {
                continue;
            }

            if let Some(intersection) = collider.intersect_ray(origin, direction) {
                let distance = (intersection - origin).magnitude();

                if distance < closest_distance {
                    closest_distance = distance;
                    closest_hit = Some(RaycastHit {
                        collider: *collider,
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

pub trait Collider: Copy {
    fn intersect_ray(&self, origin: Vector3<f32>, direction: Vector3<f32>) -> Option<Vector3<f32>>;
    fn normal(&self, point: Vector3<f32>) -> Vector3<f32>;
    fn is_player(&self) -> bool;
}

#[derive(Clone, Copy)]
pub struct SphereCollider {
    pub center: Vector3<f32>,
    pub radius: f32,
    pub player_id: Option<u32>,
}

impl SphereCollider {
    const EMPTY: Self = SphereCollider {
        center: Vector3::new(0.0, 0.0, 0.0),
        radius: 0.0,
        player_id: None,
    };
}

impl Collider for SphereCollider {
    #[inline]
    fn intersect_ray(&self, origin: Vector3<f32>, direction: Vector3<f32>) -> Option<Vector3<f32>> {
        let oc = origin - self.center;
        let a = direction.dot(&direction);
        let b = 2.0 * oc.dot(&direction);
        let c = oc.dot(&oc) - self.radius * self.radius;
        let discriminant = b * b - 4.0 * a * c;

        if discriminant < 0.0 {
            None
        } else {
            let sqrt_disc = discriminant.sqrt();
            let inv_2a = 1.0 / (2.0 * a);
            let t1 = (-b - sqrt_disc) * inv_2a;
            let t2 = (-b + sqrt_disc) * inv_2a;

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

    #[inline]
    fn normal(&self, point: Vector3<f32>) -> Vector3<f32> {
        (point - self.center).normalize()
    }

    #[inline]
    fn is_player(&self) -> bool {
        self.player_id.is_some()
    }
}
