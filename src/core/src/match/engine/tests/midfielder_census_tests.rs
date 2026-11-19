//! **Every midfielder state is counted under its own name.** The census
//! tables are sized from `MidfielderState::SLOTS`, and the cross-role
//! diagnostics group midfielder states by `MidfielderState::family`, so a
//! state added later is neither dropped nor lumped in with "other".

#![cfg(all(test, feature = "match-logs"))]

use super::goal_celebration_tests::squad;
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::ball::ball::diagnostics::lane_diag::LaneDiag;
use crate::r#match::engine::teamplay::plans::attack::AttackPlan;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::diagnostics::defender_census::{HeaderBooking, HeaderRule};
use crate::r#match::player::diagnostics::midfielder_census::{MidHeaderRule, MidfielderCensus};
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{MatchField, MatchPlayer, PenaltyArea};
use nalgebra::Vector3;

fn midfielder_of(field: &mut MatchField) -> &mut MatchPlayer {
    field
        .players
        .iter_mut()
        .find(|p| {
            p.tactical_position.current_position.position_group()
                == PlayerFieldPositionGroup::Midfielder
        })
        .expect("a 4-4-2 has midfielders")
}

#[test]
fn every_midfielder_state_has_its_own_slot_and_name_in_every_table() {
    let mut field = MatchField::new(840, 545, squad(1, 100), squad(2, 200));
    let mut census = MidfielderCensus::default();
    let attacks = [(1, AttackPlan::idle()), (2, AttackPlan::idle())];
    let areas = [
        PenaltyArea::new(Vector3::zeros(), Vector3::new(1.0, 1.0, 0.0)),
        PenaltyArea::new(Vector3::zeros(), Vector3::new(1.0, 1.0, 0.0)),
    ];
    let other = LaneDiag::STATES.len() - 1;
    for (tick, state) in MidfielderState::ALL.into_iter().enumerate() {
        let slot = state as usize;
        assert!(slot < MidfielderState::SLOTS, "{state} has no slot");
        assert_eq!(MidfielderState::from_slot(slot), Some(state));
        assert_eq!(MidfielderCensus::slot_name(slot), state.to_string());
        assert_ne!(
            LaneDiag::state_slot(&PlayerState::Midfielder(state)),
            other,
            "{state} falls to \"other\" in the lane census"
        );

        let midfielder = midfielder_of(&mut field);
        midfielder.transition_to(
            PlayerState::Midfielder(state),
            TransitionSource::EventHandler,
        );
        MidfielderCensus::note_header(
            HeaderRule::AerialDuel,
            midfielder,
            false,
            HeaderBooking::Nothing,
        );
        census.sample(&field, attacks, areas, tick as u64 + 1);
        let snapshot = MidfielderCensus::snapshot();
        assert!(
            snapshot.header_state[MidHeaderRule::AerialDuel as usize][slot] > 0,
            "{state} was not counted in the aerial census"
        );
        assert!(
            snapshot.state_ticks[slot] > 0,
            "{state} was not counted in the motion census"
        );
    }
}
