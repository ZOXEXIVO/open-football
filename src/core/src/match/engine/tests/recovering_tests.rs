//! **A player recovers after a committed challenge.**

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::common_states::{ChallengeKind, CommonRecoveringState};
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::ball::ball::PlayerReach;
use crate::r#match::engine::result::Score;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayer, MatchPlayerCollection,
    StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

fn pitch() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    (field, context)
}

fn defender(field: &MatchField) -> MatchPlayer {
    field
        .players
        .iter()
        .find(|p| p.team_id == 1 && p.tactical_position.current_position.is_defender())
        .expect("a home defender")
        .clone()
}

#[test]
fn a_missed_slide_costs_more_than_a_missed_block() {
    for (agility, balance) in [(1.0, 1.0), (10.0, 12.0), (20.0, 20.0)] {
        let block = ChallengeKind::Block.recovery_ticks(agility, balance);
        let slide = ChallengeKind::Slide.recovery_ticks(agility, balance);
        assert!(slide > block, "{slide} against {block}");
    }
}

#[test]
fn recovery_times_are_the_ones_a_body_takes() {
    // An AI tick is 20 ms: 0.3-0.5 s after a block, 0.8-1.2 s after a slide.
    assert_eq!(ChallengeKind::Block.recovery_ticks(1.0, 1.0), 25);
    assert_eq!(ChallengeKind::Block.recovery_ticks(20.0, 20.0), 15);
    assert_eq!(ChallengeKind::Slide.recovery_ticks(1.0, 1.0), 60);
    assert_eq!(ChallengeKind::Slide.recovery_ticks(20.0, 20.0), 40);
}

#[test]
fn an_agile_player_gets_up_sooner() {
    for kind in [ChallengeKind::Block, ChallengeKind::Slide] {
        assert!(kind.recovery_ticks(18.0, 18.0) < kind.recovery_ticks(6.0, 6.0));
    }
}

#[test]
fn a_miss_puts_him_in_recovery_for_its_own_time() {
    let (field, _) = pitch();
    let mut player = defender(&field);
    player.on_challenge_missed(ChallengeKind::Slide);
    assert_eq!(player.state, PlayerState::Recovering);
    assert_eq!(player.in_state_time, 0);
    assert_eq!(
        player.recovery_ticks,
        ChallengeKind::Slide.recovery_ticks(
            player.skills.physical.agility,
            player.skills.physical.balance
        )
    );
}

#[test]
fn while_recovering_he_cannot_play_a_ball_at_his_feet() {
    let (mut field, _) = pitch();
    let mut player = defender(&field);
    field.ball.position = player.position;
    assert!(PlayerReach::can_touch(&field.ball, &player));
    player.on_challenge_missed(ChallengeKind::Block);
    assert!(!PlayerReach::can_touch(&field.ball, &player));
}

#[test]
fn while_recovering_he_slows_and_stays_out_of_the_chase() {
    let (field, context) = pitch();
    let mut player = defender(&field);
    player.velocity = Vector3::new(0.5, 0.0, 0.0);
    player.on_challenge_missed(ChallengeKind::Slide);
    assert!(player.state.is_committed_action());

    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: 1,
        player: &player,
        context: &context,
        tick_context: &tick,
    };
    let velocity = CommonRecoveringState::default()
        .velocity(&ctx)
        .expect("a recovering man has a velocity");
    assert!(velocity.norm() < 0.5, "he decelerates");
    assert!(velocity.norm() > 0.0, "he does not stop dead");
    assert!(CommonRecoveringState::default().process(&ctx).is_none());
}

#[test]
fn he_goes_back_to_his_roles_play_when_it_is_over() {
    let (field, context) = pitch();
    let mut player = defender(&field);
    player.on_challenge_missed(ChallengeKind::Block);
    let tick = GameTickContext::new(&field, &context.players);
    let ctx = StateProcessingContext {
        in_state_time: player.recovery_ticks,
        player: &player,
        context: &context,
        tick_context: &tick,
    };
    assert_eq!(
        CommonRecoveringState::default()
            .process(&ctx)
            .and_then(|change| change.state),
        Some(PlayerState::Defender(DefenderState::Standing)),
    );
}

/// A won tackle hands a state with an event and no miss, so `on_challenge_missed`
/// is never called for it. Pinned at the source: only the `Missed` arm of each
/// `settle` may name a kind of miss.
#[test]
fn only_the_missed_arm_of_a_challenge_costs_recovery() {
    let sources = [
        include_str!("../player/strategies/defenders/states/tackling/mod.rs"),
        include_str!("../player/strategies/midfielders/states/tackling/mod.rs"),
        include_str!("../player/strategies/forwarders/states/tackling/mod.rs"),
    ];
    for source in sources {
        let start = source.find("fn settle(").expect("a settle");
        let body = &source[start..source[start..].find("\n    }\n").unwrap() + start];
        let missed = body.find("TackleOutcome::Missed").expect("a miss arm");
        let named = body.find("with_missed_challenge").expect("a miss named");
        assert!(named > missed, "a miss named outside the Missed arm");
        assert_eq!(body.matches("with_missed_challenge").count(), 1);
    }
}
