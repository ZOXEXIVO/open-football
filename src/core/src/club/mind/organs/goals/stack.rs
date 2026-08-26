//! The goal stack — everything a player currently wants, and the
//! periodic think that keeps it honest.
//!
//! Twelve slots. That is not a limit imposed for storage (though it
//! costs 244 bytes and allocates nothing); it is the point. A man
//! chasing twelve things is chasing none of them, and the competition
//! rules mean the strong wants actively suppress the ones that
//! contradict them.

use super::catalog::{GoalDirection, GoalKind};
use super::escalation::{Escalation, StatusChange};
use super::evidence::{GoalBlocker, GoalDomain, GoalEvidence, GoalOrigin};
use super::goal::{GoalStatus, MindGoal};
use crate::club::mind::organs::memory::{EpochDay, FixedStore, MindClock};
use std::cmp::Ordering;

/// Everything he currently wants.
pub type GoalStore = FixedStore<MindGoal, 12>;

/// The stack, plus the clock for its periodic review.
#[derive(Debug, Clone, Copy, Default)]
pub struct GoalStack {
    goals: GoalStore,
    /// Last time the stack was reviewed. Review is weekly; this gates it
    /// without a separate scheduler.
    pub last_reviewed: EpochDay,
}

/// A want that appeared, and the day it did.
///
/// The day is carried rather than assumed to be the review's, because a
/// want is formed by whichever emit site noticed the thing that prompted
/// it — any day of the week — while the review that *reports* it runs on
/// Mondays. A reader dating them all to the review would be guessing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FormedWant {
    pub kind: GoalKind,
    pub day: EpochDay,
}

/// What one review pass changed. Returned for the census harness, the
/// event feed, and the tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GoalReviewReport {
    pub reviewed: u16,
    /// Goals that climbed a rung.
    pub escalated: u16,
    /// Goals that fell back.
    pub eased: u16,
    pub satisfied: u16,
    pub frustrated: u16,
    pub abandoned: u16,
    /// Goals newly said out loud this pass. The emit hook for mood
    /// events once the sub-minds land.
    pub newly_voiced: u16,
    /// Goals newly escalated to a formal demand.
    pub newly_pressing: u16,
    /// Wants that appeared this pass.
    formed: [FormedWant; Self::MAX_LISTED],
    formed_count: u8,
    /// Rungs walked this pass.
    changes: [StatusChange; Self::MAX_LISTED],
    change_count: u8,
}

impl GoalReviewReport {
    /// How many turns of each sort the report *names*. The counts above
    /// stay authoritative — a pass that turned more goals than this
    /// still counts every one of them, it just cannot list them all.
    ///
    /// Fixed-capacity and `Copy` because the report crosses the tick
    /// boundary by value: a `Vec` here would put a heap allocation into
    /// the weekly think of every player in the world.
    pub const MAX_LISTED: usize = 6;

    /// The wants that appeared this pass, in stack order.
    pub fn formed(&self) -> impl Iterator<Item = FormedWant> + '_ {
        self.formed[..self.formed_count as usize].iter().copied()
    }

    /// The rungs walked this pass, in the order they were decided.
    pub fn changes(&self) -> impl Iterator<Item = StatusChange> + '_ {
        self.changes[..self.change_count as usize].iter().copied()
    }

    fn push_formed(&mut self, kind: GoalKind, day: EpochDay) {
        if (self.formed_count as usize) < Self::MAX_LISTED {
            self.formed[self.formed_count as usize] = FormedWant { kind, day };
            self.formed_count += 1;
        }
    }

    fn push_change(&mut self, change: StatusChange) {
        if (self.change_count as usize) < Self::MAX_LISTED {
            self.changes[self.change_count as usize] = change;
            self.change_count += 1;
        }
    }
}

impl GoalStack {
    /// Days between reviews. Weekly, matching the tick that already
    /// exists for happiness.
    pub const REVIEW_PERIOD_DAYS: u16 = 7;

    /// How much a goal weakens the goals it competes with, per review,
    /// scaled by its own pressure. Deliberately gentle — a man who has
    /// decided to stay does not instantly stop wanting to go, he stops
    /// wanting it over a season.
    pub const COMPETITION_PRESSURE: f32 = 0.08;

    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.goals.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.goals.is_empty()
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &MindGoal> {
        self.goals.iter()
    }

    /// Only the goals still in play.
    pub fn live(&self) -> impl Iterator<Item = &MindGoal> {
        self.goals.iter().filter(|g| g.is_live())
    }

    /// Borrow one goal.
    #[inline]
    pub fn get(&self, kind: GoalKind) -> Option<&MindGoal> {
        self.goals.find(|g| g.kind == kind && g.is_live())
    }

    #[inline]
    pub fn get_mut(&mut self, kind: GoalKind) -> Option<&mut MindGoal> {
        self.goals.find_mut(|g| g.kind == kind && g.is_live())
    }

    /// Does he hold this want at all, and how hard does it press?
    /// 0.0 when absent — the read every decision site wants.
    pub fn pressure_of(&self, kind: GoalKind) -> f32 {
        self.get(kind).map(|g| g.pressure()).unwrap_or(0.0)
    }

    /// Where this want sits on the ladder. [`GoalStatus::Abandoned`] when
    /// he does not hold it, since that reads correctly at every call
    /// site: not live, not public, not shaping anything.
    pub fn status_of(&self, kind: GoalKind) -> GoalStatus {
        self.get(kind)
            .map(|g| g.status)
            .unwrap_or(GoalStatus::Abandoned)
    }

    /// Want something, or want it more.
    ///
    /// The single write the whole simulation uses. Forms the goal if it
    /// is new, strengthens it if it is not, and folds in whatever signal
    /// prompted the call. `amount` is 0..1 — how much this particular
    /// piece of evidence should move him.
    ///
    /// Returns `true` if the goal was newly formed.
    pub fn pursue(
        &mut self,
        kind: GoalKind,
        origin: GoalOrigin,
        evidence: GoalEvidence,
        amount: f32,
        today: EpochDay,
    ) -> bool {
        if kind == GoalKind::None {
            return false;
        }

        if let Some(existing) = self.get_mut(kind) {
            existing.reinforce(amount, evidence, today);
            return false;
        }

        let mut goal = MindGoal::new(kind, origin, evidence, today);
        // The prompting evidence counts as the first reinforcement, so a
        // strong signal forms a stronger want than a passing one.
        goal.reinforce(amount, evidence, today);

        // A new want displaces the least worth keeping — resolved goals
        // first, then the weakest live one.
        self.goals
            .push_evicting(goal, |existing| existing.keep_rank(), |_| true);
        true
    }

    /// Something took the heat out of a want without answering it.
    ///
    /// Distinct from [`Self::advance`], and the difference is the point:
    /// advancing says the want is closer to being *met*, easing says he
    /// simply minds less for now. A new manager arriving does the second
    /// to a man who wanted away — nothing about his situation has
    /// changed, and he is willing to wait and see.
    pub fn ease(&mut self, kind: GoalKind, amount: f32) {
        if let Some(goal) = self.get_mut(kind) {
            goal.yield_to_competition(amount);
        }
    }

    /// Move a goal toward being satisfied.
    pub fn advance(&mut self, kind: GoalKind, amount: f32) {
        if let Some(goal) = self.get_mut(kind) {
            goal.advance(amount);
        }
    }

    /// Set how much the timing presses on a goal — a closing window, an
    /// expiring contract, years running out.
    pub fn set_urgency(&mut self, kind: GoalKind, urgency: f32) {
        if let Some(goal) = self.get_mut(kind) {
            goal.set_urgency(urgency);
        }
    }

    /// Record that he cannot act on this one right now. A blocked goal
    /// keeps its strength and goes on colouring his mood — it simply has
    /// nowhere to go.
    pub fn block(&mut self, kind: GoalKind, blocker: GoalBlocker) {
        if let Some(goal) = self.get_mut(kind) {
            goal.blocked_by = blocker;
        }
    }

    /// Give himself until a date on a goal he holds. "I'll see how the
    /// first half of the season goes."
    pub fn commit_until(&mut self, kind: GoalKind, deadline: EpochDay) {
        if let Some(goal) = self.get_mut(kind) {
            goal.commit_until(deadline);
        }
    }

    /// Drop a goal outright — he got what he wanted, or the world
    /// changed under it. `satisfied` decides whether it resolves as an
    /// achievement or as something he let go.
    pub fn resolve(&mut self, kind: GoalKind, satisfied: bool) {
        if let Some(goal) = self.get_mut(kind) {
            goal.status = if satisfied {
                goal.set_progress(1.0);
                GoalStatus::Satisfied
            } else {
                GoalStatus::Abandoned
            };
        }
    }

    /// The want that dominates him right now, if any. What the narrative
    /// layers should lead with.
    pub fn strongest(&self) -> Option<&MindGoal> {
        self.live().max_by(|a, b| {
            a.pressure()
                .partial_cmp(&b.pressure())
                .unwrap_or(Ordering::Equal)
        })
    }

    /// The strongest want in one domain — what the career mind, the
    /// financial mind and so on each lead with.
    pub fn strongest_in(&self, domain: GoalDomain) -> Option<&MindGoal> {
        self.live()
            .filter(|g| g.kind.domain() == domain)
            .max_by(|a, b| {
                a.pressure()
                    .partial_cmp(&b.pressure())
                    .unwrap_or(Ordering::Equal)
            })
    }

    /// Net pull away from the club, 0..1.
    ///
    /// The read the transfer market wants, and the thing
    /// `big_stage_inclination` did for exactly one want. Sums what points
    /// out, nets off what points in, and counts goals he has *not* said
    /// out loud — well below the level that produces a request, a player
    /// will still listen when the right club calls.
    pub fn wants_to_leave(&self) -> f32 {
        let mut out = 0.0f32;
        let mut stay = 0.0f32;
        for goal in self.live().filter(|g| g.status.shapes_decisions()) {
            match goal.kind.direction() {
                GoalDirection::Leave => out = out.max(goal.pressure()),
                GoalDirection::Stay => stay = stay.max(goal.pressure()),
                GoalDirection::Neutral => {}
            }
        }
        (out - stay * 0.7).clamp(0.0, 1.0)
    }

    /// Has he formally demanded anything? The `Req` signal.
    pub fn is_pressing(&self) -> bool {
        self.live().any(|g| g.status == GoalStatus::Pressing)
    }

    /// Every want he has said out loud or demanded — what the manager,
    /// the press and the UI can see.
    pub fn public_goals(&self) -> impl Iterator<Item = &MindGoal> {
        self.live().filter(|g| g.status.is_public())
    }

    /// How much an event of this character bears on what he currently
    /// wants, 0..1, with 0.5 meaning "nothing in particular".
    ///
    /// This is what makes memory goal-aware: being left out brands
    /// itself on a man whose whole ambition is first-team football and
    /// barely registers for a settled veteran. Fed straight into
    /// [`EncodingInputs::relevance`].
    ///
    /// [`EncodingInputs::relevance`]: crate::club::mind::organs::memory::EncodingInputs::relevance
    pub fn relevance_of(&self, domain: GoalDomain) -> f32 {
        let strongest = self
            .live()
            .filter(|g| g.kind.domain() == domain)
            .map(|g| g.pressure())
            .fold(0.0f32, f32::max);
        // Neutral at 0.5, rising to 1.0 when a dominant want in this
        // domain is at full pressure.
        (0.5 + strongest * 0.5).clamp(0.0, 1.0)
    }

    /// The weekly think. Decays what nothing feeds, lets the strong
    /// wants suppress what contradicts them, then walks each goal one
    /// rung. Cheap no-op between reviews, so it is safe to call daily.
    pub fn review(&mut self, today: EpochDay) -> Option<GoalReviewReport> {
        if MindClock::elapsed(self.last_reviewed, today) < Self::REVIEW_PERIOD_DAYS {
            return None;
        }
        // Days since the previous review — the span decay is applied
        // over. Zero on the first review of a fresh stack, whose
        // `last_reviewed` is the epoch rather than a real date.
        let elapsed = if self.last_reviewed == 0 {
            0
        } else {
            MindClock::elapsed(self.last_reviewed, today)
        };
        let previously_reviewed = self.last_reviewed;
        self.last_reviewed = today;

        let mut report = GoalReviewReport::default();

        // Wants that appeared since the last think, named before
        // anything moves them. A want formed and voiced in the same pass
        // reports both turns, in the order they actually happened —
        // which is what a diary of them has to preserve.
        //
        // The window is the same exactly-once discipline the mood sweep
        // uses: strictly *after* the previous review, so a want cannot
        // be announced twice, and a fresh stack (`last_reviewed` at the
        // epoch) announces what it already holds at each want's own
        // formation date rather than pretending they all began today.
        for goal in self.goals.iter() {
            if goal.is_live() && goal.formed_on > previously_reviewed {
                report.push_formed(goal.kind, goal.formed_on);
            }
        }

        for goal in self.goals.iter_mut() {
            if goal.is_live() {
                goal.decay(goal.kind.spec().decay_per_month, elapsed);
            }
        }
        self.apply_competition();

        for goal in self.goals.iter_mut() {
            if !goal.is_live() {
                continue;
            }
            report.reviewed += 1;
            let Some(change) = Escalation::review(goal, today) else {
                continue;
            };
            match change.to {
                GoalStatus::Satisfied => report.satisfied += 1,
                GoalStatus::Frustrated => report.frustrated += 1,
                GoalStatus::Abandoned => report.abandoned += 1,
                _ => {
                    if change.to.rung() > change.from.rung() {
                        report.escalated += 1;
                    } else {
                        report.eased += 1;
                    }
                }
            }
            if change.to == GoalStatus::Voiced && change.from.rung() < GoalStatus::Voiced.rung() {
                report.newly_voiced += 1;
            }
            if change.to == GoalStatus::Pressing {
                report.newly_pressing += 1;
            }
            report.push_change(change);
        }

        // Resolved goals stay one pass so the caller can read the report,
        // then make room.
        self.goals
            .retain(|g| g.is_live() || g.status == GoalStatus::Satisfied);

        Some(report)
    }

    /// Strong wants suppress the ones that contradict them. Applied
    /// before escalation so a man who has just decided to stay does not
    /// also voice a demand to leave in the same pass.
    fn apply_competition(&mut self) {
        // Collect the suppressive force each live goal exerts, then
        // apply. Two passes because a goal cannot borrow the stack while
        // the stack is being mutated — and a fixed 12 slots makes the
        // copy free.
        let mut suppressors: [(GoalKind, f32); 12] = [(GoalKind::None, 0.0); 12];
        let mut count = 0usize;
        for goal in self.goals.iter() {
            if !goal.is_live() || goal.kind.spec().competes_with.is_empty() {
                continue;
            }
            suppressors[count] = (goal.kind, goal.pressure());
            count += 1;
        }

        for (kind, pressure) in suppressors.iter().take(count) {
            let mask = kind.spec().competes_with;
            let force = pressure * Self::COMPETITION_PRESSURE;
            if force <= 0.0 {
                continue;
            }
            for goal in self.goals.iter_mut() {
                if goal.is_live() && goal.kind != *kind && mask.contains(goal.kind) {
                    goal.yield_to_competition(force);
                }
            }
        }
    }

    /// Called when the player changes club. Wants that were about *this*
    /// club are met or moot; wants about himself travel with him.
    ///
    /// Deliberately not a blanket clear. A man who moved to get first-team
    /// football still wants first-team football, and if the new club does
    /// not give it to him the want is already there rather than having to
    /// be rediscovered from scratch.
    pub fn on_club_change(&mut self) {
        for goal in self.goals.iter_mut() {
            if !goal.is_live() {
                continue;
            }
            match goal.kind.direction() {
                // He got out. Whatever he wanted out *of* is answered.
                GoalDirection::Leave => {
                    goal.set_progress(1.0);
                    goal.status = GoalStatus::Satisfied;
                }
                // Wants that were about staying somewhere he no longer is.
                GoalDirection::Stay => {
                    goal.status = GoalStatus::Abandoned;
                }
                // Being underpaid, wanting a trophy, missing home — none
                // of that is settled by changing employer. It travels,
                // softened: a fresh start does take some heat out.
                GoalDirection::Neutral => {
                    goal.yield_to_competition(0.35);
                    goal.blocked_by = GoalBlocker::JustArrived;
                }
            }
        }
        self.goals.retain(|g| g.is_live());
    }

    /// Census for the `.dev/mind` harness and the player profile.
    pub fn census(&self) -> GoalCensus {
        let mut census = GoalCensus::default();
        for goal in self.goals.iter() {
            match goal.status {
                GoalStatus::Latent => census.latent += 1,
                GoalStatus::Active => census.active += 1,
                GoalStatus::Voiced => census.voiced += 1,
                GoalStatus::Pressing => census.pressing += 1,
                GoalStatus::Satisfied => census.satisfied += 1,
                GoalStatus::Frustrated => census.frustrated += 1,
                GoalStatus::Abandoned => census.abandoned += 1,
            }
        }
        census
    }
}

/// What the stack currently holds, by rung.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GoalCensus {
    pub latent: u16,
    pub active: u16,
    pub voiced: u16,
    pub pressing: u16,
    pub satisfied: u16,
    pub frustrated: u16,
    pub abandoned: u16,
}

impl GoalCensus {
    /// Goals still in play.
    pub fn live(&self) -> u16 {
        self.latent + self.active + self.voiced + self.pressing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: EpochDay = 10_000;

    fn feed(stack: &mut GoalStack, kind: GoalKind, times: usize, today: EpochDay) {
        for _ in 0..times {
            stack.pursue(kind, GoalOrigin::SelfDrive, GoalEvidence::EMPTY, 1.0, today);
        }
    }

    /// Run enough weekly reviews for the ladder to settle, feeding the
    /// goal each week so it does not simply decay.
    fn weeks(stack: &mut GoalStack, kind: GoalKind, count: u16, from: EpochDay) -> EpochDay {
        weeks_as(stack, kind, GoalOrigin::SelfDrive, count, from)
    }

    fn weeks_as(
        stack: &mut GoalStack,
        kind: GoalKind,
        origin: GoalOrigin,
        count: u16,
        from: EpochDay,
    ) -> EpochDay {
        let mut day = from;
        for _ in 0..count {
            day += 7;
            stack.pursue(kind, origin, GoalEvidence::EMPTY, 1.0, day);
            stack.review(day);
        }
        day
    }

    #[test]
    fn pursuing_forms_then_strengthens() {
        let mut stack = GoalStack::new();
        assert!(stack.pursue(
            GoalKind::StepUpToABiggerClub,
            GoalOrigin::SelfDrive,
            GoalEvidence::EMPTY,
            1.0,
            TODAY
        ));
        let first = stack.pressure_of(GoalKind::StepUpToABiggerClub);

        assert!(!stack.pursue(
            GoalKind::StepUpToABiggerClub,
            GoalOrigin::SelfDrive,
            GoalEvidence::EMPTY,
            1.0,
            TODAY
        ));
        assert!(stack.pressure_of(GoalKind::StepUpToABiggerClub) > first);
        assert_eq!(stack.len(), 1, "reinforcement must not duplicate the goal");
    }

    #[test]
    fn a_want_he_does_not_hold_reads_as_nothing() {
        let stack = GoalStack::new();
        assert_eq!(stack.pressure_of(GoalKind::GoHome), 0.0);
        assert_eq!(stack.status_of(GoalKind::GoHome), GoalStatus::Abandoned);
        assert!(!stack.is_pressing());
        assert_eq!(stack.wants_to_leave(), 0.0);
    }

    #[test]
    fn review_is_weekly() {
        let mut stack = GoalStack::new();
        feed(&mut stack, GoalKind::StepUpToABiggerClub, 3, TODAY);
        assert!(stack.review(TODAY).is_some(), "first review always runs");
        assert!(stack.review(TODAY + 3).is_none(), "too soon");
        assert!(stack.review(TODAY + 8).is_some());
    }

    #[test]
    fn he_holds_an_intention_across_months() {
        // The headline behaviour: a want formed once, carried coherently,
        // climbing as evidence accumulates — not re-derived every tick.
        let mut stack = GoalStack::new();
        let day = weeks(&mut stack, GoalKind::PlayFirstTeamFootball, 20, TODAY);

        let goal = stack.get(GoalKind::PlayFirstTeamFootball).unwrap();
        assert_eq!(goal.formed_on, TODAY + 7, "formed once, at the start");
        assert!(
            goal.age_days(day) > 120,
            "and carried for months: {} days",
            goal.age_days(day)
        );
        assert!(goal.status.is_public(), "by now he has said it");
        assert!(goal.reinforcements > 15);
    }

    #[test]
    fn a_want_shapes_decisions_long_before_it_is_spoken() {
        let mut stack = GoalStack::new();
        // Just enough to be Active, not enough to be Voiced.
        stack.pursue(
            GoalKind::StepUpToABiggerClub,
            GoalOrigin::SelfDrive,
            GoalEvidence::EMPTY,
            1.0,
            TODAY,
        );
        stack.pursue(
            GoalKind::StepUpToABiggerClub,
            GoalOrigin::SelfDrive,
            GoalEvidence::EMPTY,
            1.0,
            TODAY,
        );
        stack.review(TODAY);
        stack.review(TODAY + 7);

        assert_eq!(
            stack.status_of(GoalKind::StepUpToABiggerClub),
            GoalStatus::Active
        );
        assert!(
            stack.wants_to_leave() > 0.0,
            "the market can read the inclination"
        );
        assert!(
            stack.public_goals().count() == 0,
            "while nobody has heard him say anything"
        );
    }

    #[test]
    fn deciding_to_stay_suppresses_wanting_to_go() {
        let mut stack = GoalStack::new();
        feed(&mut stack, GoalKind::LeaveThisClub, 8, TODAY);
        let before = stack.pressure_of(GoalKind::LeaveThisClub);

        // A strong attachment forms and is fed every week.
        let mut day = TODAY;
        for _ in 0..30 {
            day += 7;
            stack.pursue(
                GoalKind::StayAtThisClub,
                GoalOrigin::Attachment,
                GoalEvidence::EMPTY,
                1.0,
                day,
            );
            stack.review(day);
        }

        assert!(
            stack.pressure_of(GoalKind::LeaveThisClub) < before * 0.5,
            "the pull to leave gives way: {before} → {}",
            stack.pressure_of(GoalKind::LeaveThisClub)
        );
    }

    #[test]
    fn competition_is_gradual_not_instant() {
        let mut stack = GoalStack::new();
        feed(&mut stack, GoalKind::LeaveThisClub, 8, TODAY);
        let before = stack.pressure_of(GoalKind::LeaveThisClub);

        feed(&mut stack, GoalKind::StayAtThisClub, 8, TODAY);
        stack.review(TODAY);

        let after = stack.pressure_of(GoalKind::LeaveThisClub);
        assert!(after < before, "it does give way");
        assert!(
            after > before * 0.7,
            "but not in a single week: {before} → {after}"
        );
    }

    #[test]
    fn wants_to_leave_nets_off_the_reasons_to_stay() {
        let mut stack = GoalStack::new();
        feed(&mut stack, GoalKind::LeaveThisClub, 8, TODAY);
        stack.review(TODAY);
        stack.review(TODAY + 7);
        let unopposed = stack.wants_to_leave();
        assert!(unopposed > 0.0);

        feed(&mut stack, GoalKind::WinBackMyPlace, 8, TODAY + 7);
        stack.review(TODAY + 14);
        stack.review(TODAY + 21);
        assert!(
            stack.wants_to_leave() < unopposed,
            "a reason to stay nets off the pull out"
        );
    }

    #[test]
    fn a_formal_demand_is_visible_as_such() {
        let mut stack = GoalStack::new();
        weeks_as(
            &mut stack,
            GoalKind::LeaveThisClub,
            GoalOrigin::Grievance,
            12,
            TODAY,
        );
        assert!(stack.is_pressing());
        assert!(
            stack
                .public_goals()
                .any(|g| g.kind == GoalKind::LeaveThisClub)
        );
    }

    #[test]
    fn getting_what_he_wanted_resolves_it() {
        let mut stack = GoalStack::new();
        weeks(&mut stack, GoalKind::WinBackMyPlace, 10, TODAY);
        assert!(stack.get(GoalKind::WinBackMyPlace).is_some());

        stack.advance(GoalKind::WinBackMyPlace, 1.0);
        let report = stack.review(TODAY + 100).unwrap();
        assert_eq!(report.satisfied, 1);
        assert!(stack.get(GoalKind::WinBackMyPlace).is_none());
    }

    #[test]
    fn a_deadline_he_set_and_missed_frustrates_the_goal() {
        let mut stack = GoalStack::new();
        feed(&mut stack, GoalKind::PlayFirstTeamFootball, 5, TODAY);
        stack.commit_until(GoalKind::PlayFirstTeamFootball, TODAY + 120);

        // He waits it out.
        let mut day = TODAY;
        for _ in 0..16 {
            day += 7;
            stack.review(day);
        }
        assert!(stack.get(GoalKind::PlayFirstTeamFootball).is_some());

        let report = stack.review(TODAY + 121).unwrap();
        assert_eq!(report.frustrated, 1);
    }

    #[test]
    fn a_move_answers_what_he_wanted_out_of_and_keeps_the_rest() {
        let mut stack = GoalStack::new();
        feed(&mut stack, GoalKind::LeaveThisClub, 8, TODAY);
        feed(&mut stack, GoalKind::BePaidWhatImWorth, 8, TODAY);
        feed(&mut stack, GoalKind::WinBackMyPlace, 8, TODAY);
        stack.review(TODAY);

        stack.on_club_change();

        assert!(
            stack.get(GoalKind::LeaveThisClub).is_none(),
            "he got out — that want is answered"
        );
        assert!(
            stack.get(GoalKind::WinBackMyPlace).is_none(),
            "a place at a club he has left means nothing"
        );
        let wage = stack
            .get(GoalKind::BePaidWhatImWorth)
            .expect("being underpaid is not settled by changing employer");
        assert_eq!(wage.blocked_by, GoalBlocker::JustArrived);
    }

    #[test]
    fn relevance_reports_what_he_currently_cares_about() {
        let mut stack = GoalStack::new();
        assert_eq!(
            stack.relevance_of(GoalDomain::Competitive),
            0.5,
            "an empty mind finds nothing especially relevant"
        );

        weeks(&mut stack, GoalKind::PlayFirstTeamFootball, 12, TODAY);
        assert!(
            stack.relevance_of(GoalDomain::Competitive) > 0.75,
            "minutes are all he thinks about"
        );
        assert_eq!(
            stack.relevance_of(GoalDomain::Financial),
            0.5,
            "and money is not"
        );
    }

    #[test]
    fn the_strongest_want_is_the_one_that_dominates() {
        let mut stack = GoalStack::new();
        feed(&mut stack, GoalKind::BePaidWhatImWorth, 2, TODAY);
        feed(&mut stack, GoalKind::PlayFirstTeamFootball, 9, TODAY);
        stack.review(TODAY);

        assert_eq!(
            stack.strongest().map(|g| g.kind),
            Some(GoalKind::PlayFirstTeamFootball)
        );
        assert_eq!(
            stack.strongest_in(GoalDomain::Financial).map(|g| g.kind),
            Some(GoalKind::BePaidWhatImWorth)
        );
    }

    #[test]
    fn a_full_stack_displaces_the_least_worth_keeping() {
        let mut stack = GoalStack::new();
        // Fill with weakly-held wants.
        for kind in GoalKind::ALL.iter().take(stack.goals.capacity()) {
            stack.pursue(
                *kind,
                GoalOrigin::SelfDrive,
                GoalEvidence::EMPTY,
                0.1,
                TODAY,
            );
        }
        assert_eq!(stack.len(), stack.goals.capacity());

        feed(&mut stack, GoalKind::MoveIntoCoaching, 1, TODAY);
        // The strong newcomer got in one way or another; the stack never
        // exceeds its cap.
        assert_eq!(stack.len(), stack.goals.capacity());
    }

    #[test]
    fn census_accounts_for_every_goal() {
        let mut stack = GoalStack::new();
        weeks_as(
            &mut stack,
            GoalKind::LeaveThisClub,
            GoalOrigin::Grievance,
            12,
            TODAY,
        );
        feed(&mut stack, GoalKind::GoHome, 2, TODAY);

        let census = stack.census();
        assert_eq!(census.live() as usize, stack.len());
        assert!(census.pressing >= 1);
    }

    #[test]
    fn the_stack_is_copy_and_bounded() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<GoalStack>();
        // Twelve goals of 24 bytes plus the review clock. The step from
        // 256 was `GoalEvidence` widening to `u64`: the 29 atoms it
        // started with filled a `u32` exactly, and the squad-standing,
        // development and international rules each needed atoms of their
        // own. Four bytes a goal, forty-eight a player, ~3 MB across a
        // world — paid deliberately, and this bound is what stops it
        // being paid again by accident.
        assert!(
            size_of::<GoalStack>() <= 320,
            "GoalStack grew to {}",
            size_of::<GoalStack>()
        );
    }
}
