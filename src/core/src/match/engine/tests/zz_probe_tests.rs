#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::RunOff;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::result::Score;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, PlayerSide,
    ResultMatchPositionData,
};
use nalgebra::Vector3;

const WIDTH: usize = 840;
const HEIGHT: usize = 545;

struct M {
    field: MatchField,
    context: MatchContext,
    tick_context: GameTickContext,
    recording: ResultMatchPositionData,
}

impl M {
    fn new() -> Self {
        let mut home = squad(1, 100);
        let mut away = squad(2, 200);
        for s in [&mut home, &mut away] {
            for p in s.main_squad.iter_mut() {
                p.skills.physical.acceleration = 14.0;
                p.skills.physical.agility = 14.0;
                p.skills.physical.stamina = 14.0;
            }
        }
        let players = MatchPlayerCollection::from_squads(&home, &away);
        let field = MatchField::new(WIDTH, HEIGHT, home, away);
        let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
        context.total_match_time = 10 * 60 * 1000;
        let tick_context = GameTickContext::new(&field, &context.players);
        M { field, context, tick_context, recording: ResultMatchPositionData::empty() }
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
}

/// A ball skied over the bar, awarded a goal kick, running out behind the
/// left goal — then the taker is SUBSTITUTED (his slot in `field.players`
/// gets a new id), exactly what `MatchField::substitute_player` does.
#[test]
fn probe_taker_vanishes_mid_restart() {
    eprintln!("RunOff::armed = {}", RunOff::armed());
    let mut m = M::new();
    let shooter = m
        .field
        .players
        .iter()
        .find(|p| p.side == Some(PlayerSide::Right) && !p.tactical_position.current_position.is_goalkeeper())
        .map(|p| (p.id, p.team_id))
        .unwrap();
    let goal_y = m.context.goal_positions.left.y;
    // over the bar, straight down the middle
    m.field.ball.position = Vector3::new(-1.0, goal_y, 3.2);
    m.field.ball.velocity = Vector3::new(-1.4, 0.0, 0.0);
    m.field.ball.current_owner = None;
    m.field.ball.previous_owner = Some(shooter.0);
    let tick = m.context.current_tick();
    m.field.ball.record_touch(shooter.0, shooter.1, tick, true);
    m.field.ball.last_shot_struck_tick = tick;
    m.tick();
    let aw = m.field.ball.awaiting_restart.expect("goal kick awarded");
    eprintln!("origin={:?} settled={} taker={}", aw.origin, aw.settled, aw.taker_id);
    let taker = aw.taker_id;

    // let it run out and settle
    for _ in 0..150 {
        m.tick();
        if m.field.ball.awaiting_restart.map(|r| r.settled) == Some(true) { break; }
    }
    let p = m.field.ball.position;
    eprintln!("settled at ({:.1},{:.1},{:.2}), prev_owner={:?}", p.x, p.y, p.z, m.field.ball.previous_owner);

    // The substitution: the taker's slot becomes a different player.
    let score_before = (m.context.score.home_team.get(), m.context.score.away_team.get());
    if let Some(slot) = m.field.players.iter_mut().find(|p| p.id == taker) {
        slot.id = 9999;
    }
    m.tick();
    let p = m.field.ball.position;
    eprintln!(
        "after the sub: awaiting={:?} goal_scored={} in_net={:?} ball ({:.1},{:.1},{:.2}) score {:?} -> {:?}",
        m.field.ball.awaiting_restart.map(|r| r.origin),
        m.field.ball.goal_scored,
        m.field.ball.in_net.is_some(),
        p.x, p.y, p.z,
        score_before,
        (m.context.score.home_team.get(), m.context.score.away_team.get())
    );
    for i in 0..5 {
        m.tick();
        let p = m.field.ball.position;
        eprintln!(
            "  +{i}: awaiting={:?} goal_scored={} ball ({:.1},{:.1},{:.2}) score {:?}",
            m.field.ball.awaiting_restart.map(|r| r.origin),
            m.field.ball.goal_scored,
            p.x, p.y, p.z,
            (m.context.score.home_team.get(), m.context.score.away_team.get())
        );
    }
}
