//! **Doors ask the destination's question.** A state that sends a man
//! somewhere and the state he arrives in have to agree about the same ball,
//! or he is handed back on the next tick.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::common_states::LooseBallChase;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::DefensiveLine;
use crate::r#match::engine::result::Score;
use crate::r#match::engine::teamplay::possession::TeamPossession;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::RunInBehind;
use crate::r#match::goalkeepers::states::common::KeeperSweepLimit;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::PressOwnership;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::defenders::states::holding_line::DefenderHoldingLineState;
use crate::r#match::player::strategies::defenders::states::intercepting::DefenderInterceptingState;
use crate::r#match::player::strategies::forwarders::states::creating_space::ForwardCreatingSpaceState;
use crate::r#match::player::strategies::midfielders::states::intercepting::MidfielderInterceptingState;
use crate::r#match::player::strategies::midfielders::states::returning::MidfielderReturningState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, PlayerSide,
    StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const HOME: u32 = 1;
const AWAY: u32 = 2;

/// Both sides stood out of the way in their own corners, ball nowhere near.
fn empty_pitch() -> (MatchField, MatchContext) {
    let home = squad(HOME, 100);
    let away = squad(AWAY, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(840, 545, home, away);
    for (index, player) in field.players.iter_mut().enumerate() {
        let x = if player.team_id == HOME { 30.0 } else { 810.0 };
        player.position = Vector3::new(x, 20.0 + index as f32 * 22.0, 0.0);
        player.velocity = Vector3::zeros();
    }
    field.ball.kickoff_taker = None;
    field.ball.current_owner = None;
    field.ball.pass_target_player_id = None;
    let context = MatchContext::new(&field, players, Score::new(HOME, AWAY), false, false);
    (field, context)
}

fn outfielder(field: &MatchField, team_id: u32, wanted: fn(&PlayerState) -> bool) -> u32 {
    field
        .players
        .iter()
        .find(|p| p.team_id == team_id && wanted(&p.state))
        .expect("a player in that role")
        .id
}

fn defender(field: &MatchField) -> u32 {
    outfielder(field, HOME, |s| matches!(s, PlayerState::Defender(_)))
}

fn midfielder(field: &MatchField) -> u32 {
    outfielder(field, HOME, |s| matches!(s, PlayerState::Midfielder(_)))
}

fn forward(field: &MatchField) -> u32 {
    outfielder(field, HOME, |s| matches!(s, PlayerState::Forward(_)))
}

fn away_man(field: &MatchField, nth: usize) -> u32 {
    field
        .players
        .iter()
        .filter(|p| p.team_id == AWAY)
        .nth(nth)
        .expect("an away player")
        .id
}

fn stand(field: &mut MatchField, id: u32, x: f32, y: f32) {
    field.get_player_mut(id).expect("a player").position = Vector3::new(x, y, 0.0);
}

/// Stand him on his line in `HoldingLine`, wherever the plan has put it with
/// the ball where it is now. Returns the depth, for the rest of the scene.
fn on_the_line(field: &mut MatchField, context: &MatchContext, id: u32) -> f32 {
    let x = {
        let decision = Decision::new(field, context);
        DefensiveLine::position_x(&decision.ctx(id, 40))
    };
    let player = field.get_player_mut(id).expect("a player");
    player.transition_to(
        PlayerState::Defender(DefenderState::HoldingLine),
        TransitionSource::Reset,
    );
    player.position = Vector3::new(x, 270.0, 0.0);
    x
}

/// An away pass in flight to `target`, coming from `from` toward `to`.
fn away_pass(field: &mut MatchField, target: u32, from: (f32, f32), to: (f32, f32)) {
    let (start, end) = (
        Vector3::new(from.0, from.1, 0.0),
        Vector3::new(to.0, to.1, 0.0),
    );
    field.ball.position = start;
    field.ball.velocity = (end - start).normalize() * 2.0;
    field.ball.current_owner = None;
    field.ball.pass_target_player_id = Some(target);
    field.ball.cached_landing_position = start;
    field.ball.flags.in_flight_state = 10;
    field.ball.pending_pass_aim = Some(end);
    // Long past everybody's read of the strike.
    field.ball.current_tick_cached = 1_000;
    field.ball.last_release_tick = 0;
}

struct Decision<'a> {
    field: &'a MatchField,
    context: &'a MatchContext,
    tick: GameTickContext,
}

impl<'a> Decision<'a> {
    fn new(field: &'a MatchField, context: &'a MatchContext) -> Self {
        Decision {
            field,
            context,
            tick: GameTickContext::new(field, &context.players),
        }
    }

    fn ctx(&self, id: u32, in_state_time: u64) -> StateProcessingContext<'_> {
        StateProcessingContext {
            in_state_time,
            player: self.field.get_player(id).expect("a player"),
            context: self.context,
            tick_context: &self.tick,
        }
    }

    fn of(&self, id: u32, handler: impl StateProcessingHandler) -> Option<PlayerState> {
        handler
            .process(&self.ctx(id, 40))
            .and_then(|change| change.state)
    }
}

// ── A player commits to an interception only for a ball he can contest ──

#[test]
fn a_pass_he_cannot_reach_first_leaves_him_in_the_line() {
    let (mut field, context) = empty_pitch();
    let id = defender(&field);
    let target = away_man(&field, 9);
    away_pass(&mut field, target, (300.0, 270.0), (250.0, 270.0));
    let x = on_the_line(&mut field, &context, id);
    // The opponent the pass is for stands right on its line, well ahead of
    // the defender, and it reaches him long before it could reach anyone else.
    stand(&mut field, target, x + 130.0, 270.0);
    away_pass(&mut field, target, (x + 160.0, 270.0), (x + 130.0, 270.0));
    field.ball.velocity *= 2.0;
    let decision = Decision::new(&field, &context);
    assert!(!LooseBallChase::can_contest(&decision.ctx(id, 40)));
    assert_eq!(decision.of(id, DefenderHoldingLineState::default()), None);
}

#[test]
fn a_carried_ball_is_not_an_interception() {
    let (mut field, context) = empty_pitch();
    let id = defender(&field);
    let carrier = away_man(&field, 9);
    let x = on_the_line(&mut field, &context, id);
    stand(&mut field, carrier, x + 60.0, 270.0);
    field.ball.current_owner = Some(carrier);
    field.ball.position = Vector3::new(x + 60.0, 270.0, 0.0);
    field.ball.velocity = Vector3::new(-2.0, 0.0, 0.0);
    on_the_line(&mut field, &context, id);
    let decision = Decision::new(&field, &context);
    assert_ne!(
        decision.of(id, DefenderHoldingLineState::default()),
        Some(PlayerState::Defender(DefenderState::Intercepting)),
    );
}

#[test]
fn a_pass_he_can_reach_first_is_his_to_go_for() {
    let (mut field, context) = empty_pitch();
    let id = defender(&field);
    let target = away_man(&field, 9);
    let x = on_the_line(&mut field, &context, id);
    stand(&mut field, target, 800.0, 100.0);
    away_pass(&mut field, target, (x + 60.0, 270.0), (x - 50.0, 270.0));
    let x = on_the_line(&mut field, &context, id);
    away_pass(&mut field, target, (x + 60.0, 270.0), (x - 50.0, 270.0));
    let decision = Decision::new(&field, &context);
    assert_eq!(
        decision.of(id, DefenderHoldingLineState::default()),
        Some(PlayerState::Defender(DefenderState::Intercepting)),
    );
}

#[test]
fn a_lost_race_with_nobody_on_the_ball_is_not_a_press() {
    let (mut field, context) = empty_pitch();
    let (def, mid) = (defender(&field), midfielder(&field));
    let target = away_man(&field, 9);
    stand(&mut field, def, 200.0, 270.0);
    stand(&mut field, mid, 210.0, 200.0);
    stand(&mut field, target, 340.0, 270.0);
    away_pass(&mut field, target, (400.0, 270.0), (340.0, 270.0));
    let decision = Decision::new(&field, &context);
    assert_ne!(
        decision.of(def, DefenderInterceptingState::default()),
        Some(PlayerState::Defender(DefenderState::Pressing)),
    );
    assert_ne!(
        decision.of(mid, MidfielderInterceptingState::default()),
        Some(PlayerState::Midfielder(MidfielderState::Pressing)),
    );
}

#[test]
fn a_lost_race_to_a_carried_ball_is_still_a_press() {
    let (mut field, context) = empty_pitch();
    let id = defender(&field);
    let carrier = away_man(&field, 9);
    stand(&mut field, id, 200.0, 270.0);
    stand(&mut field, carrier, 330.0, 270.0);
    field.ball.current_owner = Some(carrier);
    field.ball.position = Vector3::new(330.0, 270.0, 0.0);
    field.ball.velocity = Vector3::new(-0.2, 0.0, 0.0);
    let decision = Decision::new(&field, &context);
    assert_eq!(
        decision.of(id, DefenderInterceptingState::default()),
        Some(PlayerState::Defender(DefenderState::Pressing)),
    );
}

// ── An off-ball player does not pass through an on-ball state ───────────

#[test]
fn a_forward_who_has_created_space_stays_off_the_ball() {
    let (mut field, context) = empty_pitch();
    let id = forward(&field);
    let holder = outfielder(&field, HOME, |s| matches!(s, PlayerState::Midfielder(_)));
    stand(&mut field, id, 620.0, 270.0);
    stand(&mut field, holder, 540.0, 270.0);
    field.ball.current_owner = Some(holder);
    field.ball.position = Vector3::new(540.0, 270.0, 0.0);
    let decision = Decision::new(&field, &context);
    for _ in 0..200 {
        assert!(
            matches!(
                decision.of(id, ForwardCreatingSpaceState::default()),
                None | Some(PlayerState::Forward(
                    ForwardState::Running
                        | ForwardState::RunningInBehind
                        | ForwardState::HoldingWidth
                ))
            ),
            "he carries on with his off-ball movement"
        );
    }
}

#[test]
fn a_forward_who_receives_the_ball_decides_as_any_carrier_does() {
    let (mut field, context) = empty_pitch();
    let id = forward(&field);
    stand(&mut field, id, 620.0, 270.0);
    field.ball.current_owner = Some(id);
    field.ball.position = Vector3::new(620.0, 270.0, 0.0);
    let decision = Decision::new(&field, &context);
    assert_eq!(
        decision.of(id, ForwardCreatingSpaceState::default()),
        Some(PlayerState::Forward(ForwardState::Running)),
    );
}

// ── A run is started on a stricter picture than it is carried on ────────

#[test]
fn a_run_with_two_men_in_the_way_cannot_start_or_carry_on() {
    let (mut field, context) = empty_pitch();
    let id = forward(&field);
    stand(&mut field, id, 600.0, 270.0);
    let (first, second) = (away_man(&field, 3), away_man(&field, 4));
    stand(&mut field, first, 620.0, 270.0);
    stand(&mut field, second, 620.0, 280.0);
    let decision = Decision::new(&field, &context);
    let ctx = decision.ctx(id, 40);
    assert!(!RunInBehind::may_commit(&ctx));
    assert!(!RunInBehind::stays_viable(&ctx));
}

#[test]
fn a_clear_run_can_start_and_carry_on() {
    let (mut field, context) = empty_pitch();
    let id = forward(&field);
    stand(&mut field, id, 600.0, 270.0);
    let decision = Decision::new(&field, &context);
    let ctx = decision.ctx(id, 40);
    assert!(RunInBehind::may_commit(&ctx));
    assert!(RunInBehind::stays_viable(&ctx));
}

/// One man in the way: over a stretch of play a quick forward commits far less
/// often than he carries on. That gap is the hysteresis.
#[test]
fn one_blocker_lets_a_run_carry_on_more_readily_than_start() {
    let (mut field, context) = empty_pitch();
    let id = forward(&field);
    let blocker = away_man(&field, 3);
    stand(&mut field, id, 600.0, 270.0);
    stand(&mut field, blocker, 620.0, 270.0);
    let decision = Decision::new(&field, &context);
    let ctx = decision.ctx(id, 40);
    let draws = 4000;
    let commits = (0..draws).filter(|_| RunInBehind::may_commit(&ctx)).count();
    let carries = (0..draws)
        .filter(|_| RunInBehind::stays_viable(&ctx))
        .count();
    assert!(
        commits < carries,
        "{commits} commits against {carries} carries"
    );
    assert!(carries < draws, "the release is what timeboxes the run");
}

// ── The press door asks whether the press is his ───────────────────────

fn press_scene(field: &mut MatchField, team_mate_x: f32) -> (u32, u32) {
    let id = midfielder(field);
    let mate = defender(field);
    let carrier = away_man(field, 9);
    stand(field, id, 300.0, 270.0);
    stand(field, mate, team_mate_x, 270.0);
    stand(field, carrier, 390.0, 270.0);
    field.ball.current_owner = Some(carrier);
    field.ball.position = Vector3::new(390.0, 270.0, 0.0);
    (id, carrier)
}

#[test]
fn a_returning_midfielder_leaves_a_press_a_team_mate_is_clearly_on() {
    let (mut field, context) = empty_pitch();
    let (id, _) = press_scene(&mut field, 380.0);
    let decision = Decision::new(&field, &context);
    assert!(PressOwnership::belongs_to_a_team_mate(
        &decision.ctx(id, 40)
    ));
    assert_ne!(
        decision.of(id, MidfielderReturningState::default()),
        Some(PlayerState::Midfielder(MidfielderState::Pressing)),
    );
}

#[test]
fn a_returning_midfielder_takes_a_press_nobody_else_is_on() {
    let (mut field, context) = empty_pitch();
    let (id, _) = press_scene(&mut field, 200.0);
    let decision = Decision::new(&field, &context);
    assert!(!PressOwnership::belongs_to_a_team_mate(
        &decision.ctx(id, 40)
    ));
    assert_eq!(
        decision.of(id, MidfielderReturningState::default()),
        Some(PlayerState::Midfielder(MidfielderState::Pressing)),
    );
}

// ── The keeper's chase and its give-up read one territory ───────────────

fn loose_ball_at_keepers_ground(
    field: &mut MatchField,
    context: &MatchContext,
    depth: f32,
) -> (u32, Vector3<f32>) {
    let keeper = outfielder(field, HOME, |s| matches!(s, PlayerState::Goalkeeper(_)));
    let (goal, out) = match field.get_player(keeper).expect("a keeper").side {
        Some(PlayerSide::Right) => (context.goal_positions.right, -1.0),
        _ => (context.goal_positions.left, 1.0),
    };
    field.ball.position = goal + Vector3::new(depth * out, 0.0, 0.0);
    field.ball.velocity = Vector3::zeros();
    field.ball.cached_landing_position = field.ball.position;
    (keeper, goal)
}

#[test]
fn a_ball_inside_the_narrowest_ground_is_in_every_keepers_ground() {
    let (mut field, context) = empty_pitch();
    let (keeper, goal) = loose_ball_at_keepers_ground(&mut field, &context, 100.0);
    let tick = GameTickContext::new(&field, &context.players);
    let inner = KeeperSweepLimit::innermost();
    let outer = KeeperSweepLimit::outermost();
    assert!(!KeeperSweepLimit::ball_has_left(keeper, &tick, goal, inner));
    assert!(!KeeperSweepLimit::ball_has_left(keeper, &tick, goal, outer));
}

#[test]
fn a_ball_on_the_boundary_is_decided_the_same_way_by_both_ends() {
    let (mut field, context) = empty_pitch();
    let inner = KeeperSweepLimit::innermost();
    let (keeper, goal) = loose_ball_at_keepers_ground(&mut field, &context, inner.0 + 1.0);
    let tick = GameTickContext::new(&field, &context.players);
    // Just past the override's bound and still inside any keeper's own: it
    // will not force him, and his give-up would not throw him out.
    assert!(KeeperSweepLimit::ball_has_left(keeper, &tick, goal, inner));
    assert!(!KeeperSweepLimit::ball_has_left(
        keeper,
        &tick,
        goal,
        KeeperSweepLimit::outermost()
    ));
}

#[test]
fn a_pass_played_to_the_keeper_is_his_wherever_it_starts_from() {
    let (mut field, context) = empty_pitch();
    let (keeper, goal) = loose_ball_at_keepers_ground(&mut field, &context, 400.0);
    field.ball.pass_target_player_id = Some(keeper);
    let tick = GameTickContext::new(&field, &context.players);
    assert!(!KeeperSweepLimit::ball_has_left(
        keeper,
        &tick,
        goal,
        KeeperSweepLimit::innermost()
    ));
}

#[test]
fn the_possession_fact_this_file_leans_on_is_the_shared_one() {
    let (field, context) = empty_pitch();
    let tick = GameTickContext::new(&field, &context.players);
    assert_eq!(tick.possession, TeamPossession::read(&field));
}
