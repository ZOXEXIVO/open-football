use crate::club::board::manager::approach::ApproachState;
use crate::club::board::manager::approach::ManagerApproach;
use crate::club::board::manager::candidate::CandidateSource;
use crate::club::board::manager::candidate::ManagerCandidate;
use crate::club::board::manager::repair::ManagerSeatRepair;
use crate::club::board::manager::search::ManagerSearch;
use crate::club::board::manager::seat::ManagerSeat;
use crate::club::board::manager::shortlist::ManagerShortlist;
use crate::club::mind::organs::memory::{ActorRef, EpisodeKind};
use crate::club::news::ClubAffair;
use crate::club::staff::StaffPosition;
use crate::club::staff::pool;
use crate::{SimulatorData, Staff, TeamType};
use chrono::NaiveDate;
use log::debug;
use rayon::prelude::*;

/// Outcome of a single club's pass through `refresh_shortlists` —
/// either the club's stale search must be cleared, or it needs a fresh
/// candidate shortlist. Carrying both decisions in one parallel sweep
/// keeps the world walk to a single pass.
enum ShortlistDecision {
    /// Club id, its world reputation, and the most it will pay a coach.
    Refresh(u32, u16, Option<u32>),
    Clear(u32),
}

pub struct ManagerMarketTick;

impl ManagerMarketTick {
    /// Run one daily tick of the world-level manager market in the
    /// canonical order: harvest expired contracts → age the pool →
    /// repair vacant seats → refresh shortlists → initiate fresh
    /// approaches → advance in-flight approaches.
    ///
    /// The order is load-bearing:
    ///   1. Sacked / contract-lapsed staff must hit the free-agent
    ///      pool *before* shortlists refresh, otherwise the freshly-
    ///      vacated seats look like they have no candidates.
    ///   2. Pool aging (satisfaction decay, retirement) runs before
    ///      shortlists so retiring coaches don't appear as candidates
    ///      this tick.
    ///   3. Seat repair runs after harvesting (so a club that just lost
    ///      its last coach is recovered) and before shortlist refresh
    ///      (so a freshly-opened vacancy search gets candidates the same
    ///      tick): every main team ends the step with a Manager or a
    ///      CaretakerManager, and every vacant club has an open search.
    ///   4. Shortlists must exist before fresh approaches initiate
    ///      (an approach picks from the shortlist).
    ///   5. Approach ticks must run *after* fresh initiation so a
    ///      brand-new approach starts at state 0 and doesn't get
    ///      advanced on the same tick.
    ///
    /// Wrapping these in one function localises the contract — adding
    /// a further step now means editing one place rather than several
    /// call sites scattered around the orchestrator.
    pub fn run(data: &mut SimulatorData, today: NaiveDate) {
        pool::StaffPool::harvest_expired(data, today);
        pool::StaffPool::tick(&mut data.free_agent_staff, today);
        ManagerSeatRepair::run(data, today);
        Self::refresh_shortlists(data);
        Self::initiate_approaches(data);
        Self::tick_approaches(data);
    }

    /// World-level pass: for every club in active manager search,
    /// refresh its `manager_shortlist` if stale. Stale searches whose
    /// permanent manager is still in post are wiped instead — keeps
    /// the manager market from generating phantom approaches against
    /// clubs that aren't really hiring.
    pub fn refresh_shortlists(data: &mut SimulatorData) {
        let today = data.date.date();

        // Snapshot pool by reference — we only need to read it. We
        // collect (club_id, club_rep) first to avoid holding an iter+mut
        // borrow across the rebuild closure. The world walk runs across
        // rayon so single-continent saves still light up every core.
        let decisions: Vec<ShortlistDecision> = data
            .continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.clubs.par_iter())
            .filter_map(|club| {
                club.board.manager_search_since?;
                if !ManagerSeat::club_has_vacancy(club) {
                    debug!(
                        "Manager market: clearing stale search at club {} (permanent manager in post)",
                        club.id
                    );
                    return Some(ShortlistDecision::Clear(club.id));
                }
                let stale = club
                    .board
                    .shortlist_built_at
                    .map(|d| (today - d).num_days() >= ManagerShortlist::REFRESH_DAYS)
                    .unwrap_or(true);
                if !stale {
                    return None;
                }
                let club_rep = club
                    .teams
                    .iter()
                    .find(|t| matches!(t.team_type, TeamType::Main))
                    .map(|t| t.reputation.world)
                    .unwrap_or(0);
                Some(ShortlistDecision::Refresh(
                    club.id,
                    club_rep,
                    ManagerShortlist::salary_ceiling(club),
                ))
            })
            .collect();

        let mut to_refresh: Vec<(u32, u16, Option<u32>)> = Vec::new();
        let mut stale_to_clear: Vec<u32> = Vec::new();
        for d in decisions {
            match d {
                ShortlistDecision::Refresh(id, rep, ceiling) => to_refresh.push((id, rep, ceiling)),
                ShortlistDecision::Clear(id) => stale_to_clear.push(id),
            }
        }

        for club_id in stale_to_clear {
            if let Some(club) = data.club_mut(club_id) {
                ManagerSearch::clear(&mut club.board);
            }
        }

        if to_refresh.is_empty() {
            return;
        }

        // Walk the world ONCE for poachable in-post managers, then let
        // each refreshing club filter + rescore that shared snapshot
        // against its own reputation. Previously every refreshing club
        // re-walked the entire world inside `combined` — O(searching ×
        // all_clubs); now it is O(all_clubs) + a cheap per-requester
        // pass over the snapshot.
        let employed_pool = ManagerShortlist::enumerate_employed_pool(data);

        // Build candidate lists outside the mutable-club borrow, then
        // write back. Reads from the free-agent pool AND from the shared
        // employed snapshot (both immutable borrows of `data`), so this
        // is a read-only sweep before the write phase. Each per-club
        // shortlist build is independent → parallel.
        let updates: Vec<(u32, Vec<ManagerCandidate>)> = to_refresh
            .par_iter()
            .map(|&(club_id, club_rep, salary_ceiling)| {
                let shortlist = ManagerShortlist::combined(
                    &data.free_agent_staff,
                    &employed_pool,
                    club_id,
                    club_rep,
                    salary_ceiling,
                    today,
                );
                (club_id, shortlist)
            })
            .collect();

        // Drop the snapshot's immutable borrow of `data` before the
        // mutable write-back loop below.
        drop(employed_pool);

        for (club_id, shortlist) in updates {
            if let Some(club) = data.club_mut(club_id) {
                debug!(
                    "Manager market: refreshed shortlist for club id {} ({} candidates)",
                    club_id,
                    shortlist.len()
                );
                club.board.manager_shortlist = shortlist;
                club.board.shortlist_built_at = Some(today);
            }
        }
    }

    /// Look at every club in active manager search and create a fresh
    /// `ManagerApproach` for the top employed candidate on their
    /// shortlist that doesn't already have one in flight. One approach
    /// per requesting club per tick — keeps the pace realistic and
    /// avoids flood-spam of identical approaches.
    pub fn initiate_approaches(data: &mut SimulatorData) {
        let today = data.date.date();

        let new_approaches: Vec<ManagerApproach> = data
            .continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.clubs.par_iter())
            .filter_map(|club| {
                if club.board.manager_search_since.is_none() {
                    return None;
                }
                // Vacancy invariant: never start an approach for a
                // club whose permanent manager is still in post
                // (stale search state). `refresh_shortlists` clears
                // such state on its next pass; until then, just skip.
                if !ManagerSeat::club_has_vacancy(club) {
                    return None;
                }
                // Pick the top-ranked Employed candidate that isn't
                // already in an in-flight approach for this club.
                let already_pursuing: Vec<u32> = data
                    .pending_manager_approaches
                    .iter()
                    .filter(|a| a.requesting_club_id == club.id)
                    .map(|a| a.staff_id)
                    .collect();
                let pick = club.board.manager_shortlist.iter().find(|c| {
                    matches!(c.source, CandidateSource::Employed { .. })
                        && !already_pursuing.contains(&c.staff_id)
                })?;
                let CandidateSource::Employed { current_club_id } = pick.source else {
                    return None;
                };
                Some(ManagerApproach {
                    requesting_club_id: club.id,
                    source_club_id: current_club_id,
                    staff_id: pick.staff_id,
                    state: ApproachState::Made,
                    offered_salary: pick.target_salary,
                    created_at: today,
                    last_action: today,
                    compensation_paid: None,
                })
            })
            .collect();

        for a in new_approaches {
            debug!(
                "Manager market: approach made — club {} → manager {} (at club {})",
                a.requesting_club_id, a.staff_id, a.source_club_id
            );
            data.pending_manager_approaches.push(a);
        }
    }

    /// Advance every pending approach one tick. State transitions are
    /// gated to one-per-day (via `last_action`) so an approach takes
    /// the full ~5 days from inception to signing.
    ///
    /// Successful approaches finalize here: the staff member is moved
    /// from the source club's roster to the requesting club's, the
    /// requesting club's search state is cleared, and the source club
    /// opens its OWN search (the "cascade").
    pub fn tick_approaches(data: &mut SimulatorData) {
        let today = data.date.date();
        if data.pending_manager_approaches.is_empty() {
            return;
        }

        // Collect all approach indices to process this tick.
        let indices: Vec<usize> = data
            .pending_manager_approaches
            .iter()
            .enumerate()
            .filter(|(_, a)| (today - a.last_action).num_days() >= 1)
            .map(|(i, _)| i)
            .collect();

        for i in indices {
            // Advanced on a clone because the state machine needs `data`
            // mutably while it works. Written back whole, so whatever the
            // step recorded on the approach itself — the compensation that
            // actually changed hands — survives the tick.
            let mut approach = data.pending_manager_approaches[i].clone();
            let next: Option<ApproachState> = approach.advance(data, today);
            if let Some(next_state) = next {
                approach.state = next_state;
                approach.last_action = today;
                data.pending_manager_approaches[i] = approach;
            }
        }

        // Reap rejected approaches (one-tick cleanup so the requesting
        // club won't immediately re-pursue the same target on the next
        // shortlist refresh).
        data.pending_manager_approaches
            .retain(|a| !matches!(a.state, ApproachState::Rejected));
    }

    /// Execute the permanent appointment for a club whose search
    /// window has elapsed. Owns the multi-step borrow choreography:
    /// peek the shortlist (needs club mut), withdraw the candidate
    /// from the global pool (needs data mut, club borrow dropped),
    /// then install them on the team (needs club mut again).
    ///
    /// Falls back to permanently promoting the sitting caretaker when
    /// the shortlist has no viable free-agent candidate — the common
    /// path for small clubs whose pool offerings are slim.
    pub fn execute_appointment(data: &mut SimulatorData, club_id: u32, today: NaiveDate) {
        if club_id == 0 {
            return;
        }

        // Vacancy invariant: never install a second permanent manager
        // on top of an existing one. If `manager_search_since` is stale
        // (a poach already filled the seat, or the search state was
        // never cleared after a renewal), wipe the search and bail
        // out — no sacking, no candidate consumed.
        {
            let Some(club) = data.club(club_id) else {
                return;
            };
            if !ManagerSeat::club_has_vacancy(club) {
                debug!(
                    "Manager market: refusing appointment at club {} — permanent manager still in post",
                    club_id
                );
                if let Some(club_mut) = data.club_mut(club_id) {
                    ManagerSearch::clear(&mut club_mut.board);
                }
                return;
            }
        }

        // Step 1: Strip the caretaker tag. The caretaker is demoted
        // back to a generic Coach role so the seat is empty when we
        // push the new permanent appointment. We don't try to remember
        // the coach's pre-promotion role — simplification doesn't
        // materially affect simulation depth.
        let club_name: String;
        {
            let Some(club) = data.club_mut(club_id) else {
                return;
            };
            club_name = club.name.clone();
            if let Some(main_team) = club.teams.main_mut() {
                ManagerSeat::clear_caretaker(main_team);
            }
        }

        // Step 2: Identify the top free-agent candidate (id + agreed
        // salary) by reading the shortlist. We don't pop yet — the
        // staff might no longer be in the pool (signed by another club
        // this tick); only pop after we confirm the move.
        let top_candidate: Option<(u32, u32)> = {
            let Some(club) = data.club(club_id) else {
                return;
            };
            club.board
                .manager_shortlist
                .iter()
                .find(|c| matches!(c.source, CandidateSource::FreeAgent))
                .map(|c| (c.staff_id, c.target_salary))
        };

        // Step 3: Try to take the candidate from the pool. If the slot
        // has been signed already, we drop the entry and fall through
        // to the caretaker-promotion path. Pool access requires no
        // club borrow.
        let signed: Option<(Staff, u32)> = if let Some((staff_id, salary)) = top_candidate {
            let pool = &mut data.free_agent_staff;
            let removed = pool
                .iter()
                .position(|s| s.id == staff_id)
                .map(|idx| pool.remove(idx));
            removed.map(|staff| (staff, salary))
        } else {
            None
        };

        // Step 4: Install the new manager (or fallback). All work back
        // inside the club borrow.
        {
            let Some(club) = data.club_mut(club_id) else {
                return;
            };

            let mut appointment: Option<ClubAffair> = None;
            if let Some((mut new_manager, salary)) = signed {
                let new_id = new_manager.id;
                new_manager.contract = Some(ManagerSeat::build_manager_contract(salary, today));
                new_manager.job_satisfaction = 70.0; // Fresh start: optimistic.
                new_manager.remember(
                    EpisodeKind::AppointedManager,
                    ActorRef::club(club_id),
                    today,
                    club_id,
                );
                if let Some(main_team) = club.teams.main_mut() {
                    main_team.staffs.push(new_manager);
                    // Out of work when the club came for him: the
                    // ordinary appointment, and a quieter story than
                    // prising a man out of a job he already had.
                    appointment = Some(ClubAffair::ManagerAppointed {
                        staff_id: new_id,
                        from_club_id: 0,
                    });
                    debug!(
                        "Free-agent signed: staff {} appointed manager at {} ({}/y)",
                        new_id, club_name, salary
                    );
                }
            } else if let Some(main_team) = club.teams.main_mut() {
                // Fallback: ex-caretaker (now Coach) → permanent
                // Manager on a 3-year deal at their existing salary.
                // The board takes the conservative option when no
                // realistic free agent stepped up during the search
                // window.
                if let Some(staff) = main_team.staffs.find_mut_by_position(StaffPosition::Coach) {
                    let salary = staff.contract.as_ref().map(|c| c.salary).unwrap_or(0);
                    let id = staff.id;
                    staff.contract = Some(ManagerSeat::build_manager_contract(salary, today));
                    // The man the club already had. He does not arrive,
                    // so nothing about his standing resets — he simply
                    // gets the job for keeps.
                    staff.remember(
                        EpisodeKind::PromotedFromWithin,
                        ActorRef::club(club_id),
                        today,
                        club_id,
                    );
                    // The caretaker keeps the job. Its own kind of story
                    // — the man the club already had, given it for keeps
                    // because nobody better would come.
                    appointment = Some(ClubAffair::CaretakerConfirmed { staff_id: id });
                    debug!(
                        "Caretaker {} confirmed as permanent manager at {} (no free-agent shortlist)",
                        id, club_name
                    );
                }
            }

            if let Some(affair) = appointment {
                club.record_affair(affair, today);
            }

            // Fresh appointment — wipe chairman loyalty toward the
            // predecessor, clear all search state.
            club.board.chairman.manager_loyalty = 50;
            ManagerSearch::clear(&mut club.board);
        }
    }
}
