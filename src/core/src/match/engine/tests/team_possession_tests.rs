//! **One fact of possession**, scenario by scenario, and what the team plans
//! do with it.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::{AwaitedRestart, RestartQuiet};
use crate::r#match::engine::result::Score;
use crate::r#match::engine::teamplay::possession::TeamPossession;
use crate::r#match::{
    DefenceRefreshInputs, DefensivePlan, GameTickContext, MatchContext, MatchField,
    MatchPlayerCollection, PassOriginRestart, StateProcessingContext,
};
use nalgebra::Vector3;

const HOME: u32 = 1;
const AWAY: u32 = 2;

fn pitch() -> (MatchField, MatchContext) {
    let home = squad(HOME, 100);
    let away = squad(AWAY, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let context = MatchContext::new(&field, players, Score::new(HOME, AWAY), false, false);
    (field, context)
}

fn man(field: &MatchField, team_id: u32, nth: usize) -> u32 {
    field
        .players
        .iter()
        .filter(|p| p.team_id == team_id)
        .nth(nth)
        .expect("a player")
        .id
}

fn stand(field: &mut MatchField, id: u32, x: f32, y: f32) {
    field.get_player_mut(id).expect("a player").position = Vector3::new(x, y, 0.0);
}

/// Everybody far from the ball except the two men named, who stand where the
/// scenario puts them.
fn loose_ball(field: &mut MatchField, home_gap: f32, away_gap: f32) -> (u32, u32) {
    let (near_home, near_away) = (man(field, HOME, 5), man(field, AWAY, 5));
    for player in field.players.iter_mut() {
        player.position =
            Vector3::new(if player.team_id == HOME { 50.0 } else { 790.0 }, 30.0, 0.0);
    }
    field.ball.position = Vector3::new(420.0, 272.0, 0.0);
    field.ball.current_owner = None;
    field.ball.pass_target_player_id = None;
    field.ball.kickoff_taker = None;
    stand(field, near_home, 420.0 - home_gap, 272.0);
    stand(field, near_away, 420.0 + away_gap, 272.0);
    (near_home, near_away)
}

/// The fact as it stands the moment the home side has just had the ball.
fn held_by_home(field: &mut MatchField) -> TeamPossession {
    field.ball.current_owner = Some(man(field, HOME, 5));
    let possession = TeamPossession::read(field);
    field.ball.current_owner = None;
    assert!(possession.held_by(HOME));
    possession
}

#[test]
fn a_backward_pass_stays_with_the_side_that_played_it() {
    let (mut field, _) = pitch();
    loose_ball(&mut field, 200.0, 5.0);
    let mut possession = held_by_home(&mut field);
    field.ball.velocity = Vector3::new(-3.0, 0.0, 0.0);
    field.ball.pass_target_player_id = Some(man(&field, HOME, 3));
    possession.update(&field);
    assert!(possession.held_by(HOME));
    assert!(!possession.held_by(AWAY));
}

#[test]
fn team_mates_at_different_distances_read_the_same_answer() {
    let (mut field, context) = pitch();
    let (near_home, _) = loose_ball(&mut field, 30.0, 40.0);
    let far_home = man(&field, HOME, 2);
    let tick_context = GameTickContext::new(&field, &context.players);
    let reading = |id: u32| {
        let ctx = StateProcessingContext {
            in_state_time: 0,
            player: field.get_player(id).expect("a player"),
            context: &context,
            tick_context: &tick_context,
        };
        ctx.team().is_control_ball()
    };
    assert_eq!(reading(near_home), reading(far_home));
}

#[test]
fn exactly_one_side_holds_a_loose_ball() {
    let (mut field, _) = pitch();
    loose_ball(&mut field, 50.0, 50.0);
    let possession = TeamPossession::read(&field);
    assert!(possession.held_by(HOME) != possession.held_by(AWAY));
}

#[test]
fn a_deflected_ball_passes_to_an_opponent_decisively_closer() {
    let (mut field, _) = pitch();
    loose_ball(&mut field, 100.0, 40.0);
    let mut possession = held_by_home(&mut field);
    possession.update(&field);
    assert!(possession.held_by(AWAY));
    // The first man to it keeps it: nearing it again is not enough until
    // the old holders are decisively closer in turn.
    loose_ball(&mut field, 60.0, 50.0);
    possession.update(&field);
    assert!(possession.held_by(AWAY));
}

#[test]
fn an_opponent_a_little_closer_does_not_take_a_loose_ball() {
    let (mut field, _) = pitch();
    loose_ball(&mut field, 60.0, 50.0);
    let mut possession = held_by_home(&mut field);
    possession.update(&field);
    assert!(possession.held_by(HOME));
}

#[test]
fn a_waiting_throw_in_belongs_to_the_side_taking_it() {
    let (mut field, context) = pitch();
    loose_ball(&mut field, 5.0, 300.0);
    let taker = man(&field, AWAY, 4);
    let mut possession = held_by_home(&mut field);
    field.ball.awaiting_restart = Some(AwaitedRestart {
        taker_id: taker,
        spot: Vector3::new(420.0, 0.0, 0.0),
        take_from: None,
        settled: false,
        carrying: false,
        origin: PassOriginRestart::ThrowIn,
        awarded_tick: context.current_tick(),
        patience_ticks: AwaitedRestart::PATIENCE_TICKS,
        settled_tick: None,
        hold: None,
        quiet: RestartQuiet::default(),
    });
    possession.update(&field);
    assert!(possession.held_by(AWAY));
}

#[test]
fn with_no_previous_owner_the_nearer_side_has_it() {
    let (mut field, _) = pitch();
    loose_ball(&mut field, 20.0, 90.0);
    assert!(TeamPossession::read(&field).held_by(HOME));
    loose_ball(&mut field, 90.0, 20.0);
    assert!(TeamPossession::read(&field).held_by(AWAY));
}

fn refreshed_plans(
    field: &MatchField,
    possession: TeamPossession,
) -> (DefensivePlan, DefensivePlan) {
    let (mut home, mut away) = (DefensivePlan::idle(), DefensivePlan::idle());
    DefensivePlan::refresh(
        &mut home,
        &mut away,
        &DefenceRefreshInputs {
            field,
            possession_holder: possession.holder(),
            home_team_id: HOME,
            away_team_id: AWAY,
            home_keeper_voice: 0.5,
            away_keeper_voice: 0.5,
        },
    );
    (home, away)
}

#[test]
fn a_sides_own_pass_in_flight_leaves_its_plan_idle() {
    let (mut field, _) = pitch();
    loose_ball(&mut field, 200.0, 5.0);
    let mut possession = held_by_home(&mut field);
    field.ball.pass_target_player_id = Some(man(&field, HOME, 3));
    possession.update(&field);
    let (home, away) = refreshed_plans(&field, possession);
    assert!(!home.active, "the side passing the ball marks nobody");
    assert!(away.active, "the side without it is defending");
}

#[test]
fn a_turnover_activates_the_plan_within_one_refresh() {
    let (mut field, _) = pitch();
    loose_ball(&mut field, 200.0, 5.0);
    let mut possession = held_by_home(&mut field);
    let (home, _) = refreshed_plans(&field, possession);
    assert!(!home.active);
    field.ball.current_owner = Some(man(&field, AWAY, 5));
    possession.update(&field);
    let (home, away) = refreshed_plans(&field, possession);
    assert!(home.active);
    assert!(!away.active);
}
