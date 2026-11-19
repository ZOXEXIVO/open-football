//! **Every keeper state is counted under its own name.** The keeper tables
//! are indexed by the state's discriminant, and the first sizing of them
//! was a literal: a state added past it panicked one census and was
//! silently dropped by another. They are sized from `GoalkeeperState` now,
//! and these note every state into each of them.

#![cfg(all(test, feature = "match-logs"))]

use super::goal_celebration_tests::squad;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::diagnostics::keeper_census::KeeperCensus;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::{MatchField, PlayerSide};
use crate::mid_run_diag::{KeeperGatherDiag, KeeperPairDiag};

#[test]
fn every_keeper_state_has_its_own_slot_and_name_in_every_table() {
    for state in GoalkeeperState::ALL {
        let slot = KeeperCensus::slot(PlayerState::Goalkeeper(state));
        assert!(
            slot < KeeperCensus::SLOTS - 1,
            "{state} has no slot of its own"
        );
        assert_eq!(KeeperCensus::slot_name(slot), state.to_string());
        assert_eq!(GoalkeeperState::from_slot(state as usize), Some(state));
        assert_eq!(
            PlayerState::Goalkeeper(state).compact_id(),
            100 + state as u16,
            "the pair census names its rows by compact id"
        );

        KeeperGatherDiag::note_state(state, true);
        let gathered = KeeperGatherDiag::state_snapshot();
        assert!(gathered[slot * 2] > 0 && gathered[slot * 2 + 1] > 0);

        for to in GoalkeeperState::ALL {
            KeeperPairDiag::note(state as usize, to as usize, true);
        }
    }
    let pairs = KeeperPairDiag::snapshot();
    for from in GoalkeeperState::ALL {
        for to in GoalkeeperState::ALL {
            let slot = from as usize * KeeperPairDiag::STATES + to as usize;
            assert!(pairs[slot] > 0, "{from} -> {to} was not counted");
            assert!(pairs[KeeperPairDiag::PAIRS + slot] > 0);
        }
    }
    assert_eq!(
        KeeperCensus::slot(PlayerState::Recovering),
        KeeperCensus::SLOTS - 1
    );
    assert_eq!(
        KeeperCensus::slot_name(KeeperCensus::SLOTS - 1),
        "not a keeper state"
    );
}

#[test]
fn a_shot_is_counted_under_the_state_the_keeper_is_in() {
    let mut field = MatchField::new(840, 545, squad(1, 100), squad(2, 200));
    let mut census = KeeperCensus::default();
    for state in GoalkeeperState::ALL {
        field
            .players
            .iter_mut()
            .find(|p| {
                p.side == Some(PlayerSide::Right)
                    && p.tactical_position.current_position.is_goalkeeper()
            })
            .expect("the away side has a goalkeeper")
            .transition_to(
                PlayerState::Goalkeeper(state),
                TransitionSource::EventHandler,
            );
        let slot = state as usize;
        let before = KeeperCensus::snapshot().strike_shots[slot];
        census.note_strike(&field, PlayerSide::Right, 1_000);
        assert!(
            KeeperCensus::snapshot().strike_shots[slot] > before,
            "a shot at a keeper in {state} was not counted"
        );
    }
}
