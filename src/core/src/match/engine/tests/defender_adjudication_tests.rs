//! **One adjudication per contact.** Every contest a defender is part of is
//! decided once, by one rule, and what it decides is carried out by the man
//! it was decided for.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::shared::{DefenderRead, ReadLevel};
use crate::r#match::engine::ball::ball::flight::roll::BallRoll;
use crate::r#match::engine::ball::ball::{AerialDelivery, AerialOutcome, Ball};
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::result::Score;
use crate::r#match::events::EventCollection;
use crate::r#match::player::events::{PlayerEvent, PlayerEventDispatcher};
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::passing::CrossType;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, MatchRng, PlayerSide,
    ResultMatchPositionData, StateProcessingContext,
};
use nalgebra::Vector3;

const HOME: u32 = 1;
const AWAY: u32 = 2;
const DL: u32 = 101;
const DCR: u32 = 103;
const AWAY_ML: u32 = 205;
const AWAY_FL: u32 = 209;

/// Home defending x = 0, ten minutes in, everyone parked out of the way.
fn pitch() -> (MatchField, MatchContext) {
    let home = squad(HOME, 100);
    let away = squad(AWAY, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(840, 545, home, away);
    assert_eq!(field.side_of(HOME), PlayerSide::Left);
    field.ball.kickoff_taker = None;
    for (index, p) in field.players.iter_mut().enumerate() {
        p.velocity = Vector3::zeros();
        let x = if p.team_id == HOME { 500.0 } else { 700.0 };
        p.position = Vector3::new(x, 20.0 + (index % 11) as f32 * 48.0, 0.0);
    }
    let mut context = MatchContext::new(&field, players, Score::new(HOME, AWAY), false, false);
    context.total_match_time = 600_000;
    (field, context)
}

fn stand(field: &mut MatchField, id: u32, x: f32, y: f32) {
    let p = field.get_player_mut(id).unwrap();
    p.position = Vector3::new(x, y, 0.0);
    p.velocity = Vector3::zeros();
}

/// A floated cross from the away left dropping at head height onto an away
/// forward in the home area, a home centre-back on him.
fn cross_into_the_area() -> (MatchField, MatchContext) {
    let (mut field, context) = pitch();
    stand(&mut field, AWAY_ML, 200.0, 60.0);
    stand(&mut field, AWAY_FL, 60.0, 268.0);
    stand(&mut field, DCR, 56.0, 276.0);
    let ball = &mut field.ball;
    ball.current_owner = None;
    ball.previous_owner = Some(AWAY_ML);
    ball.pass_target_player_id = Some(AWAY_FL);
    ball.position = Vector3::new(62.0, 270.0, 2.2);
    ball.velocity = Vector3::new(-0.6, 0.05, -0.03);
    ball.pending_cross_type = Some(CrossType::FloatedFarPost);
    ball.pending_pass_was_cross = true;
    ball.cross_contest_resolved = false;
    ball.aerial_decided = false;
    ball.flags.in_flight_state = 60;
    (field, context)
}

#[test]
fn a_cross_is_contested_while_nothing_has_decided_it() {
    let (mut field, mut context) = cross_into_the_area();
    let mut events = EventCollection::new();
    FootballEngine::<840, 545>::resolve_cross_contest(&mut field, &mut context, &mut events);
    assert!(
        field.ball.aerial_decided,
        "the contest did not decide the flight"
    );
}

#[test]
fn a_cross_the_duel_has_decided_is_not_contested_again() {
    let (mut field, mut context) = cross_into_the_area();
    let mut events = EventCollection::new();
    field
        .ball
        .try_aerial_duel(&context, &field.players, &mut events);
    assert!(
        field.ball.aerial_decided,
        "the duel did not decide the flight"
    );
    let (position, velocity) = (field.ball.position, field.ball.velocity);
    let mut contest = EventCollection::new();
    FootballEngine::<840, 545>::resolve_cross_contest(&mut field, &mut context, &mut contest);
    assert!(!contest.has_events(), "the contest decided it again");
    assert_eq!(field.ball.position, position);
    assert_eq!(field.ball.velocity, velocity);
}

// ── A header a defender has won is played by him, once ──────────────────

/// A cross the contest has given the home centre-back to clear: dropping
/// onto him from 3 m away, delivered for the clearance it decided.
fn awarded_cross(seed: u64) -> (MatchField, MatchContext) {
    let (mut field, mut context) = cross_into_the_area();
    context.rng = MatchRng::from_seed(seed);
    let head = field.get_player(DCR).unwrap().position + Vector3::new(0.0, 0.0, 2.3);
    let ball = &mut field.ball;
    ball.position = head + Vector3::new(24.0, 0.0, 0.6);
    ball.velocity = Vector3::new(-0.8, 0.0, -0.03);
    ball.pass_target_player_id = None;
    ball.cross_contest_resolved = true;
    ball.aerial_decided = true;
    ball.aerial_delivery = Some(AerialDelivery {
        winner_id: DCR,
        target: head,
        outcome: AerialOutcome::Cleared {
            attacked_goal: Vector3::new(0.0, 272.0, 0.0),
            range: 240.0,
            apex: 6.0,
            turn: 0.0,
        },
        arrival_height: 2.4,
        deadline_tick: context.current_tick() + 200,
        force_heading: false,
    });
    field.get_player_mut(DCR).unwrap().on_aerial_award();
    (field, context)
}

fn engine_tick(field: &mut MatchField, context: &mut MatchContext) {
    let mut data = ResultMatchPositionData::empty();
    let mut tick = GameTickContext::new(field, &context.players);
    FootballEngine::<840, 545>::game_tick(field, context, &mut data, &mut tick);
    context.increment_time();
}

#[test]
fn a_defender_awarded_a_cross_goes_up_for_it_from_every_state() {
    for state in [
        DefenderState::Standing,
        DefenderState::Walking,
        DefenderState::HoldingLine,
        DefenderState::Returning,
        DefenderState::TrackingBack,
        DefenderState::Marking,
        DefenderState::Covering,
        DefenderState::Pressing,
        DefenderState::Jockeying,
        DefenderState::Intercepting,
    ] {
        let (mut field, context) = awarded_cross(0);
        field
            .get_player_mut(DCR)
            .unwrap()
            .transition_to(PlayerState::Defender(state), TransitionSource::Reset);
        let tick = GameTickContext::new(&field, &context.players);
        let ctx = StateProcessingContext {
            in_state_time: 40,
            player: field.get_player(DCR).unwrap(),
            context: &context,
            tick_context: &tick,
        };
        assert_eq!(
            DefenderRead::above(&ctx, ReadLevel::LooseBall),
            Some(DefenderState::Heading),
            "from {state:?}"
        );
    }
}

/// Every seed: he is up, in `Heading`, when the ball comes off his head,
/// it goes away from his goal, and no shot is his.
#[test]
fn the_winner_heads_it_himself_off_the_ground_and_it_is_a_clearance() {
    for seed in 0..40u64 {
        let (mut field, mut context) = awarded_cross(seed);
        let mut headed = None;
        for _ in 0..80 {
            let (state, up) = {
                let p = field.get_player(DCR).unwrap();
                (p.state, p.is_airborne())
            };
            engine_tick(&mut field, &mut context);
            if field.ball.aerial_delivery.is_none() {
                headed = Some((state, up));
                break;
            }
        }
        let (state, up) = headed.unwrap_or_else(|| panic!("seed {seed}: never headed"));
        assert_eq!(
            state,
            PlayerState::Defender(DefenderState::Heading),
            "seed {seed}"
        );
        assert!(up, "seed {seed}: headed flat-footed");
        assert!(
            field.ball.velocity.x > 0.0,
            "seed {seed}: not away from his goal"
        );
        assert_eq!(
            field.get_player(DCR).unwrap().memory.shots_taken,
            0,
            "seed {seed}"
        );
    }
}

// ── A ball taken off a carrier is a tackle ──────────────────────────────

#[test]
fn a_ball_taken_off_a_carrier_is_his_tackle_whatever_took_it() {
    let (mut field, mut context) = pitch();
    stand(&mut field, AWAY_FL, 150.0, 270.0);
    stand(&mut field, DCR, 146.0, 270.0);
    let mut data = ResultMatchPositionData::empty();
    PlayerEventDispatcher::dispatch(
        PlayerEvent::Dispossessed(AWAY_FL, DCR),
        &mut field,
        &mut context,
        &mut data,
    );
    assert_eq!(field.get_player(DCR).unwrap().statistics.tackles, 1);

    // A team-mate's ball is not a tackle.
    PlayerEventDispatcher::dispatch(
        PlayerEvent::Dispossessed(DCR - 1, DCR),
        &mut field,
        &mut context,
        &mut data,
    );
    assert_eq!(field.get_player(DCR).unwrap().statistics.tackles, 1);
}

// ── A carried ball is taken by a challenge ──────────────────────────────

/// The away forward on the ball, long settled on it, carrying at `carry` on
/// every attribute of the carry; a home centre-back who tackles at 16 half a
/// metre from him.
fn at_close_quarters(carry: f32) -> (MatchField, MatchContext) {
    let (mut field, context) = pitch();
    stand(&mut field, AWAY_FL, 300.0, 270.0);
    stand(&mut field, DCR, 304.0, 270.0);
    let carrier = field.get_player_mut(AWAY_FL).unwrap();
    let skills = &mut carrier.skills;
    for attribute in [
        &mut skills.technical.dribbling,
        &mut skills.technical.technique,
        &mut skills.mental.flair,
        &mut skills.physical.agility,
        &mut skills.physical.acceleration,
        &mut skills.physical.balance,
        &mut skills.mental.composure,
        &mut skills.mental.decisions,
    ] {
        *attribute = carry;
    }
    let skills = &mut field.get_player_mut(DCR).unwrap().skills;
    for attribute in [
        &mut skills.technical.tackling,
        &mut skills.mental.positioning,
        &mut skills.mental.anticipation,
        &mut skills.technical.marking,
    ] {
        *attribute = 16.0;
    }
    let ball = &mut field.ball;
    ball.position = Vector3::new(300.0, 270.0, 0.0);
    ball.velocity = Vector3::zeros();
    ball.current_owner = Some(AWAY_FL);
    ball.previous_owner = None;
    ball.pass_target_player_id = None;
    ball.ownership_duration = 200;
    ball.claim_cooldown = 0;
    ball.flags.in_flight_state = 0;
    (field, context)
}

#[test]
fn standing_close_is_not_winning_it() {
    let (mut field, context) = at_close_quarters(10.0);
    field.get_player_mut(DCR).unwrap().tackle_cooldown = 1_000;
    for _ in 0..100 {
        let mut events = EventCollection::new();
        field
            .ball
            .process_ownership(&context, &field.players, &mut events);
        assert_eq!(
            field.ball.current_owner,
            Some(AWAY_FL),
            "taken by standing there"
        );
    }
}

/// Scanned until the man at his feet has made his challenge: who has the
/// ball after it.
#[test]
fn the_carriers_skill_counts() {
    let kept = |carry: f32| {
        (0..400u64)
            .filter(|seed| {
                let (mut field, mut context) = at_close_quarters(carry);
                context.rng = MatchRng::from_seed(*seed);
                // A moment worth going in for: a carrier on the move, a
                // team-mate covering behind, a defender with some bite.
                stand(&mut field, DL, 250.0, 280.0);
                field.get_player_mut(AWAY_FL).unwrap().velocity = Vector3::new(0.4, 0.0, 0.0);
                let defender = field.get_player_mut(DCR).unwrap();
                defender.contact_ticks = 10;
                defender.skills.mental.aggression = 14.0;
                defender.skills.mental.decisions = 12.0;
                for _ in 0..6_000 {
                    let mut events = EventCollection::new();
                    field
                        .ball
                        .process_ownership(&context, &field.players, &mut events);
                    if field.ball.current_owner != Some(AWAY_FL) {
                        return false;
                    }
                    if events.has_events() {
                        return true;
                    }
                }
                panic!("seed {seed}: no challenge in 60 s of contact");
            })
            .count()
    };
    let (poor, excellent) = (kept(4.0), kept(19.0));
    assert!(poor < 400, "the challenge was never won");
    assert!(
        excellent > poor,
        "excellent {excellent} vs poor {poor} of 400 kept"
    );
}

// ── Where a ball a defender does not keep comes to rest ─────────────────

#[test]
fn a_blocked_shot_comes_to_rest_where_its_outcome_says() {
    let rng = MatchRng::from_seed(7);
    for (band, metres) in [
        (Ball::LOOSE_REST, (3.0, 8.0)),
        (Ball::SIDEWAYS_REST, (8.0, 20.0)),
    ] {
        for _ in 0..200 {
            let rest = BallRoll::range(Ball::rest_speed(band, 4.0, &rng)) * 0.125;
            assert!(
                rest >= metres.0 - 0.05 && rest <= metres.1 + 0.05,
                "{rest} m outside {metres:?}"
            );
        }
    }
    // A weak shot gains nothing from the block.
    assert!(Ball::rest_speed(Ball::SIDEWAYS_REST, 0.3, &rng) <= 0.3);
}

#[test]
fn a_tackles_knock_comes_to_rest_two_to_eight_metres_away() {
    let (mut field, _) = pitch();
    stand(&mut field, DCR, 300.0, 270.0);
    let tackler = field.get_player(DCR).unwrap().clone();
    let mut knocked = 0;
    for seed in 0..400u64 {
        let rng = MatchRng::from_seed(seed);
        let ball = &mut field.ball;
        ball.position = Vector3::new(314.0, 270.0, 0.0);
        ball.velocity = Vector3::zeros();
        ball.current_owner = Some(AWAY_FL);
        if ball.on_tackle_won(&tackler, 0.0, Vector3::new(0.4, 0.0, 0.0), &rng) {
            continue;
        }
        knocked += 1;
        let rest = BallRoll::range(ball.velocity.xy().norm()) * 0.125;
        assert!(
            (1.95..=8.05).contains(&rest),
            "seed {seed}: rests {rest} m away"
        );
    }
    assert!(knocked > 0, "never knocked loose");
}

/// A cross that comes at his head, not over it: he still attacks it on the
/// jump.
#[test]
fn a_cross_at_head_height_is_still_headed_off_the_ground() {
    for seed in 0..20u64 {
        let (mut field, mut context) = awarded_cross(seed);
        let head = field.get_player(DCR).unwrap().position + Vector3::new(0.0, 0.0, 1.7);
        field.ball.position = head + Vector3::new(24.0, 0.0, 0.2);
        if let Some(delivery) = field.ball.aerial_delivery.as_mut() {
            delivery.target = head;
            delivery.arrival_height = 1.9;
        }
        let mut up_when_headed = None;
        for _ in 0..80 {
            let up = field.get_player(DCR).unwrap().is_airborne();
            engine_tick(&mut field, &mut context);
            if field.ball.aerial_delivery.is_none() {
                up_when_headed = Some(up);
                break;
            }
        }
        assert_eq!(up_when_headed, Some(true), "seed {seed}");
    }
}
