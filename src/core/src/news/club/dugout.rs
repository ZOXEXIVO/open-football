use crate::club::board::manager::ApproachState;
use crate::club::news::{ClubDugoutWatch, ManagerPursuit};
use crate::world::SimulatorData;
use rustc_hash::FxHashMap;

/// Who is being chased for whose dugout this week.
///
/// The manager market keeps its in-flight approaches in one world-level
/// registry, because a pursuit belongs to neither club: the requesting
/// side has no field saying "we have moved for him" and the source side
/// has no field saying "somebody wants ours". Both find out the same way
/// a supporter does — from the papers — so both are handed their half of
/// every live approach here.
pub(super) struct WeeklyDugout {
    by_club: FxHashMap<u32, ClubDugoutWatch>,
}

impl WeeklyDugout {
    pub(super) fn from_world(data: &SimulatorData) -> Self {
        let mut by_club: FxHashMap<u32, ClubDugoutWatch> = FxHashMap::default();

        for approach in &data.pending_manager_approaches {
            // A dead approach is not a link, and the registry keeps
            // rejected entries for one tick before reaping them.
            if matches!(approach.state, ApproachState::Rejected) {
                continue;
            }

            by_club
                .entry(approach.requesting_club_id)
                .or_default()
                .pursuits
                .push(ManagerPursuit {
                    staff_id: approach.staff_id,
                    other_club_id: approach.source_club_id,
                    we_are_asking: true,
                });
            by_club
                .entry(approach.source_club_id)
                .or_default()
                .pursuits
                .push(ManagerPursuit {
                    staff_id: approach.staff_id,
                    other_club_id: approach.requesting_club_id,
                    we_are_asking: false,
                });
        }

        WeeklyDugout { by_club }
    }

    pub(super) fn for_club(&self, club_id: u32) -> Option<&ClubDugoutWatch> {
        self.by_club.get(&club_id)
    }
}
