//! **A decision resolves within its tick.** The chain walked against scripted
//! decisions, so each rule can be pinned on its own, and once against the real
//! states to show a trial leaves the player as it found him.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::PlayerFieldPositionGroup;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::result::Score;
use crate::r#match::events::{Event, EventCollection};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::state::{DecisionResolver, PlayerState};
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayer, MatchPlayerCollection,
    StateProcessingResult,
};
use nalgebra::Vector3;

const HOLDING: PlayerState = PlayerState::Defender(DefenderState::HoldingLine);
const INTERCEPTING: PlayerState = PlayerState::Defender(DefenderState::Intercepting);
const PRESSING: PlayerState = PlayerState::Defender(DefenderState::Pressing);
const MARKING: PlayerState = PlayerState::Defender(DefenderState::Marking);
const COVERING: PlayerState = PlayerState::Defender(DefenderState::Covering);
const TRACKING: PlayerState = PlayerState::Defender(DefenderState::TrackingBack);
const HEADING: PlayerState = PlayerState::Defender(DefenderState::Heading);

fn pitch() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    (field, context)
}

fn defender_holding_the_line() -> MatchPlayer {
    let (field, _) = pitch();
    let mut player = field
        .players
        .iter()
        .find(|p| p.team_id == 1 && p.tactical_position.current_position.is_defender())
        .expect("a home defender")
        .clone();
    player.redirect_to_fresh(HOLDING, TransitionSource::Reset);
    player.in_state_time = 57;
    player
}

fn stays() -> StateProcessingResult {
    StateProcessingResult::new()
}

fn hands_on(to: PlayerState) -> StateProcessingResult {
    StateProcessingResult {
        state: Some(to),
        ..StateProcessingResult::new()
    }
}

fn foul() -> EventCollection {
    EventCollection::with_event(Event::PlayerEvent(PlayerEvent::RequestBallReceive(7)))
}

/// Each state's answer, as the resolver asks it.
fn script(
    answers: Vec<(PlayerState, StateProcessingResult)>,
) -> impl FnMut(&mut MatchPlayer) -> StateProcessingResult {
    let mut answers = answers;
    move |player| {
        let at = answers
            .iter()
            .position(|(state, _)| state.compact_id() == player.state.compact_id())
            .expect("the chain only asks states in the script");
        answers.remove(at).1
    }
}

#[test]
fn a_destination_that_keeps_him_is_where_he_goes() {
    let mut player = defender_holding_the_line();
    let decision = DecisionResolver::resolve(
        &mut player,
        INTERCEPTING,
        script(vec![(INTERCEPTING, stays())]),
    );
    assert_eq!(decision.landing, Some(INTERCEPTING));
    assert!(decision.performer.is_none());
}

#[test]
fn a_chain_back_to_where_he_was_leaves_him_there_with_his_timer_running() {
    let mut player = defender_holding_the_line();
    let decision = DecisionResolver::resolve(
        &mut player,
        INTERCEPTING,
        script(vec![
            (INTERCEPTING, hands_on(PRESSING)),
            (PRESSING, hands_on(HOLDING)),
        ]),
    );
    assert_eq!(decision.landing, None);
    assert_eq!(player.state, HOLDING);
    assert_eq!(
        player.in_state_time, 57,
        "a refusal does not reset his timer"
    );
}

#[test]
fn a_hop_state_is_skipped() {
    let mut player = defender_holding_the_line();
    let decision = DecisionResolver::resolve(
        &mut player,
        INTERCEPTING,
        script(vec![
            (INTERCEPTING, hands_on(PRESSING)),
            (PRESSING, stays()),
        ]),
    );
    assert_eq!(decision.landing, Some(PRESSING));
    assert!(
        decision.performer.is_none(),
        "the hop did nothing to record"
    );
}

#[test]
fn a_hop_that_acts_fires_it_and_is_recorded_as_visited() {
    let mut player = defender_holding_the_line();
    let mut acting = hands_on(COVERING);
    acting.events = foul();
    let decision = DecisionResolver::resolve(
        &mut player,
        INTERCEPTING,
        script(vec![(INTERCEPTING, hands_on(PRESSING)), (PRESSING, acting)]),
    );
    assert_eq!(decision.landing, Some(COVERING));
    assert_eq!(decision.performer, Some(PRESSING));
    assert!(decision.events.is_some_and(|events| events.has_events()));
}

#[test]
fn an_action_in_the_state_that_keeps_him_is_not_lost() {
    let mut player = defender_holding_the_line();
    let mut acting = hands_on(INTERCEPTING);
    acting.events = foul();
    let decision = DecisionResolver::resolve(
        &mut player,
        INTERCEPTING,
        script(vec![(INTERCEPTING, acting)]),
    );
    assert_eq!(decision.landing, Some(INTERCEPTING));
    assert!(decision.events.is_some_and(|events| events.has_events()));
}

#[test]
fn a_committed_action_ends_the_chain_unasked() {
    let mut player = defender_holding_the_line();
    let decision = DecisionResolver::resolve(
        &mut player,
        INTERCEPTING,
        script(vec![(INTERCEPTING, hands_on(HEADING))]),
    );
    assert_eq!(decision.landing, Some(HEADING));
}

#[test]
fn the_chain_is_limited_to_three_hops() {
    let mut player = defender_holding_the_line();
    let decision = DecisionResolver::resolve(
        &mut player,
        INTERCEPTING,
        script(vec![
            (INTERCEPTING, hands_on(PRESSING)),
            (PRESSING, hands_on(MARKING)),
            (MARKING, hands_on(COVERING)),
            (COVERING, hands_on(TRACKING)),
        ]),
    );
    assert_eq!(decision.landing, Some(COVERING));
}

#[test]
fn a_trial_cooldown_takes_effect() {
    let mut player = defender_holding_the_line();
    assert_eq!(player.tackle_cooldown, 0);
    let mut went_in = stays();
    went_in.start_tackle_cooldown = true;
    DecisionResolver::resolve(
        &mut player,
        INTERCEPTING,
        script(vec![(INTERCEPTING, went_in)]),
    );
    assert!(player.tackle_cooldown > 0);
}

#[test]
fn no_chain_starts_from_an_action_or_a_committed_state() {
    let player = defender_holding_the_line();
    let plain = stays();
    assert!(DecisionResolver::applies(&player, INTERCEPTING, &plain));

    let mut with_event = stays();
    with_event.events = foul();
    assert!(!DecisionResolver::applies(
        &player,
        INTERCEPTING,
        &with_event
    ));

    let mut with_reason = stays();
    with_reason.shot_reason = Some("TEST");
    assert!(!DecisionResolver::applies(
        &player,
        INTERCEPTING,
        &with_reason
    ));

    assert!(!DecisionResolver::applies(&player, HEADING, &plain));

    let mut committed = defender_holding_the_line();
    committed.redirect_to_fresh(HEADING, TransitionSource::Reset);
    assert!(!DecisionResolver::applies(&committed, INTERCEPTING, &plain));
}

/// Every state a defender can be tried in, against the real handlers.
#[test]
fn a_trial_leaves_the_player_exactly_as_it_found_him() {
    let (mut field, context) = pitch();
    let id = field
        .players
        .iter()
        .find(|p| p.team_id == 1 && p.tactical_position.current_position.is_defender())
        .expect("a home defender")
        .id;
    field.get_player_mut(id).expect("him").velocity = Vector3::new(0.1, 0.2, 0.0);
    let tick = GameTickContext::new(&field, &context.players);
    let mut player = field.get_player(id).expect("him").clone();
    player.player_attributes.condition = 7000;
    player.fatigue_accumulator = 0.25;
    player.in_state_time = 57;
    let before = (
        player.position,
        player.velocity,
        player.player_attributes.condition,
        player.fatigue_accumulator,
        player.state,
        player.previous_state,
        player.in_state_time,
    );
    for state in DefenderState::ALL {
        player.trying_state(PlayerState::Defender(state), |p| {
            PlayerFieldPositionGroup::decide(p, &context, &tick)
        });
        assert_eq!(
            before,
            (
                player.position,
                player.velocity,
                player.player_attributes.condition,
                player.fatigue_accumulator,
                player.state,
                player.previous_state,
                player.in_state_time,
            ),
            "trying {state:?}"
        );
    }
}
