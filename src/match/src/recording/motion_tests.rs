use super::replay::*;
use bevy::prelude::Vec3;

fn track(rows: &[(u32, f32, f32)]) -> Track {
    let mut track = Track::default();
    track.merge(
        rows.iter()
            .map(|&(t, x, y)| Sample { t, x, y, z: 0.0 })
            .collect(),
    );
    track
}

#[test]
fn local_motion_averages_quantization_without_touching_positions() {
    let rows: Vec<_> = (0..=30)
        .map(|i| (i * 30, (i as f32 * 0.72 * 10.0).round() / 10.0, 0.0))
        .collect();
    let mut track = track(&rows);
    for time in [200.0, 216.0, 230.0, 250.0, 270.0, 280.0] {
        let before = track.position_at(time);
        let cue = track.motion_at(time, 0.0, 900.0);
        assert!((cue.velocity.x - 3.0).abs() < 0.08, "{:?}", cue.velocity);
        assert_eq!(before, track.position_at(time));
        assert_eq!(before, track.position_ahead(time));
    }
}

#[test]
fn local_motion_does_not_anticipate_a_step_after_a_heartbeat() {
    let track = track(&[(0, 0.0, 0.0), (600, 1.6, 0.0), (630, 3.2, 0.0)]);
    assert_eq!(track.motion_at(300.0, 0.0, 900.0).velocity, Vec3::ZERO);
    assert_eq!(track.motion_at(550.0, 0.0, 900.0).velocity, Vec3::ZERO);
    assert!(track.motion_at(590.0, 0.0, 900.0).velocity.x > 0.0);
}

#[test]
fn local_motion_keeps_a_sparse_walk_steady() {
    let track = track(&[
        (0, 0.0, 0.0),
        (150, 0.3, 0.0),
        (300, 0.6, 0.0),
        (450, 0.9, 0.0),
    ]);
    assert!((track.motion_at(200.0, 0.0, 450.0).velocity.x - 0.25).abs() < 1e-5);
}

#[test]
fn local_motion_never_averages_across_a_placement() {
    let track = track(&[
        (0, 0.0, 0.0),
        (30, 0.72, 0.0),
        (60, 500.0, 0.0),
        (90, 500.72, 0.0),
        (120, 501.44, 0.0),
    ]);
    let cue = track.motion_at(45.0, 0.0, 120.0);
    assert!(cue.placed);
    assert_eq!(cue.velocity, Vec3::ZERO);
    assert!((track.motion_at(75.0, 0.0, 120.0).velocity.x - 3.0).abs() < 0.001);
}

#[test]
fn local_motion_is_one_sided_at_stream_and_clip_edges() {
    let track = track(&[
        (0, 0.0, 0.0),
        (30, 0.72, 0.0),
        (60, 1.44, 0.0),
        (90, 1.44, 0.72),
    ]);
    assert!((track.motion_at(5.0, 0.0, 60.0).velocity.x - 3.0).abs() < 1e-5);
    let edge = track.motion_at(55.0, 0.0, 60.0);
    assert!((edge.velocity.x - 3.0).abs() < 1e-5);
    assert_eq!(edge.velocity.z, 0.0);
    assert_eq!(track.motion_at(120.0, 0.0, 150.0).velocity, Vec3::ZERO);
    assert_eq!(track.motion_at(5.0, 10.0, 60.0).velocity, Vec3::ZERO);
}

#[test]
fn local_motion_handles_empty_and_single_sample_tracks() {
    assert_eq!(
        Track::default().motion_at(0.0, 0.0, 100.0).velocity,
        Vec3::ZERO
    );
    assert_eq!(
        track(&[(0, 0.0, 0.0)]).motion_at(0.0, 0.0, 100.0).velocity,
        Vec3::ZERO
    );
}

#[test]
fn local_motion_detects_placement_hidden_by_fast_playback_average() {
    let track = track(&[
        (0, 0.0, 0.0),
        (30, 0.0, 0.0),
        (60, 40.0, 0.0),
        (90, 40.0, 0.0),
        (1000, 40.0, 0.0),
    ]);
    assert!(track.placed_between(0.0, 1000.0));
    assert!(!track.placed_between(60.0, 1000.0));
    assert!(!track.placed_between(0.0, 30.0));
}
