use crate::club::StaffPosition;
use crate::club::board::manager;
use crate::club::board::sale::ForcedSale;
use crate::club::board::severance::Severance;
use crate::club::board::{BoardDecision, BoardFacility, BoardMoodState, DecisionReason};
use crate::club::facilities::FacilityLevel;
use crate::club::mind::organs::memory::{ActorRef, EpisodeKind};
use crate::club::news::ClubAffair;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::staff::SeparationCause;
use crate::league::result::LeagueProcessAccess;
use crate::{Club, HappinessEventType, Staff, StaffEventType, TeamType};
use chrono::{Datelike, NaiveDate};
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
    /// Promises whose deadline lapsed unfulfilled at this season turn.
    /// The trust penalty is applied inside the board; the count is
    /// carried out so the club can date the broken word for the press —
    /// "the board said there would be money" is a story, and it is one
    /// no persistent field can put a day on.
    pub promises_broken: u8,
    /// Promises the board delivered on this tick. Counted out for the
    /// same reason `promises_broken` is: the trust reward is applied
    /// inside the board, but only the club can put a day on it — and
    /// the manager's own read of whether these people keep their word
    /// is built out of dated events, not a running total.
    pub promises_kept: u8,
    /// A takeover the club had been living under fell through this tick.
    pub takeover_collapsed: bool,
}

impl Default for BoardResult {
    fn default() -> Self {
        Self::new()
    }
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
            promises_broken: 0,
            promises_kept: 0,
            takeover_collapsed: false,
        }
    }

    /// Tell the manager something the board did, `times` over.
    ///
    /// Wrapped rather than repeated at each site so a tap is one line
    /// and cannot forget the club id, which is what makes the episode
    /// cue-able by club a decade later.
    fn tell_the_manager(&self, club: &mut Club, today: NaiveDate, times: u8, kind: EpisodeKind) {
        if times == 0 {
            return;
        }
        let club_id = club.id;
        let Some(main_team) = club.teams.main_mut() else {
            return;
        };
        let Some(mgr) = main_team
            .staffs
            .find_mut_by_position(StaffPosition::Manager)
        else {
            return;
        };
        for _ in 0..times {
            mgr.remember(kind, ActorRef::board(club_id), today, club_id);
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
        let sacked_staff;
        // Mirror for confirm-new-manager: the appointment runs in
        // `manager_market::execute_appointment` after the club borrow
        // ends because it needs concurrent access to the pool.
        let do_confirm = self.confirm_new_manager;

        {
            let club = match data.club_mut(self.club_id) {
                Some(c) => c,
                None => return,
            };

            self.apply_board_affairs(club, today);
            self.apply_manager_satisfaction(club);
            self.apply_manager_meeting(club, today);
            self.apply_manager_renewal(club, today);
            self.apply_ultimatum(club, today);
            sacked_staff = self.apply_sacking(club, today);
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

    /// The boardroom's written record for this tick: decisions, promises,
    /// the manager's own memory of them, and a collapsed takeover.
    fn apply_board_affairs(&self, club: &mut Club, today: NaiveDate) {
        // Budget movements flow exclusively through `BoardDecision`
        // entries now — see `ClubBoard::emit_budget_decisions`. The
        // legacy `cut_transfer_budget` / `bonus_transfer_funds` flags
        // and the board mood are kept for the UI but no longer drive a
        // separate percentage tweak here (that double-applied with the
        // decision amounts). The applier is the single mutation
        // point for budgets, facility upgrades, and takeover injections.
        BoardDecisionApplier::new(club, today).apply_all(&self.decisions);

        // A promise the board made and did not keep. Recorded here
        // rather than inside the ledger because only the club can
        // date it, and a broken word is one of the few boardroom
        // stories a supporter hears about at all.
        for _ in 0..self.promises_broken {
            club.record_affair(ClubAffair::PromiseBroken, today);
        }

        // The manager's side of the same morning. `PromiseLedger`
        // has always recorded what the board pledged; nothing until
        // now recorded whether the man they pledged it to still
        // believes them.
        self.tell_the_manager(
            club,
            today,
            self.promises_broken,
            EpisodeKind::BoardBrokeItsPromise,
        );
        self.tell_the_manager(
            club,
            today,
            self.promises_kept,
            EpisodeKind::BoardKeptItsPromise,
        );
        if self.takeover_collapsed {
            club.record_affair(ClubAffair::TakeoverCollapsed, today);
        }
    }

    /// Push the board's mood onto the manager's own job satisfaction —
    /// a coach at a happy club feels secure, a coach under Poor mood
    /// feels the pressure building. Applied after the sacking path so
    /// we don't adjust a seat that's just been vacated.
    fn apply_manager_satisfaction(&self, club: &mut Club) {
        if self.manager_satisfaction_delta.abs() > 0.01
            && !self.manager_sacked
            && let Some(main_team) = club.teams.main_mut()
            && let Some(mgr) = main_team
                .staffs
                .find_mut_by_position(StaffPosition::Manager)
        {
            mgr.job_satisfaction =
                (mgr.job_satisfaction + self.manager_satisfaction_delta).clamp(0.0, 100.0);
        }
    }

    fn apply_manager_meeting(&self, club: &mut Club, today: NaiveDate) {
        if let Some(meeting) = self.manager_meeting {
            let club_id = self.club_id;
            if let Some(main_team) = club.teams.main_mut()
                && let Some(mgr) = main_team
                    .staffs
                    .find_mut_by_position(StaffPosition::Manager)
            {
                let event = match meeting {
                    BoardManagerMeeting::Backing => StaffEventType::TrustBuilt,
                    BoardManagerMeeting::Warning => StaffEventType::PerformanceDeclined,
                    BoardManagerMeeting::Crisis => StaffEventType::Conflict,
                };
                mgr.add_event(event);

                // A public backing is not a kindness. In football
                // it is what a board says on the way to sacking
                // someone, and a manager who has been sacked
                // before reads it that way immediately — see
                // `AmbitionMind::cynicism`.
                if matches!(meeting, BoardManagerMeeting::Backing) {
                    mgr.remember(
                        EpisodeKind::GivenAVoteOfConfidence,
                        ActorRef::board(club_id),
                        today,
                        club_id,
                    );
                }
            }
        }
    }

    /// Season-start renewal: if the board wants to keep the manager,
    /// extend the contract by two years and give a salary bump. The
    /// manager is trusted, so the terms are friendly; this prevents
    /// a successful coach from running down their deal and walking
    /// for free. Only fires when the current contract is short enough
    /// to genuinely be at risk (≤18 months out).
    fn apply_manager_renewal(&self, club: &mut Club, today: NaiveDate) {
        if self.offer_manager_renewal && !self.manager_sacked {
            let mut extended: Option<u32> = None;
            let club_id = self.club_id;
            let club_name = club.name.clone();
            if let Some(main_team) = club.teams.main_mut()
                && let Some(mgr) = main_team
                    .staffs
                    .find_mut_by_position(StaffPosition::Manager)
            {
                let should_offer = mgr
                    .contract
                    .as_ref()
                    .map(|c| (c.expired - today).num_days() < 540)
                    .unwrap_or(true);
                if should_offer && let Some(contract) = mgr.contract.as_mut() {
                    let new_expires = today
                        .with_year(today.year() + 2)
                        .unwrap_or(contract.expired);
                    if new_expires > contract.expired {
                        contract.expired = new_expires;
                    }
                    contract.salary = ((contract.salary as f32) * 1.15) as u32;
                    mgr.job_satisfaction = (mgr.job_satisfaction + 10.0).clamp(0.0, 100.0);
                    extended = Some(mgr.id);
                    mgr.remember(
                        EpisodeKind::ContractRenewed,
                        ActorRef::board(club_id),
                        today,
                        club_id,
                    );
                    info!(
                        "Board offered renewal (+2y, +15% salary) to manager {} at {}",
                        mgr.id, club_name
                    );
                }
            }
            if let Some(staff_id) = extended {
                club.record_affair(ClubAffair::ManagerContractExtended { staff_id }, today);
            }
        }
    }

    /// Ultimatum made public this tick: the squad reads the
    /// situation through each player's own bond with the head
    /// coach — loyalists rally to save his job, the rest sense a
    /// change coming (and hold their pens on new deals via the
    /// mood's morale drag). Skipped when a total confidence
    /// collapse sacks the manager the same tick.
    fn apply_ultimatum(&self, club: &mut Club, today: NaiveDate) {
        if self.manager_ultimatum_announced && !self.manager_sacked {
            let mut warned: u32 = 0;
            let club_id = self.club_id;
            if let Some(main_team) = club.teams.main_mut() {
                let coach_id = main_team.staffs.head_coach().id;
                warned = coach_id;
                // Being put on final warning in public. The squad
                // reacts below; this is the manager's own record of
                // the morning.
                if let Some(mgr) = main_team.staffs.head_coach_mut() {
                    mgr.remember(
                        EpisodeKind::ChairmanUndercutMePublicly,
                        ActorRef::board(club_id),
                        today,
                        club_id,
                    );
                }
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
            // The one rung of the sacking ladder a supporter gets to
            // live through in advance, so the paper prints it as its
            // own week rather than as hindsight after the axe.
            club.record_affair(ClubAffair::ManagerUltimatum { staff_id: warned }, today);
        }
    }

    /// Sacking: terminate the manager contract on the main team and
    /// promote the best available coaching-staff member to caretaker.
    /// The caretaker runs the team until the 30-day search concludes
    /// (see `confirm_new_manager`). The sacked staff member is
    /// *removed* from the team's roster (not just stripped of contract)
    /// and returned for the global free-agent pool.
    fn apply_sacking(&self, club: &mut Club, today: NaiveDate) -> Option<Staff> {
        if !self.manager_sacked {
            return None;
        }

        let club_name = club.name.clone();
        let outcome = self.dismiss_manager(club, today, &club_name);
        self.record_sacking(club, today, &club_name, &outcome);
        self.open_manager_search(club, today);
        outcome.staff
    }

    /// Take the manager off the main team, settle what ending the deal
    /// early costs, and install the best available caretaker.
    fn dismiss_manager(
        &self,
        club: &mut Club,
        today: NaiveDate,
        club_name: &str,
    ) -> SackingOutcome {
        let mut outcome = SackingOutcome::default();
        // What ending the deal early costs, settled before the man
        // leaves the roster — his contract goes with him.
        let severance_share = club.board.ownership.ownership_type.severance_share();
        if let Some(main_team) = club.teams.main_mut() {
            let mut sacked_salary: u32 = 0;
            if let Some(mut staff) = main_team.staffs.take_by_position(StaffPosition::Manager) {
                let id = staff.id;
                if let Some(c) = &staff.contract {
                    sacked_salary = c.salary;
                    let owed = Severance::owed(c.salary, c.expired, today, severance_share);
                    if owed > 0 {
                        outcome.severance = Some((id, owed));
                    }
                }
                info!(
                    "Board sacked manager (staff id {}) at {} — confidence {}",
                    id, club_name, self.confidence
                );

                // The flashbulb of a manager's career. Recorded
                // before the spell is closed out, so it is filed
                // against the club he was still at — which is
                // what makes `TheySackedMe` about the right
                // badge a decade later. It also forms
                // `ProveThemWrong`, the one want in the catalog
                // that only exists because memory does.
                staff.remember(
                    EpisodeKind::SackedByClub,
                    ActorRef::club(self.club_id),
                    today,
                    self.club_id,
                );
                // And every working relationship he had here ends
                // with it. Being sacked is not the players' doing
                // and the record says so — the partings are filed
                // against the club rather than against them.
                staff.leave_club_as(self.club_id, SeparationCause::IWasSacked, today);

                outcome.dismissed = Some(id);
                outcome.staff = Some(staff);
            }

            let installed =
                manager::ManagerSeat::promote_best_caretaker(main_team, sacked_salary, today);
            if installed {
                outcome.caretaker = main_team
                    .staffs
                    .find_by_position(StaffPosition::CaretakerManager)
                    .map(|staff| staff.id);
                debug!("Caretaker promoted at {} after sacking", club_name);
            }
        }

        outcome
    }

    /// File the morning's stories and pay the bill for a dismissal.
    fn record_sacking(
        &self,
        club: &mut Club,
        today: NaiveDate,
        club_name: &str,
        outcome: &SackingOutcome,
    ) {
        // Two separate pieces of news on the same morning: the
        // man who went, and the man who has to pick the team on
        // Saturday. The press used to be able to tell neither
        // apart from a poach or a permanent appointment.
        if let Some(staff_id) = outcome.dismissed {
            club.record_affair(ClubAffair::ManagerSacked { staff_id }, today);
        }
        // The bill. Cash out, and a story: a club that sacks three
        // managers in two years is a club whose accounts say so.
        if let Some((staff_id, amount)) = outcome.severance {
            club.finance.balance.push_cash_outflow(amount);
            club.record_affair(ClubAffair::SeverancePaid { staff_id, amount }, today);
            info!(
                "Severance of {} paid to sacked manager {} at {}",
                amount, staff_id, club_name
            );
        }
        if let Some(staff_id) = outcome.caretaker {
            club.record_affair(ClubAffair::CaretakerAppointed { staff_id }, today);
        }
    }

    /// Start the search clock on the board, locking in the rep-scaled
    /// search window. Top clubs hunt for ~60 days, small clubs ~21. The
    /// window stays stable across the search even if reputation
    /// fluctuates.
    fn open_manager_search(&self, club: &mut Club, today: NaiveDate) {
        let club_rep = club
            .teams
            .iter()
            .find(|t| matches!(t.team_type, TeamType::Main))
            .map(|t| t.reputation.world)
            .unwrap_or(0);
        manager::ManagerSearch::open(&mut club.board, today, club_rep);
    }

    /// Post-expansion ground capacity for a stadium upgrade (~+15%), or
    /// `None` when there's no stadium to expand. A capacity of 0 means
    /// "unmodelled" — expanding it would change nothing visible, so the
    /// caller must not debit cash for it.
    fn expanded_capacity(current: u32) -> Option<u32> {
        if current == 0 {
            None
        } else {
            Some(current + (current / 7).max(1))
        }
    }
}

/// What one dismissal took off the main team, and what it will cost.
#[derive(Default)]
struct SackingOutcome {
    dismissed: Option<u32>,
    severance: Option<(u32, i64)>,
    caretaker: Option<u32>,
    staff: Option<Staff>,
}

/// Apply the board decisions that have concrete club-state effects:
/// transfer/wage budget adjustments, approved facility upgrades, and a
/// takeover cash injection. Other variants (meetings, sackings, search,
/// rumours, demands) are informational or handled by legacy fields.
struct BoardDecisionApplier<'a> {
    club: &'a mut Club,
    today: NaiveDate,
}

impl<'a> BoardDecisionApplier<'a> {
    fn new(club: &'a mut Club, today: NaiveDate) -> Self {
        Self { club, today }
    }

    fn apply_all(&mut self, decisions: &[BoardDecision]) {
        for decision in decisions {
            self.apply(decision);
        }
    }

    fn apply(&mut self, decision: &BoardDecision) {
        match decision {
            BoardDecision::IncreaseTransferBudget { amount, .. } => {
                self.increase_transfer_budget(*amount);
            }
            BoardDecision::CutTransferBudget { amount, .. } => {
                self.cut_transfer_budget(*amount);
            }
            BoardDecision::AdjustWageBudget { amount, .. } => {
                self.adjust_wage_budget(*amount);
            }
            BoardDecision::OwnerTopUp { amount, .. } => self.owner_top_up(*amount),
            BoardDecision::ApproveFacilityUpgrade { facility, cost } => {
                self.approve_facility_upgrade(*facility, *cost);
            }
            BoardDecision::StartTakeoverRumour => {
                self.club
                    .record_affair(ClubAffair::TakeoverRumour, self.today);
            }
            BoardDecision::CompleteTakeover => self.complete_takeover(),
            // Four decisions the board really takes, which used to
            // be discarded here and so left no trace anywhere a
            // reader could see. Each of them is a morning a
            // supporter would have heard about from somebody at the
            // club, so each of them now reaches the diary the
            // boardroom desk reads.
            BoardDecision::HoldCrisisMeeting => {
                self.club
                    .record_affair(ClubAffair::CrisisMeetingHeld, self.today);
            }
            BoardDecision::DemandPlayerSale { reason } => self.demand_player_sale(*reason),
            BoardDecision::BlockTransfer { player_id, .. } => {
                self.club.record_affair(
                    ClubAffair::TransferBlocked {
                        player_id: *player_id,
                    },
                    self.today,
                );
            }
            BoardDecision::RejectFacilityUpgrade { facility, .. } => {
                self.club.record_affair(
                    ClubAffair::FacilityUpgradeRejected {
                        facility: *facility,
                    },
                    self.today,
                );
            }

            // Informational / handled elsewhere. The backing and the
            // warning reach the page through the board's confidence
            // state and the ultimatum affair; the sacking is
            // recorded by the manager market, which knows who.
            BoardDecision::IssueManagerBacking
            | BoardDecision::IssueFormalWarning
            | BoardDecision::SackManager
            | BoardDecision::ApproveTransferException { .. } => {}
        }
    }

    /// Money the board moved. Filed for the press only when a budget
    /// actually existed to move — a club with no transfer budget modelled
    /// has had nothing put on or taken off its table, and "the board
    /// release $12m" for a sum that changed no figure anywhere is the same
    /// invented number the press has been caught printing before.
    fn increase_transfer_budget(&mut self, amount: i64) {
        // Written to the MANDATE as well as to the live budget.
        // The live figure is rebuilt from the mandate at every
        // monthly recompute, so money added only here lasted
        // until the next month and then evaporated.
        if let Some(targets) = self.club.board.season_targets.as_mut() {
            targets.mandate_adjustment += amount;
        }
        if let Some(budget) = self.club.finance.transfer_budget.as_mut() {
            budget.amount += amount as f64;
            self.club
                .record_affair(ClubAffair::WarChest { amount }, self.today);
        }
        debug!(
            "Board raised transfer budget at {} by {}",
            self.club.name, amount
        );
    }

    fn cut_transfer_budget(&mut self, amount: i64) {
        if let Some(targets) = self.club.board.season_targets.as_mut() {
            targets.mandate_adjustment -= amount;
        }
        if let Some(budget) = self.club.finance.transfer_budget.as_mut() {
            let before = budget.amount;
            budget.amount = (budget.amount - amount as f64).max(0.0);
            // A cut against an already-empty budget takes
            // nothing off the table and is not a story.
            let taken = before - budget.amount;
            if taken > 0.0 {
                self.club.record_affair(
                    ClubAffair::BudgetCut {
                        amount: taken as i64,
                    },
                    self.today,
                );
            }
        }
    }

    fn adjust_wage_budget(&mut self, amount: i64) {
        if let Some(targets) = self.club.board.season_targets.as_mut() {
            targets.wage_mandate_adjustment += amount;
        }
        if let Some(budget) = self.club.finance.wage_budget.as_mut() {
            budget.amount = (budget.amount + amount as f64).max(0.0);
        }
    }

    fn owner_top_up(&mut self, amount: i64) {
        // Owner funding, not revenue: the cash arrives on the
        // balance sheet without ever entering the P&L, so
        // next season's revenue-derived budgets and the
        // market's inflation read are untouched by it.
        self.club.finance.balance.push_owner_investment(amount);
        self.club
            .record_affair(ClubAffair::OwnerBailout { amount }, self.today);
    }

    fn complete_takeover(&mut self) {
        self.club
            .record_affair(ClubAffair::TakeoverCompleted, self.today);
        // New owner injects cash proportional to the club's wage
        // bill — a war chest to back the fresh ambition.
        let wages: u32 = self.club.teams.iter().map(|t| t.get_annual_salary()).sum();
        let injection = (wages as i64).max(20_000_000);
        self.club.finance.balance.push_income(injection);
        info!(
            "Takeover completed at {} — owner injects {}",
            self.club.name, injection
        );
    }

    fn demand_player_sale(&mut self, reason: DecisionReason) {
        // A demand names somebody. It used to record a headline
        // and stop: no player listed, no price, no deadline, and
        // nothing anywhere that noticed whether the money came
        // in — so a board could demand a sale every month of a
        // crisis and the squad never lost anybody.
        let (league_rep, club_rep) = self
            .club
            .teams
            .main()
            .map(|t| (t.reputation.national, t.reputation.world))
            .unwrap_or((5_000, 5_000));
        let today = self.today;
        if let Some(target) = ForcedSale::execute(self.club, reason, today, league_rep, club_rep) {
            self.club.board.open_sale_mandate(
                target,
                self.club.finance.season_fees.received,
                today,
            );
            self.club.record_affair(
                ClubAffair::PlayerSaleDemanded {
                    player_id: target.player_id,
                },
                today,
            );
        }
    }

    fn approve_facility_upgrade(&mut self, facility: BoardFacility, cost: i64) {
        // Only a change somebody can see is news. An upgrade
        // that found nothing to upgrade debits no cash and
        // files no story.
        if let Some(affair) = self.apply_facility_upgrade(facility, cost) {
            self.club.record_affair(affair, self.today);
        }
    }

    /// Bump the targeted facility one level (debiting the cost) or expand
    /// the stadium's capacity proxy. Costs draw down cash via the finance
    /// balance so the upgrade has a real budget consequence.
    ///
    /// Returns the affair to file when something actually changed — the
    /// ground genuinely got bigger, or a facility genuinely moved up a
    /// level. An approval that found nothing left to upgrade changes
    /// nothing, costs nothing, and is not news.
    fn apply_facility_upgrade(&mut self, facility: BoardFacility, cost: i64) -> Option<ClubAffair> {
        let mut affair = Some(ClubAffair::FacilityUpgrade { facility });
        let upgraded = match facility {
            BoardFacility::Training => Self::step_up(&mut self.club.facilities.training),
            BoardFacility::Youth => Self::step_up(&mut self.club.facilities.youth),
            BoardFacility::Academy => Self::step_up(&mut self.club.facilities.academy),
            BoardFacility::Recruitment => Self::step_up(&mut self.club.facilities.recruitment),
            BoardFacility::Stadium => {
                // Expansion now grows the ground itself. Capacity is the
                // permanent property; average attendance is an observed
                // outcome the revenue model rewrites each month, so bumping
                // it here would be overwritten within a tick.
                let rep = self
                    .club
                    .teams
                    .main()
                    .map(|t| t.reputation.overall_score())
                    .unwrap_or(0.0);
                let current = self.club.facilities.capacity_or_estimate(rep);
                match BoardResult::expanded_capacity(current) {
                    Some(next) => {
                        self.club.facilities.stadium_capacity = next;
                        affair = Some(ClubAffair::StadiumExpansion { capacity: next });
                        true
                    }
                    // No stadium model to change — the expansion is a
                    // news-only announcement, so we must NOT debit cash for
                    // a change nothing can see.
                    None => false,
                }
            }
        };
        if !upgraded {
            return None;
        }
        self.club.finance.balance.push_cash_outflow(cost.max(0));
        debug!(
            "Board approved {:?} upgrade at {} (cost {})",
            facility, self.club.name, cost
        );
        affair
    }

    fn step_up(level: &mut FacilityLevel) -> bool {
        if let Some(next) = level.next_better() {
            *level = next;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stadium_expansion_is_news_only_when_unmodelled() {
        // No stadium model (0) → no state change → caller must not debit.
        assert_eq!(BoardResult::expanded_capacity(0), None);
    }

    #[test]
    fn stadium_expansion_grows_a_real_capacity() {
        let next = BoardResult::expanded_capacity(28_000).expect("modelled stadium expands");
        assert!(next > 28_000, "expansion should raise capacity, got {next}");
    }
}
