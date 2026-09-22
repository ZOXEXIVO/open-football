//! Defenders must react to the carrier while maintaining coordinated marks.

use super::goal_celebration_tests::squad;
use crate::r#match::common_states::TackleEngagement;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::DefensiveRecovery;
use crate::r#match::defenders::states::covering::DefenderCoveringState;
use crate::r#match::defenders::states::marking::DefenderMarkingState;
use crate::r#match::defenders::states::pressing::DefenderPressingState;
use crate::r#match::engine::result::Score;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{
    DefenceRefreshInputs, DefensiveDuty, DefensivePlan, GameTickContext, MatchContext, MatchField,
    MatchPlayerCollection, PlayerSide, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

fn setup(side: PlayerSide) -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(840, 545, home, away);
    for p in field.players.iter_mut() {
        p.position = Vector3::new(600.0, 500.0, 0.0);
        p.velocity = Vector3::zeros();
        p.side = Some(if p.team_id == 1 {
            side
        } else {
            side.opposite()
        });
    }
    for (id, x, y) in [
        (101, 210.0, 272.0),
        (102, 180.0, 292.0),
        (103, 170.0, 242.0),
        (104, 150.0, 302.0),
        (209, 220.0, 272.0),
        (210, 210.0, 230.0),
    ] {
        field.get_player_mut(id).unwrap().position = Vector3::new(x, y, 0.0);
    }
    if side == PlayerSide::Right {
        for p in field.players.iter_mut() {
            p.position.x = 840.0 - p.position.x;
        }
    }
    field.ball.position = field.get_player(209).unwrap().position;
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(209);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = 600_000;
    DefensivePlan::refresh(
        &mut context.defence_home,
        &mut context.defence_away,
        &DefenceRefreshInputs {
            field: &field,
            home_team_id: 1,
            away_team_id: 2,
            home_keeper_voice: 0.5,
            away_keeper_voice: 0.5,
        },
    );
    assert_eq!(context.defence_home.presser(), Some(101));
    assert_eq!(context.defence_home.duty_of(102), DefensiveDuty::Cover);
    assert_eq!(context.defence_home.mark_of(103), Some(210));
    (field, context)
}

fn ctx<'a>(
    field: &'a MatchField,
    context: &'a MatchContext,
    tick: &'a GameTickContext,
    id: u32,
) -> StateProcessingContext<'a> {
    StateProcessingContext {
        in_state_time: 10,
        player: field.players.iter().find(|p| p.id == id).unwrap(),
        context,
        tick_context: tick,
    }
}

#[test]
fn defender_marking_hands_new_press_and_cover_duties_to_the_right_states() {
    for side in [PlayerSide::Left, PlayerSide::Right] {
        let (mut field, context) = setup(side);
        field.get_player_mut(101).unwrap().position.x -= 90.0 * side.forward_dir_x();
        let tick = GameTickContext::new(&field, &context.players);
        for (id, expected) in [
            (101, DefenderState::Pressing),
            (102, DefenderState::Covering),
        ] {
            let result = DefenderMarkingState::default().process(&ctx(&field, &context, &tick, id));
            assert_eq!(
                result.and_then(|r| r.state),
                Some(PlayerState::Defender(expected))
            );
        }
    }
}

#[test]
fn defender_marking_challenges_a_carrier_running_past_his_off_ball_mark() {
    for side in [PlayerSide::Left, PlayerSide::Right] {
        let (mut field, context) = setup(side);
        field.get_player_mut(101).unwrap().position.x -= 60.0 * side.forward_dir_x();
        let carrier = field.ball.position;
        field.get_player_mut(103).unwrap().position =
            carrier - Vector3::new(8.0 * side.forward_dir_x(), 0.0, 0.0);
        let tick = GameTickContext::new(&field, &context.players);
        let situation = ctx(&field, &context, &tick, 103);
        assert!(TackleEngagement::should_commit(&situation, 8.0));
        assert_eq!(
            DefenderMarkingState::default()
                .process(&situation)
                .and_then(|r| r.state),
            Some(PlayerState::Defender(DefenderState::Tackling))
        );
        assert!(DefensiveRecovery::depth_override(&situation).is_none());

        // Permission survives the entry band, so the next tick doesn't
        // abandon an ongoing challenge before its normal release distance.
        field.get_player_mut(103).unwrap().position.x -= 10.0 * side.forward_dir_x();
        let tick = GameTickContext::new(&field, &context.players);
        assert!(TackleEngagement::may_engage_carrier(&ctx(
            &field, &context, &tick, 103
        )));
    }
}

#[test]
fn defender_local_challenge_respects_the_active_presser_and_tackle_cooldown() {
    let (mut field, context) = setup(PlayerSide::Left);
    field.get_player_mut(103).unwrap().position = field.ball.position - Vector3::new(8.0, 0.0, 0.0);
    let tick = GameTickContext::new(&field, &context.players);
    assert!(!TackleEngagement::should_commit(
        &ctx(&field, &context, &tick, 103),
        8.0
    ));

    field.get_player_mut(101).unwrap().position.x -= 60.0;
    field.get_player_mut(103).unwrap().tackle_cooldown = 100;
    let tick = GameTickContext::new(&field, &context.players);
    assert!(!TackleEngagement::should_commit(
        &ctx(&field, &context, &tick, 103),
        8.0
    ));
    assert_eq!(
        DefenderMarkingState::default()
            .process(&ctx(&field, &context, &tick, 103))
            .and_then(|r| r.state),
        Some(PlayerState::Defender(DefenderState::Pressing)),
        "a recovering tackler should contain the carrier while his cooldown expires"
    );
}

#[test]
fn defender_assigned_cover_does_not_bounce_back_into_marking() {
    let (mut field, context) = setup(PlayerSide::Left);
    // Make the cover third closest without refreshing the team plan.
    field.get_player_mut(103).unwrap().position = Vector3::new(205.0, 280.0, 0.0);
    let tick = GameTickContext::new(&field, &context.players);
    let mut situation = ctx(&field, &context, &tick, 102);
    situation.in_state_time = 50;
    assert!(
        DefenderCoveringState::default()
            .process(&situation)
            .is_none()
    );
}

#[test]
fn defender_local_challenge_elects_only_one_of_two_equally_close_markers() {
    let (mut field, context) = setup(PlayerSide::Left);
    field.get_player_mut(101).unwrap().position.x -= 60.0;
    for id in [103, 104] {
        field.get_player_mut(id).unwrap().position =
            field.ball.position - Vector3::new(8.0, 0.0, 0.0);
    }
    let tick = GameTickContext::new(&field, &context.players);
    assert!(TackleEngagement::should_commit(
        &ctx(&field, &context, &tick, 103),
        8.0
    ));
    assert!(!TackleEngagement::should_commit(
        &ctx(&field, &context, &tick, 104),
        8.0
    ));
}

#[test]
fn defender_marking_a_stationary_receiver_on_the_ball_has_finite_velocity() {
    let (mut field, context) = setup(PlayerSide::Left);
    field.ball.position = field.get_player(210).unwrap().position;
    field.ball.current_owner = Some(210);
    let tick = GameTickContext::new(&field, &context.players);
    let velocity = DefenderMarkingState::default()
        .velocity(&ctx(&field, &context, &tick, 103))
        .unwrap();
    assert!(velocity.iter().all(|v| v.is_finite()), "{velocity:?}");
}

#[test]
fn defender_with_better_field_reading_leads_a_crossing_run_further() {
    let (mut field, context) = setup(PlayerSide::Left);
    field.get_player_mut(101).unwrap().position = Vector3::new(120.0, 272.0, 0.0);
    field.get_player_mut(209).unwrap().velocity = Vector3::new(0.0, 0.3, 0.0);
    for attribute in [
        "vision",
        "anticipation",
        "positioning",
        "concentration",
        "decisions",
    ] {
        let mut angles = Vec::new();
        for skill in [5.0, 18.0] {
            let mental = &mut field.get_player_mut(101).unwrap().skills.mental;
            mental.vision = 12.0;
            mental.anticipation = 12.0;
            mental.positioning = 12.0;
            mental.concentration = 12.0;
            mental.decisions = 12.0;
            match attribute {
                "vision" => mental.vision = skill,
                "anticipation" => mental.anticipation = skill,
                "positioning" => mental.positioning = skill,
                "concentration" => mental.concentration = skill,
                "decisions" => mental.decisions = skill,
                _ => unreachable!(),
            }
            let tick = GameTickContext::new(&field, &context.players);
            let velocity = DefenderPressingState::default()
                .velocity(&ctx(&field, &context, &tick, 101))
                .unwrap();
            assert!(velocity.x > 0.0);
            angles.push(velocity.y.atan2(velocity.x));
        }
        assert!(
            angles[1] > angles[0],
            "better {attribute} should help cut across the run: {angles:?}"
        );
    }
}
