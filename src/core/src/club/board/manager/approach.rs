use crate::club::board::manager::scorer::ManagerCandidateScorer;
use crate::club::board::manager::search::ManagerSearch;
use crate::club::board::manager::seat::ManagerSeat;
use crate::club::mind::organs::memory::{ActorRef, EpisodeKind};
use crate::club::news::ClubAffair;
use crate::club::staff::StaffPosition;
use crate::{Relations, SimulatorData, Staff, TeamType};
use chrono::NaiveDate;
use log::{debug, info};

/// State machine for an in-flight approach. One day per state advance:
/// `Made` (day 0) → either `CompensationDemanded` or `Rejected` (day 1)
/// → `CompensationAgreed` or `Rejected` (day 2) → `TermsAccepted` or
/// `Rejected` (day 3) → finalized (day 4, removes the approach).
///
/// Approaches are stored on `SimulatorData.pending_manager_approaches`
/// — a global registry so cascade hires (poached source club starting
/// its own search) can see the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApproachState {
    /// The requesting club has notified the source club. Awaiting
    /// permission-to-talk + compensation demand.
    Made,
    /// Source club agreed to release the manager for `amount` in
    /// compensation. Requesting club must now accept or walk away.
    CompensationDemanded { amount: u32 },
    /// Compensation agreed and (notionally) paid; the approach now
    /// proceeds to personal-terms negotiation with the candidate.
    CompensationAgreed,
    /// Candidate has accepted the offered terms. Next tick will
    /// finalize the move (and trigger source-club cascade).
    TermsAccepted,
    /// Approach is dead — recorded so the requesting club doesn't
    /// re-approach the same target while the entry is being cleaned
    /// up. Removed from the registry one tick after being set.
    Rejected,
}

/// One in-flight pursuit of an employed manager. Stored on
/// `SimulatorData.pending_manager_approaches` and ticked daily by the
/// world-level manager-market phase.
#[derive(Debug, Clone)]
pub struct ManagerApproach {
    pub requesting_club_id: u32,
    pub source_club_id: u32,
    pub staff_id: u32,
    pub state: ApproachState,
    /// Salary the requesting club is willing to offer the candidate.
    /// Set at approach creation; never re-negotiated within a single
    /// approach (a rejection forces the requesting club to start over
    /// with a fresh approach, possibly at a higher offer).
    pub offered_salary: u32,
    pub created_at: NaiveDate,
    /// Day the approach last transitioned. Used to enforce a one-day
    /// cooldown between state advances so the pipeline takes the
    /// realistic ~5 days from approach to signing.
    pub last_action: NaiveDate,
    /// Compensation that has actually changed hands, so a move that then
    /// falls through can be unwound. `None` until it is paid.
    pub compensation_paid: Option<u32>,
}

// ─── ManagerApproach — poaching state machine ──────────────────────────

impl ManagerApproach {
    /// True when this approach's `staff_id` is still the permanent
    /// `Manager` at `source_club_id`. Used as a guard at every state
    /// transition that would otherwise disturb the source club — a
    /// 5-day approach window leaves room for the target to be sacked,
    /// poached out from underneath, or have their contract lapse, so
    /// we re-verify each time we touch the source.
    fn target_is_source_manager(&self, data: &SimulatorData) -> bool {
        let Some(src) = data.club(self.source_club_id) else {
            return false;
        };
        let Some(main) = src.teams.main() else {
            return false;
        };
        let Some(mgr) = main.staffs.find_by_position(StaffPosition::Manager) else {
            return false;
        };
        if mgr.id != self.staff_id {
            return false;
        }
        mgr.contract.is_some()
    }

    /// Decide the next state for this approach based on the current
    /// state + the live state of the involved clubs. Side effects
    /// (moving staff, cascading search) are applied here too — the
    /// state-machine and effects are interleaved deliberately so a
    /// successful TermsAccepted transition immediately performs the
    /// move.
    pub(crate) fn advance(
        &mut self,
        data: &mut SimulatorData,
        today: NaiveDate,
    ) -> Option<ApproachState> {
        use ApproachState::*;

        match self.state {
            Made => {
                // Target sanity: the staff must still be the source
                // club's permanent manager. If they were sacked,
                // demoted, or moved elsewhere since the approach was
                // queued, drop it now — before the source club is ever
                // asked to entertain talks.
                if !self.target_is_source_manager(data) {
                    debug!(
                        "Approach rejected: staff {} no longer manager at source club {}",
                        self.staff_id, self.source_club_id
                    );
                    return Some(Rejected);
                }

                // Source club decides whether to entertain the approach.
                let (source_conf, source_overperforming, source_rep) = {
                    let Some(src) = data.club(self.source_club_id) else {
                        return Some(Rejected);
                    };
                    let conf = src.board.confidence.level;
                    let overperf = src.board.confidence.level >= 70;
                    let rep = src
                        .teams
                        .iter()
                        .find(|t| matches!(t.team_type, TeamType::Main))
                        .map(|t| t.reputation.world)
                        .unwrap_or(0);
                    (conf, overperf, rep)
                };
                if ManagerCandidateScorer::source_refuses_outright(
                    source_conf,
                    source_overperforming,
                ) {
                    debug!(
                        "Approach rejected: source club {} won't release manager",
                        self.source_club_id
                    );
                    return Some(Rejected);
                }

                // Look up the manager's current contract to compute
                // compensation. The previous guard already confirmed
                // the target still holds Manager + has a contract; the
                // let-else ladder is kept defensive against concurrent
                // mutation.
                let (current_salary, days_left) = {
                    let Some(src) = data.club(self.source_club_id) else {
                        return Some(Rejected);
                    };
                    let Some(main) = src
                        .teams
                        .iter()
                        .find(|t| matches!(t.team_type, TeamType::Main))
                    else {
                        return Some(Rejected);
                    };
                    let Some(mgr) = main.staffs.find_by_position(StaffPosition::Manager) else {
                        return Some(Rejected);
                    };
                    let Some(contract) = mgr.contract.as_ref() else {
                        return Some(Rejected);
                    };
                    let days_left = (contract.expired - today).num_days().max(30);
                    (contract.salary, days_left)
                };

                let years_left = (days_left as f32 / 365.0).max(0.5);
                let mult = ManagerCandidateScorer::compensation_multiplier(source_rep);
                let demand = ((current_salary as f32) * years_left * mult) as u32;
                Some(CompensationDemanded { amount: demand })
            }

            CompensationDemanded { amount } => {
                // Vacancy re-check before money changes hands. If the
                // requesting club already filled the seat (parallel
                // free-agent hire, caretaker confirmation, or another
                // poach finalising on this same tick), reject without
                // paying — compensation is wasted spend on an approach
                // that finalize would refuse anyway.
                let requesting_vacant = data
                    .club(self.requesting_club_id)
                    .map(|c| ManagerSeat::club_has_vacancy(c))
                    .unwrap_or(false);
                if !requesting_vacant {
                    debug!(
                        "Approach rejected: club {} already filled the seat — no compensation paid",
                        self.requesting_club_id
                    );
                    return Some(Rejected);
                }
                // Target re-check: the source manager may have moved
                // on since the demand was issued. Don't pay
                // compensation for someone who is no longer there.
                if !self.target_is_source_manager(data) {
                    debug!(
                        "Approach rejected: source target {} no longer Manager — no compensation paid",
                        self.staff_id
                    );
                    return Some(Rejected);
                }
                // Requesting club checks whether they can stomach the
                // compensation. Cap: 30% of cash balance, with a hard
                // floor of 200k so smaller clubs can still poach.
                let can_pay = {
                    let Some(req) = data.club(self.requesting_club_id) else {
                        return Some(Rejected);
                    };
                    let cap = ((req.finance.balance.balance as f32) * 0.30) as i64;
                    let cap = cap.max(200_000);
                    (amount as i64) <= cap
                };
                if !can_pay {
                    debug!(
                        "Approach rejected: club {} can't afford {} compensation for staff {}",
                        self.requesting_club_id, amount, self.staff_id
                    );
                    return Some(Rejected);
                }
                // Pay it — and to somebody. The buyer's outflow used to
                // go nowhere: the money left one club's books and was
                // never credited to the other's, so a club that had its
                // manager taken was paid a compensation it never
                // received. Posted as a staff-wages expense on the buyer
                // (the same line as the new manager's salary) and as
                // income on the club that loses him.
                if let Some(req) = data.club_mut(self.requesting_club_id) {
                    req.finance.balance.push_expense_staff_wages(amount as i64);
                }
                if let Some(src) = data.club_mut(self.source_club_id) {
                    src.finance.balance.push_income(amount as i64);
                }
                self.compensation_paid = Some(amount);
                Some(CompensationAgreed)
            }

            CompensationAgreed => {
                // Final sanity check before personal-terms negotiation
                // — the candidate cannot accept terms from a club they
                // no longer work for, or for a seat that's already
                // filled.
                if !self.target_is_source_manager(data) {
                    debug!(
                        "Approach rejected: source target {} no longer Manager pre-terms",
                        self.staff_id
                    );
                    return Some(Rejected);
                }
                // Personal terms: candidate compares offered_salary +
                // requesting-club prestige against current package.
                let accepted = ManagerCandidateScorer::candidate_accepts_terms(data, self);
                if accepted {
                    Some(TermsAccepted)
                } else {
                    debug!(
                        "Approach rejected: candidate {} rejected personal terms from club {}",
                        self.staff_id, self.requesting_club_id
                    );
                    Some(Rejected)
                }
            }

            TermsAccepted => {
                // Finalize: move staff from source to requesting;
                // cascade source search; clear requesting club's
                // search state.
                self.finalize(data, today);
                // Mark Rejected so the registry cleanup pass removes
                // the entry next tick. (We could add a `Signed`
                // terminal state, but the cleanup behaviour is
                // identical.)
                Some(Rejected)
            }

            Rejected => None,
        }
    }

    /// Give back a compensation paid for a move that then fell through.
    ///
    /// The approach reaches `finalize` only after `CompensationAgreed`, so
    /// by the time either re-check fails the money has already moved. Both
    /// clubs are put back where they were.
    fn refund_compensation(&self, data: &mut SimulatorData) {
        let Some(amount) = self.compensation_paid else {
            return;
        };
        if amount == 0 {
            return;
        }
        if let Some(req) = data.club_mut(self.requesting_club_id) {
            req.finance.balance.push_income(amount as i64);
        }
        if let Some(src) = data.club_mut(self.source_club_id) {
            src.finance.balance.push_outcome(amount as i64);
        }
    }

    /// Move the staff member from source to requesting club, install
    /// them as the new manager, clear the requesting club's search
    /// state, and open a fresh manager search on the source club
    /// (cascade).
    fn finalize(&self, data: &mut SimulatorData, today: NaiveDate) {
        // Vacancy re-check: an approach takes ~5 days to mature, and
        // the requesting club's seat may have been filled in the
        // meantime (free-agent hire, caretaker confirmation, parallel
        // poach). If it has, abort without disturbing the source club
        // and hand the compensation back — a move that never happened
        // is not a move anybody should have been paid for, and the
        // buyer used to eat the whole bill for a manager it did not get.
        // The pending approach is reaped via `Rejected` in the caller.
        let requesting_vacant = data
            .club(self.requesting_club_id)
            .map(|c| ManagerSeat::club_has_vacancy(c))
            .unwrap_or(false);
        if !requesting_vacant {
            info!(
                "Approach aborted: club {} already filled the seat before staff {} could be poached",
                self.requesting_club_id, self.staff_id
            );
            self.refund_compensation(data);
            return;
        }

        // Source target re-check: only poach a staff member who is
        // still the source club's permanent `Manager`. If the target
        // has been sacked, demoted, or already poached elsewhere since
        // the approach began, bail out — `take_by_id` would otherwise
        // move a Coach or Assistant who happens to share the id.
        if !self.target_is_source_manager(data) {
            info!(
                "Approach aborted: staff {} is no longer the manager at source club {}",
                self.staff_id, self.source_club_id
            );
            self.refund_compensation(data);
            return;
        }

        // Step 1: take the staff out of the source club's main team.
        // We pull by position (verified above to be the same id) so a
        // stale id can't inadvertently relocate a non-manager. Capture
        // the departing salary while we still hold the borrow — the
        // source club's caretaker promotion uses it as the salary
        // floor below.
        let mut staff: Option<Staff> = None;
        let mut source_salary: u32 = 0;
        if let Some(src) = data.club_mut(self.source_club_id) {
            if let Some(main) = src.teams.main_mut() {
                if let Some(taken) = main.staffs.take_by_position(StaffPosition::Manager) {
                    source_salary = taken.contract.as_ref().map(|c| c.salary).unwrap_or(0);
                    staff = Some(taken);
                }
            }
        }

        let Some(mut staff) = staff else {
            return;
        };

        // Step 2: install on requesting club. Demote any sitting
        // caretaker first so the head-coach seat is unique. Reset
        // relations so the new manager doesn't carry stale player
        // rapport from the old squad.
        let new_id = staff.id;
        staff.contract = Some(ManagerSeat::build_manager_contract(
            self.offered_salary,
            today,
        ));
        staff.relations = Relations::new();
        staff.fatigue = 0.0;
        staff.job_satisfaction = 75.0; // Fresh job: optimistic.

        // He leaves one place and arrives at another on the same
        // morning, and the order matters: the spell is closed against
        // the club he was at, and only then does he take the new job.
        // Memory and his judgements of players travel with him; his
        // standing with that board, room and crowd does not.
        staff.leave_club(self.source_club_id);
        staff.remember(
            EpisodeKind::AppointedManager,
            ActorRef::club(self.requesting_club_id),
            today,
            self.requesting_club_id,
        );

        let mut signed = false;
        if let Some(req) = data.club_mut(self.requesting_club_id) {
            if let Some(main) = req.teams.main_mut() {
                ManagerSeat::clear_caretaker(main);
                main.staffs.push(staff);
                signed = true;
                info!(
                    "Manager poached: staff {} → club {} (compensation paid, terms agreed)",
                    new_id, self.requesting_club_id
                );
            }
            if signed {
                // The loud version of an appointment: he was somebody
                // else's manager on Friday.
                req.record_affair(
                    ClubAffair::ManagerAppointed {
                        staff_id: new_id,
                        from_club_id: self.source_club_id,
                    },
                    today,
                );
            }
            // Clear requesting club's search state — the seat is filled.
            req.board.chairman.manager_loyalty = 50;
            ManagerSearch::clear(&mut req.board);
        }

        if !signed {
            // Defensive: requesting club lookup failed during the
            // install step (only reachable if the requesting club was
            // deleted mid-tick — not a path the simulator currently
            // exercises). The staff is already taken from source;
            // nothing left to do for them here, log so we'd notice if
            // this ever fires.
            log::warn!(
                "Manager market: lost staff {} mid-finalize for club {}",
                new_id,
                self.requesting_club_id
            );
            return;
        }

        // Step 3: cascade — source club's seat is now vacant. Open a
        // fresh search on them with their rep-scaled window AND
        // promote an interim caretaker so match/training code keeps a
        // head_coach() fallback. This chain reaction makes the world
        // feel alive: one Real Madrid hire of a Bayer Leverkusen coach
        // forces Leverkusen into their own search, which in turn might
        // poach from a Bundesliga rival, and so on.
        if let Some(src) = data.club_mut(self.source_club_id) {
            let src_rep = src
                .teams
                .iter()
                .find(|t| matches!(t.team_type, TeamType::Main))
                .map(|t| t.reputation.world)
                .unwrap_or(0);
            let mut caretaker: Option<u32> = None;
            if let Some(main) = src.teams.main_mut() {
                if ManagerSeat::promote_best_caretaker(main, source_salary, today) {
                    caretaker = main
                        .staffs
                        .find_by_position(StaffPosition::CaretakerManager)
                        .map(|staff| staff.id);
                }
            }
            // Losing a manager to a bigger club is a different morning
            // from being talked into sacking one, and the source club's
            // paper has always had to print them as the same sentence.
            src.record_affair(
                ClubAffair::ManagerPoached {
                    staff_id: new_id,
                    to_club_id: self.requesting_club_id,
                },
                today,
            );
            if let Some(staff_id) = caretaker {
                src.record_affair(ClubAffair::CaretakerAppointed { staff_id }, today);
            }
            ManagerSearch::open(&mut src.board, today, src_rep);
            // Reset confidence so the new search starts on neutral
            // footing — the departing manager's good results don't
            // become a head-wind against finding a successor.
            src.board.confidence.level = 50;
            src.board.poor_mood_months = 0;
            info!(
                "Cascade: club {} enters manager search after losing staff {}",
                self.source_club_id, new_id
            );
        }
    }
}
