//! The welfare mind — the workload, the life, and whether he still
//! wants to do this.
//!
//! What it replaces: `if rand::random::<f32>() < resignation_probability`.
//! A manager walking away is the end of a process, not a coin flip —
//! strain accumulates faster than it clears, a life outside the game
//! either supports it or does not, and a career has an end that arrives
//! by degrees.
//!
//! It is also the faculty that owns the one genuinely good thing about
//! being sacked: it is a rest.

use super::organs::StaffOrgans;
use super::submind::{MindOption, ReasonSet, StaffSubMind, StaffView};
use crate::club::mind::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin};
use crate::club::mind::organs::memory::{EpisodeKind, MindEpisode};
use crate::club::mind::verdict::MoodContribution;

/// What the job is costing him.
#[derive(Debug, Clone, Copy, Default)]
pub struct WelfareMind {
    /// 0..=100: accumulated strain. Rises with load and crisis, clears
    /// slowly, and clears fastest when he is out of work.
    strain_pct: u8,
    /// −100..=100: how much he still wants to do this. The number the
    /// die roll was standing in for.
    appetite_pct: i8,
    /// Seasons managed, anywhere. Never resets.
    pub seasons_in_the_game: u8,
    /// Something outside football is going on.
    pub life_is_hard: bool,
}

impl WelfareMind {
    /// How much of the gap to the current load strain closes per think.
    /// Deliberately asymmetric — see [`Self::RECOVERY`].
    pub const ACCRUAL: f32 = 0.06;

    /// And how much it clears when the load is off. A season takes
    /// longer to recover from than it took to accumulate, which is why
    /// managers take years out.
    pub const RECOVERY: f32 = 0.012;

    /// Appetite below which he is done.
    pub const SPENT: f32 = -0.55;

    #[inline]
    pub fn strain(&self) -> f32 {
        self.strain_pct as f32 / 100.0
    }

    #[inline]
    pub fn appetite(&self) -> f32 {
        self.appetite_pct as f32 / 100.0
    }

    /// Is he burnt out? Not a threshold on fatigue — strain that has
    /// outlasted the appetite to carry it.
    #[inline]
    pub fn is_burnt_out(&self) -> bool {
        self.strain() > 0.7 && self.appetite() < 0.0
    }

    fn shift_strain(&mut self, delta: f32) {
        let value = (self.strain() + delta).clamp(0.0, 1.0);
        self.strain_pct = (value * 100.0).round() as u8;
    }

    fn shift_appetite(&mut self, delta: f32) {
        let value = (self.appetite() + delta).clamp(-1.0, 1.0);
        self.appetite_pct = (value * 100.0).round() as i8;
    }
}

impl StaffSubMind for WelfareMind {
    fn domain(&self) -> GoalDomain {
        GoalDomain::Welfare
    }

    fn observe(&mut self, episode: &MindEpisode, _organs: &mut StaffOrgans) {
        match episode.kind {
            // The one good thing about it.
            EpisodeKind::SackedByClub => {
                self.shift_strain(-0.15);
                self.shift_appetite(-0.12);
            }
            EpisodeKind::ResignedFromClub => {
                self.shift_strain(-0.25);
                self.shift_appetite(-0.20);
            }
            EpisodeKind::AppointedManager | EpisodeKind::PromotedFromWithin => {
                self.seasons_in_the_game = self.seasons_in_the_game.saturating_add(1);
                self.shift_appetite(0.25);
            }
            EpisodeKind::WonLeagueTitle
            | EpisodeKind::WonContinentalTrophy
            | EpisodeKind::WonDomesticCup
            | EpisodeKind::Promoted
            | EpisodeKind::SurvivedARelegationFight => self.shift_appetite(0.15),
            EpisodeKind::Relegated | EpisodeKind::FailedToSurviveIt => {
                self.shift_strain(0.20);
                self.shift_appetite(-0.15);
            }
            EpisodeKind::LostTheDressingRoom => {
                self.shift_strain(0.20);
                self.shift_appetite(-0.20);
            }
            EpisodeKind::SupportersTurnedOnMe | EpisodeKind::ChairmanUndercutMePublicly => {
                self.shift_strain(0.12);
                self.shift_appetite(-0.08);
            }
            EpisodeKind::Bereavement => {
                self.life_is_hard = true;
                self.shift_strain(0.30);
                self.shift_appetite(-0.25);
            }
            EpisodeKind::FamilyUnsettled => {
                self.life_is_hard = true;
                self.shift_strain(0.12);
            }
            EpisodeKind::ChildBorn | EpisodeKind::FamilySettled => {
                self.life_is_hard = false;
                self.shift_appetite(0.10);
            }
            _ => {}
        }
    }

    fn reflect(&mut self, view: &StaffView<'_>, organs: &mut StaffOrgans) {
        let s = view.situation;
        let today = view.today();

        // Load, plus what the job is doing to him on top of it.
        let load =
            (s.strain * 0.6 + s.job_exposure() * 0.3 + s.relegation_danger() * 0.3).clamp(0.0, 1.0);
        let gap = load - self.strain();
        // Strain accrues faster than it clears. The asymmetry is the
        // model: a bad season leaves a mark a good one does not remove.
        let rate = if gap > 0.0 {
            Self::ACCRUAL
        } else {
            Self::RECOVERY
        };
        self.shift_strain(gap * rate);

        // Appetite drains under sustained strain and is topped up by the
        // job going well.
        let drain = self.strain() * 0.035;
        let lift = s.against_expectation().max(0.0) * 0.025;
        self.shift_appetite(lift - drain);

        // The end of a career arrives by degrees rather than on a
        // birthday: age, strain, and having nothing left to prove.
        let winding_down = s.career_stage();
        if winding_down > 0.0 && (self.is_burnt_out() || winding_down > 0.5) {
            let mut evidence = GoalEvidence::of(&[GoalEvidence::LATE_CAREER]);
            if s.trophies_here > 0 || winding_down > 0.7 {
                evidence.insert(GoalEvidence::NOTHING_LEFT_TO_PROVE);
            }
            organs.shared.goals.pursue(
                GoalKind::RetireFromTheGame,
                GoalOrigin::Survival,
                evidence,
                winding_down * 0.12 + self.strain() * 0.10,
                today,
            );
        }

        // A man who is finished with it wants out of this job first.
        if self.appetite() < Self::SPENT {
            organs.shared.goals.pursue(
                GoalKind::GetOutOfHere,
                GoalOrigin::Survival,
                GoalEvidence::of(&[GoalEvidence::LATE_CAREER]),
                0.20,
                today,
            );
        }
    }

    fn appraise(&self, organs: &StaffOrgans) -> MoodContribution {
        let value = self.appetite() * 5.0 - self.strain() * 4.0
            + if self.life_is_hard { -2.0 } else { 0.0 }
            - organs.pressure_in(GoalDomain::Welfare) * 2.0;
        // He always knows how tired he is.
        MoodContribution::new(GoalDomain::Welfare, value, 0.75)
    }

    fn weigh(&self, option: MindOption, organs: &StaffOrgans) -> ReasonSet {
        let mut reasons = ReasonSet::new();

        match option {
            MindOption::Resign => {
                if self.is_burnt_out() {
                    reasons.push(GoalKind::RetireFromTheGame, self.strain());
                }
                if self.appetite() < 0.0 {
                    reasons.push(GoalKind::GetOutOfHere, -self.appetite());
                }
                let retiring = organs.shared.goals.pressure_of(GoalKind::RetireFromTheGame);
                if retiring > 0.05 {
                    reasons.push(GoalKind::RetireFromTheGame, retiring);
                }
            }

            MindOption::TakeTheJob(_) => {
                // A tired man does not take on another one.
                if self.strain() > 0.5 {
                    reasons.push(GoalKind::RetireFromTheGame, -self.strain());
                }
                if self.appetite() > 0.2 {
                    reasons.push(GoalKind::GetABiggerJob, self.appetite() * 0.5);
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
    use crate::club::mind::organs::memory::ActorRef;
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
        fn think(mind: &mut StaffMind, situation: &StaffSituation, weeks: i64, club: u32) {
            let start = Self::date(2030, 8, 1);
            for week in 0..weeks {
                let date = start + Duration::days(week * 7);
                mind.tick_with(&Self::context(date, club), situation);
            }
        }

        /// A season nobody would want.
        fn brutal() -> StaffSituation {
            let mut situation = StaffSituation::neutral();
            situation.strain = 0.9;
            situation.board_trust = 0.2;
            situation.league_size = 20;
            situation.expected_position = 8;
            situation.league_position = 19;
            situation.season_progress = 0.75;
            situation
        }
    }

    #[test]
    fn a_bad_season_leaves_a_mark_a_good_one_does_not_remove() {
        let mut mind = StaffMind::new();
        Fixture::think(&mut mind, &Fixture::brutal(), 38, 7);
        let after_the_season = mind.welfare.strain();
        assert!(after_the_season > 0.5, "{after_the_season}");

        // A calm year afterwards.
        let mut calm = StaffSituation::neutral();
        calm.strain = 0.1;
        calm.board_trust = 0.85;
        let start = Fixture::date(2031, 8, 1);
        for week in 0..38 {
            let date = start + chrono::Duration::days(week * 7);
            mind.tick_with(&Fixture::context(date, 7), &calm);
        }

        assert!(mind.welfare.strain() < after_the_season, "it does clear");
        assert!(
            mind.welfare.strain() > after_the_season * 0.35,
            "but not in one quiet season: {} from {after_the_season}",
            mind.welfare.strain()
        );
    }

    #[test]
    fn being_sacked_is_a_rest_and_a_blow_at_the_same_time() {
        let mut mind = StaffMind::new();
        Fixture::think(&mut mind, &Fixture::brutal(), 20, 7);
        let strain = mind.welfare.strain();
        let appetite = mind.welfare.appetite();

        let c = Fixture::context(Fixture::date(2031, 3, 1), 7);
        mind.remember(EpisodeKind::SackedByClub, ActorRef::club(7), &c);

        assert!(mind.welfare.strain() < strain, "the load comes off");
        assert!(
            mind.welfare.appetite() < appetite,
            "and so does the appetite"
        );
    }

    #[test]
    fn a_career_ends_by_degrees_rather_than_on_a_birthday() {
        let mut young = StaffMind::new();
        let mut situation = StaffSituation::neutral();
        situation.age = 40.0;
        Fixture::think(&mut young, &situation, 30, 7);

        let mut old = StaffMind::new();
        situation.age = 64.0;
        Fixture::think(&mut old, &situation, 30, 7);

        assert_eq!(young.pressure_of(GoalKind::RetireFromTheGame), 0.0);
        assert!(old.pressure_of(GoalKind::RetireFromTheGame) > 0.0);
    }

    #[test]
    fn burnout_is_strain_that_has_outlasted_the_appetite() {
        let mut mind = StaffMind::new();
        Fixture::think(&mut mind, &Fixture::brutal(), 60, 7);

        assert!(
            mind.welfare.is_burnt_out(),
            "strain {} appetite {}",
            mind.welfare.strain(),
            mind.welfare.appetite()
        );
        let reasons = mind.welfare.weigh(MindOption::Resign, &mind.organs);
        assert!(reasons.net() > 0.0, "and it argues for walking away");
    }

    #[test]
    fn a_bereavement_reaches_the_job() {
        let mut mind = StaffMind::new();
        let before = mind.welfare.appetite();
        let c = Fixture::context(Fixture::date(2031, 2, 2), 7);
        mind.remember(EpisodeKind::Bereavement, ActorRef::NONE, &c);

        assert!(mind.welfare.life_is_hard);
        assert!(mind.welfare.appetite() < before);
    }
}
