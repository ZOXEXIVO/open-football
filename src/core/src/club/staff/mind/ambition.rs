//! The ambition mind — where his career is going, and whether this job
//! survives.
//!
//! What it replaces: `job_satisfaction` moving on four nudges and then
//! `if rand::random::<f32>() < resignation_probability`. A manager's
//! reading of his own position is not a die roll — it is a belief built
//! out of what the board has actually done, and it is what makes him
//! start looking before he is pushed.
//!
//! The faculty holds **his** read of the job's security, which is not
//! the board's `ManagerRelationship`. He can be more confident than he
//! should be, and he can be reading the room correctly while the board
//! is still saying the right things in public — which is exactly what
//! `GivenAVoteOfConfidence` is for.

use super::organs::StaffOrgans;
use super::submind::{MindOption, ReasonSet, StaffSubMind, StaffView};
use crate::club::mind::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin};
use crate::club::mind::organs::memory::{
    ActorKind, ActorRef, EpisodeKind, EpochDay, FactClaim, MindClock, MindEpisode,
};
use crate::club::mind::verdict::MoodContribution;

/// Where his career is going, and how safe this job is.
#[derive(Debug, Clone, Copy, Default)]
pub struct AmbitionMind {
    /// The club he currently works for, as an actor. Adopted on first
    /// contact so a manager appointed before his first think already
    /// knows whose badge is on his coat.
    pub club: ActorRef,
    /// −100..=100: does he think this job is safe? His belief.
    security_pct: i8,
    /// 0..=100: how much he wants something bigger than this. Built by
    /// over-performing at a small club, cooled by a build he is in the
    /// middle of.
    restlessness_pct: u8,
    /// Times he has been sacked. Never resets — it is the number that
    /// makes an experienced manager read a wavering board faster than a
    /// young one.
    pub sackings: u8,
    /// Trophies won, across his whole career.
    pub honours: u8,
    /// Seasons at this club, as whole seasons started.
    pub seasons_here: u8,
    /// The day he took this job. The mind is the only thing that knows
    /// — `StaffClubContract` carries an expiry and no start — so tenure
    /// is derived from what he remembers rather than plumbed in.
    pub appointed: EpochDay,
    /// Trophies won at *this* club, as distinct from the career count.
    pub honours_here: u8,
    /// 0..=100: how much of himself is in this squad — the fraction of
    /// the side he brought in, plus the time he has had to shape it.
    ///
    /// This is the counterweight. Left alone, ambition churns managers
    /// every season; investment is why a man in the middle of something
    /// turns down an approach he would have taken a year earlier.
    investment_pct: u8,
}

impl AmbitionMind {
    /// How far one board decision moves his read of the job. Slower than
    /// results: a manager does not conclude he is finished on one
    /// meeting.
    pub const SHIFT: f32 = 0.15;

    /// Security below which he starts believing it is over.
    pub const DOOMED: f32 = -0.45;

    /// Exposure above which the job is in genuine trouble, whatever he
    /// happens to believe.
    pub const IN_TROUBLE: f32 = 0.55;

    #[inline]
    pub fn security(&self) -> f32 {
        self.security_pct as f32 / 100.0
    }

    #[inline]
    pub fn restlessness(&self) -> f32 {
        self.restlessness_pct as f32 / 100.0
    }

    /// How much of himself is in this squad, 0..1.
    #[inline]
    pub fn investment(&self) -> f32 {
        self.investment_pct as f32 / 100.0
    }

    fn shift_security(&mut self, delta: f32) {
        let value = (self.security() + delta).clamp(-1.0, 1.0);
        self.security_pct = (value * 100.0).round() as i8;
    }

    fn shift_restlessness(&mut self, delta: f32) {
        let value = (self.restlessness() + delta).clamp(0.0, 1.0);
        self.restlessness_pct = (value * 100.0).round() as u8;
    }

    /// How readily he reads a board that is going cold. Rises with every
    /// sacking — a man who has been through it twice sees it coming.
    #[inline]
    pub fn cynicism(&self) -> f32 {
        (self.sackings as f32 / 4.0).clamp(0.0, 1.0)
    }

    /// How long he has been in this job, in months. Zero when he has
    /// no record of taking it.
    pub fn months_in_the_job(&self, today: EpochDay) -> u16 {
        if self.appointed == 0 {
            return 0;
        }
        MindClock::elapsed(self.appointed, today) / 30
    }

    /// He has taken a job. Everything about the old one is behind him
    /// except what he learned.
    pub fn on_appointment(&mut self, club: ActorRef, today: EpochDay) {
        self.club = club;
        self.security_pct = 35;
        self.restlessness_pct = 0;
        self.seasons_here = 0;
        self.investment_pct = 0;
        self.honours_here = 0;
        self.appointed = today;
    }

    /// He has lost one.
    pub fn on_departure(&mut self, sacked: bool) {
        if sacked {
            self.sackings = self.sackings.saturating_add(1);
        }
        self.club = ActorRef::NONE;
        self.security_pct = 0;
        self.seasons_here = 0;
        self.investment_pct = 0;
        self.honours_here = 0;
        self.appointed = 0;
    }
}

impl StaffSubMind for AmbitionMind {
    fn domain(&self) -> GoalDomain {
        GoalDomain::Management
    }

    fn observe(&mut self, episode: &MindEpisode, organs: &mut StaffOrgans) {
        if self.club.is_none() && episode.who.kind == ActorKind::Club {
            self.club = episode.who;
        }

        match episode.kind {
            EpisodeKind::AppointedManager | EpisodeKind::PromotedFromWithin => {
                self.on_appointment(episode.who, episode.when)
            }
            EpisodeKind::SackedByClub => {
                self.on_departure(true);
                // The one want in the catalog that only exists because
                // memory does. The goal stack holds no target of its
                // own — the club it is pointed at is the `TheySackedMe`
                // conviction memory forms from the same episode, which
                // is what makes a fixture against them a big match for
                // him years later.
                organs.shared.goals.pursue(
                    GoalKind::ProveThemWrong,
                    GoalOrigin::Grievance,
                    GoalEvidence::of(&[GoalEvidence::MANAGER_DOES_NOT_RATE_HIM]),
                    0.85,
                    episode.when,
                );
            }
            EpisodeKind::ResignedFromClub => self.on_departure(false),
            EpisodeKind::WonLeagueTitle
            | EpisodeKind::WonContinentalTrophy
            | EpisodeKind::WonDomesticCup => {
                self.honours = self.honours.saturating_add(1);
                self.honours_here = self.honours_here.saturating_add(1);
                self.shift_security(Self::SHIFT * 1.5);
                // Winning things where nobody expected it is the thing
                // that gets a manager noticed.
                self.shift_restlessness(0.12);
            }
            EpisodeKind::Promoted | EpisodeKind::SurvivedARelegationFight => {
                self.shift_security(Self::SHIFT * 1.4)
            }
            EpisodeKind::WonManagerOfTheMonth => {
                self.shift_security(Self::SHIFT * 0.4);
                self.shift_restlessness(0.06);
            }
            EpisodeKind::FailedToSurviveIt | EpisodeKind::Relegated => {
                self.shift_security(-Self::SHIFT * 2.0)
            }
            // The public vote of confidence. In football this is what a
            // board says on the way to sacking someone, and a manager
            // who has been sacked before reads it that way immediately.
            EpisodeKind::GivenAVoteOfConfidence => {
                self.shift_security(-Self::SHIFT * (0.4 + self.cynicism()))
            }
            EpisodeKind::ChairmanUndercutMePublicly => self.shift_security(-Self::SHIFT * 1.2),
            EpisodeKind::BoardBackedMeInTheWindow => self.shift_security(Self::SHIFT * 0.8),
            EpisodeKind::BoardBrokeItsPromise => self.shift_security(-Self::SHIFT),
            EpisodeKind::LostTheDressingRoom => self.shift_security(-Self::SHIFT * 1.6),
            EpisodeKind::SupportersTurnedOnMe => self.shift_security(-Self::SHIFT * 0.7),
            EpisodeKind::MediaWroteMeOff => self.shift_security(-Self::SHIFT * 0.3),
            _ => {}
        }
    }

    fn reflect(&mut self, view: &StaffView<'_>, organs: &mut StaffOrgans) {
        let s = view.situation;
        let today = view.today();

        // A different badge. Adopt it rather than carrying the last
        // club's read onto this one.
        let club = if view.tick.club_id == 0 {
            ActorRef::NONE
        } else {
            ActorRef::club(view.tick.club_id)
        };
        if club.is_some() && club != self.club {
            self.on_appointment(club, today);
        }

        // His read drifts toward what the job actually looks like, at a
        // rate set by how much attention he pays to the signs. A man who
        // has been sacked before closes the gap faster.
        let truth = 1.0 - s.job_exposure() * 2.0;
        let rate = 0.10 + self.cynicism() * 0.18;
        self.shift_security((truth - self.security()) * rate);

        // Ambition builds where he is outgrowing the club, and cools
        // where he is in the middle of something.
        let outgrown = s.outgrown_the_club();
        self.investment_pct = ((s.squad_is_his * 0.6
            + (s.months_in_the_job.min(48) as f32 / 48.0) * 0.4)
            .clamp(0.0, 1.0)
            * 100.0)
            .round() as u8;
        self.seasons_here = (s.months_in_the_job / 12) as u8;
        self.shift_restlessness(outgrown * 0.10 - self.investment() * 0.04);

        // ── This job ────────────────────────────────────────────
        let exposure = s.job_exposure();
        if exposure > 0.25 {
            let mut evidence = GoalEvidence::EMPTY;
            if s.board_trust < 0.4 {
                evidence.insert(GoalEvidence::MANAGER_DOES_NOT_RATE_HIM);
            }
            if s.against_expectation() < -0.2 {
                evidence.insert(GoalEvidence::NOT_A_CONTENDER);
            }
            organs.shared.goals.pursue(
                GoalKind::KeepThisJob,
                GoalOrigin::Survival,
                evidence,
                exposure * 0.55,
                today,
            );
            // Survival is measured against the board, not the calendar:
            // a manager whose board has stopped believing in him is out
            // of time whatever the fixture list says.
            organs
                .shared
                .goals
                .set_urgency(GoalKind::KeepThisJob, exposure);
        }

        let danger = s.relegation_danger();
        if danger > 0.2 {
            organs.shared.goals.pursue(
                GoalKind::SurviveTheSeason,
                GoalOrigin::Survival,
                GoalEvidence::of(&[GoalEvidence::RELEGATED]),
                danger * 0.6,
                today,
            );
            organs
                .shared
                .goals
                .set_urgency(GoalKind::SurviveTheSeason, s.season_progress);
        }

        // Wanting to win something here is what keeps a manager at a
        // club he could leave. It builds with time and with a squad he
        // has made his own.
        if s.months_in_the_job > 6 && s.trophies_here == 0 && exposure < Self::IN_TROUBLE {
            organs.shared.goals.pursue(
                GoalKind::WinSomethingHere,
                GoalOrigin::SelfDrive,
                GoalEvidence::EMPTY,
                0.10 + s.squad_is_his * 0.15,
                today,
            );
        }

        // ── The next one ────────────────────────────────────────
        if self.restlessness() > 0.35 && outgrown > 0.1 {
            let mut evidence = GoalEvidence::of(&[GoalEvidence::OUTGROWN_CLUB]);
            if s.club_standing < 0.35 {
                evidence.insert(GoalEvidence::LEAGUE_IS_A_CEILING);
            }
            organs.shared.goals.pursue(
                GoalKind::GetABiggerJob,
                GoalOrigin::SelfDrive,
                evidence,
                self.restlessness() * 0.35,
                today,
            );
        }

        // The terminus. A manager who believes it is over and has been
        // through it before does not wait to be told.
        if self.security() < Self::DOOMED {
            organs.shared.goals.pursue(
                GoalKind::GetOutOfHere,
                GoalOrigin::Grievance,
                GoalEvidence::of(&[GoalEvidence::MANAGER_DOES_NOT_RATE_HIM]),
                0.25 + self.cynicism() * 0.20,
                today,
            );
        }

        // A safe job answers the want to keep it.
        if exposure < 0.2 {
            organs.shared.goals.advance(GoalKind::KeepThisJob, 0.12);
        }
        if s.trophies_here > 0 {
            organs.shared.goals.advance(GoalKind::WinSomethingHere, 0.5);
        }
    }

    fn appraise(&self, organs: &StaffOrgans) -> MoodContribution {
        if self.club.is_none() {
            return MoodContribution::silent(GoalDomain::Management);
        }

        // Security is most of it; the pressure of a want he cannot
        // satisfy is the rest.
        let pressing = organs.pressure_in(GoalDomain::Management);
        let value = self.security() * 6.0 - pressing * 4.0;
        // He is sure of his own read in proportion to how long he has
        // been here to form one.
        let confidence = (0.3 + self.seasons_here as f32 * 0.2).clamp(0.0, 1.0);
        MoodContribution::new(GoalDomain::Management, value, confidence)
    }

    fn weigh(&self, option: MindOption, organs: &StaffOrgans) -> ReasonSet {
        let mut reasons = ReasonSet::new();

        match option {
            MindOption::TakeTheJob(club_id) => {
                let club = ActorRef::club(club_id);

                // What he remembers about the place. This is the whole
                // gap the plan closes: today a manager offered his old
                // club has no memory of it at all.
                let sacked_here = organs.memory().believes(FactClaim::TheySackedMe, club);
                let never_backed = organs.memory().believes(FactClaim::TheyNeverBackedMe, club);
                let graveyard = organs
                    .memory()
                    .believes(FactClaim::ThatPlaceWasAGraveyard, club);
                let built = organs
                    .memory()
                    .believes(FactClaim::IBuiltSomethingThere, club);
                let kept_word = organs.memory().believes(FactClaim::TheyKeptTheirWord, club);

                if never_backed > 0.1 {
                    reasons.push(GoalKind::BeBackedInTheMarket, -never_backed);
                }
                if graveyard > 0.1 {
                    reasons.push(GoalKind::WinSomethingHere, -graveyard);
                }
                if built > 0.1 || kept_word > 0.1 {
                    reasons.push(GoalKind::WinSomethingHere, built.max(kept_word));
                }
                // Being sacked by a club cuts both ways, and which way
                // depends on the want he formed at the time. A man
                // carrying `ProveThemWrong` about this club wants the
                // job precisely because of what happened.
                if sacked_here > 0.1 {
                    let vengeance = organs.shared.goals.pressure_of(GoalKind::ProveThemWrong);
                    reasons.push(GoalKind::ProveThemWrong, vengeance - sacked_here * 0.6);
                }

                // Ambition, and the counterweight.
                if self.restlessness() > 0.2 {
                    reasons.push(GoalKind::GetABiggerJob, self.restlessness());
                }
                // The counterweight is what he has already put in, not
                // what he is still missing: a manager halfway through
                // building a side has more to lose by leaving than one
                // still trying to make an inherited squad his own.
                let invested = self.investment();
                if invested > 0.05 {
                    reasons.push(GoalKind::WinSomethingHere, -invested);
                }
                // A man who thinks he is about to be sacked takes calls
                // he would not otherwise take.
                if self.security() < 0.0 {
                    reasons.push(GoalKind::KeepThisJob, -self.security());
                }
            }

            MindOption::Resign => {
                if self.security() < Self::DOOMED {
                    reasons.push(GoalKind::GetOutOfHere, -self.security());
                }
                let out = organs.shared.goals.pressure_of(GoalKind::GetOutOfHere);
                if out > 0.05 {
                    reasons.push(GoalKind::GetOutOfHere, out);
                }
                let keep = organs.shared.goals.pressure_of(GoalKind::KeepThisJob);
                if keep > 0.05 {
                    reasons.push(GoalKind::KeepThisJob, -keep);
                }
            }

            _ => {}
        }

        reasons
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::person::PersonAttributes;
    use crate::club::staff::mind::{StaffMind, StaffSituation, StaffTickContext};
    use chrono::{Duration, NaiveDate};

    /// Fixture builders. Grouped on a type rather than left loose, so
    /// the file reads as `Fixture::context()` at every call site.
    struct Fixture;

    impl Fixture {
        fn attributes() -> PersonAttributes {
            PersonAttributes {
                adaptability: 12.0,
                ambition: 14.0,
                controversy: 5.0,
                loyalty: 10.0,
                pressure: 12.0,
                professionalism: 14.0,
                sportsmanship: 12.0,
                temperament: 10.0,
                consistency: 12.0,
                important_matches: 12.0,
                dirtiness: 4.0,
            }
        }

        fn date(year: i32, month: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(year, month, day).expect("valid fixture date")
        }

        fn context(date: NaiveDate, club: u32) -> StaffTickContext {
            StaffTickContext::new(date, club, &Self::attributes(), 50.0)
        }
    }

    impl Fixture {
        /// A manager doing well at a club he has outgrown.
        fn thriving() -> StaffSituation {
            let mut s = StaffSituation::neutral();
            s.standing = 0.72;
            s.club_standing = 0.35;
            s.board_trust = 0.85;
            s.league_size = 20;
            s.expected_position = 14;
            s.league_position = 5;
            s.months_in_the_job = 30;
            s
        }

        /// A manager whose board has stopped believing in him.
        fn sinking() -> StaffSituation {
            let mut s = StaffSituation::neutral();
            s.board_trust = 0.12;
            s.board_pressure = 0.8;
            s.league_size = 20;
            s.expected_position = 9;
            s.league_position = 19;
            s.season_progress = 0.8;
            s.months_in_the_job = 14;
            s
        }

        fn think(mind: &mut StaffMind, situation: &StaffSituation, weeks: u16, club: u32) {
            let start = Self::date(2030, 8, 1);
            for week in 0..weeks {
                let date = start + Duration::days(week as i64 * 7);
                mind.tick_with(&Self::context(date, club), situation);
            }
        }
    }

    #[test]
    fn a_manager_under_pressure_wants_to_keep_his_job() {
        let mut mind = StaffMind::new();
        Fixture::think(&mut mind, &Fixture::sinking(), 8, 7);

        assert!(
            mind.pressure_of(GoalKind::KeepThisJob) > 0.3,
            "survival is the want, and it is loud"
        );
        assert!(mind.pressure_of(GoalKind::SurviveTheSeason) > 0.0);
        assert!(mind.ambition.security() < 0.0, "and he can read it");
    }

    #[test]
    fn a_manager_outgrowing_a_small_club_starts_looking() {
        let mut mind = StaffMind::new();
        Fixture::think(&mut mind, &Fixture::thriving(), 40, 7);

        assert!(
            mind.pressure_of(GoalKind::GetABiggerJob) > 0.0,
            "over-achieving at a small club is how a manager gets restless"
        );
        assert!(mind.ambition.security() > 0.3);
    }

    #[test]
    fn being_sacked_creates_a_want_pointed_at_a_specific_club() {
        let mut mind = StaffMind::new();
        let c = Fixture::context(Fixture::date(2030, 5, 20), 7);
        mind.remember(EpisodeKind::SackedByClub, ActorRef::club(7), &c);

        assert!(mind.pressure_of(GoalKind::ProveThemWrong) > 0.0);
        assert_eq!(mind.ambition.sackings, 1);
    }

    #[test]
    fn a_vote_of_confidence_is_bad_news_and_worse_the_second_time() {
        let mut first_job = StaffMind::new();
        let mut been_here_before = StaffMind::new();
        been_here_before.ambition.sackings = 3;

        let c = Fixture::context(Fixture::date(2030, 11, 4), 7);
        first_job.remember(EpisodeKind::GivenAVoteOfConfidence, ActorRef::board(7), &c);
        been_here_before.remember(EpisodeKind::GivenAVoteOfConfidence, ActorRef::board(7), &c);

        assert!(first_job.ambition.security() < 0.0);
        assert!(
            been_here_before.ambition.security() < first_job.ambition.security(),
            "a man who has been sacked before knows what it means"
        );
    }

    #[test]
    fn a_new_appointment_wipes_the_read_of_the_last_job() {
        let mut mind = StaffMind::new();
        Fixture::think(&mut mind, &Fixture::sinking(), 8, 7);
        assert!(mind.ambition.security() < 0.0);

        let c = Fixture::context(Fixture::date(2031, 6, 1), 9);
        mind.remember(EpisodeKind::AppointedManager, ActorRef::club(9), &c);

        assert!(
            mind.ambition.security() > 0.0,
            "a new job starts with cautious optimism, not the last board's verdict"
        );
        assert_eq!(mind.ambition.club, ActorRef::club(9));
    }

    #[test]
    fn a_manager_with_nowhere_to_be_has_nothing_to_say_about_it() {
        let mind = StaffMind::new();
        assert!(mind.ambition.appraise(&mind.organs).is_silent());
    }
}
