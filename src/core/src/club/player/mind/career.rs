//! The career mind — where he is going, and how much time is left to
//! get there.
//!
//! The faculty that thinks in seasons rather than weeks. It holds a
//! read of his own trajectory that can lag reality (a player who has
//! plateaued still thinks of himself as rising, for a while), and it
//! owns the wants that are about the shape of a career: stepping up,
//! stronger leagues, continental football, and eventually stopping.
//!
//! Absorbs, over the phases: the desire detectors in
//! `transfer/processing.rs`, `big_stage_pull.rs`, and
//! `lifecycle.rs::CareerStageDetector`.

use crate::transfers::pipeline::trace::MarketSwitches;

use super::anchors::{
    AnchorView, CareerAnchors, CareerChapter, CareerDreamReview, CareerSummit, ChapterKind,
    DreamKind,
};
use super::organs::MindOrgans;
use super::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin, GoalStatus};
use super::organs::memory::{ActorRef, EpisodeKind, EpochDay, FactClaim, MindEpisode};
use super::plan::{CareerArc, CareerPlan, CareerPlanView, CareerPlanner, PlanStage};
use super::situation::MindSituation;
use super::submind::{MindOption, MindView, MoodContribution, ReasonSet, SubMind};
use crate::club::player::happiness::LoanSpellVerdict;

/// Where a player is in his working life. Held rather than derived, so
/// it can lag the calendar — which is how people actually experience
/// this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum CareerStage {
    /// Still arriving.
    #[default]
    Emerging,
    /// Establishing himself.
    Establishing,
    /// The years that count.
    Prime,
    /// Still good, and aware it will not last.
    Veteran,
    /// Winding down.
    Closing,
}

impl CareerStage {
    /// The stage the calendar says he is in.
    pub fn from_age(age: u8) -> Self {
        match age {
            0..=20 => CareerStage::Emerging,
            21..=23 => CareerStage::Establishing,
            24..=29 => CareerStage::Prime,
            30..=33 => CareerStage::Veteran,
            _ => CareerStage::Closing,
        }
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            CareerStage::Emerging => "mind_career_stage_emerging",
            CareerStage::Establishing => "mind_career_stage_establishing",
            CareerStage::Prime => "mind_career_stage_prime",
            CareerStage::Veteran => "mind_career_stage_veteran",
            CareerStage::Closing => "mind_career_stage_closing",
        }
    }
}

/// His read of his own trajectory.
#[derive(Debug, Clone, Copy, Default)]
pub struct CareerMind {
    pub stage: CareerStage,
    /// Trophies, titles and continental nights he has actually had.
    /// Saturates — the difference between eight and eighty is not
    /// meaningful to how he feels about the next one.
    pub honours: u8,
    /// Moves that took him up a level.
    pub step_ups: u8,
    /// Times he has gone down a level, been relegated, or been let go.
    pub setbacks: u8,
    /// What he thinks he has become lately — an EMA of his observable
    /// level, ×10. 0 until the first read.
    level_fast_x10: u16,
    /// What he thought he was a year or more ago, same units. The pair
    /// is the whole trend detector: one number that keeps up and one
    /// that lags, and the distance between them is whether he is
    /// getting better.
    level_slow_x10: u16,
    /// Weeks he has judged himself to be standing still. Saturates.
    pub stagnant_weeks: u8,
    /// The arc he is living out, and the deadline he gave it. One at a
    /// time — see [`CareerPlan`].
    pub plan: Option<CareerPlan>,
    /// Arcs he has already tried, kept across an answered plan. A loan
    /// he never got is not forgotten because he broke through in the
    /// meantime, and a man on his third go is not starting fresh.
    attempts: u8,
    /// What a career leaves behind. No club change touches these.
    pub anchors: CareerAnchors,
}

impl CareerMind {
    /// Ambition at or above which he actively wants more, on the 0–20
    /// personality scale.
    pub const AMBITIOUS: f32 = 12.0;

    /// Days at one level after which a player starts feeling the
    /// ceiling. Two full seasons.
    pub const PLATEAU_DAYS: u16 = 730;

    /// How quickly his read of himself keeps up, per weekly think. About
    /// a two-month memory.
    const FAST_ALPHA: f32 = 0.12;
    /// And how slowly the other one does. About a year — deliberately
    /// longer than a season, so a good autumn is not mistaken for
    /// getting better.
    const SLOW_ALPHA: f32 = 0.02;

    /// What living his dream says for signing on, whatever else he wants.
    const ARRIVED_SIGNS: f32 = 0.6;
    /// How much of his belief in a climb argues against a long deal. A man
    /// certain of it, with his prime ahead, says no to the first offer and
    /// asks for a way out; anybody less sure signs and carries on wanting.
    const TIED_DOWN: f32 = 0.6;

    /// Weeks of standing still before a player concludes anything from
    /// it. A season's worth: nobody decides he has plateaued in March
    /// because February was quiet.
    pub const STAGNANT_WEEKS: u8 = 30;

    /// How settled he is in his career, −1..+1: honours and step-ups
    /// against setbacks. Feeds the mood contribution.
    pub fn trajectory(&self) -> f32 {
        let up = self.honours as f32 * 0.5 + self.step_ups as f32;
        let down = self.setbacks as f32;
        ((up - down) / 6.0).clamp(-1.0, 1.0)
    }

    /// Is he old enough, and finished enough, to be thinking about the
    /// end?
    #[inline]
    pub fn is_winding_down(&self) -> bool {
        matches!(self.stage, CareerStage::Closing)
    }

    /// Is he still getting better? −1..+1.
    ///
    /// **Deliberately not his ability.** A player cannot read his own
    /// current ability any more than a coach can read another man's —
    /// what he can read is what he looks like from outside, which is the
    /// same thing the rest of football judges him on. The number is the
    /// distance between a fast read of that and a slow one, so it is a
    /// *trend* rather than a level: a career-long 7-out-of-10 reads as
    /// standing still, which is exactly what it is.
    pub fn improvement(&self) -> f32 {
        if self.level_slow_x10 == 0 {
            return 0.0;
        }
        let delta = self.level_fast_x10 as f32 - self.level_slow_x10 as f32;
        // Four levels of daylight between the two reads is a man who is
        // plainly on his way up.
        (delta / 10.0 / 4.0).clamp(-1.0, 1.0)
    }

    /// Has he concluded he has stopped going anywhere?
    #[inline]
    pub fn feels_stalled(&self) -> bool {
        self.stagnant_weeks >= Self::STAGNANT_WEEKS
    }

    /// Take this week's reading of what he looks like from outside.
    fn track_progress(&mut self, observable_level: u8) {
        let reading = observable_level as f32 * 10.0;
        if self.level_slow_x10 == 0 {
            // First look. Both reads start where he is, so a player is
            // never born believing he is already in decline.
            self.level_fast_x10 = reading as u16;
            self.level_slow_x10 = reading as u16;
            return;
        }
        let fast = self.level_fast_x10 as f32;
        let slow = self.level_slow_x10 as f32;
        self.level_fast_x10 = (fast + (reading - fast) * Self::FAST_ALPHA).round() as u16;
        self.level_slow_x10 = (slow + (reading - slow) * Self::SLOW_ALPHA).round() as u16;

        // Standing still is a run, not a week. It accrues while nothing
        // is moving and clears the moment it does — a player who starts
        // improving again stops thinking about it immediately, which is
        // what makes a new coach or a new role able to answer this.
        if self.improvement() > 0.10 {
            self.stagnant_weeks = 0;
        } else {
            self.stagnant_weeks = self.stagnant_weeks.saturating_add(1);
        }
    }
}

impl SubMind for CareerMind {
    fn domain(&self) -> GoalDomain {
        GoalDomain::Career
    }

    fn observe(&mut self, episode: &MindEpisode, _organs: &mut MindOrgans) {
        match episode.kind {
            EpisodeKind::WonLeagueTitle
            | EpisodeKind::WonContinentalTrophy
            | EpisodeKind::WonDomesticCup
            | EpisodeKind::NationalTeamGlory => {
                if self.honours == 0 && !MarketSwitches::career_anchors_off() {
                    self.anchors.tell(CareerChapter {
                        kind: ChapterKind::FirstMajorHonour,
                        on: episode.when,
                        club_id: episode.where_club,
                        ..Default::default()
                    });
                }
                self.honours = self.honours.saturating_add(1);
            }
            EpisodeKind::Relegated | EpisodeKind::ReleasedByClub | EpisodeKind::SoldAgainstWill => {
                self.setbacks = self.setbacks.saturating_add(1);
            }
            _ => {}
        }
    }

    fn reflect(&mut self, view: &MindView<'_>, organs: &mut MindOrgans) {
        let s = view.situation;
        let today = view.today();

        // The stage only ever advances. A player does not get younger,
        // and the read catching up late is the point — it lags the
        // calendar rather than tracking it.
        let by_age = CareerStage::from_age(s.age);
        if by_age > self.stage {
            self.stage = by_age;
        }

        self.keep_his_anchors(view, organs);

        // The arc, before the wants. Everything below forms goals; the
        // plan is what orders them, and it pushes its own goal on the
        // stack so the two can never disagree about where he is going.
        self.consider_his_plan(view, organs);

        // ── The end of it ───────────────────────────────────────
        if self.is_winding_down() {
            // A man closing out a career who is no longer playing starts
            // thinking about stopping. One who is still first choice
            // does not — which is why this reads the minutes and not
            // just the birthday.
            let fading = (1.0 - s.starter_ratio).clamp(0.0, 1.0) * s.career_spent();
            if fading > 0.25 {
                organs.goals.pursue(
                    GoalKind::RetireOnMyTerms,
                    GoalOrigin::Survival,
                    GoalEvidence::of(&[GoalEvidence::LATE_CAREER]),
                    fading,
                    today,
                );
            }
            // A decorated veteran starts looking at what comes after.
            if self.honours >= 2 {
                organs.goals.pursue(
                    GoalKind::MoveIntoCoaching,
                    GoalOrigin::SelfDrive,
                    GoalEvidence::of(&[GoalEvidence::LATE_CAREER]),
                    0.25,
                    today,
                );
            }
            // And stops chasing what he is no longer going to get.
            return;
        }

        // ── Still going somewhere? ──────────────────────────────
        self.consider_his_development(view, organs);

        // ── His country ─────────────────────────────────────────
        self.consider_his_country(view, organs);

        // ── The loan, from whichever end he is at ───────────────
        //
        // Returns early for a man out on loan: nothing below this — the
        // long stay, the plateau, the step up — is a thing he can act
        // on, because he does not belong to the club he is playing for
        // and the one he does belong to is not where he is.
        if self.consider_the_loan(view, organs) {
            return;
        }

        // ── Wanting more ────────────────────────────────────────
        let drive = ((s.ambition - Self::AMBITIOUS) / (20.0 - Self::AMBITIOUS)).clamp(0.0, 1.0);

        // A club he has concluded is going nowhere. This one does not
        // wait on ambition — being taken down a division is a thing that
        // happened to him, not a thing he wants.
        let here = ActorRef::club(view.tick.club_id);
        if organs.memory.believes(FactClaim::RelegatedWithThem, here) > 0.4 {
            organs.goals.pursue(
                GoalKind::KeepPlayingAtThisLevel,
                GoalOrigin::Circumstance,
                GoalEvidence::of(&[GoalEvidence::RELEGATED]),
                (0.35 + drive * 0.6).clamp(0.0, 1.0),
                today,
            );
        }

        // He has been here long enough to feel the ceiling. What that
        // does to him is decided by character, and it is three different
        // men rather than one.
        if s.days_at_club >= Self::PLATEAU_DAYS {
            self.consider_the_long_stay(view, organs, drive);
        }
    }

    fn weigh(&self, option: MindOption, organs: &MindOrgans) -> ReasonSet {
        self.weigh_option(option, organs)
    }

    fn appraise(&self, _organs: &MindOrgans) -> MoodContribution {
        // A career going the way he wanted lifts him; one going wrong
        // weighs. Confidence rises with how much career there is to
        // judge — a teenager with two appearances has no trajectory yet.
        let evidence = (self.honours as f32 + self.step_ups as f32 + self.setbacks as f32) / 4.0;
        let mut confidence = evidence.clamp(0.0, 1.0);
        // Whether he is still improving is part of how a career feels,
        // and for a young player with no honours yet it is most of it —
        // but only once he has been looked at. A mind that has never
        // taken a reading still has nothing to say.
        if self.level_slow_x10 > 0 {
            confidence = confidence.max(0.3);
        }
        let value = self.trajectory() * 5.0 + self.improvement() * 2.0;
        MoodContribution::new(GoalDomain::Career, value, confidence)
    }
}

impl CareerMind {
    /// What he comes into the world with: the record he has, and the dream
    /// who he is and what he has done argue for, dated today. A man with a
    /// senior career behind him has no first club left to tell.
    pub fn seed_anchors(
        &mut self,
        s: &MindSituation,
        club_id: u32,
        summit: Option<CareerSummit>,
        has_senior_record: bool,
        today: EpochDay,
    ) {
        self.anchors.summit = summit;
        self.anchors.runway = s.career_runway();
        self.anchors.told_first_club = has_senior_record;
        self.anchors.form_dream(s, club_id, today);
    }

    /// What this week adds to the career he carries: a move read for what
    /// it did, a new record, a dream formed or reviewed, and whether he is
    /// living it.
    fn keep_his_anchors(&mut self, view: &MindView<'_>, organs: &mut MindOrgans) {
        if MarketSwitches::career_anchors_off() {
            return;
        }
        let s = view.situation;
        let today = view.today();
        let club_id = view.tick.club_id;
        self.anchors.runway = s.career_runway();

        let moved = self.anchors.read_the_move(s, club_id, organs, today);
        if moved.stepped_up {
            self.step_ups = self.step_ups.saturating_add(1);
        }
        if moved.fell {
            self.setbacks = self.setbacks.saturating_add(1);
        }
        self.anchors.record_summit(s, club_id, today);
        if self.anchors.dream.is_some() {
            self.anchors
                .review_dream(s, club_id, self.improvement(), today);
        } else if s.age >= CareerDreamReview::MIN_AGE && s.own_level > 0 {
            self.anchors.form_dream(s, club_id, today);
        }
        self.anchors.read_arrival(s, club_id, today);
        self.pursue_his_dream(view, organs);
    }

    /// What a dream asks of him now, through the wants that already reach
    /// decisions: so a man who means to belong somewhere holds the wish to
    /// stay long before he has the service, and nothing reads the dream twice.
    fn pursue_his_dream(&self, view: &MindView<'_>, organs: &mut MindOrgans) {
        let Some(dream) = self.anchors.dream else {
            return;
        };
        let s = view.situation;
        let today = view.today();
        match dream.kind {
            DreamKind::ReachTheTop if dream.arrived => {
                // Met through progress, not resolved outright: the review has
                // to see the turn for the diary to say so.
                organs.goals.advance(GoalKind::StepUpToABiggerClub, 1.0);
            }
            DreamKind::BelongHere if dream.target_club == view.tick.club_id => {
                organs.goals.pursue(
                    GoalKind::StayAtThisClub,
                    GoalOrigin::Attachment,
                    GoalEvidence::of(&[GoalEvidence::HERITAGE_PULL]),
                    0.3 + 0.5 * dream.conviction,
                    today,
                );
                if dream.arrived {
                    organs.goals.pursue(
                        GoalKind::BecomeAClubLegend,
                        GoalOrigin::Attachment,
                        GoalEvidence::of(&[GoalEvidence::HERITAGE_PULL]),
                        0.6 * dream.conviction,
                        today,
                    );
                }
            }
            DreamKind::FinishAtHome if s.is_abroad => {
                organs.goals.pursue(
                    GoalKind::GoHome,
                    GoalOrigin::Attachment,
                    GoalEvidence::of(&[GoalEvidence::LATE_CAREER]),
                    dream.conviction * (0.3 + 0.7 * s.career_spent()),
                    today,
                );
            }
            _ => {}
        }
    }

    /// The three rungs of wanting to get better.
    ///
    /// Wanting to develop is not wanting a bigger club, and collapsing
    /// the two is what turns every plateau in a simulation into a sale.
    /// A player who has stopped improving wants, in this order: a better
    /// teacher, a role that stretches him, and only when neither arrives,
    /// somewhere else. The first two rungs are things the club can
    /// answer without him going anywhere — which is the point of having
    /// them.
    fn consider_his_development(&mut self, view: &MindView<'_>, organs: &mut MindOrgans) {
        let s = view.situation;
        let today = view.today();

        // What he looks like from outside. He has no other way of
        // judging himself, and neither does anybody else.
        if s.own_level > 0 {
            self.track_progress(s.own_level);
        }

        // A man at the end of his career is not standing still, he is
        // going down, and he knows the difference.
        if self.is_winding_down() {
            return;
        }

        // Improving again answers both rungs at once.
        if self.improvement() > 0.15 {
            organs.goals.advance(GoalKind::KeepImproving, 0.25);
            organs.goals.advance(GoalKind::WorkWithABetterCoach, 0.20);
            return;
        }

        // The coaching here can no longer take a player of his level any
        // further. Not "is this a good staff" — is it good *for him*.
        let shortfall = s.coaching_shortfall();
        if shortfall < 0.15 {
            // It is not the coaching. If he has been improving under it,
            // that is the answer; if he simply is not, the want below
            // still forms, and a role change or a training focus is what
            // resolves it.
            organs.goals.advance(GoalKind::WorkWithABetterCoach, 0.15);
        }

        if !self.feels_stalled() {
            return;
        }

        // How much he minds is ambition and professionalism together: a
        // man with neither has stopped noticing, and both is the player
        // who drives himself.
        let cares = (s.ambition_drive() * 0.6 + s.diligence() * 0.4).clamp(0.0, 1.0);
        // And how much time he has left to do something about it. This
        // is a young player's want above all.
        let want = cares * s.career_runway();
        if want < 0.1 {
            return;
        }

        let mut evidence = GoalEvidence::of(&[GoalEvidence::NO_LONGER_IMPROVING]);
        if shortfall > 0.15 {
            evidence.insert(GoalEvidence::COACHING_CEILING);
        }
        if s.career_spent() > 0.4 {
            evidence.insert(GoalEvidence::PRIME_YEARS_PASSING);
        }

        organs.goals.pursue(
            GoalKind::KeepImproving,
            GoalOrigin::SelfDrive,
            evidence,
            want,
            today,
        );

        // Rung two. He has decided *why* he is not improving, and it is
        // the people teaching him. Still answerable in place — a club
        // that hires a better coach resolves this without losing him,
        // which is a thing that happens in football and which no
        // detector in this simulation could previously express.
        if shortfall > 0.25 {
            organs.goals.pursue(
                GoalKind::WorkWithABetterCoach,
                GoalOrigin::SelfDrive,
                evidence,
                (want * (0.5 + shortfall)).clamp(0.0, 1.0),
                today,
            );
        }
    }

    /// Long service, forked by the kind of man he is.
    ///
    /// Football has at least three answers to having been somewhere a
    /// long time, and which one a player gives is the most legible thing
    /// about his character — so the fork is his loyalty and his ambition
    /// against each other, not the size of the club.
    fn consider_the_long_stay(&mut self, view: &MindView<'_>, organs: &mut MindOrgans, drive: f32) {
        let s = view.situation;
        let today = view.today();

        let mut evidence = GoalEvidence::of(&[GoalEvidence::LONG_SERVICE]);
        let outgrown = s.outgrown_the_club();
        if outgrown > 0.25 {
            evidence.insert(GoalEvidence::OUTGROWN_CLUB);
        }
        if self.honours > 0 {
            evidence.insert(GoalEvidence::NOTHING_LEFT_TO_PROVE);
        }
        if s.career_spent() > 0.5 {
            evidence.insert(GoalEvidence::PRIME_YEARS_PASSING);
        }
        if s.ambition >= Self::AMBITIOUS {
            evidence.insert(GoalEvidence::HIGH_AMBITION);
        }
        if s.days_at_club >= MindSituation::CLUB_SERVANT_DAYS {
            evidence.insert(GoalEvidence::CLUB_SERVANT);
        }

        // A man of ordinary ambition who has been here six years just
        // stays. That is the commonest outcome in football and it should
        // be a decision rather than an absence of rules — so it exits
        // here rather than falling through into a want he does not have.
        if s.ambition < Self::AMBITIOUS {
            return;
        }

        // ── The loyal and ambitious man ─────────────────────────
        //
        // He does not leave. He asks the club to match him — the
        // player's side of a manager wanting to be backed in the market,
        // and the reason a one-club career survives a good player
        // outgrowing his surroundings. He gives them a season to answer.
        if s.loyalty_drive() > 0.6 && outgrown > 0.2 {
            let newly_formed = organs.goals.pursue(
                GoalKind::PlayWithBetterPlayers,
                GoalOrigin::SelfDrive,
                evidence,
                (drive * 0.5 + outgrown * 0.5).clamp(0.0, 1.0),
                today,
            );
            if newly_formed {
                organs
                    .goals
                    .commit_until(GoalKind::PlayWithBetterPlayers, today.saturating_add(365));
            }
            // The squad around him improved. He got what he asked for.
            if outgrown < 0.15 {
                organs.goals.advance(GoalKind::PlayWithBetterPlayers, 0.35);
            }

            // And if they never answered, only then does he look. The
            // flip is what makes the loyalty real rather than decorative
            // — he was always going to go if nothing changed, and he
            // waited a year to find out.
            if organs.goals.status_of(GoalKind::PlayWithBetterPlayers) != GoalStatus::Frustrated {
                return;
            }
            evidence.insert(GoalEvidence::CLUB_IS_SELLING_UP);
        }

        // ── The restless man ────────────────────────────────────
        //
        // What he does about the ceiling is the `StepUp` arc's business,
        // and it pushes `StepUpToABiggerClub` itself from
        // `consider_his_plan`. Form the same want on a second curve here
        // and the same man wants a bigger club at two strengths on the
        // same morning — with the arc, the thing that holds a decision
        // for seasons, the one that loses.
        //
        // What is left here is the want the arc has no rung for: a man
        // who has NOT outgrown the place and simply wants a new test.
        if outgrown <= 0.2 && !self.anchors.dream.is_some_and(|dream| dream.arrived) {
            let restlessness = drive * 0.5 * (1.0 - s.loyalty_drive() * 0.6);
            organs.goals.pursue(
                GoalKind::FindANewChallenge,
                GoalOrigin::SelfDrive,
                evidence,
                restlessness,
                today,
            );
            organs
                .goals
                .set_urgency(GoalKind::FindANewChallenge, s.career_spent());
        }
    }

    /// His country.
    ///
    /// The tournament cycle is the only clock in football that puts a
    /// hard date on a player's club situation. A fringe international
    /// who is not playing in the January before a World Cup will move,
    /// and will drop a level to do it — which is a thing the resignation
    /// path could previously only reach after years of failure.
    fn consider_his_country(&mut self, view: &MindView<'_>, organs: &mut MindOrgans) {
        let s = view.situation;
        let today = view.today();

        if !s.national_standing.is_international() {
            return;
        }

        let pressure = s.tournament_pressure();
        let mut evidence = GoalEvidence::EMPTY;
        if pressure > 0.2 {
            evidence.insert(GoalEvidence::TOURNAMENT_YEAR);
        }

        // Wanting the shirt is ordinary for anyone who has worn it. What
        // changes is the urgency, and the urgency is the calendar.
        let want = (0.3 + s.ambition_drive() * 0.4 + pressure * 0.3).clamp(0.0, 1.0);
        organs.goals.pursue(
            GoalKind::GetIntoTheNationalSquad,
            GoalOrigin::SelfDrive,
            evidence,
            want,
            today,
        );
        organs
            .goals
            .set_urgency(GoalKind::GetIntoTheNationalSquad, pressure);
    }
}

impl CareerMind {
    /// The loan, from whichever end of it he is at. Returns true when he
    /// is out on loan, because the wants below it in `reflect` belong to
    /// a man at his own club and he is not one.
    ///
    /// Three goals in the catalog were formed by nothing before this, and
    /// `MindSituation::is_on_loan` reached no faculty at all. All three
    /// are ordinary football:
    ///
    /// * A boy playing every week somewhere small does not want to go
    ///   back and sit down — he wants to stay where the football is.
    /// * A boy playing every week and doing well wants the opposite as
    ///   soon as the spell ends: his chance at the club that owns him.
    ///   Both are true at once, which is exactly why they are separate
    ///   wants rather than one with a sign.
    /// * And a young player at his own club with no route into the side
    ///   wants out on loan, which is the one want a *club* can grant him
    ///   without losing him.
    fn consider_the_loan(&mut self, view: &MindView<'_>, organs: &mut MindOrgans) -> bool {
        let s = view.situation;
        let today = view.today();

        if !s.is_on_loan {
            // The other end of it. A young man at his own club, settled
            // enough to have seen the pecking order, with nothing to
            // play for.
            let young = s.age <= 23;
            let no_route = s.starter_ratio < 0.20
                && (!s.has_squad_view() || s.pecking_rank >= 3 || s.can_wait_for_the_shirt());
            if young && no_route && s.is_settled() {
                organs.goals.pursue(
                    GoalKind::GoOutOnLoan,
                    GoalOrigin::Survival,
                    GoalEvidence::of(&[GoalEvidence::NO_FIRST_TEAM_FOOTBALL]),
                    (0.35 + s.diligence() * 0.4).clamp(0.0, 1.0),
                    today,
                );
            } else {
                // The route home has opened, or he has outgrown the
                // idea. He never had the loan, so this is not the want
                // being met — it is a want he minds less each week the
                // side keeps picking him, until he has let it go.
                // Advancing it here resolved it as *got*, and his diary
                // read "he got what he wanted: a loan" for a loan that
                // never happened.
                organs.goals.ease(GoalKind::GoOutOnLoan, 0.3);
            }
            return false;
        }

        // He is out on loan and playing. Two wants, pointing at two
        // different clubs, and both of them real.
        organs.goals.resolve(GoalKind::GoOutOnLoan, true);

        if s.is_playing() {
            let going_well = (s.starter_ratio - 0.4).max(0.0) / 0.6;
            organs.goals.pursue(
                GoalKind::StayAtThisLoanClub,
                GoalOrigin::Attachment,
                GoalEvidence::EMPTY,
                (going_well * (1.0 - s.ambition_drive() * 0.5)).clamp(0.0, 1.0),
                today,
            );
            organs.goals.pursue(
                GoalKind::ProveMyselfAtMyParentClub,
                GoalOrigin::SelfDrive,
                GoalEvidence::EMPTY,
                (going_well * (0.3 + s.ambition_drive() * 0.7)).clamp(0.0, 1.0),
                today,
            );
        } else {
            // A loan that is not working is worse than no loan at all —
            // he is losing a season somewhere that does not even own him.
            organs.goals.pursue(
                GoalKind::ProveMyselfAtMyParentClub,
                GoalOrigin::Survival,
                GoalEvidence::of(&[GoalEvidence::NO_FIRST_TEAM_FOOTBALL]),
                0.45,
                today,
            );
            organs.goals.resolve(GoalKind::StayAtThisLoanClub, false);
        }

        true
    }
}

impl CareerMind {
    /// The arc, reviewed and re-formed.
    ///
    /// Three things in order, and the order is the design: a deadline
    /// that has arrived is answered before anything new is considered; a
    /// live plan pushes its own goal onto the stack; and only then does
    /// a rival arc get a chance to displace it — by a clear margin,
    /// because a plan that changes every time a week goes badly is not
    /// a plan.
    fn consider_his_plan(&mut self, view: &MindView<'_>, organs: &mut MindOrgans) {
        if MarketSwitches::career_plan_off() {
            self.plan = None;
            return;
        }
        let s = view.situation;
        let today = view.today();
        let runway = s.career_runway();
        let home_desire = organs.goals.pressure_of(GoalKind::GoHome);

        let anchors = AnchorView::of(&self.anchors, today, s.age, s.own_level);
        if let Some(plan) = self.plan {
            if CareerPlanner::is_answered(&plan, s, &anchors) {
                self.remember_attempts(&plan);
                self.plan = None;
            } else if plan.review_due(today) {
                self.plan = CareerPlanner::review(&plan, s, &anchors, today);
                if plan.arc == CareerArc::StepUp
                    && self.plan.is_some_and(|next| next.arc == CareerArc::StepUp)
                {
                    self.anchors.on_window_missed();
                }
            }
        }

        let candidate = CareerPlanner::arc_for(s, view.tick.club_id, home_desire, &anchors);
        match (self.plan, candidate) {
            (Some(current), Some((arc, _, strength))) if current.arc == arc => {
                // The same circumstances, still there. They harden it.
                if let Some(plan) = self.plan.as_mut() {
                    plan.strength = plan.strength.max(strength);
                }
            }
            (Some(current), Some((arc, origin, strength)))
                if strength > current.strength + CareerPlan::SWITCH_MARGIN =>
            {
                self.remember_attempts(&current);
                self.plan = Some(self.fresh_plan(arc, origin, strength, s, today, runway));
            }
            (None, Some((arc, origin, strength))) => {
                self.plan = Some(self.fresh_plan(arc, origin, strength, s, today, runway));
            }
            // The circumstances that formed it are no longer there. A
            // want nobody feeds fades, and a plan is a want he tells
            // himself — without this one formed at 0.81 could never be
            // displaced, because `strength` only ever rose.
            (Some(current), None) => {
                if let Some(plan) = self.plan.as_mut() {
                    plan.fade(Self::goal_for(current.arc).spec().decay_per_month);
                }
            }
            _ => {}
        }

        let Some(plan) = self.plan else {
            return;
        };
        // The plan is the arc; the goal is how loudly he is living it.
        // Pushing it here is what keeps the two from ever disagreeing,
        // and it is the only write the plan makes to the stack.
        let goal = Self::goal_for(plan.arc);
        organs
            .goals
            .pursue(goal, plan.origin, GoalEvidence::EMPTY, plan.strength, today);
        organs
            .goals
            .set_urgency(goal, plan.deadline_pressure(today).max(s.career_spent()));

        // …and the ladder that want has climbed IS how far along the
        // arc he is. A plan does not need a second escalation model.
        let stage = match organs.goals.status_of(goal) {
            GoalStatus::Voiced | GoalStatus::Pressing => PlanStage::Asking,
            GoalStatus::Active => PlanStage::Committed,
            _ => PlanStage::Forming,
        };
        if let Some(plan) = self.plan.as_mut() {
            plan.escalate(stage);
            // Whatever rung the want is at, a man who is actually away
            // is acting on it.
            if s.is_on_loan && plan.arc == CareerArc::ProveOnLoan {
                plan.escalate(PlanStage::Acting);
            }
        }
    }

    /// A plan formed from scratch, carrying what he has already tried
    /// and the deadline the arc gives itself.
    fn fresh_plan(
        &self,
        arc: CareerArc,
        origin: GoalOrigin,
        strength: f32,
        situation: &MindSituation,
        today: EpochDay,
        runway: f32,
    ) -> CareerPlan {
        let mut plan = CareerPlan::new(
            arc,
            origin,
            strength,
            today,
            CareerPlanner::review_days_for(arc, situation),
            runway,
        );
        plan.attempts = self.attempts;
        plan
    }

    /// Keep the count of what he has tried when a plan ends.
    fn remember_attempts(&mut self, plan: &CareerPlan) {
        self.attempts = self.attempts.max(plan.attempts);
    }

    /// The want each arc is lived through. One-to-one, so the arc is
    /// always legible in the goal stack and the escalation ladder the
    /// mind already has is the plan's own.
    fn goal_for(arc: CareerArc) -> GoalKind {
        match arc {
            CareerArc::BreakThroughHere => GoalKind::WinBackMyPlace,
            CareerArc::ProveOnLoan => GoalKind::GoOutOnLoan,
            CareerArc::ClaimMyPlace => GoalKind::ProveMyselfAtMyParentClub,
            CareerArc::StepUp => GoalKind::StepUpToABiggerClub,
            CareerArc::StepDownToPlay => GoalKind::PlayFirstTeamFootball,
            CareerArc::SettleAtMyLevel => GoalKind::StayAtThisClub,
            CareerArc::FinishAtHome => GoalKind::GoHome,
            CareerArc::StayAndLead => GoalKind::BecomeAClubLegend,
        }
    }

    /// A spell has ended and the parent has read it. The arc answers
    /// the verdict rather than waiting for its own deadline — this is
    /// the milestone the plan exists for.
    pub fn on_loan_spell_reviewed(
        &mut self,
        verdict: LoanSpellVerdict,
        runway: f32,
        loan_band: f32,
        today: EpochDay,
    ) {
        let Some(plan) = self.plan else {
            // He went out without a plan of his own. Coming home with a
            // record IS one.
            if verdict.is_positive() {
                self.plan = Some(CareerPlan::new(
                    CareerArc::ClaimMyPlace,
                    GoalOrigin::SelfDrive,
                    0.5,
                    today,
                    CareerPlan::CLAIM_REVIEW_DAYS,
                    runway,
                ));
            }
            return;
        };
        self.plan = Some(CareerPlanner::after_loan(
            &plan, verdict, runway, loan_band, today,
        ));
    }

    /// He has moved. Whatever arc he was living out was about a club he
    /// no longer plays for — a claim on a shirt somebody else now wears,
    /// a step up he has just taken — so it resolves, and the next weekly
    /// think forms one from where he actually is.
    ///
    /// **A loan return is not this.** He comes back to the club that
    /// owns him, the plan is the thing that carried him through the
    /// spell, and its milestone is the verdict rather than the move.
    pub fn on_club_change(&mut self) {
        self.plan = None;
    }

    /// The plan he is carrying, flattened for everything outside the
    /// mind.
    #[inline]
    pub fn plan_view(&self, today: EpochDay) -> CareerPlanView {
        self.plan
            .as_ref()
            .map(|plan| CareerPlanView::of(plan, today))
            .unwrap_or_default()
    }

    /// Is he living what he dreamed of?
    pub fn has_arrived(&self) -> bool {
        self.anchors.dream.is_some_and(|dream| dream.arrived)
    }

    /// How hard a long deal cuts across what he is reaching for, 0..1: he
    /// believes it, and has the years left to go and get it.
    fn dream_against_signing(&self) -> f32 {
        self.anchors
            .dream
            .filter(|dream| {
                !dream.arrived
                    && matches!(
                        dream.kind,
                        DreamKind::ReachTheTop | DreamKind::PlayForHisClub
                    )
            })
            .map_or(0.0, |dream| {
                dream.conviction * self.anchors.runway * Self::TIED_DOWN
            })
    }

    /// What a career has left him with, flat, for anybody weighing him.
    pub fn anchor_view(&self, today: EpochDay, age: u8, own_level: u8) -> AnchorView {
        AnchorView::of(&self.anchors, today, age, own_level)
    }
}

impl CareerMind {
    /// What a career says about a decision.
    ///
    /// Called through [`SubMind::weigh`]; kept here so the deliberation
    /// rules sit beside the state they read.
    pub(super) fn weigh_option(&self, option: MindOption, organs: &MindOrgans) -> ReasonSet {
        let mut reasons = ReasonSet::new();

        match option {
            MindOption::JoinClub(club_id) => {
                let club = ActorRef::club(club_id);

                // The ten-year return, and the whole point of the memory
                // organ: a player offered his old club is not choosing
                // between two strangers. What he made of the place lasts
                // long after the episodes that taught it to him.
                let broke_through = organs.memory.believes(FactClaim::BrokeThroughHere, club);
                let won_everything = organs.memory.believes(FactClaim::WonEverythingHere, club);
                let sold_against_will = organs
                    .memory
                    .believes(FactClaim::WasSoldAgainstMyWill, club);
                let discarded = organs.memory.believes(FactClaim::DiscardedMe, club);
                let never_played = organs.memory.believes(FactClaim::NeverPlayedHere, club);

                if broke_through > 0.1 || won_everything > 0.1 {
                    reasons.push(
                        GoalKind::PlayForMyBoyhoodClub,
                        broke_through.max(won_everything),
                    );
                }
                if sold_against_will > 0.1 {
                    // Not a refusal. A grudge against a club he made his
                    // name at is an argument he has with himself, and
                    // plenty of players go back anyway.
                    reasons.push(GoalKind::LeaveThisClub, -sold_against_will * 0.7);
                }
                if discarded > 0.1 || never_played > 0.1 {
                    reasons.push(
                        GoalKind::PlayFirstTeamFootball,
                        -discarded.max(never_played),
                    );
                }

                // And what he currently wants out of where he is.
                let step_up = organs.goals.pressure_of(GoalKind::StepUpToABiggerClub);
                let challenge = organs.goals.pressure_of(GoalKind::FindANewChallenge);
                if step_up > 0.1 {
                    reasons.push(GoalKind::StepUpToABiggerClub, step_up);
                }
                if challenge > 0.1 {
                    reasons.push(GoalKind::FindANewChallenge, challenge);
                }
            }

            MindOption::RequestTransfer => {
                let wants_out = organs.goals.wants_to_leave();
                if wants_out > 0.1 {
                    reasons.push(GoalKind::LeaveThisClub, wants_out);
                }
                // Wanting to get better is an argument for moving only
                // once the two rungs below it have gone unanswered —
                // which is what having them as separate wants buys.
                let coach = organs.goals.pressure_of(GoalKind::WorkWithABetterCoach);
                if coach > 0.5 {
                    reasons.push(GoalKind::WorkWithABetterCoach, coach * 0.6);
                }
            }

            MindOption::StayAndFight => {
                if self.improvement() > 0.1 {
                    // Whatever else is wrong, he is getting better here.
                    reasons.push(GoalKind::KeepImproving, self.improvement());
                }
                let stalled = organs.goals.pressure_of(GoalKind::KeepImproving);
                if stalled > 0.3 {
                    reasons.push(GoalKind::KeepImproving, -stalled * 0.5);
                }
            }

            MindOption::SignContract => {
                // A man who has decided to go does not sign on. The loan
                // want is left out on purpose: a club re-signs a boy and
                // then lends him, and he agrees to both.
                let wants_out = [
                    GoalKind::LeaveThisClub,
                    GoalKind::BeAllowedToLeave,
                    GoalKind::FindANewChallenge,
                ]
                .into_iter()
                .map(|kind| organs.goals.pressure_of(kind))
                .fold(0.0f32, f32::max);
                if wants_out > 0.1 {
                    reasons.push(GoalKind::LeaveThisClub, -wants_out);
                }
                // Reaching higher is a reason not to be tied down, which the
                // terms can answer: its own line, so the handler can read it.
                let climbing = organs
                    .goals
                    .pressure_of(GoalKind::StepUpToABiggerClub)
                    .max(self.dream_against_signing());
                if climbing > 0.1 {
                    reasons.push(GoalKind::StepUpToABiggerClub, -climbing);
                }
                if self.has_arrived() {
                    reasons.push(GoalKind::StayAtThisClub, Self::ARRIVED_SIGNS);
                }
            }

            MindOption::Retire => {
                let winding_down = organs.goals.pressure_of(GoalKind::RetireOnMyTerms);
                if winding_down > 0.1 {
                    reasons.push(GoalKind::RetireOnMyTerms, winding_down);
                }
                // A man still winning things does not stop.
                if self.honours > 0 && !self.is_winding_down() {
                    reasons.push(GoalKind::WinATrophy, -0.5);
                }
            }

            _ => {}
        }

        reasons
    }
}

#[cfg(test)]
mod tests {
    use super::super::anchors::CareerDream;
    use super::super::organs::goals::GoalBlocker;
    use super::super::organs::memory::{EpochDay, MindClock};
    use super::super::organs::{MindNoteKind, SpellChange};
    use super::super::situation::MindSituation;
    use super::super::{MindTickContext, PlayerMind};
    use super::*;
    use crate::club::person::PersonAttributes;
    use chrono::NaiveDate;

    fn attrs() -> PersonAttributes {
        PersonAttributes {
            adaptability: 12.0,
            ambition: 12.0,
            controversy: 5.0,
            loyalty: 12.0,
            pressure: 12.0,
            professionalism: 12.0,
            sportsmanship: 12.0,
            temperament: 10.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    fn tick() -> MindTickContext {
        MindTickContext::new(
            NaiveDate::from_ymd_opt(2030, 6, 1).unwrap(),
            7,
            &attrs(),
            50.0,
        )
    }

    fn reflect(mind: &mut CareerMind, situation: &MindSituation, organs: &mut MindOrgans) {
        let tick = tick();
        let view = MindView {
            tick: &tick,
            situation,
        };
        mind.reflect(&view, organs);
    }

    fn episode(kind: EpisodeKind) -> MindEpisode {
        MindEpisode::new(kind, ActorRef::NONE, 7, 100, kind.spec().valence, 0.8)
    }

    /// A regular at a club of this standing, reaching for the very top.
    fn star_at(club_reputation: f32) -> MindSituation {
        MindSituation {
            age: 25,
            ambition: 19.0,
            loyalty: 8.0,
            own_level: 170,
            ceiling: 180,
            club_reputation,
            division_reputation: 9000,
            starter_ratio: 0.95,
            appearances_tracked: 30,
            days_at_club: 3000,
            ..MindSituation::neutral()
        }
    }

    fn today() -> EpochDay {
        MindClock::day(NaiveDate::from_ymd_opt(2030, 6, 1).unwrap())
    }

    #[test]
    fn nobody_at_the_top_wants_a_bigger_club() {
        let s = star_at(0.95);
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        mind.seed_anchors(&s, 7, None, true, today());
        for _ in 0..3 {
            reflect(&mut mind, &s, &mut organs);
        }
        assert!(mind.anchors.dream.unwrap().arrived);
        assert_ne!(
            mind.plan.map(|plan| plan.arc),
            Some(CareerArc::StepUp),
            "he has been here eight years and he has nothing to outgrow"
        );
        assert_eq!(organs.goals.pressure_of(GoalKind::StepUpToABiggerClub), 0.0);
        assert_eq!(organs.goals.pressure_of(GoalKind::FindANewChallenge), 0.0);
    }

    #[test]
    fn a_man_reaching_higher_plans_to_step_up_once_he_has_settled() {
        let s = MindSituation {
            days_at_club: 120,
            ..star_at(0.6)
        };
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        mind.seed_anchors(&s, 7, None, true, today());
        reflect(&mut mind, &s, &mut organs);
        assert_eq!(
            mind.plan.map(|plan| plan.arc),
            Some(CareerArc::StepUp),
            "and he does not wait two seasons to do it"
        );

        let just_arrived = MindSituation {
            days_at_club: 10,
            ..s
        };
        let mut new_man = CareerMind::default();
        new_man.seed_anchors(&just_arrived, 7, None, true, today());
        reflect(&mut new_man, &just_arrived, &mut MindOrgans::new());
        assert_eq!(
            new_man.plan, None,
            "he gives the new club a fair look first"
        );
    }

    #[test]
    fn not_playing_does_not_end_a_dream() {
        let s = MindSituation {
            age: 20,
            ambition: 18.0,
            own_level: 90,
            ceiling: 150,
            club_reputation: 0.9,
            division_reputation: 9000,
            starter_ratio: 0.02,
            appearances_tracked: 30,
            days_at_club: 400,
            pecking_rank: 4,
            rivals_at_position: 4,
            top_rival_age: 24,
            rival_gap: -40,
            ..MindSituation::neutral()
        };
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        mind.seed_anchors(&s, 7, None, true, today());
        let dream = mind.anchors.dream;
        reflect(&mut mind, &s, &mut organs);
        assert_eq!(mind.plan.map(|plan| plan.arc), Some(CareerArc::ProveOnLoan));
        assert_eq!(
            mind.anchors.dream, dream,
            "his plan is the loan, his dream is unchanged"
        );
    }

    #[test]
    fn a_window_closes_on_a_plan_to_step_up() {
        let s = MindSituation {
            days_to_main_window_close: 30,
            ..star_at(0.6)
        };
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        mind.seed_anchors(&s, 7, None, true, today());
        reflect(&mut mind, &s, &mut organs);
        let formed = mind.plan.unwrap();
        assert_eq!(formed.arc, CareerArc::StepUp);

        let mut plan = formed;
        plan.review_on = today();
        mind.plan = Some(plan);
        reflect(&mut mind, &s, &mut organs);

        let after = mind.plan.unwrap();
        assert_eq!(after.arc, CareerArc::StepUp, "he still plans to step up");
        assert_eq!(
            after.attempts,
            formed.attempts + 1,
            "one more attempt counted"
        );
        assert_eq!(
            mind.anchors.dream.unwrap().windows_missed,
            1,
            "and the dream's next review weighs it"
        );
    }

    #[test]
    fn arriving_answers_a_plan_to_step_up() {
        let climbing = star_at(0.6);
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        mind.seed_anchors(&climbing, 7, None, true, today());
        reflect(&mut mind, &climbing, &mut organs);
        assert_eq!(mind.plan.map(|plan| plan.arc), Some(CareerArc::StepUp));

        reflect(&mut mind, &star_at(0.95), &mut organs);
        assert!(mind.anchors.dream.unwrap().arrived);
        assert_ne!(mind.plan.map(|plan| plan.arc), Some(CareerArc::StepUp));
    }

    #[test]
    fn a_man_dreaming_of_belonging_holds_the_stay_want_without_long_service() {
        let s = MindSituation {
            age: 24,
            loyalty: 18.0,
            own_level: 110,
            ceiling: 115,
            club_reputation: 0.6,
            starter_ratio: 0.9,
            appearances_tracked: 30,
            days_at_club: 200,
            ..MindSituation::neutral()
        };
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        mind.anchors.dream = Some(CareerDream {
            kind: DreamKind::BelongHere,
            target_club: 7,
            conviction: 0.8,
            ..CareerDreamReview::form(&s, 7, today())
        });
        reflect(&mut mind, &s, &mut organs);
        assert!(organs.goals.pressure_of(GoalKind::StayAtThisClub) > 0.0);
    }

    #[test]
    fn a_man_dreaming_of_home_wants_to_go_home() {
        let s = MindSituation {
            age: 31,
            is_abroad: true,
            adaptability: 4.0,
            own_level: 110,
            ceiling: 110,
            club_reputation: 0.6,
            days_at_club: 400,
            ..MindSituation::neutral()
        };
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        mind.seed_anchors(&s, 7, None, true, today());
        assert_eq!(mind.anchors.dream.unwrap().kind, DreamKind::FinishAtHome);
        reflect(&mut mind, &s, &mut organs);
        assert!(organs.goals.pressure_of(GoalKind::GoHome) > 0.0);
        assert_eq!(
            mind.plan.map(|plan| plan.arc),
            Some(CareerArc::FinishAtHome)
        );
    }

    #[test]
    fn a_real_step_up_lifts_his_read_of_his_career() {
        let s = MindSituation {
            own_level: 120,
            days_at_club: 400,
            starter_ratio: 0.9,
            appearances_tracked: 30,
            ..MindSituation::neutral()
        };
        let in_a_small_league = MindSituation {
            division_reputation: 6000,
            club_reputation: 0.4,
            ..s
        };
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        reflect(&mut mind, &in_a_small_league, &mut organs);
        let before = mind.trajectory();

        let a_bigger_one = MindSituation {
            division_reputation: 8000,
            club_reputation: 0.7,
            ..s
        };
        let tick = MindTickContext::new(
            NaiveDate::from_ymd_opt(2030, 6, 1).unwrap(),
            9,
            &attrs(),
            50.0,
        );
        mind.reflect(
            &MindView {
                tick: &tick,
                situation: &a_bigger_one,
            },
            &mut organs,
        );
        assert_eq!(mind.step_ups, 1);
        assert!(mind.trajectory() > before, "his read of his career rises");
    }

    #[test]
    fn with_no_dream_the_plan_reads_as_it_always_did() {
        let outgrown = MindSituation {
            age: 24,
            ambition: 18.0,
            own_level: 170,
            club_reputation: 0.3,
            days_at_club: 800,
            starter_ratio: 0.9,
            appearances_tracked: 30,
            ..MindSituation::neutral()
        };
        let (arc, _, _) =
            CareerPlanner::arc_for(&outgrown, 7, 0.0, &AnchorView::default()).unwrap();
        assert_eq!(arc, CareerArc::StepUp);
    }

    #[test]
    fn a_climber_is_slow_to_commit() {
        let at = |ambition: f32, loyalty: f32| MindSituation {
            age: 23,
            ambition,
            loyalty,
            own_level: 150,
            ceiling: 175,
            club_reputation: 0.55,
            division_reputation: 8000,
            starter_ratio: 0.9,
            appearances_tracked: 30,
            days_at_club: 700,
            ..MindSituation::neutral()
        };
        let verdict = |s: &MindSituation| {
            let mut mind = CareerMind::default();
            mind.seed_anchors(s, 7, None, true, today());
            mind.weigh_option(MindOption::SignContract, &MindOrgans::new())
                .net()
        };
        let climber = verdict(&at(19.0, 6.0));
        let team_mate = verdict(&MindSituation {
            ceiling: 150,
            club_reputation: 0.85,
            ..at(9.0, 12.0)
        });
        assert!(climber < team_mate, "{climber} vs {team_mate}");
        assert!(climber < 0.0, "{climber}");
    }

    #[test]
    fn a_man_with_no_dream_left_to_chase_has_no_reason_to_refuse() {
        let s = MindSituation {
            age: 31,
            ambition: 5.0,
            own_level: 100,
            ceiling: 100,
            club_reputation: 0.3,
            ..MindSituation::neutral()
        };
        let mut mind = CareerMind::default();
        mind.anchors.dream = Some(CareerDream {
            kind: DreamKind::MakeACareer,
            target_standing: 0.3,
            ..CareerDream::default_on(today())
        });
        mind.anchors.runway = s.career_runway();
        let verdict = mind.weigh_option(MindOption::SignContract, &MindOrgans::new());
        assert_eq!(verdict.net(), 0.0);
    }

    #[test]
    fn a_man_who_has_arrived_has_a_reason_to_sign() {
        let s = star_at(0.95);
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        mind.seed_anchors(&s, 7, None, true, today());
        reflect(&mut mind, &s, &mut organs);
        assert!(mind.has_arrived());
        assert!(mind.weigh_option(MindOption::SignContract, &organs).net() > 0.0);
    }

    fn sold_with_a_want_to_step_up() -> PlayerMind {
        let ctx = MindTickContext::new(
            NaiveDate::from_ymd_opt(2030, 6, 1).unwrap(),
            7,
            &attrs(),
            50.0,
        );
        let mut mind = PlayerMind::new();
        mind.pursue(
            GoalKind::StepUpToABiggerClub,
            GoalOrigin::SelfDrive,
            GoalEvidence::EMPTY,
            0.7,
            &ctx,
        );
        mind.pursue(
            GoalKind::LeaveThisClub,
            GoalOrigin::Grievance,
            GoalEvidence::EMPTY,
            0.7,
            &ctx,
        );
        mind.on_spell_change(SpellChange::transfer(7, false));
        mind
    }

    #[test]
    fn sold_to_a_smaller_club() {
        let mind = sold_with_a_want_to_step_up();
        let want = mind.goals().get(GoalKind::StepUpToABiggerClub).unwrap();
        assert!(
            want.is_live(),
            "he still wants a bigger club at the new one"
        );
        assert_eq!(
            want.blocked_by,
            GoalBlocker::JustArrived,
            "held back until he has settled"
        );
    }

    #[test]
    fn leaving_is_all_he_asked() {
        let mind = sold_with_a_want_to_step_up();
        assert!(
            mind.goals().get(GoalKind::LeaveThisClub).is_none(),
            "the want to leave is met by leaving"
        );
    }

    #[test]
    fn sold_to_the_level_he_wanted() {
        let s = star_at(0.95);
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        mind.seed_anchors(&star_at(0.6), 7, None, true, today());
        organs.goals.pursue(
            GoalKind::StepUpToABiggerClub,
            GoalOrigin::SelfDrive,
            GoalEvidence::EMPTY,
            0.7,
            today(),
        );
        reflect(&mut mind, &s, &mut organs);
        assert!(mind.anchors.dream.unwrap().arrived);

        let later = today() + 7;
        let report = organs.goals.review(later).expect("a week on");
        organs.journal_tick(Some(&report), None, later);
        assert!(
            organs.journal.iter().any(|note| {
                note.kind == MindNoteKind::WantSatisfied
                    && note.goal == GoalKind::StepUpToABiggerClub
            }),
            "the want to move up is met, and the note says so"
        );
    }

    #[test]
    fn the_stage_advances_and_never_goes_back() {
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();

        reflect(
            &mut mind,
            &MindSituation {
                age: 27,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert_eq!(mind.stage, CareerStage::Prime);

        reflect(
            &mut mind,
            &MindSituation {
                age: 22,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert_eq!(mind.stage, CareerStage::Prime, "nobody gets younger");
    }

    #[test]
    fn honours_and_setbacks_shape_the_trajectory() {
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        assert_eq!(mind.trajectory(), 0.0);

        mind.observe(&episode(EpisodeKind::WonLeagueTitle), &mut organs);
        mind.observe(&episode(EpisodeKind::WonContinentalTrophy), &mut organs);
        assert!(mind.trajectory() > 0.0);

        for _ in 0..4 {
            mind.observe(&episode(EpisodeKind::Relegated), &mut organs);
        }
        assert!(mind.trajectory() < 0.0, "it can go the other way too");
    }

    #[test]
    fn games_at_his_own_club_let_the_loan_want_go_rather_than_meet_it() {
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        let day = MindClock::day(tick().today);

        // Twenty-one, settled, and not in the picture at all.
        let stuck = MindSituation {
            age: 21,
            days_at_club: 400,
            starter_ratio: 0.10,
            ..MindSituation::neutral()
        };
        reflect(&mut mind, &stuck, &mut organs);
        assert!(
            organs.goals.pressure_of(GoalKind::GoOutOnLoan) > 0.0,
            "a young man with no route to the side wants a loan"
        );

        // Then he starts getting games at home. The loan never
        // happened, so it cannot be something he got.
        let playing = MindSituation {
            starter_ratio: 0.60,
            ..stuck
        };
        let mut let_go = 0;
        for week in 1..=16u16 {
            reflect(&mut mind, &playing, &mut organs);
            let Some(report) = organs.goals.review(day + EpochDay::from(week) * 7) else {
                continue;
            };
            for change in report.changes() {
                if change.kind != GoalKind::GoOutOnLoan {
                    continue;
                }
                assert_ne!(
                    change.to,
                    GoalStatus::Satisfied,
                    "a loan he never went on is not a want he got"
                );
                if change.to == GoalStatus::Abandoned {
                    let_go += 1;
                }
            }
        }
        assert_eq!(let_go, 1, "he lets it go instead");
        assert!(organs.goals.get(GoalKind::GoOutOnLoan).is_none());
    }

    #[test]
    fn a_content_player_forms_no_upward_wants() {
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                age: 27,
                ambition: 6.0,
                days_at_club: 2000,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert!(
            organs.goals.is_empty(),
            "not everyone wants more, and the model must allow that"
        );
    }

    #[test]
    fn an_ambitious_man_at_a_small_club_wants_a_bigger_one() {
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                age: 26,
                ambition: 18.0,
                days_at_club: 900,
                club_reputation: 0.3,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert!(organs.goals.pressure_of(GoalKind::StepUpToABiggerClub) > 0.0);
        assert_eq!(
            mind.plan.map(|plan| plan.arc),
            Some(CareerArc::StepUp),
            "the want is the arc's, and the arc is what holds it for a season"
        );
    }

    #[test]
    fn the_same_restlessness_at_a_big_club_is_a_new_challenge() {
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                age: 26,
                ambition: 18.0,
                days_at_club: 900,
                club_reputation: 0.9,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert!(organs.goals.pressure_of(GoalKind::FindANewChallenge) > 0.0);
        assert_eq!(organs.goals.pressure_of(GoalKind::StepUpToABiggerClub), 0.0);
    }

    #[test]
    fn a_recent_arrival_has_not_hit_any_ceiling_yet() {
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                age: 26,
                ambition: 18.0,
                days_at_club: 120,
                club_reputation: 0.3,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert_eq!(organs.goals.pressure_of(GoalKind::StepUpToABiggerClub), 0.0);
    }

    #[test]
    fn time_running_out_sharpens_the_same_want() {
        let build = |age: u8| {
            let mut mind = CareerMind::default();
            let mut organs = MindOrgans::new();
            reflect(
                &mut mind,
                &MindSituation {
                    age,
                    ambition: 18.0,
                    days_at_club: 900,
                    club_reputation: 0.3,
                    ..MindSituation::neutral()
                },
                &mut organs,
            );
            organs
                .goals
                .get(GoalKind::StepUpToABiggerClub)
                .map(|g| g.urgency())
                .unwrap_or(0.0)
        };
        assert!(build(31) > build(24));
    }

    #[test]
    fn a_fading_veteran_thinks_about_stopping() {
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                age: 36,
                starter_ratio: 0.05,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert!(organs.goals.pressure_of(GoalKind::RetireOnMyTerms) > 0.0);
    }

    #[test]
    fn a_veteran_still_first_choice_is_not_thinking_about_it() {
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                age: 36,
                starter_ratio: 0.95,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert_eq!(
            organs.goals.pressure_of(GoalKind::RetireOnMyTerms),
            0.0,
            "he is still enjoying his football"
        );
    }

    #[test]
    fn a_man_winding_down_stops_chasing_a_bigger_club() {
        let mut mind = CareerMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                age: 37,
                ambition: 20.0,
                days_at_club: 2000,
                club_reputation: 0.2,
                starter_ratio: 0.1,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert_eq!(organs.goals.pressure_of(GoalKind::StepUpToABiggerClub), 0.0);
        assert!(organs.goals.pressure_of(GoalKind::RetireOnMyTerms) > 0.0);
    }

    #[test]
    fn appraisal_is_silent_until_there_is_a_career_to_judge() {
        let mind = CareerMind::default();
        let organs = MindOrgans::new();
        assert!(mind.appraise(&organs).is_silent());
    }

    #[test]
    fn a_decorated_career_reads_positive_a_broken_one_negative() {
        let mut organs = MindOrgans::new();

        let mut decorated = CareerMind::default();
        for _ in 0..3 {
            decorated.observe(&episode(EpisodeKind::WonLeagueTitle), &mut organs);
        }
        assert!(decorated.appraise(&organs).weighted() > 0.0);

        let mut broken = CareerMind::default();
        for _ in 0..3 {
            broken.observe(&episode(EpisodeKind::Relegated), &mut organs);
        }
        assert!(broken.appraise(&organs).weighted() < 0.0);
    }

    #[test]
    fn the_clock_helper_agrees_with_the_tick() {
        let tick = tick();
        let situation = MindSituation::neutral();
        let view = MindView {
            tick: &tick,
            situation: &situation,
        };
        assert_eq!(view.today(), MindClock::day(tick.today));
    }
    #[test]
    fn a_loan_return_keeps_the_arc_and_the_verdict_decides_it() {
        let mut mind = CareerMind::default();
        let day = MindClock::day(tick().today);
        mind.plan = Some(CareerPlan::new(
            CareerArc::ProveOnLoan,
            GoalOrigin::Survival,
            0.6,
            day,
            CareerPlan::DEFAULT_REVIEW_DAYS,
            0.8,
        ));

        mind.on_loan_spell_reviewed(LoanSpellVerdict::Standout, 0.8, 0.6, day + 300);

        let plan = mind
            .plan
            .expect("the arc survives the drive home by construction");
        assert_eq!(plan.arc, CareerArc::ClaimMyPlace);
    }

    #[test]
    fn a_move_resolves_the_arc_it_was_about() {
        let mut mind = CareerMind {
            plan: Some(CareerPlan::new(
                CareerArc::ClaimMyPlace,
                GoalOrigin::SelfDrive,
                0.6,
                100,
                CareerPlan::CLAIM_REVIEW_DAYS,
                0.8,
            )),
            ..Default::default()
        };
        mind.on_club_change();
        assert!(
            mind.plan.is_none(),
            "a claim on a shirt somebody else now wears is not a plan"
        );
    }
}
