use crate::Staff;
use crate::club::board::ClubBoard;
use crate::club::board::manager::candidate::CandidateSource;
use crate::club::board::manager::scorer::ManagerCandidateScorer;
use chrono::NaiveDate;

pub struct ManagerSearch;

impl ManagerSearch {
    /// Initialise search state on a fresh sacking. Records the start
    /// day and locks in the search-window length based on club rep —
    /// kept on the board so we don't recompute from scratch every tick.
    pub fn open(board: &mut ClubBoard, today: NaiveDate, club_rep: u16) {
        board.manager_search_since = Some(today);
        board.search_window_days = ManagerCandidateScorer::search_window_days(club_rep);
        board.manager_shortlist.clear();
        board.shortlist_built_at = None;
    }

    /// Wipe shortlist state once a hire is finalised. Called from
    /// `result.rs` on confirm_new_manager whether the hire succeeded
    /// or fell back to the caretaker.
    pub fn clear(board: &mut ClubBoard) {
        board.manager_shortlist.clear();
        board.shortlist_built_at = None;
        board.manager_search_since = None;
        board.search_window_days = 0;
    }

    /// Pull the top free-agent candidate off a board's shortlist and
    /// remove the matching `Staff` from the global pool. Returns the
    /// owned staff member ready to be assigned to the team's roster,
    /// or `None` if the shortlist is empty / the candidate has already
    /// been signed by someone else (race during a daily tick).
    pub fn take_top_free_agent(
        board: &mut ClubBoard,
        pool: &mut Vec<Staff>,
    ) -> Option<(Staff, u32)> {
        while let Some(candidate) = board.manager_shortlist.first().cloned() {
            // Drop this entry up-front — even if we fail to find the
            // staff (signed elsewhere already), the entry was stale
            // either way.
            board.manager_shortlist.remove(0);
            if !matches!(candidate.source, CandidateSource::FreeAgent) {
                // Slice B only handles free-agent path; employed
                // candidates are handled via the approach pipeline in
                // slice C.
                continue;
            }
            let idx = pool.iter().position(|s| s.id == candidate.staff_id)?;
            let staff = pool.remove(idx);
            return Some((staff, candidate.target_salary));
        }
        None
    }
}
