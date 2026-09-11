use crate::Staff;

/// Where this candidate came from. Slice C adds `Employed` and the
/// approach pipeline that operates on it; for now only `FreeAgent`
/// is reachable, but the variant exists so callers can pattern-match
/// without breaking when slice C lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateSource {
    FreeAgent,
    Employed { current_club_id: u32 },
}

/// A ranked entry on a club's manager shortlist. `fit_score` is the
/// composite ranking value; `target_salary` is what the candidate
/// would expect to be offered (the board's actual offer may flex).
#[derive(Debug, Clone)]
pub struct ManagerCandidate {
    pub staff_id: u32,
    pub fit_score: i32,
    pub target_salary: u32,
    pub source: CandidateSource,
}

/// A poachable in-post manager, captured by ONE world walk before the
/// per-club shortlist refresh. Holds a borrow of the seated manager
/// plus the requester-independent filter inputs (its club id and the
/// club's world reputation). Each refreshing club then filters this
/// shared snapshot by its own reputation ceiling and rescores the
/// survivors — collapsing the old `O(searching_clubs × all_clubs)`
/// re-walk into `O(all_clubs)` plus a cheap per-requester pass.
pub(crate) struct EmployedCandidateRaw<'a> {
    pub(crate) manager: &'a Staff,
    pub(crate) club_id: u32,
    pub(crate) club_world_rep: u16,
}
