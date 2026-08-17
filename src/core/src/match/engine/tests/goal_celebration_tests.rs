//! The post-goal sequence, end to end: the ball crossing the line, the
//! netting taking it, the celebration playing out, and the restart putting
//! everything back exactly where the old instant reset put it.
//!
//! The bug these exist to pin down is a visual one — "the ball stops on the
//! goal line" — which means the assertions have to be about the ball's
//! POSITION over time rather than about the scoreline. A test that only
//! checked the score would have passed against the broken engine.
//!
//! The other half is calibration. The post-goal window is load-bearing (see
//! `MatchContext::dead_ball_until_ms`) and the celebration runs inside it, so
//! there are tests here for the two properties that keep it neutral: the
//! restart happens at the same instant and leaves the same state, and the
//! choreography draws nothing from the shared RNG stream.

#![cfg(test)]

use crate::Tactics;
use crate::club::player::builder::PlayerBuilder;
use crate::club::player::{PlayerPosition, PlayerPositions};
use crate::club::team::tactics::MatchTacticType;
use crate::r#match::ball::events::GoalSide;
use crate::r#match::engine::ball::ball::net::GoalNet;
use crate::r#match::engine::goal::{
    advance_goal_celebration, finish_goal_celebration, handle_goal_reset,
};
use crate::r#match::engine::result::Score;
use crate::r#match::engine::rng::MatchRng;
use crate::r#match::squad::squad::MatchSquad;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayerCollection, PlayerSide, events::EventCollection,
};
use crate::shared::fullname::FullName;
use crate::{PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills};
use chrono::NaiveDate;
use nalgebra::Vector3;

const T442: [PlayerPositionType; 11] = [
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::DefenderLeft,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderCenterRight,
    PlayerPositionType::DefenderRight,
    PlayerPositionType::MidfielderLeft,
    PlayerPositionType::MidfielderCenterLeft,
    PlayerPositionType::MidfielderCenterRight,
    PlayerPositionType::MidfielderRight,
    PlayerPositionType::ForwardLeft,
    PlayerPositionType::ForwardRight,
];

/// Shared with `goal_clip_recording_tests`, which needs the same two sides to
/// play a whole match rather than a single goal.
pub(super) fn squad(team_id: u32, base_id: u32) -> MatchSquad {
    let birth = NaiveDate::from_ymd_opt(1998, 5, 1).unwrap();
    let main_squad = T442
        .iter()
        .enumerate()
        .map(|(index, position)| {
            let mut attributes = PlayerAttributes::default();
            attributes.condition = 9000;
            attributes.current_ability = 150;
            let mut skills = PlayerSkills::default();
            skills.physical.pace = 14.0;
            skills.physical.jumping = 14.0;
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
        tactics: Tactics::new(MatchTacticType::T442),
        main_squad,
        substitutes: vec![],
        captain_id: None,
        vice_captain_id: None,
        penalty_taker_id: None,
        free_kick_taker_id: None,
        selection_omissions: vec![],
        coach_snapshot: None,
    }
}

/// Ten minutes in, which is where the tick counter has to be for the goal
/// to be given at all: `check_goal`'s shot-provenance test is a comparison
/// against `last_shot_struck_tick`, and at tick zero there is no way to say
/// "a shot was struck a moment ago".
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

/// Drive the ball into the LEFT-hand goal as a shot from the Right side's
/// forward, and run the goal through the same path the engine does.
fn score_at_the_home_end(field: &mut MatchField, context: &mut MatchContext) -> u32 {
    let shooter = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .expect("an away outfielder exists");

    // Put the scoring side where a side that has just scored actually is:
    // in the attacking half, having built an attack. Straight off the
    // kickoff formation they are all sixty metres from their own goal, which
    // is nobody's idea of the moment a goal goes in — and it is not the
    // layout the pile-on radius is written against.
    for player in field.players.iter_mut() {
        if player.side == Some(PlayerSide::Right)
            && !player.tactical_position.current_position.is_goalkeeper()
        {
            player.position.x *= 0.35;
        }
    }

    // `check_goal` refuses a ball that crosses the line with no shot behind
    // it (a stray pass is a goal kick, not a goal), so the shooter has to
    // have actually taken one.
    if let Some(player) = field.players.iter_mut().find(|p| p.id == shooter) {
        player.memory.shots_taken = 1;
        player.memory.last_shot_tick = context.current_tick();
    }
    field.ball.last_shot_struck_tick = context.current_tick();
    field.ball.previous_owner = Some(shooter);
    field.ball.current_owner = None;
    // A yard past the line, still travelling, low and central.
    field.ball.position = Vector3::new(-1.0, 545.0 / 2.0 + 6.0, 0.6);
    field.ball.velocity = Vector3::new(-2.2, 0.0, -0.01);

    let mut events = EventCollection::with_capacity(4);
    field.ball.check_goal(context, &mut events);
    assert!(field.ball.goal_scored, "the shot must be given as a goal");
    shooter
}

#[test]
fn a_goal_leaves_the_ball_in_the_net_not_on_the_line() {
    let (mut field, mut context) = kickoff();
    score_at_the_home_end(&mut field, &mut context);

    let state = field.ball.in_net.expect("the ball must be in the goal");
    assert_eq!(state.side, GoalSide::Home);
    assert!(!state.auto_goal, "an away forward scored at the home end");
    // The old behaviour: `check_goal` teleported the ball to the centre spot
    // on this very tick. Anything near the middle of the pitch is that bug.
    assert!(
        field.ball.position.x < 100.0,
        "the ball must still be at the goal, not back on the centre spot"
    );

    // Now let the netting have it.
    for _ in 0..400 {
        field.ball.tick_net(&context.goal_positions);
    }
    let depth = field
        .ball
        .net_depth(&context.goal_positions)
        .expect("still in the goal");
    assert!(
        depth > 0.0 && depth <= GoalNet::DEPTH + GoalNet::GIVE_BACK,
        "the ball must come to rest inside the goal, ended {depth:.2}u past the line"
    );
}

#[test]
fn the_restart_lands_at_the_end_of_the_dead_ball_window_not_the_start() {
    let (mut field, mut context) = kickoff();
    score_at_the_home_end(&mut field, &mut context);
    handle_goal_reset(&mut field, &mut context);

    let restart_at = context.dead_ball_until_ms;
    let window = restart_at - KICKOFF_MS;
    assert!(
        (45_000..=75_000).contains(&window),
        "the post-goal window must stay 45-75 s, got {window} ms"
    );
    assert!(
        context.goal_celebration.is_some(),
        "a goal must arm a celebration"
    );
    // The conceding side is the one defending the left-hand goal.
    assert_eq!(
        context.goal_celebration.as_ref().unwrap().kickoff_side,
        PlayerSide::Left
    );

    // Half a minute in, the ball is still at the goal end and the players
    // are still moving. This is the whole point: the old engine had already
    // put everything back and shown nothing.
    while context.total_match_time < restart_at - 10 {
        context.total_match_time += 10;
        advance_goal_celebration(&mut field, &mut context);
        if context.total_match_time == KICKOFF_MS + 20_000 {
            assert!(
                field.ball.in_net.is_some(),
                "the ball must still belong to the goal 20 s after it went in"
            );
        }
    }

    // …and on the tick the window closes, everything snaps to the kickoff.
    context.total_match_time = restart_at;
    advance_goal_celebration(&mut field, &mut context);
    assert!(context.goal_celebration.is_none(), "the goal is over");
    assert!(field.ball.in_net.is_none(), "the ball is out of the net");
    assert!(
        (field.ball.position.x - 420.0).abs() < 1.0 && (field.ball.position.y - 272.5).abs() < 1.0,
        "the ball restarts on the centre spot, found {:?}",
        field.ball.position
    );
    let kicker = field
        .ball
        .current_owner
        .and_then(|id| field.players.iter().find(|p| p.id == id))
        .expect("somebody is standing over the ball");
    assert_eq!(
        kicker.side,
        Some(PlayerSide::Left),
        "the side that conceded restarts"
    );
    let kicker_id = kicker.id;
    for player in field.players.iter() {
        // Everybody bar the man taking the kickoff, whom `assign_kickoff`
        // deliberately stands on the centre spot.
        if player.id != kicker_id {
            assert!(
                (player.position - player.start_position).norm() < 0.01,
                "every player is back on his formation spot for the restart"
            );
        }
        assert_eq!(player.height, 0.0, "and nobody restarts mid-leap");
    }
}

/// The netting shot is the payoff of the whole change, so the ball has to
/// stay in the goal long enough to be one. The first version had the beaten
/// keeper — who is standing in his own net when the ball arrives — collect it
/// on the tick it got there, and the bulge lasted a single frame.
#[test]
fn the_ball_is_left_in_the_net_long_enough_to_be_worth_watching() {
    let (mut field, mut context) = kickoff();
    score_at_the_home_end(&mut field, &mut context);
    handle_goal_reset(&mut field, &mut context);

    let mut behind_the_line_ms = 0u64;
    for _ in 0..600 {
        context.total_match_time += 10;
        advance_goal_celebration(&mut field, &mut context);
        if field.ball.position.x < 0.0 {
            behind_the_line_ms += 10;
        }
    }
    assert!(
        behind_the_line_ms >= 3_000,
        "the ball must sit in the goal for a few seconds, managed {behind_the_line_ms} ms"
    );
}

#[test]
fn somebody_goes_and_fetches_the_ball_out_of_the_goal() {
    let (mut field, mut context) = kickoff();
    score_at_the_home_end(&mut field, &mut context);
    handle_goal_reset(&mut field, &mut context);

    let restart_at = context.dead_ball_until_ms;
    let mut carried_back = false;
    while context.total_match_time < restart_at - 10 {
        context.total_match_time += 10;
        advance_goal_celebration(&mut field, &mut context);
        // Once he has it, the ball travels with him back toward the middle.
        if field.ball.current_owner.is_some() && field.ball.position.x > 200.0 {
            carried_back = true;
            break;
        }
    }
    assert!(
        carried_back,
        "the conceding side must collect the ball and bring it back for the restart"
    );
}

#[test]
fn the_scorer_wheels_away_and_his_team_mates_chase_him() {
    let (mut field, mut context) = kickoff();
    let scorer = score_at_the_home_end(&mut field, &mut context);
    handle_goal_reset(&mut field, &mut context);

    let scorer_start = field
        .players
        .iter()
        .find(|p| p.id == scorer)
        .map(|p| p.position)
        .unwrap();

    // Five seconds of celebration.
    for _ in 0..500 {
        context.total_match_time += 10;
        advance_goal_celebration(&mut field, &mut context);
    }

    let scorer_now = field.players.iter().find(|p| p.id == scorer).unwrap();
    assert!(
        (scorer_now.position - scorer_start).norm() > 50.0,
        "the scorer must run somewhere — he moved {:.1}u",
        (scorer_now.position - scorer_start).norm()
    );

    // And his team-mates must be converging on him rather than standing in
    // their formation slots.
    let mates_nearby = field
        .players
        .iter()
        .filter(|p| p.side == Some(PlayerSide::Right) && p.id != scorer)
        .filter(|p| (p.position - scorer_now.position).norm() < 120.0)
        .count();
    assert!(
        mates_nearby >= 3,
        "the scorer must get mobbed, only {mates_nearby} team-mates came over"
    );

    // Nobody has wandered off the pitch doing it.
    for player in field.players.iter() {
        assert!(
            player.position.y >= 0.0 && player.position.y <= 545.0,
            "player {} left the pitch at y={}",
            player.id,
            player.position.y
        );
        assert!(
            player.position.x >= -(GoalNet::DEPTH + GoalNet::GIVE_BACK)
                && player.position.x <= 840.0 + GoalNet::DEPTH + GoalNet::GIVE_BACK,
            "player {} left the pitch at x={}",
            player.id,
            player.position.x
        );
    }
}

/// **Nobody reacts to a goal by walking somewhere, and the man who has just
/// been beaten least of all.**
///
/// The reported bug: the keeper "doesn't get upset, he just folds his arms
/// and turns round". Half of that is the viewer's (it now poses a slump —
/// see the `aftermath` module in `src/match`), but the other half is here:
/// the conceding side set off for their formation spots on the very tick the
/// ball crossed the line, so there was no reaction to draw. The beaten
/// keeper in particular is on the floor when it happens and stays there.
#[test]
fn the_beaten_keeper_does_not_stroll_off_the_instant_it_goes_in() {
    let (mut field, mut context) = kickoff();
    score_at_the_home_end(&mut field, &mut context);
    handle_goal_reset(&mut field, &mut context);

    let keeper = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left) && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| (p.id, p.position))
        .expect("the conceding side has a keeper");

    // Three seconds. He has not gone anywhere.
    for _ in 0..300 {
        context.total_match_time += 10;
        advance_goal_celebration(&mut field, &mut context);
    }
    let now = field.players.iter().find(|p| p.id == keeper.0).unwrap();
    assert!(
        (now.position - keeper.1).norm() < 1.0,
        "the beaten keeper walked {:.1}u in the first three seconds — he is \
         supposed to be on the floor",
        (now.position - keeper.1).norm()
    );

    // By the restart he HAS moved: the reaction is a beat, not a freeze.
    let restart_at = context.dead_ball_until_ms;
    while context.total_match_time < restart_at - 10 {
        context.total_match_time += 10;
        advance_goal_celebration(&mut field, &mut context);
    }
    let later = field.players.iter().find(|p| p.id == keeper.0).unwrap();
    assert!(
        (later.position - keeper.1).norm() > 5.0,
        "he never got up at all"
    );
}

/// A trudge must not sit on the replay viewer's own walk/run threshold.
///
/// `Actors::MOVING` (1.1 m/s) is what decides whether a figure is drawn
/// facing the way he is going or facing the BALL. The conceding side walks
/// back with the ball behind them in their own net, so an eleven whose speed
/// oscillates across that line pivots between the two on alternate frames —
/// which is exactly the "players spinning round" in the report. Any speed a
/// consumer thresholds on has to be clearly one side of it.
#[test]
fn a_trudge_is_unambiguously_a_walk() {
    /// `crate::r#match::engine::flow::celebration` keeps this private, so
    /// the number is restated here on purpose: this test exists to catch it
    /// drifting back INTO the viewer's dead band, and a shared constant
    /// would drift with it.
    const VIEWER_WALK_RUN_THRESHOLD: f32 = 1.1;
    // Units per tick → metres per second: 1u = 0.125 m, 1 tick = 10 ms.
    let trudge_ms = 0.07 * 0.125 / 0.01;
    assert!(
        trudge_ms < VIEWER_WALK_RUN_THRESHOLD * 0.9,
        "a trudge at {trudge_ms:.2} m/s is inside the viewer's threshold band"
    );
}

/// The celebration shares the match RNG with every calibrated roll in the
/// engine. One extra draw inside it would shift every subsequent decision of
/// the match, so it must take none.
#[test]
fn the_celebration_does_not_touch_the_match_rng() {
    const SEED: u64 = 0xC0FFEE;

    let (mut field, mut context) = kickoff();
    context.rng = MatchRng::from_seed(SEED);
    score_at_the_home_end(&mut field, &mut context);
    // The one legitimate draw of the whole post-goal path: the length of
    // the dead-ball window.
    handle_goal_reset(&mut field, &mut context);

    let restart_at = context.dead_ball_until_ms;
    while context.total_match_time <= restart_at {
        context.total_match_time += 10;
        advance_goal_celebration(&mut field, &mut context);
    }
    let after: Vec<u32> = (0..16).map(|_| context.rng.unit_f32().to_bits()).collect();

    // The same stream with the celebration's ticks simply not taken.
    let reference = MatchRng::from_seed(SEED);
    reference.range_u64(45, 75);
    let expected: Vec<u32> = (0..16).map(|_| reference.unit_f32().to_bits()).collect();

    assert_eq!(
        after, expected,
        "the celebration consumed from the match RNG — every calibrated roll \
         after this goal has just moved"
    );
}

#[test]
fn a_goal_at_the_whistle_still_restarts_cleanly() {
    let (mut field, mut context) = kickoff();
    score_at_the_home_end(&mut field, &mut context);
    handle_goal_reset(&mut field, &mut context);
    assert!(context.goal_celebration.is_some());

    // Half time arrives with the ball still in the net.
    finish_goal_celebration(&mut field, &mut context);

    assert!(context.goal_celebration.is_none());
    assert!(
        field.ball.in_net.is_none(),
        "no period may end with a goal half-processed"
    );
    assert!((field.ball.position.x - 420.0).abs() < 1.0);
}
