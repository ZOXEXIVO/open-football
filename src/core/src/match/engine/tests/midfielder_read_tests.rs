//! **One read of the game for every midfielder without the ball.** The same
//! situation, put to a midfielder in each state he can hold without it, gets
//! one answer; one assignment chooses who closes a carrier; the phase is his
//! readiness on one place; the rungs of the duel hand to one another on the
//! spec's terms.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::common_states::{ChallengeKind, ShapeTempo, TackleEngagement};
use crate::r#match::defenders::states::DefenderMarkingState;
use crate::r#match::engine::ball::ball::{
    AwaitedRestart, OffsideLine, PassOriginRestart, RestartQuiet,
};
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::result::Score;
use crate::r#match::engine::teamplay::possession::TeamPossession;
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::common::MidfielderRead;
use crate::r#match::midfielders::states::{
    MidfielderAttackSupportingState, MidfielderCreatingSpaceState, MidfielderGuardingState,
    MidfielderHeadingState, MidfielderHoldingShapeState, MidfielderHoldingWidthState,
    MidfielderJockeyingState, MidfielderPressingState, MidfielderReturningState,
    MidfielderRunningState, MidfielderStandingState, MidfielderState, MidfielderTacklingState,
    MidfielderTrackingBackState, MidfielderWalkingState,
};
use crate::r#match::player::events::{FoulSource, PlayerEvent};
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::players::ops::support::SupportOffer;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    ActivityIntensity, ConditionContext, DefenceRefreshInputs, DefensivePlan, GameTickContext,
    MatchContext, MatchField, MatchPlayerCollection, MovementEffort, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const HOME: u32 = 1;
const AWAY: u32 = 2;
const DCL: u32 = 102;
const ML: u32 = 105;
const MCL: u32 = 106;
const MCR: u32 = 107;
const MR: u32 = 108;
const HOME_MIDFIELD: [u32; 4] = [ML, MCL, MCR, MR];
const AWAY_MCL: u32 = 206;
const AWAY_MR: u32 = 208;
const AWAY_FL: u32 = 209;
const AWAY_FR: u32 = 210;

/// Every state a midfielder holds without the ball, below the loose-ball
/// rung.
const OFF_BALL: [MidfielderState; 11] = [
    MidfielderState::Standing,
    MidfielderState::Walking,
    MidfielderState::HoldingShape,
    MidfielderState::Returning,
    MidfielderState::TrackingBack,
    MidfielderState::Guarding,
    MidfielderState::Pressing,
    MidfielderState::Jockeying,
    MidfielderState::AttackSupporting,
    MidfielderState::CreatingSpace,
    MidfielderState::HoldingWidth,
];

/// Home on the left, defending x = 0, ten minutes in. Everyone parked at the
/// far end until a test stands him somewhere.
struct Pitch {
    field: MatchField,
    context: MatchContext,
    /// How long the man asked has been in his state, in AI ticks.
    in_state_time: u64,
}

impl Pitch {
    fn new() -> Self {
        let home = squad(HOME, 100);
        let away = squad(AWAY, 200);
        let players = MatchPlayerCollection::from_squads(&home, &away);
        let mut field = MatchField::new(840, 545, home, away);
        assert_eq!(field.side_of(HOME), PlayerSide::Left);
        field.ball.kickoff_taker = None;
        // Long after the last strike, so every man has read it and the
        // chase election has its rows.
        field.ball.current_tick_cached = 1_000;
        for (index, p) in field.players.iter_mut().enumerate() {
            p.velocity = Vector3::zeros();
            let x = if p.team_id == HOME { 780.0 } else { 820.0 };
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

    /// Both sides in a 4-4-2 block, lines 12.5 m apart: home's back four at
    /// `home_back`, the away back four at `away_back`, each keeper on his
    /// line.
    fn blocks(&mut self, home_back: f32, away_back: f32) -> &mut Self {
        const ACROSS: [f32; 4] = [90.0, 210.0, 335.0, 455.0];
        for (team, base, back, ahead) in [
            (HOME, 100, home_back, 100.0),
            (AWAY, 200, away_back, -100.0),
        ] {
            let keeper_x = if team == HOME { 20.0 } else { 820.0 };
            self.stand(base, keeper_x, 272.0);
            for (i, y) in ACROSS.iter().enumerate() {
                self.stand(base + 1 + i as u32, back, *y);
                self.stand(base + 5 + i as u32, back + ahead, *y);
            }
            self.stand(base + 9, back + 2.0 * ahead, ACROSS[1]);
            self.stand(base + 10, back + 2.0 * ahead, ACROSS[2]);
        }
        self
    }

    /// Both sides' tactical states and plans as the engine refreshes them,
    /// ten seconds into the possession: past the transition window.
    fn refresh(&mut self) -> &mut Self {
        for _ in 0..10 {
            self.refresh_after(100);
        }
        self
    }

    /// …and the tick the ball was won.
    fn refresh_won(&mut self) -> &mut Self {
        self.refresh_after(1)
    }

    fn refresh_after(&mut self, ticks: u32) -> &mut Self {
        let possession = TeamPossession::read(&self.field);
        FootballEngine::<840, 545>::refresh_tactical_states(
            &mut self.field,
            &mut self.context,
            possession,
            ticks,
        );
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

    fn put_in(&mut self, id: u32, state: MidfielderState) {
        self.field
            .get_player_mut(id)
            .unwrap()
            .transition_to(PlayerState::Midfielder(state), TransitionSource::Reset);
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

    fn process(&mut self, id: u32, state: MidfielderState) -> Option<StateChangeResult> {
        self.put_in(id, state);
        self.with_ctx(id, |ctx| match state {
            MidfielderState::Standing => MidfielderStandingState::default().process(ctx),
            MidfielderState::Walking => MidfielderWalkingState::default().process(ctx),
            MidfielderState::HoldingShape => MidfielderHoldingShapeState::default().process(ctx),
            MidfielderState::Returning => MidfielderReturningState::default().process(ctx),
            MidfielderState::TrackingBack => MidfielderTrackingBackState::default().process(ctx),
            MidfielderState::Guarding => MidfielderGuardingState::default().process(ctx),
            MidfielderState::Pressing => MidfielderPressingState::default().process(ctx),
            MidfielderState::Jockeying => MidfielderJockeyingState::default().process(ctx),
            MidfielderState::AttackSupporting => {
                MidfielderAttackSupportingState::default().process(ctx)
            }
            MidfielderState::CreatingSpace => MidfielderCreatingSpaceState::default().process(ctx),
            MidfielderState::HoldingWidth => MidfielderHoldingWidthState::default().process(ctx),
            MidfielderState::Running => MidfielderRunningState::default().process(ctx),
            MidfielderState::Tackling => MidfielderTacklingState::default().process(ctx),
            MidfielderState::Heading => MidfielderHeadingState::default().process(ctx),
            other => panic!("no handler wired for {other:?}"),
        })
    }

    fn velocity(&mut self, id: u32, state: MidfielderState) -> Vector3<f32> {
        self.put_in(id, state);
        self.with_ctx(id, |ctx| match state {
            MidfielderState::Walking => MidfielderWalkingState::default().velocity(ctx),
            MidfielderState::HoldingShape => MidfielderHoldingShapeState::default().velocity(ctx),
            MidfielderState::Guarding => MidfielderGuardingState::default().velocity(ctx),
            MidfielderState::Jockeying => MidfielderJockeyingState::default().velocity(ctx),
            MidfielderState::AttackSupporting => {
                MidfielderAttackSupportingState::default().velocity(ctx)
            }
            MidfielderState::CreatingSpace => MidfielderCreatingSpaceState::default().velocity(ctx),
            other => panic!("no handler wired for {other:?}"),
        })
        .expect("a velocity")
    }

    /// Where he goes from `state`: the state handed to, or `state` itself.
    fn answer(&mut self, id: u32, state: MidfielderState) -> MidfielderState {
        match self.process(id, state).and_then(|change| change.state) {
            None => state,
            Some(PlayerState::Midfielder(next)) => next,
            Some(other) => panic!("{state:?} handed a midfielder to {other:?}"),
        }
    }

    /// The read's answer, and every off-ball state's, which must be it.
    fn one_answer(&mut self, id: u32) -> MidfielderState {
        self.put_in(id, MidfielderState::Standing);
        let read = self.with_ctx(id, MidfielderRead::resolve);
        for state in OFF_BALL {
            assert_eq!(self.answer(id, state), read, "from {state:?}");
        }
        read
    }

    fn anchor(&self, id: u32) -> Vector3<f32> {
        self.with_ctx(id, |ctx| ctx.team().my_anchor())
    }
}

// ── One answer from every state ─────────────────────────────────────────

#[test]
fn a_carrier_in_reach_is_contained_from_every_state() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 300.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(MCL, 290.0, 272.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(MCL));
    assert_eq!(pitch.one_answer(MCL), MidfielderState::Jockeying);
}

#[test]
fn a_carrier_near_a_midfielder_who_is_not_the_man_for_him_is_left() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 300.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(DCL, 294.0, 272.0)
        .stand(MCL, 300.0, 336.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(DCL));
    for state in OFF_BALL {
        let next = pitch.answer(MCL, state);
        assert!(
            !matches!(
                next,
                MidfielderState::Pressing | MidfielderState::Jockeying | MidfielderState::Tackling
            ),
            "from {state:?} he closed a carrier 8 m away who is not his: {next:?}"
        );
    }
}

#[test]
fn a_marked_man_is_marked_from_every_state() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_MCL, 600.0, 272.0)
        .on_the_ball(AWAY_MCL)
        .stand(MCR, 590.0, 272.0)
        .stand(AWAY_FL, 420.0, 200.0)
        .stand(MCL, 440.0, 230.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.mark_of(MCL), Some(AWAY_FL));
    assert_eq!(pitch.one_answer(MCL), MidfielderState::Guarding);
}

/// Game units a tick, in metres a second.
const MS: f32 = 12.5;

#[test]
fn the_marker_of_a_walking_man_walks() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_MCL, 600.0, 272.0)
        .on_the_ball(AWAY_MCL)
        .stand(MCR, 590.0, 272.0)
        .stand(AWAY_FL, 420.0, 200.0)
        .stand(MCL, 440.0, 230.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.mark_of(MCL), Some(AWAY_FL));
    // His man strolls across the pitch at 1.25 m/s; the marker is walked
    // along with him until he is in position.
    let stroll = Vector3::new(0.0, 0.1, 0.0);
    pitch.field.get_player_mut(AWAY_FL).unwrap().velocity = stroll;
    let mut speed = 0.0;
    for _ in 0..120 {
        let v = pitch.velocity(MCL, MidfielderState::Guarding);
        speed = v.xy().norm();
        let marker = pitch.field.get_player_mut(MCL).unwrap();
        marker.position += Vector3::new(v.x, v.y, 0.0);
        marker.velocity = v;
        pitch.field.get_player_mut(AWAY_FL).unwrap().position += stroll;
    }
    assert!(
        speed * MS <= 2.0,
        "{} m/s on a man walking at 1.25",
        speed * MS
    );
}

// ── The pace the play asks for ───────────────────────────────────────────

#[test]
fn far_from_the_play_he_walks() {
    // His own side on the ball 50 m from his place.
    let mut pitch = Pitch::new();
    pitch.stand(109, 700.0, 230.0).on_the_ball(109).refresh();
    let place = pitch.anchor(MCL);
    pitch.field.get_player_mut(109).unwrap().position.x = place.x + 400.0;
    pitch.field.ball.position.x = place.x + 400.0;
    for off in [0.0, 80.0] {
        pitch.stand(MCL, place.x - off, place.y);
        let pace = pitch.with_ctx(MCL, |ctx| ShapeTempo::midfielder(ctx, place).pace());
        assert!(
            pace * MS <= 2.0,
            "{} m/s from {} m off his place",
            pace * MS,
            off / 8.0
        );
    }
}

#[test]
fn with_the_ball_in_behind_him_he_runs_back_at_full_effort() {
    let mut pitch = Pitch::new();
    pitch
        .stand(MCL, 400.0, 272.0)
        .stand(AWAY_FL, 300.0, 272.0)
        .on_the_ball(AWAY_FL)
        .plan(AWAY);
    for state in [
        MidfielderState::Walking,
        MidfielderState::HoldingShape,
        MidfielderState::TrackingBack,
    ] {
        pitch.put_in(MCL, state);
        let place = pitch.anchor(MCL);
        let effort = pitch.with_ctx(MCL, |ctx| ShapeTempo::midfielder(ctx, place).effort());
        assert_eq!(effort, 1.0, "from {state:?}");
    }
}

#[test]
fn with_the_ball_dead_he_walks_into_his_place() {
    let mut pitch = Pitch::new();
    pitch.stand(AWAY_MR, 600.0, 545.0).on_the_ball(AWAY_MR);
    pitch.field.ball.current_owner = None;
    let now = pitch.context.current_tick();
    pitch.field.ball.awaiting_restart = Some(AwaitedRestart {
        taker_id: AWAY_MR,
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
    pitch.put_in(MCL, MidfielderState::Returning);
    let place = pitch.anchor(MCL);
    pitch.stand(MCL, place.x + 64.0, place.y);
    let pace = pitch.with_ctx(MCL, |ctx| ShapeTempo::midfielder(ctx, place).pace());
    assert!(
        pace * MS <= 1.6,
        "{} m/s, 8 m from his place with the ball dead",
        pace * MS
    );
}

#[test]
fn out_of_his_place_with_no_job_he_tracks_back_from_every_state() {
    let mut pitch = Pitch::new();
    let place = pitch.anchor(MCL);
    pitch
        .stand(AWAY_MCL, 600.0, 60.0)
        .on_the_ball(AWAY_MCL)
        .stand(MCR, 595.0, 70.0)
        .stand(MCL, place.x + 200.0, place.y)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(MCR));
    assert_eq!(pitch.one_answer(MCL), MidfielderState::TrackingBack);
}

/// The election gives this one to nobody he can beat to it, so it is the
/// other half of the loose-ball rung: a ball in flight coming to him.
#[test]
fn a_ball_coming_to_him_is_met_from_every_state() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_MR, 600.0, 272.0)
        .on_the_ball(AWAY_MR)
        .stand(MCL, 300.0, 300.0)
        .plan(AWAY);
    let at = pitch.field.get_player(MCL).unwrap().position;
    let (start, end) = (
        at + Vector3::new(60.0, 0.0, 0.0),
        at - Vector3::new(50.0, 0.0, 0.0),
    );
    let ball = &mut pitch.field.ball;
    ball.position = start;
    ball.velocity = (end - start).normalize() * 2.0;
    ball.current_owner = None;
    ball.pass_target_player_id = Some(AWAY_MR);
    ball.cached_landing_position = start;
    ball.flags.in_flight_state = 10;
    ball.pending_pass_aim = Some(end);
    ball.current_tick_cached = 1_000;
    ball.last_release_tick = 0;
    let answer = pitch.one_answer(MCL);
    assert!(
        matches!(
            answer,
            MidfielderState::Intercepting | MidfielderState::TakeBall
        ),
        "{answer:?}"
    );
}

#[test]
fn the_ball_lost_sends_nobody_on_with_his_attacking_job() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_MCL, 300.0, 272.0)
        .on_the_ball(AWAY_MCL)
        .stand(MCL, 520.0, 200.0);
    for state in [
        MidfielderState::AttackSupporting,
        MidfielderState::CreatingSpace,
        MidfielderState::HoldingWidth,
    ] {
        pitch.put_in(MCL, state);
        let read = pitch.with_ctx(MCL, MidfielderRead::resolve);
        assert_eq!(pitch.answer(MCL, state), read, "from {state:?}");
        assert_ne!(read, state);
    }
}

// ── One assignment ──────────────────────────────────────────────────────

#[test]
fn the_best_chaser_does_not_close_another_mans_carrier() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 300.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(MCR, 296.0, 290.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(MCR));
    pitch.stand(MCL, 300.0, 260.0);
    assert!(
        pitch.with_ctx(MCL, |ctx| ctx.team().is_best_player_to_chase_ball()),
        "he would win the race to the carrier's feet"
    );
    for state in OFF_BALL {
        let next = pitch.answer(MCL, state);
        assert!(
            !matches!(next, MidfielderState::Pressing | MidfielderState::Jockeying),
            "from {state:?} he took the presser's carrier: {next:?}"
        );
    }
}

#[test]
fn nobody_named_the_best_placed_closes_him() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 300.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(MCL, 330.0, 272.0);
    assert_eq!(pitch.context.defence_home.presser(), None);
    assert_eq!(
        pitch.answer(MCL, MidfielderState::Walking),
        MidfielderState::Pressing
    );
}

// ── Readiness ───────────────────────────────────────────────────────────

#[test]
fn far_from_the_play_he_is_relaxed() {
    let mut pitch = Pitch::new();
    let place = pitch.anchor(MCL);
    pitch
        .stand(AWAY_MCL, place.x + 400.0, 272.0)
        .on_the_ball(AWAY_MCL)
        .stand(MCL, place.x, place.y);
    assert_eq!(
        pitch.with_ctx(MCL, MidfielderRead::resolve),
        MidfielderState::Walking
    );
}

#[test]
fn the_ball_lost_near_him_has_him_alert() {
    let mut pitch = Pitch::new();
    let place = pitch.anchor(MCL);
    pitch
        .stand(AWAY_MCL, place.x + 280.0, place.y)
        .on_the_ball(AWAY_MCL)
        .stand(MCL, place.x, place.y);
    for state in [MidfielderState::Walking, MidfielderState::HoldingShape] {
        assert_eq!(
            pitch.answer(MCL, state),
            MidfielderState::HoldingShape,
            "from {state:?}"
        );
    }
}

#[test]
fn the_same_place_in_both_phases() {
    let mut pitch = Pitch::new();
    let place = pitch.anchor(MCL);
    pitch
        .stand(AWAY_MCL, place.x + 280.0, place.y)
        .on_the_ball(AWAY_MCL)
        .stand(MCL, place.x - 60.0, place.y - 30.0);
    let to_place = (place - Vector3::new(place.x - 60.0, place.y - 30.0, 0.0)).normalize();
    for state in [MidfielderState::Walking, MidfielderState::HoldingShape] {
        let v = pitch.velocity(MCL, state);
        assert!(
            v.normalize().dot(&to_place) > 0.99,
            "{state:?} steers somewhere else: {v:?}"
        );
    }
}

// ── Recovery ────────────────────────────────────────────────────────────

#[test]
fn with_the_ball_behind_him_he_tracks_back() {
    let mut pitch = Pitch::new();
    let place = pitch.anchor(MCL);
    pitch
        .stand(AWAY_MCL, place.x - 40.0, 480.0)
        .on_the_ball(AWAY_MCL)
        .stand(DCL, place.x - 50.0, 470.0)
        .stand(MCL, place.x + 200.0, place.y)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(DCL));
    assert_eq!(
        pitch.with_ctx(MCL, MidfielderRead::resolve),
        MidfielderState::TrackingBack
    );
}

// ── Marking ─────────────────────────────────────────────────────────────

#[test]
fn a_man_given_to_another_player_is_left_to_him() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_MCL, 600.0, 272.0)
        .on_the_ball(AWAY_MCL)
        .stand(MCR, 590.0, 272.0)
        .stand(AWAY_FL, 420.0, 200.0)
        .stand(DCL, 410.0, 200.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.mark_of(DCL), Some(AWAY_FL));
    pitch.stand(AWAY_FR, 440.0, 240.0).stand(MCL, 430.0, 225.0);
    assert_eq!(pitch.context.defence_home.mark_of(MCL), None);
    let man = pitch.with_ctx(MCL, MidfielderRead::man).map(|m| m.id);
    assert_eq!(man, Some(AWAY_FR), "he took a man the plan gave to another");
}

#[test]
fn tight_near_goal_off_in_midfield() {
    let pitch = Pitch::new();
    let own_goal = pitch.context.goal_positions.left;
    let tight = pitch.with_ctx(MCL, |ctx| {
        DefenderSkillProfile::from_ctx(ctx).ideal_marking_distance
    });
    let at = |metres: f32| own_goal + Vector3::new(metres * 8.0, 0.0, 0.0);
    let near = DefenderMarkingState::stand_off(own_goal, at(20.0), tight);
    let far = DefenderMarkingState::stand_off(own_goal, at(50.0), tight);
    assert!(near <= 14.0, "{} m off a man 20 m out", near / 8.0);
    assert!(
        far <= 20.0 && far >= near,
        "{} m off a man 50 m out",
        far / 8.0
    );
}

// ── The duel ────────────────────────────────────────────────────────────

/// The carrier `gap` units in front of a goal-side midfielder who is the
/// plan's presser, in midfield.
fn duel(gap: f32) -> Pitch {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 340.0 + gap, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(MCL, 340.0, 272.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(MCL));
    pitch
}

#[test]
fn a_closing_midfielder_on_cooldown_contains() {
    let mut pitch = duel(TackleEngagement::COMMIT - 2.0);
    pitch.field.get_player_mut(MCL).unwrap().tackle_cooldown = 100;
    assert_eq!(
        pitch.answer(MCL, MidfielderState::Pressing),
        MidfielderState::Jockeying
    );
}

#[test]
fn a_midfielder_who_declines_the_challenge_keeps_containing() {
    let mut pitch = duel(TackleEngagement::CONTACT - 2.0);
    pitch.field.get_player_mut(MCL).unwrap().tackle_cooldown = 100;
    for _ in 0..50 {
        let change = pitch.process(MCL, MidfielderState::Jockeying);
        assert!(
            change.and_then(|c| c.state).is_none(),
            "he left his containment"
        );
    }
    let carrier = pitch.field.get_player(AWAY_FL).unwrap().position;
    let me = pitch.field.get_player(MCL).unwrap().position;
    assert!(me.x < carrier.x, "goal-side");
    // Standing on the carrier's goal side, he holds it rather than running
    // into the man and the ball.
    let v = pitch.velocity(MCL, MidfielderState::Jockeying);
    let to_ball = (pitch.field.ball.position - me).normalize();
    assert!(v.dot(&to_ball) <= 0.05, "he runs at the ball: {v:?}");
}

#[test]
fn a_carrier_beyond_disengage_is_closed_down_again() {
    let mut pitch = duel(TackleEngagement::DISENGAGE + 8.0);
    assert_eq!(
        pitch.answer(MCL, MidfielderState::Jockeying),
        MidfielderState::Pressing
    );
}

#[test]
fn a_carrier_who_releases_the_ball_hands_him_to_the_read() {
    let mut pitch = duel(TackleEngagement::CONTACT);
    pitch.field.ball.current_owner = None;
    pitch.field.ball.velocity = Vector3::new(2.0, 0.0, 0.0);
    pitch.put_in(MCL, MidfielderState::Jockeying);
    let read = pitch.with_ctx(MCL, MidfielderRead::resolve);
    assert_eq!(pitch.answer(MCL, MidfielderState::Jockeying), read);
    assert_ne!(read, MidfielderState::Jockeying);
}

#[test]
fn the_licence_passing_to_another_man_sends_him_to_the_read() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 300.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(DCL, 294.0, 272.0)
        .plan(AWAY)
        .stand(MCL, 380.0, 272.0);
    assert_eq!(pitch.context.defence_home.presser(), Some(DCL));
    pitch.put_in(MCL, MidfielderState::Pressing);
    let read = pitch.with_ctx(MCL, MidfielderRead::resolve);
    assert_ne!(read, MidfielderState::Pressing);
    assert_eq!(pitch.answer(MCL, MidfielderState::Pressing), read);
}

/// A block tackle staged on a midfielder in contact with the carrier, rolled
/// `rolls` times: every roll leaves `Tackling` in the tick it was made, as
/// one of the three outcomes.
fn challenges(rolls: usize, arriving: Vector3<f32>, set_ticks: u16) -> [usize; 3] {
    let mut pitch = duel(TackleEngagement::CONTACT - 2.0);
    let challenger = pitch.field.get_player_mut(MCL).unwrap();
    challenger.velocity = arriving;
    challenger.contact_ticks = set_ticks;
    let mut seen = [0usize; 3];
    for _ in 0..rolls {
        pitch.put_in(MCL, MidfielderState::Tackling);
        pitch.field.get_player_mut(MCL).unwrap().staged_challenge = Some(ChallengeKind::Block);
        let mut change = pitch
            .with_ctx(MCL, |ctx| MidfielderTacklingState::default().process(ctx))
            .expect("a challenge is decided on entry");
        assert!(change.start_tackle_cooldown);
        let events: Vec<Event> = change.events.drain().collect();
        let won = events
            .iter()
            .any(|e| matches!(e, Event::PlayerEvent(PlayerEvent::TacklingBall(MCL))));
        let fouled = events.iter().any(|e| {
            matches!(e, Event::PlayerEvent(PlayerEvent::CommitFoul(foul))
                if foul.fouled == AWAY_FL && foul.source == FoulSource::Tackle)
        });
        let missed = change.missed_challenge == Some(ChallengeKind::Block)
            && change.state.is_none()
            && events.is_empty();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::PlayerEvent(PlayerEvent::ClaimBall(_)))),
            "the challenge claimed a ball of its own"
        );
        assert_eq!(
            [won, fouled, missed].iter().filter(|o| **o).count(),
            1,
            "a challenge has exactly one outcome"
        );
        seen[[won, fouled, missed].iter().position(|o| *o).unwrap()] += 1;
    }
    seen
}

#[test]
fn a_challenge_is_won_fouled_or_missed_in_the_tick_it_is_made() {
    let [won, fouled, missed] = challenges(3000, Vector3::zeros(), 0);
    assert!(won > 0, "never won");
    assert!(fouled > 0, "never a foul");
    assert!(missed > 0, "never missed");
}

#[test]
fn diving_in_wins_less_and_fouls_more_than_being_set() {
    let [rushed_won, rushed_fouls, _] = challenges(2_000, Vector3::new(0.6, 0.0, 0.0), 0);
    let [set_won, set_fouls, _] = challenges(2_000, Vector3::zeros(), 25);
    assert!(
        set_won > rushed_won,
        "set won {set_won}, at a sprint {rushed_won}"
    );
    assert!(
        set_fouls < rushed_fouls,
        "set fouled {set_fouls}, at a sprint {rushed_fouls}"
    );
}

#[test]
fn pace_can_beat_a_midfielder_backing_off() {
    let mut pitch = duel(TackleEngagement::COMMIT - 2.0);
    let conditions = pitch.context.conditions.clone();
    let midfielder = pitch.field.get_player_mut(MCL).unwrap();
    MidfielderJockeyingState::default().process_conditions(ConditionContext {
        in_state_time: 10,
        player: midfielder,
        conditions: &conditions,
        match_progress: 0.3,
        side_on_ball: false,
        ball_dead: false,
    });
    assert!(matches!(
        midfielder.last_activity_intensity,
        ActivityIntensity::High
    ));
    let carrier = pitch.field.get_player(AWAY_FL).unwrap();
    assert!(
        MovementEffort::speed_fraction(ActivityIntensity::High, 100)
            < MovementEffort::carrier_ceiling(carrier, 30, 100),
        "a man containing keeps up with a carrier at full stride"
    );
}

#[test]
fn a_loose_ball_beside_the_duel_is_the_elections() {
    let mut pitch = duel(TackleEngagement::CONTACT - 2.0);
    // The carrier loses it; a team-mate is nearer the ball than he is.
    pitch.field.ball.current_owner = None;
    pitch.field.ball.position = Vector3::new(352.0, 280.0, 0.0);
    pitch.stand(MCR, 354.0, 282.0);
    pitch.put_in(MCL, MidfielderState::Tackling);
    pitch.field.get_player_mut(MCL).unwrap().staged_challenge = Some(ChallengeKind::Block);
    let mut change = pitch
        .with_ctx(MCL, |ctx| MidfielderTacklingState::default().process(ctx))
        .expect("he leaves");
    let events: Vec<Event> = change.events.drain().collect();
    assert!(events.is_empty(), "{} events", events.len());
    assert_ne!(
        change.state,
        Some(PlayerState::Midfielder(MidfielderState::TakeBall)),
        "he took the chase from the man the election gives it to"
    );
}

// ── The man on the ball ─────────────────────────────────────────────────

#[test]
fn a_midfielder_without_the_ball_is_never_left_in_running() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 300.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(MCL, 360.0, 300.0)
        .plan(AWAY);
    assert_ne!(
        pitch.answer(MCL, MidfielderState::Running),
        MidfielderState::Running
    );
    pitch.stand(MCR, 360.0, 200.0).on_the_ball(MCR);
    assert_ne!(
        pitch.answer(MCL, MidfielderState::Running),
        MidfielderState::Running
    );
}

// ── The attack's jobs ───────────────────────────────────────────────────

/// Every home midfielder off the ball goes on a run only when the plan gives
/// him one, and does the plan's job when it gives one.
fn runs_only_when_given(pitch: &mut Pitch) {
    let plan = pitch.context.attack_home;
    for id in HOME_MIDFIELD {
        if pitch.field.ball.current_owner == Some(id) {
            continue;
        }
        let given_run = plan.slot_of(id).is_some() || plan.far_runner == Some(id);
        for state in [MidfielderState::Walking, MidfielderState::HoldingShape] {
            pitch.put_in(id, state);
            let read = pitch.with_ctx(id, MidfielderRead::resolve);
            assert_eq!(
                read == MidfielderState::AttackSupporting,
                given_run,
                "{id} from {state:?} answered {read:?}"
            );
            if plan.near_support == Some(id) && plan.wide.flank_of(id).is_none() {
                assert_eq!(read, MidfielderState::CreatingSpace, "the outlet {id}");
            }
        }
    }
}

/// Home attacking the away box: the ball with the left forward at its edge.
fn final_third() -> Pitch {
    let mut pitch = Pitch::new();
    pitch
        .blocks(480.0, 760.0)
        .stand(109, 700.0, 230.0)
        .on_the_ball(109)
        .refresh();
    assert!(pitch.context.attack_home.active, "the attack is on");
    pitch
}

#[test]
fn an_outlet_is_named_in_the_build_up() {
    let mut pitch = Pitch::new();
    pitch.blocks(150.0, 560.0).on_the_ball(DCL).refresh();
    let plan = pitch.context.attack_home;
    assert!(!plan.active, "a build-up commits nobody forward");
    let outlet = plan.near_support.expect("an outlet");
    assert_ne!(outlet, DCL);
    assert!(
        plan.wide.flank_of(outlet).is_none(),
        "{outlet} holds a touchline"
    );
}

#[test]
fn the_man_kept_behind_the_ball_stays() {
    // The ball on the right, the left side pushed on, and the left-centre
    // midfielder a few strides behind his own place: he is one of the men
    // the attack keeps back.
    let mut pitch = Pitch::new();
    pitch
        .blocks(480.0, 760.0)
        .stand(101, 700.0, 90.0)
        .stand(ML, 660.0, 90.0)
        .stand(104, 640.0, 455.0)
        .stand(110, 700.0, 340.0)
        .on_the_ball(110)
        .refresh();
    let place = pitch.with_ctx(MCL, |ctx| {
        ctx.team()
            .shape()
            .anchor_of(MCL)
            .unwrap_or(ctx.player.start_position)
    });
    pitch.stand(MCL, place.x - 100.0, place.y).refresh();
    let plan = pitch.context.attack_home;
    assert!(plan.active, "the attack is on");
    assert!(plan.is_rest_defence(MCL), "kept behind the ball");
    let ball_x = pitch.field.ball.position.x;
    assert!(
        pitch.anchor(MCL).x <= ball_x - 40.0 + 1e-3,
        "his place is {} with the ball at {ball_x}",
        pitch.anchor(MCL).x
    );
    pitch.in_state_time = 10_000;
    for state in [MidfielderState::Walking, MidfielderState::HoldingShape] {
        pitch.put_in(MCL, state);
        let read = pitch.with_ctx(MCL, MidfielderRead::resolve);
        assert!(
            matches!(
                read,
                MidfielderState::Walking | MidfielderState::HoldingShape
            ),
            "kept behind the ball, he answered {read:?}"
        );
    }
    assert_ne!(
        pitch.answer(MCL, MidfielderState::AttackSupporting),
        MidfielderState::AttackSupporting,
        "kept on a run he was not given"
    );
}

#[test]
fn the_runner_runs() {
    // Their back four 18.75 m from their goal line: room behind it for a run
    // (with the line on the edge of their own area nobody is named for one).
    let mut pitch = Pitch::new();
    pitch
        .blocks(480.0, 690.0)
        .stand(109, 700.0, 230.0)
        .on_the_ball(109)
        .refresh();
    assert!(pitch.context.attack_home.active, "the attack is on");
    let plan = pitch.context.attack_home;
    assert!(
        HOME_MIDFIELD
            .iter()
            .any(|id| plan.slot_of(*id).is_some() || plan.far_runner == Some(*id)),
        "a midfielder given a run"
    );
    runs_only_when_given(&mut pitch);
}

#[test]
fn nobody_goes_on_a_clock() {
    let mut pitch = final_third();
    pitch.in_state_time = 10_000;
    runs_only_when_given(&mut pitch);
}

#[test]
fn winning_the_ball_does_not_send_everybody() {
    let mut pitch = Pitch::new();
    pitch
        .blocks(250.0, 600.0)
        .stand(MCR, 360.0, 335.0)
        .on_the_ball(MCR)
        .refresh_won();
    assert!(
        pitch.context.attack_home.active,
        "the transition commits runners"
    );
    runs_only_when_given(&mut pitch);
}

#[test]
fn a_run_is_held_at_the_offside_line() {
    let mut pitch = Pitch::new();
    pitch.blocks(300.0, 600.0).on_the_ball(DCL).refresh();
    let plan = pitch.context.attack_home;
    let runners: Vec<u32> = HOME_MIDFIELD
        .into_iter()
        .filter(|id| plan.slot_of(*id).is_some() || plan.far_runner == Some(*id))
        .collect();
    assert!(!runners.is_empty(), "a midfielder given a run");
    let line = OffsideLine::second_last(
        pitch
            .field
            .players
            .iter()
            .filter(|p| p.team_id == AWAY)
            .map(|p| p.position.x),
        PlayerSide::Left,
    )
    .expect("a line");
    for id in runners {
        let y = pitch.field.get_player(id).unwrap().position.y;
        pitch.stand(id, line - 1.0, y);
        let v = pitch.velocity(id, MidfielderState::AttackSupporting);
        assert!(
            v.x <= v.norm() * 0.1 + 1e-4,
            "{id} on the line runs beyond it: {v:?}"
        );
    }
}

#[test]
fn onside_is_level_with_the_line_or_the_ball() {
    assert_eq!(
        OffsideLine::onside_x(PlayerSide::Left, 700.0, 400.0, 600.0),
        600.0
    );
    assert_eq!(
        OffsideLine::onside_x(PlayerSide::Left, 700.0, 650.0, 600.0),
        650.0
    );
    assert_eq!(
        OffsideLine::onside_x(PlayerSide::Left, 500.0, 400.0, 600.0),
        500.0
    );
    assert_eq!(
        OffsideLine::onside_x(PlayerSide::Right, 100.0, 400.0, 250.0),
        250.0
    );
    assert_eq!(
        OffsideLine::onside_x(PlayerSide::Right, 100.0, 200.0, 250.0),
        200.0
    );
}

#[test]
fn a_midfielder_with_no_job_is_never_in_attack_supporting() {
    let mut pitch = final_third();
    let plan = pitch.context.attack_home;
    for id in HOME_MIDFIELD {
        if plan.slot_of(id).is_some() || plan.far_runner == Some(id) {
            continue;
        }
        assert_ne!(
            pitch.answer(id, MidfielderState::AttackSupporting),
            MidfielderState::AttackSupporting,
            "{id}"
        );
    }
}

/// The right-centre midfielder on the ball in his own half, his partner
/// 13 m beside him and nearer than anybody else.
fn outlet_scene() -> Pitch {
    let mut pitch = Pitch::new();
    pitch
        .blocks(120.0, 640.0)
        .stand(109, 420.0, 150.0)
        .stand(110, 420.0, 400.0)
        .stand(MCR, 240.0, 335.0)
        .stand(MCL, 240.0, 230.0)
        .on_the_ball(MCR)
        .refresh();
    pitch
}

#[test]
fn a_midfielder_on_the_ball_still_has_an_outlet() {
    let pitch = outlet_scene();
    let outlet = pitch.context.attack_home.near_support.expect("an outlet");
    assert_ne!(outlet, MCR);
    assert_eq!(outlet, MCL);
}

#[test]
fn the_outlet_shows_off_the_carrier_on_his_own_bearing() {
    let mut pitch = outlet_scene();
    assert_eq!(
        pitch.with_ctx(MCL, MidfielderRead::resolve),
        MidfielderState::CreatingSpace
    );
    let carrier = pitch.field.get_player(MCR).unwrap().position;
    let me = pitch.field.get_player(MCL).unwrap().position;
    let offer = pitch
        .with_ctx(MCL, |ctx| SupportOffer::target(ctx, carrier))
        .expect("an offer");
    let gap = (offer - carrier).xy().norm();
    assert!(
        (76.0..=160.0).contains(&gap),
        "{} m off the carrier",
        gap / 8.0
    );
    let own = (me - carrier).xy().normalize();
    assert!(
        (offer - carrier).xy().normalize().dot(&own) > 0.5,
        "off his own bearing"
    );
    let v = pitch.velocity(MCL, MidfielderState::CreatingSpace);
    assert!(
        v.xy().normalize().dot(&(offer - me).xy().normalize()) > 0.99,
        "he steers somewhere else: {v:?}"
    );
}

#[test]
fn a_carrier_who_gets_goal_side_is_chased() {
    // The carrier a stride past him, on the way to his goal.
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 150.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(MCL, 162.0, 268.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(MCL));
    let chaser = pitch.field.get_player_mut(MCL).unwrap();
    chaser.stretch_ticks = 10;
    chaser.skills.technical.tackling = 15.0;
    chaser.skills.mental.aggression = 14.0;
    chaser.skills.mental.decisions = 12.0;
    let mut stretches = 0;
    for _ in 0..3000 {
        let Some(change) = pitch.process(MCL, MidfielderState::Pressing) else {
            continue;
        };
        assert_eq!(
            change.state,
            Some(PlayerState::Midfielder(MidfielderState::Tackling)),
            "a man gone past stretches for it; he cannot contain"
        );
        assert_eq!(change.staged_challenge, Some(ChallengeKind::Slide));
        stretches += 1;
    }
    assert!(stretches > 0, "he never stretched for it");
}

// ── The high ball ───────────────────────────────────────────────────────

/// A ball at head height beside him that nobody holds.
fn ball_at_his_head(pitch: &mut Pitch, id: u32) {
    let at = pitch.field.get_player(id).unwrap().position;
    let ball = &mut pitch.field.ball;
    ball.current_owner = None;
    ball.position = Vector3::new(at.x + 4.0, at.y, 2.0);
    ball.velocity = Vector3::new(-0.2, 0.0, -0.05);
    ball.pass_target_player_id = None;
}

#[test]
fn an_award_sends_him_up_from_any_state() {
    let mut pitch = Pitch::new();
    pitch.stand(MCL, 300.0, 272.0);
    ball_at_his_head(&mut pitch, MCL);
    pitch.field.ball.aerial_contest_winner = Some(MCL);
    pitch.field.get_player_mut(MCL).unwrap().on_aerial_award();
    assert_eq!(pitch.one_answer(MCL), MidfielderState::Heading);
}

#[test]
fn a_ball_no_contest_gave_him_is_not_headed() {
    let mut pitch = Pitch::new();
    pitch.stand(MCL, 300.0, 272.0);
    ball_at_his_head(&mut pitch, MCL);
    assert_ne!(pitch.one_answer(MCL), MidfielderState::Heading);
}

#[test]
fn a_ball_awarded_to_another_man_is_left() {
    let mut pitch = Pitch::new();
    pitch.stand(MCL, 300.0, 272.0).stand(MCR, 312.0, 272.0);
    ball_at_his_head(&mut pitch, MCL);
    pitch.field.ball.aerial_contest_winner = Some(MCR);
    pitch.field.get_player_mut(MCR).unwrap().on_aerial_award();
    assert_ne!(pitch.one_answer(MCL), MidfielderState::Heading);
    assert_ne!(
        pitch.answer(MCL, MidfielderState::Heading),
        MidfielderState::Heading,
        "up for a ball the contest gave somebody else"
    );
}

#[test]
fn the_winner_goes_up() {
    let mut pitch = Pitch::new();
    pitch.stand(MCL, 400.0, 272.0);
    ball_at_his_head(&mut pitch, MCL);
    pitch.field.ball.aerial_contest_winner = Some(MCL);
    pitch.field.get_player_mut(MCL).unwrap().on_aerial_award();
    let mut change = pitch
        .process(MCL, MidfielderState::Heading)
        .expect("he takes off");
    assert_eq!(
        change.state,
        Some(PlayerState::Midfielder(MidfielderState::Heading)),
        "still up for it"
    );
    let events: Vec<Event> = change.events.drain().collect();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::PlayerEvent(PlayerEvent::Leap(MCL, _)))),
        "he heads it off the ground"
    );
}

#[test]
fn no_second_roll() {
    // Awarded, up for it and in reach: he heads it every time he meets it.
    let mut pitch = Pitch::new();
    pitch.stand(MCL, 400.0, 272.0);
    ball_at_his_head(&mut pitch, MCL);
    pitch.field.ball.aerial_contest_winner = Some(MCL);
    let man = pitch.field.get_player_mut(MCL).unwrap();
    man.on_aerial_award();
    man.height = 0.3;
    for _ in 0..200 {
        let mut change = pitch
            .process(MCL, MidfielderState::Heading)
            .expect("he heads it");
        let events: Vec<Event> = change.events.drain().collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::PlayerEvent(PlayerEvent::KnockDown(..)))),
            "a header he won, not played: {}",
            events.len()
        );
    }
}

/// **A midfielder plays the cross he won.** The contest has given him the
/// header; with the ball above the boot and within a head's reach he heads
/// it — at goal or away — rather than letting the award lapse.
#[test]
fn a_midfielder_who_wins_a_cross_heads_it() {
    let mut pitch = Pitch::new();
    pitch.stand(MCL, 755.0, 272.0);
    let man = pitch.field.get_player_mut(MCL).unwrap();
    man.on_aerial_award();
    man.height = 0.3;
    pitch.field.ball.current_owner = None;
    pitch.field.ball.position = Vector3::new(755.4, 272.0, 2.0);
    pitch.field.ball.velocity = Vector3::new(0.1, 0.0, -0.02);
    pitch.field.ball.aerial_contest_winner = Some(MCL);
    let change = pitch
        .with_ctx(MCL, |ctx| MidfielderHeadingState::default().process(ctx))
        .expect("he plays the header");
    let mut events = change.events;
    let events: Vec<Event> = events.drain().collect();
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::PlayerEvent(
                PlayerEvent::Shoot(_) | PlayerEvent::KnockDown(..) | PlayerEvent::ClearBall(..)
            )
        )),
        "a won header is struck"
    );
}

/// …but not from a stride and a half away: a head does not reach as far as
/// a boot.
#[test]
fn a_midfielder_does_not_head_a_ball_a_stride_and_a_half_away() {
    let mut pitch = Pitch::new();
    pitch.stand(MCL, 755.0, 272.0);
    let man = pitch.field.get_player_mut(MCL).unwrap();
    man.on_aerial_award();
    man.height = 0.3;
    pitch.field.ball.current_owner = None;
    pitch.field.ball.position = Vector3::new(755.0 + 12.0, 272.0, 2.0);
    pitch.field.ball.velocity = Vector3::new(0.1, 0.0, -0.02);
    pitch.field.ball.aerial_contest_winner = Some(MCL);
    let change = pitch.with_ctx(MCL, |ctx| MidfielderHeadingState::default().process(ctx));
    assert!(
        change.is_none_or(|mut c| c.events.drain().next().is_none()),
        "1.5 m is outside a head's reach"
    );
}

/// **A wide midfielder who has tracked back is pinned as a full-back is.**
/// On the ball against his own touchline in his own third, pressed and with
/// the line ahead closed, he puts it out up the line.
#[test]
fn a_wide_midfielder_pinned_in_his_own_half_puts_it_out() {
    let mut pitch = Pitch::new();
    pitch.stand(ML, 150.0, 10.0).on_the_ball(ML);
    pitch.stand(AWAY_FL, 148.0, 12.0);
    pitch.stand(AWAY_FR, 190.0, 12.0);
    let change = pitch
        .with_ctx(ML, |ctx| MidfielderRunningState::default().process(ctx))
        .expect("he plays it");
    let mut events = change.events;
    assert!(
        events
            .drain()
            .any(|e| matches!(e, Event::PlayerEvent(PlayerEvent::PutOut(id, _)) if id == ML)),
        "he puts it out"
    );
}
