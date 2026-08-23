//! The philosophy mind — how he believes football should be played, and
//! how far he will bend.
//!
//! The faculty with **no player equivalent**. A conviction about the
//! game is what makes two managers with identical attributes behave
//! differently, and it is the axis on which `ManagerRelationship`'s
//! `style_alignment` finally has a counterparty: today the board has an
//! opinion about the manager's football and the manager has none about
//! the board's.
//!
//! The interesting state is not the conviction on its own — it is the
//! gap between what he believes and what he is currently doing. A man
//! playing football he does not believe in is unhappy in a way no
//! results-based reading can express, and a man who refuses to bend when
//! the results demand it gets sacked for it. Both are correct outcomes.

use super::organs::StaffOrgans;
use super::submind::{MindOption, ReasonSet, StaffSubMind, StaffView};
use crate::club::mind::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin};
use crate::club::mind::organs::memory::{ActorRef, EpisodeKind, FactClaim, MindEpisode};
use crate::club::mind::verdict::MoodContribution;

/// His football, and what he is currently doing about it.
#[derive(Debug, Clone, Copy)]
pub struct PhilosophyMind {
    /// 0..=100: how firmly he holds his idea of the game. High
    /// conviction is not a virtue here — it is a trade, and the trade is
    /// what makes a manager interesting.
    conviction_pct: u8,
    /// −100..=100: how far he has bent from it. Negative means he is
    /// playing football he does not believe in; positive means results
    /// have let him go further toward his idea than the squad really
    /// supports.
    bent_pct: i8,
    /// Gambles that came off, and gambles that did not. The evidence he
    /// judges his own football by.
    pub gambles_paid: u8,
    pub gambles_failed: u8,
}

impl Default for PhilosophyMind {
    fn default() -> Self {
        PhilosophyMind {
            // A manager with no history has an idea of the game but has
            // not had to defend it. Mid-range, not zero.
            conviction_pct: 55,
            bent_pct: 0,
            gambles_paid: 0,
            gambles_failed: 0,
        }
    }
}

impl PhilosophyMind {
    /// How far one result moves how far he has bent.
    pub const BEND: f32 = 0.12;

    /// Bend below which he is visibly not managing his own team.
    pub const COMPROMISED: f32 = -0.45;

    #[inline]
    pub fn conviction(&self) -> f32 {
        self.conviction_pct as f32 / 100.0
    }

    /// How far he has bent from his idea. Negative is a compromise.
    #[inline]
    pub fn bent(&self) -> f32 {
        self.bent_pct as f32 / 100.0
    }

    /// How willing he is to change what he does under pressure. The
    /// inverse of conviction, floored: nobody is completely immovable.
    #[inline]
    pub fn flexibility(&self) -> f32 {
        (1.0 - self.conviction()).max(0.1)
    }

    /// Is he currently managing a team that plays his football?
    #[inline]
    pub fn is_compromised(&self) -> bool {
        self.bent() < Self::COMPROMISED
    }

    fn shift_conviction(&mut self, delta: f32) {
        let value = (self.conviction() + delta).clamp(0.0, 1.0);
        self.conviction_pct = (value * 100.0).round() as u8;
    }

    fn shift_bent(&mut self, delta: f32) {
        let value = (self.bent() + delta).clamp(-1.0, 1.0);
        self.bent_pct = (value * 100.0).round() as i8;
    }

    /// A new job is a chance to play his own football again.
    pub fn on_club_change(&mut self) {
        self.bent_pct = 0;
    }
}

impl StaffSubMind for PhilosophyMind {
    fn domain(&self) -> GoalDomain {
        GoalDomain::Philosophy
    }

    fn observe(&mut self, episode: &MindEpisode, _organs: &mut StaffOrgans) {
        match episode.kind {
            EpisodeKind::MyGambleCameOff => {
                self.gambles_paid = self.gambles_paid.saturating_add(1);
                self.shift_conviction(0.06);
                self.shift_bent(Self::BEND);
            }
            EpisodeKind::MyGambleBackfired => {
                self.gambles_failed = self.gambles_failed.saturating_add(1);
                // A failed gamble does not shake a conviction as much as
                // a successful one confirms it. That asymmetry is what
                // keeps a manager with an idea from abandoning it after
                // one bad night — and what gets him sacked.
                self.shift_conviction(-0.03);
                self.shift_bent(-Self::BEND * 0.5);
            }
            EpisodeKind::WonLeagueTitle
            | EpisodeKind::WonContinentalTrophy
            | EpisodeKind::Promoted => self.shift_conviction(0.08),
            EpisodeKind::HeavyDefeat => self.shift_bent(-Self::BEND * 0.4),
            EpisodeKind::Relegated | EpisodeKind::FailedToSurviveIt => {
                self.shift_conviction(-0.10);
                self.shift_bent(-Self::BEND);
            }
            EpisodeKind::AppointedManager | EpisodeKind::PromotedFromWithin => {
                self.on_club_change()
            }
            _ => {}
        }
    }

    fn reflect(&mut self, view: &StaffView<'_>, organs: &mut StaffOrgans) {
        let s = view.situation;
        let today = view.today();

        // Under pressure, a manager either bends or does not, and which
        // one is decided by his conviction rather than by a threshold.
        // A results crisis pushes; conviction pushes back; the squad
        // being his own makes his football easier to play.
        let crisis = s.job_exposure().max(s.relegation_danger());
        let push = crisis * self.flexibility();
        let support = s.squad_is_his * 0.35;
        self.shift_bent((support - push) * Self::BEND);

        // Time and success drift him back toward his own idea.
        if s.against_expectation() > 0.15 {
            self.shift_bent(Self::BEND * 0.3);
        }

        // A manager playing football he does not believe in wants his
        // own squad more than anything else — it is the only route back
        // to managing the way he means to.
        if self.is_compromised() {
            organs.shared.goals.pursue(
                GoalKind::GetMyOwnSquad,
                GoalOrigin::SelfDrive,
                GoalEvidence::of(&[GoalEvidence::SQUAD_BELOW_HIS_LEVEL]),
                -self.bent() * 0.35,
                today,
            );
        }
    }

    fn appraise(&self, organs: &StaffOrgans) -> MoodContribution {
        // A manager always has a view on his own football, but how
        // strongly it weighs on him scales with how much he holds it.
        let value = self.bent() * 6.0 * self.conviction();
        let teacher = organs
            .memory()
            .believes(FactClaim::IAmATeacherNotAWinner, ActorRef::NONE);
        // Self-knowledge makes the reading firmer, either way.
        let confidence = (0.35 + self.conviction() * 0.4 + teacher * 0.25).clamp(0.0, 1.0);
        MoodContribution::new(GoalDomain::Philosophy, value, confidence)
    }

    fn weigh(&self, option: MindOption, organs: &StaffOrgans) -> ReasonSet {
        let mut reasons = ReasonSet::new();

        match option {
            MindOption::ChangeTheSystem => {
                // The whole trade, in one number. A man of conviction
                // argues against changing; a flexible one shrugs.
                reasons.push(GoalKind::WinSomethingHere, -self.conviction());
                let crisis = organs.shared.goals.pressure_of(GoalKind::SurviveTheSeason);
                if crisis > 0.05 {
                    reasons.push(GoalKind::SurviveTheSeason, crisis);
                }
            }

            MindOption::TakeTheJob(_) => {
                // A compromised manager is readier to move than a
                // comfortable one — a new job is where his football
                // gets to be his own again.
                if self.is_compromised() {
                    reasons.push(GoalKind::GetMyOwnSquad, -self.bent() * 0.6);
                }
            }

            MindOption::Resign => {
                if self.is_compromised() {
                    reasons.push(GoalKind::GetOutOfHere, -self.bent() * self.conviction());
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
                ambition: 12.0,
                controversy: 5.0,
                loyalty: 12.0,
                pressure: 12.0,
                professionalism: 13.0,
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
        fn crisis() -> StaffSituation {
            let mut s = StaffSituation::neutral();
            s.board_trust = 0.2;
            s.league_size = 20;
            s.expected_position = 8;
            s.league_position = 18;
            s.season_progress = 0.7;
            s.squad_is_his = 0.2;
            s
        }

        fn think(mind: &mut StaffMind, situation: &StaffSituation, weeks: i64) {
            let start = Self::date(2030, 8, 1);
            for week in 0..weeks {
                let date = start + Duration::days(week * 7);
                mind.tick_with(&Self::context(date, 7), situation);
            }
        }
    }

    #[test]
    fn two_managers_in_the_same_crisis_do_different_things() {
        let mut immovable = StaffMind::new();
        immovable.philosophy.conviction_pct = 95;
        let mut pragmatic = StaffMind::new();
        pragmatic.philosophy.conviction_pct = 15;

        Fixture::think(&mut immovable, &Fixture::crisis(), 20);
        Fixture::think(&mut pragmatic, &Fixture::crisis(), 20);

        assert!(
            pragmatic.philosophy.bent() < immovable.philosophy.bent(),
            "the pragmatist bends and the man of conviction does not: {} vs {}",
            pragmatic.philosophy.bent(),
            immovable.philosophy.bent()
        );
        assert!(pragmatic.philosophy.is_compromised());
    }

    #[test]
    fn a_gamble_that_comes_off_hardens_a_conviction_more_than_one_that_fails_softens_it() {
        let mut paid = StaffMind::new();
        let mut failed = StaffMind::new();
        let c = Fixture::context(Fixture::date(2030, 10, 5), 7);

        let base = paid.philosophy.conviction();
        paid.remember(EpisodeKind::MyGambleCameOff, ActorRef::NONE, &c);
        failed.remember(EpisodeKind::MyGambleBackfired, ActorRef::NONE, &c);

        let gained = paid.philosophy.conviction() - base;
        let lost = base - failed.philosophy.conviction();
        assert!(gained > lost, "{gained} should exceed {lost}");
    }

    #[test]
    fn a_compromised_manager_wants_his_own_squad() {
        let mut mind = StaffMind::new();
        mind.philosophy.conviction_pct = 20;
        Fixture::think(&mut mind, &Fixture::crisis(), 24);

        assert!(mind.philosophy.is_compromised());
        assert!(mind.pressure_of(GoalKind::GetMyOwnSquad) > 0.0);
    }

    #[test]
    fn a_new_job_lets_him_manage_his_own_way_again() {
        let mut mind = StaffMind::new();
        mind.philosophy.conviction_pct = 20;
        Fixture::think(&mut mind, &Fixture::crisis(), 24);
        assert!(mind.philosophy.bent() < 0.0);

        let c = Fixture::context(Fixture::date(2031, 6, 1), 9);
        mind.remember(EpisodeKind::AppointedManager, ActorRef::club(9), &c);
        assert_eq!(mind.philosophy.bent(), 0.0);
    }

    #[test]
    fn conviction_is_what_argues_against_changing_the_system() {
        let mut immovable = StaffMind::new();
        immovable.philosophy.conviction_pct = 95;
        let mut pragmatic = StaffMind::new();
        pragmatic.philosophy.conviction_pct = 15;

        let held = immovable
            .philosophy
            .weigh(MindOption::ChangeTheSystem, &immovable.organs);
        let shrugged = pragmatic
            .philosophy
            .weigh(MindOption::ChangeTheSystem, &pragmatic.organs);

        assert!(held.net() < shrugged.net());
    }
}
