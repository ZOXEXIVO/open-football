//! The shapes a free kick, a penalty and a throw-in are set up in.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::PassOriginRestart;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::officiating::restart_shape::{RestartShape, RestartStation};
use crate::r#match::engine::player::events::players::{FoulSeverity, PlayerEventDispatcher};
use crate::r#match::engine::result::Score;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, PlayerSide,
    ResultMatchPositionData,
};
use nalgebra::Vector3;

const W: f32 = 840.0;
const H: f32 = 545.0;

fn pitch() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    (field, context)
}

fn left_outfielder(field: &MatchField) -> u32 {
    field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .unwrap()
}

/// Everybody on the Right side crowded round a spot.
fn crowd_round(field: &mut MatchField, spot: Vector3<f32>) {
    for (i, p) in field
        .players
        .iter_mut()
        .filter(|p| {
            p.side == Some(PlayerSide::Right)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .enumerate()
    {
        p.position = spot + Vector3::new(10.0 + i as f32 * 3.0, (i as f32 - 5.0) * 4.0, 0.0);
    }
}

fn station_of(stations: &[RestartStation], id: u32) -> Option<Vector3<f32>> {
    stations
        .iter()
        .find(|s| s.player_id == id)
        .map(|s| s.position)
}

#[test]
fn a_free_kick_in_range_gets_a_wall_and_everyone_else_steps_back() {
    let (mut field, _) = pitch();
    let taker = left_outfielder(&field);
    // 22 m out, central, attacking the right-hand goal.
    let spot = Vector3::new(W - 176.0, H * 0.5, 0.0);
    crowd_round(&mut field, spot);

    let stations = RestartShape::plan(
        PassOriginRestart::DirectFreeKick,
        &field.players,
        taker,
        spot,
        W,
        H,
    );

    let goal = Vector3::new(W, H * 0.5, 0.0);
    let dir = (goal - spot).normalize();
    let on_the_line = stations
        .iter()
        .filter(|s| {
            let rel = s.position - spot;
            let along = rel.dot(&dir);
            let across = (rel - dir * along).norm();
            (70.0..=90.0).contains(&along) && across <= 20.0
        })
        .count();
    assert!(on_the_line >= 3, "a wall of {on_the_line}");

    for p in field
        .players
        .iter()
        .filter(|p| p.side == Some(PlayerSide::Right))
    {
        let at = station_of(&stations, p.id).unwrap_or(p.position);
        assert!(
            (at - spot).norm() >= RestartShape::RETREAT,
            "{} is {:.1}u from the ball",
            p.id,
            (at - spot).norm()
        );
    }
}

#[test]
fn a_penalty_empties_the_area_and_the_arc() {
    let (mut field, _) = pitch();
    let taker = left_outfielder(&field);
    let spot = Vector3::new(W - 88.0, H * 0.5, 0.0);
    crowd_round(&mut field, Vector3::new(W - 60.0, H * 0.5, 0.0));
    for p in field
        .players
        .iter_mut()
        .filter(|p| p.side == Some(PlayerSide::Left))
    {
        if p.id != taker && !p.tactical_position.current_position.is_goalkeeper() {
            p.position = Vector3::new(W - 100.0, H * 0.5 + 30.0, 0.0);
        }
    }

    let stations = RestartShape::plan(
        PassOriginRestart::Penalty,
        &field.players,
        taker,
        spot,
        W,
        H,
    );

    for p in field.players.iter() {
        let defending_keeper = p.side == Some(PlayerSide::Right)
            && p.tactical_position.current_position.is_goalkeeper();
        if p.id == taker || defending_keeper {
            continue;
        }
        let at = station_of(&stations, p.id).unwrap_or(p.position);
        let in_area = at.x > W - 132.0 && (at.y - H * 0.5).abs() < 161.3;
        assert!(!in_area, "{} still in the area at {at:?}", p.id);
        assert!(
            (at - spot).norm() >= RestartShape::RETREAT,
            "{} inside the arc",
            p.id
        );
    }
}

#[test]
fn a_throw_in_gets_two_short_options() {
    let (field, _) = pitch();
    let taker = left_outfielder(&field);
    let spot = Vector3::new(500.0, 0.0, 0.0);

    let stations = RestartShape::plan(
        PassOriginRestart::ThrowIn,
        &field.players,
        taker,
        spot,
        W,
        H,
    );

    assert_eq!(stations.len(), 2);
    for s in &stations {
        assert!(
            (s.position - spot).norm() < 160.0,
            "{:?} is no short option",
            s.position
        );
        assert!(s.position.y > 0.0, "an option inside the pitch");
    }
}

#[test]
fn stations_are_kept_while_the_free_kick_waits() {
    let (mut field, mut context) = pitch();
    let fouler = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .unwrap();
    let spot = Vector3::new(W - 176.0, H * 0.5, 0.0);
    crowd_round(&mut field, spot);
    field.ball.position = spot;

    PlayerEventDispatcher::award_restart_for_foul(
        fouler,
        FoulSeverity::Normal,
        spot,
        &mut field,
        &mut context,
    );
    let mut match_data = ResultMatchPositionData::empty();
    let mut tick_context = GameTickContext::new(&field, &context.players);
    FootballEngine::<840, 545>::game_tick(
        &mut field,
        &mut context,
        &mut match_data,
        &mut tick_context,
    );

    let held = field
        .players
        .iter()
        .filter(|p| p.set_piece_station.is_some())
        .count();
    assert!(held >= 3, "{held} players have a station");
    assert!(
        field.ball.awaiting_restart.is_some(),
        "the taker is still walking to it"
    );
    // Another tick of waiting does not clear them.
    context.increment_time();
    FootballEngine::<840, 545>::game_tick(
        &mut field,
        &mut context,
        &mut match_data,
        &mut tick_context,
    );
    assert_eq!(
        field
            .players
            .iter()
            .filter(|p| p.set_piece_station.is_some())
            .count(),
        held
    );
}
