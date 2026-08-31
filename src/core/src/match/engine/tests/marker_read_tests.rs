//! Contract of the marking-read duel (`MarkerEvasion::holder_quality`
//! vs `mover_quality`) — the shared blends both halves of the engine
//! read: the attacker's evasion edge, the back line's tracking
//! prediction (`DefenderMarkingState`), and the midfield guard
//! (`MidfielderGuardingState`).
//!
//! Background (2026-08-31): the tracker's read was `(reading − 0.5)` on
//! a blend without `marking`, so the man being tracked contributed
//! nothing — an 18-off_the_ball mover degraded his marker's
//! extrapolation exactly as much as a 6, evasion-amplitude sweeps
//! measured zero separation, and the definitive sweep had off_the_ball
//! leaning wrong-side (+0.24 pooled) with marking at −0.16. The read is
//! now a duel of these two blends, centred at zero for an even contest
//! at any level.

#![cfg(test)]

use crate::PlayerSkills;
use crate::club::player::builder::PlayerBuilder;
use crate::r#match::MatchPlayer;
use crate::r#match::player::strategies::players::MarkerEvasion;
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
};
use chrono::NaiveDate;

fn build(marking: f32, off_the_ball: f32) -> MatchPlayer {
    let mut skills = PlayerSkills::default();
    skills.technical.marking = marking;
    skills.mental.off_the_ball = off_the_ball;
    // Hold the supporting attributes at the population mean so each
    // test moves exactly one lever.
    skills.mental.anticipation = 12.0;
    skills.mental.positioning = 12.0;
    skills.mental.concentration = 12.0;
    skills.physical.acceleration = 12.0;
    skills.physical.agility = 12.0;
    let player = PlayerBuilder::new()
        .id(1)
        .full_name(FullName::new("M".to_string(), "R".to_string()))
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
fn marking_leads_the_holder_blend() {
    let sticky = build(18.0, 12.0);
    let loose = build(6.0, 12.0);
    let h_sticky = MarkerEvasion::holder_quality(&sticky);
    let h_loose = MarkerEvasion::holder_quality(&loose);
    assert!(
        h_sticky > h_loose + 0.15,
        "marking 18 must hold visibly better than 6: {h_sticky} vs {h_loose}"
    );
}

#[test]
fn off_the_ball_leads_the_mover_blend() {
    let tricky = build(12.0, 18.0);
    let statue = build(12.0, 6.0);
    let m_tricky = MarkerEvasion::mover_quality(&tricky, 30);
    let m_statue = MarkerEvasion::mover_quality(&statue, 30);
    assert!(
        m_tricky > m_statue + 0.15,
        "off_the_ball 18 must move visibly better than 6: {m_tricky} vs {m_statue}"
    );
}

#[test]
fn even_contest_reads_level() {
    // A population-mean tracker on a population-mean mover: the read
    // edge that the tracking states compute from these two blends must
    // sit near zero, or the mechanism shifts the calibrated marking
    // tightness under itself.
    let marker = build(11.8, 11.8);
    let mover = build(11.8, 11.8);
    let edge = MarkerEvasion::holder_quality(&marker) - MarkerEvasion::mover_quality(&mover, 30);
    assert!(
        edge.abs() < 0.08,
        "even contest must read ~level, got edge {edge}"
    );
}
