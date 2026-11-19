//! **The errors a midfielder's play was getting wrong.** The clocks his
//! take-on and his time on the ball are counted on, the units his marking and
//! his carrying move in, the reach a header is played at and what it is
//! booked as, the touchline a marker gave his man up at, the shirt pull after
//! his side has won the ball, and who may challenge a carrier who runs into
//! him.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::common_states::TackleEngagement;
use crate::r#match::engine::ball::ball::PlayerReach;
use crate::r#match::engine::result::Score;
use crate::r#match::events::{Event, EventCollection, EventDispatcher};
use crate::r#match::midfielders::states::common::MidfielderRead;
use crate::r#match::midfielders::states::running::ReleaseClock;
use crate::r#match::midfielders::states::{
    MidfielderDribblingState, MidfielderGuardingState, MidfielderHeadingState, MidfielderState,
};
use crate::r#match::player::events::{FoulSource, PlayerEvent};
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    DefenceRefreshInputs, DefensivePlan, GameTickContext, MatchContext, MatchField,
    MatchPlayerCollection, MatchRng, PlayerSide, ResultMatchPositionData, StateChangeResult,
    StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const HOME: u32 = 1;
const AWAY: u32 = 2;
const DCL: u32 = 102;
const ML: u32 = 105;
const MCL: u32 = 106;
const MCR: u32 = 107;
const AWAY_MR: u32 = 208;
const AWAY_FL: u32 = 209;

/// Home on the left, defending x = 0, ten minutes in, everyone parked out of
/// the way until a test stands him somewhere.
struct Pitch {
    field: MatchField,
    context: MatchContext,
}

impl Pitch {
    fn new() -> Self {
        let home = squad(HOME, 100);
        let away = squad(AWAY, 200);
        let players = MatchPlayerCollection::from_squads(&home, &away);
        let mut field = MatchField::new(840, 545, home, away);
        assert_eq!(field.side_of(HOME), PlayerSide::Left);
        field.ball.kickoff_taker = None;
        for (index, p) in field.players.iter_mut().enumerate() {
            p.velocity = Vector3::zeros();
            let x = if p.team_id == HOME { 700.0 } else { 790.0 };
            p.position = Vector3::new(x, 20.0 + (index % 11) as f32 * 48.0, 0.0);
        }
        let mut context = MatchContext::new(&field, players, Score::new(HOME, AWAY), false, false);
        context.total_match_time = 600_000;
        Pitch { field, context }
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
            in_state_time: 40,
            player: self.field.get_player(id).unwrap(),
            context: &self.context,
            tick_context: &tick,
        };
        read(&ctx)
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

// ── Clocks ───────────────────────────────────────────────────────────────

#[test]
fn a_take_on_lasts_as_long_as_it_says() {
    // 0.9 + 0.375 × 1.6 = 1.5 s: 75 AI ticks of 20 ms, not 150.
    assert_eq!(MidfielderDribblingState::allowance(0.375), 75);
}

#[test]
fn a_pressed_midfielder_gets_the_time_his_composure_buys() {
    // However long he ran without the ball, the clock starts at the
    // reception: half composure buys 0.8 s, 80 engine ticks.
    assert!(!ReleaseClock::forced(true, 10, 0.5, false));
    assert!(!ReleaseClock::forced(true, 80, 0.5, false));
    assert!(ReleaseClock::forced(true, 81, 0.5, false));
    assert!(!ReleaseClock::forced(false, 81, 0.5, false));
    assert!(ReleaseClock::forced(false, 901, 0.5, false));
}

// ── Units ────────────────────────────────────────────────────────────────

#[test]
fn a_marker_never_moves_faster_than_he_can() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 300.0, 400.0)
        .on_the_ball(AWAY_FL)
        .stand(MCL, 320.0, 150.0)
        .stand(AWAY_MR, 260.0, 120.0);
    pitch.put_in(MCL, MidfielderState::Guarding);
    let top = pitch.top_speed(MCL);
    let v = pitch
        .with_ctx(MCL, |ctx| MidfielderGuardingState::default().velocity(ctx))
        .expect("a velocity");
    assert!(
        v.norm() <= top + 1e-4,
        "{} against a top speed of {top}",
        v.norm()
    );
}

#[test]
fn a_dribbler_never_moves_faster_than_he_can() {
    let mut pitch = Pitch::new();
    pitch.stand(MCL, 400.0, 272.0).on_the_ball(MCL);
    pitch.put_in(MCL, MidfielderState::Dribbling);
    let top = pitch.top_speed(MCL);
    let v = pitch
        .with_ctx(MCL, |ctx| MidfielderDribblingState::default().velocity(ctx))
        .expect("a velocity");
    assert!(
        v.norm() <= top + 1e-4,
        "{} against a top speed of {top}",
        v.norm()
    );
}

// ── Headers ──────────────────────────────────────────────────────────────

#[test]
fn a_header_is_measured_across_the_grass() {
    let pitch = Pitch::new();
    let man = pitch.field.get_player(MCL).unwrap();
    let above = Vector3::new(man.position.x, man.position.y, 2.4);
    assert!(
        PlayerReach::can_head(above, man),
        "2.4 m up, straight above him"
    );
    let wide = Vector3::new(man.position.x + 12.0, man.position.y, 2.0);
    assert!(
        !PlayerReach::can_head(wide, man),
        "1.5 m away across the grass"
    );
    let low = Vector3::new(man.position.x, man.position.y, 1.2);
    assert!(!PlayerReach::can_head(low, man), "a ball for his boot");
}

/// A ball the contest has given him, coming down on him at `x` as he meets
/// it in the air, with the ball loose and his side the last to play it.
fn header_at(x: f32) -> (Pitch, Vec<PlayerEvent>) {
    let mut pitch = Pitch::new();
    pitch.stand(MCL, x, 272.0);
    pitch.field.ball.current_owner = None;
    pitch.field.ball.position = Vector3::new(x + 4.0, 272.0, 2.0);
    pitch.field.ball.velocity = Vector3::new(-0.2, 0.0, -0.05);
    pitch.field.ball.aerial_contest_winner = Some(MCL);
    pitch.put_in(MCL, MidfielderState::Heading);
    let man = pitch.field.get_player_mut(MCL).unwrap();
    man.on_aerial_award();
    man.height = 0.3;
    let change = pitch
        .with_ctx(MCL, |ctx| MidfielderHeadingState::default().process(ctx))
        .expect("he heads it");
    (pitch, events_of(change))
}

#[test]
fn a_clearing_header_is_not_a_shot() {
    let (mut pitch, events) = header_at(60.0);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, PlayerEvent::ClearBall(..))),
        "{events:?}"
    );
    assert!(!events.iter().any(|e| matches!(e, PlayerEvent::Shoot(..))));
    pitch.dispatch(&events);
    let man = pitch.field.get_player(MCL).unwrap();
    assert_eq!(man.statistics.clearances, 1);
    assert_eq!(man.memory.shots_taken, 0);
}

#[test]
fn a_knock_down_is_not_a_shot() {
    // The middle third, in his own half: a ball struck away from there
    // counts as a clearance, a knock-down does not.
    let (mut pitch, events) = header_at(330.0);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, PlayerEvent::KnockDown(..))),
        "{events:?}"
    );
    pitch.dispatch(&events);
    let man = pitch.field.get_player(MCL).unwrap();
    assert_eq!(man.statistics.clearances, 0);
    assert_eq!(man.memory.shots_taken, 0);
}

// ── Marking ──────────────────────────────────────────────────────────────

#[test]
fn a_winger_on_the_touchline_is_still_marked() {
    let mut pitch = Pitch::new();
    let start = pitch.field.get_player(ML).unwrap().start_position;
    pitch
        .stand(AWAY_FL, 300.0, 300.0)
        .on_the_ball(AWAY_FL)
        .stand(ML, start.x, 10.0)
        .stand(AWAY_MR, start.x - 20.0, 8.0);
    pitch.put_in(ML, MidfielderState::Guarding);
    let refused = pitch.with_ctx(ML, |ctx| {
        let man = MidfielderRead::man(ctx).expect("a man to mark");
        assert_eq!(man.id, AWAY_MR);
        MidfielderRead::refuses(ctx, &man)
    });
    assert!(!refused, "a man a metre inside the touchline is still his");
    let next = pitch
        .with_ctx(ML, |ctx| MidfielderGuardingState::default().process(ctx))
        .and_then(|change| change.state);
    assert_ne!(
        next,
        Some(PlayerState::Midfielder(MidfielderState::Returning)),
        "he gave his man up at the touchline"
    );
}

#[test]
fn no_shirt_pull_once_his_side_has_the_ball() {
    for seed in 0..300u64 {
        let mut pitch = Pitch::new();
        pitch.context.rng = MatchRng::from_seed(0x5417_0000 + seed);
        pitch
            .stand(MCR, 250.0, 300.0)
            .on_the_ball(MCR)
            .stand(MCL, 200.0, 200.0)
            .stand(AWAY_MR, 205.0, 200.0);
        pitch.put_in(MCL, MidfielderState::Guarding);
        let change = pitch.with_ctx(MCL, |ctx| MidfielderGuardingState::default().process(ctx));
        let held = change.map(events_of).unwrap_or_default().iter().any(
            |e| matches!(e, PlayerEvent::CommitFoul(foul) if foul.source == FoulSource::Holding),
        );
        assert!(
            !held,
            "seed {seed}: a shirt pulled with his own side on the ball"
        );
    }
}

// ── The carrier who runs into him ────────────────────────────────────────

/// The away forward on the ball at the halfway line, the home centre-back
/// made the plan's presser from beside him, then moved to `presser_gap`
/// units off the carrier with the midfielder at `mid_gap`.
fn run_into(presser_gap: f32, mid_gap: f32) -> bool {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 420.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(DCL, 412.0, 272.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(DCL));
    pitch
        .stand(DCL, 420.0 - presser_gap, 272.0)
        .stand(MCL, 420.0, 272.0 + mid_gap);
    pitch.with_ctx(MCL, TackleEngagement::may_engage_carrier)
}

#[test]
fn a_carrier_runs_into_a_midfielder() {
    assert!(
        run_into(80.0, 16.0),
        "2 m from the carrier, the presser 10 m off"
    );
}

#[test]
fn the_assigned_closer_within_disengage_keeps_the_duel() {
    assert!(!run_into(20.0, 12.0), "the presser 2.5 m off keeps him");
}

// ── The second ball ──────────────────────────────────────────────────────

/// An away clearance coming down at head height on the home left-centre
/// midfielder, with the away right midfielder under it too when `contested`.
fn clearance_dropping(contested: bool, seed: u64) -> Pitch {
    let mut pitch = Pitch::new();
    pitch.context.rng = MatchRng::from_seed(seed);
    pitch.stand(MCL, 400.0, 272.0);
    if contested {
        pitch.stand(AWAY_MR, 408.0, 272.0);
    }
    let ball = &mut pitch.field.ball;
    ball.current_owner = None;
    ball.pass_target_player_id = None;
    ball.position = Vector3::new(404.0, 272.0, 2.2);
    ball.velocity = Vector3::new(-0.5, 0.0, -0.05);
    ball.last_touch_team_id = Some(AWAY);
    ball.aerial_decided = false;
    ball.flags.in_flight_state = 0;
    pitch
}

#[test]
fn a_clearance_dropping_in_midfield_is_contested_once() {
    let mut won = [0; 2];
    for seed in 0..40 {
        let mut pitch = clearance_dropping(true, seed);
        let mut events = EventCollection::new();
        let ball = &mut pitch.field.ball;
        ball.try_aerial_duel(&pitch.context, &pitch.field.players, &mut events);
        assert!(ball.aerial_decided, "nothing decided it");
        let winner = ball
            .aerial_delivery
            .map(|d| d.winner_id)
            .expect("the winner is staged");
        assert_eq!(ball.pending_aerial_strike, Some(winner));
        won[usize::from(winner == AWAY_MR)] += 1;

        ball.aerial_delivery = None;
        ball.pending_aerial_strike = None;
        ball.try_aerial_duel(&pitch.context, &pitch.field.players, &mut events);
        assert!(ball.aerial_delivery.is_none(), "decided twice");
    }
    assert!(won[0] > 0 && won[1] > 0, "one side always wins: {won:?}");
}

#[test]
fn nobody_to_beat() {
    let mut pitch = clearance_dropping(false, 7);
    let mut events = EventCollection::new();
    let ball = &mut pitch.field.ball;
    ball.try_aerial_duel(&pitch.context, &pitch.field.players, &mut events);
    assert!(
        !ball.aerial_decided,
        "a ball only one side can reach was rolled for"
    );
    assert!(ball.aerial_delivery.is_none());
    assert_eq!(ball.pending_aerial_strike, None);
}
