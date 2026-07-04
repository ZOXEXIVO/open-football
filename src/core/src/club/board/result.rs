use crate::club::StaffPosition;
use crate::club::board::manager_market;
use crate::club::board::{BoardDecision, BoardFacility, BoardMoodState};
use crate::club::facilities::FacilityLevel;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::league::result::LeagueProcessAccess;
use crate::{Club, HappinessEventType, Staff, StaffEventType, TeamType};
use chrono::Datelike;
use log::{debug, info};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardManagerMeeting {
    Backing,
    Warning,
    Crisis,
}

pub struct BoardResult {
    pub club_id: u32,
    pub players_loan_listed: u32,
    pub players_transfer_listed: u32,
    pub mood: BoardMoodState,
    pub confidence: i32,
    pub cut_transfer_budget: bool,
    /// Board releases extra funds for overperformance
    pub bonus_transfer_funds: bool,
    pub squad_over_limit: bool,
    pub squad_excess: usize,
    pub squad_under_limit: bool,
    /// Team is significantly below expected league position
    pub underperforming: bool,
    /// Board has lost confidence — terminate manager contract this tick.
    pub manager_sacked: bool,
    /// The board's first crisis meeting put the manager on a public
    /// final warning THIS tick — the squad reacts once, forked by each
    /// player's own bond with the head coach.
    pub manager_ultimatum_announced: bool,
    /// Search period (≥30 days) has elapsed — promote the sitting
    /// caretaker to a permanent manager contract.
    pub confirm_new_manager: bool,
    /// Signed delta applied to the main-team manager's job_satisfaction
    /// this tick. Positive when the board is happy; negative when
    /// confidence is sliding. Applied in `process`.
    pub manager_satisfaction_delta: f32,
    /// Trigger a mid/late-contract renewal offer to the incumbent
    /// manager — set at season start when board confidence is high
    /// and the contract is approaching its tail end.
    pub offer_manager_renewal: bool,
    /// Monthly board contact with the head coach: public backing, a formal
    /// warning, or a crisis meeting before/around dismissal risk.
    pub manager_meeting: Option<BoardManagerMeeting>,
    /// Explainable, machine-readable board decisions emitted this tick.
    /// `process` applies the ones with real effects (budgets, facilities,
    /// takeover); the rest are informational for the UI.
    pub decisions: Vec<BoardDecision>,
}

impl BoardResult {
    pub fn new() -> Self {
        BoardResult {
            club_id: 0,
            players_loan_listed: 0,
            players_transfer_listed: 0,
            mood: BoardMoodState::Normal,
            confidence: 65,
            cut_transfer_budget: false,
            bonus_transfer_funds: false,
            squad_over_limit: false,
            squad_excess: 0,
            squad_under_limit: false,
            underperforming: false,
            manager_sacked: false,
            manager_ultimatum_announced: false,
            confirm_new_manager: false,
            manager_satisfaction_delta: 0.0,
            offer_manager_renewal: false,
            manager_meeting: None,
            decisions: Vec::new(),
        }
    }

    pub fn process<D: LeagueProcessAccess>(&self, data: &mut D) {
        if self.club_id == 0 {
            return;
        }

        // Grab the sim date before we take a mutable club borrow.
        let today = data.date().date();

        // Sacked staff is collected during the club-mut block and admitted
        // to the global free-agent pool *after* the club borrow ends —
        // `data.free_agent_staff` is on the same `data` and the borrow
        // checker won't allow both mut paths simultaneously.
        let mut sacked_staff: Option<Staff> = None;
        // Mirror for confirm-new-manager: the appointment runs in
        // `manager_market::execute_appointment` after the club borrow
        // ends because it needs concurrent access to the pool.
        let do_confirm = self.confirm_new_manager;

        {
            let club = match data.club_mut(self.club_id) {
                Some(c) => c,
                None => return,
            };

            // Budget movements flow exclusively through `BoardDecision`
            // entries now — see `ClubBoard::emit_budget_decisions`. The
            // legacy `cut_transfer_budget` / `bonus_transfer_funds` flags
            // and the board mood are kept for the UI but no longer drive a
            // separate percentage tweak here (that double-applied with the
            // decision amounts). `apply_decisions` is the single mutation
            // point for budgets, facility upgrades, and takeover injections.
            Self::apply_decisions(&self.decisions, club);

            // Push the board's mood onto the manager's own job satisfaction —
            // a coach at a happy club feels secure, a coach under Poor mood
            // feels the pressure building. Applied after the sacking path so
            // we don't adjust a seat that's just been vacated.
            if self.manager_satisfaction_delta.abs() > 0.01 && !self.manager_sacked {
                if let Some(main_team) = club.teams.main_mut() {
                    if let Some(mgr) = main_team
                        .staffs
                        .find_mut_by_position(StaffPosition::Manager)
                    {
                        mgr.job_satisfaction = (mgr.job_satisfaction
                            + self.manager_satisfaction_delta)
                            .clamp(0.0, 100.0);
                    }
                }
            }

            if let Some(meeting) = self.manager_meeting {
                if let Some(main_team) = club.teams.main_mut() {
                    if let Some(mgr) = main_team
                        .staffs
                        .find_mut_by_position(StaffPosition::Manager)
                    {
                        let event = match meeting {
                            BoardManagerMeeting::Backing => StaffEventType::TrustBuilt,
                            BoardManagerMeeting::Warning => StaffEventType::PerformanceDeclined,
                            BoardManagerMeeting::Crisis => StaffEventType::Conflict,
                        };
                        mgr.add_event(event);
                    }
                }
            }

            // Season-start renewal: if the board wants to keep the manager,
            // extend the contract by two years and give a salary bump. The
            // manager is trusted, so the terms are friendly; this prevents
            // a successful coach from running down their deal and walking
            // for free. Only fires when the current contract is short enough
            // to genuinely be at risk (≤18 months out).
            if self.offer_manager_renewal && !self.manager_sacked {
                if let Some(main_team) = club.teams.main_mut() {
                    if let Some(mgr) = main_team
                        .staffs
                        .find_mut_by_position(StaffPosition::Manager)
                    {
                        let should_offer = mgr
                            .contract
                            .as_ref()
                            .map(|c| (c.expired - today).num_days() < 540)
                            .unwrap_or(true);
                        if should_offer {
                            if let Some(contract) = mgr.contract.as_mut() {
                                let new_expires = today
                                    .with_year(today.year() + 2)
                                    .unwrap_or(contract.expired);
                                if new_expires > contract.expired {
                                    contract.expired = new_expires;
                                }
                                contract.salary = ((contract.salary as f32) * 1.15) as u32;
                                mgr.job_satisfaction =
                                    (mgr.job_satisfaction + 10.0).clamp(0.0, 100.0);
                                info!(
                                    "Board offered renewal (+2y, +15% salary) to manager {} at {}",
                                    mgr.id, club.name
                                );
                            }
                        }
                    }
                }
            }

            // Ultimatum made public this tick: the squad reads the
            // situation through each player's own bond with the head
            // coach — loyalists rally to save his job, the rest sense a
            // change coming (and hold their pens on new deals via the
            // mood's morale drag). Skipped when a total confidence
            // collapse sacks the manager the same tick.
            if self.manager_ultimatum_announced && !self.manager_sacked {
                if let Some(main_team) = club.teams.main_mut() {
                    let coach_id = main_team.staffs.head_coach().id;
                    let cfg = HappinessConfig::default();
                    for player in main_team.players.players.iter_mut() {
                        let bond = player
                            .relations
                            .get_staff(coach_id)
                            .map(|r| r.personal_bond + r.trust_in_abilities + r.loyalty * 0.5)
                            .unwrap_or(0.0);
                        if bond >= 100.0 {
                            player.happiness.add_event_with_cooldown(
                                HappinessEventType::RalliesBehindManager,
                                cfg.catalog.rallies_behind_manager,
                                45,
                            );
                        } else {
                            player.happiness.add_event_with_cooldown(
                                HappinessEventType::SensesManagerChange,
                                cfg.catalog.senses_manager_change,
                                45,
                            );
                        }
                    }
                }
            }

            // Sacking: terminate the manager contract on the main team and
            // promote the best available coaching-staff member to caretaker.
            // The caretaker runs the team until the 30-day search concludes
            // (see `confirm_new_manager` below). The sacked staff member is
            // *removed* from the team's roster (not just stripped of contract)
            // and routed into the global free-agent pool below the block, so
            // a rival club can sign them next tick.
            if self.manager_sacked {
                let club_name = club.name.clone();
                if let Some(main_team) = club.teams.main_mut() {
                    let mut sacked_salary: u32 = 0;
                    if let Some(staff) = main_team.staffs.take_by_position(StaffPosition::Manager) {
                        let id = staff.id;
                        if let Some(c) = &staff.contract {
                            sacked_salary = c.salary;
                        }
                        info!(
                            "Board sacked manager (staff id {}) at {} — confidence {}",
                            id, club_name, self.confidence
                        );
                        sacked_staff = Some(staff);
                    }

                    let installed = manager_market::ManagerSeat::promote_best_caretaker(
                        main_team,
                        sacked_salary,
                        today,
                    );
                    if installed {
                        debug!("Caretaker promoted at {} after sacking", club_name);
                    }
                }

                // Start the search clock on the board, locking in the
                // rep-scaled search window. Top clubs hunt for ~60 days,
                // small clubs ~21. The window stays stable across the
                // search even if reputation fluctuates.
                let club_rep = club
                    .teams
                    .iter()
                    .find(|t| matches!(t.team_type, TeamType::Main))
                    .map(|t| t.reputation.world)
                    .unwrap_or(0);
                manager_market::ManagerSearch::open(&mut club.board, today, club_rep);
            }
        } // end of `club` mutable-borrow scope

        // The club borrow has ended — routes the cross-cutting writes
        // through the trait. SimulatorData applies them inline (same
        // semantics as before); CountryProcessCtx pushes onto its
        // DeferredGlobalOps queue and the simulator drains it
        // serially after the parallel pass joins.
        if let Some(staff) = sacked_staff {
            data.admit_free_agent_staff(staff);
        }
        if do_confirm {
            data.queue_manager_appointment(self.club_id);
        }
    }

    /// Apply the board decisions that have concrete club-state effects:
    /// transfer/wage budget adjustments, approved facility upgrades, and a
    /// takeover cash injection. Other variants (meetings, sackings, search,
    /// rumours, demands) are informational or handled by legacy fields.
    fn apply_decisions(decisions: &[BoardDecision], club: &mut Club) {
        for decision in decisions {
            match decision {
                BoardDecision::IncreaseTransferBudget { amount, .. } => {
                    if let Some(budget) = club.finance.transfer_budget.as_mut() {
                        budget.amount += *amount as f64;
                    }
                    debug!(
                        "Board raised transfer budget at {} by {}",
                        club.name, amount
                    );
                }
                BoardDecision::CutTransferBudget { amount, .. } => {
                    if let Some(budget) = club.finance.transfer_budget.as_mut() {
                        budget.amount = (budget.amount - *amount as f64).max(0.0);
                    }
                }
                BoardDecision::AdjustWageBudget { amount, .. } => {
                    if let Some(budget) = club.finance.wage_budget.as_mut() {
                        budget.amount = (budget.amount + *amount as f64).max(0.0);
                    }
                }
                BoardDecision::ApproveFacilityUpgrade { facility, cost } => {
                    Self::apply_facility_upgrade(club, *facility, *cost);
                }
                BoardDecision::CompleteTakeover => {
                    // New owner injects cash proportional to the club's wage
                    // bill — a war chest to back the fresh ambition.
                    let wages: u32 = club.teams.iter().map(|t| t.get_annual_salary()).sum();
                    let injection = (wages as i64).max(20_000_000);
                    club.finance.balance.push_income(injection);
                    info!(
                        "Takeover completed at {} — owner injects {}",
                        club.name, injection
                    );
                }
                // Informational / handled elsewhere.
                BoardDecision::IssueManagerBacking
                | BoardDecision::IssueFormalWarning
                | BoardDecision::HoldCrisisMeeting
                | BoardDecision::SackManager
                | BoardDecision::RejectFacilityUpgrade { .. }
                | BoardDecision::DemandPlayerSale { .. }
                | BoardDecision::BlockTransfer { .. }
                | BoardDecision::ApproveTransferException { .. }
                | BoardDecision::StartTakeoverRumour => {}
            }
        }
    }

    /// Bump the targeted facility one level (debiting the cost) or expand
    /// the stadium's capacity proxy. Costs draw down cash via the finance
    /// balance so the upgrade has a real budget consequence.
    fn apply_facility_upgrade(club: &mut Club, facility: BoardFacility, cost: i64) {
        let upgraded = match facility {
            BoardFacility::Training => Self::step_up(&mut club.facilities.training),
            BoardFacility::Youth => Self::step_up(&mut club.facilities.youth),
            BoardFacility::Academy => Self::step_up(&mut club.facilities.academy),
            BoardFacility::Recruitment => Self::step_up(&mut club.facilities.recruitment),
            BoardFacility::Stadium => {
                match Self::expanded_attendance(club.facilities.average_attendance) {
                    Some(next) => {
                        club.facilities.average_attendance = next;
                        true
                    }
                    // No real stadium/attendance model for this club — the
                    // expansion is a news-only announcement, so we must NOT
                    // debit cash for a change nothing can see.
                    None => false,
                }
            }
        };
        if upgraded {
            club.finance.balance.push_cash_outflow(cost.max(0));
            debug!(
                "Board approved {:?} upgrade at {} (cost {})",
                facility, club.name, cost
            );
        }
    }

    fn step_up(level: &mut FacilityLevel) -> bool {
        if let Some(next) = level.next_better() {
            *level = next;
            true
        } else {
            false
        }
    }

    /// Post-expansion average attendance for a stadium upgrade (~+15%), or
    /// `None` when there's no stadium model to change. `average_attendance`
    /// of 0 means "unmodelled" — expanding it would change nothing visible,
    /// so the caller must not debit cash for it.
    ///
    /// TODO: when real stadium capacity is modelled, key this off capacity
    /// rather than the average-attendance proxy.
    fn expanded_attendance(current: u32) -> Option<u32> {
        if current == 0 {
            None
        } else {
            Some(current + (current / 7).max(1))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stadium_expansion_is_news_only_when_unmodelled() {
        // No attendance model (0) → no state change → caller must not debit.
        assert_eq!(BoardResult::expanded_attendance(0), None);
    }

    #[test]
    fn stadium_expansion_grows_a_real_attendance() {
        let next = BoardResult::expanded_attendance(28_000).expect("modelled stadium expands");
        assert!(
            next > 28_000,
            "expansion should raise attendance, got {next}"
        );
    }
}
