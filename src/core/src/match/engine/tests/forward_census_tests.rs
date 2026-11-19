//! **Every forward state is counted under its own name.** The census tables
//! are sized from `ForwardState::SLOTS`, and the cross-role diagnostics group
//! forward states by `ForwardState::family`, so a state added later is
//! neither dropped nor lumped in with "other".

#![cfg(all(test, feature = "match-logs"))]

use super::goal_celebration_tests::squad;
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::ball::ball::diagnostics::lane_diag::LaneDiag;
use crate::r#match::engine::teamplay::plans::attack::AttackPlan;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::diagnostics::defender_census::{HeaderBooking, HeaderRule};
use crate::r#match::player::diagnostics::forward_census::{ForwardCensus, FwdHeaderRule};
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{MatchField, MatchPlayer};

fn forward_of(field: &mut MatchField) -> &mut MatchPlayer {
    field
        .players
        .iter_mut()
        .find(|p| {
            p.tactical_position.current_position.position_group()
                == PlayerFieldPositionGroup::Forward
        })
        .expect("a 4-4-2 has forwards")
}

#[test]
fn every_forward_state_has_its_own_slot_and_name_in_every_table() {
    let mut field = MatchField::new(840, 545, squad(1, 100), squad(2, 200));
    let mut census = ForwardCensus::default();
    let attacks = [(1, AttackPlan::idle()), (2, AttackPlan::idle())];
    let other = LaneDiag::STATES.len() - 1;
    for (tick, state) in ForwardState::ALL.into_iter().enumerate() {
        let slot = state as usize;
        assert!(slot < ForwardState::SLOTS, "{state} has no slot");
        assert_eq!(ForwardState::from_slot(slot), Some(state));
        assert_eq!(ForwardCensus::slot_name(slot), state.to_string());
        assert_ne!(
            LaneDiag::state_slot(&PlayerState::Forward(state)),
            other,
            "{state} falls to \"other\" in the lane census"
        );

        let forward = forward_of(&mut field);
        forward.transition_to(PlayerState::Forward(state), TransitionSource::EventHandler);
        ForwardCensus::note_header(
            HeaderRule::AerialDuel,
            forward,
            false,
            HeaderBooking::Nothing,
        );
        census.sample(&field, attacks, tick as u64 + 1);
        let snapshot = ForwardCensus::snapshot();
        assert!(
            snapshot.header_state[FwdHeaderRule::AerialDuel as usize][slot] > 0,
            "{state} was not counted in the aerial census"
        );
        assert!(
            snapshot.state_ticks[slot] > 0,
            "{state} was not counted in the motion census"
        );
    }
}
