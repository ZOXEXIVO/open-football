//! **A goal is an own goal by the football rule.** The last player to touch
//! the ball decides it: the defending side's touch makes an own goal unless
//! the ball was already a shot on its way in, and how long he had it does
//! not enter.
//!
//! The left side attacks the goal at `x = 840`, so the right side concedes
//! every goal here, and every contact that decides it has to be booked as a
//! touch.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::ball::events::{BallEvent, BallEventDispatcher, GoalSide};
use crate::r#match::engine::ball::ball::contest::pass_block::PassBlockCommit;
use crate::r#match::engine::player::events::players::PlayerEventDispatcher;
use crate::r#match::engine::result::Score;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayerCollection, MatchRng, PlayerSide, ResultMatchPositionData,
    events::EventCollection,
};
use nalgebra::Vector3;

const KICKOFF_MS: u64 = 10 * 60 * 1000;

fn kickoff() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = KICKOFF_MS;
    (field, context)
}

fn outfielder(field: &MatchField, side: PlayerSide, nth: usize) -> u32 {
    field
        .players
        .iter()
        .filter(|p| p.side == Some(side) && !p.tactical_position.current_position.is_goalkeeper())
        .nth(nth)
        .map(|p| p.id)
        .expect("a side has outfielders")
}

fn keeper(field: &MatchField, side: PlayerSide) -> u32 {
    field
        .players
        .iter()
        .find(|p| p.side == Some(side) && p.tactical_position.current_position.is_goalkeeper())
        .map(|p| p.id)
        .expect("a side has a keeper")
}

fn touch(field: &mut MatchField, context: &MatchContext, player: u32) {
    let team = field
        .players
        .iter()
        .find(|p| p.id == player)
        .unwrap()
        .team_id;
    field
        .ball
        .record_touch(player, team, context.current_tick(), false);
}

/// The left side strikes a shot at the right side's goal, on frame or not.
fn shoot(field: &mut MatchField, context: &MatchContext, on_frame: bool) -> u32 {
    let shooter = outfielder(field, PlayerSide::Left, 4);
    if let Some(player) = field.players.iter_mut().find(|p| p.id == shooter) {
        player.memory.shots_taken = 1;
        player.memory.last_shot_tick = context.current_tick();
    }
    field.ball.last_shot_shooter_id = Some(shooter);
    field.ball.last_shot_struck_tick = context.current_tick();
    field.ball.previous_owner = Some(shooter);
    touch(field, context, shooter);
    field.ball.last_shot_on_frame = on_frame;
    shooter
}

/// The left side's crosser strikes a ball across the box.
fn cross(field: &mut MatchField, context: &MatchContext) -> u32 {
    let crosser = outfielder(field, PlayerSide::Left, 0);
    field.ball.previous_owner = Some(crosser);
    touch(field, context, crosser);
    field.ball.record_passer(crosser, 1, context.current_tick());
    crosser
}

/// The ball crosses the right side's goal line, low and central. Returns
/// the goal event, if one was given.
fn into_the_net(
    field: &mut MatchField,
    context: &MatchContext,
) -> Option<(u32, Option<u32>, bool)> {
    field.ball.position = Vector3::new(841.0, 545.0 / 2.0, 0.5);
    field.ball.velocity = Vector3::new(1.5, 0.0, 0.0);
    field.ball.current_owner = None;
    let mut events = EventCollection::with_capacity(4);
    field.ball.check_goal(context, &mut events);
    events.drain().into_iter().find_map(|event| match event {
        crate::r#match::events::Event::BallEvent(BallEvent::Goal(goal)) => {
            assert_eq!(goal.side, GoalSide::Away, "the right side's goal");
            Some((
                goal.goalscorer_player_id,
                goal.assist_player_id,
                goal.auto_goal,
            ))
        }
        _ => None,
    })
}

#[test]
fn a_defenders_touch_on_a_shot_heading_on_frame_marks_the_shot() {
    let (mut field, context) = kickoff();
    let shooter = shoot(&mut field, &context, true);
    let defender = outfielder(&field, PlayerSide::Right, 0);
    touch(&mut field, &context, defender);
    let met = field.ball.shot_met.expect("his touch met the shot");
    assert_eq!(met.shooter, shooter);
    assert!(met.on_frame);
}

#[test]
fn a_defenders_touch_on_a_cross_does_not() {
    let (mut field, context) = kickoff();
    cross(&mut field, &context);
    let defender = outfielder(&field, PlayerSide::Right, 0);
    touch(&mut field, &context, defender);
    assert!(field.ball.shot_met.is_none(), "a cross is not a shot");
}

#[test]
fn a_cross_deflected_in_is_an_own_goal() {
    let (mut field, context) = kickoff();
    cross(&mut field, &context);
    let defender = outfielder(&field, PlayerSide::Right, 1);
    touch(&mut field, &context, defender);
    let (scorer, assist, own_goal) = into_the_net(&mut field, &context).expect("a goal");
    assert_eq!(scorer, defender, "booked against the defender");
    assert!(own_goal, "not a goal for any attacker");
    assert_eq!(assist, None, "an own goal carries no assist");
}

#[test]
fn a_shot_on_target_deflected_in_is_the_shooters_goal() {
    let (mut field, context) = kickoff();
    let shooter = shoot(&mut field, &context, true);
    let defender = outfielder(&field, PlayerSide::Right, 1);
    touch(&mut field, &context, defender);
    let (scorer, _, own_goal) = into_the_net(&mut field, &context).expect("a goal");
    assert_eq!(scorer, shooter);
    assert!(!own_goal);
}

#[test]
fn a_shot_going_wide_turned_in_is_an_own_goal() {
    let (mut field, context) = kickoff();
    shoot(&mut field, &context, false);
    let defender = outfielder(&field, PlayerSide::Right, 1);
    touch(&mut field, &context, defender);
    let (scorer, _, own_goal) = into_the_net(&mut field, &context).expect("a goal");
    assert_eq!(scorer, defender);
    assert!(own_goal);
}

#[test]
fn a_save_that_comes_back_in_off_a_defender_is_an_own_goal() {
    let (mut field, context) = kickoff();
    shoot(&mut field, &context, true);
    let keeper = keeper(&field, PlayerSide::Right);
    touch(&mut field, &context, keeper);
    let defender = outfielder(&field, PlayerSide::Right, 1);
    touch(&mut field, &context, defender);
    let (scorer, _, own_goal) = into_the_net(&mut field, &context).expect("a goal");
    assert_eq!(
        scorer, defender,
        "the ball that reached him was no longer the shot"
    );
    assert!(own_goal);
}

#[test]
fn a_save_that_goes_in_off_the_keeper_is_the_shooters_goal() {
    let (mut field, context) = kickoff();
    let shooter = shoot(&mut field, &context, true);
    let keeper = keeper(&field, PlayerSide::Right);
    touch(&mut field, &context, keeper);
    let (scorer, _, own_goal) = into_the_net(&mut field, &context).expect("a goal");
    assert_eq!(scorer, shooter);
    assert!(!own_goal);
}

#[test]
fn a_brief_touch_is_still_an_own_goal() {
    let (mut field, context) = kickoff();
    cross(&mut field, &context);
    let defender = outfielder(&field, PlayerSide::Right, 1);
    touch(&mut field, &context, defender);
    field.ball.ownership_duration = 0;
    let (scorer, _, own_goal) = into_the_net(&mut field, &context).expect("a goal");
    assert_eq!(scorer, defender, "however short his touch");
    assert!(own_goal);
}

#[test]
fn an_attackers_touch_last_is_his_goal_not_an_own_goal() {
    let (mut field, context) = kickoff();
    let shooter = shoot(&mut field, &context, true);
    let (scorer, _, own_goal) = into_the_net(&mut field, &context).expect("a goal");
    assert_eq!(scorer, shooter);
    assert!(!own_goal);
}

/// **Every defending contact is a touch.** The own-goal rule reads the last
/// touch, so a contact that books none leaves the goal on whoever touched
/// the ball before him.
#[test]
fn a_blocks_deferred_contact_books_the_blockers_touch() {
    let (mut field, context) = kickoff();
    let passer = outfielder(&field, PlayerSide::Left, 3);
    let blocker = outfielder(&field, PlayerSide::Right, 0);
    let at = Vector3::new(600.0, 272.0, 0.2);
    field
        .players
        .iter_mut()
        .find(|p| p.id == blocker)
        .unwrap()
        .position = Vector3::new(at.x, at.y, 0.0);
    touch(&mut field, &context, passer);
    field.ball.position = at;
    field.ball.velocity = Vector3::new(1.5, 0.0, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(passer);
    field.ball.flags.in_flight_state = 20;
    field.ball.pass_blocked_by = Some(PassBlockCommit {
        blocker_id: blocker,
        outcome_roll: 0.5,
        reaction: 1.0,
    });
    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(4);
    field.ball.try_block_pass(&context, &players, &mut events);
    assert_eq!(field.ball.last_touch_player_id, Some(blocker));
}

#[test]
fn a_headed_clearance_books_the_defenders_touch() {
    let (mut field, context) = kickoff();
    cross(&mut field, &context);
    let defender = outfielder(&field, PlayerSide::Right, 0);
    let mut out = EventCollection::with_capacity(4);
    let at = field.ball.position;
    BallEventDispatcher::dispatch(
        BallEvent::HeadedClear(defender, at),
        &mut field,
        &context,
        &mut out,
    );
    assert_eq!(field.ball.last_touch_player_id, Some(defender));
}

/// A clearance or a keeper's punch off a ball he did not own.
#[test]
fn a_clearance_and_a_punch_book_the_defenders_touch() {
    for punch in [false, true] {
        let (mut field, mut context) = kickoff();
        cross(&mut field, &context);
        let clearer = if punch {
            keeper(&field, PlayerSide::Right)
        } else {
            outfielder(&field, PlayerSide::Right, 0)
        };
        let at = Vector3::new(800.0, 272.0, 0.4);
        field
            .players
            .iter_mut()
            .find(|p| p.id == clearer)
            .unwrap()
            .position = Vector3::new(at.x, at.y, 0.0);
        field.ball.position = at;
        field.ball.velocity = Vector3::new(0.5, 0.0, 0.0);
        field.ball.current_owner = None;
        let mut data = ResultMatchPositionData::new();
        PlayerEventDispatcher::dispatch(
            PlayerEvent::ClearBall(clearer, Vector3::new(-2.0, 0.5, 0.1)),
            &mut field,
            &mut context,
            &mut data,
        );
        assert_eq!(
            field.ball.last_touch_player_id,
            Some(clearer),
            "punch: {punch}"
        );
    }
}

#[test]
fn a_keepers_fumble_books_his_touch() {
    let (mut field, context) = kickoff();
    cross(&mut field, &context);
    let keeper = keeper(&field, PlayerSide::Right);
    let player = field
        .players
        .iter()
        .find(|p| p.id == keeper)
        .unwrap()
        .clone();
    field.ball.flap_off(&player, 40.0, 10.0);
    assert_eq!(field.ball.last_touch_player_id, Some(keeper));
}

#[test]
fn a_tackle_that_knocks_it_loose_books_the_tacklers_touch() {
    for seed in 0..64 {
        let (mut field, context) = kickoff();
        let carrier = outfielder(&field, PlayerSide::Left, 3);
        let tackler = outfielder(&field, PlayerSide::Right, 0);
        let at = Vector3::new(400.0, 272.0, 0.0);
        field.ball.position = at;
        field.ball.current_owner = Some(carrier);
        touch(&mut field, &context, carrier);
        let stretched = field
            .players
            .iter_mut()
            .find(|p| p.id == tackler)
            .map(|p| {
                p.position = at + Vector3::new(14.0, 0.0, 0.0);
                p.clone()
            })
            .unwrap();
        let rng = MatchRng::from_seed(seed);
        if !field
            .ball
            .on_tackle_won(&stretched, 0.0, Vector3::zeros(), &rng)
        {
            assert_eq!(field.ball.last_touch_player_id, Some(tackler));
            return;
        }
    }
    panic!("no tackle at full stretch knocked the ball loose in 64 tries");
}
