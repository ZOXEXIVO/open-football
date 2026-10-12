//! What a player's nerves do to what happens on the pitch, measured where
//! the football happens rather than on the lapse lane's formulas.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::{PassOriginRestart, PhaseOrigin};
use crate::r#match::engine::goal::GOAL_WIDTH as GOAL_MOUTH_HALF_WIDTH;
use crate::r#match::engine::psychology::PsychState;
use crate::r#match::player::strategies::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::engine::result::Score;
use crate::r#match::events::{EventCollection, EventDispatcher};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::players::ops::xg::ShotType;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, MatchPlayerLite, PlayerSide,
    ResultMatchPositionData, StateProcessingContext,
};
use nalgebra::Vector3;

const KICKOFF_MS: u64 = 10 * 60 * 1000;
const GOAL_CENTRE_Y: f32 = 545.0 / 2.0;

fn kickoff() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = KICKOFF_MS;
    (field, context)
}

fn home_forward(field: &MatchField) -> u32 {
    field
        .players
        .iter()
        .filter(|p| p.side == Some(PlayerSide::Left))
        .find(|p| p.tactical_position.current_position.is_forward())
        .map(|p| p.id)
        .expect("a home forward")
}

fn strike(field: &mut MatchField, context: &mut MatchContext, shooter: u32, shot_type: ShotType) {
    let mut events = EventCollection::with_capacity(4);
    events.add_player_event(PlayerEvent::Shoot(ShootingEventContext {
        from_player_id: shooter,
        target: Vector3::new(840.0, GOAL_CENTRE_Y, 1.0),
        force: 1.0,
        reason: "nerves_outcome_tests",
        tick: context.current_tick(),
        shot_type,
    }));
    let mut match_data = ResultMatchPositionData::empty();
    EventDispatcher::dispatch(&mut events, field, context, &mut match_data, true);
}

/// Share of `n` identical strikes from the edge of the area that head wide
/// of the posts, with the shooter carrying `head`.
fn wide_share(head: PsychState, n: u32) -> f32 {
    let (mut field, mut context) = kickoff();
    let shooter = home_forward(&field);
    if let Some(player) = field.get_player_mut(shooter) {
        let s = &mut player.skills;
        s.technical.finishing = 17.0;
        s.technical.technique = 17.0;
        s.technical.long_shots = 17.0;
        s.mental.composure = 17.0;
        s.mental.decisions = 17.0;
    }
    context.psychology.players.insert(shooter, head);
    let spot = Vector3::new(700.0, GOAL_CENTRE_Y, 0.0);
    let mut wide = 0;
    for _ in 0..n {
        context.total_match_time += 2_000;
        if let Some(player) = field.get_player_mut(shooter) {
            player.position = spot;
        }
        field.ball.position = Vector3::new(spot.x, spot.y, 0.1);
        field.ball.velocity = Vector3::zeros();
        field.ball.current_owner = Some(shooter);
        field.ball.previous_owner = None;
        strike(&mut field, &mut context, shooter, ShotType::FootOpenPlay);
        // Where the strike crosses the line, as the engine projected it.
        if let Some(target) = field.ball.cached_shot_target.as_ref()
            && (target.goal_line_y - GOAL_CENTRE_Y).abs() > GOAL_MOUTH_HALF_WIDTH
        {
            wide += 1;
        }
    }
    wide as f32 / n as f32
}

#[test]
fn nerves_cost_finishing() {
    let settled = wide_share(PsychState::default(), 1_000);
    let nervous = wide_share(
        PsychState {
            nervousness: 0.6,
            ..PsychState::default()
        },
        1_000,
    );
    // A shot is mostly the skill and the chance: nerves cost a striker some
    // of his finishing, never all of it.
    assert!(
        nervous > settled + 0.03 && nervous < settled + 0.25,
        "wide {nervous:.3} nervous against {settled:.3} settled"
    );
}

/// Ticks out of `ticks` the home centre-back is late to an away forward
/// running at his goal, with the centre-back carrying `head`.
fn late_ticks(head: PsychState, running: bool, ticks: u64) -> u32 {
    let (mut field, mut context) = kickoff();
    let marker = field
        .players
        .iter()
        .find(|p| p.side == Some(PlayerSide::Left) && p.tactical_position.current_position.is_defender())
        .map(|p| p.id)
        .expect("a home defender");
    let man = home_forward(&field) + 100;
    let goal = context.goal_positions.left;
    let lite = field
        .players
        .iter_mut()
        .find(|p| p.id == man)
        .map(|p| {
            let run = (goal - p.position).try_normalize(0.01).unwrap_or_default();
            p.velocity = if running { run * 0.5 } else { run * 0.1 };
            MatchPlayerLite {
                id: p.id,
                position: p.position,
                tactical_positions: p.tactical_position.current_position,
            }
        })
        .expect("an away forward");
    context.psychology.players.insert(marker, head);
    let player = field.get_player(marker).unwrap().clone();
    let mut late = 0;
    for t in 0..ticks {
        context.total_match_time = KICKOFF_MS + t * 10;
        let tick_context = GameTickContext::new(&field, &context.players);
        let ctx = StateProcessingContext {
            in_state_time: 0,
            player: &player,
            context: &context,
            tick_context: &tick_context,
        };
        if DefenderSkillProfile::from_ctx(&ctx).late_to_the_run(&ctx, &lite) {
            late += 1;
        }
    }
    late
}

#[test]
fn a_nervous_marker_is_late_to_more_runs() {
    let settled = late_ticks(PsychState::default(), true, 12_000);
    let nervous = late_ticks(
        PsychState {
            nervousness: 0.6,
            ..PsychState::default()
        },
        true,
        12_000,
    );
    assert!(nervous > settled * 2, "late {nervous} ticks nervous against {settled} settled");
    assert_eq!(late_ticks(PsychState::default(), false, 3_000), 0, "a man jogging is not lost");
}

/// A penalty is the taker's miss until it goes in as a penalty.
#[test]
fn a_penalty_is_a_miss_until_it_goes_in() {
    let (mut field, mut context) = kickoff();
    let taker = home_forward(&field);
    let spot = Vector3::new(840.0 - 88.0, GOAL_CENTRE_Y, 0.0);
    if let Some(player) = field.get_player_mut(taker) {
        player.position = spot;
    }
    field.ball.position = Vector3::new(spot.x, spot.y, 0.1);
    field.ball.current_owner = Some(taker);
    strike(&mut field, &mut context, taker, ShotType::Penalty);
    assert_eq!(context.penalty_misses, vec![taker]);

    let team = field.get_player(taker).unwrap().team_id;
    let tick = context.current_tick();
    field.ball.phase_origin = Some(PhaseOrigin::open(PassOriginRestart::Penalty, team, taker, tick));
    let mut data = ResultMatchPositionData::empty();
    let mut events = EventCollection::with_capacity(2);
    events.add_player_event(PlayerEvent::Goal(taker, false));
    EventDispatcher::dispatch(&mut events, &mut field, &mut context, &mut data, true);
    assert!(context.penalty_misses.is_empty(), "{:?}", context.penalty_misses);
}
