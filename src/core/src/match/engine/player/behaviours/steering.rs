﻿use crate::r#match::position::VectorExtensions;
use crate::r#match::MatchPlayer;
use nalgebra::Vector3;
use std::f32::NAN;

pub enum SteeringBehavior<'p> {
    Seek {
        target: Vector3<f32>,
    },
    Arrive {
        target: Vector3<f32>,
        slowing_distance: f32,
    },
    Pursuit {
        target: &'p MatchPlayer,
    },
    Evade {
        target: &'p MatchPlayer,
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
}

impl<'p> SteeringBehavior<'p> {
    pub fn calculate(&self, player: &MatchPlayer) -> SteeringOutput {
        match self {
            SteeringBehavior::Seek { target } => {
                let desired_velocity = (*target - player.position).normalize();
                let steering = desired_velocity - player.velocity;

                let max_force = 0.3;
                let steering = Self::limit_magnitude(steering, max_force);

                SteeringOutput {
                    velocity: steering,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Arrive {
                target,
                slowing_distance,
            } => {
                let distance = (*target - player.position).length();
                let desired_velocity = (*target - player.position).normalize()
                    * (distance / *slowing_distance * player.skills.max_speed());
                let steering = desired_velocity - player.velocity;
                let max_acceleration = player.skills.max_speed();
                let steering_length = steering.norm();

                // println!(
                //     "max_acceleration = {}, steering_length = {} ",
                //     max_acceleration, steering_length
                // );

                // Limit the steering to the maximum acceleration
                let steering_ratio = max_acceleration / steering_length;
                let mut limited_steering = steering * steering_ratio;

                if limited_steering.x == NAN || limited_steering.x == NAN {
                    limited_steering = Vector3::zeros();
                }

                SteeringOutput {
                    velocity: limited_steering,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Pursuit { target } => {
                let distance = (target.position - player.position).length();
                let prediction = distance / player.skills.max_speed();
                let target_position = target.position + (target.velocity * prediction);
                let desired_velocity =
                    (target_position - player.position).normalize() * player.skills.max_speed();
                let mut steering = desired_velocity - player.velocity;

                if steering.x == NAN || steering.x == NAN {
                    steering = Vector3::zeros();
                }

                SteeringOutput {
                    velocity: steering,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Evade { target } => {
                let distance = (target.position - player.position).length();
                let prediction = distance / player.skills.max_speed();
                let target_position = target.position + target.velocity * prediction;
                let desired_velocity =
                    (player.position - target_position).normalize() * player.skills.max_speed();
                let mut steering = desired_velocity - player.velocity;

                if steering.x == NAN || steering.x == NAN {
                    steering = Vector3::zeros();
                }

                SteeringOutput {
                    velocity: steering,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Wander {
                target,
                radius,
                jitter,
                distance,
                angle: _,
            } => {
                let rand_vec = Vector3::random_in_unit_circle() * *jitter;
                let target = rand_vec + *target;
                let target_offset = target - player.position;
                let mut target_offset = target_offset.normalize() * *distance;
                target_offset = target_offset.add_scalar(player.heading() * *radius);
                let mut steering = target_offset - player.velocity;

                if steering.x == NAN || steering.x == NAN {
                    steering = Vector3::zeros();
                }

                SteeringOutput {
                    velocity: steering,
                    rotation: 0.0,
                }
            }
            SteeringBehavior::Flee { target } => {
                let desired_velocity =
                    (player.position - *target).normalize() * player.skills.max_speed();
                let mut steering = desired_velocity - player.velocity;

                if steering.x == NAN || steering.x == NAN {
                    steering = Vector3::zeros();
                }

                SteeringOutput {
                    velocity: steering,
                    rotation: 0.0,
                }
            }
        }
    }

    fn limit_magnitude(v: Vector3<f32>, max_magnitude: f32) -> Vector3<f32> {
        let current_magnitude = v.norm();
        if current_magnitude > max_magnitude {
            let ratio = max_magnitude / current_magnitude;
            v * ratio
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
