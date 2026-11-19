//! **The forward's states, put to one situation at a time.** What a forward
//! does with a ball he gets on a run, and — as the read lands — the same
//! situation put to him in each state he can hold without the ball.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::PlayerSkills;
use crate::r#match::common_states::{ChallengeKind, ClosingPoint, ShapeTempo};
use crate::r#match::engine::ball::ball::{AwaitedRestart, PassOriginRestart, RestartQuiet};
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::result::Score;
use crate::r#match::engine::teamplay::plans::attack::AttackPlan;
use crate::r#match::engine::teamplay::possession::TeamPossession;
use crate::r#match::events::{Event, EventCollection, EventDispatcher};
use crate::r#match::forwarders::states::common::{ForwardRead, RunLine};
use crate::r#match::forwarders::states::{
    ForwardCreatingSpaceState, ForwardCrossReceivingState, ForwardHeadingState,
    ForwardHoldingShapeState, ForwardHoldingWidthState, ForwardInterceptingState,
    ForwardJockeyingState, ForwardPassingState, ForwardPressingState, ForwardReturningState,
    ForwardRunningInBehindState, ForwardRunningState, ForwardStandingState, ForwardState,
    ForwardTacklingState, ForwardTakeBallState, ForwardWalkingState,
};
use crate::r#match::passing::{CrossModel, CrossType, FlankPlay};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::players::WonHeader;
use crate::r#match::player::strategies::players::ops::box_movement::BoxMovement;
use crate::r#match::player::strategies::players::ops::support::SupportOffer;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    ActivityIntensity, BoxSlot, ConditionContext, DefenceRefreshInputs, DefensivePlan,
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, MatchRng, PlayerSide,
    ResultMatchPositionData, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const HOME: u32 = 1;
const AWAY: u32 = 2;
const FL: u32 = 109;
const FR: u32 = 110;
const AWAY_GK: u32 = 200;
const AWAY_DCL: u32 = 202;
const AWAY_MCL: u32 = 206;
const AWAY_DCR: u32 = 203;
const HOME_DCL: u32 = 102;
const HOME_MCL: u32 = 106;
const HOME_MCR: u32 = 107;

/// Home on the left, attacking x = 840, ten minutes in. Everyone parked at
/// the far end until a test stands him somewhere.
struct Pitch {
    field: MatchField,
    context: MatchContext,
    /// How long the man asked has been in his state, in AI ticks.
    in_state_time: u64,
}

impl Pitch {
    fn new() -> Self {
        let mut home = squad(HOME, 100);
        let mut away = squad(AWAY, 200);
        for p in home.main_squad.iter_mut().chain(away.main_squad.iter_mut()) {
            Self::footballer(&mut p.skills);
        }
        let players = MatchPlayerCollection::from_squads(&home, &away);
        let mut field = MatchField::new(840, 545, home, away);
        assert_eq!(field.side_of(HOME), PlayerSide::Left);
        field.ball.kickoff_taker = None;
        field.ball.current_tick_cached = 1_000;
        for (index, p) in field.players.iter_mut().enumerate() {
            p.velocity = Vector3::zeros();
            let x = if p.team_id == HOME { 60.0 } else { 20.0 };
            p.position = Vector3::new(x, 20.0 + (index % 11) as f32 * 48.0, 0.0);
        }
        let mut context = MatchContext::new(&field, players, Score::new(HOME, AWAY), false, false);
        context.total_match_time = 600_000;
        Pitch {
            field,
            context,
            in_state_time: 40,
        }
    }

    /// Ordinary technical and mental skills: the shared fixture leaves them
    /// at zero, and nobody passes to anybody with a passing of 0.
    fn footballer(skills: &mut PlayerSkills) {
        let t = &mut skills.technical;
        for v in [
            &mut t.corners,
            &mut t.crossing,
            &mut t.dribbling,
            &mut t.finishing,
            &mut t.first_touch,
            &mut t.free_kicks,
            &mut t.heading,
            &mut t.long_shots,
            &mut t.long_throws,
            &mut t.marking,
            &mut t.passing,
            &mut t.penalty_taking,
            &mut t.tackling,
            &mut t.technique,
        ] {
            *v = 14.0;
        }
        let m = &mut skills.mental;
        for v in [
            &mut m.aggression,
            &mut m.anticipation,
            &mut m.bravery,
            &mut m.composure,
            &mut m.concentration,
            &mut m.decisions,
            &mut m.determination,
            &mut m.flair,
            &mut m.leadership,
            &mut m.off_the_ball,
            &mut m.positioning,
            &mut m.teamwork,
            &mut m.vision,
            &mut m.work_rate,
        ] {
            *v = 14.0;
        }
    }

    fn refresh(&mut self) -> &mut Self {
        for _ in 0..10 {
            let possession = TeamPossession::read(&self.field);
            FootballEngine::<840, 545>::refresh_tactical_states(
                &mut self.field,
                &mut self.context,
                possession,
                100,
            );
        }
        self
    }

    fn stand(&mut self, id: u32, x: f32, y: f32) -> &mut Self {
        let p = self.field.get_player_mut(id).unwrap();
        p.position = Vector3::new(x, y, 0.0);
        p.velocity = Vector3::zeros();
        self
    }

    fn on_the_ball(&mut self, id: u32) -> &mut Self {
        let at = self.field.get_player(id).unwrap().position;
        self.field.ball.position = at;
        self.field.ball.velocity = Vector3::zeros();
        self.field.ball.current_owner = Some(id);
        self.field.ball.pass_target_player_id = None;
        self
    }

    fn put_in(&mut self, id: u32, state: ForwardState) {
        self.field
            .get_player_mut(id)
            .unwrap()
            .transition_to(PlayerState::Forward(state), TransitionSource::Reset);
    }

    fn with_ctx<R>(&self, id: u32, read: impl FnOnce(&StateProcessingContext) -> R) -> R {
        let tick = GameTickContext::new(&self.field, &self.context.players);
        let ctx = StateProcessingContext {
            in_state_time: self.in_state_time,
            player: self.field.get_player(id).unwrap(),
            context: &self.context,
            tick_context: &tick,
        };
        read(&ctx)
    }

    fn process(&mut self, id: u32, state: ForwardState) -> Option<StateChangeResult> {
        self.put_in(id, state);
        self.with_ctx(id, |ctx| match state {
            ForwardState::Walking => ForwardWalkingState::default().process(ctx),
            ForwardState::Passing => ForwardPassingState::default().process(ctx),
            ForwardState::Heading => ForwardHeadingState::default().process(ctx),
            ForwardState::RunningInBehind => ForwardRunningInBehindState::default().process(ctx),
            ForwardState::Running => ForwardRunningState::default().process(ctx),
            ForwardState::Pressing => ForwardPressingState::default().process(ctx),
            ForwardState::CreatingSpace => ForwardCreatingSpaceState::default().process(ctx),
            ForwardState::CrossReceiving => ForwardCrossReceivingState::default().process(ctx),
            ForwardState::Tackling => ForwardTacklingState::default().process(ctx),
            ForwardState::TakeBall => ForwardTakeBallState::default().process(ctx),
            ForwardState::Intercepting => ForwardInterceptingState::default().process(ctx),
            ForwardState::Returning => ForwardReturningState::default().process(ctx),
            ForwardState::HoldingWidth => ForwardHoldingWidthState::default().process(ctx),
            ForwardState::Jockeying => ForwardJockeyingState::default().process(ctx),
            ForwardState::HoldingShape => ForwardHoldingShapeState::default().process(ctx),
            ForwardState::Standing => ForwardStandingState::default().process(ctx),
            other => panic!("no handler wired for {other:?}"),
        })
    }

    fn velocity(&mut self, id: u32, state: ForwardState) -> Vector3<f32> {
        self.put_in(id, state);
        self.with_ctx(id, |ctx| match state {
            ForwardState::Walking => ForwardWalkingState::default().velocity(ctx),
            ForwardState::HoldingShape => ForwardHoldingShapeState::default().velocity(ctx),
            ForwardState::Jockeying => ForwardJockeyingState::default().velocity(ctx),
            other => panic!("no velocity wired for {other:?}"),
        })
        .expect("a velocity")
    }

    /// The defensive plan for the side out of possession, with `holder` on
    /// the ball.
    fn plan(&mut self, holder: u32) -> &mut Self {
        DefensivePlan::refresh(
            &mut self.context.defence_home,
            &mut self.context.defence_away,
            &DefenceRefreshInputs {
                field: &self.field,
                possession_holder: Some(holder),
                home_team_id: HOME,
                away_team_id: AWAY,
                home_keeper_voice: 0.5,
                away_keeper_voice: 0.5,
            },
        );
        self
    }

    fn condition(&mut self, id: u32, condition: i16) -> &mut Self {
        self.field
            .get_player_mut(id)
            .unwrap()
            .player_attributes
            .condition = condition;
        self
    }

    fn anchor(&self, id: u32) -> Vector3<f32> {
        self.with_ctx(id, |ctx| ctx.team().my_anchor())
    }

    fn read(&self, id: u32) -> ForwardState {
        self.with_ctx(id, ForwardRead::resolve)
    }

    fn quickest(&mut self, id: u32) -> &mut Self {
        let skills = &mut self.field.get_player_mut(id).unwrap().skills;
        skills.physical.pace = 20.0;
        skills.physical.acceleration = 20.0;
        skills.mental.off_the_ball = 20.0;
        self
    }

    fn attack(&self) -> AttackPlan {
        *self.context.attack_plan_for_team(HOME)
    }

    /// A high ball nobody owns, at `(x, y, z)` and moving with `velocity`.
    fn ball_in_the_air(&mut self, at: Vector3<f32>, velocity: Vector3<f32>) -> &mut Self {
        self.field.ball.current_owner = None;
        self.field.ball.position = at;
        self.field.ball.velocity = velocity;
        self
    }

    fn top_speed(&self, id: u32) -> f32 {
        self.field
            .get_player(id)
            .unwrap()
            .max_speed_with_condition_cached()
    }

    fn dispatch(&mut self, played: &[PlayerEvent]) {
        let mut events = EventCollection::with_capacity(4);
        for event in played {
            events.add_player_event(event.clone());
        }
        let mut data = ResultMatchPositionData::empty();
        EventDispatcher::dispatch(
            &mut events,
            &mut self.field,
            &mut self.context,
            &mut data,
            true,
        );
    }
}

fn events_of(change: StateChangeResult) -> Vec<PlayerEvent> {
    let mut events = change.events;
    events
        .drain()
        .filter_map(|e| match e {
            Event::PlayerEvent(p) => Some(p),
            _ => None,
        })
        .collect()
}

fn state_of(change: &Option<StateChangeResult>) -> Option<PlayerState> {
    change.as_ref().and_then(|c| c.state)
}

/// Whether a state change strikes, or sends him to strike, at goal.
fn shoots(change: &mut StateChangeResult) -> bool {
    let events: Vec<Event> = change.events.drain().collect();
    matches!(
        change.state,
        Some(PlayerState::Forward(
            ForwardState::Shooting | ForwardState::Finishing
        ))
    ) || change.shot_reason.is_some()
        || events
            .iter()
            .any(|e| matches!(e, Event::PlayerEvent(PlayerEvent::Shoot(_))))
}

// ── The ball arrives ──────────────────────────────────────────────────────

/// A forward who gets the ball in any state he holds without it is, on that
/// tick, in the state that decides what to do with it.
#[test]
fn the_ball_arrives_in_the_state_that_decides_what_to_do_with_it() {
    for state in [
        ForwardState::Walking,
        ForwardState::Pressing,
        ForwardState::Tackling,
        ForwardState::Returning,
        ForwardState::Intercepting,
        ForwardState::RunningInBehind,
        ForwardState::CreatingSpace,
        ForwardState::HoldingWidth,
        ForwardState::CrossReceiving,
        ForwardState::Heading,
        ForwardState::TakeBall,
    ] {
        let mut pitch = Pitch::new();
        pitch.stand(FL, 600.0, 272.0).on_the_ball(FL).refresh();
        let change = pitch.process(FL, state);
        assert_eq!(
            state_of(&change),
            Some(PlayerState::Forward(ForwardState::Running)),
            "{state:?} kept the decision"
        );
    }
}

/// A runner who gets the ball eight metres out on the byline hands the
/// decision to `Running` on that tick, and no shot is struck that the shot
/// helper did not approve: there it sees no goal to aim at and holds.
#[test]
fn a_runner_on_the_ball_near_goal_does_not_shoot_when_the_helper_holds() {
    let mut pitch = Pitch::new();
    pitch
        .stand(FL, 839.0, 272.5 + 64.0)
        .on_the_ball(FL)
        .refresh();
    let mut change = pitch
        .process(FL, ForwardState::RunningInBehind)
        .expect("a runner on the ball leaves the run");
    assert_eq!(
        change.state,
        Some(PlayerState::Forward(ForwardState::Running))
    );
    assert!(!shoots(&mut change), "the run itself shot");
    let mut decided = pitch.process(FL, ForwardState::Running);
    assert!(
        !decided.as_mut().is_some_and(shoots),
        "the helper held and he shot anyway: {:?}",
        state_of(&decided)
    );
}

// ── His time on the ball ─────────────────────────────────────────────────

/// He has waited ten seconds for the ball and gets it in the area with no
/// shot on: he keeps it for the time a forward is allowed there, counted
/// from when he got it, and not past it.
#[test]
fn a_forward_who_has_waited_gets_his_time() {
    let mut pitch = Pitch::new();
    pitch.stand(FL, 835.0, 380.0).on_the_ball(FL).refresh();
    pitch.in_state_time = 500;
    pitch.field.ball.ownership_duration = 20;
    let early = pitch.process(FL, ForwardState::Running);
    assert!(
        early.is_none(),
        "made to give it up 0.2 s after he got it: {:?}",
        state_of(&early)
    );
    pitch.field.ball.ownership_duration = 700;
    assert!(
        pitch.process(FL, ForwardState::Running).is_some(),
        "kept it 7 s in the area"
    );
}

/// The pressed snapshot belongs to the first moment after he gets the ball:
/// it is there after a long wait in his state, and gone a second after the
/// ball came to him, however recently he changed state.
#[test]
fn a_first_time_strike_is_first_time() {
    let snapshots = |in_state_time: u64, on_the_ball: u32| {
        let mut pitch = Pitch::new();
        pitch
            .stand(AWAY_GK, 830.0, 272.0)
            .stand(AWAY_DCL, 787.0, 280.0)
            .stand(FL, 785.0, 272.0)
            .on_the_ball(FL)
            .refresh();
        pitch
            .field
            .get_player_mut(FL)
            .unwrap()
            .skills
            .technical
            .first_touch = 8.0;
        pitch.in_state_time = in_state_time;
        pitch.field.ball.ownership_duration = on_the_ball;
        (0..200u64)
            .filter(|seed| {
                pitch.context.rng = MatchRng::from_seed(*seed);
                pitch
                    .process(FL, ForwardState::Running)
                    .is_some_and(|c| c.shot_reason == Some("FWD_SNAPSHOT_PRESSED"))
            })
            .count()
    };
    assert!(
        snapshots(300, 4) > 0,
        "no snapshot on his first touch after a long wait"
    );
    assert_eq!(
        snapshots(0, 100),
        0,
        "a snapshot a second after he got the ball"
    );
}

// ── The offside line ─────────────────────────────────────────────────────

/// The door that starts a run reads the referee's line — the second-last
/// opponent, whoever he is on the team sheet — and not the deepest man
/// listed as a defender.
#[test]
fn the_run_door_reads_the_referees_line() {
    let mut pitch = Pitch::new();
    // The back four are parked at the far end; a midfielder is second-last.
    pitch
        .stand(AWAY_GK, 830.0, 272.0)
        .stand(AWAY_MCL, 760.0, 300.0)
        .stand(FR, 700.0, 200.0)
        .on_the_ball(FR)
        .stand(FL, 740.0, 272.0)
        .refresh();
    assert!(
        !pitch.with_ctx(FL, |ctx| RunLine::is_offside(ctx)),
        "onside, a few metres inside the second-last man"
    );
    pitch.stand(FL, 780.0, 272.0).refresh();
    assert!(
        pitch.with_ctx(FL, |ctx| RunLine::is_offside(ctx)),
        "beyond the second-last man and the ball"
    );
}

/// The run is held inside the line, and in his own half there is no line
/// to hold.
#[test]
fn a_runner_in_his_own_half_is_not_held() {
    let mut pitch = Pitch::new();
    // Their second-last man has stepped up into his half, at 380; halfway
    // is 420.
    pitch
        .stand(AWAY_GK, 700.0, 272.0)
        .stand(AWAY_DCL, 380.0, 300.0)
        .stand(FL, 330.0, 272.0)
        .refresh();
    let aim = Vector3::new(395.0, 272.0, 0.0);
    let held = pitch.with_ctx(FL, |ctx| ForwardRunningInBehindState::held(ctx, aim));
    assert_eq!(held.x, aim.x, "held in his own half");

    // Their line deep in their own half holds him.
    pitch
        .stand(AWAY_DCL, 600.0, 300.0)
        .stand(FL, 560.0, 272.0)
        .refresh();
    let aim = Vector3::new(650.0, 272.0, 0.0);
    let held = pitch.with_ctx(FL, |ctx| ForwardRunningInBehindState::held(ctx, aim));
    assert!(held.x < 600.0, "not held at their line: {}", held.x);
}

// ── Headers ───────────────────────────────────────────────────────────────

/// FL up in the air for a high ball at `ball`, awarded to him or to
/// `awarded_to`: what his heading state does with it on this tick.
fn header_at(ball: Vector3<f32>, awarded_to: u32, seed: u64) -> Pitch {
    let mut pitch = Pitch::new();
    pitch.stand(FL, 780.0, 272.0).refresh();
    pitch.context.rng = MatchRng::from_seed(seed);
    pitch.ball_in_the_air(ball, Vector3::new(-0.2, 0.0, -0.05));
    pitch.field.ball.aerial_contest_winner = Some(awarded_to);
    let man = pitch.field.get_player_mut(FL).unwrap();
    man.on_aerial_award();
    man.height = 0.3;
    pitch
}

fn heads(change: Option<StateChangeResult>) -> bool {
    change.map(events_of).unwrap_or_default().iter().any(|e| {
        matches!(
            e,
            PlayerEvent::Shoot(..) | PlayerEvent::ClearBall(..) | PlayerEvent::KnockDown(..)
        )
    })
}

/// The one reach rule, across the grass: a ball 2.4 m up straight above him
/// is headed; one a metre or a metre and a half away is not, however
/// little its height adds to the distance.
#[test]
fn a_header_is_measured_across_the_grass() {
    let mut above = header_at(Vector3::new(780.5, 272.0, 2.4), FL, 1);
    assert!(
        heads(above.process(FL, ForwardState::Heading)),
        "2.4 m up, above him"
    );
    for (dx, what) in [(8.0, "1 m away"), (12.0, "1.5 m away")] {
        let mut wide = header_at(Vector3::new(780.0 + dx, 272.0, 2.0), FL, 1);
        assert!(
            !heads(wide.process(FL, ForwardState::Heading)),
            "{what} across the grass"
        );
    }
}

/// Awarded and in reach, he heads it, every time: nothing rolls again for
/// whether he wins it.
#[test]
fn no_second_roll() {
    for seed in 0..60 {
        let mut pitch = header_at(Vector3::new(780.0, 272.0, 2.0), FL, seed);
        assert!(
            heads(pitch.process(FL, ForwardState::Heading)),
            "seed {seed}: an awarded header not played"
        );
    }
}

/// A header he plays himself is not taken off for a stride and a half
/// short of the ball: he goes up under it.
#[test]
fn a_won_header_is_taken_off_for_under_the_ball() {
    let leaps = |dx: f32| {
        let mut pitch = header_at(Vector3::new(780.0 + dx, 272.0, 2.0), FL, 1);
        pitch.field.get_player_mut(FL).unwrap().height = 0.0;
        pitch
            .process(FL, ForwardState::Heading)
            .map(events_of)
            .unwrap_or_default()
            .iter()
            .any(|e| matches!(e, PlayerEvent::Leap(..)))
    };
    assert!(!leaps(12.0), "took off 1.5 m short of it");
    assert!(leaps(4.0), "under it, he stayed down");
}

/// A high ball nobody was awarded, in reach: his heading state leaves it,
/// and no state sends him up for it.
#[test]
fn a_ball_nobody_awarded_him_is_not_headed() {
    let mut pitch = header_at(Vector3::new(780.0, 272.0, 2.0), FL, 1);
    pitch.field.ball.aerial_contest_winner = None;
    pitch.field.get_player_mut(FL).unwrap().aerial_award = false;
    assert!(!heads(pitch.process(FL, ForwardState::Heading)));
    assert_ne!(pitch.read(FL), ForwardState::Heading);
}

/// A high ball awarded to an opponent beside him is left.
#[test]
fn a_ball_awarded_to_another_man_is_left() {
    let mut pitch = header_at(Vector3::new(780.0, 272.0, 2.0), AWAY_DCL, 1);
    pitch.stand(AWAY_DCL, 782.0, 272.0);
    pitch.field.get_player_mut(FL).unwrap().aerial_award = false;
    assert!(!heads(pitch.process(FL, ForwardState::Heading)));
    for state in OFF_BALL {
        let got = state_of(&pitch.process(FL, state));
        assert_ne!(
            got,
            Some(PlayerState::Forward(ForwardState::Heading)),
            "{state:?}"
        );
    }
}

/// A header that is not an attempt is booked by where it is played: a
/// clearance in his own third, nothing anywhere else.
#[test]
fn a_glance_in_his_own_third_is_a_clearance() {
    let glance_at = |x: f32| {
        let mut pitch = Pitch::new();
        pitch.stand(FL, x, 272.0).refresh();
        pitch.ball_in_the_air(Vector3::new(x, 272.0, 2.0), Vector3::zeros());
        pitch.with_ctx(FL, WonHeader::glance)
    };
    assert!(
        matches!(
            glance_at(150.0),
            Event::PlayerEvent(PlayerEvent::ClearBall(..))
        ),
        "his own third"
    );
    assert!(
        matches!(
            glance_at(400.0),
            Event::PlayerEvent(PlayerEvent::KnockDown(..))
        ),
        "the middle third, in his own half"
    );
    assert!(
        matches!(
            glance_at(800.0),
            Event::PlayerEvent(PlayerEvent::KnockDown(..))
        ),
        "their area"
    );
}

/// A won header glanced away inside their area is neither a shot nor a
/// clearance.
#[test]
fn a_knock_down_is_not_a_clearance() {
    for seed in 0..40 {
        let mut pitch = header_at(Vector3::new(780.0, 272.0, 2.0), FL, seed);
        let events = pitch
            .process(FL, ForwardState::Heading)
            .map(events_of)
            .unwrap_or_default();
        if events.iter().any(|e| matches!(e, PlayerEvent::Shoot(..))) {
            continue;
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, PlayerEvent::KnockDown(..))),
            "{events:?}"
        );
        pitch.dispatch(&events);
        let man = pitch.field.get_player(FL).unwrap();
        assert_eq!(man.statistics.clearances, 0);
        assert_eq!(man.memory.shots_taken, 0);
        return;
    }
    panic!("forty won headers in their area and every one an attempt");
}

// ── The settle ────────────────────────────────────────────────────────────

/// Settling over the ball — shielding from a man on him, or going at goal
/// with nobody near — never moves him faster than he can run.
#[test]
fn settling_over_the_ball_never_runs_above_his_top_speed() {
    let mut pitch = Pitch::new();
    pitch.stand(FL, 800.0, 272.0).on_the_ball(FL).refresh();
    let top = pitch.top_speed(FL);
    let alone = pitch.with_ctx(FL, |ctx| {
        ForwardRunningState::default().settle_over_the_ball(ctx)
    });
    assert!(alone.norm() <= top + 1e-4, "{} against {top}", alone.norm());
    pitch.stand(AWAY_DCL, 810.0, 280.0).refresh();
    let shielding = pitch.with_ctx(FL, |ctx| {
        ForwardRunningState::default().settle_over_the_ball(ctx)
    });
    assert!(
        shielding.norm() <= top + 1e-4,
        "{} against {top}",
        shielding.norm()
    );
}

// ── One read ──────────────────────────────────────────────────────────────

/// The off-ball states a forward can be found in.
const OFF_BALL: [ForwardState; 12] = [
    ForwardState::Standing,
    ForwardState::Walking,
    ForwardState::HoldingShape,
    ForwardState::Returning,
    ForwardState::Pressing,
    ForwardState::Intercepting,
    ForwardState::TakeBall,
    ForwardState::RunningInBehind,
    ForwardState::CreatingSpace,
    ForwardState::HoldingWidth,
    ForwardState::CrossReceiving,
    ForwardState::Jockeying,
];

/// The away left centre-back on the ball at `carrier`, the home left forward
/// at `forward`, everybody else parked, and the plan made for it.
fn carrier_scene(carrier: (f32, f32), forward: (f32, f32)) -> Pitch {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_DCL, carrier.0, carrier.1)
        .on_the_ball(AWAY_DCL)
        .stand(FL, forward.0, forward.1)
        .refresh()
        .plan(AWAY_DCL);
    pitch
}

/// A forward the plan names to close a centre-back 6 m from him closes him
/// from every state he can hold without the ball.
#[test]
fn a_named_forward_closes_the_carrier_from_every_state() {
    let mut pitch = carrier_scene((600.0, 272.0), (650.0, 272.0));
    assert_eq!(pitch.context.defence_home.presser(), Some(FL));
    for state in OFF_BALL {
        if state == ForwardState::Jockeying {
            continue;
        }
        let change = pitch.process(FL, state);
        let got = state_of(&change).unwrap_or(PlayerState::Forward(state));
        assert_eq!(
            got,
            PlayerState::Forward(ForwardState::Pressing),
            "{state:?}"
        );
    }
}

/// A carrier 6 m from a forward the plan has not named, with the named man
/// on him: the forward closes him from no state.
#[test]
fn the_best_chaser_does_not_close_another_mans_carrier() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_DCL, 600.0, 272.0)
        .on_the_ball(AWAY_DCL)
        .stand(HOME_MCL, 590.0, 272.0)
        .stand(FL, 650.0, 272.0)
        .refresh()
        .plan(AWAY_DCL);
    assert_eq!(pitch.context.defence_home.presser(), Some(HOME_MCL));
    for state in OFF_BALL {
        let change = pitch.process(FL, state);
        let got = state_of(&change).unwrap_or(PlayerState::Forward(state));
        assert!(
            !matches!(
                got,
                PlayerState::Forward(
                    ForwardState::Pressing | ForwardState::Jockeying | ForwardState::Tackling
                )
            ),
            "{state:?} closed another man's carrier: {got:?}"
        );
    }
}

/// Named for a carrier in his own half, he closes him, and the press does
/// not send him back to his place and out again.
#[test]
fn a_forward_named_in_his_own_half_keeps_closing() {
    let mut pitch = carrier_scene((300.0, 272.0), (350.0, 272.0));
    assert_eq!(pitch.context.defence_home.presser(), Some(FL));
    assert_eq!(
        state_of(&pitch.process(FL, ForwardState::Walking)),
        Some(PlayerState::Forward(ForwardState::Pressing))
    );
    assert!(
        pitch.process(FL, ForwardState::Pressing).is_none(),
        "the press gave him up in his own half"
    );
}

/// Nobody within the plan's reach: the best-placed man closes the carrier,
/// and he may be a forward.
#[test]
fn with_nobody_named_the_best_placed_forward_closes() {
    let mut pitch = carrier_scene((500.0, 272.0), (720.0, 272.0));
    assert_eq!(pitch.context.defence_home.presser(), None);
    assert_eq!(
        state_of(&pitch.process(FL, ForwardState::Walking)),
        Some(PlayerState::Forward(ForwardState::Pressing))
    );
}

// ── Readiness ─────────────────────────────────────────────────────────────

/// FL on his place, the away centre-back on the ball `away` units from it
/// along the pitch, a home midfielder on the carrier.
fn phase_scene(away: f32, condition: i16) -> Pitch {
    let mut pitch = Pitch::new();
    // Their keeper and a centre-back deep in their own half, so their last
    // line is ahead of him and he is not stranded beyond it.
    pitch
        .stand(AWAY_GK, 830.0, 272.0)
        .stand(AWAY_DCR, 780.0, 320.0)
        .stand(AWAY_DCL, 420.0, 272.0)
        .on_the_ball(AWAY_DCL)
        .condition(FL, condition)
        .refresh();
    // His place is read off the shape this refresh made; the carrier is
    // then put `away` from it, and no refresh moves the place again.
    let place = pitch.anchor(FL);
    let x = if place.x - away >= 20.0 {
        place.x - away
    } else {
        place.x + away
    };
    pitch
        .stand(FL, place.x, place.y)
        .stand(AWAY_DCL, x, place.y)
        .on_the_ball(AWAY_DCL)
        .stand(HOME_MCL, x - 10.0, place.y)
        .plan(AWAY_DCL);
    pitch
}

#[test]
fn a_forward_is_relaxed_while_play_is_far_away() {
    let pitch = phase_scene(420.0, 10_000);
    assert_eq!(pitch.read(FL), ForwardState::Walking);
}

#[test]
fn a_forward_is_alert_when_the_ball_comes_near() {
    let mut pitch = phase_scene(240.0, 10_000);
    assert_eq!(pitch.read(FL), ForwardState::HoldingShape);
    assert_eq!(
        state_of(&pitch.process(FL, ForwardState::Walking)),
        Some(PlayerState::Forward(ForwardState::HoldingShape))
    );
}

/// 47 m off: alert stays alert inside the margin, relaxed stays relaxed.
#[test]
fn the_phase_does_not_flicker() {
    let mut pitch = phase_scene(376.0, 10_000);
    assert!(pitch.process(FL, ForwardState::HoldingShape).is_none());
    assert!(pitch.process(FL, ForwardState::Walking).is_none());
}

#[test]
fn a_forward_is_relaxed_with_his_own_side_on_the_ball_far_off() {
    let mut pitch = Pitch::new();
    pitch.refresh();
    let place = pitch.anchor(FL);
    pitch
        .stand(FL, place.x, place.y)
        .stand(HOME_MCL, (place.x - 320.0).max(20.0), place.y)
        .on_the_ball(HOME_MCL)
        .refresh();
    assert_eq!(
        pitch.with_ctx(FL, ForwardRead::phase),
        ForwardState::Walking
    );
}

/// Relaxed and alert steer at the same point.
#[test]
fn the_same_place_in_both_phases() {
    let mut pitch = phase_scene(420.0, 10_000);
    let place = pitch.anchor(FL);
    pitch.stand(FL, place.x - 60.0, place.y - 60.0);
    let walking = pitch.velocity(FL, ForwardState::Walking);
    let alert = pitch.velocity(FL, ForwardState::HoldingShape);
    assert!(
        walking.normalize().dot(&alert.normalize()) > 0.99,
        "{walking:?} against {alert:?}"
    );
}

/// At 40 m a fresh forward is alert and one at 55% condition is relaxed.
#[test]
fn a_tired_forward_is_relaxed_where_a_fresh_one_is_alert() {
    assert_eq!(
        phase_scene(320.0, 10_000).read(FL),
        ForwardState::HoldingShape
    );
    assert_eq!(phase_scene(320.0, 5_500).read(FL), ForwardState::Walking);
}

/// A tired forward still walks back to his place and still takes the press
/// the plan gives him.
#[test]
fn a_tired_forward_is_not_stopped() {
    let mut pitch = phase_scene(420.0, 4_000);
    let place = pitch.anchor(FL);
    pitch.stand(FL, place.x - 80.0, place.y);
    assert!(pitch.velocity(FL, ForwardState::Walking).norm() > 0.0);

    let mut pitch = carrier_scene((600.0, 272.0), (650.0, 272.0));
    pitch.condition(FL, 4_000);
    assert_eq!(
        state_of(&pitch.process(FL, ForwardState::Walking)),
        Some(PlayerState::Forward(ForwardState::Pressing))
    );
}

// ── Getting back ─────────────────────────────────────────────────────────

/// Their back line at 700, the ball lost with him beyond it: back inside,
/// whichever shape or job state he was in.
#[test]
fn a_forward_beyond_their_line_comes_back_when_the_ball_is_lost() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_GK, 830.0, 272.0)
        .stand(AWAY_DCL, 700.0, 300.0)
        .stand(AWAY_MCL, 500.0, 200.0)
        .on_the_ball(AWAY_MCL)
        .stand(HOME_MCL, 490.0, 200.0)
        .stand(FL, 760.0, 272.0)
        .refresh()
        .plan(AWAY_MCL);
    for state in [
        ForwardState::Standing,
        ForwardState::Walking,
        ForwardState::HoldingShape,
        ForwardState::CreatingSpace,
        ForwardState::HoldingWidth,
    ] {
        assert_eq!(
            state_of(&pitch.process(FL, state)),
            Some(PlayerState::Forward(ForwardState::Returning)),
            "{state:?}"
        );
    }
}

/// Getting back past a carrier he is not the man for, he carries on.
#[test]
fn getting_back_is_not_a_press() {
    let mut pitch = Pitch::new();
    pitch.refresh();
    let place = pitch.anchor(FL);
    let at = Vector3::new(place.x + 200.0, place.y, 0.0);
    pitch
        .stand(FL, at.x, at.y)
        .stand(AWAY_DCL, at.x - 60.0, at.y + 40.0)
        .on_the_ball(AWAY_DCL)
        .stand(HOME_MCL, at.x - 70.0, at.y + 40.0)
        .refresh()
        .plan(AWAY_DCL);
    assert!(
        pitch.process(FL, ForwardState::Returning).is_none(),
        "{:?}",
        pitch.read(FL)
    );
}

/// His side wins the ball on his way back and the attack names him for the
/// run: he makes it from where he is.
#[test]
fn the_ball_won_on_the_way_is_the_runs() {
    let mut pitch = possession_scene(600.0);
    pitch.quickest(FL).refresh();
    assert_eq!(pitch.attack().far_runner, Some(FL));
    assert_eq!(
        state_of(&pitch.process(FL, ForwardState::Returning)),
        Some(PlayerState::Forward(ForwardState::RunningInBehind))
    );
}

// ── Closing, containing, challenging ─────────────────────────────────────

/// Named, goal-side and inside `COMMIT`: he contains, whatever his tackle
/// cooldown says.
#[test]
fn a_forward_on_cooldown_contains() {
    let mut pitch = carrier_scene((600.0, 272.0), (588.0, 272.0));
    assert_eq!(pitch.context.defence_home.presser(), Some(FL));
    pitch.field.get_player_mut(FL).unwrap().tackle_cooldown = 50;
    assert_eq!(
        state_of(&pitch.process(FL, ForwardState::Pressing)),
        Some(PlayerState::Forward(ForwardState::Jockeying))
    );
}

/// The licence passes to a midfielder: the press is the read's.
#[test]
fn the_licence_passing_to_another_man_ends_the_press() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_DCL, 600.0, 272.0)
        .on_the_ball(AWAY_DCL)
        .stand(HOME_MCL, 590.0, 272.0)
        .stand(FL, 650.0, 272.0)
        .refresh()
        .plan(AWAY_DCL);
    let read = pitch.read(FL);
    assert_eq!(
        state_of(&pitch.process(FL, ForwardState::Pressing)),
        Some(PlayerState::Forward(read))
    );
    assert_ne!(read, ForwardState::Pressing);
}

/// Containing and declining: he stays, goal-side, on a station off the man
/// rather than on the man.
#[test]
fn a_forward_who_declines_the_challenge_keeps_containing() {
    let mut pitch = carrier_scene((600.0, 272.0), (592.0, 272.0));
    pitch.field.get_player_mut(FL).unwrap().tackle_cooldown = 50;
    pitch.put_in(FL, ForwardState::Jockeying);
    assert!(pitch.process(FL, ForwardState::Jockeying).is_none());
    let (goal_side, station, carrier) = pitch.with_ctx(FL, |ctx| {
        let carrier = ctx.players().opponents().with_ball().next().unwrap();
        (
            ForwardRead::is_goal_side(ctx, &carrier),
            ClosingPoint::for_carrier(ctx, &carrier),
            carrier.position,
        )
    });
    assert!(goal_side);
    assert!((station - carrier).norm() > 1.0, "he steers at the man");
}

#[test]
fn a_carrier_beyond_disengage_is_closed_again() {
    let mut pitch = carrier_scene((600.0, 272.0), (568.0, 272.0));
    assert_eq!(
        state_of(&pitch.process(FL, ForwardState::Jockeying)),
        Some(PlayerState::Forward(ForwardState::Pressing))
    );
}

#[test]
fn a_carrier_who_releases_the_ball_hands_him_to_the_read() {
    let mut pitch = carrier_scene((600.0, 272.0), (592.0, 272.0));
    pitch
        .stand(HOME_MCL, 560.0, 300.0)
        .on_the_ball(HOME_MCL)
        .refresh();
    let read = pitch.read(FL);
    assert_eq!(
        state_of(&pitch.process(FL, ForwardState::Jockeying)),
        Some(PlayerState::Forward(read))
    );
}

/// The challenge is rolled once, on entry, and always leaves: won with the
/// ball, missed and getting up, or a foul.
#[test]
fn the_challenge_is_decided_on_entry() {
    let (mut won, mut missed, mut fouled) = (0, 0, 0);
    for seed in 0..300 {
        let mut pitch = carrier_scene((600.0, 272.0), (592.0, 272.0));
        pitch.context.rng = MatchRng::from_seed(seed);
        pitch.field.get_player_mut(FL).unwrap().staged_challenge = Some(ChallengeKind::Block);
        let change = pitch
            .process(FL, ForwardState::Tackling)
            .expect("the challenge leaves");
        if change.missed_challenge.is_some() {
            missed += 1;
            continue;
        }
        let state = change.state;
        let events = events_of(change);
        if events
            .iter()
            .any(|e| matches!(e, PlayerEvent::TacklingBall(_)))
        {
            assert_eq!(state, Some(PlayerState::Forward(ForwardState::Running)));
            won += 1;
        } else if events
            .iter()
            .any(|e| matches!(e, PlayerEvent::CommitFoul(_)))
        {
            assert_ne!(state, Some(PlayerState::Forward(ForwardState::Tackling)));
            fouled += 1;
        } else {
            panic!("a challenge that neither won, missed nor fouled: {state:?}");
        }
    }
    assert!(
        won > 0 && missed > 0 && fouled > 0,
        "{won} {missed} {fouled}"
    );
}

/// A ball loose beside the duel is not claimed by the challenge.
#[test]
fn a_loose_ball_beside_the_duel_is_the_elections() {
    let mut pitch = carrier_scene((600.0, 272.0), (592.0, 272.0));
    pitch.field.ball.current_owner = None;
    pitch.field.ball.position = Vector3::new(596.0, 280.0, 0.0);
    pitch.field.get_player_mut(FL).unwrap().staged_challenge = Some(ChallengeKind::Block);
    let read = pitch.read(FL);
    let change = pitch
        .process(FL, ForwardState::Tackling)
        .expect("a decision");
    assert_eq!(change.state, Some(PlayerState::Forward(read)));
    assert!(
        !events_of(change)
            .iter()
            .any(|e| matches!(e, PlayerEvent::ClaimBall(_)))
    );
}

// ── Running is the man on the ball ──────────────────────────────────────

#[test]
fn a_forward_without_the_ball_is_not_left_in_running_while_they_have_it() {
    let mut pitch = carrier_scene((500.0, 272.0), (650.0, 272.0));
    let got = state_of(&pitch.process(FL, ForwardState::Running));
    assert!(
        got.is_some() && got != Some(PlayerState::Forward(ForwardState::Running)),
        "{got:?}"
    );
}

// ── One job in possession ────────────────────────────────────────────────

/// The home midfielder on the ball in the middle, two strikers ahead of
/// him, their back two at `line_x` and their keeper on his line.
fn possession_scene(line_x: f32) -> Pitch {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_GK, 830.0, 272.0)
        .stand(AWAY_DCL, line_x, 230.0)
        .stand(AWAY_DCR, line_x, 320.0)
        .stand(HOME_MCL, 420.0, 272.0)
        .on_the_ball(HOME_MCL)
        .stand(FL, 520.0, 220.0)
        .stand(FR, 520.0, 330.0)
        .refresh();
    pitch
}

/// Two strikers, the ball in midfield, 30 m behind their line: one runner,
/// and he has no place in the box.
#[test]
fn one_runner() {
    let pitch = possession_scene(600.0);
    let plan = pitch.attack();
    let runner = plan.far_runner.expect("a runner");
    assert!(plan.slot_of(runner).is_none());
}

/// The quickest man within reach of goal is a striker who would suit the
/// box: he is the runner, and the place goes to somebody else.
#[test]
fn a_striker_can_be_the_runner() {
    let mut pitch = possession_scene(600.0);
    pitch.quickest(FR).refresh();
    let plan = pitch.attack();
    assert_eq!(plan.far_runner, Some(FR));
    assert!(plan.slot_of(FR).is_none());
}

/// Their line on the edge of their own area: nobody runs in behind.
#[test]
fn no_space_no_runner() {
    let pitch = possession_scene(720.0);
    assert_eq!(pitch.attack().far_runner, None);
}

/// The ball in his own third, their line near halfway: a runner is named,
/// and he waits inside the line.
#[test]
fn a_runner_in_the_build_up() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_GK, 830.0, 272.0)
        .stand(AWAY_DCL, 450.0, 230.0)
        .stand(AWAY_DCR, 450.0, 320.0)
        .stand(HOME_DCL, 150.0, 272.0)
        .on_the_ball(HOME_DCL)
        .stand(FL, 420.0, 220.0)
        .stand(FR, 420.0, 330.0)
        .quickest(FL)
        .refresh();
    assert_eq!(pitch.attack().far_runner, Some(FL));
    assert_eq!(pitch.read(FL), ForwardState::RunningInBehind);
    let aim = Vector3::new(520.0, 220.0, 0.0);
    let held = pitch.with_ctx(FL, |ctx| ForwardRunningInBehindState::held(ctx, aim));
    assert!(held.x < 450.0, "he waits beyond their line: {}", held.x);
}

/// The attack on the box begins: a striker with a place attacks it, and the
/// outlet is somebody else.
#[test]
fn a_striker_shows_before_he_attacks_the_box() {
    let mut pitch = possession_scene(600.0);
    pitch.quickest(FR).refresh();
    assert_ne!(
        pitch.attack().near_support,
        Some(FR),
        "the runner is not the outlet"
    );
    pitch
        .stand(HOME_MCL, 800.0, 120.0)
        .on_the_ball(HOME_MCL)
        .refresh();
    let plan = pitch.attack();
    for id in [FL, FR] {
        if plan.slot_of(id).is_some() {
            assert_ne!(plan.near_support, Some(id), "a box man stays the outlet");
            assert!(pitch.with_ctx(id, BoxMovement::stage) > 0.0);
            assert_eq!(pitch.read(id), ForwardState::CrossReceiving);
        }
    }
}

/// A forward the attack gives nothing holds his place, however long, and
/// whatever his pace.
#[test]
fn nobody_goes_on_a_roll() {
    let mut pitch = possession_scene(720.0);
    pitch.quickest(FL).refresh();
    let plan = pitch.attack();
    if plan.near_support == Some(FL) || plan.slot_of(FL).is_some() || plan.far_runner == Some(FL) {
        return;
    }
    for t in [0u64, 50, 200, 1000] {
        pitch.in_state_time = t;
        let state = pitch.read(FL);
        assert!(
            matches!(state, ForwardState::Walking | ForwardState::HoldingShape),
            "{state:?} after {t} AI ticks"
        );
    }
}

/// Winning the ball in midfield sends at most one striker in behind.
#[test]
fn winning_the_ball_does_not_send_everybody() {
    let pitch = possession_scene(600.0);
    let runs = [FL, FR]
        .into_iter()
        .filter(|id| pitch.read(*id) == ForwardState::RunningInBehind)
        .count();
    assert!(runs <= 1, "{runs} strikers in behind");
}

/// Two seconds into showing for the ball, still the outlet: still showing.
#[test]
fn a_job_is_kept() {
    let mut pitch = possession_scene(600.0);
    let plan = pitch.attack();
    let Some(outlet) = [FL, FR]
        .into_iter()
        .find(|id| plan.near_support == Some(*id))
    else {
        return;
    };
    pitch.in_state_time = 100;
    assert!(pitch.process(outlet, ForwardState::CreatingSpace).is_none());
}

/// The ball lost with one striker in behind and the other in the box: on the
/// next tick neither is on his job.
#[test]
fn the_ball_lost_during_a_run_ends_the_run() {
    let mut pitch = possession_scene(600.0);
    pitch
        .stand(AWAY_MCL, 430.0, 272.0)
        .on_the_ball(AWAY_MCL)
        .refresh();
    for (id, job) in [
        (FL, ForwardState::RunningInBehind),
        (FR, ForwardState::CrossReceiving),
    ] {
        let got = state_of(&pitch.process(id, job));
        assert!(
            got.is_some() && got != Some(PlayerState::Forward(job)),
            "{job:?} carried on: {got:?}"
        );
    }
}

// ── The run in behind ────────────────────────────────────────────────────

/// The runner waits inside the line; a pass struck to somebody else does
/// not release him; his marker beside him does not end the run.
#[test]
fn the_runner_waits_inside_the_line() {
    let mut pitch = possession_scene(600.0);
    pitch.quickest(FL).refresh();
    assert_eq!(pitch.attack().far_runner, Some(FL));
    let aim = Vector3::new(680.0, 220.0, 0.0);
    let held = pitch.with_ctx(FL, |ctx| ForwardRunningInBehindState::held(ctx, aim));
    assert!(
        held.x < 600.0,
        "a well-timed runner waits beyond the line: {}",
        held.x
    );

    // A pass struck to FR, travelling his way, changes nothing about his
    // run; one struck to him releases it.
    pitch.put_in(FL, ForwardState::RunningInBehind);
    let run = |pitch: &Pitch| {
        pitch.with_ctx(FL, |ctx| {
            ForwardRunningInBehindState::default()
                .velocity(ctx)
                .unwrap()
        })
    };
    let waiting = run(&pitch);
    pitch.field.ball.current_owner = None;
    pitch.field.ball.velocity = Vector3::new(1.5, -0.3, 0.0);
    pitch.field.ball.pass_target_player_id = Some(FR);
    assert!(
        (run(&pitch) - waiting).norm() < 1e-5,
        "released by somebody else's pass"
    );
    pitch.field.ball.pass_target_player_id = Some(FL);
    assert!(
        (run(&pitch) - waiting).norm() > 1e-3,
        "not released by his own"
    );

    // The last defender within 3 m: still the run.
    pitch
        .stand(HOME_MCL, 420.0, 272.0)
        .on_the_ball(HOME_MCL)
        .stand(AWAY_DCL, 540.0, 225.0)
        .refresh();
    assert!(pitch.process(FL, ForwardState::RunningInBehind).is_none());
}

// ── The box and the outlet ───────────────────────────────────────────────

/// A forward with a place in the box steers by `BoxMovement`, and one with
/// no staged place is never sent there.
#[test]
fn a_box_place_is_attacked_by_box_movement() {
    let mut pitch = possession_scene(600.0);
    pitch
        .stand(HOME_MCL, 780.0, 110.0)
        .on_the_ball(HOME_MCL)
        .refresh();
    let plan = pitch.attack();
    for id in [FL, FR] {
        let staged = pitch.with_ctx(id, BoxMovement::stage) > 0.0;
        let in_box = pitch.read(id) == ForwardState::CrossReceiving;
        assert!(!in_box || (plan.slot_of(id).is_some() && staged));
        if let Some(slot) = plan.slot_of(id) {
            pitch.put_in(id, ForwardState::CrossReceiving);
            let want = pitch.with_ctx(id, |ctx| BoxMovement::steer(ctx, slot));
            let got = pitch.with_ctx(id, |ctx| {
                ForwardCrossReceivingState::default().velocity(ctx).unwrap()
            });
            assert!((want - got).norm() < 1e-4);
        }
    }
}

/// **The runners move for the byline ball.** With a team-mate carrying the
/// ball inside the box at the byline, the near-post runner goes to the near
/// post and the edge runner to the edge of the area, instead of holding the
/// places they took for a cross from deep.
#[test]
fn the_runners_move_for_the_byline_ball() {
    let target = |at: (f32, f32), slot: BoxSlot| {
        let mut pitch = possession_scene(600.0);
        pitch
            .stand(HOME_MCL, at.0, at.1)
            .on_the_ball(HOME_MCL)
            .refresh();
        pitch.with_ctx(FL, |ctx| BoxMovement::target(ctx, slot))
    };
    let at_the_byline = (830.0, 170.0);
    let from_deep = (700.0, 40.0);
    let near_post = target(at_the_byline, BoxSlot::NearPost);
    assert!(
        840.0 - near_post.x <= 56.0,
        "within 7 m of the line: {near_post:?}"
    );
    assert!(near_post.y < 272.0, "on the carrier's side: {near_post:?}");
    let waiting = target(from_deep, BoxSlot::NearPost);
    assert!(
        840.0 - waiting.x > 56.0,
        "for a cross from deep he waits further out: {waiting:?}"
    );
    let edge = target(at_the_byline, BoxSlot::CutbackEdge);
    assert!(
        (104.0..=152.0).contains(&(840.0 - edge.x)),
        "at the edge: {edge:?}"
    );
}

/// The outlet steers at the offer, 9.5–20 m off the carrier; the other
/// striker does not steer at the same point.
#[test]
fn the_outlet_shows_at_an_angle_and_a_distance() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_GK, 830.0, 272.0)
        .stand(AWAY_DCL, 600.0, 230.0)
        .stand(AWAY_DCR, 600.0, 320.0)
        .stand(HOME_MCR, 430.0, 272.0)
        .on_the_ball(HOME_MCR)
        .stand(FL, 470.0, 250.0)
        .stand(FR, 470.0, 295.0)
        .refresh();
    let plan = pitch.attack();
    let Some(outlet) = [FL, FR]
        .into_iter()
        .find(|id| plan.near_support == Some(*id))
    else {
        return;
    };
    let carrier = pitch.field.get_player(HOME_MCR).unwrap().position;
    let target = pitch
        .with_ctx(outlet, |ctx| SupportOffer::target(ctx, carrier))
        .expect("an offer");
    let gap = (target - carrier).norm();
    assert!((76.0..=160.0).contains(&gap), "{gap} units off the carrier");
    let other = if outlet == FL { FR } else { FL };
    let theirs = pitch.with_ctx(other, |ctx| ctx.team().my_anchor());
    assert!((theirs - target).norm() > 8.0, "both strikers on one point");
}

// ── Pace ──────────────────────────────────────────────────────────────────

/// Metres a second in a unit a tick.
const MS: f32 = 12.5;

/// On his place with the opposition on the ball 50 m away, he walks.
#[test]
fn play_far_away_is_walked() {
    let pitch = phase_scene(400.0, 10_000);
    let place = pitch.anchor(FL);
    let pace = pitch.with_ctx(FL, |ctx| ShapeTempo::forward(ctx, place).pace());
    assert!(pace * MS <= 2.0, "{} m/s", pace * MS);
}

/// With the ball dead and time before the restart, 8 m from his place he
/// walks into it.
#[test]
fn with_the_ball_dead_he_walks_into_his_place() {
    let mut pitch = Pitch::new();
    pitch.stand(AWAY_MCL, 600.0, 545.0).on_the_ball(AWAY_MCL);
    pitch.field.ball.current_owner = None;
    let now = pitch.context.current_tick();
    pitch.field.ball.awaiting_restart = Some(AwaitedRestart {
        taker_id: AWAY_MCL,
        spot: Vector3::new(600.0, 545.0, 0.0),
        take_from: None,
        settled: true,
        carrying: false,
        origin: PassOriginRestart::ThrowIn,
        awarded_tick: now,
        patience_ticks: AwaitedRestart::PATIENCE_TICKS,
        settled_tick: None,
        hold: None,
        quiet: RestartQuiet::default(),
    });
    pitch.refresh();
    let place = pitch.anchor(FL);
    pitch.stand(FL, place.x + 64.0, place.y);
    let pace = pitch.with_ctx(FL, |ctx| ShapeTempo::forward(ctx, place).pace());
    assert!(pace * MS <= 1.6, "{} m/s", pace * MS);
}

/// Two equally fresh forwards without the ball, one four seconds into
/// `Running` and one just in it, move at the same speed.
#[test]
fn time_in_a_state_is_not_tiredness() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_DCL, 420.0, 272.0)
        .on_the_ball(AWAY_DCL)
        .refresh();
    let place = pitch.anchor(FL);
    pitch.stand(FL, place.x - 80.0, place.y);
    pitch.put_in(FL, ForwardState::Running);
    let at = |pitch: &mut Pitch, t: u64| {
        pitch.in_state_time = t;
        pitch.with_ctx(FL, |ctx| {
            ForwardRunningState::default().velocity(ctx).unwrap()
        })
    };
    let fresh = at(&mut pitch, 0);
    let long = at(&mut pitch, 200);
    assert!(fresh.norm() > 0.1, "he stood still: {fresh:?}");
    assert!((fresh - long).norm() < 1e-5, "{fresh:?} against {long:?}");
}

/// Level with their line, the line walking up, no ball played to him: he
/// walks with it. A ball played to him: he goes at full effort.
#[test]
fn the_runner_walks_with_the_line_and_breaks_on_the_ball() {
    let mut pitch = possession_scene(600.0);
    pitch.quickest(FL).refresh();
    assert_eq!(pitch.attack().far_runner, Some(FL));
    for id in [AWAY_DCL, AWAY_DCR] {
        pitch.field.get_player_mut(id).unwrap().velocity = Vector3::new(-0.1, 0.0, 0.0);
    }
    // On his station: twice, as his marker reads where he stands.
    pitch.put_in(FL, ForwardState::RunningInBehind);
    for _ in 0..2 {
        let station = pitch
            .with_ctx(FL, ForwardRunningInBehindState::station)
            .expect("a line to hold");
        pitch.stand(FL, station.x, station.y);
    }
    let waiting = pitch.with_ctx(FL, |ctx| {
        ForwardRunningInBehindState::default()
            .velocity(ctx)
            .unwrap()
    });
    assert!(
        waiting.norm() * MS <= 2.5,
        "{} m/s on the shoulder",
        waiting.norm() * MS
    );

    pitch.field.ball.current_owner = None;
    pitch.field.ball.pass_target_player_id = Some(FL);
    pitch.field.ball.velocity = Vector3::new(1.5, -0.3, 0.0);
    let top = pitch.top_speed(FL);
    let breaking = pitch.with_ctx(FL, |ctx| {
        ForwardRunningInBehindState::default()
            .velocity(ctx)
            .unwrap()
    });
    assert!(
        breaking.norm() >= 0.95 * top,
        "{} against a top speed of {top}",
        breaking.norm()
    );
}

/// Attacking his place in the box, he may reach his full effort: the state
/// declares the top tier, where it capped him at a jog.
#[test]
fn a_cross_is_attacked_at_full_effort() {
    let mut pitch = Pitch::new();
    pitch.stand(FL, 760.0, 272.0).refresh();
    let conditions = pitch.context.conditions.clone();
    let forward = pitch.field.get_player_mut(FL).unwrap();
    ForwardCrossReceivingState::default().process_conditions(ConditionContext {
        in_state_time: 10,
        player: forward,
        conditions: &conditions,
        match_progress: 0.3,
        side_on_ball: true,
        ball_dead: false,
    });
    assert!(matches!(
        forward.last_activity_intensity,
        ActivityIntensity::VeryHigh
    ));
}

// ── The duel ─────────────────────────────────────────────────────────────

/// Two equal forwards make the same block challenge, one arriving at a
/// sprint and one set: the set one wins more and fouls less.
#[test]
fn diving_in_costs() {
    let tally = |closing: f32| {
        let (mut won, mut fouled) = (0, 0);
        for seed in 0..600 {
            let mut pitch = carrier_scene((600.0, 272.0), (592.0, 272.0));
            pitch.context.rng = MatchRng::from_seed(seed);
            let man = pitch.field.get_player_mut(FL).unwrap();
            man.staged_challenge = Some(ChallengeKind::Block);
            man.velocity = Vector3::new(closing, 0.0, 0.0);
            let change = pitch.process(FL, ForwardState::Tackling).unwrap();
            let events = events_of(change);
            if events
                .iter()
                .any(|e| matches!(e, PlayerEvent::TacklingBall(_)))
            {
                won += 1;
            }
            if events
                .iter()
                .any(|e| matches!(e, PlayerEvent::CommitFoul(_)))
            {
                fouled += 1;
            }
        }
        (won, fouled)
    };
    let (set_won, set_fouled) = tally(0.0);
    let (rushed_won, rushed_fouled) = tally(0.55);
    assert!(
        set_won > rushed_won,
        "won {set_won} set, {rushed_won} rushed"
    );
    assert!(
        set_fouled < rushed_fouled,
        "fouled {set_fouled} set, {rushed_fouled} rushed"
    );
}

/// A carrier who has gone past him: he closes again, and the only challenge
/// open to him is the stretch from behind.
#[test]
fn a_carrier_who_gets_goal_side_is_chased() {
    let mut pitch = carrier_scene((600.0, 272.0), (606.0, 272.0));
    assert_eq!(
        state_of(&pitch.process(FL, ForwardState::Jockeying)),
        Some(PlayerState::Forward(ForwardState::Pressing))
    );
    for seed in 0..200 {
        pitch.context.rng = MatchRng::from_seed(seed);
        if let Some(change) = pitch.process(FL, ForwardState::Pressing) {
            assert_ne!(
                change.state,
                Some(PlayerState::Forward(ForwardState::Jockeying)),
                "contained a man already past him"
            );
        }
    }
}

/// **The byline strip is the wide area.** A carrier who has come inside
/// along the goal line is still a wide carrier — 3 m off the line inside
/// the box's width — and one 10 m off it is not.
#[test]
fn a_carrier_inside_the_box_at_the_byline_is_still_wide() {
    let mut pitch = Pitch::new();
    pitch.stand(FL, 840.0 - 24.0, 272.0 + 120.0);
    assert!(pitch.with_ctx(FL, CrossModel::is_in_wide_position));
    pitch.stand(FL, 840.0 - 80.0, 272.0 + 120.0);
    assert!(!pitch.with_ctx(FL, CrossModel::is_in_wide_position));
}

/// At the byline with the goal line open toward the near post, he carries
/// on along it to the corner of the six-yard box.
#[test]
fn a_winger_with_the_goal_line_open_goes_into_the_box() {
    let mut pitch = Pitch::new();
    pitch.stand(FL, 840.0 - 44.0, 272.0 + 165.0).on_the_ball(FL);
    let aim = pitch
        .with_ctx(FL, |ctx| FlankPlay::carry_aim(ctx, 0.0))
        .expect("a carry");
    let across = (aim.y - 272.0).abs();
    assert!(
        (73.0..100.0).contains(&across),
        "to the six-yard box's corner, not {across}"
    );
    assert!(840.0 - aim.x < 30.0, "along the goal line");
}

/// A defender on the goal line a stride or two from him: he does not run
/// into him — the carry turns back to the corner of the area.
#[test]
fn a_defender_on_the_goal_line_stops_the_carry() {
    let mut pitch = Pitch::new();
    pitch.stand(FL, 840.0 - 44.0, 272.0 + 165.0).on_the_ball(FL);
    pitch.stand(AWAY_DCR, 840.0 - 30.0, 272.0 + 140.0);
    let aim = pitch
        .with_ctx(FL, |ctx| FlankPlay::carry_aim(ctx, 0.0))
        .expect("a carry");
    assert!(
        (aim.y - 272.0).abs() > 150.0,
        "not into the man on the line"
    );
}

/// A carrier with the inside lane open may still cut in.
#[test]
fn an_open_inside_lane_is_still_his_choice() {
    let mut pitch = Pitch::new();
    pitch.stand(FL, 840.0 - 44.0, 272.0 + 165.0).on_the_ball(FL);
    assert!(
        pitch
            .with_ctx(FL, |ctx| FlankPlay::carry_aim(ctx, 1.0))
            .is_none()
    );
}

/// Inside the box at the byline, the defenders dropped onto the six-yard
/// line and a team-mate free at the edge: the ball is pulled back to him.
#[test]
fn a_free_runner_at_the_edge_gets_the_pull_back() {
    let mut pitch = Pitch::new();
    pitch.stand(FL, 840.0 - 20.0, 272.0 + 110.0).on_the_ball(FL);
    pitch.stand(FR, 840.0 - 125.0, 272.0 + 20.0);
    pitch.stand(AWAY_DCL, 840.0 - 44.0, 272.0);
    pitch.stand(AWAY_DCR, 840.0 - 44.0, 272.0 - 30.0);
    pitch.stand(AWAY_GK, 836.0, 272.0);
    let decision = pitch.with_ctx(FL, CrossModel::pick).expect("a delivery");
    assert_eq!(decision.target_id, FR);
    assert_eq!(decision.cross_type, CrossType::Cutback);
}

/// …and with the edge covered and a team-mate attacking the near post, it
/// is driven across the six-yard box in front of the keeper.
#[test]
fn a_crowded_edge_gets_the_ball_across_the_six_yard_box() {
    let mut pitch = Pitch::new();
    pitch.stand(FL, 840.0 - 20.0, 272.0 + 110.0).on_the_ball(FL);
    pitch.stand(FR, 840.0 - 30.0, 272.0 + 40.0);
    pitch.stand(AWAY_DCL, 750.0, 300.0);
    pitch.stand(AWAY_DCR, 755.0, 325.0);
    pitch.stand(AWAY_MCL, 725.0, 300.0);
    pitch.stand(AWAY_GK, 836.0, 272.0);
    let decision = pitch.with_ctx(FL, CrossModel::pick).expect("a delivery");
    assert_eq!(decision.target_id, FR);
    assert_eq!(decision.cross_type, CrossType::DrivenLowCross);
    assert!(
        840.0 - decision.aim_point.x < 44.0,
        "across the face of the six-yard box"
    );
}

/// From the byline one man arriving in the box is enough to deliver to.
#[test]
fn one_runner_is_enough_from_the_byline() {
    let mut pitch = Pitch::new();
    pitch.stand(FL, 840.0 - 20.0, 272.0 + 110.0).on_the_ball(FL);
    pitch.stand(FR, 840.0 - 60.0, 272.0 - 20.0);
    assert!(pitch.with_ctx(FL, CrossModel::pick).is_some());
}

/// **The winner runs to meet the ball where his head can.** With a delivery
/// on its way to him he goes to where it comes down through heading
/// height, not to where it would land.
#[test]
fn the_winner_goes_to_where_the_delivery_comes_down_through_heading_height() {
    use crate::r#match::engine::ball::ball::{AerialDelivery, AerialOutcome};
    let mut pitch = Pitch::new();
    pitch.stand(FL, 760.0, 272.0);
    pitch.ball_in_the_air(
        Vector3::new(700.0, 272.0, 4.0),
        Vector3::new(1.5, 0.0, -0.05),
    );
    let meet = Vector3::new(752.0, 268.0, 2.2);
    pitch.field.ball.aerial_delivery = Some(AerialDelivery {
        winner_id: FL,
        target: meet,
        outcome: AerialOutcome::Header {
            drift: Vector3::new(0.12, 0.0, -0.02),
        },
        arrival_height: 2.2,
        deadline_tick: 10_000,
        force_heading: true,
    });
    let point = pitch.with_ctx(FL, WonHeader::meeting_point);
    assert_eq!((point.x, point.y), (meet.x, meet.y));
}
