//! The bodies on the pitch as spheres, for "is there anybody in the way?"
//! — the lane from a passer to a team-mate, or from a shooter to goal.

use crate::r#match::MatchField;
use nalgebra::Vector3;

const MAX_COLLIDERS: usize = 24; // 22 players + 2 spare

/// Half a metre: the width of a man in the way of a ball.
const BODY_RADIUS: f32 = 4.0;

pub struct Space {
    colliders: [SphereCollider; MAX_COLLIDERS],
    len: usize,
}

impl From<&MatchField> for Space {
    fn from(field: &MatchField) -> Self {
        let mut space = Space::new();
        for player in &field.players {
            space.push(SphereCollider {
                center: player.position,
                radius: BODY_RADIUS,
                player_id: player.id,
            });
        }
        space
    }
}

impl Default for Space {
    fn default() -> Self {
        Self::new()
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
            for (collider, player) in self.colliders.iter_mut().zip(field.players.iter()) {
                collider.center = player.position;
            }
        } else {
            *self = Space::from(field);
        }
    }

    /// The nearest body a ray from `origin` meets within `max_distance`,
    /// looking through the players in `see_through` — the man playing the
    /// ball and the man it is for are never in their own way.
    pub fn cast_ray(
        &self,
        origin: Vector3<f32>,
        direction: Vector3<f32>,
        max_distance: f32,
        see_through: &[u32],
    ) -> Option<RaycastHit<SphereCollider>> {
        let mut closest_hit: Option<RaycastHit<SphereCollider>> = None;
        let mut closest_distance = max_distance;

        for collider in &self.colliders[..self.len] {
            if see_through.contains(&collider.player_id) {
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
}

#[derive(Clone, Copy)]
pub struct SphereCollider {
    pub center: Vector3<f32>,
    pub radius: f32,
    pub player_id: u32,
}

impl SphereCollider {
    const EMPTY: Self = SphereCollider {
        center: Vector3::new(0.0, 0.0, 0.0),
        radius: 0.0,
        player_id: 0,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(player_id: u32, x: f32, y: f32) -> SphereCollider {
        SphereCollider {
            center: Vector3::new(x, y, 0.0),
            radius: BODY_RADIUS,
            player_id,
        }
    }

    #[test]
    fn a_defender_on_the_shooting_line_blocks_it_and_one_beside_it_does_not() {
        let shooter = Vector3::new(600.0, 272.0, 0.0);
        let goal = Vector3::new(840.0, 272.0, 0.0);
        let direction = (goal - shooter).normalize();
        let reach = (goal - shooter).norm();

        let mut space = Space::new();
        space.add_collider(body(1, 600.0, 272.0));
        space.add_collider(body(2, 720.0, 274.0));
        let hit = space.cast_ray(shooter, direction, reach, &[1]);
        assert_eq!(hit.map(|h| h.collider.player_id), Some(2));

        let mut space = Space::new();
        space.add_collider(body(1, 600.0, 272.0));
        space.add_collider(body(2, 720.0, 290.0));
        assert!(space.cast_ray(shooter, direction, reach, &[1]).is_none());
    }

    #[test]
    fn the_man_on_the_ball_is_not_in_his_own_way() {
        let passer = Vector3::new(400.0, 272.0, 0.0);
        let mut space = Space::new();
        space.add_collider(body(7, 400.0, 272.0));
        space.add_collider(body(9, 500.0, 272.0));
        let direction = Vector3::new(1.0, 0.0, 0.0);
        assert!(space.cast_ray(passer, direction, 100.0, &[7, 9]).is_none());
        assert!(space.cast_ray(passer, direction, 100.0, &[9]).is_some());
    }
}
