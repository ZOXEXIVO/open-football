//! The corner set-up, over real engine ticks.
//!
//! `CornerShape` has unit tests for the plan itself — who goes where,
//! which posts get filled first, what a nine-man side gets. What they
//! cannot test is the only thing that made the corner look wrong on the
//! pitch: whether anybody is actually standing there when the ball comes
//! in.
//!
//! Three separate pieces have to hold for that, and each fails silently
//! on its own. The plan has to be *staged* by the corner award; the
//! engine has to *drain* it while it holds `&mut field.players`; and the
//! shape has to *survive* the second and a half between the award and the
//! delivery, against twenty state machines that each think it is ordinary
//! open play. Break any one and the box is empty again — which is the bug
//! this file exists for: a player watched a corner come in with nobody but
//! the goalkeeper defending the goal.
//!
//! The fourth assertion is the counterweight. A shape that never releases
//! is worse than no shape, because the corner's restart origin only decays
//! when somebody *touches* the ball and the pin is what stops anyone
//! reaching it — so "everyone stands still forever" is a stable state the
//! naive implementation really does reach.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::AwaitedRestart;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::result::Score;
use crate::r#match::{
    CornerShape, GameTickContext, MatchContext, MatchField, MatchPlayer, MatchPlayerCollection,
    PassOriginRestart, PlayerSide, ResultMatchPositionData,
};
use nalgebra::Vector3;

const WIDTH: usize = 840;
const HEIGHT: usize = 545;
/// The goal the corner is being taken at.
const DEFENDED_GOAL_X: f32 = 0.0;

/// A match sitting at a kickoff, plus the two scratch buffers a tick needs.
struct CornerMatch {
    field: MatchField,
    context: MatchContext,
    tick_context: GameTickContext,
    recording: ResultMatchPositionData,
}

impl CornerMatch {
    fn new() -> Self {
        let home = squad(1, 100);
        let away = squad(2, 200);
        let players = MatchPlayerCollection::from_squads(&home, &away);
        let field = MatchField::new(WIDTH, HEIGHT, home, away);
        let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
        context.total_match_time = 10 * 60 * 1000;
        let tick_context = GameTickContext::new(&field, &context.players);
        CornerMatch {
            field,
            context,
            tick_context,
            recording: ResultMatchPositionData::empty(),
        }
    }

    /// Put everybody in the middle of the pitch, which is the shape that
    /// produced the empty box: a counter-attack that ends with a defender
    /// hooking the ball behind leaves nobody anywhere near either area.
    fn crowd_the_centre_circle(&mut self) {
        for (i, player) in self.field.players.iter_mut().enumerate() {
            let y = 200.0 + (i % 8) as f32 * 18.0;
            player.position = Vector3::new(400.0 + (i % 5) as f32 * 12.0, y, 0.0);
            player.velocity = Vector3::zeros();
        }
    }

    /// Roll the ball behind the left-hand goal off a left-side defender,
    /// which is a corner to the right-hand side.
    fn concede_a_corner(&mut self) {
        let toucher = self
            .field
            .players
            .iter()
            .find(|p| {
                p.side == Some(PlayerSide::Left)
                    && !p.tactical_position.current_position.is_goalkeeper()
            })
            .map(|p| (p.id, p.team_id))
            .expect("a side has outfielders");
        self.field.ball.position = Vector3::new(-1.0, 100.0, 0.0);
        self.field.ball.velocity = Vector3::new(-1.0, 0.0, 0.0);
        self.field.ball.current_owner = None;
        self.field.ball.previous_owner = Some(toucher.0);
        let tick = self.context.current_tick();
        self.field
            .ball
            .record_touch(toucher.0, toucher.1, tick, true);
        self.tick();
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

    fn tick_n(&mut self, n: usize) {
        for _ in 0..n {
            self.tick();
        }
    }

    /// Outfielders of `side` inside the defended penalty area.
    fn in_the_box(&self, side: PlayerSide) -> usize {
        self.field
            .players
            .iter()
            .filter(|p| {
                p.side == Some(side)
                    && !p.is_sent_off
                    && !p.tactical_position.current_position.is_goalkeeper()
                    && CornerShape::is_in_penalty_area(p.position, DEFENDED_GOAL_X, HEIGHT as f32)
            })
            .count()
    }

    fn pinned(&self) -> Vec<&MatchPlayer> {
        self.field
            .players
            .iter()
            .filter(|p| p.set_piece_station.is_some())
            .collect()
    }

    /// Run the set-up out: the taker fetches the ball from the byline and
    /// carries it to the arc. Returns the ticks it took.
    ///
    /// ⚠ **Every assertion about the shape has to be made after this, not
    /// after `concede_a_corner`.** The corner used to be complete on the
    /// tick it was awarded — ball on the flag, taker on the ball, twenty
    /// players written into the shape — so the two moments were the same
    /// and the tests could not tell them apart. They are now seconds apart
    /// by design, and that gap IS the set-up: read at the award, the box
    /// is still whatever open play left it, which is the state these tests
    /// exist to catch. See `AwaitedRestart::take_from`.
    fn walk_the_corner_in(&mut self) -> usize {
        const BOUND: usize = 3200;
        // Tick THEN check, so this returns on the tick the kick is taken
        // rather than one after it. A corner is delivered within a few
        // ticks of the ball being set down, and a spare tick here is
        // enough for the cross to have already left.
        for elapsed in 1..=BOUND {
            self.tick();
            if self.field.ball.awaiting_restart.is_none() {
                return elapsed;
            }
        }
        panic!("the corner never got taken inside {BOUND} ticks");
    }
}

#[test]
fn conceding_a_corner_brings_the_defence_back_into_its_own_box() {
    let mut m = CornerMatch::new();
    m.crowd_the_centre_circle();
    assert_eq!(
        m.in_the_box(PlayerSide::Left),
        0,
        "fixture check: the box starts empty"
    );

    m.concede_a_corner();

    assert_eq!(
        m.field.ball.pass_origin_restart,
        PassOriginRestart::Corner,
        "the ball behind off a defender is a corner"
    );
    m.walk_the_corner_in();
    let defenders = m.in_the_box(PlayerSide::Left);
    assert!(
        defenders >= 6,
        "a corner is defended by the whole side, not the goalkeeper alone — \
         {defenders} outfielders in the box"
    );
    let attackers = m.in_the_box(PlayerSide::Right);
    assert!(
        (3..=7).contains(&attackers),
        "the attacking box load should be a corner, not an evacuation: {attackers}"
    );
}

#[test]
fn the_shape_is_still_there_when_the_delivery_arrives() {
    let mut m = CornerMatch::new();
    m.crowd_the_centre_circle();
    m.concede_a_corner();
    m.walk_the_corner_in();
    let at_setup = m.in_the_box(PlayerSide::Left);

    // The cross leaves the taker ~5 ticks after he sets the ball down and
    // takes about a second and a half to reach the far post. Half a second
    // in is comfortably inside the delivery.
    m.tick_n(50);

    let arriving = m.in_the_box(PlayerSide::Left);
    assert!(
        arriving + 2 >= at_setup,
        "the box emptied while the ball was in the air: {at_setup} at the award, \
         {arriving} at the delivery"
    );
    assert!(
        arriving >= 5,
        "only {arriving} defenders were still in the box when the cross came in"
    );
}

#[test]
fn the_shape_lets_go_of_everyone_once_the_corner_is_over() {
    let mut m = CornerMatch::new();
    m.crowd_the_centre_circle();
    m.concede_a_corner();
    assert!(
        !m.pinned().is_empty(),
        "fixture check: the corner pins somebody"
    );

    // Past the hard ceiling on the pin, which runs from the KICK — so the
    // set-up has to be walked out first or the 400 ticks below are spent
    // on the taker's run and the deadline has not started.
    m.walk_the_corner_in();
    m.tick_n(400);

    let held: Vec<u32> = m.pinned().iter().map(|p| p.id).collect();
    assert!(
        held.is_empty(),
        "players are still pinned to a corner that finished long ago: {held:?}"
    );
    assert!(
        m.field.ball.corner_shape.is_none(),
        "the corner shape was never released"
    );
}

/// **The ball is not moved to the flag when the corner is awarded.**
///
/// It used to be, on the same tick, from wherever along the byline it went
/// out — a mean 220u, **27.5 m**, and the largest relocation left in the
/// engine. The taker was teleported after it and the other twenty were
/// written into the shape, so a replay showed the ball, one player and
/// then everybody else jump at once.
#[test]
fn awarding_a_corner_moves_nothing() {
    let mut m = CornerMatch::new();
    m.crowd_the_centre_circle();
    let before: Vec<(u32, Vector3<f32>)> =
        m.field.players.iter().map(|p| (p.id, p.position)).collect();

    m.concede_a_corner();

    let ball = m.field.ball.position;
    assert!(
        ball.y > 60.0,
        "the ball was written onto the flag instead of dying where it went \
         out, at {ball:?}"
    );
    assert!(
        m.field.ball.current_owner.is_none(),
        "a corner nobody has walked to yet belongs to nobody"
    );
    assert_eq!(
        m.field.ball.pending_set_piece_teleport, None,
        "the taker was teleported onto the ball"
    );
    // One tick of ordinary movement is expected — they are all under their
    // own AI. What must not have happened is a placement.
    for (id, was) in before {
        let now = m
            .field
            .players
            .iter()
            .find(|p| p.id == id)
            .expect("nobody left the pitch")
            .position;
        let moved = (now - was).magnitude();
        assert!(
            moved < 5.0,
            "player {id} was placed {moved:.1}u into the corner shape — \
             the twenty walk into it, they are not written there"
        );
    }
    assert!(
        !m.pinned().is_empty(),
        "…but they do get stations to walk to"
    );
}

#[test]
fn the_taker_is_on_the_ball_and_the_ball_is_on_the_flag() {
    let mut m = CornerMatch::new();
    m.crowd_the_centre_circle();
    m.concede_a_corner();
    m.walk_the_corner_in();

    // Within a stride of the arc, not exactly on it.
    //
    // The taker CARRIES the ball to the flag and the kick goes live when
    // he arrives, where "arrives" is `AwaitedRestart::REACH` — 1.5 m,
    // sized so `SteeringBehavior::Arrive`'s 3 u deadzone cannot deadlock
    // against the spot. So the ball ends up wherever he stopped, and
    // nothing writes it the rest of the way: that write would be the
    // teleport this whole mechanism exists to remove, on the one axis and
    // at the one moment a replay is already watching. A referee gives the
    // same tolerance.
    let ball = m.field.ball.position;
    let arc = Vector3::new(
        AwaitedRestart::SPOT_INSET,
        if ball.y < HEIGHT as f32 * 0.5 {
            AwaitedRestart::SPOT_INSET
        } else {
            HEIGHT as f32 - AwaitedRestart::SPOT_INSET
        },
        0.0,
    );
    let off_the_arc = (ball.xy() - arc.xy()).magnitude();
    assert!(
        off_the_arc <= AwaitedRestart::REACH,
        "the corner is taken from the flag, not {off_the_arc:.1}u away at {ball:?}"
    );
    // `current_owner` OR the last toucher: he owns it the tick he sets it
    // down and the cross can leave inside the same tick, which clears the
    // ownership again. Either answer is "he took the corner".
    let taker = m
        .field
        .ball
        .current_owner
        .or(m.field.ball.last_touch_player_id)
        .and_then(|id| m.field.players.iter().find(|p| p.id == id))
        .expect("a corner has a taker");
    assert_eq!(
        taker.side,
        Some(PlayerSide::Right),
        "the corner belongs to the side that did not put it out"
    );
    assert!(
        (taker.position - ball).magnitude() < 15.0,
        "the taker is standing {}u from his own corner",
        (taker.position - ball).magnitude()
    );
}

/// How long the set-up actually takes, and what the taker is doing while
/// it runs.
///
/// The census over full matches says **44% of corners never complete the
/// fetch** — the taker times out a mean 35 m short — and raising the bound
/// from 12 s to 30 s barely moved it, which rules out "he ran out of
/// clock". This drives one corner through real ticks and reports the two
/// numbers that tell a man who is coming from a man who is not.
#[test]
fn the_taker_actually_runs_at_the_ball() {
    let mut m = CornerMatch::new();
    m.crowd_the_centre_circle();
    m.concede_a_corner();

    let taker = m
        .field
        .ball
        .awaiting_restart
        .expect("a corner is an awaited restart")
        .taker_id;
    let spot = m.field.ball.awaiting_restart.unwrap().spot;
    let start = m
        .field
        .players
        .iter()
        .find(|p| p.id == taker)
        .expect("the taker is on the pitch")
        .position;
    let fetch = (start - spot).magnitude();

    // Half a second in he must have set off, and in the states that mean
    // he is going for it.
    m.tick_n(50);
    let after = m
        .field
        .players
        .iter()
        .find(|p| p.id == taker)
        .expect("the taker is on the pitch");
    let covered = (start - after.position).magnitude();
    let state = after.state;
    assert!(
        covered > 5.0,
        "the taker had {fetch:.0}u to cover and moved {covered:.1}u in half a \
         second — he is in {state}, not on his way"
    );

    let elapsed = m.walk_the_corner_in();
    // He is sprinting: `TakeBall` is `ActivityIntensity::VeryHigh`. At the
    // engine's ~0.6 u/tick that is a bit over one tick per unit, and the
    // carry to the flag is on top. Ten times that is a generous ceiling
    // and still nowhere near the 3000-tick bound.
    let budget = (fetch * 10.0) as usize + 600;
    assert!(
        elapsed < budget,
        "the set-up took {elapsed} ticks for a {fetch:.0}u fetch — budget {budget}"
    );
}
