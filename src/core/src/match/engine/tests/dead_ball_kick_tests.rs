//! Dead-ball kicks end to end. A penalty: the area clears, the taker
//! strikes it, the keeper guesses and dives, and the ball goes in, is
//! saved or misses. A direct free kick: the taker shoots or plays it, and
//! a shot beats the wall and the keeper or does not. A corner: the taker
//! delivers it, somebody gets to it first, and the attack does or does not
//! get a shot and a goal out of it.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::flow::rng::MatchRng;
use crate::r#match::engine::officiating::restart_shape::RestartShape;
use crate::r#match::engine::player::events::players::{FoulSeverity, PlayerEventDispatcher};
use crate::r#match::engine::result::Score;
use crate::r#match::engine::set_pieces::CornerRoutine;
use crate::r#match::squad::squad::MatchSquad;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, PassOriginRestart,
    ResultMatchPositionData,
};
use crate::{PlayerFieldPositionGroup, PlayerSkills};
use nalgebra::Vector3;

const WIDTH: f32 = 840.0;
const HEIGHT: f32 = 545.0;
const GOAL_HALF_WIDTH: f32 = 29.0;
const CROSSBAR: f32 = 2.44;
/// Where the keeper stands, a metre off his line: the ball is read there
/// rather than at the goal line, so a save and a goal are measured alike.
const KEEPER_PLANE: f32 = 828.0;
/// A foul 7.5 m out gives a penalty, one 22 m out a free kick in range.
const PENALTY_FOUL: f32 = 60.0;
const FREE_KICK_FOUL: f32 = 176.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Outcome {
    Goal,
    Saved,
    Missed,
    Unresolved,
}

#[derive(Debug, Clone, Copy)]
struct Kick {
    outcome: Outcome,
    on_frame: bool,
    /// Which side of the goal the ball went, and which the keeper dived
    /// to: -1, 0 (central), +1.
    ball_side: i32,
    dive_side: Option<i32>,
}

fn keeper_of(field: &MatchField, team_id: u32) -> u32 {
    field
        .players
        .iter()
        .find(|p| {
            p.team_id == team_id
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
        })
        .map(|p| p.id)
        .unwrap()
}

fn tick(field: &mut MatchField, context: &mut MatchContext) {
    let mut data = ResultMatchPositionData::empty();
    let mut tick_context = GameTickContext::new(field, &context.players);
    FootballEngine::<840, 545>::game_tick(field, context, &mut data, &mut tick_context);
    context.increment_time();
}

fn side_of(offset: f32) -> i32 {
    if offset.abs() < 6.0 {
        0
    } else if offset > 0.0 {
        1
    } else {
        -1
    }
}

/// Every technical, mental and keeping attribute at `level`: footballers
/// of one standard, rather than the fixture's blank skill sheet.
fn at_level(skills: &mut PlayerSkills, level: f32) {
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
        *v = level;
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
        *v = level;
    }
    let g = &mut skills.goalkeeping;
    for v in [
        &mut g.aerial_reach,
        &mut g.command_of_area,
        &mut g.communication,
        &mut g.eccentricity,
        &mut g.first_touch,
        &mut g.handling,
        &mut g.kicking,
        &mut g.one_on_ones,
        &mut g.passing,
        &mut g.punching,
        &mut g.reflexes,
        &mut g.rushing_out,
        &mut g.throwing,
    ] {
        *v = level;
    }
}

fn levelled(team_id: u32, base_id: u32, level: f32) -> MatchSquad {
    let mut squad = squad(team_id, base_id);
    for p in squad
        .main_squad
        .iter_mut()
        .chain(squad.substitutes.iter_mut())
    {
        at_level(&mut p.skills, level);
    }
    squad
}

fn pitch(level: f32, seed: u64) -> (MatchField, MatchContext) {
    let home = levelled(1, 100, level);
    let away = levelled(2, 300, level);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = 60 * 60_000;
    context.rng = MatchRng::from_seed(seed);
    // The kickoff reading a match takes, so skills are read against the
    // standard of these two sides.
    FootballEngine::<840, 545>::refresh_tactical_states(&field, &mut context, 1);
    (field, context)
}

/// A defender brings a man down `depth` units out from the right-hand
/// goal, in line with it.
fn award(field: &mut MatchField, context: &mut MatchContext, depth: f32) -> PassOriginRestart {
    let fouler = field
        .players
        .iter()
        .find(|p| {
            p.team_id == 2
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Defender
        })
        .map(|p| p.id)
        .unwrap();
    let foul_at = Vector3::new(WIDTH - depth, HEIGHT * 0.5, 0.0);
    field.ball.position = foul_at;
    PlayerEventDispatcher::award_restart_for_foul(
        fouler,
        FoulSeverity::Normal,
        foul_at,
        field,
        context,
    );
    field.ball.pass_origin_restart
}

fn take_penalty(level: f32, seed: u64) -> Kick {
    let (mut field, mut context) = pitch(level, seed);
    let keeper = keeper_of(&field, 2);
    assert_eq!(
        award(&mut field, &mut context, PENALTY_FOUL),
        PassOriginRestart::Penalty
    );
    let goal_y = HEIGHT * 0.5;
    let home_goals = context.score.home_team.get();
    let mut struck_z: Option<f32> = None;
    let mut crossed: Option<f32> = None;
    let mut dive: Option<f32> = None;
    let mut touched_at: Option<u32> = None;
    let outcome = 'kick: {
        for t in 0..4_000 + PassOriginRestart::Penalty.set_routine_ticks() as u32 {
            tick(&mut field, &mut context);
            if context.score.home_team.get() > home_goals {
                break 'kick Outcome::Goal;
            }
            if struck_z.is_none() {
                struck_z = field
                    .ball
                    .cached_shot_target
                    .map(|target| target.goal_line_z);
            }
            if dive.is_none() {
                dive = field.get_player(keeper).and_then(|k| k.dive_aim);
            }
            if struck_z.is_none() {
                continue;
            }
            if crossed.is_none() && field.ball.position.x >= KEEPER_PLANE {
                crossed = Some(field.ball.position.y);
            }
            if touched_at.is_none() && field.ball.last_touch_player_id == Some(keeper) {
                touched_at = Some(t);
            }
            let dead = field
                .ball
                .awaiting_restart
                .is_some_and(|r| r.origin != PassOriginRestart::Penalty);
            if dead {
                break 'kick if touched_at.is_some() {
                    Outcome::Saved
                } else {
                    Outcome::Missed
                };
            }
            if touched_at.is_some_and(|at| t > at + 150) {
                break 'kick Outcome::Saved;
            }
        }
        Outcome::Unresolved
    };
    let offset = crossed.map_or(0.0, |y| y - goal_y);
    Kick {
        outcome,
        on_frame: crossed.is_some()
            && offset.abs() < GOAL_HALF_WIDTH
            && struck_z.is_some_and(|z| z < CROSSBAR),
        ball_side: side_of(offset),
        dive_side: dive.map(|d| side_of(d - goal_y)),
    }
}

#[derive(Debug, Default)]
struct Census {
    kicks: u32,
    goals: u32,
    saved: u32,
    missed: u32,
    unresolved: u32,
    on_frame: u32,
    right_way: u32,
    scored_right_way: u32,
    wrong_way: u32,
    scored_wrong_way: u32,
    saved_wrong_way: u32,
    central: u32,
    central_scored: u32,
}

impl Census {
    fn of(level: f32, seeds: std::ops::Range<u64>) -> Self {
        let mut c = Census::default();
        for seed in seeds {
            let kick = take_penalty(level, 0x9E4A_0000 + seed);
            let scored = kick.outcome == Outcome::Goal;
            c.kicks += 1;
            match kick.outcome {
                Outcome::Goal => c.goals += 1,
                Outcome::Saved => c.saved += 1,
                Outcome::Missed => c.missed += 1,
                Outcome::Unresolved => c.unresolved += 1,
            }
            if !kick.on_frame {
                continue;
            }
            c.on_frame += 1;
            match (kick.ball_side, kick.dive_side) {
                (0, _) => {
                    c.central += 1;
                    c.central_scored += scored as u32;
                }
                (side, Some(dive)) if dive == side => {
                    c.right_way += 1;
                    c.scored_right_way += scored as u32;
                }
                _ => {
                    c.wrong_way += 1;
                    c.scored_wrong_way += scored as u32;
                    c.saved_wrong_way += (kick.outcome == Outcome::Saved) as u32;
                }
            }
        }
        c
    }

    fn conversion(&self) -> f32 {
        self.goals as f32 / self.kicks.max(1) as f32
    }
}

#[test]
fn nobody_but_the_taker_and_the_keeper_is_in_the_area_at_the_kick() {
    // From the kickoff shape nobody is in the area at the award, and the
    // nearest defenders walk into it towards the ball while the taker
    // comes from his own half.
    let (mut field, mut context) = pitch(14.0, 0x9E4A_0006);
    assert_eq!(
        award(&mut field, &mut context, PENALTY_FOUL),
        PassOriginRestart::Penalty
    );
    let mut penalty = None;
    for _ in 0..4_000 + PassOriginRestart::Penalty.set_routine_ticks() {
        tick(&mut field, &mut context);
        if field.ball.cached_shot_target.is_some() {
            break;
        }
        // The mark as awarded: the arc is measured from it, not from
        // wherever the taker has the ball at his feet.
        penalty = penalty.or(field.ball.untaken_set_piece());
    }
    assert!(field.ball.cached_shot_target.is_some(), "never struck");
    let (origin, taker_id, spot) = penalty.expect("never set up");
    let taker = field.get_player(taker_id).unwrap();
    let inside: Vec<(u32, f32, f32)> = field
        .players
        .iter()
        .filter(|p| {
            (WIDTH - p.position.x < 132.0 && (p.position.y - HEIGHT * 0.5).abs() < 161.3)
                || (p.position - spot).xy().norm() < RestartShape::RETREAT
        })
        .map(|p| (p.id, p.position.x, p.position.y))
        .collect();
    assert!(
        RestartShape::formed(origin, &field.players, taker, spot, WIDTH, HEIGHT),
        "taker {taker_id} at {spot:?}, inside: {inside:?}"
    );
}

#[test]
fn penalties_go_in_about_four_times_in_five() {
    let c = Census::of(14.0, 0..40);
    assert_eq!(c.unresolved, 0, "{c:#?}");
    // The woodwork may still keep one out; the keeper may not.
    assert_eq!(
        c.saved_wrong_way, 0,
        "a keeper who went the wrong way kept one out: {c:#?}"
    );
    assert!(
        c.scored_right_way < c.right_way,
        "a keeper who guessed right never saved one: {c:#?}"
    );
    assert!((0.65..=0.92).contains(&c.conversion()), "{c:#?}");
}

#[test]
#[ignore = "census: PK_LEVEL=14 cargo test -p core --lib penalty_census -- --ignored --nocapture"]
fn penalty_census() {
    let level = std::env::var("PK_LEVEL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(14.0);
    let c = Census::of(level, 0..300);
    println!("{c:#?}");
    println!(
        "conversion {:.1}%  saved {:.1}%  missed {:.1}%  on frame {:.1}%",
        c.conversion() * 100.0,
        c.saved as f32 * 100.0 / c.kicks as f32,
        c.missed as f32 * 100.0 / c.kicks as f32,
        c.on_frame as f32 * 100.0 / c.kicks as f32,
    );
}

/// What became of a direct free kick.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FreeKick {
    /// Played to a team-mate.
    Played,
    /// Stopped by the wall, or by a defender past it.
    Blocked,
    Saved,
    Missed,
    Goal,
    Unresolved,
}

/// A direct free kick 22 m out in line with the goal.
fn take_free_kick(level: f32, seed: u64) -> FreeKick {
    let (mut field, mut context) = pitch(level, seed);
    let keeper = keeper_of(&field, 2);
    assert_eq!(
        award(&mut field, &mut context, FREE_KICK_FOUL),
        PassOriginRestart::DirectFreeKick
    );
    let home_goals = context.score.home_team.get();
    let mut handed = false;
    let mut struck_at: Option<u32> = None;
    let mut touched = false;
    let mut blocked = false;
    // Where the ball was heading as it last flew free: a keeper who gets a
    // hand to one going wide or over has not saved a shot on target.
    let mut heading_on_frame = false;
    for t in 0..4_000 + PassOriginRestart::DirectFreeKick.set_routine_ticks() as u32 {
        tick(&mut field, &mut context);
        if context.score.home_team.get() > home_goals {
            return FreeKick::Goal;
        }
        if !handed {
            handed = field.ball.set_piece_kicker.is_some();
            continue;
        }
        if struck_at.is_none() {
            if field.ball.set_piece_kicker.is_some() {
                continue;
            }
            // A pass names its man.
            if field.ball.pass_target_player_id.is_some() {
                return FreeKick::Played;
            }
            struck_at = Some(t);
        }
        let last = field.ball.last_touch_player_id;
        if !touched && field.ball.current_owner.is_none() {
            heading_on_frame = Ball::ballistic_crossing(
                field.ball.position,
                field.ball.velocity,
                field.ball.spin,
                WIDTH,
            )
            .is_some_and(|(y, z, _)| (y - HEIGHT * 0.5).abs() < GOAL_HALF_WIDTH && z < CROSSBAR);
        }
        touched |= last == Some(keeper);
        blocked |= !touched
            && last.is_some_and(|id| {
                id != keeper && field.get_player(id).is_some_and(|p| p.team_id == 2)
            });
        if field.ball.awaiting_restart.is_some() || struck_at.is_some_and(|at| t > at + 300) {
            return if touched && heading_on_frame {
                FreeKick::Saved
            } else if blocked {
                FreeKick::Blocked
            } else {
                FreeKick::Missed
            };
        }
    }
    FreeKick::Unresolved
}

#[derive(Debug, Default)]
struct FreeKickCensus {
    kicks: u32,
    played: u32,
    blocked: u32,
    saved: u32,
    missed: u32,
    goals: u32,
    unresolved: u32,
}

impl FreeKickCensus {
    fn of(level: f32, seeds: std::ops::Range<u64>) -> Self {
        let mut c = Self::default();
        for seed in seeds {
            c.kicks += 1;
            match take_free_kick(level, 0x9E4A_0000 + seed) {
                FreeKick::Played => c.played += 1,
                FreeKick::Blocked => c.blocked += 1,
                FreeKick::Saved => c.saved += 1,
                FreeKick::Missed => c.missed += 1,
                FreeKick::Goal => c.goals += 1,
                FreeKick::Unresolved => c.unresolved += 1,
            }
        }
        c
    }

    fn shots(&self) -> u32 {
        self.blocked + self.saved + self.missed + self.goals
    }

    fn share_of_shots(&self, n: u32) -> f32 {
        n as f32 / self.shots().max(1) as f32
    }
}

#[test]
#[ignore = "census: PK_LEVEL=14 cargo test --release -p core --lib free_kick_census -- --ignored --nocapture"]
fn free_kick_census() {
    let level = std::env::var("PK_LEVEL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(14.0);
    let c = FreeKickCensus::of(level, 0..2_000);
    println!(
        "level {level}: {} kicks, shot {:.1}%  |  of shots: blocked {:.1}%  saved {:.1}%  missed {:.1}%  goal {:.1}%  |  unresolved {}",
        c.kicks,
        c.shots() as f32 * 100.0 / c.kicks as f32,
        c.share_of_shots(c.blocked) * 100.0,
        c.share_of_shots(c.saved) * 100.0,
        c.share_of_shots(c.missed) * 100.0,
        c.share_of_shots(c.goals) * 100.0,
        c.unresolved,
    );
}

#[test]
fn a_free_kick_is_struck_or_played_and_never_carried() {
    let c = FreeKickCensus::of(14.0, 0..30);
    assert_eq!(c.unresolved, 0, "{c:#?}");
    assert_eq!(c.played + c.shots(), c.kicks, "{c:#?}");
    let shot = c.shots() as f32 / c.kicks as f32;
    assert!(
        (0.5..=0.95).contains(&shot),
        "22 m out in line with goal: {c:#?}"
    );
}

#[test]
fn the_wall_stands_off_a_free_kick_until_it_is_kicked() {
    let (mut field, mut context) = pitch(14.0, 0x9E4A_0003);
    assert_eq!(
        award(&mut field, &mut context, FREE_KICK_FOUL),
        PassOriginRestart::DirectFreeKick
    );
    let spot = Vector3::new(WIDTH - FREE_KICK_FOUL, HEIGHT * 0.5, 0.0);
    let mut handed = false;
    for _ in 0..4_000 + PassOriginRestart::DirectFreeKick.set_routine_ticks() {
        tick(&mut field, &mut context);
        if field.ball.set_piece_kicker.is_some() {
            handed = true;
        } else if handed {
            break;
        }
    }
    assert!(handed, "never handed over");
    let inside: Vec<u32> = field
        .players
        .iter()
        .filter(|p| {
            p.team_id == 2
                && !p.tactical_position.current_position.is_goalkeeper()
                && (p.position - spot).xy().norm() < RestartShape::RETREAT
        })
        .map(|p| p.id)
        .collect();
    assert!(inside.is_empty(), "inside 9.15 m at the kick: {inside:?}");
}

/// What became of a corner in the six seconds after it was kicked.
#[derive(Debug, Default, Clone, Copy)]
struct CornerPhase {
    routine: Option<CornerRoutine>,
    kicked: bool,
    lofted: bool,
    /// Who got to it first: an attacker, a defender, the keeper.
    attacker_first: bool,
    defender_first: bool,
    keeper_first: bool,
    shot: bool,
    /// What the shot met first: the keeper's hands or a defender's body.
    saved: bool,
    blocked: bool,
    goal: bool,
}

/// A corner to the home side at the right-hand goal, between sides of
/// `level`: the ball run over the byline off an away defender.
fn take_corner(level: f32, seed: u64) -> CornerPhase {
    let (mut field, mut context) = pitch(level, seed);
    let keeper = keeper_of(&field, 2);
    let defender = field
        .players
        .iter()
        .find(|p| {
            p.team_id == 2
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Defender
        })
        .map(|p| p.id)
        .unwrap();
    field.ball.position = Vector3::new(WIDTH + 1.0, 100.0, 0.0);
    field.ball.velocity = Vector3::new(1.0, 0.0, 0.0);
    field.ball.current_owner = None;
    field.ball.previous_owner = Some(defender);
    let now = context.current_tick();
    field.ball.record_touch(defender, 2, now, true);
    let home_goals = context.score.home_team.get();
    let mut phase = CornerPhase::default();
    let mut handed = false;
    let mut taker = None;
    let mut kicked_at = None;
    let mut shot_at: Option<u64> = None;
    for t in 0..5_000u32 {
        tick(&mut field, &mut context);
        if !handed {
            if field.ball.pass_origin_restart == PassOriginRestart::Corner {
                taker = field.ball.set_piece_kicker;
            }
            handed = taker.is_some();
            continue;
        }
        if kicked_at.is_none() {
            if field.ball.set_piece_kicker.is_some() {
                continue;
            }
            phase.kicked = true;
            phase.lofted = field.ball.velocity.z > 0.02;
            phase.routine = field.ball.pending_corner_routine;
            kicked_at = Some(t);
        }
        if context.score.home_team.get() > home_goals {
            phase.goal = true;
            phase.shot = true;
            return phase;
        }
        let first = phase.attacker_first || phase.defender_first || phase.keeper_first;
        if let Some(id) = field
            .ball
            .last_touch_player_id
            .filter(|id| Some(*id) != taker)
            && !first
            && let Some(p) = field.get_player(id)
        {
            phase.keeper_first = id == keeper;
            phase.defender_first = p.team_id == 2 && id != keeper;
            phase.attacker_first = p.team_id == 1;
        }
        if let Some(at) = shot_at
            && !phase.saved
            && !phase.blocked
            && field.ball.last_touch_tick > at
            && let Some(id) = field.ball.last_touch_player_id
        {
            phase.saved = id == keeper;
            phase.blocked = id != keeper && field.get_player(id).is_some_and(|p| p.team_id == 2);
        }
        if shot_at.is_none()
            && field.ball.cached_shot_target.is_some()
            && field
                .ball
                .last_touch_player_id
                .and_then(|id| field.get_player(id))
                .is_some_and(|p| p.team_id == 1)
        {
            phase.shot = true;
            shot_at = Some(field.ball.last_touch_tick);
        }
        if kicked_at.is_some_and(|at| t > at + 600) {
            return phase;
        }
    }
    phase
}

#[derive(Debug, Default)]
struct CornerCensus {
    /// Shots and corners by routine: near, spot, far, short, edge.
    by_routine: [(u32, u32); 5],
    corners: u32,
    kicked: u32,
    lofted: u32,
    attacker_first: u32,
    defender_first: u32,
    keeper_first: u32,
    shots: u32,
    saved: u32,
    blocked: u32,
    goals: u32,
}

impl CornerCensus {
    fn of(level: f32, seeds: std::ops::Range<u64>) -> Self {
        let mut c = Self::default();
        for seed in seeds {
            let corner = take_corner(level, 0x9E4A_0000 + seed);
            c.corners += 1;
            c.kicked += corner.kicked as u32;
            c.lofted += corner.lofted as u32;
            c.attacker_first += corner.attacker_first as u32;
            c.defender_first += corner.defender_first as u32;
            c.keeper_first += corner.keeper_first as u32;
            c.shots += corner.shot as u32;
            c.saved += corner.saved as u32;
            c.blocked += corner.blocked as u32;
            c.goals += corner.goal as u32;
            if let Some(routine) = corner.routine {
                let slot = match routine {
                    CornerRoutine::NearPost => 0,
                    CornerRoutine::PenaltySpot => 1,
                    CornerRoutine::FarPost => 2,
                    CornerRoutine::Short => 3,
                    CornerRoutine::EdgeCutback => 4,
                };
                c.by_routine[slot].0 += 1;
                c.by_routine[slot].1 += corner.shot as u32;
            }
        }
        c
    }

    fn share(&self, n: u32) -> f32 {
        n as f32 * 100.0 / self.corners.max(1) as f32
    }
}

#[test]
#[ignore = "census: PK_LEVEL=14 cargo test --release -p core --lib corner_census -- --ignored --nocapture"]
fn corner_census() {
    let level = std::env::var("PK_LEVEL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(14.0);
    let c = CornerCensus::of(level, 0..1_000);
    println!(
        "level {level}: {} corners, kicked {:.1}%, lofted {:.1}%  |  first contact: attacker {:.1}%  defender {:.1}%  keeper {:.1}%  |  shot {:.1}%  blocked {:.1}%  saved {:.1}%  goal {:.1}%",
        c.corners,
        c.share(c.kicked),
        c.share(c.lofted),
        c.share(c.attacker_first),
        c.share(c.defender_first),
        c.share(c.keeper_first),
        c.share(c.shots),
        c.share(c.blocked),
        c.share(c.saved),
        c.share(c.goals),
    );
    for (name, (n, shots)) in ["near", "spot", "far", "short", "edge"]
        .iter()
        .zip(c.by_routine)
    {
        println!(
            "  {name}: {n} corners, shot {:.1}%",
            shots as f32 * 100.0 / n.max(1) as f32
        );
    }
}

#[test]
fn a_corner_is_kicked_by_its_taker_and_delivered_into_the_box() {
    let c = CornerCensus::of(14.0, 0..24);
    assert_eq!(c.kicked, c.corners, "{c:#?}");
    let on_the_deck = c.by_routine[3].0 + c.by_routine[4].0;
    assert_eq!(c.lofted + on_the_deck, c.corners, "{c:#?}");
}

#[test]
fn corners_bring_shots_without_the_keeper_owning_the_box() {
    let c = CornerCensus::of(14.0, 0..48);
    let shot = c.share(c.shots);
    let keeper = c.share(c.keeper_first);
    assert!((6.0..=45.0).contains(&shot), "shot {shot:.1}%: {c:#?}");
    assert!(keeper <= 30.0, "keeper first {keeper:.1}%: {c:#?}");
}
