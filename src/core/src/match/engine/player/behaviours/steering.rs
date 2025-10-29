use crate::r#match::MatchPlayer;
use nalgebra::Vector3;

pub enum SteeringBehavior {
    Seek {
        target: Vector3<f32>,
    },
    Arrive {
        target: Vector3<f32>,
        slowing_distance: f32,
    },
    Pursuit {
        target: Vector3<f32>
    },
    Evade {
        target: Vector3<f32>
    },
    Wander {
        target: Vector3<f32>,
        radius: f32,
        jitter: f32,
        distance: f32,
        angle: f32,
    },
    Flee {
        target: Vector3<f32>,
    },
    FollowPath {
        waypoints: Vec<Vector3<f32>>,
        current_waypoint: usize,
        path_offset: f32,
    },
}

impl SteeringBehavior {
    pub fn calculate(&self, player: &MatchPlayer) -> SteeringOutput {
        match self {
            SteeringBehavior::Seek { target } => {
                let to_target = *target - player.position;
                let desired_velocity = if to_target.norm() > 0.0 {
                    to_target.normalize() * player.skills.max_speed()
                } else {
                    Vector3::zeros()
                };

                let steering = desired_velocity - player.velocity;

                let max_force = player.skills.physical.acceleration / 20.0;
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to get new absolute velocity
                let new_velocity = player.velocity + steering;
                let final_velocity = Self::limit_magnitude(new_velocity, player.skills.max_speed());

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Arrive {
                target,
                slowing_distance,
            } => {
                let to_target = *target - player.position;
                let distance = to_target.norm();

                // Stop if very close to target
                const ARRIVAL_DEADZONE: f32 = 0.5;
                if distance < ARRIVAL_DEADZONE {
                    // Apply gentle braking
                    let braking_force = -player.velocity * 0.3;
                    let new_velocity = player.velocity + braking_force;
                    return SteeringOutput {
                        velocity: new_velocity,
                        rotation: 0.0,
                    };
                }

                // Normalize skill values
                let pace_normalized = 0.8 + (player.skills.physical.pace - 1.0) / 19.0;
                let agility_normalized = 0.8 + (player.skills.physical.agility - 1.0) / 19.0;

                // Ensure slowing_distance is never zero
                let safe_slowing_distance = slowing_distance.max(5.0);

                // Calculate desired speed based on distance
                let max_speed = player.skills.max_speed() * pace_normalized;
                let desired_speed = if distance < safe_slowing_distance {
                    // Smooth quadratic deceleration as we approach target
                    let ratio = (distance / safe_slowing_distance).clamp(0.0, 1.0);
                    max_speed * ratio * ratio
                } else {
                    max_speed
                };

                // Calculate desired velocity direction
                let desired_velocity = if distance > 0.0 {
                    to_target.normalize() * desired_speed
                } else {
                    Vector3::zeros()
                };

                // Calculate steering force (the change in velocity needed)
                let steering = desired_velocity - player.velocity;

                // Limit steering force based on agility
                let max_force = player.skills.max_speed() * agility_normalized * 0.7;
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to current velocity to get new absolute velocity
                let new_velocity = player.velocity + steering;

                // Clamp to max speed
                let final_velocity = Self::limit_magnitude(new_velocity, max_speed);

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Pursuit { target } => {
                let to_target = *target - player.position;
                let distance = to_target.norm();

                // Deadzone to prevent oscillation when very close
                const PURSUIT_DEADZONE: f32 = 1.5;
                const SLOWING_DISTANCE: f32 = 10.0;

                if distance < PURSUIT_DEADZONE {
                    // Very close to target - apply strong braking
                    let braking_force = -player.velocity * 0.9;
                    let new_velocity = player.velocity + braking_force;
                    return SteeringOutput {
                        velocity: new_velocity,
                        rotation: 0.0,
                    };
                }

                // Normalize skill values to a range of 0.5 to 1.5
                let acceleration_normalized = 0.9 + (player.skills.physical.acceleration - 1.0) / 19.0;
                let pace_normalized = 0.8 + (player.skills.physical.pace - 1.0) / 19.0;
                let agility_normalized = 0.8 + (player.skills.physical.agility - 1.0) / 19.0;

                let max_speed = player.skills.max_speed() * pace_normalized * acceleration_normalized;

                // Calculate desired speed based on distance - slow down when approaching
                let desired_speed = if distance < SLOWING_DISTANCE {
                    // Within slowing distance - reduce speed proportionally
                    let speed_ratio = (distance / SLOWING_DISTANCE).clamp(0.2, 1.0);
                    max_speed * speed_ratio
                } else {
                    // Full speed when far away
                    max_speed
                };

                let desired_velocity = if distance > 0.0 {
                    to_target.normalize() * desired_speed
                } else {
                    Vector3::zeros()
                };

                // Use direct velocity blending when close to prevent oscillation
                let final_velocity = if distance < SLOWING_DISTANCE {
                    // Close to target - blend toward desired velocity to prevent overshoot
                    let blend_factor = (distance / SLOWING_DISTANCE).clamp(0.0, 1.0);
                    let damping = 0.7 - (blend_factor * 0.3); // More damping when closer

                    desired_velocity * (1.0 - damping) + player.velocity * damping
                } else {
                    // Far from target - use normal steering accumulation
                    let steering = desired_velocity - player.velocity;
                    let max_acceleration = player.skills.max_speed() * agility_normalized * acceleration_normalized;
                    let limited_steering = Self::limit_magnitude(steering, max_acceleration);

                    let move_velocity = player.velocity + limited_steering;
                    Self::limit_magnitude(move_velocity, max_speed)
                };

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Evade { target } => {
                let to_player = player.position - *target;

                let desired_velocity = if to_player.norm() > 0.0 {
                    to_player.normalize() * player.skills.max_speed()
                } else {
                    Vector3::zeros()
                };

                let steering = desired_velocity - player.velocity;

                // Limit the steering force like other behaviors
                let max_force = player.skills.physical.acceleration / 20.0;
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to get new absolute velocity
                let new_velocity = player.velocity + steering;
                let final_velocity = Self::limit_magnitude(new_velocity, player.skills.max_speed());

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Wander {
                target: _,
                radius,
                jitter: _,
                distance,
                angle
            } => {
                // The wander circle is projected in front of the player
                let circle_center = player.position + player.velocity.normalize() * *distance;

                // Calculate the displacement around the circle using the stored angle
                let displacement = Vector3::new(
                    angle.cos() * *radius,
                    angle.sin() * *radius,
                    0.0,
                );

                // The wander target is on the circle's edge
                let wander_target = circle_center + displacement;

                // Calculate desired velocity toward the wander target
                let to_target = wander_target - player.position;
                let desired_velocity = if to_target.norm() > 0.0 {
                    to_target.normalize() * player.skills.max_speed() * 0.3 // Reduced speed for wandering
                } else {
                    Vector3::zeros()
                };

                let steering = desired_velocity - player.velocity;

                // Limit steering force
                let max_force = player.skills.physical.acceleration / 30.0; // Gentler force
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to get new absolute velocity
                let new_velocity = player.velocity + steering;
                let wander_max_speed = player.skills.max_speed() * 0.3; // Wandering is slower
                let final_velocity = Self::limit_magnitude(new_velocity, wander_max_speed);

                let rotation = if final_velocity.x != 0.0 || final_velocity.y != 0.0 {
                    final_velocity.y.atan2(final_velocity.x)
                } else {
                    0.0
                };

                SteeringOutput {
                    velocity: final_velocity,
                    rotation,
                }
            }
            SteeringBehavior::Flee { target } => {
                let to_player = player.position - *target;
                let desired_velocity = if to_player.norm() > 0.0 {
                    to_player.normalize() * player.skills.max_speed()
                } else {
                    Vector3::zeros()
                };

                let steering = desired_velocity - player.velocity;
                let max_force = player.skills.physical.acceleration / 20.0;
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to get new absolute velocity
                let new_velocity = player.velocity + steering;
                let final_velocity = Self::limit_magnitude(new_velocity, player.skills.max_speed());

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }

            SteeringBehavior::FollowPath { waypoints, current_waypoint, path_offset } => {
                if waypoints.is_empty() {
                    return SteeringOutput {
                        velocity: Vector3::zeros(),
                        rotation: 0.0,
                    };
                }

                // Get the current target waypoint
                if *current_waypoint >= waypoints.len() {
                    return SteeringOutput {
                        velocity: Vector3::zeros(),
                        rotation: 0.0,
                    };
                }

                let target = waypoints[*current_waypoint];

                // Calculate distance to current waypoint
                let to_waypoint = target - player.position;
                let distance = to_waypoint.norm();

                // Calculate desired velocity toward waypoint with slight offset for natural movement
                let direction = if distance > 0.0 {
                    to_waypoint.normalize()
                } else {
                    Vector3::zeros()
                };

                // Apply slight offset if specified (makes movement more natural)
                let offset_direction = if *path_offset > 0.0 {
                    // Create a perpendicular vector for offset
                    Vector3::new(-direction.y, direction.x, 0.0) * *path_offset
                } else {
                    Vector3::zeros()
                };

                let desired_velocity = (direction + offset_direction.normalize() * 0.1) * player.skills.max_speed();
                let steering = desired_velocity - player.velocity;

                // Limit steering force
                let max_force = player.skills.physical.acceleration / 20.0;
                let steering = Self::limit_magnitude(steering, max_force);

                // Apply steering force to get new absolute velocity
                let new_velocity = player.velocity + steering;
                let final_velocity = Self::limit_magnitude(new_velocity, player.skills.max_speed());

                SteeringOutput {
                    velocity: final_velocity,
                    rotation: 0.0,
                }
            }
        }
    }

    fn limit_magnitude(v: Vector3<f32>, max_magnitude: f32) -> Vector3<f32> {
        let current_magnitude = v.norm();
        if current_magnitude > max_magnitude && current_magnitude > 0.0 {
            v * (max_magnitude / current_magnitude)
        } else {
            v
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SteeringOutput {
    pub velocity: Vector3<f32>,
    pub rotation: f32,
}

impl SteeringOutput {
    pub fn new(velocity: Vector3<f32>, rotation: f32) -> Self {
        SteeringOutput { velocity, rotation }
    }
}
