//! **The delivery census books each ball where it belongs.** A cross is
//! followed from its strike to its first contact and its end, and every
//! corner, throw-in and own goal lands under the source that made it.
//!
//! The left side attacks the goal at `x = 840` throughout, so its crosses
//! go in from the right wing and the right side defends them.

#![cfg(all(test, feature = "match-logs"))]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::delivery_diag::{DeliveryCensus, TouchKind};
use crate::r#match::engine::result::Score;
use crate::r#match::passing::CrossType;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayerCollection, PlayerSide, events::EventCollection,
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

fn team_of(field: &MatchField, player: u32) -> u32 {
    field
        .players
        .iter()
        .find(|p| p.id == player)
        .unwrap()
        .team_id
}

/// One touch, named by `kind` the way its site names it.
fn touch(
    field: &mut MatchField,
    context: &MatchContext,
    player: u32,
    kind: Option<TouchKind>,
    controlled: bool,
    height: f32,
) {
    let team = team_of(field, player);
    field.ball.position.z = height;
    field.ball.touch_kind_hint = kind;
    field
        .ball
        .record_touch(player, team, context.current_tick(), controlled);
}

/// A left-side wide man strikes `cross_type` from the right wing, deep in
/// the final third.
fn cross(field: &mut MatchField, context: &MatchContext, cross_type: Option<CrossType>) {
    let crosser = outfielder(field, PlayerSide::Left, 0);
    let from = Vector3::new(780.0, 60.0, 0.0);
    field
        .players
        .iter_mut()
        .find(|p| p.id == crosser)
        .unwrap()
        .position = from;
    field.ball.position = from;
    field.ball.previous_owner = Some(crosser);
    touch(field, context, crosser, None, true, 0.0);
    field.ball.velocity = Vector3::new(0.6, 1.9, 0.05);
    DeliveryCensus::on_strike(
        field,
        context,
        crosser,
        cross_type,
        true,
        true,
        Vector3::new(800.0, 255.0, 0.0),
    );
}

/// Over the byline at the left side's attacking end, beside the post.
fn behind(field: &mut MatchField, context: &MatchContext) {
    field.ball.position = Vector3::new(841.0, 140.0, 0.0);
    field.ball.velocity = Vector3::new(1.5, 0.0, 0.0);
    field.ball.current_owner = None;
    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(4);
    field
        .ball
        .check_wide_of_goal(context, &players, &mut events);
}

/// Over the top touchline.
fn into_touch(field: &mut MatchField, context: &MatchContext) {
    field.ball.position = Vector3::new(420.0, -1.0, 0.0);
    field.ball.velocity = Vector3::new(0.0, -1.4, 0.0);
    field.ball.current_owner = None;
    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(4);
    field.ball.check_throw_in(context, &players, &mut events);
}

/// Into the right side's goal, low and central.
fn into_the_net(field: &mut MatchField, context: &MatchContext) {
    field.ball.position = Vector3::new(841.0, 545.0 / 2.0, 0.5);
    field.ball.velocity = Vector3::new(1.5, 0.0, 0.0);
    field.ball.current_owner = None;
    let mut events = EventCollection::with_capacity(4);
    field.ball.check_goal(context, &mut events);
}

fn corner_row(label: &str) -> usize {
    DeliveryCensus::CORNER_SOURCES
        .iter()
        .position(|l| *l == label)
        .unwrap()
}

fn throw_in_row(label: &str) -> usize {
    DeliveryCensus::THROW_SOURCES
        .iter()
        .position(|l| *l == label)
        .unwrap()
}

fn own_goal_row(label: &str) -> usize {
    DeliveryCensus::OWN_GOAL_ROUTES
        .iter()
        .position(|l| *l == label)
        .unwrap()
}

#[test]
fn a_cross_blocked_behind_is_booked_as_a_block_and_a_cross_sourced_corner() {
    let (mut field, context) = kickoff();
    cross(&mut field, &context, Some(CrossType::WhippedNearPost));
    let closer = outfielder(&field, PlayerSide::Right, 0);
    touch(
        &mut field,
        &context,
        closer,
        Some(TouchKind::Block),
        false,
        0.4,
    );
    behind(&mut field, &context);

    let tally = &field.ball.delivery_tally;
    let (kind, slot) = (
        DeliveryCensus::OPEN_PLAY,
        CrossType::WhippedNearPost.diag_index(),
    );
    assert_eq!(tally.struck[kind][slot], 1);
    assert_eq!(tally.first[kind][slot][DeliveryCensus::DEFENDER_BLOCK], 1);
    assert_eq!(tally.end[kind][slot][DeliveryCensus::BEHIND_CORNER], 1);
    assert_eq!(tally.corners[corner_row("cross blocked behind")], 1);
    assert!(
        field.ball.delivery_ledger.is_none(),
        "the corner ends the delivery"
    );
}

#[test]
fn a_cross_won_in_the_air_and_headed_at_goal_is_the_attackers_first_contact() {
    let (mut field, context) = kickoff();
    cross(&mut field, &context, Some(CrossType::FloatedFarPost));
    let header = outfielder(&field, PlayerSide::Left, 5);
    touch(
        &mut field,
        &context,
        header,
        Some(TouchKind::Shot),
        false,
        2.0,
    );

    let tally = &field.ball.delivery_tally;
    let (kind, slot) = (
        DeliveryCensus::OPEN_PLAY,
        CrossType::FloatedFarPost.diag_index(),
    );
    assert_eq!(tally.first[kind][slot][DeliveryCensus::ATTACKER_HEAD], 1);
    assert_eq!(tally.end[kind][slot][DeliveryCensus::SHOT], 1);
    assert_eq!(tally.headers_at_goal[kind][slot], 1);
    assert_eq!(tally.open_play_headers_at_goal(), 1);
}

#[test]
fn a_cross_headed_behind_is_a_cross_headed_or_cleared_corner() {
    let (mut field, context) = kickoff();
    cross(&mut field, &context, Some(CrossType::WhippedNearPost));
    let defender = outfielder(&field, PlayerSide::Right, 1);
    touch(
        &mut field,
        &context,
        defender,
        Some(TouchKind::Head),
        false,
        2.1,
    );
    behind(&mut field, &context);

    let tally = &field.ball.delivery_tally;
    assert_eq!(
        tally.corners[corner_row("cross headed or cleared behind")],
        1
    );
    let slot = CrossType::WhippedNearPost.diag_index();
    assert_eq!(
        tally.first[DeliveryCensus::OPEN_PLAY][slot][DeliveryCensus::DEFENDER_HEADER],
        1
    );
}

#[test]
fn a_pull_back_turned_behind_is_its_own_corner_source() {
    let (mut field, context) = kickoff();
    cross(&mut field, &context, Some(CrossType::Cutback));
    let defender = outfielder(&field, PlayerSide::Right, 2);
    touch(&mut field, &context, defender, None, false, 0.0);
    behind(&mut field, &context);

    assert_eq!(
        field.ball.delivery_tally.corners[corner_row("pull-back or driven ball turned behind")],
        1
    );
}

#[test]
fn a_corner_off_a_shot_block_is_not_a_cross_corner() {
    let (mut field, context) = kickoff();
    let defender = outfielder(&field, PlayerSide::Right, 0);
    touch(
        &mut field,
        &context,
        defender,
        Some(TouchKind::ShotBlock),
        false,
        0.5,
    );
    behind(&mut field, &context);

    assert_eq!(
        field.ball.delivery_tally.corners[corner_row("shot blocked")],
        1
    );
}

/// Every kind of last touch lands in its own throw-in row, split by the
/// half the toucher was in.
#[test]
fn a_throw_in_is_counted_once_under_the_last_touchers_action() {
    for (kind, row) in [
        (TouchKind::Tackle, "tackle"),
        (TouchKind::Block, "block"),
        (TouchKind::Clearance, "clearance"),
        (TouchKind::Head, "header"),
        (TouchKind::Control, "carry"),
        (TouchKind::PutOut, "put out under pressure"),
        (TouchKind::Feet, "other"),
    ] {
        let (mut field, context) = kickoff();
        let toucher = outfielder(&field, PlayerSide::Right, 0);
        field.ball.position = Vector3::new(420.0, 5.0, 0.0);
        touch(
            &mut field,
            &context,
            toucher,
            Some(kind),
            kind == TouchKind::Control,
            0.0,
        );
        into_touch(&mut field, &context);

        let tally = &field.ball.delivery_tally;
        let total: u32 = tally.throw_ins.iter().map(|h| h[0] + h[1]).sum();
        assert_eq!(total, 1, "{kind:?}: one throw-in");
        let [own, theirs] = tally.throw_ins[throw_in_row(row)];
        assert_eq!(own + theirs, 1, "{kind:?} lands under {row}");
    }
}

#[test]
fn a_pass_and_a_cross_put_out_are_booked_as_what_they_were() {
    for (was_cross, row) in [(false, "pass"), (true, "cross")] {
        let (mut field, context) = kickoff();
        let passer = outfielder(&field, PlayerSide::Left, 3);
        touch(&mut field, &context, passer, None, true, 0.0);
        DeliveryCensus::on_strike(
            &mut field,
            &context,
            passer,
            None,
            was_cross,
            false,
            Vector3::new(500.0, 10.0, 0.0),
        );
        into_touch(&mut field, &context);
        let [own, theirs] = field.ball.delivery_tally.throw_ins[throw_in_row(row)];
        assert_eq!(own + theirs, 1, "a {row} put out");
    }
}

/// The touchline census's clock runs from the previous throw-in: a touch
/// a second before the ball went out again does not reset it.
#[test]
fn back_to_back_throw_ins_are_measured_from_the_previous_throw_in() {
    let (mut field, mut context) = kickoff();
    let toucher = outfielder(&field, PlayerSide::Right, 0);
    touch(&mut field, &context, toucher, None, true, 0.0);
    into_touch(&mut field, &context);
    field.ball.awaiting_restart = None;

    context.total_match_time += 9_000;
    let other = outfielder(&field, PlayerSide::Left, 0);
    touch(&mut field, &context, other, None, true, 0.0);
    context.total_match_time += 1_000;
    into_touch(&mut field, &context);

    let tally = &field.ball.delivery_tally;
    assert_eq!(tally.throw_in_gaps, 1);
    assert_eq!(tally.throw_in_gap_ticks, 1_000, "ten seconds, not one");
}

#[test]
fn a_deflected_cross_into_the_net_is_booked_under_the_cross() {
    let (mut field, context) = kickoff();
    cross(&mut field, &context, Some(CrossType::DrivenLowCross));
    let defender = outfielder(&field, PlayerSide::Right, 0);
    touch(&mut field, &context, defender, None, false, 0.2);
    into_the_net(&mut field, &context);

    let row = field.ball.delivery_tally.own_goals[own_goal_row("deflected cross or pull-back")];
    assert_eq!(row.iter().sum::<u32>(), 1);
}

/// A shot met by a defender before anybody else touched it goes under the
/// shot, split by whether it was heading on frame.
#[test]
fn a_deflected_shot_is_booked_by_where_it_was_heading() {
    for (on_frame, row) in [
        (false, "deflected shot heading wide"),
        (true, "deflected shot on frame"),
    ] {
        let (mut field, context) = kickoff();
        let shooter = outfielder(&field, PlayerSide::Left, 4);
        field.ball.last_shot_shooter_id = Some(shooter);
        field.ball.last_shot_struck_tick = context.current_tick();
        field.ball.previous_owner = Some(shooter);
        touch(
            &mut field,
            &context,
            shooter,
            Some(TouchKind::Shot),
            false,
            0.3,
        );
        field.ball.last_shot_on_frame = on_frame;
        let defender = outfielder(&field, PlayerSide::Right, 0);
        touch(&mut field, &context, defender, None, false, 0.3);
        assert_eq!(field.ball.shot_met.map(|met| met.on_frame), Some(on_frame));
        into_the_net(&mut field, &context);

        let booked = field.ball.delivery_tally.own_goals[own_goal_row(row)];
        assert_eq!(booked.iter().sum::<u32>(), 1, "{row}");
    }
}

#[test]
fn a_clearance_a_keepers_touch_and_anything_else_each_have_a_route() {
    for (who, kind, row) in [
        (0, Some(TouchKind::Clearance), "clearance or header"),
        (1, None, "the keeper"),
        (0, Some(TouchKind::Control), "other"),
    ] {
        let (mut field, context) = kickoff();
        field.ball.previous_owner = Some(outfielder(&field, PlayerSide::Left, 3));
        let toucher = if who == 1 {
            keeper(&field, PlayerSide::Right)
        } else {
            outfielder(&field, PlayerSide::Right, 0)
        };
        touch(
            &mut field,
            &context,
            toucher,
            kind,
            kind == Some(TouchKind::Control),
            0.3,
        );
        into_the_net(&mut field, &context);

        let booked = field.ball.delivery_tally.own_goals[own_goal_row(row)];
        assert_eq!(booked.iter().sum::<u32>(), 1, "{row}");
    }
}

/// A shot's touch ends the shot-met fact for the next man: only the first
/// touch after the strike meets an untouched shot.
#[test]
fn the_shot_met_fact_clears_at_the_next_touch() {
    let (mut field, context) = kickoff();
    let shooter = outfielder(&field, PlayerSide::Left, 4);
    field.ball.last_shot_shooter_id = Some(shooter);
    field.ball.last_shot_struck_tick = context.current_tick();
    touch(
        &mut field,
        &context,
        shooter,
        Some(TouchKind::Shot),
        false,
        0.3,
    );
    field.ball.last_shot_on_frame = true;
    let first = outfielder(&field, PlayerSide::Right, 0);
    touch(&mut field, &context, first, None, false, 0.3);
    assert!(
        field.ball.shot_met.is_some(),
        "the first defender met the shot"
    );
    let second = outfielder(&field, PlayerSide::Right, 1);
    touch(&mut field, &context, second, None, false, 0.3);
    assert!(field.ball.shot_met.is_none(), "the second met a deflection");
}
