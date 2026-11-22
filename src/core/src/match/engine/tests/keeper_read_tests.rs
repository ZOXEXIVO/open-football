//! **One read for every keeper state.** The same situation put to the keeper
//! in each state he can be in when it arises sends him to the same place; a
//! man through on goal is met with one plan; a keeper on the floor is down
//! until he is up; and a punch goes high and wide.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::engine::ball::ball::PlayerReach;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::goal::GOAL_WIDTH;
use crate::r#match::engine::officiating::restart_shape::{RestartShape, RestartStation};
use crate::r#match::engine::result::Score;
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::shared::{
    KeeperAerialClaim, KeeperBallClaim, KeeperFeetDecision, KeeperOneOnOne, KeeperRestPosition,
    KeeperSetPieceStance, KeeperSetPosition, KeeperShotReaction,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::goalkeepers::states::{
    GoalkeeperCatchingState, GoalkeeperClearingState, GoalkeeperComingOutState,
    GoalkeeperDivingState, GoalkeeperGettingUpState, GoalkeeperJumpingState,
    GoalkeeperOneOnOneState, GoalkeeperPickingUpState, GoalkeeperPreparingForSaveState,
    GoalkeeperPunchingState, GoalkeeperReturningGoalState, GoalkeeperStandingState,
    GoalkeeperTakeBallState, GoalkeeperWalkingState,
};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::passing::CrossType;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::shared_states::ChallengeKind;
use crate::r#match::{
    Ball, ClaimIntent, ClaimOutcome, GameTickContext, MatchContext, MatchField, MatchPlayer,
    MatchPlayerCollection, MatchRng, PassOriginRestart, PlayerSide, ResultMatchPositionData,
    SaveOutcome, ShotTarget, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const KICKOFF_MS: u64 = 10 * 60 * 1000;
/// The away keeper defends the goal on x = 840, centred on this y.
const GOAL_Y: f32 = 272.5;
/// Every state a keeper out of possession can be in when a situation
/// arises, short of the ones that are already acting on the ball.
const READERS: [GoalkeeperState; 6] = [
    GoalkeeperState::Standing,
    GoalkeeperState::Walking,
    GoalkeeperState::ReturningToGoal,
    GoalkeeperState::PreparingForSave,
    GoalkeeperState::ComingOut,
    GoalkeeperState::TakeBall,
];

fn kickoff() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = KICKOFF_MS;
    (field, context)
}

fn keeper_of(field: &MatchField) -> u32 {
    field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .expect("the away side has a goalkeeper")
}

fn outfielders(field: &MatchField, side: PlayerSide) -> Vec<u32> {
    field
        .players
        .iter()
        .filter(|p| p.side == Some(side) && !p.tactical_position.current_position.is_goalkeeper())
        .map(|p| p.id)
        .collect()
}

fn player_mut(field: &mut MatchField, id: u32) -> &mut MatchPlayer {
    field
        .players
        .iter_mut()
        .find(|p| p.id == id)
        .expect("player on the field")
}

fn put(field: &mut MatchField, id: u32, x: f32, y: f32) {
    let player = player_mut(field, id);
    player.position = Vector3::new(x, y, 0.0);
    player.velocity = Vector3::zeros();
}

/// The keeper at `at`, everybody else out of the way upfield.
fn scene(at: (f32, f32)) -> (MatchField, MatchContext, u32) {
    let (mut field, context) = kickoff();
    let keeper = keeper_of(&field);
    let ids: Vec<u32> = field.players.iter().map(|p| p.id).collect();
    for (index, id) in ids.into_iter().enumerate() {
        if id == keeper {
            put(&mut field, id, at.0, at.1);
        } else {
            put(
                &mut field,
                id,
                300.0 + (index % 4) as f32 * 30.0,
                60.0 + index as f32 * 18.0,
            );
        }
    }
    field.ball.current_owner = None;
    field.ball.previous_owner = None;
    field.ball.kickoff_taker = None;
    field.ball.pass_target_player_id = None;
    field.ball.velocity = Vector3::zeros();
    (field, context, keeper)
}

/// What the keeper decides in `state`, the way the state machine asks it.
fn decide(
    field: &mut MatchField,
    context: &MatchContext,
    keeper: u32,
    state: GoalkeeperState,
    in_state_time: u64,
) -> Option<StateChangeResult> {
    player_mut(field, keeper).transition_to(
        PlayerState::Goalkeeper(state),
        TransitionSource::EventHandler,
    );
    let field = &*field;
    let tick = GameTickContext::new(field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time,
        player: field.get_player(keeper).expect("the keeper"),
        context,
        tick_context: &tick,
    };
    match state {
        GoalkeeperState::Standing => GoalkeeperStandingState::default().process(&ctx),
        GoalkeeperState::Walking => GoalkeeperWalkingState::default().process(&ctx),
        GoalkeeperState::ReturningToGoal => GoalkeeperReturningGoalState::default().process(&ctx),
        GoalkeeperState::PreparingForSave => {
            GoalkeeperPreparingForSaveState::default().process(&ctx)
        }
        GoalkeeperState::ComingOut => GoalkeeperComingOutState::default().process(&ctx),
        GoalkeeperState::TakeBall => GoalkeeperTakeBallState::default().process(&ctx),
        GoalkeeperState::OneOnOne => GoalkeeperOneOnOneState::default().process(&ctx),
        GoalkeeperState::GettingUp => GoalkeeperGettingUpState::default().process(&ctx),
        GoalkeeperState::Diving => GoalkeeperDivingState::default().process(&ctx),
        GoalkeeperState::Punching => GoalkeeperPunchingState::default().process(&ctx),
        GoalkeeperState::Jumping => GoalkeeperJumpingState::default().process(&ctx),
        GoalkeeperState::Catching => GoalkeeperCatchingState::default().process(&ctx),
        GoalkeeperState::PickingUpBall => GoalkeeperPickingUpState::default().process(&ctx),
        other => panic!("no reader for {other:?}"),
    }
}

/// Where he goes from `state`: the state he is sent to, or `state` itself.
fn destination(
    field: &mut MatchField,
    context: &MatchContext,
    keeper: u32,
    state: GoalkeeperState,
) -> GoalkeeperState {
    match decide(field, context, keeper, state, 40).and_then(|change| change.state) {
        Some(PlayerState::Goalkeeper(next)) => next,
        Some(other) => panic!("a keeper sent to {other:?}"),
        None => state,
    }
}

/// A question about the scene put the way a state handler would put it.
fn read<T>(
    field: &MatchField,
    context: &MatchContext,
    keeper: u32,
    question: impl Fn(&StateProcessingContext) -> T,
) -> T {
    let tick = GameTickContext::new(field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 40,
        player: field.get_player(keeper).expect("the keeper"),
        context,
        tick_context: &tick,
    };
    question(&ctx)
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

// ── The same situation, the same answer ─────────────────────────────────

/// A high ball dropping into his six-yard box, nobody near it.
fn high_ball(field: &mut MatchField) {
    field.ball.position = Vector3::new(812.0, 268.0, 5.0);
    field.ball.velocity = Vector3::new(0.05, 0.0, 0.0);
    field.ball.cached_landing_position = Vector3::new(817.0, 268.0, 0.0);
    field.ball.previous_owner = outfielders(field, PlayerSide::Left).first().copied();
}

#[test]
fn a_high_ball_reads_the_same_from_every_state() {
    let (mut field, context, keeper) = scene((822.0, 272.0));
    high_ball(&mut field);
    player_mut(&mut field, keeper).transition_to(
        PlayerState::Goalkeeper(GoalkeeperState::Standing),
        TransitionSource::Reset,
    );
    assert!(
        read(&field, &context, keeper, |ctx| KeeperAerialClaim::assess(
            ctx
        )
        .is_some()),
        "the scene has no claim in it, so nothing is tested"
    );
    let from_rest = destination(&mut field, &context, keeper, GoalkeeperState::Standing);
    assert!(
        matches!(
            from_rest,
            GoalkeeperState::ComingOut | GoalkeeperState::Catching | GoalkeeperState::Jumping
        ),
        "he did not come for it: {from_rest:?}"
    );
    for state in READERS {
        assert_eq!(
            destination(&mut field, &context, keeper, state),
            from_rest,
            "from {state:?}"
        );
    }
}

#[test]
fn a_cross_while_repositioning_is_met_as_from_his_rest_point() {
    let (mut field, context, keeper) = scene((808.0, 292.0));
    high_ball(&mut field);
    let from_rest = destination(&mut field, &context, keeper, GoalkeeperState::Standing);
    assert!(
        matches!(
            from_rest,
            GoalkeeperState::ComingOut | GoalkeeperState::Catching | GoalkeeperState::Jumping
        ),
        "no claim from here, so nothing is tested: {from_rest:?}"
    );
    assert_eq!(
        destination(
            &mut field,
            &context,
            keeper,
            GoalkeeperState::ReturningToGoal
        ),
        from_rest,
        "he jogged home under a cross he would have come for"
    );
}

#[test]
fn a_loose_ball_an_opponent_is_nearer_is_not_set_off_for() {
    let (mut field, context, keeper) = scene((820.0, GOAL_Y));
    let opponent = outfielders(&field, PlayerSide::Left)[0];
    // Two metres from the keeper, one from the opponent.
    field.ball.position = Vector3::new(804.0, GOAL_Y, 0.11);
    put(&mut field, opponent, 796.0, GOAL_Y);
    for state in [
        GoalkeeperState::Standing,
        GoalkeeperState::Walking,
        GoalkeeperState::ReturningToGoal,
        GoalkeeperState::PreparingForSave,
    ] {
        let next = destination(&mut field, &context, keeper, state);
        assert!(
            !matches!(
                next,
                GoalkeeperState::ComingOut
                    | GoalkeeperState::TakeBall
                    | GoalkeeperState::Catching
                    | GoalkeeperState::PickingUpBall
            ),
            "from {state:?} he set off for it: {next:?}"
        );
    }
}

#[test]
fn a_back_pass_does_not_set_him() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y));
    let back = outfielders(&field, PlayerSide::Right)[0];
    put(&mut field, back, 740.0, GOAL_Y);
    field.ball.position = Vector3::new(752.0, GOAL_Y, 0.11);
    field.ball.velocity = Vector3::new(1.6, 0.0, 0.0);
    field.ball.previous_owner = Some(back);
    field.ball.pass_target_player_id = Some(keeper);
    for state in [
        GoalkeeperState::Standing,
        GoalkeeperState::Walking,
        GoalkeeperState::ReturningToGoal,
    ] {
        assert_ne!(
            destination(&mut field, &context, keeper, state),
            GoalkeeperState::PreparingForSave,
            "his own side's ball set him from {state:?}"
        );
    }
}

#[test]
fn a_ball_at_his_feet_is_one_decision_from_every_state() {
    let (mut field, context, keeper) = scene((818.0, GOAL_Y));
    let presser = outfielders(&field, PlayerSide::Left)[0];
    put(&mut field, presser, 796.0, GOAL_Y + 6.0);
    field.ball.position = Vector3::new(818.5, GOAL_Y, 0.11);
    field.ball.current_owner = Some(keeper);
    field.ball.held_in_hands = false;
    let rule = read(&field, &context, keeper, |ctx| {
        KeeperFeetDecision::state_for(ctx)
    });
    for state in READERS {
        assert_eq!(
            destination(&mut field, &context, keeper, state),
            rule,
            "from {state:?}"
        );
    }
}

fn shot_at(field: &mut MatchField, from: Vector3<f32>) {
    let shooter = outfielders(field, PlayerSide::Left)[0];
    field.ball.position = from + Vector3::new(4.0, 0.0, 0.3);
    field.ball.velocity = Vector3::new(2.4, 0.0, 0.0);
    field.ball.previous_owner = Some(shooter);
    field.ball.flags.in_flight_state = 60;
    field.ball.cached_shot_target = Some(ShotTarget {
        goal_line_y: GOAL_Y + 12.0,
        goal_line_z: 0.5,
        defending_side: PlayerSide::Right,
        save_rolled: false,
        block_rolled: true,
        blocked_by: None,
        deflected: false,
        shooter_threat: 0.5,
        struck_from: from,
        seen_from: from,
    });
}

#[test]
fn a_struck_shot_sets_him_from_every_state() {
    let (mut field, context, keeper) = scene((812.0, GOAL_Y));
    shot_at(&mut field, Vector3::new(690.0, GOAL_Y, 0.0));
    for state in [
        GoalkeeperState::Standing,
        GoalkeeperState::Walking,
        GoalkeeperState::ReturningToGoal,
        GoalkeeperState::ComingOut,
        GoalkeeperState::TakeBall,
        GoalkeeperState::OneOnOne,
    ] {
        assert_eq!(
            destination(&mut field, &context, keeper, state),
            GoalkeeperState::PreparingForSave,
            "from {state:?}"
        );
    }
}

// ── A man through on goal ───────────────────────────────────────────────

/// A home striker running at the away goal with the ball, from `x`.
fn breakaway(field: &mut MatchField, x: f32) -> u32 {
    let striker = outfielders(field, PlayerSide::Left)[0];
    let runner = player_mut(field, striker);
    runner.position = Vector3::new(x, GOAL_Y, 0.0);
    runner.velocity = Vector3::new(0.45, 0.0, 0.0);
    field.ball.position = Vector3::new(x + 2.0, GOAL_Y, 0.11);
    field.ball.velocity = Vector3::new(0.45, 0.0, 0.0);
    field.ball.current_owner = Some(striker);
    field.ball.previous_owner = Some(striker);
    striker
}

#[test]
fn a_man_through_is_met_from_every_state() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y));
    breakaway(&mut field, 720.0);
    for state in [
        GoalkeeperState::Standing,
        GoalkeeperState::Walking,
        GoalkeeperState::ReturningToGoal,
        GoalkeeperState::PreparingForSave,
        GoalkeeperState::ComingOut,
        GoalkeeperState::TakeBall,
    ] {
        assert_eq!(
            destination(&mut field, &context, keeper, state),
            GoalkeeperState::OneOnOne,
            "from {state:?}"
        );
    }
}

#[test]
fn a_covered_carrier_is_the_defences() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y));
    breakaway(&mut field, 720.0);
    let cover = outfielders(&field, PlayerSide::Right)[0];
    put(&mut field, cover, 770.0, GOAL_Y + 4.0);
    assert!(read(&field, &context, keeper, |ctx| KeeperOneOnOne::duel(
        ctx
    )
    .is_none()));
    for state in READERS {
        assert_ne!(
            destination(&mut field, &context, keeper, state),
            GoalkeeperState::OneOnOne,
            "a covered carrier drew him out from {state:?}"
        );
    }
}

/// `Arrive` brakes a man inside 3u of where he is going.
const AT_THE_POINT: f32 = 3.0;

/// Where a keeper's `OneOnOne` steering leaves him.
struct ClosedDown {
    at: Vector3<f32>,
    /// The stand-off point the duel asks for, read from where he ends up.
    point: Vector3<f32>,
    speed: f32,
}

/// Plays the keeper's `OneOnOne` steering against a carrier stood on the
/// ball, at the speed the engine lets him run.
fn close_down(start: (f32, f32), from: GoalkeeperState) -> ClosedDown {
    let (mut field, context, keeper) = scene(start);
    breakaway(&mut field, 720.0);
    assert_eq!(
        destination(&mut field, &context, keeper, from),
        GoalkeeperState::OneOnOne,
        "the breakaway did not draw him from {from:?}"
    );
    let mut speed = 0.0;
    let mut point = Vector3::zeros();
    for step in 0..400 {
        let (velocity, cap, holds) = {
            let tick = GameTickContext::new(&field, &context.players);
            let ctx = StateProcessingContext {
                in_state_time: 20 + step,
                player: field.get_player(keeper).expect("the keeper"),
                context: &context,
                tick_context: &tick,
            };
            point = KeeperOneOnOne::point(&ctx, &GoalkeeperSkillProfile::from_ctx(&ctx));
            let velocity = GoalkeeperOneOnOneState::default()
                .velocity(&ctx)
                .expect("a velocity");
            let cap = ctx.player.skills.goalkeeper_max_speed(
                ctx.player.player_attributes.condition,
                GoalkeeperSpeedContext::Active,
            );
            let holds = GoalkeeperOneOnOneState::default().process(&ctx).is_none();
            (velocity, cap, holds)
        };
        assert!(holds, "he gave up the duel at step {step}");
        let velocity = if velocity.norm() > cap {
            velocity * (cap / velocity.norm())
        } else {
            velocity
        };
        speed = velocity.norm();
        player_mut(&mut field, keeper).position += velocity;
    }
    ClosedDown {
        at: field.get_player(keeper).expect("the keeper").position,
        point,
        speed,
    }
}

#[test]
fn the_same_breakaway_from_two_states_ends_at_one_stand_off_point() {
    let at_rest = close_down((826.0, GOAL_Y), GoalkeeperState::Standing);
    let recovering = close_down((800.0, GOAL_Y + 40.0), GoalkeeperState::ReturningToGoal);
    assert!(
        (at_rest.point - recovering.point).norm() < 1e-3,
        "two stand-off points for one breakaway: {:?} and {:?}",
        at_rest.point,
        recovering.point
    );
    for closed in [&at_rest, &recovering] {
        assert!(
            (closed.point - closed.at).norm() < AT_THE_POINT,
            "he stopped {:.1}u off the stand-off point",
            (closed.point - closed.at).norm()
        );
    }
    assert!(
        at_rest.at.x < 826.0 - 8.0,
        "he never came off his line: {:?}",
        at_rest.at
    );
}

#[test]
fn a_carrier_in_close_control_finds_him_stood_at_the_point() {
    for (start, from) in [
        ((826.0, GOAL_Y), GoalkeeperState::Standing),
        ((800.0, GOAL_Y + 40.0), GoalkeeperState::ReturningToGoal),
    ] {
        let closed = close_down(start, from);
        assert!(
            (closed.point - closed.at).norm() < AT_THE_POINT,
            "he stopped {:.1}u off the stand-off point",
            (closed.point - closed.at).norm()
        );
        assert_eq!(
            closed.speed, 0.0,
            "he ran through the point at a man in control"
        );
    }
}

// ── On the floor ────────────────────────────────────────────────────────

#[test]
fn a_dive_lands_into_getting_up_with_or_without_the_ball() {
    let (mut field, context, keeper) = scene((828.0, GOAL_Y + 30.0));
    field.ball.position = Vector3::new(400.0, GOAL_Y, 0.11);
    player_mut(&mut field, keeper).height = 0.3;
    assert_eq!(
        decide(&mut field, &context, keeper, GoalkeeperState::Diving, 6).and_then(|c| c.state),
        None,
        "out of the dive before he landed"
    );
    player_mut(&mut field, keeper).height = 0.0;
    assert_eq!(
        decide(&mut field, &context, keeper, GoalkeeperState::Diving, 6).and_then(|c| c.state),
        Some(PlayerState::Goalkeeper(GoalkeeperState::GettingUp))
    );
    field.ball.position = Vector3::new(828.0, GOAL_Y + 30.0, 1.15);
    field.ball.current_owner = Some(keeper);
    field.ball.held_in_hands = true;
    assert_eq!(
        decide(&mut field, &context, keeper, GoalkeeperState::Diving, 6).and_then(|c| c.state),
        Some(PlayerState::Goalkeeper(GoalkeeperState::GettingUp)),
        "a held dive went straight to his feet"
    );
}

/// How long the keeper in `scene` stays down, in AI ticks.
fn time_down(field: &mut MatchField, context: &MatchContext, keeper: u32) -> u64 {
    (0..200)
        .find(|t| decide(field, context, keeper, GoalkeeperState::GettingUp, *t).is_some())
        .expect("he never got up")
}

#[test]
fn an_agile_keeper_is_up_sooner() {
    let up_after = |agility: f32, balance: f32| {
        let (mut field, context, keeper) = scene((828.0, GOAL_Y + 30.0));
        field.ball.position = Vector3::new(400.0, GOAL_Y, 0.11);
        let body = &mut player_mut(&mut field, keeper).skills.physical;
        body.agility = agility;
        body.balance = balance;
        time_down(&mut field, &context, keeper)
    };
    let nimble = up_after(18.0, 18.0);
    let heavy = up_after(6.0, 6.0);
    assert!(nimble < heavy, "up at {nimble} and {heavy} ticks");
    // 0.6 to 1.1 s across the whole range, in 20 ms AI ticks.
    assert!((30..=55).contains(&nimble), "{nimble}");
    assert!((30..=55).contains(&heavy), "{heavy}");
}

#[test]
fn a_rebound_finds_him_down_and_not_diving() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y + 10.0));
    shot_at(&mut field, Vector3::new(760.0, GOAL_Y - 10.0, 0.0));
    let half_a_second = 25;
    let body = &field
        .get_player(keeper)
        .expect("the keeper")
        .skills
        .physical;
    assert!(half_a_second < ChallengeKind::Dive.recovery_ticks(body.agility, body.balance));
    let next = decide(
        &mut field,
        &context,
        keeper,
        GoalkeeperState::GettingUp,
        half_a_second,
    )
    .and_then(|change| change.state);
    assert_eq!(next, None, "he left the floor for a rebound: {next:?}");
}

/// Engine ticks: whatever he does on the floor, he does it at a scramble.
#[test]
fn a_landed_keeper_moves_no_faster_than_a_scramble() {
    let (mut field, mut context, keeper) = scene((826.0, GOAL_Y + 24.0));
    field.ball.position = Vector3::new(400.0, GOAL_Y, 0.11);
    {
        let diver = player_mut(&mut field, keeper);
        diver.transition_to(
            PlayerState::Goalkeeper(GoalkeeperState::Diving),
            TransitionSource::EventHandler,
        );
        diver.height = 0.05;
        diver.vertical_speed = -0.01;
        diver.velocity = Vector3::new(-0.3, 0.6, 0.0);
    }
    let scramble = 1.5 / 12.5;
    let mut down_ticks = 0;
    for _ in 0..300 {
        let before = field.get_player(keeper).expect("the keeper").position;
        let mut data = ResultMatchPositionData::empty();
        let mut tick = GameTickContext::new(&field, &context.players);
        FootballEngine::<840, 545>::game_tick(&mut field, &mut context, &mut data, &mut tick);
        context.increment_time();
        let keeper_now = field.get_player(keeper).expect("the keeper");
        if keeper_now.state != PlayerState::Goalkeeper(GoalkeeperState::GettingUp) {
            if down_ticks > 0 {
                break;
            }
            continue;
        }
        down_ticks += 1;
        // The tick he lands on is the dive's.
        if down_ticks > 2 {
            let step = (keeper_now.position - before).norm();
            assert!(
                step <= scramble + 1e-4,
                "{:.2} m/s on the floor",
                step * 12.5
            );
        }
    }
    assert!(down_ticks > 40, "down for only {down_ticks} engine ticks");
}

// ── The punch ───────────────────────────────────────────────────────────

/// A ball at head height in front of the middle of his goal, in a crowd.
fn punch_scene(airborne: bool) -> (MatchField, MatchContext, u32) {
    let (mut field, context, keeper) = scene((824.0, GOAL_Y));
    for (index, id) in outfielders(&field, PlayerSide::Left)
        .into_iter()
        .take(4)
        .enumerate()
    {
        put(
            &mut field,
            id,
            816.0 + index as f32 * 3.0,
            GOAL_Y - 6.0 + index as f32 * 4.0,
        );
    }
    if airborne {
        player_mut(&mut field, keeper).height = 0.5;
    }
    field.ball.position = Vector3::new(821.0, GOAL_Y, GoalkeeperPunchingState::FLOOR + 0.3);
    field.ball.velocity = Vector3::new(0.8, 0.0, -0.01);
    (field, context, keeper)
}

/// **A flap lands him on his feet.** The contest decided he got to it and
/// lost it: in the air he only comes down, and on his feet the read has
/// him. Never a dive.
#[test]
fn a_flapped_claim_lands_him_on_his_feet() {
    for (state, airborne) in [
        (GoalkeeperState::Jumping, true),
        (GoalkeeperState::Catching, false),
        (GoalkeeperState::ComingOut, false),
    ] {
        let (mut field, context, keeper) = punch_scene(airborne);
        player_mut(&mut field, keeper).pending_claim = Some(ClaimOutcome::Flapped);
        let next = decide(&mut field, &context, keeper, state, 3).and_then(|change| change.state);
        assert!(
            !matches!(
                next,
                Some(PlayerState::Goalkeeper(
                    GoalkeeperState::Diving | GoalkeeperState::GettingUp
                ))
            ),
            "a flap in {state:?} went to {next:?}"
        );
        if airborne {
            assert_eq!(next, None, "in the air after a flap he only comes down");
        }
    }
}

#[test]
fn a_leap_after_a_missed_punch_only_comes_down() {
    let (mut field, context, keeper) = punch_scene(true);
    player_mut(&mut field, keeper).transition_to(
        PlayerState::Goalkeeper(GoalkeeperState::Punching),
        TransitionSource::EventHandler,
    );
    for _ in 0..50 {
        assert_eq!(
            decide(&mut field, &context, keeper, GoalkeeperState::Jumping, 3)
                .and_then(|change| change.state),
            None,
            "he went for the ball again on the leap he had already used"
        );
        // `decide` put him in `Jumping`; the leap is still the punch's.
        player_mut(&mut field, keeper).transition_to(
            PlayerState::Goalkeeper(GoalkeeperState::Punching),
            TransitionSource::EventHandler,
        );
    }
}

#[test]
fn punches_from_the_middle_of_his_goal_come_down_wide() {
    let (mut field, context, keeper) = punch_scene(false);
    let six_yard_half_width = 9.16;
    let (mut punches, mut wide) = (0, 0);
    for _ in 0..400 {
        player_mut(&mut field, keeper).pending_claim = Some(ClaimOutcome::Punched);
        let Some(change) = decide(&mut field, &context, keeper, GoalkeeperState::Jumping, 3) else {
            continue;
        };
        let Some(launch) = player_events(change).into_iter().find_map(|e| match e {
            PlayerEvent::ClearBall(_, velocity) => Some(velocity),
            _ => None,
        }) else {
            continue;
        };
        punches += 1;
        assert!(launch.x < 0.0, "punched toward his own goal: {launch:?}");
        let across = launch.y * Ball::hang_ticks(launch.z) * 0.125;
        if across.abs() > six_yard_half_width {
            wide += 1;
        }
    }
    assert_eq!(punches, 400, "a punch the contest gave him was not thrown");
    assert!(
        wide * 2 > punches,
        "{wide} of {punches} punches came down wider than the six-yard box"
    );
}

/// A floated cross dropping toward the away keeper, who has come for it
/// meaning to `intent` it, with a home forward attacking it beside him.
fn cross_scene(seed: u64, intent: ClaimIntent) -> (MatchField, MatchContext, u32) {
    let (mut field, mut context, keeper) = scene((815.0, GOAL_Y));
    context.rng = MatchRng::from_seed(seed);
    let home = outfielders(&field, PlayerSide::Left);
    put(&mut field, home[0], 808.0, GOAL_Y - 6.0);
    {
        let gk = player_mut(&mut field, keeper);
        gk.transition_to(
            PlayerState::Goalkeeper(GoalkeeperState::Catching),
            TransitionSource::EventHandler,
        );
        gk.claim_intent = Some(intent);
    }
    field.ball.position = Vector3::new(800.0, GOAL_Y - 2.0, 2.8);
    field.ball.velocity = Vector3::new(1.0, 0.1, -0.02);
    field.ball.previous_owner = Some(home[1]);
    field.ball.pending_cross_type = Some(CrossType::FloatedFarPost);
    field.ball.pending_pass_was_cross = true;
    field.ball.cross_contest_resolved = false;
    field.ball.flags.in_flight_state = 60;
    (field, context, keeper)
}

fn engine_tick(field: &mut MatchField, context: &mut MatchContext) {
    let mut data = ResultMatchPositionData::empty();
    let mut tick = GameTickContext::new(field, &context.players);
    FootballEngine::<840, 545>::game_tick(field, context, &mut data, &mut tick);
    context.increment_time();
}

/// **A cross he comes for is decided once.** The contest decides it as it
/// is delivered and keeps its call on the ball; what he does with the ball
/// when it reaches him is that call, whatever the dice do afterwards.
#[test]
fn a_cross_he_comes_for_is_decided_once() {
    let mut decided = [0usize; 3];
    for seed in 0..160u64 {
        let intent = if seed % 2 == 0 {
            ClaimIntent::Catch
        } else {
            ClaimIntent::Punch
        };
        let (mut field, mut context, keeper) = cross_scene(seed, intent);
        engine_tick(&mut field, &mut context);
        let Some(call) = field.ball.keeper_claim else {
            continue;
        };
        context.rng = MatchRng::from_seed(seed.wrapping_mul(7919).wrapping_add(1));
        for _ in 0..60 {
            engine_tick(&mut field, &mut context);
            if field.ball.keeper_claim.is_none() {
                break;
            }
        }
        assert!(
            field.ball.keeper_claim.is_none(),
            "seed {seed}: the ball never reached him"
        );
        match call.outcome {
            ClaimOutcome::Caught => {
                assert_eq!(
                    field.ball.current_owner,
                    Some(keeper),
                    "seed {seed}: not held"
                );
                decided[0] += 1;
            }
            ClaimOutcome::Punched => {
                assert_eq!(field.ball.last_touch_player_id, Some(keeper), "seed {seed}");
                assert!(
                    field.ball.velocity.x < 0.0 && field.ball.velocity.z > 0.0,
                    "seed {seed}: the punch was not thrown: {:?}",
                    field.ball.velocity
                );
                decided[1] += 1;
            }
            ClaimOutcome::Flapped => {
                assert_eq!(
                    field.ball.pending_failed_claim_gk_id,
                    Some(keeper),
                    "seed {seed}"
                );
                assert_ne!(
                    field.ball.current_owner,
                    Some(keeper),
                    "seed {seed}: a flap held"
                );
                decided[2] += 1;
            }
        }
    }
    assert!(
        decided[0] >= 5 && decided[1] >= 5,
        "too few calls carried out (caught, punched, flapped): {decided:?}"
    );
}

/// **He can play what he came for.** A ball just under the leap a claim
/// commits him to is one the contest lets him play; the outfield reach it
/// used to grant him stopped short of it.
#[test]
fn a_ball_he_committed_to_under_his_ceiling_is_his_to_play_in_the_contest() {
    let (mut field, _context, keeper) = scene((815.0, GOAL_Y));
    let gk = field.get_player(keeper).expect("the keeper").clone();
    let ceiling = KeeperAerialClaim::leap_ceiling(gk.skills.physical.jumping);
    let ball = Vector3::new(812.0, GOAL_Y, ceiling - 0.1);
    assert!(
        KeeperAerialClaim::contest_reach(&gk, ball),
        "a ball {:.2} m up, under his {ceiling:.2} m ceiling, was out of the contest's reach",
        ball.z
    );
    field.ball.position = ball;
    assert!(
        !PlayerReach::can_possess(&field.ball, &gk),
        "the outfield reach takes this ball too, so nothing was tested"
    );
}

// ── A save carried out (design D5's outcome table) ───────────────────────

/// The ball in the keeper's gloves, `held_for` engine ticks ago.
fn in_his_gloves(field: &mut MatchField, context: &MatchContext, keeper: u32, held_for: u64) {
    let at = field.get_player(keeper).expect("the keeper").position;
    field.ball.position = at + Vector3::new(0.0, 0.0, 1.15);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(keeper);
    field.ball.held_in_hands = true;
    field.ball.hands_since_tick = context.current_tick().saturating_sub(held_for);
}

fn next_state(change: Option<StateChangeResult>) -> Option<PlayerState> {
    change.and_then(|c| c.state)
}

#[test]
fn a_save_held_on_his_feet_is_gathered_then_held() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y));
    player_mut(&mut field, keeper).transition_to(
        PlayerState::Goalkeeper(GoalkeeperState::PreparingForSave),
        TransitionSource::Reset,
    );
    in_his_gloves(&mut field, &context, keeper, 0);
    player_mut(&mut field, keeper).on_save_outcome(SaveOutcome::Held);
    assert_eq!(
        next_state(decide(
            &mut field,
            &context,
            keeper,
            GoalkeeperState::PreparingForSave,
            3
        )),
        Some(PlayerState::Goalkeeper(GoalkeeperState::Catching)),
        "a held save did not go through the gather"
    );
    player_mut(&mut field, keeper).pending_save = None;
    assert_eq!(
        next_state(decide(
            &mut field,
            &context,
            keeper,
            GoalkeeperState::Catching,
            1
        )),
        None,
        "out of the gather the tick the ball went in"
    );
    in_his_gloves(&mut field, &context, keeper, 30);
    assert_eq!(
        next_state(decide(
            &mut field,
            &context,
            keeper,
            GoalkeeperState::Catching,
            15
        )),
        Some(PlayerState::Goalkeeper(GoalkeeperState::HoldingBall))
    );
}

#[test]
fn a_save_held_in_a_dive_comes_down_with_him() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y + 20.0));
    player_mut(&mut field, keeper).transition_to(
        PlayerState::Goalkeeper(GoalkeeperState::Diving),
        TransitionSource::Reset,
    );
    player_mut(&mut field, keeper).height = 0.3;
    in_his_gloves(&mut field, &context, keeper, 0);
    player_mut(&mut field, keeper).on_save_outcome(SaveOutcome::Held);
    assert_eq!(
        field.get_player(keeper).expect("the keeper").pending_save,
        None,
        "a diving keeper's save was staged as if he were on his feet"
    );
    assert_eq!(
        next_state(decide(
            &mut field,
            &context,
            keeper,
            GoalkeeperState::Diving,
            6
        )),
        None,
        "out of the dive before he landed"
    );
    player_mut(&mut field, keeper).height = 0.0;
    assert_eq!(
        next_state(decide(
            &mut field,
            &context,
            keeper,
            GoalkeeperState::Diving,
            6
        )),
        Some(PlayerState::Goalkeeper(GoalkeeperState::GettingUp))
    );
    assert_eq!(
        next_state(decide(
            &mut field,
            &context,
            keeper,
            GoalkeeperState::GettingUp,
            5
        )),
        None,
        "up with the ball before his time down"
    );
    assert_eq!(
        next_state(decide(
            &mut field,
            &context,
            keeper,
            GoalkeeperState::GettingUp,
            200
        )),
        Some(PlayerState::Goalkeeper(GoalkeeperState::HoldingBall))
    );
}

#[test]
fn a_save_not_held_in_a_dive_lands_him_on_the_floor() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y + 20.0));
    player_mut(&mut field, keeper).transition_to(
        PlayerState::Goalkeeper(GoalkeeperState::Diving),
        TransitionSource::Reset,
    );
    field.ball.position = Vector3::new(790.0, GOAL_Y + 60.0, 0.3);
    field.ball.velocity = Vector3::new(-0.6, 0.4, 0.0);
    player_mut(&mut field, keeper).on_save_outcome(SaveOutcome::Loose);
    assert_eq!(
        field.get_player(keeper).expect("the keeper").pending_save,
        None
    );
    assert_eq!(
        next_state(decide(
            &mut field,
            &context,
            keeper,
            GoalkeeperState::Diving,
            6
        )),
        Some(PlayerState::Goalkeeper(GoalkeeperState::GettingUp))
    );
}

#[test]
fn a_save_not_held_on_his_feet_is_the_reads() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y));
    // Parried, and dropping a metre in front of him with nobody near.
    field.ball.position = Vector3::new(818.0, GOAL_Y, 0.11);
    field.ball.velocity = Vector3::new(-0.2, 0.0, 0.0);
    let unstaged = destination(&mut field, &context, keeper, GoalkeeperState::Standing);
    player_mut(&mut field, keeper).on_save_outcome(SaveOutcome::Loose);
    assert_eq!(
        destination(&mut field, &context, keeper, GoalkeeperState::Standing),
        unstaged,
        "a parry on his feet sent him somewhere the read does not"
    );
    assert!(
        matches!(
            unstaged,
            GoalkeeperState::PickingUpBall | GoalkeeperState::Catching
        ),
        "the live ball at his feet did not bring him to it: {unstaged:?}"
    );
}

/// A shot the save model has already ruled past him: still in flight,
/// latched, and running a metre from his gloves.
fn beaten_shot(field: &mut MatchField, keeper: u32) {
    let at = field.get_player(keeper).expect("the keeper").position;
    shot_at(field, Vector3::new(700.0, GOAL_Y, 0.0));
    field.ball.position = at + Vector3::new(4.0, 8.0, 0.6);
    field.ball.velocity = Vector3::new(1.6, 0.4, 0.0);
    if let Some(target) = field.ball.cached_shot_target.as_mut() {
        target.save_rolled = true;
    }
}

#[test]
fn a_beaten_shot_is_never_handed_to_him_later_in_its_flight() {
    for state in [
        GoalkeeperState::Catching,
        GoalkeeperState::Diving,
        GoalkeeperState::GettingUp,
        GoalkeeperState::PreparingForSave,
    ] {
        let (mut field, context, keeper) = scene((812.0, GOAL_Y));
        beaten_shot(&mut field, keeper);
        if state == GoalkeeperState::Diving {
            player_mut(&mut field, keeper).height = 0.3;
        }
        for _ in 0..300 {
            let Some(change) = decide(&mut field, &context, keeper, state, 3) else {
                continue;
            };
            let next = change.state;
            assert!(
                !player_events(change).iter().any(|e| matches!(
                    e,
                    PlayerEvent::CaughtBall(_)
                        | PlayerEvent::ClaimBall(_)
                        | PlayerEvent::GainBall(_)
                )),
                "a beaten shot was handed to him in {state:?}"
            );
            assert_ne!(
                next,
                Some(PlayerState::Goalkeeper(GoalkeeperState::HoldingBall)),
                "a beaten shot ended in his hands from {state:?}"
            );
        }
    }
}

// ── One gather model ─────────────────────────────────────────────────────

/// A loose ball at the away keeper's feet in his six-yard box, last played
/// by a forward: slow with nobody near and good hands, or quick with two
/// forwards closing and poor hands.
fn gather_scene(pressed: bool) -> (MatchField, MatchContext, u32) {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y));
    let forwards = outfielders(&field, PlayerSide::Left);
    player_mut(&mut field, keeper).skills.goalkeeping.handling = if pressed { 1.0 } else { 20.0 };
    if pressed {
        put(&mut field, forwards[0], 808.0, GOAL_Y - 8.0);
        put(&mut field, forwards[1], 808.0, GOAL_Y + 8.0);
    }
    let tick = context.current_tick();
    let team = field.get_player(forwards[2]).expect("a forward").team_id;
    field.ball.record_touch(forwards[2], team, tick, true);
    field.ball.previous_owner = Some(forwards[2]);
    field.ball.position = Vector3::new(825.6, GOAL_Y, 0.11);
    field.ball.velocity = Vector3::new(if pressed { 0.9 } else { 0.05 }, 0.0, 0.0);
    (field, context, keeper)
}

/// The gather the scene offers him: one attempt, and that attempt spread
/// over the ticks a state that tries on every one of them has his gloves
/// on it.
fn shared_gather(field: &MatchField, context: &MatchContext, keeper: u32) -> (f32, f32) {
    let tick = GameTickContext::new(field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 0,
        player: field.get_player(keeper).expect("the keeper"),
        context,
        tick_context: &tick,
    };
    let odds = KeeperBallClaim::gather_odds(&ctx);
    let per_tick =
        GoalkeeperSkillProfile::from_ctx(&ctx).per_tick_save(odds, KeeperBallClaim::CONTACT_TICKS);
    (odds, per_tick)
}

/// **Every gather is the same gather.** On the floor, a ball arriving at
/// him in `Catching`, a dive onto a loose ball and a reach from the ground
/// all take a ball into his gloves at the odds `gather_odds` gives it, and
/// move with it from a calm ball to a pressed one.
#[test]
fn every_route_into_his_gloves_is_the_same_gather() {
    const TRIES: usize = 4000;
    for pressed in [false, true] {
        let (mut field, context, keeper) = gather_scene(pressed);
        let (odds, per_tick) = shared_gather(&field, &context, keeper);
        for (state, expected, height) in [
            (GoalkeeperState::PickingUpBall, odds, 0.0),
            (GoalkeeperState::Catching, per_tick, 0.0),
            (GoalkeeperState::Diving, per_tick, 0.4),
            (GoalkeeperState::GettingUp, per_tick, 0.0),
        ] {
            player_mut(&mut field, keeper).height = height;
            let gathered = (0..TRIES)
                .filter(|_| {
                    decide(&mut field, &context, keeper, state, 1).is_some_and(|change| {
                        player_events(change)
                            .iter()
                            .any(|e| matches!(e, PlayerEvent::CaughtBall(id) if *id == keeper))
                    })
                })
                .count();
            let rate = gathered as f32 / TRIES as f32;
            let spread = (expected * (1.0 - expected) / TRIES as f32).sqrt();
            assert!(
                (rate - expected).abs() <= 4.0 * spread + 0.005,
                "{state:?} (pressed {pressed}) gathered {rate:.3} a try against the shared {expected:.3}"
            );
        }
    }
}

// ── Physical realism ─────────────────────────────────────────────────────

/// **Set, not sprinting.** With no shot in flight, a forward changing the
/// angle 20 m out moves even the most agile keeper across his goal at no
/// more than a set keeper's shuffle, 4 m/s.
#[test]
fn a_set_keeper_adjusts_at_a_shuffle() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y - 40.0));
    player_mut(&mut field, keeper).skills.physical.agility = 20.0;
    let forward = outfielders(&field, PlayerSide::Left)[0];
    put(&mut field, forward, 680.0, GOAL_Y + 90.0);
    field.ball.position = Vector3::new(682.0, GOAL_Y + 90.0, 0.0);
    field.ball.current_owner = Some(forward);
    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 10,
        player: field.get_player(keeper).expect("the keeper"),
        context: &context,
        tick_context: &tick,
    };
    let step = GoalkeeperPreparingForSaveState::default()
        .velocity(&ctx)
        .expect("he holds or adjusts");
    let metres_per_second = step.xy().norm() * 12.5;
    assert!(
        metres_per_second > 0.0,
        "he did not adjust at all, so nothing was tested"
    );
    assert!(
        metres_per_second <= 4.0 + 1e-3,
        "he set himself at {metres_per_second:.2} m/s"
    );
}

/// The keeper's `PreparingForSave` step, stood on his set point for the
/// ball where it is, with the ball loose 16 m out and travelling `across`.
fn set_step(
    field: &mut MatchField,
    context: &MatchContext,
    keeper: u32,
    across: f32,
) -> Vector3<f32> {
    let goal = Vector3::new(840.0, GOAL_Y, 0.0);
    let ball = Vector3::new(840.0 - 128.0, GOAL_Y + 40.0, 0.0);
    field.ball.position = ball;
    field.ball.velocity = Vector3::new(0.0, across, 0.0);
    field.ball.current_owner = None;
    let read = {
        let tick = GameTickContext::new(field, &context.players);
        let ctx = StateProcessingContext {
            in_state_time: 10,
            player: field.get_player(keeper).expect("the keeper"),
            context,
            tick_context: &tick,
        };
        GoalkeeperSkillProfile::from_ctx(&ctx).positioning
    };
    let set = KeeperSetPosition::guard_point(goal, ball, 840.0, read);
    put(field, keeper, set.x, set.y);
    let tick = GameTickContext::new(field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 10,
        player: field.get_player(keeper).expect("the keeper"),
        context,
        tick_context: &tick,
    };
    GoalkeeperPreparingForSaveState::default()
        .velocity(&ctx)
        .expect("he holds or adjusts")
}

/// Every attribute in his read of the game at the top of the scale.
fn reads_the_game(field: &mut MatchField, keeper: u32) {
    let gk = player_mut(field, keeper);
    let mental = &mut gk.skills.mental;
    mental.positioning = 20.0;
    mental.anticipation = 20.0;
    mental.decisions = 20.0;
    mental.concentration = 20.0;
    mental.teamwork = 20.0;
    mental.composure = 20.0;
    gk.skills.goalkeeping.command_of_area = 20.0;
    gk.skills.goalkeeping.communication = 20.0;
}

/// **He reads where the ball is going.** On his set point for a ball
/// standing 16 m out, a keeper who reads the game holds; play the same
/// ball across his box and he steps the way it is going before it gets
/// there, as his rest point already did.
#[test]
fn a_set_keeper_moves_with_a_ball_played_across_his_box() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y));
    reads_the_game(&mut field, keeper);
    assert_eq!(
        set_step(&mut field, &context, keeper, 0.0),
        Vector3::zeros(),
        "on his set point for a still ball he did not hold, so nothing was tested"
    );
    let step = set_step(&mut field, &context, keeper, -2.0);
    assert!(
        step.y < 0.0,
        "with the ball played across he stepped {step:?}, not the way it went"
    );
}

/// **…and so does the one-on-one.** A carrier cutting across him is met
/// where he is going, not where he was.
#[test]
fn the_one_on_one_point_reads_where_the_carrier_is_going() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y));
    reads_the_game(&mut field, keeper);
    let carrier = breakaway(&mut field, 760.0);
    let point = |field: &mut MatchField, across: f32| {
        let run = Vector3::new(0.45, across, 0.0);
        player_mut(field, carrier).velocity = run;
        field.ball.velocity = run;
        let tick = GameTickContext::new(field, &context.players);
        let ctx = StateProcessingContext {
            in_state_time: 20,
            player: field.get_player(keeper).expect("the keeper"),
            context: &context,
            tick_context: &tick,
        };
        KeeperOneOnOne::point(&ctx, &GoalkeeperSkillProfile::from_ctx(&ctx))
    };
    let straight = point(&mut field, 0.0);
    let cutting = point(&mut field, 0.5);
    assert!(
        cutting.y > straight.y + 0.5,
        "met a carrier cutting across at y={:.1}, where he meets one running straight (y={:.1})",
        cutting.y,
        straight.y
    );
}

/// **Caught moving.** The same shot, struck once at a set keeper and once
/// at him running across his goal: the running keeper starts his save
/// later.
#[test]
fn a_keeper_caught_moving_reacts_later() {
    let (mut field, context, keeper) = scene((826.0, GOAL_Y));
    let struck_from = Vector3::new(680.0, GOAL_Y, 0.4);
    field.ball.cached_shot_target = Some(ShotTarget {
        goal_line_y: GOAL_Y,
        goal_line_z: 1.0,
        defending_side: PlayerSide::Right,
        save_rolled: false,
        block_rolled: true,
        blocked_by: None,
        deflected: false,
        shooter_threat: 0.5,
        struck_from,
        seen_from: struck_from,
    });
    field.ball.velocity = Vector3::new(2.0, 0.0, 0.0);
    let reacted = |field: &MatchField, strike_speed: f32| {
        let mut gk = field.get_player(keeper).expect("the keeper").clone();
        gk.strike_speed = strike_speed;
        let tick = GameTickContext::new(field, &context.players);
        let ctx = StateProcessingContext {
            in_state_time: 0,
            player: &gk,
            context: &context,
            tick_context: &tick,
        };
        let prof = GoalkeeperSkillProfile::from_ctx(&ctx);
        (
            KeeperShotReaction::reaction_ticks(&prof),
            KeeperShotReaction::has_reacted(&ctx, &prof),
        )
    };
    let (reaction, _) = reacted(&field, 0.0);
    // A fifth of his reaction past it: a set keeper is moving, one caught
    // at twice his shuffle is not yet.
    field.ball.position = struck_from + field.ball.velocity * (reaction * 1.2);
    let agility = field
        .get_player(keeper)
        .expect("the keeper")
        .skills
        .physical
        .agility;
    let running = 2.0 * KeeperShotReaction::set_shuffle(agility);
    assert!(reacted(&field, 0.0).1, "the set keeper had not reacted");
    assert!(
        !reacted(&field, running).1,
        "the keeper caught running had reacted as soon as the set one"
    );
}

/// **On the bisector.** With the ball 16 m out and 16 m wide of the middle
/// of his goal, he stands on the bisector of the angle the ball makes with
/// the two posts, which lies nearer his near post than the line from the
/// ball to the middle of the goal.
#[test]
fn he_stands_on_the_bisector_of_the_angle_at_the_ball() {
    let goal = Vector3::new(840.0, GOAL_Y, 0.0);
    let ball = Vector3::new(840.0 - 128.0, GOAL_Y + 128.0, 0.0);
    let rest = KeeperRestPosition::point(
        goal,
        ball,
        Vector3::zeros(),
        PlayerSide::Right,
        700.0,
        840.0,
        0.5,
        1.0,
    );
    let middle_line_y = goal.y + (ball.y - goal.y) * (rest.x - goal.x) / (ball.x - goal.x);
    assert!(
        (rest.y - KeeperSetPosition::guard_y(goal, ball, rest.x)).abs() < 1e-3,
        "a keeper who reads the game perfectly is off the bisector: {rest:?}"
    );
    assert!(
        rest.y > middle_line_y + 1.0,
        "he stands at y={:.1}, no nearer his near post than the middle line's {middle_line_y:.1}",
        rest.y
    );
    assert!(
        rest.y < goal.y + GOAL_WIDTH,
        "he stands outside his near post: y={:.1}",
        rest.y
    );
}

/// Where across the pitch a clearance from the away keeper lands, standing
/// 6 m below the middle of his goal, with a forward `press` units from him
/// or none at all.
fn clearance_landing_y(field_seed: u64, press: Option<f32>) -> f32 {
    let (mut field, mut context, keeper) = scene((760.0, GOAL_Y + 48.0));
    context.rng = MatchRng::from_seed(field_seed);
    let at = field.get_player(keeper).expect("the keeper").position;
    if let Some(gap) = press {
        let forward = outfielders(&field, PlayerSide::Left)[0];
        put(&mut field, forward, at.x - gap, at.y);
    }
    field.ball.position = at;
    field.ball.current_owner = Some(keeper);
    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 0,
        player: field.get_player(keeper).expect("the keeper"),
        context: &context,
        tick_context: &tick,
    };
    let change = GoalkeeperClearingState::default()
        .process(&ctx)
        .expect("he clears it");
    let launch = player_events(change)
        .into_iter()
        .find_map(|e| match e {
            PlayerEvent::ClearBall(_, velocity) => Some(velocity),
            _ => None,
        })
        .expect("the clearance is struck");
    at.y + launch.y * Ball::hang_ticks(launch.z)
}

/// **A pressured clearance goes wide.** With a forward on him the keeper
/// lands it nearer the touchline than the middle of the pitch; with nobody
/// near, it goes down the middle where his midfielders can contest it.
#[test]
fn a_pressured_clearance_lands_nearer_the_touchline() {
    let height = 545.0;
    for seed in 0..20 {
        let pressed = clearance_landing_y(seed, Some(12.0));
        assert!(
            pressed > height * 0.75,
            "pressed, it landed at y={pressed:.0}, no nearer the touchline than the middle"
        );
        let free = clearance_landing_y(seed, None);
        assert!(
            free < height * 0.75,
            "unpressed, it went wide too (y={free:.0}), so nothing was tested"
        );
    }
}

/// A direct free kick 22 m out, level with the away keeper's near post,
/// with the wall the restart shape lines up for it and the wall's men at
/// their stations where `in_place` says so. Where he stands across his
/// goal for it, and the wall's stations, outermost first.
fn free_kick_stance(in_place: impl Fn(usize) -> bool) -> (f32, Vec<Vector3<f32>>) {
    let (mut field, context, keeper) = scene((830.0, GOAL_Y));
    let spot = Vector3::new(840.0 - 176.0, GOAL_Y + GOAL_WIDTH, 0.0);
    let taker = outfielders(&field, PlayerSide::Left)[0];
    put(&mut field, taker, spot.x - 4.0, spot.y);
    field.ball.position = spot;
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(taker);
    field.ball.pass_origin_restart = PassOriginRestart::DirectFreeKick;
    let defending: Vec<u32> = outfielders(&field, PlayerSide::Right);
    let wall: Vec<RestartStation> = RestartShape::plan(
        PassOriginRestart::DirectFreeKick,
        &field.players,
        taker,
        spot,
        840.0,
        545.0,
    )
    .into_iter()
    .filter(|s| defending.contains(&s.player_id))
    .collect();
    for (i, station) in wall.iter().enumerate() {
        if in_place(i) {
            put(
                &mut field,
                station.player_id,
                station.position.x,
                station.position.y,
            );
        }
    }
    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 0,
        player: field.get_player(keeper).expect("the keeper"),
        context: &context,
        tick_context: &tick,
    };
    let point = KeeperSetPieceStance::point(&ctx).expect("a free kick he sets for");
    (point.y, wall.iter().map(|s| s.position).collect())
}

/// **The wall has the near post and the keeper the far.** At a free kick
/// level with his near post, the wall lines up on the near side, from the
/// line between the ball and that post inward, and he stands toward his
/// far post.
#[test]
fn a_free_kick_level_with_a_post_has_the_wall_near_and_the_keeper_far() {
    let (keeper_y, wall) = free_kick_stance(|_| true);
    assert!(wall.len() >= 3, "a wall of {}", wall.len());
    assert!(
        (wall[0].y - (GOAL_Y + GOAL_WIDTH)).abs() < 0.5,
        "the outermost man is off the line to the near post: {:?}",
        wall[0]
    );
    for man in &wall {
        assert!(
            man.y >= GOAL_Y,
            "a man of the wall stands on the far side: {man:?}"
        );
    }
    assert!(
        keeper_y < GOAL_Y,
        "behind a near-side wall he stood at y={keeper_y:.1}, not toward his far post"
    );
}

/// **He follows the wall that is there.** With only its outermost man in
/// place, the near post is still open, and he stands nearer it than he
/// does behind the whole wall.
#[test]
fn the_keeper_follows_a_wall_late_to_its_stations() {
    let (behind_the_wall, _) = free_kick_stance(|_| true);
    let (one_man_up, _) = free_kick_stance(|i| i == 0);
    assert!(
        one_man_up > behind_the_wall + 2.0,
        "with the wall still walking up he stood at y={one_man_up:.1}, as behind the whole \
         wall (y={behind_the_wall:.1})"
    );
}
