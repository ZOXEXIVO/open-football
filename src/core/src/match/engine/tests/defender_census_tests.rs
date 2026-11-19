//! **Every defender state is counted under its own name.** The defender
//! tables used to be sized by literals: `DEF_STATE_AT_STRIKE` held 21 slots
//! and silently dropped `Overlapping`, and `ABSURD_BY_STATE` held 96 slots
//! for ids from 100 up. They are sized from the state sets now, and these
//! note every state into each of them.

#![cfg(all(test, feature = "match-logs"))]

use super::goal_celebration_tests::squad;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::ball::ball::diagnostics::block_diag::BlockDiag;
use crate::r#match::engine::ball::ball::diagnostics::flight_diag::FlightDiag;
use crate::r#match::engine::teamplay::plans::defence::DefensivePlan;
use crate::r#match::player::diagnostics::defender_census::{
    DefenderCensus, HeaderBooking, HeaderRule,
};
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{MatchField, PenaltyArea};
use nalgebra::Vector3;

fn defender_of(field: &mut MatchField) -> &mut crate::r#match::MatchPlayer {
    field
        .players
        .iter_mut()
        .find(|p| {
            p.tactical_position.current_position.position_group()
                == crate::PlayerFieldPositionGroup::Defender
        })
        .expect("a 4-4-2 has defenders")
}

#[test]
fn every_defender_state_has_its_own_slot_and_name_in_every_table() {
    assert!(DefenderState::ALL.contains(&DefenderState::Overlapping));
    let mut field = MatchField::new(840, 545, squad(1, 100), squad(2, 200));
    let mut census = DefenderCensus::default();
    let plans = [(1, DefensivePlan::idle()), (2, DefensivePlan::idle())];
    let areas = [
        PenaltyArea::new(Vector3::zeros(), Vector3::new(1.0, 1.0, 0.0)),
        PenaltyArea::new(Vector3::zeros(), Vector3::new(1.0, 1.0, 0.0)),
    ];
    for (tick, state) in DefenderState::ALL.into_iter().enumerate() {
        let slot = state as usize;
        assert!(slot < DefenderState::SLOTS, "{state} has no slot");
        assert_eq!(DefenderState::from_slot(slot), Some(state));
        assert_eq!(DefenderCensus::slot_name(slot), state.to_string());

        BlockDiag::note_defender_state(slot);
        assert!(
            BlockDiag::defender_state_snapshot()[slot] > 0,
            "{state} was not counted at the strike"
        );

        let id = PlayerState::Defender(state).compact_id() as usize;
        assert!(id < PlayerState::COMPACT_SLOTS);
        FlightDiag::note_launch(10.0, 0.0, Some(id));
        assert!(
            FlightDiag::absurd_by_state()[id] > 0,
            "{state} was not counted among absurd launches"
        );

        let defender = defender_of(&mut field);
        defender.transition_to(PlayerState::Defender(state), TransitionSource::EventHandler);
        DefenderCensus::note_header(
            HeaderRule::AerialDuel,
            defender,
            false,
            HeaderBooking::Nothing,
        );
        census.sample(&field, plans, areas, tick as u64 + 1);
        let snapshot = DefenderCensus::snapshot();
        assert!(
            snapshot.header_state[HeaderRule::AerialDuel as usize][slot] > 0,
            "{state} was not counted in the aerial census"
        );
        assert!(
            snapshot.state_ticks[slot] > 0,
            "{state} was not counted in the motion census"
        );
    }
}
