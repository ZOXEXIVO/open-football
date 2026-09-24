//! **The kick-off, from the set-up to the first touch.**
//!
//! The reported defect is a lone striker playing the kick-off to himself.
//! He did, and it was two things rather than one, so this file pins both:
//! [`KickoffShape`] has to stand somebody beside him, and
//! [`KickoffDelivery`] has to be what plays the ball, because his own
//! state machine will not.
//!
//! Every fixture here is a 4-2-3-1 — the shape that produces a single
//! forward, which is the shape the report is about. Nothing is asserted
//! about which man takes it: that is `assign_kickoff`'s choice and it is
//! allowed to change.
//!
//! [`KickoffShape`]: crate::r#match::engine::kickoff_shape::KickoffShape
//! [`KickoffDelivery`]: crate::r#match::common_states::KickoffDelivery

#![cfg(test)]

use super::recording_globals::RecordingGlobals;
use crate::club::player::builder::PlayerBuilder;
use crate::club::player::{PlayerPosition, PlayerPositions};
use crate::club::team::tactics::MatchTacticType;
use crate::r#match::common_states::KickoffDelivery;
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::goal::assign_kickoff;
use crate::r#match::engine::kickoff_shape::KickoffShape;
use crate::r#match::engine::result::Score;
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::squad::squad::MatchSquad;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, PlayerSide,
    StateProcessingContext,
};
use crate::shared::fullname::FullName;
use crate::{
    MatchRuntime, PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills, Tactics,
};
use chrono::NaiveDate;
use nalgebra::Vector3;

const T4231: [PlayerPositionType; 11] = [
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::DefenderLeft,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderCenterRight,
    PlayerPositionType::DefenderRight,
    PlayerPositionType::MidfielderCenterLeft,
    PlayerPositionType::MidfielderCenterRight,
    PlayerPositionType::AttackingMidfielderLeft,
    PlayerPositionType::AttackingMidfielderCenter,
    PlayerPositionType::AttackingMidfielderRight,
    PlayerPositionType::Striker,
];

const WIDTH: usize = 840;
const HEIGHT: usize = 545;

/// How long the test is prepared to watch him stand over it. Comfortably
/// past `KickoffDelivery::SCAN_TICKS` without pinning its value.
const KICKOFF_SCAN_BOUND: u32 = 200;

fn squad(team_id: u32, base_id: u32) -> MatchSquad {
    let birth = NaiveDate::from_ymd_opt(1998, 5, 1).unwrap();
    let main_squad = T4231
        .iter()
        .enumerate()
        .map(|(index, position)| {
            let attributes = PlayerAttributes {
                condition: 9000,
                current_ability: 150,
                ..Default::default()
            };
            let mut skills = PlayerSkills::default();
            skills.physical.pace = 14.0;
            skills.physical.acceleration = 14.0;
            skills.physical.agility = 14.0;
            skills.physical.stamina = 14.0;
            skills.physical.natural_fitness = 14.0;
            skills.physical.jumping = 14.0;
            skills.technical.passing = 14.0;
            skills.technical.first_touch = 14.0;
            let player = PlayerBuilder::new()
                .id(base_id + index as u32)
                .full_name(FullName::new(
                    "T".to_string(),
                    format!("P{}", base_id + index as u32),
                ))
                .birth_date(birth)
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(skills)
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: *position,
                        level: 18,
                    }],
                })
                .player_attributes(attributes)
                .build()
                .unwrap();
            crate::r#match::MatchPlayer::from_player(team_id, &player, *position, false, None)
        })
        .collect();

    MatchSquad {
        team_id,
        team_name: format!("Team{team_id}"),
        tactics: Tactics::new(MatchTacticType::T4231),
        main_squad,
        substitutes: vec![],
        captain_id: None,
        vice_captain_id: None,
        penalty_taker_id: None,
        free_kick_taker_id: None,
        selection_omissions: vec![],
        overlooked: vec![],
        coach_snapshot: None,
    }
}

/// A field with `side`'s kick-off already set.
///
/// Both sides, everywhere below: the away column of `POSITION_POSITIONING`
/// is the home column reflected, and a set-up that reads the reflection
/// wrong handicaps one side at every kick-off of every match. The Right
/// arm is the one a goal against the away team reaches.
fn set_kickoff(side: PlayerSide) -> MatchField {
    let mut field = MatchField::new(WIDTH, HEIGHT, squad(1, 100), squad(2, 200));
    assign_kickoff(&mut field, side, None);
    field
}

const BOTH_SIDES: [PlayerSide; 2] = [PlayerSide::Left, PlayerSide::Right];

fn position_of(field: &MatchField, id: u32) -> Vector3<f32> {
    field
        .players
        .iter()
        .find(|p| p.id == id)
        .expect("the man is on the field")
        .position
}

#[test]
fn the_kick_off_stands_a_team_mate_beside_the_taker() {
    for side in BOTH_SIDES {
        let field = set_kickoff(side);
        let taker = field.ball.kickoff_taker.expect("somebody takes it");
        let partner = field.ball.kickoff_partner.expect("somebody receives it");
        assert_ne!(taker, partner);

        // Before the set-up existed this was 60 u — 7.5 m, and behind him.
        let apart = (position_of(&field, partner) - position_of(&field, taker)).norm();
        assert!(
            apart <= 40.0,
            "{side:?}: the partner must be within a roll of the ball, stood {apart:.1}u away"
        );

        let partner_player = field
            .players
            .iter()
            .find(|p| p.id == partner)
            .expect("the partner is on the field");
        assert_eq!(partner_player.side, Some(side));
        assert!(
            !partner_player
                .tactical_position
                .current_position
                .is_goalkeeper(),
            "a goalkeeper does not walk up for the kick-off"
        );
    }
}

#[test]
fn the_opponents_stand_outside_the_centre_circle() {
    for side in BOTH_SIDES {
        let field = set_kickoff(side);
        let spot = field.ball.position;
        // Law 8, and the whole reason the taker used to be closed down
        // before he could play it: the opposing striker's formation dot is
        // 15 u from the centre mark.
        for player in field
            .players
            .iter()
            .filter(|p| p.side == Some(side.opposite()))
        {
            let gap = (player.position - spot).norm();
            assert!(
                gap >= KickoffShape::CIRCLE - 0.5,
                "{side:?}: {:?} stood {gap:.1}u from the ball at the kick-off",
                player.tactical_position.current_position
            );
        }
    }
}

#[test]
fn everybody_stays_in_his_own_half_for_the_kick_off() {
    let halfway = WIDTH as f32 / 2.0;
    for side in BOTH_SIDES {
        let field = set_kickoff(side);
        let taker = field.ball.kickoff_taker.expect("somebody takes it");
        for player in field.players.iter().filter(|p| p.id != taker) {
            let Some(player_side) = player.side else {
                continue;
            };
            // Positive when he is on his own side of the halfway line,
            // whichever end he defends.
            let own_half = player_side.forward_dir_x() * (halfway - player.position.x);
            assert!(
                own_half >= 0.0,
                "{side:?} kick-off: a {player_side:?} player stood at x={:.1}",
                player.position.x
            );
        }
    }
}

#[test]
fn a_lone_striker_rolls_the_kick_off_to_his_partner() {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(WIDTH, HEIGHT, home, away);
    let context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    assign_kickoff(&mut field, PlayerSide::Left, None);

    let taker = field.ball.kickoff_taker.expect("somebody takes it");
    let partner = field.ball.kickoff_partner.expect("somebody receives it");

    // He stands over it first — a kick-off is not struck on the tick the
    // referee sets it — and then he plays it, to the man the set-up put
    // there. Left to his own state machine he found no pass at all and
    // ran off with it on the thirty-first tick.
    let mut kicked = None;
    for held in 0..KICKOFF_SCAN_BOUND {
        field.ball.ownership_duration = held;
        let tick_context = GameTickContext::new(&field, &context.players);
        let player = field
            .players
            .iter()
            .find(|p| p.id == taker)
            .expect("the taker is on the field");
        let ctx = StateProcessingContext {
            in_state_time: held as u64,
            player,
            context: &context,
            tick_context: &tick_context,
        };
        assert!(
            KickoffDelivery::taking(&ctx),
            "the taker owns the restart until he plays it"
        );
        if let Some(event) = KickoffDelivery::deliver(&ctx) {
            kicked = Some((held, event));
            break;
        }
    }

    let (held, event) = kicked.expect("the kick-off must be played");
    assert!(held > 0, "the ball may not be struck on the setting tick");
    match event {
        Event::PlayerEvent(PlayerEvent::PassTo(pass)) => {
            assert_eq!(pass.from_player_id, taker);
            assert_eq!(
                pass.to_player_id, partner,
                "the kick-off goes to the man who walked up for it"
            );
        }
        _ => panic!("a kick-off is a pass"),
    }
}

#[test]
fn the_kick_off_is_over_once_the_taker_lets_go_of_it() {
    let mut field = set_kickoff(PlayerSide::Left);
    let taker = field.ball.kickoff_taker.expect("somebody takes it");

    // Law 8's second-touch bar is the same fact as the override's exit:
    // the restart IS that one touch, so nothing may hold him to it
    // afterwards — including the delivery, which would otherwise play a
    // second kick-off if the ball came back to him.
    field
        .ball
        .note_release(taker, position_of(&field, taker), 40);
    assert_eq!(field.ball.kickoff_taker, None);
    assert_eq!(field.ball.kickoff_partner, None);

    // …and the other half of the same sentence: a kick-off he never got
    // to take is over the moment anybody else plays the ball. Without it
    // the marker outlives the restart and he takes it again whenever the
    // ball next comes back to him.
    let mut field = set_kickoff(PlayerSide::Left);
    assert!(field.ball.kickoff_taker.is_some());
    let opponent = field
        .players
        .iter()
        .find(|p| p.side == Some(PlayerSide::Right))
        .map(|p| (p.id, p.team_id))
        .expect("the other side is on the field");
    field.ball.record_touch(opponent.0, opponent.1, 40, true);
    assert_eq!(field.ball.kickoff_taker, None);
    assert_eq!(field.ball.kickoff_partner, None);
}

#[test]
fn a_real_match_kicks_off_with_a_pass() {
    let _globals = RecordingGlobals::lock();
    let events_were_on = MatchRuntime::events_mode();
    MatchRuntime::set_events_mode(true);

    let mut config = MatchEngineConfig::seeded(0x0F00_0021);
    config.match_recordings = true;
    let result = FootballEngine::<840, 545>::play_with_config(squad(1, 100), squad(2, 200), config);

    MatchRuntime::set_events_mode(events_were_on);

    // The kick-off is played inside the first second and it is a PASS.
    // Before this existed the first pass of the match landed much later:
    // the taker spent thirty ticks looking for one, gave up, and carried
    // the ball into the opposition half by himself.
    let opening = result.position_data.get_passes_in_window(0, 1_000);
    assert!(
        !opening.is_empty(),
        "no pass in the first second of the match — the kick-off was carried"
    );
}
