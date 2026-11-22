//! **The stance moves only the levers a side really uses.** A side
//! protecting a lead runs at full pace, may pass at once, keeps its live
//! counter, and weighs a chance exactly as it would with the score level.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::PlayerPositionType;
use crate::r#match::defenders::states::DefenderRunningState;
use crate::r#match::engine::result::Score;
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::forward_shot_decision::ShotDecision;
use crate::r#match::{
    CoachCard, DefensivePlan, GamePlan, GameTickContext, MatchContext, MatchField, MatchPlayer,
    MatchPlayerCollection, Outlook, ReadCounters, RollingTeamMetrics, Situation, SquadFit,
    StateChangeResult, StateProcessingContext, StateProcessingHandler, StateProcessor,
    TacticalNeed,
};
use nalgebra::Vector3;

const HOME: u32 = 1;
const AWAY: u32 = 2;

fn pitch() -> (MatchField, MatchContext) {
    pitches().0
}

/// The same two squads on two pitches, so two situations can be compared
/// with nothing else different.
fn pitches() -> ((MatchField, MatchContext), (MatchField, MatchContext)) {
    let (home, away) = (squad(HOME, 100), squad(AWAY, 200));
    let build = || {
        let players = MatchPlayerCollection::from_squads(&home, &away);
        let field = MatchField::new(840, 545, home.clone(), away.clone());
        let context = MatchContext::new(&field, players, Score::new(HOME, AWAY), false, false);
        (field, context)
    };
    (build(), build())
}

/// The home side one up in the 85th minute with its stance turned fully
/// toward protecting the lead — as far as the levers will ever see it.
fn protecting(context: &mut MatchContext) {
    context.score.increment_home_goals();
    context.total_match_time = 85 * 60_000;
    let tactical = &mut context.tactical_home;
    tactical.stance = -1.0;
    tactical.game_management_intensity = 1.0;
    tactical.risk_appetite = (tactical.plan_risk - 0.2).max(0.05);
}

fn level_late(context: &mut MatchContext) {
    context.total_match_time = 85 * 60_000;
}

fn home_player(field: &MatchField, outfield: impl Fn(&MatchPlayer) -> bool) -> MatchPlayer {
    field
        .players
        .iter()
        .find(|p| p.team_id == HOME && outfield(p))
        .expect("a home player")
        .clone()
}

/// A state that asks for one fixed run and nothing else.
struct Run;

impl StateProcessingHandler for Run {
    fn process(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(-0.55, 0.05, 0.0))
    }
}

fn run_velocity(context: &MatchContext, field: &MatchField) -> Vector3<f32> {
    let mut player = home_player(field, |p| {
        p.tactical_position.current_position.is_defender()
    });
    player.position = Vector3::new(400.0, 272.0, 0.0);
    let tick = GameTickContext::new(field, &context.players);
    StateProcessor::new(0, &mut player, context, &tick)
        .process_inner(Run)
        .velocity
        .expect("the run has a velocity")
}

#[test]
fn a_protecting_side_s_recovering_defender_runs_as_fast_as_at_level() {
    let ((field, mut level), (_, mut ahead)) = pitches();
    level_late(&mut level);
    protecting(&mut ahead);
    let (at_level, protecting_lead) = (run_velocity(&level, &field), run_velocity(&ahead, &field));
    assert!(
        (at_level.norm() - protecting_lead.norm()).abs() < 1e-6,
        "level {at_level:?}, protecting {protecting_lead:?}"
    );
}

fn context_for<'a>(
    player: &'a MatchPlayer,
    context: &'a MatchContext,
    tick: &'a GameTickContext,
) -> StateProcessingContext<'a> {
    StateProcessingContext {
        in_state_time: 0,
        player,
        context,
        tick_context: tick,
    }
}

#[test]
fn keeping_the_ball_follows_the_plan_and_the_stance() {
    let (field, mut context) = pitch();
    let player = home_player(&field, |p| {
        !p.tactical_position.current_position.is_goalkeeper()
    });
    let tick = GameTickContext::new(&field, &context.players);

    context.coach_home.plan = GamePlan::neutral();
    assert!(
        !context_for(&player, &context, &tick)
            .team()
            .prefers_keeping_the_ball()
    );

    context.tactical_home.stance = -0.25;
    assert!(
        context_for(&player, &context, &tick)
            .team()
            .prefers_keeping_the_ball()
    );

    context.tactical_home.stance = 0.0;
    context.coach_home.plan = GamePlan::from_pin("tempo=0.2/direct=0.2").unwrap();
    assert!(
        context_for(&player, &context, &tick)
            .team()
            .prefers_keeping_the_ball()
    );
}

#[test]
fn a_protecting_carrier_with_an_open_team_mate_passes_on_the_first_tick() {
    let (mut field, mut context) = pitch();
    protecting(&mut context);
    let carrier = home_player(&field, |p| {
        p.tactical_position.current_position == PlayerPositionType::DefenderCenterLeft
    })
    .id;
    // The opposition is back in its own half; nobody is near him.
    for (i, p) in field
        .players
        .iter_mut()
        .filter(|p| p.team_id == AWAY)
        .enumerate()
    {
        p.position = Vector3::new(620.0 + 10.0 * (i % 3) as f32, 40.0 + 45.0 * i as f32, 0.0);
    }
    let at = Vector3::new(250.0, 272.0, 0.0);
    field.get_player_mut(carrier).unwrap().position = at;
    field.ball.position = at;
    field.ball.current_owner = Some(carrier);
    field.ball.ownership_duration = 0;
    let player = field.get_player(carrier).unwrap().clone();
    let mut tick = GameTickContext::new(&field, &context.players);
    tick.refresh_ball(&field);
    tick.possession.update(&field);
    let passed = DefenderRunningState::default()
        .process(&context_for(&player, &context, &tick))
        .is_some_and(|mut result| {
            result
                .events
                .drain()
                .any(|event| matches!(event, Event::PlayerEvent(PlayerEvent::PassTo(_))))
        });
    assert!(passed, "he plays it on the first tick");
}

#[test]
fn a_protecting_side_still_breaks_when_the_counter_is_on() {
    let (mut field, mut context) = pitch();
    protecting(&mut context);
    let carrier = home_player(&field, |p| {
        p.tactical_position.current_position.is_midfielder()
    })
    .id;
    // Won the ball this instant, with five of theirs caught upfield in our half.
    context.coach_home.last_possession_gain_tick = context.current_tick();
    let mut caught = 0;
    for p in field.players.iter_mut().filter(|p| p.team_id == AWAY) {
        if caught < 5 && !p.tactical_position.current_position.is_goalkeeper() {
            p.position = Vector3::new(250.0, 100.0 + 60.0 * caught as f32, 0.0);
            caught += 1;
        }
    }
    let ball_at = Vector3::new(300.0, 272.0, 0.0);
    field.get_player_mut(carrier).unwrap().position = ball_at;
    field.ball.position = ball_at;
    field.ball.current_owner = Some(carrier);
    let player = field.get_player(carrier).unwrap().clone();
    let mut tick = GameTickContext::new(&field, &context.players);
    tick.refresh_ball(&field);
    tick.possession.update(&field);
    let ctx = context_for(&player, &context, &tick);
    assert!(
        ctx.team().prefers_keeping_the_ball(),
        "the side means to keep it"
    );
    assert!(ctx.team().counter_window(), "the counter is on");
    assert!(!ctx.team().should_play_possession(), "and it breaks");
}

#[test]
fn a_protecting_side_s_chance_is_weighed_as_at_level() {
    let shooter_at = |field: &mut MatchField| {
        let shooter = home_player(field, |p| p.tactical_position.current_position.is_forward()).id;
        for (i, p) in field.players.iter_mut().enumerate() {
            if p.id != shooter && !p.tactical_position.current_position.is_goalkeeper() {
                p.position = Vector3::new(200.0, 40.0 + 10.0 * i as f32, 0.0);
            }
        }
        let at = Vector3::new(730.0, 260.0, 0.0);
        field.get_player_mut(shooter).unwrap().position = at;
        field.ball.position = at;
        field.ball.current_owner = Some(shooter);
        shooter
    };
    let ((level_field, level), (ahead_field, ahead)) = pitches();
    let decide = |mut field: MatchField, mut context: MatchContext, is_ahead: bool| {
        if is_ahead {
            protecting(&mut context);
        } else {
            level_late(&mut context);
        }
        let shooter = shooter_at(&mut field);
        let player = field.get_player(shooter).unwrap().clone();
        let mut tick = GameTickContext::new(&field, &context.players);
        tick.refresh_ball(&field);
        tick.possession.update(&field);
        format!(
            "{:?}",
            ShotDecision::evaluate(&context_for(&player, &context, &tick), "TEST")
        )
    };
    assert_eq!(
        decide(ahead_field, ahead, true),
        decide(level_field, level, false)
    );
}

#[test]
fn a_fifteenth_minute_goal_moves_both_stances_and_the_bench_on_the_next_reading() {
    let (_, mut context) = pitch();
    for coach in [&mut context.coach_home, &mut context.coach_away] {
        coach.on_kickoff(
            CoachCard::neutral(),
            Outlook::default(),
            &SquadFit::default(),
            None,
        );
    }
    let read = RollingTeamMetrics::default();
    let counters = ReadCounters::default();
    let observe = |context: &mut MatchContext, home_difference: i32, minute: f32| {
        let at = |difference: i32| Situation {
            difference,
            minutes_left: 90.0 - minute,
            ..Situation::default()
        };
        context
            .coach_home
            .observe(at(home_difference), minute, counters);
        context
            .coach_away
            .observe(at(-home_difference), minute, counters);
        let need = |stance: f32| TacticalNeed::from_coach(stance, read, 0.8);
        (
            context.coach_home.stance(),
            context.coach_away.stance(),
            need(context.coach_away.stance()),
        )
    };
    let (home_before, away_before, away_need_before) = observe(&mut context, 0, 14.9);
    context.total_match_time = 15 * 60_000;
    context.on_goal(true);
    let (home_after, away_after, away_need_after) = observe(&mut context, 1, 15.0 + 1.0 / 12.0);
    assert!(home_after < home_before, "{home_after} {home_before}");
    assert!(away_after > away_before, "{away_after} {away_before}");
    assert_ne!(
        away_need_after, away_need_before,
        "the side behind wants a goal"
    );
}

#[test]
fn a_ball_played_past_the_last_line_counts_one_break() {
    let (mut field, mut context) = pitch();
    // The home side defends the left goal with its outfielders on a line
    // 37.5 m out; the away side has a runner past it.
    for p in field.players.iter_mut().filter(|p| p.team_id == HOME) {
        p.position.x = if p.tactical_position.current_position.is_goalkeeper() {
            20.0
        } else {
            300.0
        };
    }
    let runner = field
        .players
        .iter()
        .find(|p| p.team_id == AWAY && p.tactical_position.current_position.is_forward())
        .expect("an away forward")
        .id;
    field.ball.kickoff_taker = None;
    let mut play_to = |field: &mut MatchField, x: f32| {
        field.get_player_mut(runner).unwrap().position = Vector3::new(x, 272.0, 0.0);
        field.ball.position = Vector3::new(x, 272.0, 0.0);
        field.ball.current_owner = Some(runner);
        context
            .coach_home
            .note_line(DefensivePlan::line_broken(field, HOME));
        context.coach_home.cum_line_breaks_against
    };
    assert_eq!(play_to(&mut field, 360.0), 0, "in front of the line");
    assert_eq!(play_to(&mut field, 250.0), 1, "through");
    assert_eq!(play_to(&mut field, 240.0), 1, "still the same break");
    assert_eq!(play_to(&mut field, 360.0), 1, "back in front");
    assert_eq!(play_to(&mut field, 250.0), 2, "through again");
}
