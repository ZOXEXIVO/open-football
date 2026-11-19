//! **The goalkeeper and the Laws around him.** A clearance is booked to the
//! man who struck it, hands are used only where Law 12 allows, the count is
//! the hands clock, an injured keeper saves nothing, a keeper's foul in a
//! one-on-one is judged as one, and a replacement keeper plays as a keeper.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use super::keeper_save_contact_tests::rest_point;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::player::injury::{InjuryCause, InjuryGrade, MatchInjury};
use crate::r#match::engine::result::Score;
use crate::r#match::engine::substitutions::Substitutions;
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{KeeperSmother, SmotherOutcome};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::goalkeepers::states::{GoalkeeperDivingState, GoalkeeperHoldingState};
use crate::r#match::player::events::{
    FoulCommitted, FoulSeverity, FoulSource, PlayerEvent, PlayerEventDispatcher,
};
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayer, MatchPlayerCollection, PlayerSide,
    ResultMatchPositionData, ShotTarget, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, events::EventCollection,
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

fn keeper_of(field: &MatchField, side: PlayerSide) -> u32 {
    field
        .players
        .iter()
        .find(|p| {
            p.side == Some(side)
                && !p.off_pitch
                && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .expect("every side has a goalkeeper")
}

fn outfielder_of(field: &MatchField, side: PlayerSide) -> u32 {
    field
        .players
        .iter()
        .find(|p| p.side == Some(side) && !p.tactical_position.current_position.is_goalkeeper())
        .map(|p| p.id)
        .expect("every side has outfielders")
}

fn player_mut(field: &mut MatchField, id: u32) -> &mut MatchPlayer {
    field
        .players
        .iter_mut()
        .find(|p| p.id == id)
        .expect("player on the field")
}

fn clearances(field: &MatchField, id: u32) -> u16 {
    field
        .players
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.statistics.clearances)
        .expect("player on the field")
}

fn clear(field: &mut MatchField, context: &mut MatchContext, by: u32, velocity: Vector3<f32>) {
    let mut data = ResultMatchPositionData::new();
    PlayerEventDispatcher::dispatch(
        PlayerEvent::ClearBall(by, velocity),
        field,
        context,
        &mut data,
    );
}

/// The away keeper defends the goal at x = 840; the home side attacks it.
#[test]
fn a_smother_knock_away_goes_away_from_his_goal_and_is_his() {
    let (mut field, mut context) = kickoff();
    let keeper = keeper_of(&field, PlayerSide::Right);
    let striker = outfielder_of(&field, PlayerSide::Left);
    player_mut(&mut field, keeper).position = Vector3::new(800.0, 272.0, 0.0);
    player_mut(&mut field, striker).position = Vector3::new(792.0, 272.0, 0.0);
    field.ball.position = Vector3::new(794.0, 272.0, 0.0);
    field.ball.current_owner = Some(striker);

    clear(
        &mut field,
        &mut context,
        keeper,
        Vector3::new(-1.2, 0.5, 0.02),
    );

    assert!(
        field.ball.velocity.x < 0.0,
        "the knock-away was turned toward his own goal: vx {:.2}",
        field.ball.velocity.x
    );
    assert_eq!(field.ball.last_touch_player_id, Some(keeper));
    assert_eq!(clearances(&field, keeper), 1);
    assert_eq!(clearances(&field, striker), 0);
}

#[test]
fn a_punch_is_guarded_and_booked_to_the_keeper() {
    let (mut field, mut context) = kickoff();
    let keeper = keeper_of(&field, PlayerSide::Right);
    let punching = player_mut(&mut field, keeper);
    punching.position = Vector3::new(820.0, 272.0, 0.0);
    punching.transition_to(
        PlayerState::Goalkeeper(GoalkeeperState::Punching),
        TransitionSource::EventHandler,
    );
    field.ball.position = Vector3::new(815.0, 272.0, 2.5);
    field.ball.current_owner = None;

    clear(
        &mut field,
        &mut context,
        keeper,
        Vector3::new(0.8, 0.3, 0.5),
    );

    assert!(
        field.ball.velocity.x < 0.0,
        "the punch went toward his own goal: vx {:.2}",
        field.ball.velocity.x
    );
    assert_eq!(field.ball.last_touch_player_id, Some(keeper));
    assert_eq!(clearances(&field, keeper), 1);
}

/// The away keeper smothers at a striker's feet, his parry-control skills
/// all at `skill`, until he gets to the ball and cannot hold it: where he
/// met it, and where it came to rest.
fn knock_away_rests(skill: f32) -> Vec<(Vector3<f32>, Vector3<f32>)> {
    (0..40)
        .filter_map(|_| {
            let (mut field, mut context) = kickoff();
            let keeper = keeper_of(&field, PlayerSide::Right);
            let striker = outfielder_of(&field, PlayerSide::Left);
            for player in field.players.iter_mut() {
                player.position = if player.id == keeper {
                    Vector3::new(800.0, 272.0, 0.0)
                } else if player.id == striker {
                    Vector3::new(792.0, 272.0, 0.0)
                } else {
                    Vector3::new(400.0, 40.0, 0.0)
                };
            }
            let skills = &mut player_mut(&mut field, keeper).skills;
            skills.goalkeeping.handling = skill;
            skills.goalkeeping.punching = skill;
            skills.goalkeeping.reflexes = skill;
            skills.mental.decisions = skill;
            skills.mental.composure = skill;
            skills.physical.strength = skill;
            skills.physical.balance = skill;
            field.ball.position = Vector3::new(794.0, 272.0, 0.0);
            field.ball.current_owner = Some(striker);

            let (met, away) = {
                let tick = GameTickContext::new(&field, &context.players);
                let ctx = StateProcessingContext {
                    in_state_time: 30,
                    player: field
                        .players
                        .iter()
                        .find(|p| p.id == keeper)
                        .expect("the keeper"),
                    context: &context,
                    tick_context: &tick,
                };
                let attempt =
                    KeeperSmother::assess(&ctx).expect("a striker on top of him is his to smother");
                let away =
                    (0..1000).find_map(|_| match KeeperSmother::resolve(&ctx, &attempt) {
                        SmotherOutcome::Blocked(away) => Some(away),
                        _ => None,
                    })?;
                (attempt.ball, away)
            };
            clear(&mut field, &mut context, keeper, away);
            rest_point(&mut field, &mut context).map(|rest| (met, rest))
        })
        .collect()
}

#[test]
fn a_smother_knock_away_comes_to_rest_within_five_metres_away_from_his_goal() {
    for skill in [1.0, 20.0] {
        let rests = knock_away_rests(skill);
        assert!(
            rests.len() >= 30,
            "only {} knock-aways at skill {skill}",
            rests.len()
        );
        for (met, rest) in rests {
            let metres = (rest - met).xy().norm() / 8.0;
            assert!(
                metres <= 5.0,
                "at skill {skill} a knock-away came to rest {metres:.2} m from where he met it"
            );
            assert!(
                rest.x < met.x,
                "at skill {skill} a knock-away came to rest goal-side of him: x {:.1} from {:.1}",
                rest.x,
                met.x
            );
        }
    }
}

/// How far each flap came to rest from where he touched it, in metres: the
/// away keeper meeting a cross at the heights he takes them, at the edges
/// of the push the contest gives a flap.
fn flap_rests() -> Vec<f32> {
    let (out_near, out_far) = FootballEngine::<840, 545>::FLAP_OUT;
    let across = FootballEngine::<840, 545>::FLAP_ACROSS;
    let mut rests = Vec::new();
    for out in [out_near, (out_near + out_far) * 0.5, out_far] {
        for lateral in [-across, 0.0, across] {
            for height in [1.6, 2.4, 3.0] {
                let (mut field, mut context) = kickoff();
                let keeper = keeper_of(&field, PlayerSide::Right);
                player_mut(&mut field, keeper).position = Vector3::new(820.0, 272.0, 0.0);
                field.ball.position = Vector3::new(819.0, 272.0, height);
                field.ball.velocity = Vector3::new(1.2, 0.4, -0.01);
                field.ball.current_owner = None;
                let touched = field.ball.position;
                let gk = field
                    .players
                    .iter()
                    .find(|p| p.id == keeper)
                    .expect("the keeper")
                    .clone();
                field.ball.flap_off(&gk, out, lateral);
                if let Some(rest) = rest_point(&mut field, &mut context) {
                    rests.push((rest - touched).xy().norm() / 8.0);
                }
            }
        }
    }
    rests
}

#[test]
fn a_flapped_cross_comes_to_rest_within_ten_metres_of_where_he_touched_it() {
    let rests = flap_rests();
    assert_eq!(rests.len(), 27, "a flap ran out of play");
    for metres in rests {
        assert!(
            metres <= 10.0,
            "a flap came to rest {metres:.2} m from where he touched it"
        );
    }
}

/// One keeper's decision in a given state, the way the state machine asks it.
fn decide(
    field: &MatchField,
    context: &MatchContext,
    id: u32,
    in_state_time: u64,
    handler: &impl StateProcessingHandler,
) -> Option<StateChangeResult> {
    let tick = GameTickContext::new(field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time,
        player: field.players.iter().find(|p| p.id == id).expect("a player"),
        context,
        tick_context: &tick,
    };
    handler.process(&ctx)
}

fn player_events(mut change: StateChangeResult) -> Vec<PlayerEvent> {
    change
        .events
        .drain()
        .filter_map(|event| match event {
            Event::PlayerEvent(e) => Some(e),
            _ => None,
        })
        .collect()
}

/// The away keeper's area runs from x = 708 to the line at 840.
#[test]
fn a_dive_at_a_loose_ball_outside_the_area_is_never_a_catch() {
    let (mut field, context) = kickoff();
    let keeper = keeper_of(&field, PlayerSide::Right);
    let diver = player_mut(&mut field, keeper);
    diver.position = Vector3::new(650.0, 272.0, 0.0);
    diver.transition_to(
        PlayerState::Goalkeeper(GoalkeeperState::Diving),
        TransitionSource::EventHandler,
    );
    // In the air: on the ground the dive is over and he is getting up.
    diver.height = 0.4;
    field.ball.position = Vector3::new(652.0, 272.0, 0.3);
    field.ball.velocity = Vector3::new(-0.1, 0.0, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = None;

    let mut feet = 0;
    for _ in 0..400 {
        let Some(change) = decide(
            &field,
            &context,
            keeper,
            30,
            &GoalkeeperDivingState::default(),
        ) else {
            continue;
        };
        let events = player_events(change);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, PlayerEvent::CaughtBall(_))),
            "a dive outside his area took the ball in his hands"
        );
        if events.iter().any(|e| matches!(e, PlayerEvent::GainBall(_))) {
            feet += 1;
        }
    }
    assert!(
        feet > 0,
        "he never got to the ball at all, so nothing was tested"
    );
}

/// A shot straight at the away keeper, a tick short of his plane, and
/// everybody else out of the lane.
fn shot_at(field: &mut MatchField, keeper: u32, keeper_x: f32) {
    for player in field.players.iter_mut() {
        player.position = if player.id == keeper {
            Vector3::new(keeper_x, 272.0, 0.0)
        } else {
            Vector3::new(60.0, 40.0, 0.0)
        };
    }
    let struck_from = Vector3::new(keeper_x - 90.0, 272.0, 0.0);
    field.ball.position = Vector3::new(keeper_x - 3.0, 272.0, 0.5);
    field.ball.velocity = Vector3::new(2.2, 0.0, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(outfielder_of(field, PlayerSide::Left));
    field.ball.flags.in_flight_state = 60;
    field.ball.held_in_hands = false;
    field.ball.pending_save_credit = None;
    field.ball.cached_shot_target = Some(ShotTarget {
        goal_line_y: 272.0,
        goal_line_z: 0.5,
        defending_side: PlayerSide::Right,
        save_rolled: false,
        block_rolled: true,
        blocked_by: None,
        deflected: false,
        shooter_threat: 0.5,
        struck_from,
        seen_from: struck_from,
    });
}

/// Saves the physics made with his hands in 200 shots at a keeper on `x`.
fn hand_saves_at(keeper_x: f32, injured: bool) -> usize {
    let (mut field, context) = kickoff();
    let keeper = keeper_of(&field, PlayerSide::Right);
    if injured {
        player_mut(&mut field, keeper)
            .transition_to(PlayerState::Injured, TransitionSource::EventHandler);
    }
    let mut saves = 0;
    for _ in 0..200 {
        shot_at(&mut field, keeper, keeper_x);
        let players = field.players.clone();
        let mut events = EventCollection::with_capacity(8);
        field.ball.try_save_shot(&context, &players, &mut events);
        if field.ball.pending_save_credit.is_some()
            || field.ball.held_in_hands
            || field.ball.current_owner == Some(keeper)
        {
            saves += 1;
        }
    }
    saves
}

#[test]
fn a_shot_met_outside_the_area_is_no_catch_no_parry_and_no_tip() {
    assert_eq!(
        hand_saves_at(690.0, false),
        0,
        "he used his hands outside his area"
    );
    assert!(
        hand_saves_at(800.0, false) > 0,
        "the same shot inside his area is never saved, so nothing was tested"
    );
}

#[test]
fn a_keeper_down_injured_saves_nothing() {
    assert_eq!(
        hand_saves_at(800.0, true),
        0,
        "a keeper lying injured made a save"
    );
}

/// The away keeper brings down a home striker who is through, nobody else
/// between them and the goal at x = 840, and the ball runs loose. Re-staged
/// until the referee blows for it; returns the keeper's yellow and red
/// cards and whether he conceded a penalty.
fn one_on_one_foul(keeper_x: f32) -> (u16, u16, u16) {
    for _ in 0..400 {
        let (mut field, mut context) = kickoff();
        let keeper = keeper_of(&field, PlayerSide::Right);
        let striker = outfielder_of(&field, PlayerSide::Left);
        for player in field.players.iter_mut() {
            player.position = if player.id == keeper {
                Vector3::new(keeper_x, 272.0, 0.0)
            } else if player.id == striker {
                Vector3::new(keeper_x - 5.0, 272.0, 0.0)
            } else {
                Vector3::new(400.0, 40.0, 0.0)
            };
        }
        field.ball.position = Vector3::new(keeper_x - 4.0, 272.0, 0.0);
        field.ball.current_owner = None;
        let mut data = ResultMatchPositionData::new();
        PlayerEventDispatcher::dispatch(
            PlayerEvent::CommitFoul(FoulCommitted {
                fouler: keeper,
                fouled: striker,
                severity: FoulSeverity::Normal,
                source: FoulSource::KeeperSmother,
                challenge: None,
            }),
            &mut field,
            &mut context,
            &mut data,
        );
        if context.pending_advantage.is_some() {
            continue;
        }
        let gk = field
            .players
            .iter()
            .find(|p| p.id == keeper)
            .expect("the keeper");
        if gk.fouls_committed > 0 {
            return (
                gk.statistics.yellow_cards_count(),
                gk.statistics.red_cards_count(),
                gk.statistics.zone_stats.penalty_fouls_conceded,
            );
        }
    }
    panic!("the referee never blew for it");
}

#[test]
fn a_keeper_who_brings_down_a_man_through_in_his_area_concedes_a_penalty_and_a_caution() {
    let (yellow, red, penalty) = one_on_one_foul(790.0);
    assert_eq!((yellow, red), (1, 0), "inside his area it is a caution");
    assert_eq!(penalty, 1, "and a penalty kick");
}

#[test]
fn a_keeper_who_brings_down_a_man_through_outside_his_area_is_sent_off() {
    let (yellow, red, penalty) = one_on_one_foul(690.0);
    assert_eq!(
        (yellow, red),
        (0, 1),
        "outside his area it is a sending-off"
    );
    assert_eq!(penalty, 0, "and a free kick, not a penalty");
}

/// The home keeper is sent off; `repair_missing_goalkeeper` puts somebody
/// in goal. Returns the man now in goal.
fn keeper_sent_off(field: &mut MatchField, context: &mut MatchContext) -> MatchPlayer {
    let keeper = keeper_of(field, PlayerSide::Left);
    let team = field.home_team_id;
    field.take_off(keeper, context.coach_for_team(team).spare_line());
    Substitutions::process_medical(field, context);
    field
        .players
        .iter()
        .find(|p| {
            p.team_id == team
                && !p.off_pitch
                && p.tactical_position.current_position.is_goalkeeper()
        })
        .cloned()
        .expect("somebody is in goal")
}

#[test]
fn a_bench_keeper_brought_on_after_a_red_card_plays_as_a_keeper() {
    let (mut field, mut context) = super::substitution_break_tests::kickoff();
    let in_goal = keeper_sent_off(&mut field, &mut context);
    assert!(
        matches!(in_goal.state, PlayerState::Goalkeeper(_)),
        "the bench keeper is in {} from his first tick",
        in_goal.state
    );
}

#[test]
fn an_emergency_keeper_plays_as_a_keeper_and_leaves_his_station() {
    let (mut field, mut context) = super::substitution_break_tests::kickoff();
    let home = field.home_team_id;
    field.substitutes.retain(|p| p.team_id != home);
    let penalty_station = Vector3::new(160.0, 272.0, 0.0);
    for player in field.players.iter_mut().filter(|p| p.team_id == home) {
        player.set_piece_station = Some(penalty_station);
    }
    let in_goal = keeper_sent_off(&mut field, &mut context);
    assert!(
        matches!(in_goal.state, PlayerState::Goalkeeper(_)),
        "the emergency keeper is in {} from his first tick",
        in_goal.state
    );
    assert_eq!(
        in_goal.set_piece_station, None,
        "he keeps a penalty station that `SetPieceHold` would walk him out of goal to"
    );
}

/// The home keeper (goal at x = 0) is sent off and an outfielder goes in
/// goal; an away shot then goes wide.
#[test]
fn a_sent_off_keeper_takes_no_goal_kick() {
    let (mut field, mut context) = kickoff();
    let sent_off = keeper_of(&field, PlayerSide::Left);
    let team = field.home_team_id;
    field.take_off(sent_off, context.coach_for_team(team).spare_line());
    Substitutions::process_medical(&mut field, &mut context);
    let in_goal = keeper_of(&field, PlayerSide::Left);
    assert_ne!(in_goal, sent_off, "nobody went in goal");

    let shooter = outfielder_of(&field, PlayerSide::Right);
    let away = field.away_team_id;
    field.ball.position = Vector3::new(-1.8, 150.0, 0.30);
    field.ball.velocity = Vector3::new(-2.4, -0.9, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(shooter);
    field
        .ball
        .record_touch(shooter, away, context.current_tick(), true);
    let players = field.players.clone();
    let mut events = EventCollection::with_capacity(4);
    field
        .ball
        .check_wide_of_goal(&context, &players, &mut events);

    assert_eq!(
        field.ball.awaiting_restart.map(|r| r.taker_id),
        Some(in_goal),
        "the goal kick went to the keeper who was sent off"
    );
}

/// Play stops for the away keeper, down injured in his own area.
#[test]
fn an_injured_keeper_is_not_given_the_drop_ball() {
    let (mut field, mut context) = kickoff();
    let keeper = keeper_of(&field, PlayerSide::Right);
    player_mut(&mut field, keeper).position = Vector3::new(820.0, 272.0, 0.0);
    field.ball.position = Vector3::new(812.0, 272.0, 0.0);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = None;

    MatchInjury::befall(
        &mut field,
        &mut context,
        keeper,
        InjuryCause::Contact,
        InjuryGrade::Hurt,
    );

    let taker = field
        .ball
        .awaiting_restart
        .map(|r| r.taker_id)
        .expect("play was stopped for him");
    assert_ne!(taker, keeper, "the drop ball went to the injured keeper");
    let away = field.away_team_id;
    assert!(
        field
            .players
            .iter()
            .any(|p| p.id == taker && p.team_id == away),
        "it went to one of his team-mates"
    );
}

/// He caught it in a dive and lay on it for a second before getting up
/// into `HoldingBall`: the referee's eight seconds started at the catch.
#[test]
fn the_count_runs_from_the_catch_not_from_getting_up() {
    let (mut field, context) = kickoff();
    let keeper = keeper_of(&field, PlayerSide::Right);
    let holder = player_mut(&mut field, keeper);
    holder.position = Vector3::new(820.0, 272.0, 0.0);
    holder.transition_to(
        PlayerState::Goalkeeper(GoalkeeperState::HoldingBall),
        TransitionSource::EventHandler,
    );
    field.ball.position = Vector3::new(820.0, 272.0, 1.0);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(keeper);
    field.ball.held_in_hands = true;
    let now = context.current_tick();
    // `in_state_time` is AI ticks of 20 ms; the hands clock is engine
    // ticks of 10 ms. One second down, then this long in `HoldingBall`.
    let concedes = |field: &mut MatchField, since_catch_ticks: u64| {
        field.ball.hands_since_tick = now - since_catch_ticks;
        let in_state = (since_catch_ticks - 100) / 2;
        decide(
            field,
            &context,
            keeper,
            in_state,
            &GoalkeeperHoldingState::default(),
        )
        .map(player_events)
        .is_some_and(|events| {
            events
                .iter()
                .any(|e| matches!(e, PlayerEvent::HeldTooLong(_)))
        })
    };
    assert!(
        !concedes(&mut field, 798),
        "the corner came before eight seconds in his hands"
    );
    assert!(
        concedes(&mut field, 802),
        "eight seconds in his hands, one of them on the floor, and no corner"
    );
}

/// The home side defends the goal at x = 0.
#[test]
fn a_defenders_clearance_is_unchanged() {
    let (mut field, mut context) = kickoff();
    let defender = outfielder_of(&field, PlayerSide::Left);
    player_mut(&mut field, defender).position = Vector3::new(100.0, 272.0, 0.0);
    field.ball.position = Vector3::new(101.0, 272.0, 0.0);
    field.ball.current_owner = Some(defender);

    clear(
        &mut field,
        &mut context,
        defender,
        Vector3::new(-1.5, 0.4, 0.6),
    );

    assert!(field.ball.velocity.x > 0.0, "turned away from his own goal");
    assert!(
        (field.ball.velocity.x - 1.5).abs() < 1e-4,
        "at the pace he struck it"
    );
    assert_eq!(field.ball.current_owner, None);
    assert_eq!(field.ball.previous_owner, Some(defender));
    assert_eq!(clearances(&field, defender), 1);
}
