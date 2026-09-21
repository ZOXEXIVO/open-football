//! Keeper distribution must move the surrounding players, including idle
//! teammates, without making a live ball at his feet unchallengeable.

use super::goal_celebration_tests::squad;
use crate::r#match::common_states::KeeperReleaseSpace;
use crate::r#match::engine::result::Score;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, PassOriginRestart,
    PlayerSide, StateProcessingContext, StateProcessingHandler, StateProcessor,
};
use nalgebra::Vector3;

fn setup(side: PlayerSide) -> (MatchField, MatchContext, u32, u32, u32, u32) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = 600_000;
    let keeper = if side == PlayerSide::Left { 100 } else { 200 };
    let teammate = keeper + 1;
    let presser = if side == PlayerSide::Left { 209 } else { 109 };
    let surplus = presser + 1;
    let at = Vector3::new(
        if side == PlayerSide::Left {
            50.0
        } else {
            790.0
        },
        272.0,
        0.0,
    );
    for player in field.players.iter_mut() {
        player.position = Vector3::new(420.0, 40.0 + (player.id % 11) as f32 * 40.0, 0.0);
        player.velocity = Vector3::zeros();
        if player.id == keeper || player.id == teammate {
            player.position = at;
        } else if player.id == presser {
            player.position = at + Vector3::new(side.forward_dir_x() * 4.0, 0.0, 0.0);
        } else if player.id == surplus {
            player.position = at + Vector3::new(side.forward_dir_x() * 16.0, 0.0, 0.0);
        }
    }
    field.ball.position = at;
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(keeper);
    assert_eq!(field.get_player(keeper).unwrap().side, Some(side));
    crate::r#match::DefensivePlan::refresh(
        &mut context.defence_home,
        &mut context.defence_away,
        &crate::r#match::DefenceRefreshInputs {
            field: &field,
            home_team_id: 1,
            away_team_id: 2,
            home_keeper_voice: 0.5,
            away_keeper_voice: 0.5,
        },
    );
    (field, context, keeper, teammate, presser, surplus)
}

fn movement(
    field: &MatchField,
    context: &MatchContext,
    tick: &GameTickContext,
    id: u32,
) -> Option<(Vector3<f32>, f32)> {
    KeeperReleaseSpace::retreat(&StateProcessingContext {
        in_state_time: 0,
        player: field.players.iter().find(|p| p.id == id).unwrap(),
        context,
        tick_context: tick,
    })
}

#[test]
fn keeper_space_teammates_open_out_on_both_sides_with_feet_or_hands() {
    for side in [PlayerSide::Left, PlayerSide::Right] {
        for hands in [false, true] {
            let (field, context, keeper, teammate, _, _) = setup(side);
            let mut tick = GameTickContext::new(&field, &context.players);
            tick.ball.held_in_hands = hands;
            let (velocity, effort) = movement(&field, &context, &tick, teammate)
                .expect("a teammate standing on the keeper must open out");
            assert!(velocity.x * side.forward_dir_x() > 0.0);
            assert!(velocity.y.abs() > 0.0, "offer an angled outlet");
            assert_eq!(velocity.z, 0.0);
            assert!(effort >= 0.4, "idle states must actually jog away");
            assert!(movement(&field, &context, &tick, keeper).is_none());
        }
    }
}

#[test]
fn keeper_space_keeps_the_live_presser_and_moves_surplus_opponents() {
    for side in [PlayerSide::Left, PlayerSide::Right] {
        let (field, context, _, _, presser, surplus) = setup(side);
        let tick = GameTickContext::new(&field, &context.players);
        assert!(
            movement(&field, &context, &tick, presser).is_none(),
            "the elected challenger must still be able to press a back-pass"
        );
        let (velocity, _) = movement(&field, &context, &tick, surplus)
            .expect("an opponent who cannot challenge must leave the crowd");
        assert!(velocity.x * side.forward_dir_x() > 0.0);
    }
}

#[test]
fn keeper_space_hands_and_pending_goal_kicks_also_move_the_presser() {
    for side in [PlayerSide::Left, PlayerSide::Right] {
        let (field, context, keeper, teammate, presser, _) = setup(side);
        let mut tick = GameTickContext::new(&field, &context.players);
        tick.ball.held_in_hands = true;
        assert!(movement(&field, &context, &tick, presser).is_some());
        tick.ball.held_in_hands = false;
        tick.ball.current_owner = None;
        tick.ball.is_owned = false;
        tick.ball.pass_origin_restart = PassOriginRestart::GoalKick;
        tick.ball.restart_taker = Some(keeper);
        assert!(movement(&field, &context, &tick, presser).is_some());
        assert!(movement(&field, &context, &tick, teammate).is_some());
    }
}

#[test]
fn keeper_space_overlapping_opponent_leaves_upfield_on_the_ground() {
    for side in [PlayerSide::Left, PlayerSide::Right] {
        let (mut field, context, _, _, presser, _) = setup(side);
        field.get_player_mut(presser).unwrap().position = field.ball.position;
        // The held ball's metric height must not become a vertical
        // movement request, or hide the zero-distance overlap fallback.
        field.ball.position.z = 1.15;
        let mut tick = GameTickContext::new(&field, &context.players);
        tick.ball.held_in_hands = true;
        let (velocity, _) = movement(&field, &context, &tick, presser).unwrap();
        assert!(velocity.x * side.forward_dir_x() > 0.0);
        assert_eq!(velocity.z, 0.0);
    }
}

#[test]
fn keeper_space_does_not_clear_the_box_for_a_keeper_playing_upfield() {
    let (mut field, context, keeper, _, _, surplus) = setup(PlayerSide::Left);
    field.get_player_mut(keeper).unwrap().position.x = 400.0;
    field.ball.position.x = 400.0;
    let tick = GameTickContext::new(&field, &context.players);
    assert!(movement(&field, &context, &tick, surplus).is_none());
}

#[test]
fn keeper_space_leaves_loose_balls_outfield_owners_and_distant_outlets_alone() {
    let (mut field, context, keeper, teammate, presser, surplus) = setup(PlayerSide::Left);
    let mut tick = GameTickContext::new(&field, &context.players);
    tick.ball.current_owner = None;
    assert!(movement(&field, &context, &tick, teammate).is_none());
    tick.ball.current_owner = Some(teammate);
    for hands in [false, true] {
        tick.ball.held_in_hands = hands;
        assert!(movement(&field, &context, &tick, surplus).is_none());
    }
    // A stale restart origin after a kick must not protect a live ball.
    tick.ball.current_owner = Some(keeper);
    tick.ball.held_in_hands = false;
    tick.ball.pass_origin_restart = PassOriginRestart::GoalKick;
    assert!(movement(&field, &context, &tick, presser).is_none());
    field
        .players
        .iter_mut()
        .find(|p| p.id == teammate)
        .unwrap()
        .position
        .x = 180.0;
    let tick = GameTickContext::new(&field, &context.players);
    assert!(movement(&field, &context, &tick, teammate).is_none());
}

#[test]
fn keeper_space_reaches_idle_states_but_does_not_move_injured_players() {
    struct Idle;
    impl StateProcessingHandler for Idle {}

    let (mut field, context, _, teammate, _, _) = setup(PlayerSide::Left);
    let tick = GameTickContext::new(&field, &context.players);
    let player = field.players.iter_mut().find(|p| p.id == teammate).unwrap();
    let result = StateProcessor::new(0, player, &context, &tick).process_inner(Idle);
    assert!(result.velocity.is_some_and(|v| v.norm() > 0.0));
    assert!(result.effort_floor >= 0.4);
    player.state = PlayerState::Injured;
    let result = StateProcessor::new(0, player, &context, &tick).process_inner(Idle);
    assert!(result.velocity.is_none());
}

#[test]
fn keeper_space_teammate_physically_leaves_the_keeper() {
    for side in [PlayerSide::Left, PlayerSide::Right] {
        let (mut field, context, keeper, teammate, _, _) = setup(side);
        let at = field.get_player(keeper).unwrap().position;
        let mut events = crate::r#match::events::EventCollection::with_capacity(8);
        // Keep possession fixed to isolate movement through the real
        // state, effort cap and acceleration integration for two seconds.
        for _ in 0..100 {
            let tick = GameTickContext::new(&field, &context.players);
            let index = field.players.iter().position(|p| p.id == teammate).unwrap();
            let player = field.players.iter_mut().find(|p| p.id == teammate).unwrap();
            player.update(index, &context, &tick, &mut events);
            player.move_to(); // intervening light tick
            events.clear();
        }
        let gap = field.get_player(teammate).unwrap().position - at;
        assert!(
            gap.norm() > 32.0,
            "teammate still crowds the keeper: {gap:?}"
        );
        assert!(
            gap.x * side.forward_dir_x() > 0.0,
            "side {side:?}, gap {gap:?}"
        );
    }
}
