//! Contract of pressured pass execution (`pass_press01` +
//! `press_transmission`) — the mechanism that makes the `passing`
//! attribute matter at outcome level.
//!
//! Background (2026-08-31 audit): before this mechanism, a 6-vs-18 pin
//! of `passing` across a whole side moved team pass accuracy by 0.4pp
//! and the scoreline by less than the noise band, because unpressured
//! targeting error (≈0.5 m for a poor passer on a 20 m ball) never
//! converts inside the 5 m receiver-claim radius. The press multiplier
//! is where the skill spread now lives, so these pin its shape:
//!
//!   1. Nobody near the passer → the addition is exactly zero (dead
//!      balls and switches in space pay nothing; the pre-change engine
//!      is the zero-pressure special case).
//!   2. Pressure is monotone in proximity, and a second closing body
//!      costs more than one.
//!   3. Composure is the lead resistance skill — the press is beaten in
//!      the head first.
//!
//! All comparative — the gain constant is calibration (see
//! `PASS_PRESS_ERROR_GAIN`), not a unit test's business.

#![cfg(test)]

use crate::PlayerSkills;
use crate::club::player::builder::PlayerBuilder;
use crate::r#match::MatchPlayer;
use crate::r#match::player::events::PlayerEventDispatcher;
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
};
use chrono::NaiveDate;
use nalgebra::Vector3;

fn opponent_at(id: u32, team_id: u32, x: f32, y: f32) -> MatchPlayer {
    let player = PlayerBuilder::new()
        .id(id)
        .full_name(FullName::new("O".to_string(), "P".to_string()))
        .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
        .country_id(1)
        .attributes(PersonAttributes::default())
        .skills(PlayerSkills::default())
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position: PlayerPositionType::MidfielderCenter,
                level: 18,
            }],
        })
        .player_attributes(PlayerAttributes::default())
        .build()
        .unwrap();
    let mut mp = MatchPlayer::from_player(
        team_id,
        &player,
        PlayerPositionType::MidfielderCenter,
        false,
        None,
    );
    mp.position = Vector3::new(x, y, 0.0);
    mp
}

#[test]
fn unpressed_pass_pays_nothing() {
    // Opponents beyond 40u (5 m — the analytics "pressure event"
    // radius) contribute zero pressure, and the error addition is
    // `press01`-scaled, so zero pressure adds exactly nothing — the
    // pre-mechanism engine is the in-space special case, not an
    // approximation of it.
    let origin = Vector3::new(100.0, 100.0, 0.0);
    let players = vec![
        opponent_at(1, 2, 145.0, 100.0),
        opponent_at(2, 2, 100.0, 160.0),
    ];
    let press = PlayerEventDispatcher::pass_press01(&players, 1, origin);
    assert_eq!(press, 0.0, "nobody within 5 m should read as no pressure");
}

#[test]
fn closing_body_degrades_execution_monotonically() {
    let origin = Vector3::new(100.0, 100.0, 0.0);
    let at = |d: f32| vec![opponent_at(1, 2, 100.0 + d, 100.0)];
    let contact = PlayerEventDispatcher::pass_press01(&at(5.0), 1, origin);
    let stride = PlayerEventDispatcher::pass_press01(&at(20.0), 1, origin);
    let edge = PlayerEventDispatcher::pass_press01(&at(38.0), 1, origin);
    assert!(
        contact > stride && stride > edge && edge > 0.0,
        "pressure must fall with distance: {contact} / {stride} / {edge}"
    );
    assert!(
        contact >= 1.0 - 1e-6,
        "a man at contact range (0.6 m) is full pressure, got {contact}"
    );
    // A second body inside the radius costs more than the first alone.
    let two = vec![
        opponent_at(1, 2, 120.0, 100.0),
        opponent_at(2, 2, 100.0, 120.0),
    ];
    let crowded = PlayerEventDispatcher::pass_press01(&two, 1, origin);
    assert!(
        crowded > stride,
        "two closing bodies {crowded} must out-press one {stride}"
    );
    // Teammates are not pressure.
    let mates = vec![opponent_at(1, 1, 105.0, 100.0)];
    assert_eq!(
        PlayerEventDispatcher::pass_press01(&mates, 1, origin),
        0.0,
        "a teammate at contact range is not a presser"
    );
}

#[test]
fn composure_leads_the_resistance() {
    // The composed technician keeps his level under the press, the
    // rattled one transmits it to his feet. Composure must out-weigh
    // either technical resistance at equal delta.
    let rattled = PlayerEventDispatcher::press_transmission(0.2, 0.5, 0.5);
    let composed = PlayerEventDispatcher::press_transmission(0.8, 0.5, 0.5);
    let technical = PlayerEventDispatcher::press_transmission(0.2, 0.8, 0.8);
    assert!(
        composed < rattled,
        "composure must damp the press: {composed} vs {rattled}"
    );
    assert!(
        composed < technical,
        "a +0.6 composure swing must beat +0.3 on BOTH technical skills: {composed} vs {technical}"
    );
    assert!(
        rattled > 0.0 && composed > 0.0,
        "a full press always transmits something: {rattled} / {composed}"
    );
}
