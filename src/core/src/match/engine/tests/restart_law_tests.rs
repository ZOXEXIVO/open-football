//! The laws a restart carries with it: what an indirect free kick may and
//! may not do, and what a foul played on under advantage is still owed.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::club::player::traits::PlayerTrait;
use crate::r#match::engine::ball::ball::{AwaitedRestart, Ball, PassChainEntry, PassOriginRestart};
use crate::r#match::engine::coach::CoachInstruction;
use crate::r#match::engine::flow::context::PendingAdvantage;
use crate::r#match::engine::officiating::management::{Dissent, TimeWastingRestart};
use crate::r#match::engine::player::events::players::{
    FoulResolver, FoulSeverity, FoulSource, PlayerEvent,
};
use crate::r#match::engine::result::Score;
use crate::r#match::engine::result::{DeadTime, OffenceTally, PlayingTime};
use crate::r#match::events::{EventCollection, EventDispatcher};
use crate::r#match::{
    MatchContext, MatchField, MatchPlayer, MatchPlayerCollection, MatchRng, PlayerSide,
    ResultMatchPositionData,
};
use nalgebra::Vector3;

fn kickoff() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = 10 * 60 * 1000;
    context.rng = MatchRng::from_seed(0x1A75);
    (field, context)
}

/// An away outfielder strikes the ball over the home goal line from an
/// indirect free kick he has just taken. Returns his id.
fn strike_into_the_home_goal_from_an_indirect_free_kick(
    field: &mut MatchField,
    context: &MatchContext,
) -> u32 {
    let taker = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .expect("an away outfielder exists");
    if let Some(player) = field.players.iter_mut().find(|p| p.id == taker) {
        player.memory.shots_taken = 1;
        player.memory.last_shot_tick = context.current_tick();
    }
    field.ball.pass_origin_restart = PassOriginRestart::IndirectFreeKick;
    field.ball.last_shot_struck_tick = context.current_tick();
    field.ball.previous_owner = Some(taker);
    field.ball.current_owner = None;
    field.ball.position = Vector3::new(-1.0, 545.0 / 2.0 + 6.0, 0.6);
    field.ball.velocity = Vector3::new(-2.2, 0.0, -0.01);
    taker
}

#[test]
fn an_indirect_free_kick_struck_straight_in_is_no_goal() {
    let (mut field, context) = kickoff();
    strike_into_the_home_goal_from_an_indirect_free_kick(&mut field, &context);

    let mut events = EventCollection::with_capacity(4);
    field.ball.check_goal(&context, &mut events);

    assert!(!field.ball.goal_scored);
}

#[test]
fn an_indirect_free_kick_touched_by_a_second_player_can_score() {
    let (mut field, context) = kickoff();
    let scorer = strike_into_the_home_goal_from_an_indirect_free_kick(&mut field, &context);
    let team_mate = field
        .players
        .iter()
        .find(|p| p.side == Some(PlayerSide::Right) && p.id != scorer)
        .map(|p| p.id)
        .unwrap();
    field.ball.recent_passers.push_back(PassChainEntry {
        player_id: team_mate,
        team_id: 2,
        tick: context.current_tick(),
    });

    let mut events = EventCollection::with_capacity(4);
    field.ball.check_goal(&context, &mut events);

    assert!(field.ball.goal_scored);
}

fn away_outfielder(field: &MatchField) -> u32 {
    field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .unwrap()
}

fn advantage_owed_for(
    fouler_id: u32,
    spot: Vector3<f32>,
    context: &MatchContext,
    yellow: f32,
) -> PendingAdvantage {
    let now = context.current_tick();
    PendingAdvantage {
        fouler_id,
        start_tick: now,
        expire_tick: now + 150,
        fouled_team_id: 1,
        severity: FoulSeverity::Normal,
        source: FoulSource::Tackle,
        spot,
        yellow_prob: yellow,
        red_prob: 0.0,
    }
}

#[test]
fn an_advantage_pulled_back_restarts_where_the_foul_was() {
    let (mut field, mut context) = kickoff();
    let fouler = away_outfielder(&field);
    let foul_spot = Vector3::new(300.0, 200.0, 0.0);
    context.pending_advantage = Some(advantage_owed_for(fouler, foul_spot, &context, 0.0));

    // The fouled side has lost it thirty metres further on.
    field.ball.position = Vector3::new(540.0, 330.0, 0.0);
    field.ball.current_owner = Some(fouler);
    FoulResolver::tick_advantage(&mut field, &mut context);

    let restart = field.ball.awaiting_restart.expect("play is pulled back");
    assert!(
        (restart.spot - foul_spot).magnitude() < 1.0,
        "restarted at {:?}, the foul was at {foul_spot:?}",
        restart.spot
    );
}

#[test]
fn a_second_foul_inside_the_window_still_books_the_first() {
    let (mut field, mut context) = kickoff();
    let first = away_outfielder(&field);
    let second = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && p.id != first
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .unwrap();
    context.pending_advantage = Some(advantage_owed_for(
        first,
        field.ball.position,
        &context,
        1.0,
    ));
    // A referee who misses nothing, on a pinned stream: the second foul is
    // whistled.
    context.referee.foul_detection = 1.0;
    context.referee.strictness = 1.0;
    context.rng = MatchRng::from_seed(0x2AD_F0);

    let mut events = EventCollection::with_capacity(4);
    events.add_player_event(PlayerEvent::CommitFoul(
        second,
        FoulSeverity::Violent,
        FoulSource::Tackle,
    ));
    let mut match_data = ResultMatchPositionData::empty();
    EventDispatcher::dispatch(&mut events, &mut field, &mut context, &mut match_data, true);

    assert!(context.pending_advantage.is_none());
    assert_eq!(field.get_player(first).unwrap().yellow_cards, 1);
}

/// A carrier clean through on the right-hand goal, brought down by an
/// away outfielder on a pinned stream. Returns the tally line.
fn professional_foul_on_a_breakaway(seed: u64) -> OffenceTally {
    let (mut field, mut context) = kickoff();
    context.rng = MatchRng::from_seed(seed);
    let fouler = away_outfielder(&field);
    let carrier = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .unwrap();
    for p in field.players.iter_mut() {
        if p.id == carrier {
            p.position = Vector3::new(700.0, 272.0, 0.0);
        } else if p.id == fouler {
            p.position = Vector3::new(694.0, 268.0, 0.0);
        } else if p.side == Some(PlayerSide::Right)
            && !p.tactical_position.current_position.is_goalkeeper()
        {
            p.position = Vector3::new(400.0, p.position.y, 0.0);
        }
    }
    field.ball.position = Vector3::new(700.0, 272.0, 0.0);
    field.ball.current_owner = Some(carrier);

    let mut events = EventCollection::with_capacity(4);
    events.add_player_event(PlayerEvent::CommitFoul(
        fouler,
        FoulSeverity::Normal,
        FoulSource::ProfessionalFoul,
    ));
    let mut match_data = ResultMatchPositionData::empty();
    EventDispatcher::dispatch(&mut events, &mut field, &mut context, &mut match_data, true);
    context.tally.offences[FoulSource::ProfessionalFoul.index()]
}

#[test]
fn a_professional_foul_is_seen_and_always_carded() {
    let lines: Vec<OffenceTally> = (0..20).map(professional_foul_on_a_breakaway).collect();
    let whistled: u16 = lines.iter().map(|l| l.whistled).sum();
    assert!(whistled >= 16, "only {whistled} of 20 seen");
    for line in &lines {
        assert_eq!(line.yellows + line.reds, line.whistled);
    }
}

// ── Time-wasting ──────────────────────────────────────────────────────

/// The home side one up in the 85th minute, told to waste time.
fn home_ahead_late(context: &mut MatchContext, instruction: CoachInstruction) {
    context.score.increment_home_goals();
    context.total_match_time = 85 * 60_000;
    context.coach_home.instruction = instruction;
    context.rng = MatchRng::from_seed(0x7_1AE);
}

fn home_throw_in_ready(field: &mut MatchField, context: &MatchContext) -> u32 {
    let taker = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .unwrap();
    let spot = Vector3::new(500.0, 3.0, 0.0);
    field.get_player_mut(taker).unwrap().position = Vector3::new(500.0, 6.0, 0.0);
    field.ball.position = spot;
    field.ball.awaiting_restart = Some(AwaitedRestart {
        taker_id: taker,
        spot,
        take_from: None,
        settled: true,
        carrying: false,
        origin: PassOriginRestart::ThrowIn,
        awarded_tick: context.current_tick(),
        patience_ticks: AwaitedRestart::PATIENCE_TICKS,
        settled_tick: Some(
            context
                .current_tick()
                .saturating_sub(PassOriginRestart::ThrowIn.set_routine_ticks()),
        ),
        hold: None,
    });
    taker
}

#[test]
fn only_a_side_ahead_late_and_told_to_slows_its_restarts() {
    let (field, mut context) = kickoff();
    let home = field.home_team_id;
    assert_eq!(
        context.time_wasting_delay_ms(home, 10.0, TimeWastingRestart::ThrowIn),
        0
    );

    home_ahead_late(&mut context, CoachInstruction::Normal);
    assert_eq!(
        context.time_wasting_delay_ms(home, 10.0, TimeWastingRestart::ThrowIn),
        0
    );

    context.coach_home.instruction = CoachInstruction::WasteTime;
    let full = context.time_wasting_delay_ms(home, 10.0, TimeWastingRestart::ThrowIn);
    context.coach_home.instruction = CoachInstruction::SlowDown;
    let half = context.time_wasting_delay_ms(home, 10.0, TimeWastingRestart::ThrowIn);
    assert!(
        full > 0 && half > 0 && half < full,
        "full {full}, half {half}"
    );

    let away = field.away_team_id;
    assert_eq!(
        context.time_wasting_delay_ms(away, 10.0, TimeWastingRestart::ThrowIn),
        0
    );
}

#[test]
fn a_wasted_throw_in_is_held_as_delay_and_given_back() {
    let (mut field, mut context) = kickoff();
    home_ahead_late(&mut context, CoachInstruction::WasteTime);
    home_throw_in_ready(&mut field, &context);

    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(4);
    field
        .ball
        .tick_awaited_restart(&mut context, &players, &mut events);

    let restart = field.ball.awaiting_restart.expect("not taken yet");
    let now = context.current_tick();
    assert_eq!(
        field.ball.playing_time(now + 1),
        PlayingTime::Dead(DeadTime::Delay)
    );
    assert!(restart.is_held(now + 1));
    assert!(context.referee.add_back(DeadTime::Delay) > 0.0);
}

#[test]
fn persistent_time_wasting_is_booked_and_a_little_is_not() {
    let (field, mut context) = kickoff();
    let home = field.home_team_id;
    context.rng = MatchRng::from_seed(0xB00C);
    context.referee.strictness = 1.0;
    for _ in 0..4 {
        assert!(!context.time_wasting_caution(home, 10_000));
    }
    let booked = (0..60).any(|_| context.time_wasting_caution(home, 10_000));
    assert!(booked);
}

#[test]
fn a_keeper_past_the_count_concedes_a_corner_to_the_other_side() {
    let (mut field, context) = kickoff();
    let keeper = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left) && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .unwrap();
    field.ball.position = Vector3::new(30.0, 272.0, 1.1);
    field.ball.current_owner = Some(keeper);
    field.ball.held_in_hands = true;

    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(4);
    field
        .ball
        .concede_corner_for_holding(keeper, &context, &players, &mut events);

    let restart = field.ball.awaiting_restart.expect("a corner is awarded");
    assert_eq!(restart.origin, PassOriginRestart::Corner);
    assert_eq!(
        field.get_player(restart.taker_id).unwrap().side,
        Some(PlayerSide::Right)
    );
}

// ── Handball and dissent ──────────────────────────────────────────────

#[test]
fn an_arm_at_shoulder_height_is_likelier_and_a_good_reader_keeps_it_in() {
    let at_arm = Ball::handball_chance(1.3, 10.0);
    let at_feet = Ball::handball_chance(0.2, 10.0);
    assert!(at_arm > at_feet * 4.0);
    assert!(Ball::handball_chance(1.3, 18.0) < Ball::handball_chance(1.3, 4.0));
}

#[test]
fn handball_in_his_own_area_is_a_penalty() {
    let (mut field, mut context) = kickoff();
    let defender = away_outfielder(&field);
    // Right defends the right-hand goal: on its penalty spot line.
    field.get_player_mut(defender).unwrap().position = Vector3::new(800.0, 272.0, 0.0);

    let mut events = EventCollection::with_capacity(4);
    events.add_player_event(PlayerEvent::Handball(defender, true));
    let mut match_data = ResultMatchPositionData::empty();
    EventDispatcher::dispatch(&mut events, &mut field, &mut context, &mut match_data, true);

    let restart = field.ball.awaiting_restart.expect("play is stopped");
    assert_eq!(restart.origin, PassOriginRestart::Penalty);
    assert_eq!(
        context.tally.offences[FoulSource::Handball.index()].whistled,
        1
    );
}

#[test]
fn a_hothead_who_argues_is_booked_for_dissent_more_often() {
    let (field, context) = kickoff();
    let mut calm = field.players[4].clone();
    calm.attributes.temperament = 18.0;
    calm.skills.mental.aggression = 4.0;
    let mut hothead = calm.clone();
    hothead.attributes.temperament = 3.0;
    hothead.skills.mental.aggression = 18.0;
    let mut arguer = hothead.clone();
    arguer.traits.push(PlayerTrait::Argues);

    let chance = |p: &MatchPlayer| Dissent::caution_chance(p, &context.referee);
    assert!(chance(&calm) < chance(&hothead));
    assert!(chance(&hothead) < chance(&arguer));
}
