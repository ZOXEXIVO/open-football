use crate::SimulatorData;
use crate::club::Club;
use crate::club::board::manager::search::ManagerSearch;
use crate::club::board::manager::seat::ManagerSeat;
use crate::club::news::ClubAffair;
use crate::club::staff::StaffPosition;
use chrono::NaiveDate;

/// Daily maintenance pass that guarantees the FM-style invariant: every
/// active main team is run by either a permanent `Manager` or an interim
/// `CaretakerManager`, and the board is actively searching whenever the
/// permanent seat is vacant.
///
/// Runs inside `ManagerMarketTick::run` after expired staff have been
/// harvested into the free-agent pool (so a club that just lost its last
/// coach this tick is repaired immediately) and before shortlists refresh
/// (so a freshly-opened search is given candidates the same tick). It is the
/// safety net behind sackings and poaches: those paths normally install a
/// caretaker themselves, but a club that has shed its whole coaching staff —
/// or a legacy save with a half-finished search — is recovered here.
pub struct ManagerSeatRepair;

impl ManagerSeatRepair {
    /// Walk every club with a main team and enforce the seat invariant.
    /// Serial mutable walk — mirrors `pool::harvest_expired_staff`; the
    /// work is a couple of position scans per club plus, in the rare vacant
    /// case, an interim promotion.
    pub fn run(data: &mut SimulatorData, today: NaiveDate) {
        for continent in &mut data.continents {
            for country in &mut continent.countries {
                for club in &mut country.clubs {
                    Self::repair_club(club, today);
                }
            }
        }
    }

    /// Enforce the head-coach-seat invariant for a single club.
    fn repair_club(club: &mut Club, today: NaiveDate) {
        let club_id = club.id;

        // Read the seat state (and main-team reputation for any search we
        // may open) up front, then drop the team borrow so we can touch the
        // board afterwards. Clubs with no main team are skipped entirely.
        let (manager_count, has_caretaker, main_rep) = {
            let Some(main) = club.teams.main() else {
                return;
            };
            (
                ManagerSeat::manager_count(main),
                ManagerSeat::has_caretaker(main),
                main.reputation.world,
            )
        };

        // ── A permanent manager is in post ──
        if manager_count >= 1 {
            if manager_count > 1 || has_caretaker {
                if let Some(main) = club.teams.main_mut() {
                    if manager_count > 1 {
                        ManagerSeat::dedupe_managers(main);
                    }
                    // A permanent manager and a caretaker can't co-hold the seat.
                    if has_caretaker {
                        ManagerSeat::clear_caretaker(main);
                    }
                }
            }
            // A filled permanent seat means any lingering search is stale.
            if club.board.manager_search_since.is_some() {
                ManagerSearch::clear(&mut club.board);
            }
            return;
        }

        // ── Interim in place, permanent seat open ──
        if has_caretaker {
            // The board must actually be hunting for a permanent successor.
            if club.board.manager_search_since.is_none() {
                ManagerSearch::open(&mut club.board, today, main_rep);
            }
            return;
        }

        // ── Nobody in the dugout — install interim cover, then search ──
        let mut caretaker: Option<u32> = None;
        let mut cupboard_bare = false;
        if let Some(main) = club.teams.main_mut() {
            // Prefer promoting the strongest internal coach; only mint a
            // synthetic caretaker when the cupboard is completely bare.
            if !ManagerSeat::promote_best_caretaker(main, 0, today) {
                ManagerSeat::install_emergency_caretaker(main, club_id, today);
                cupboard_bare = true;
            }
            caretaker = main
                .staffs
                .find_by_position(StaffPosition::CaretakerManager)
                .map(|staff| staff.id);
        }
        if let Some(staff_id) = caretaker {
            club.record_affair(ClubAffair::CaretakerAppointed { staff_id }, today);
        }
        // Promoting a coach is an interim appointment and reads as one.
        // Having to invent somebody because there was nobody left to
        // promote is a different story entirely — a club with no
        // coaching staff at all — and it had no way onto a page.
        if cupboard_bare {
            club.record_affair(ClubAffair::BackroomEmpty, today);
        }
        // Don't reset an existing search clock — only open one if the board
        // wasn't already searching (e.g. a sacking that found no caretaker).
        if club.board.manager_search_since.is_none() {
            ManagerSearch::open(&mut club.board, today, main_rep);
        }
    }
}
