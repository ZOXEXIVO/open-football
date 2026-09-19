//! The arc a player is living out — the thing that sequences his wants.
//!
//! The goal stack answers "what does he want today" and is very good at
//! it. What it cannot hold is the SHAPE of a career: that a boy who goes
//! out on loan means to come back and claim a shirt, that a man who has
//! accepted a level drop is not going to ask for a bigger club next
//! spring, that a second failed loan turns a prospect into a squad
//! player. Those are decisions about several seasons at once, and without
//! them every want is re-derived weekly from ground truth and nothing a
//! player does is coherent across a year.
//!
//! One plan at a time. It is formed from the same continuous drives the
//! goals are ([`MindSituation`]), it lags reality on purpose, and it is
//! reviewed on a date he gave it rather than every week. The goals keep
//! forming exactly as they did; the plan is what orders them, and what
//! everything outside the mind reads when it wants to know where he
//! thinks he is going.

use super::organs::goals::GoalOrigin;
use super::organs::memory::EpochDay;
use super::situation::MindSituation;
use crate::club::player::happiness::LoanSpellVerdict;
use crate::transfers::squad::LevelBand;

/// The shape of the next few seasons, as he means them to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CareerArc {
    /// Win the shirt at the club that owns him.
    BreakThroughHere,
    /// Go down to play, come back a candidate.
    ProveOnLoan,
    /// A returnee with a record; the parent owes him a look.
    ClaimMyPlace,
    /// He has outgrown the place and means to move up.
    StepUp,
    /// A permanent drop in level, accepted, for football.
    StepDownToPlay,
    /// He has found his level and is content at it.
    SettleAtMyLevel,
    /// The last spell, in his own country.
    FinishAtHome,
    /// The club servant.
    StayAndLead,
}

impl CareerArc {
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            CareerArc::BreakThroughHere => "career_arc_break_through_here",
            CareerArc::ProveOnLoan => "career_arc_prove_on_loan",
            CareerArc::ClaimMyPlace => "career_arc_claim_my_place",
            CareerArc::StepUp => "career_arc_step_up",
            CareerArc::StepDownToPlay => "career_arc_step_down_to_play",
            CareerArc::SettleAtMyLevel => "career_arc_settle_at_my_level",
            CareerArc::FinishAtHome => "career_arc_finish_at_home",
            CareerArc::StayAndLead => "career_arc_stay_and_lead",
        }
    }

    pub fn as_token(self) -> &'static str {
        match self {
            CareerArc::BreakThroughHere => "break_through_here",
            CareerArc::ProveOnLoan => "prove_on_loan",
            CareerArc::ClaimMyPlace => "claim_my_place",
            CareerArc::StepUp => "step_up",
            CareerArc::StepDownToPlay => "step_down_to_play",
            CareerArc::SettleAtMyLevel => "settle_at_my_level",
            CareerArc::FinishAtHome => "finish_at_home",
            CareerArc::StayAndLead => "stay_and_lead",
        }
    }

    /// The arc means him to leave, one way or another.
    #[inline]
    pub fn points_away(self) -> bool {
        matches!(
            self,
            CareerArc::ProveOnLoan
                | CareerArc::StepUp
                | CareerArc::StepDownToPlay
                | CareerArc::FinishAtHome
        )
    }

    /// The lowest band he will accept for the next move — anywhere, and
    /// then toward his own country — and the band he means to be playing
    /// at when the arc resolves.
    ///
    /// `runway` is the only input beyond the arc itself, because how far
    /// a man will drop to play is how much career he has left to spend on
    /// getting back up. Home is a property of the DESTINATION, so both
    /// floors travel and the side that knows where he is going picks:
    /// reading it off where he is standing when the arc formed made the
    /// home row unreachable for the one arc that is about going home.
    pub fn bands(self, runway: f32) -> (f32, f32, f32) {
        match self {
            CareerArc::BreakThroughHere => (0.6, 0.6, 1.0),
            CareerArc::ProveOnLoan => {
                let floor = if runway >= 0.7 { -0.2 } else { 0.2 };
                (floor, floor, 0.9)
            }
            CareerArc::ClaimMyPlace => (0.6, 0.6, 1.0),
            CareerArc::StepUp => (0.9, 0.9, 0.8),
            CareerArc::StepDownToPlay => (0.1, 0.1, 0.9),
            CareerArc::SettleAtMyLevel => (0.5, 0.5, 0.9),
            CareerArc::FinishAtHome => (0.3, -0.3, 0.7),
            CareerArc::StayAndLead => (0.8, 0.8, 1.0),
        }
    }
}

/// How far along the arc he is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlanStage {
    /// He has decided, and is giving the situation time to answer it.
    Forming,
    /// He is committed to it and shaping his decisions around it.
    Committed,
    /// He is asking somebody for it — the manager, the club, his agent.
    Asking,
    /// It is happening: the loan is agreed, the move is in motion.
    Acting,
}

impl PlanStage {
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            PlanStage::Forming => "plan_stage_forming",
            PlanStage::Committed => "plan_stage_committed",
            PlanStage::Asking => "plan_stage_asking",
            PlanStage::Acting => "plan_stage_acting",
        }
    }

    /// He is at the rung where somebody else hears about it.
    #[inline]
    pub fn is_asking(self) -> bool {
        matches!(self, PlanStage::Asking | PlanStage::Acting)
    }
}

/// The held, multi-season intention. `Copy` and fixed-size, because it
/// rides inside [`crate::PlayerMind`] and a player is cloned constantly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CareerPlan {
    pub arc: CareerArc,
    pub stage: PlanStage,
    pub formed_on: EpochDay,
    /// The deadline he gave it.
    pub review_on: EpochDay,
    /// Arcs re-entered — a second loan, a second ask.
    pub attempts: u8,
    /// The lowest [`LevelBand`] he will accept for the next move, and
    /// the lower one a move toward his own country is measured against.
    pub band_floor: f32,
    pub band_floor_home: f32,
    /// The band he means to be playing at when the arc resolves.
    pub band_target: f32,
    /// The band the last spell actually put him at. A man who has played
    /// a season at a level has proved he belongs there, and it is the
    /// floor he will not go below afterwards — which the arc's own table
    /// cannot know.
    pub loan_band: f32,
    pub origin: GoalOrigin,
    /// How firmly he holds it. A rival arc has to beat this by
    /// [`CareerPlan::SWITCH_MARGIN`] to displace it.
    pub strength: f32,
}

impl CareerPlan {
    /// How much better a rival arc has to look before a man changes his
    /// mind about his own career. Deliberately wide: a plan that flips
    /// every time a week goes badly is not a plan.
    pub const SWITCH_MARGIN: f32 = 0.15;
    /// Days a returnee gives the club to give him the look his record
    /// earned.
    pub const CLAIM_REVIEW_DAYS: u16 = 120;
    /// Loans a man will go out on before the answer is a level drop.
    pub const MAX_PROVE_ATTEMPTS: u8 = 2;
    /// Days a plan with no deadline of its own runs before he looks at
    /// it again. A season.
    pub const DEFAULT_REVIEW_DAYS: u16 = 365;
    /// Days after a window opens by which a man who meant to go
    /// somewhere expects to have gone.
    pub const WINDOW_REVIEW_DAYS: u16 = 45;
    /// How close the deadline has to be before it starts pushing him
    /// toward accepting what is on the table.
    pub const DEADLINE_HORIZON: f32 = 120.0;

    pub fn new(
        arc: CareerArc,
        origin: GoalOrigin,
        strength: f32,
        today: EpochDay,
        review_in_days: u16,
        runway: f32,
    ) -> Self {
        let (band_floor, band_floor_home, band_target) = arc.bands(runway);
        CareerPlan {
            arc,
            stage: PlanStage::Forming,
            formed_on: today,
            review_on: today.saturating_add(review_in_days),
            attempts: 0,
            band_floor,
            band_floor_home,
            band_target,
            loan_band: LevelBand::MIN,
            origin,
            strength: strength.clamp(0.0, 1.0),
        }
    }

    /// Re-enter the same arc after a failed attempt.
    pub fn retry(&self, today: EpochDay, review_in_days: u16) -> Self {
        CareerPlan {
            stage: PlanStage::Forming,
            formed_on: today,
            review_on: today.saturating_add(review_in_days),
            attempts: self.attempts.saturating_add(1),
            ..*self
        }
    }

    /// Push the deadline out, changing nothing else. A man who has had
    /// no football to be judged on has not failed at anything yet.
    pub fn defer(&self, today: EpochDay, review_in_days: u16) -> Self {
        CareerPlan {
            review_on: today.saturating_add(review_in_days),
            ..*self
        }
    }

    /// Move on to a different arc, keeping the count of what he has
    /// already tried — a man on his third plan is not starting fresh.
    pub fn succeed_with(
        &self,
        arc: CareerArc,
        today: EpochDay,
        review_in_days: u16,
        runway: f32,
    ) -> Self {
        let (band_floor, band_floor_home, band_target) = arc.bands(runway);
        CareerPlan {
            arc,
            stage: PlanStage::Forming,
            formed_on: today,
            review_on: today.saturating_add(review_in_days),
            attempts: self.attempts,
            band_floor,
            band_floor_home,
            band_target,
            loan_band: self.loan_band,
            origin: self.origin,
            strength: self.strength,
        }
    }

    /// How hard the deadline is pressing, 0..1.
    pub fn deadline_pressure(&self, today: EpochDay) -> f32 {
        let left = self.review_on.saturating_sub(today) as f32;
        (1.0 - left / Self::DEADLINE_HORIZON).clamp(0.0, 1.0)
    }

    #[inline]
    pub fn review_due(&self, today: EpochDay) -> bool {
        today >= self.review_on
    }

    /// How well a destination at `band` matches what the plan is for,
    /// −1..1. Positive is a fit; negative is below the floor he set.
    pub fn fit_for(&self, band: f32, going_home: bool) -> f32 {
        let floor = if going_home {
            self.band_floor_home
        } else {
            self.band_floor
        };
        LevelBand::fit(band, self.band_target)
            - Self::FLOOR_PENALTY_SCALE * ((floor - band) / 0.4).clamp(0.0, 1.0)
    }

    /// How steeply falling below his own floor is punished, relative to
    /// how well matching his target is rewarded.
    const FLOOR_PENALTY_SCALE: f32 = 1.43;

    /// Advance one rung. The ladder is one-way inside a plan; going
    /// backwards is a new plan.
    pub fn escalate(&mut self, to: PlanStage) {
        if to > self.stage {
            self.stage = to;
        }
    }

    /// The successor of a deadline that has passed. A man whose claim
    /// went unanswered for a season has already said what he thinks, so
    /// the arc it turns into does not start again at `Forming` — which
    /// is what made the ask unreachable whenever his review day fell on
    /// the same weekly tick that read it.
    pub fn already_asked(mut self) -> Self {
        self.stage = PlanStage::Asking;
        self
    }

    /// A milestone may lower him. A spell nobody could read is not a man
    /// on his way out of the door this week.
    pub fn stepped_back_to(mut self, stage: PlanStage) -> Self {
        self.stage = self.stage.min(stage);
        self
    }

    /// Shed what a week of the forming rule no longer holding costs it.
    /// Without this a plan formed at 0.81 could never be displaced,
    /// because `strength` only ever rose.
    pub fn fade(&mut self, per_month: f32) {
        const WEEKS_PER_MONTH: f32 = 52.0 / 12.0;
        self.strength = (self.strength * (1.0 - per_month / WEEKS_PER_MONTH)).clamp(0.0, 1.0);
    }
}

/// The plan as everything outside the mind reads it — a flat, `Copy`
/// view that carries no borrow of the mind and no way to change it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CareerPlanView {
    pub arc: Option<CareerArc>,
    pub stage: Option<PlanStage>,
    pub band_floor: f32,
    pub band_floor_home: f32,
    pub band_target: f32,
    /// 0..1 — how close the deadline he gave it is.
    pub deadline_pressure: f32,
    pub attempts: u8,
    /// How hard he holds it, 0..1 — what he pushes with when he asks.
    pub strength: f32,
}

impl CareerPlanView {
    /// No plan: every band reads as "no view" rather than as a refusal.
    pub fn none() -> Self {
        CareerPlanView {
            arc: None,
            stage: None,
            band_floor: LevelBand::MIN,
            band_floor_home: LevelBand::MIN,
            band_target: 0.0,
            deadline_pressure: 0.0,
            attempts: 0,
            strength: 0.0,
        }
    }

    pub fn of(plan: &CareerPlan, today: EpochDay) -> Self {
        CareerPlanView {
            arc: Some(plan.arc),
            stage: Some(plan.stage),
            band_floor: plan.band_floor,
            band_floor_home: plan.band_floor_home,
            band_target: plan.band_target,
            deadline_pressure: plan.deadline_pressure(today),
            attempts: plan.attempts,
            strength: plan.strength,
        }
    }

    /// Does he hold this arc, at or past this rung?
    pub fn is(&self, arc: CareerArc, stage: PlanStage) -> bool {
        self.arc == Some(arc) && self.stage.map(|s| s >= stage).unwrap_or(false)
    }

    /// He holds this arc, whatever rung it is at.
    pub fn holds(&self, arc: CareerArc) -> bool {
        self.arc == Some(arc)
    }

    /// How hard his own plan is pushing for a season somewhere else,
    /// 0..1 — what opens a parent's hold on its first choice.
    ///
    /// Only the two arcs that actually want a move away speak, and they
    /// speak in proportion to how firmly he holds them and how far
    /// along he is: a plan he has not said out loud is not a request.
    pub fn loan_push(&self) -> f32 {
        let voiced = match self.stage {
            Some(stage) if stage.is_asking() => 1.0,
            Some(PlanStage::Committed) => 0.6,
            _ => 0.3,
        };
        match self.arc {
            Some(CareerArc::ProveOnLoan) => self.strength * voiced,
            // He would rather move for good, but a season away is the
            // same football and he will take it.
            Some(CareerArc::StepDownToPlay) => self.strength * voiced * 0.7,
            _ => 0.0,
        }
    }

    /// How far his own decision to drop a level widens the reputation
    /// gap he will listen to, 0..1.
    ///
    /// The gap a renown band measures is a statement about his NAME, and
    /// a man who has decided he is going down to play has already made
    /// his peace with what that says about him. One reading, so the
    /// guard and the plausibility gate cannot take two different ones of
    /// the same fact.
    pub fn renown_widening(&self) -> f32 {
        match self.arc {
            Some(CareerArc::ProveOnLoan) | Some(CareerArc::StepDownToPlay) => self.strength,
            _ => 0.0,
        }
    }

    /// How well a destination at `band` serves the plan, −1..1. Zero
    /// with no plan — the appraisal's other axes own the decision then.
    pub fn fit_for(&self, band: f32, going_home: bool) -> f32 {
        if self.arc.is_none() {
            return 0.0;
        }
        let floor = if going_home {
            self.band_floor_home
        } else {
            self.band_floor
        };
        LevelBand::fit(band, self.band_target)
            - CareerPlan::FLOOR_PENALTY_SCALE * ((floor - band) / 0.4).clamp(0.0, 1.0)
    }
}

impl Default for CareerPlanView {
    fn default() -> Self {
        Self::none()
    }
}

/// The rules that form one, and the rules that resolve it.
///
/// Held apart from [`crate::club::player::mind::CareerMind`] so the
/// formation table reads as one list rather than as branches inside a
/// faculty that is also doing four other things.
pub struct CareerPlanner;

impl CareerPlanner {
    /// Start share below which he is not in the side, whatever the club
    /// calls him.
    const NOT_PLAYING_SHARE: f32 = 0.25;
    /// Runway at or above which going out to play is a career step
    /// rather than a demotion.
    const LOAN_RUNWAY: f32 = 0.45;
    /// Seasons of no football that make a level drop the answer.
    const DROUGHT_BAR: f32 = 0.5;
    /// How far past his club's level he has to have grown before he
    /// thinks about a bigger one.
    const OUTGROWN_BAR: f32 = 0.25;
    /// Ambition below which a man is not chasing anything.
    const CONTENT_AMBITION: f32 = 0.55;
    /// Ambition at or above which he is.
    const CLIMBER_AMBITION: f32 = 0.6;
    /// Loyalty at or above which a long server means to stay for good.
    const SERVANT_LOYALTY: f32 = 0.7;
    /// How much of a climb a stayer's loyalty takes back out of him.
    const LOYALTY_BRAKE: f32 = 0.6;
    /// Days at one club before "staying" and "moving on" are real
    /// choices rather than a settling-in period. Two seasons.
    const TENURE_FOR_A_CHOICE: u16 = 730;
    /// Career spent at which a man starts thinking about where he
    /// finishes.
    const HOMEWARD_SPENT: f32 = 0.75;

    /// The deadline a man who means to be somewhere else gives it: the
    /// next registration window, plus the grace he allows for the move
    /// to happen inside one. A season when no calendar is in view.
    fn move_deadline(situation: &MindSituation) -> u16 {
        if situation.days_to_next_window == u16::MAX {
            return CareerPlan::DEFAULT_REVIEW_DAYS;
        }
        situation
            .days_to_next_window
            .saturating_add(CareerPlan::WINDOW_REVIEW_DAYS)
    }

    /// How long an arc runs before he looks at it again. The two that
    /// are about being somewhere else are measured against the
    /// registration calendar, because that is when they can happen; the
    /// returnee's claim against the deadline he gave the club; the rest
    /// run a season.
    pub fn review_days_for(arc: CareerArc, situation: &MindSituation) -> u16 {
        match arc {
            CareerArc::ProveOnLoan | CareerArc::StepUp => Self::move_deadline(situation),
            CareerArc::ClaimMyPlace => CareerPlan::CLAIM_REVIEW_DAYS,
            _ => CareerPlan::DEFAULT_REVIEW_DAYS,
        }
    }

    /// The arc his situation argues for, with the strength he would hold
    /// it at. `None` when nothing in his circumstances points anywhere —
    /// which is most players most of the time, and correct.
    pub fn arc_for(
        situation: &MindSituation,
        home_desire: f32,
    ) -> Option<(CareerArc, GoalOrigin, f32)> {
        let band_here = situation.band_here();
        let runway = situation.career_runway();
        let spent = situation.career_spent();

        // The club servant, first, because it is the one arc that
        // suppresses every other reading of the same facts.
        if situation.loyalty_drive() >= Self::SERVANT_LOYALTY
            && situation.days_at_club >= MindSituation::CLUB_SERVANT_DAYS
            && band_here.map(|b| b >= 0.8).unwrap_or(false)
        {
            return Some((
                CareerArc::StayAndLead,
                GoalOrigin::Attachment,
                0.4 + 0.5 * situation.loyalty_drive(),
            ));
        }

        // The last spell, at home.
        if spent >= Self::HOMEWARD_SPENT
            && situation.is_abroad
            && (home_desire >= 0.3 || situation.familiar_teammates == 0)
        {
            return Some((
                CareerArc::FinishAtHome,
                GoalOrigin::Attachment,
                0.3 + 0.5 * home_desire.max(spent - Self::HOMEWARD_SPENT),
            ));
        }

        // Not playing at a club that has looked at him. A squad nobody
        // has ranked is not a queue he is at the back of, so no view is
        // no arc rather than the worst reading of one.
        let not_playing =
            situation.has_playing_view() && situation.starter_ratio < Self::NOT_PLAYING_SHARE;
        if not_playing
            && situation.has_squad_view()
            && situation.is_settled()
            && !situation.is_on_loan
        {
            let conviction =
                0.35 + 0.40 * situation.diligence() + 0.25 * situation.blocked_unfairly();
            // The boy who can see the shirt coming to him waits for it.
            if situation.can_wait_for_the_shirt() {
                return Some((
                    CareerArc::BreakThroughHere,
                    GoalOrigin::SelfDrive,
                    conviction,
                ));
            }
            // Everybody else it is not coming to, deep in the queue or
            // second in it behind a man his own age — which is the
            // archetypal loan and the one the old reading refused.
            if runway >= Self::LOAN_RUNWAY {
                return Some((CareerArc::ProveOnLoan, GoalOrigin::Survival, conviction));
            }
        }

        // Out of runway and out of football: he will drop a level to
        // play, which is the one move the ambition model could never
        // produce.
        if runway < Self::LOAN_RUNWAY && situation.football_drought >= Self::DROUGHT_BAR {
            return Some((
                CareerArc::StepDownToPlay,
                GoalOrigin::Survival,
                0.30 + 0.45 * situation.football_drought + 0.25 * spent,
            ));
        }

        // Long enough here to have a view of the place, and plainly
        // bigger than it.
        if situation.days_at_club >= Self::TENURE_FOR_A_CHOICE {
            // A move up is an investment in a career, so it needs one
            // left to invest in. That is the only thing age says here.
            if situation.outgrown_the_club() >= Self::OUTGROWN_BAR
                && situation.ambition_drive() >= Self::CLIMBER_AMBITION
                && runway > 0.0
            {
                let restlessness =
                    0.3 + 0.4 * situation.ambition_drive() + 0.3 * situation.outgrown_the_club();
                return Some((
                    CareerArc::StepUp,
                    GoalOrigin::SelfDrive,
                    // Loyalty is the brake. A man who wants to stay still
                    // feels the ceiling; he simply feels it less, and it
                    // takes him longer to act on it.
                    restlessness * (1.0 - Self::LOYALTY_BRAKE * situation.loyalty_drive()),
                ));
            }
            if situation.ambition_drive() < Self::CONTENT_AMBITION
                && band_here.map(|b| (0.6..=1.2).contains(&b)).unwrap_or(false)
            {
                return Some((
                    CareerArc::SettleAtMyLevel,
                    GoalOrigin::Attachment,
                    0.35 + 0.3 * (1.0 - situation.ambition_drive()),
                ));
            }
        }

        None
    }

    /// Start share at which a returnee's claim on a shirt counts as
    /// answered, whatever his paperwork calls him.
    const MIN_CLAIM_SHARE: f32 = 0.35;

    /// Is he playing? `None` until enough competitive football has gone
    /// past for the share to be a reading rather than a placeholder —
    /// the trap that wiped every post-return plan on the first Monday.
    fn playing(situation: &MindSituation) -> Option<bool> {
        situation.has_playing_view().then(|| {
            // What the paperwork promised him, what his own record has
            // taught him to expect, and the floor below which nobody is
            // playing whatever anybody calls him.
            let bar = situation
                .expected_start_share
                .max(situation.own_expected_start_share)
                .max(Self::MIN_CLAIM_SHARE);
            situation.starter_ratio >= bar
        })
    }

    /// Has the thing the arc was for simply happened?
    ///
    /// A plan is not only answered by its deadline: a boy who wanted a
    /// loan because he was not playing, and who is now playing, has got
    /// what he wanted without anybody arranging anything. Resolving it
    /// here is what stops a held intention outliving its own reason.
    pub fn is_answered(plan: &CareerPlan, situation: &MindSituation) -> bool {
        let playing = Self::playing(situation) == Some(true);
        match plan.arc {
            // Playing AT HIS OWN CLUB. A man out on loan and playing is
            // the plan working, not the plan finished — the spell's
            // verdict is what ends it.
            CareerArc::BreakThroughHere | CareerArc::ProveOnLoan => {
                !situation.is_on_loan && playing
            }
            CareerArc::ClaimMyPlace | CareerArc::StepDownToPlay => playing,
            CareerArc::FinishAtHome => !situation.is_abroad,
            // Steady states and open-ended ambitions: only a deadline
            // resolves these.
            CareerArc::StepUp | CareerArc::SettleAtMyLevel | CareerArc::StayAndLead => false,
        }
    }

    /// What a plan whose deadline has arrived turns into.
    ///
    /// Every arc resolves the same way — did the thing it was for
    /// happen — and the successor is the honest next question. `None`
    /// ends the plan and lets formation start again from scratch.
    pub fn review(
        plan: &CareerPlan,
        situation: &MindSituation,
        today: EpochDay,
    ) -> Option<CareerPlan> {
        let runway = situation.career_runway();
        // A deadline arriving on football nobody has watched is not an
        // answer to anything. The two arcs that resolve on something
        // other than minutes are read without it.
        let playing = match Self::playing(situation) {
            Some(playing) => playing,
            None if matches!(plan.arc, CareerArc::FinishAtHome | CareerArc::StepUp) => false,
            None => return Some(plan.defer(today, CareerPlan::WINDOW_REVIEW_DAYS)),
        };

        match plan.arc {
            // He waited for the shirt. Either it came, or the wait is
            // over and the answer is somewhere he plays.
            CareerArc::BreakThroughHere => {
                if playing {
                    return None;
                }
                Some(
                    plan.succeed_with(
                        CareerArc::ProveOnLoan,
                        today,
                        Self::review_days_for(CareerArc::ProveOnLoan, situation),
                        runway,
                    )
                    .already_asked(),
                )
            }
            // The loan never happened. He asks again while he is young
            // enough, and accepts a permanent drop when he is not.
            CareerArc::ProveOnLoan => {
                if plan.attempts + 1 >= CareerPlan::MAX_PROVE_ATTEMPTS && !playing {
                    return Some(
                        plan.succeed_with(
                            CareerArc::StepDownToPlay,
                            today,
                            CareerPlan::DEFAULT_REVIEW_DAYS,
                            runway,
                        )
                        .already_asked(),
                    );
                }
                if playing {
                    return None;
                }
                Some(
                    plan.retry(
                        today,
                        Self::review_days_for(CareerArc::ProveOnLoan, situation),
                    )
                    .already_asked(),
                )
            }
            // The look he came home for. He got it, or he did not — and
            // which way he goes then is whether he has outgrown the
            // place or merely been passed over at it.
            CareerArc::ClaimMyPlace => {
                if playing {
                    return None;
                }
                let arc = if situation.outgrown_the_club() > 0.3 {
                    CareerArc::StepUp
                } else {
                    CareerArc::StepDownToPlay
                };
                let mut next = plan
                    .succeed_with(arc, today, Self::review_days_for(arc, situation), runway)
                    .already_asked();
                if arc == CareerArc::StepDownToPlay && plan.loan_band > LevelBand::MIN {
                    // He will not go below where the loan already put
                    // him: he played a season at that level and has
                    // proved he belongs at it. The arc's own floor is a
                    // table; this is his record.
                    next.band_floor = plan.loan_band;
                    next.band_floor_home = plan.loan_band;
                }
                Some(next)
            }
            // Nobody came. The move he wanted is not available at the
            // level he wanted it, so the question becomes his level.
            CareerArc::StepUp => Some(plan.succeed_with(
                CareerArc::SettleAtMyLevel,
                today,
                CareerPlan::DEFAULT_REVIEW_DAYS,
                runway,
            )),
            // These three are their own answer: they resolve when the
            // facts that formed them change, and formation re-runs.
            CareerArc::StepDownToPlay | CareerArc::SettleAtMyLevel | CareerArc::StayAndLead => {
                if playing {
                    return None;
                }
                Some(plan.retry(today, CareerPlan::DEFAULT_REVIEW_DAYS))
            }
            CareerArc::FinishAtHome => {
                if !situation.is_abroad {
                    return None;
                }
                Some(plan.retry(today, CareerPlan::DEFAULT_REVIEW_DAYS))
            }
        }
    }

    /// What a finished loan spell turns the plan into.
    ///
    /// The one milestone that is a verdict rather than a date, so it
    /// reads the verdict rather than the calendar.
    pub fn after_loan(
        plan: &CareerPlan,
        verdict: LoanSpellVerdict,
        runway: f32,
        loan_band: f32,
        today: EpochDay,
    ) -> CareerPlan {
        let plan = &CareerPlan { loan_band, ..*plan };
        if verdict.is_positive() {
            return plan.succeed_with(
                CareerArc::ClaimMyPlace,
                today,
                CareerPlan::CLAIM_REVIEW_DAYS,
                runway,
            );
        }
        if matches!(verdict, LoanSpellVerdict::Inconclusive) {
            // A spell nobody could read changes nothing about the arc —
            // but he is home, so he is no longer acting on it, and a
            // returnee asking for another loan the same week is the one
            // reading `Acting` could never take back.
            return plan.stepped_back_to(PlanStage::Committed);
        }
        if plan.attempts + 1 >= CareerPlan::MAX_PROVE_ATTEMPTS || runway < Self::LOAN_RUNWAY {
            return plan.succeed_with(
                CareerArc::StepDownToPlay,
                today,
                CareerPlan::DEFAULT_REVIEW_DAYS,
                runway,
            );
        }
        let mut again = plan.succeed_with(
            CareerArc::ProveOnLoan,
            today,
            CareerPlan::WINDOW_REVIEW_DAYS,
            runway,
        );
        again.attempts = plan.attempts.saturating_add(1);
        again
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerFieldPositionGroup;

    struct Fx;

    impl Fx {
        const TODAY: EpochDay = 9_000;
        /// The band the spell in these fixtures actually put him at: a
        /// starter one level below where he came from.
        const LOAN_BAND: f32 = 0.6;

        /// A man nobody has any reason to think about: mid-twenties,
        /// ordinary in every drive, playing the share his role implies.
        fn settled() -> MindSituation {
            MindSituation {
                ambition: 12.0,
                days_at_club: 800,
                // A season of football behind him, so the start share is
                // a reading rather than the placeholder every arc
                // refuses to act on.
                appearances_tracked: 30,
                // A shade above the key-player floor of a club at this
                // standing: he is at his level, which is what makes the
                // arcs that read a band read THIS band.
                own_level: 95,
                position_group: PlayerFieldPositionGroup::Midfielder,
                club_reputation: 0.5,
                ..MindSituation::neutral()
            }
        }

        /// Young, settled, and nowhere near the side.
        fn blocked_youngster() -> MindSituation {
            MindSituation {
                age: 20,
                starter_ratio: 0.05,
                pecking_rank: 4,
                rivals_at_position: 5,
                top_rival_age: 26,
                ..Fx::settled()
            }
        }

        fn plan(arc: CareerArc, attempts: u8) -> CareerPlan {
            let mut plan = CareerPlan::new(
                arc,
                GoalOrigin::Survival,
                0.6,
                Self::TODAY,
                CareerPlan::DEFAULT_REVIEW_DAYS,
                0.7,
            );
            plan.attempts = attempts;
            plan
        }
    }

    #[test]
    fn a_blocked_youngster_decides_to_go_and_play() {
        let (arc, _, strength) = CareerPlanner::arc_for(&Fx::blocked_youngster(), 0.0)
            .expect("he has decided something");
        assert_eq!(arc, CareerArc::ProveOnLoan);
        assert!(strength > 0.0);
    }

    #[test]
    fn a_boy_behind_an_old_man_waits_for_the_shirt_instead() {
        let waiting = MindSituation {
            pecking_rank: 2,
            top_rival_age: 33,
            age: 21,
            ..Fx::blocked_youngster()
        };
        let (arc, _, _) = CareerPlanner::arc_for(&waiting, 0.0).unwrap();
        assert_eq!(
            arc,
            CareerArc::BreakThroughHere,
            "the shirt is coming to him; going away is what happens if it does not"
        );
    }

    /// The archetypal loan, and the one the old reading refused: second
    /// in the queue behind a man his own age, so the shirt is not coming
    /// and there is nothing to wait for.
    #[test]
    fn a_backup_behind_a_man_his_own_age_goes_out_to_play() {
        let blocked = MindSituation {
            age: 21,
            pecking_rank: 2,
            rivals_at_position: 2,
            top_rival_age: 22,
            rival_gap: 2,
            starter_ratio: 0.05,
            ..Fx::settled()
        };
        let (arc, _, _) = CareerPlanner::arc_for(&blocked, 0.0)
            .expect("the shirt is not coming and he is young enough to go and play");
        assert_eq!(arc, CareerArc::ProveOnLoan);
    }

    /// A squad nobody has ranked is not a queue he is at the back of.
    #[test]
    fn a_squad_nobody_has_ranked_argues_for_nothing() {
        let unranked = MindSituation {
            age: 21,
            pecking_rank: 0,
            starter_ratio: 0.05,
            ..Fx::settled()
        };
        assert!(CareerPlanner::arc_for(&unranked, 0.0).is_none());
    }

    /// `at_home` describes the destination, not where he is standing
    /// when the arc forms — which is why the arc that is ABOUT going
    /// home could never reach its own home floor.
    #[test]
    fn the_home_floor_belongs_to_the_destination() {
        let plan = Fx::plan(CareerArc::FinishAtHome, 0);
        let modest = -0.2;
        assert!(
            plan.fit_for(modest, true) > plan.fit_for(modest, false),
            "he will drop further for a club at home than for one anywhere else"
        );
        assert!(
            plan.fit_for(modest, true) >= 0.0,
            "a modest club at home is not something he argues against"
        );
        assert!(
            plan.fit_for(modest, false) < 0.0,
            "the same club abroad is below the floor he set"
        );
    }

    /// The claim ran out. The successor is something he has already
    /// said, so it starts at the rung he reached rather than back at
    /// `Forming` — which is what made the ask unreachable whenever the
    /// review day fell on the tick that read it.
    #[test]
    fn the_successor_of_a_passed_deadline_is_already_asked() {
        let ignored = MindSituation {
            starter_ratio: 0.05,
            own_level: 150,
            club_reputation: 0.3,
            ..Fx::settled()
        };
        let next = CareerPlanner::review(
            &Fx::plan(CareerArc::ClaimMyPlace, 0),
            &ignored,
            Fx::TODAY + CareerPlan::CLAIM_REVIEW_DAYS,
        )
        .expect("an unanswered claim does not simply evaporate");
        assert_eq!(next.stage, PlanStage::Asking);
    }

    /// A plan only ever hardened, so one formed at 0.81 could never be
    /// displaced by anything. The circumstances leaving take it back
    /// down again.
    #[test]
    fn a_plan_the_circumstances_no_longer_argue_for_fades() {
        let mut plan = Fx::plan(CareerArc::ProveOnLoan, 0);
        let before = plan.strength;
        for _ in 0..8 {
            plan.fade(0.22);
        }
        assert!(plan.strength < before * 0.8, "{}", plan.strength);
        assert!(plan.strength > 0.0, "fading is not forgetting");
    }

    #[test]
    fn a_man_playing_regularly_decides_nothing() {
        assert!(
            CareerPlanner::arc_for(&Fx::settled(), 0.0).is_none(),
            "most footballers most weeks are simply getting on with it"
        );
    }

    #[test]
    fn a_man_out_of_runway_and_out_of_football_accepts_a_level_drop() {
        let stuck = MindSituation {
            age: 32,
            starter_ratio: 0.0,
            football_drought: 0.8,
            ..Fx::settled()
        };
        let (arc, _, strength) = CareerPlanner::arc_for(&stuck, 0.0).unwrap();
        assert_eq!(arc, CareerArc::StepDownToPlay);
        assert!(strength > 0.5, "years of it is not a mild preference");
    }

    #[test]
    fn a_man_plainly_bigger_than_his_club_means_to_move_up() {
        let outgrown = MindSituation {
            age: 25,
            ambition: 17.0,
            own_level: 150,
            club_reputation: 0.3,
            starter_ratio: 0.8,
            ..Fx::settled()
        };
        let (arc, _, _) = CareerPlanner::arc_for(&outgrown, 0.0).unwrap();
        assert_eq!(arc, CareerArc::StepUp);
    }

    /// A move up is an investment in a career. With none left the same
    /// facts read as a man finishing where he is.
    #[test]
    fn the_same_man_at_the_end_of_his_career_does_not() {
        let done = MindSituation {
            age: 36,
            ambition: 17.0,
            own_level: 150,
            club_reputation: 0.3,
            starter_ratio: 0.8,
            ..Fx::settled()
        };
        assert!(!matches!(
            CareerPlanner::arc_for(&done, 0.0),
            Some((CareerArc::StepUp, _, _))
        ));
    }

    #[test]
    fn a_content_long_server_at_his_own_level_settles() {
        let content = MindSituation {
            age: 28,
            ambition: 7.0,
            starter_ratio: 0.7,
            ..Fx::settled()
        };
        let (arc, _, _) = CareerPlanner::arc_for(&content, 0.0).unwrap();
        assert_eq!(arc, CareerArc::SettleAtMyLevel);
    }

    #[test]
    fn a_veteran_abroad_who_wants_home_means_to_finish_there() {
        let homesick = MindSituation {
            age: 33,
            is_abroad: true,
            familiar_teammates: 0,
            ..Fx::settled()
        };
        let (arc, _, _) = CareerPlanner::arc_for(&homesick, 0.6).unwrap();
        assert_eq!(arc, CareerArc::FinishAtHome);
    }

    #[test]
    fn a_club_servant_means_to_stay() {
        let servant = MindSituation {
            age: 30,
            loyalty: 18.0,
            days_at_club: 2_200,
            own_level: 125,
            starter_ratio: 0.8,
            ..Fx::settled()
        };
        let (arc, _, _) = CareerPlanner::arc_for(&servant, 0.0).unwrap();
        assert_eq!(arc, CareerArc::StayAndLead);
    }

    #[test]
    fn a_good_spell_turns_the_loan_into_a_claim_on_the_shirt() {
        let after = CareerPlanner::after_loan(
            &Fx::plan(CareerArc::ProveOnLoan, 0),
            LoanSpellVerdict::Standout,
            0.8,
            Fx::LOAN_BAND,
            Fx::TODAY,
        );
        assert_eq!(after.arc, CareerArc::ClaimMyPlace);
        assert_eq!(
            after.review_on,
            Fx::TODAY + CareerPlan::CLAIM_REVIEW_DAYS,
            "he gives the club a deadline, not an open-ended hope"
        );
    }

    #[test]
    fn a_bad_spell_is_worth_one_more_go_and_then_a_level_drop() {
        let again = CareerPlanner::after_loan(
            &Fx::plan(CareerArc::ProveOnLoan, 0),
            LoanSpellVerdict::Peripheral,
            0.8,
            Fx::LOAN_BAND,
            Fx::TODAY,
        );
        assert_eq!(again.arc, CareerArc::ProveOnLoan);
        assert_eq!(again.attempts, 1);

        let exhausted = CareerPlanner::after_loan(
            &Fx::plan(CareerArc::ProveOnLoan, 1),
            LoanSpellVerdict::Peripheral,
            0.8,
            Fx::LOAN_BAND,
            Fx::TODAY,
        );
        assert_eq!(exhausted.arc, CareerArc::StepDownToPlay);
    }

    #[test]
    fn a_spell_nobody_can_read_leaves_the_plan_where_it_was() {
        let plan = Fx::plan(CareerArc::ProveOnLoan, 0);
        let after = CareerPlanner::after_loan(
            &plan,
            LoanSpellVerdict::Inconclusive,
            0.8,
            Fx::LOAN_BAND,
            Fx::TODAY,
        );
        assert_eq!(after.arc, plan.arc);
        assert_eq!(after.review_on, plan.review_on);
        assert_eq!(after.attempts, plan.attempts);
        assert_eq!(
            after.loan_band,
            Fx::LOAN_BAND,
            "the level he played at is his record whatever anybody made of it"
        );
    }

    #[test]
    fn a_claim_the_club_never_answered_becomes_a_move() {
        let ignored = MindSituation {
            starter_ratio: 0.05,
            own_level: 150,
            club_reputation: 0.3,
            ..Fx::settled()
        };
        let next = CareerPlanner::review(
            &Fx::plan(CareerArc::ClaimMyPlace, 0),
            &ignored,
            Fx::TODAY + CareerPlan::CLAIM_REVIEW_DAYS,
        )
        .expect("an unanswered claim does not simply evaporate");
        assert_eq!(
            next.arc,
            CareerArc::StepUp,
            "he has outgrown the place, so the move is upward"
        );
    }

    #[test]
    fn a_claim_the_club_answered_is_over() {
        let playing = MindSituation {
            starter_ratio: 0.8,
            ..Fx::settled()
        };
        assert!(CareerPlanner::is_answered(
            &Fx::plan(CareerArc::ClaimMyPlace, 0),
            &playing
        ));
    }

    /// Playing every week ON LOAN is the plan working, not the plan
    /// finished — the spell's verdict is what ends it.
    #[test]
    fn a_loanee_playing_every_week_has_not_finished_his_arc() {
        let away = MindSituation {
            is_on_loan: true,
            starter_ratio: 0.9,
            ..Fx::settled()
        };
        assert!(!CareerPlanner::is_answered(
            &Fx::plan(CareerArc::ProveOnLoan, 0),
            &away
        ));
    }

    #[test]
    fn a_destination_at_the_target_band_fits_and_one_below_the_floor_does_not() {
        let plan = Fx::plan(CareerArc::ProveOnLoan, 0);
        assert!(plan.fit_for(plan.band_target, false) > 0.9);
        assert!(plan.fit_for(plan.band_floor - 0.4, false) < 0.0);
    }

    #[test]
    fn the_deadline_presses_harder_as_it_approaches() {
        let plan = Fx::plan(CareerArc::ProveOnLoan, 0);
        let early = plan.deadline_pressure(Fx::TODAY);
        let late = plan.deadline_pressure(plan.review_on - 10);
        assert!(early < late);
        assert_eq!(plan.deadline_pressure(plan.review_on), 1.0);
    }

    #[test]
    fn the_plan_is_copy_so_the_mind_stays_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<CareerPlan>();
        assert_copy::<CareerPlanView>();
    }
}
