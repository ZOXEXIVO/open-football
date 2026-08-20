//! **The ball going out of play, over real engine ticks.**
//!
//! Reported 2026-08-20: *"the ball stops on the line behind the goal, but
//! must go beyond the goal, and the goalkeeper must put it into play. The
//! same applies elsewhere. The ball must not stop on the edge of the
//! field, but go beyond it."*
//!
//! Every restart used to write the ball onto its own spot on the tick it
//! crossed the line — two units *inside* the pitch, velocity and spin
//! zeroed — so a shot that missed by a foot came to a dead stop a hand's
//! width the wrong side of the byline and the keeper walked to a ball that
//! had never left the field. It is 25 cm, which is why it survived every
//! other relocation the restarts have had removed
//! ([[goal-kick-restart-teleport]], [[restarts-throw-in-and-offside]]),
//! and it is the one you watch on every goal kick of every match.
//!
//! What has to hold now is a SEQUENCE, and no single-tick assertion can
//! see it:
//!
//! 1. the ball crosses the line and keeps the pace it crossed with;
//! 2. it runs out across the run-off and the hoardings stop it
//!    ([`RunOff`]);
//! 3. the taker goes out there — off the pitch, which no player could do
//!    before — and picks it up;
//! 4. he carries it back to the point the restart is legally taken from
//!    and play resumes ON the pitch.
//!
//! Break any one and the others still look right: a ball that never
//! settles ends in the backstop teleport, a taker pinned on the byline
//! ends in the same place, and a restart taken from outside the pitch
//! immediately awards itself to the other side.
//!
//! [`RunOff`]: crate::r#match::engine::ball::ball::RunOff

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::RunOff;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::result::Score;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, PassOriginRestart,
    PlayerSide, ResultMatchPositionData,
};
use nalgebra::Vector3;

const WIDTH: usize = 840;
const HEIGHT: usize = 545;

/// A match at a kickoff, plus the scratch buffers a real tick needs.
struct RunOutMatch {
    field: MatchField,
    context: MatchContext,
    tick_context: GameTickContext,
    recording: ResultMatchPositionData,
}

impl RunOutMatch {
    fn new() -> Self {
        // ⚠ **The shared fixture's players cannot accelerate.**
        // `squad()` builds on `PlayerSkills::default()`, which leaves
        // `physical.acceleration` at zero — and `SteeringBehavior::Seek`
        // limits its steering force to `acceleration / 20`, so a player at
        // rest stays at rest however far away the thing he is chasing is.
        // Every assertion in this file is about a man covering ground, so
        // without this they measure the fixture rather than the engine: the
        // keeper stands on his line for the whole patience window and the
        // restart resolves through the backstop teleport, which looks
        // identical from the end state.
        let mut home = squad(1, 100);
        let mut away = squad(2, 200);
        for squad in [&mut home, &mut away] {
            for player in squad.main_squad.iter_mut() {
                player.skills.physical.acceleration = 14.0;
                player.skills.physical.agility = 14.0;
                player.skills.physical.stamina = 14.0;
            }
        }
        let players = MatchPlayerCollection::from_squads(&home, &away);
        let field = MatchField::new(WIDTH, HEIGHT, home, away);
        let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
        context.total_match_time = 10 * 60 * 1000;
        let tick_context = GameTickContext::new(&field, &context.players);
        RunOutMatch {
            field,
            context,
            tick_context,
            recording: ResultMatchPositionData::empty(),
        }
    }

    fn tick(&mut self) {
        self.context.increment_time();
        FootballEngine::<WIDTH, HEIGHT>::game_tick(
            &mut self.field,
            &mut self.context,
            &mut self.recording,
            &mut self.tick_context,
        );
    }

    /// A shot from the away side that misses the left-hand goal wide of
    /// the post, struck hard enough to have somewhere to go afterwards.
    /// The last touch is the SHOOTER's, which is what makes the restart a
    /// goal kick rather than a corner.
    fn shoot_wide_of_the_left_goal(&mut self) {
        let shooter = self
            .field
            .players
            .iter()
            .find(|p| {
                p.side == Some(PlayerSide::Right)
                    && !p.tactical_position.current_position.is_goalkeeper()
            })
            .map(|p| (p.id, p.team_id))
            .expect("the away side has outfielders");
        // Beside the post, below the bar, still carrying shot pace.
        self.field.ball.position = Vector3::new(-1.0, 200.0, 0.25);
        self.field.ball.velocity = Vector3::new(-1.6, -0.2, 0.0);
        self.field.ball.current_owner = None;
        self.field.ball.previous_owner = Some(shooter.0);
        let tick = self.context.current_tick();
        self.field
            .ball
            .record_touch(shooter.0, shooter.1, tick, true);
        self.tick();
    }

    /// Run until the restart resolves, or give up. Returns the ticks taken.
    fn play_the_restart_out(&mut self, bound: usize) -> Option<usize> {
        for elapsed in 1..=bound {
            self.tick();
            if self.field.ball.awaiting_restart.is_none() {
                return Some(elapsed);
            }
        }
        None
    }
}

/// **The ball goes past the goal, and stops out there.**
///
/// This is the report itself. The award must move nothing; the physics
/// must carry the ball on behind the byline; and the hoardings — not the
/// old 10-unit pitch clamp — must be what eventually stops it.
#[test]
fn a_shot_that_misses_runs_on_behind_the_goal() {
    if !RunOff::armed() {
        // See `the_off_arm_still_places_the_ball_on_the_spot`.
        return;
    }
    let mut m = RunOutMatch::new();
    m.shoot_wide_of_the_left_goal();

    let awaited = m
        .field
        .ball
        .awaiting_restart
        .expect("a ball wide of the post is a goal kick");
    assert_eq!(awaited.origin, PassOriginRestart::GoalKick);
    assert!(
        !awaited.settled,
        "the ball crossed the line this tick — it cannot already be at rest"
    );

    // Run the ball out. Deliberately measured over ticks: a ball that is
    // WRITTEN behind the goal and one that TRAVELS there end up in the
    // same place, and only the sequence tells them apart.
    let mut deepest = m.field.ball.position.x;
    let mut settled = None;
    for elapsed in 1..=600 {
        let before = m.field.ball.position;
        m.tick();
        let step = (m.field.ball.position - before).magnitude();
        assert!(
            step < 4.0,
            "the ball jumped {step:.1}u in one tick — a run-out is travel, \
             not a relocation with a longer name"
        );
        deepest = deepest.min(m.field.ball.position.x);
        match m.field.ball.awaiting_restart {
            Some(restart) if restart.settled => {
                settled = Some((elapsed, m.field.ball.position));
                break;
            }
            Some(_) => {}
            None => panic!("the restart resolved before the ball stopped moving"),
        }
    }
    let (elapsed, rest) = settled.expect("the ball has to come to rest inside six seconds");

    assert!(
        rest.x < 0.0,
        "the ball came to rest ON or INSIDE the pitch, at x={:.1} — it went \
         out of play, it has to be out of play",
        rest.x
    );
    assert!(
        deepest >= -RunOff::END - 1.0e-3,
        "the ball went past the hoardings, to x={deepest:.1} against a \
         perimeter at {:.1}",
        -RunOff::END
    );
    assert!(
        elapsed < 200,
        "the run-out took {elapsed} ticks — that is dead time the restart \
         has to pay for"
    );
}

/// **A ball that goes out is never a goal.**
///
/// `GoalPosition::is_goal` is a half-space test with no depth bound —
/// `x <= 0`, inside the posts, under the bar — which was safe for exactly
/// as long as nothing could be behind the goal without having gone into
/// it. A ball running out behind the byline drifts through that band all
/// the time, so `check_goal` has to stand down for a ball that is already
/// out of play. Without the guard the engine awards a goal off a miss,
/// silently, seconds after the shot.
#[test]
fn a_ball_running_out_behind_the_goal_never_scores() {
    let mut m = RunOutMatch::new();
    // Aimed to cross wide of the post and then drift back across the face
    // of the goal behind the line — the exact path that satisfies
    // `is_goal` a few ticks after the ball is already dead.
    let shooter = m
        .field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| (p.id, p.team_id))
        .expect("the away side has outfielders");
    let goal_y = m.context.goal_positions.left.y;
    m.field.ball.position = Vector3::new(-1.0, goal_y - 40.0, 0.20);
    m.field.ball.velocity = Vector3::new(-1.2, 0.9, 0.0);
    m.field.ball.current_owner = None;
    m.field.ball.previous_owner = Some(shooter.0);
    let tick = m.context.current_tick();
    m.field.ball.record_touch(shooter.0, shooter.1, tick, true);
    let score_before = (
        m.context.score.home_team.get(),
        m.context.score.away_team.get(),
    );
    m.tick();

    for _ in 0..400 {
        m.tick();
        assert!(
            !m.field.ball.goal_scored,
            "a goal was given for a ball that had already gone out of play, \
             at ({:.1}, {:.1}, {:.2})",
            m.field.ball.position.x,
            m.field.ball.position.y,
            m.field.ball.position.z
        );
    }
    assert_eq!(
        (
            m.context.score.home_team.get(),
            m.context.score.away_team.get()
        ),
        score_before,
        "the scoreline moved on a ball that missed"
    );
}

/// **The keeper goes out and gets it, and the kick is taken on the pitch.**
///
/// The two halves fail in opposite directions and each hides the other.
/// If the player clamp still pinned him 12.5 cm past his own byline he
/// could never reach a ball four metres behind it, and every goal kick
/// would fall through to the backstop teleport — which puts the ball on
/// the spot and looks, from the end state, exactly like success. And if he
/// were allowed to take the kick from where he picked the ball up, play
/// would restart from outside the pitch and the first thing the kick did
/// was cross the byline again and be given as a corner.
#[test]
fn the_keeper_fetches_it_from_behind_the_goal_and_restarts_on_the_pitch() {
    if !RunOff::armed() {
        // See `the_off_arm_still_places_the_ball_on_the_spot`.
        return;
    }
    let mut m = RunOutMatch::new();
    m.shoot_wide_of_the_left_goal();
    let taker = m.field.ball.awaiting_restart.expect("armed").taker_id;
    let take_from = m
        .field
        .ball
        .awaiting_restart
        .expect("armed")
        .take_from
        .expect("a run-out restart is taken from the crossing point");

    // Did he ever actually leave the pitch? That is the half the old clamp
    // made impossible, and a restart that succeeds without it succeeded
    // through the backstop.
    let mut went_off_the_pitch = false;
    let mut resolved = None;
    for elapsed in 1..=3000 {
        m.tick();
        if let Some(keeper) = m.field.players.iter().find(|p| p.id == taker) {
            if keeper.position.x < 0.0 {
                went_off_the_pitch = true;
            }
        }
        if m.field.ball.awaiting_restart.is_none() {
            resolved = Some(elapsed);
            break;
        }
    }
    let elapsed = resolved.expect("the goal kick never got taken");

    assert_eq!(
        m.field.ball.pending_set_piece_teleport, None,
        "the keeper was placed on the ball after {elapsed} ticks — the \
         backstop fired, so nothing below is evidence of anything"
    );
    assert!(
        went_off_the_pitch,
        "the keeper never left the pitch, so he cannot have fetched a ball \
         that was behind the goal"
    );
    assert_eq!(
        m.field.ball.current_owner,
        Some(taker),
        "and it has to end up his"
    );
    assert!(
        m.field.ball.position.x >= 0.0,
        "the kick is being taken from x={:.1} — outside the pitch, which \
         puts the ball straight back out of play",
        m.field.ball.position.x
    );
    assert!(
        (m.field.ball.position - take_from).magnitude() < 40.0,
        "and it has to be taken from somewhere near the spot it was awarded \
         at, not from wherever he happened to stop"
    );
}

/// **Switched off, nothing above happens.**
///
/// `OF_RUN_OUT` is the A/B, and an A/B is only worth having if the off arm
/// is the old behaviour exactly. It cannot be exercised in the same
/// process as the tests above — the switch is a `OnceLock` read once per
/// run — so this pins the shape of the off arm instead: with the run-out
/// declined, the award carries no second leg and the ball is at rest on
/// its spot from the first tick.
#[test]
fn the_off_arm_still_places_the_ball_on_the_spot() {
    if RunOff::armed() {
        // Armed is the default and is what every other test in this file
        // measures. Run this arm with `OF_RUN_OUT=off`.
        return;
    }
    let mut m = RunOutMatch::new();
    m.shoot_wide_of_the_left_goal();
    let awaited = m.field.ball.awaiting_restart.expect("armed");
    assert!(awaited.settled, "there is nothing to run out");
    assert_eq!(
        awaited.take_from, None,
        "and nothing to carry back — the ball is already on the spot"
    );
    assert!(
        m.field.ball.position.x > 0.0,
        "which is on the pitch, got x={:.1}",
        m.field.ball.position.x
    );
    assert!(
        m.play_the_restart_out(3000).is_some(),
        "and the restart still resolves"
    );
}

/// **A restart must not leave its taker pinned to the spot afterwards.**
///
/// The carry leg writes the taker a `set_piece_station`, and until this
/// change the corner was the only restart that had one — so
/// `TickEngine::clear_expired_corner_stations`, whose first act is to
/// return when no corner shape is armed, was a sufficient owner for it.
/// Every throw-in and goal kick has a carry leg now, and a station written
/// by one of those was never cleared by anything: not that function, not
/// the half-time reset, not the goal reset.
///
/// It then lay dormant until the next CORNER, because `CornerHold::apply`
/// bails unless the restart origin is `Corner` — a guard exactly the wrong
/// way round for a stale station. Measured before the fix: the keeper
/// carried a goal kick in from `(6.0, 199.8)`, kept that station, and on
/// the next corner was walked 12 m off his line back onto it with his
/// goalkeeping AI overridden for the whole delivery.
#[test]
fn a_taken_restart_leaves_nobody_holding_a_station() {
    if !RunOff::armed() {
        // See `the_off_arm_still_places_the_ball_on_the_spot`.
        return;
    }
    let mut m = RunOutMatch::new();
    m.shoot_wide_of_the_left_goal();
    let taker = m.field.ball.awaiting_restart.expect("armed").taker_id;

    // Play the goal kick right out, so the carry leg definitely ran.
    let carried = {
        let mut seen = false;
        let mut done = false;
        for _ in 0..3000 {
            m.tick();
            seen |= m
                .field
                .ball
                .awaiting_restart
                .is_some_and(|restart| restart.carrying);
            if m.field.ball.awaiting_restart.is_none() {
                done = true;
                break;
            }
        }
        assert!(done, "the goal kick never got taken");
        seen
    };
    assert!(
        carried,
        "the taker never carried the ball, so the leak this pins was never opened"
    );

    // A few ticks for the tick engine to run its own housekeeping.
    for _ in 0..5 {
        m.tick();
    }
    let pinned: Vec<u32> = m
        .field
        .players
        .iter()
        .filter(|p| p.set_piece_station.is_some())
        .map(|p| p.id)
        .collect();
    assert!(
        pinned.is_empty(),
        "the restart is over and {pinned:?} are still standing on a set-piece \
         station (the taker is {taker}) — it will be read as a corner shape by \
         the next corner and override their own AI"
    );
}
