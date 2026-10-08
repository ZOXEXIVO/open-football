//! A ball in the air is predicted to come down where it really does: the
//! same flight the physics runs, drag and bounce included.

#![cfg(test)]

use crate::r#match::engine::ball::ball::{Ball, CONTROL_DISTANCE, GRAVITY_PER_TICK};
use nalgebra::Vector3;

/// Where the physics first puts a ball within a man's reach: the second
/// time it comes down to the turf, or where it settles after the first.
fn first_playable_contact(ball: &mut Ball) -> Vector3<f32> {
    let mut contacts = 0;
    for _ in 0..1_500 {
        let airborne = ball.position.z > 0.0;
        ball.update_velocity();
        // The bounce at the first contact left nothing in it: it rolls on
        // from where it came down.
        if contacts == 1 && ball.position.z <= 0.0 && ball.velocity.z <= 0.0 {
            break;
        }
        ball.apply_movement();
        if airborne && ball.position.z <= 0.0 {
            contacts += 1;
            if contacts == 2 {
                break;
            }
        }
    }
    Vector3::new(ball.position.x, ball.position.y, 0.0)
}

fn lofted(horizontal: f32, apex: f32) -> Ball {
    let mut ball = Ball::with_coord(840.0, 545.0);
    ball.position = Vector3::new(120.0, 200.0, 0.2);
    ball.velocity = Vector3::new(
        horizontal,
        horizontal * 0.3,
        Ball::launch_speed_for_apex(apex),
    );
    ball.current_owner = None;
    ball
}

#[test]
fn a_long_lofted_ball_comes_down_where_it_was_predicted() {
    for (horizontal, apex) in [(1.8, 8.0), (2.4, 14.0), (3.0, 22.0)] {
        let mut ball = lofted(horizontal, apex);
        let predicted = ball.calculate_landing_position();
        let actual = first_playable_contact(&mut ball);
        let miss = (predicted - actual).norm();
        assert!(
            miss <= CONTROL_DISTANCE,
            "{horizontal} u/tick to {apex} m: predicted {predicted:?}, came down {actual:?} ({miss:.1}u off)"
        );
    }
}

#[test]
fn drag_brings_a_long_ball_in_short_of_the_parabola() {
    let ball = lofted(3.0, 22.0);
    let predicted = ball.calculate_landing_position();
    let vz = ball.velocity.z;
    let parabola_ticks = 2.0 * vz / GRAVITY_PER_TICK;
    let parabola_x = ball.position.x + ball.velocity.x * parabola_ticks;
    assert!(
        predicted.x < parabola_x,
        "{} vs drag-free {}",
        predicted.x,
        parabola_x
    );
}
