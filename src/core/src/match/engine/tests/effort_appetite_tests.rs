//! Contract of the off-ball effort appetite
//! (`MovementEffort::effort_appetite`) — the hosts that make
//! `work_rate` and `determination` matter at outcome level.
//!
//! Background (2026-08-31 sweep): both attributes pinned 6:18 across a
//! whole side measured null (+0.28 / +0.13 goal differential) because
//! every prior host was either a relative election among teammates
//! (side-wide pins cancel there), a low-volume gate, or a diluted
//! weight. The appetite multiplies the effort ceiling at the one point
//! every off-ball player's speed passes through, so these pin its
//! shape:
//!
//!   1. `work_rate` scales sub-maximal movement all match and leaves
//!      `VeryHigh` alone — urgency is universal.
//!   2. `determination` fires only when the game asks the question:
//!      from 70' always, from 55' when trailing. Early and level it is
//!      silent.
//!   3. Both bands are population-centred: an ordinary player (~11.8)
//!      multiplies by ~1.0, so the calibrated ground-covered and
//!      chance-supply numbers do not move underneath the mechanism.
//!
//! All comparative — exact band constants are calibration.

#![cfg(test)]

use crate::PlayerSkills;
use crate::club::player::builder::PlayerBuilder;
use crate::r#match::ActivityIntensity;
use crate::r#match::MatchPlayer;
use crate::r#match::MovementEffort;
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
};
use chrono::NaiveDate;

fn build_mover(work_rate: f32, determination: f32) -> MatchPlayer {
    let mut skills = PlayerSkills::default();
    skills.mental.work_rate = work_rate;
    skills.mental.determination = determination;
    let player = PlayerBuilder::new()
        .id(1)
        .full_name(FullName::new("W".to_string(), "R".to_string()))
        .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
        .country_id(1)
        .attributes(PersonAttributes::default())
        .skills(skills)
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position: PlayerPositionType::MidfielderCenter,
                level: 18,
            }],
        })
        .player_attributes(PlayerAttributes::default())
        .build()
        .unwrap();
    MatchPlayer::from_player(
        1,
        &player,
        PlayerPositionType::MidfielderCenter,
        false,
        None,
    )
}

#[test]
fn work_rate_scales_the_amble_not_the_sprint() {
    let willing = build_mover(18.0, 12.0);
    let idle = build_mover(6.0, 12.0);
    // Sub-maximal: the willing runner uses visibly more of the band.
    let w = MovementEffort::effort_appetite(&willing, ActivityIntensity::Moderate, 30, false);
    let i = MovementEffort::effort_appetite(&idle, ActivityIntensity::Moderate, 30, false);
    assert!(
        w > i + 0.08,
        "work_rate 18 should out-move 6 on a jog: {w} vs {i}"
    );
    // Explosive: urgency is universal — the attribute pays nothing.
    let w_hi = MovementEffort::effort_appetite(&willing, ActivityIntensity::VeryHigh, 30, false);
    let i_hi = MovementEffort::effort_appetite(&idle, ActivityIntensity::VeryHigh, 30, false);
    assert_eq!(
        w_hi, i_hi,
        "a sprint is a sprint whatever the work rate: {w_hi} vs {i_hi}"
    );
}

#[test]
fn determination_answers_only_when_the_game_asks() {
    let iron = build_mover(12.0, 18.0);
    let soft = build_mover(12.0, 6.0);
    // Minute 30, level: silent — same appetite.
    let early_iron = MovementEffort::effort_appetite(&iron, ActivityIntensity::High, 30, false);
    let early_soft = MovementEffort::effort_appetite(&soft, ActivityIntensity::High, 30, false);
    assert_eq!(
        early_iron, early_soft,
        "determination must not act in a level first half"
    );
    // Minute 80: character decides who keeps moving.
    let late_iron = MovementEffort::effort_appetite(&iron, ActivityIntensity::High, 80, false);
    let late_soft = MovementEffort::effort_appetite(&soft, ActivityIntensity::High, 80, false);
    assert!(
        late_iron > late_soft + 0.05,
        "determination 18 should hold effort late: {late_iron} vs {late_soft}"
    );
    // Minute 60, trailing: the chase window opens early.
    let chase_iron = MovementEffort::effort_appetite(&iron, ActivityIntensity::High, 60, true);
    let chase_soft = MovementEffort::effort_appetite(&soft, ActivityIntensity::High, 60, true);
    assert!(
        chase_iron > chase_soft,
        "a trailing side's determined players push from 55': {chase_iron} vs {chase_soft}"
    );
    // …and it reaches the sprint band too: refusing to stop sprinting
    // late is what the attribute names.
    let sprint_iron =
        MovementEffort::effort_appetite(&iron, ActivityIntensity::VeryHigh, 80, false);
    let sprint_soft =
        MovementEffort::effort_appetite(&soft, ActivityIntensity::VeryHigh, 80, false);
    assert!(sprint_iron > sprint_soft);
}

#[test]
fn population_mean_appetite_is_neutral() {
    // The generator mean at the calibration level is ~11.8 — a player
    // there must multiply by ~1.0 in every window, or the mechanism
    // shifts the calibrated population under itself.
    let ordinary = build_mover(11.8, 11.8);
    for (minute, trailing) in [(30u32, false), (80, false), (60, true)] {
        let m = MovementEffort::effort_appetite(
            &ordinary,
            ActivityIntensity::Moderate,
            minute,
            trailing,
        );
        assert!(
            (0.96..=1.04).contains(&m),
            "ordinary appetite should be ~1.0, got {m} at minute {minute}"
        );
    }
}
