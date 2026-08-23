//! The professional mind — his read of the manager.
//!
//! The faculty with a theory of mind. It does not consult the coach's
//! actual opinion (`CoachPlayerBond` lives on the staff side and is the
//! coach's own record); it holds the player's *belief* about where he
//! stands, which can be wrong, can lag, and can be revised — and which
//! is what he actually acts on.
//!
//! It also owns the one thing a player never forgets: whether a man's
//! word is good. That lives in memory as a standing account and a
//! conviction; this faculty is what reads them and decides what to do.

use super::organs::MindOrgans;
use super::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin};
use super::organs::memory::{ActorKind, ActorRef, EpisodeKind, FactClaim, MindEpisode};
use super::submind::{MindView, MoodContribution, SubMind};

/// His read of the man picking the team.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProfessionalMind {
    /// Who he currently believes the manager to be. Compared against the
    /// situation each think, so a change of manager resets the read
    /// rather than carrying a grudge onto an innocent successor.
    pub manager: ActorRef,
    /// −100..=100: does he think this man rates him? His belief, not the
    /// truth.
    rated_pct: i8,
    /// −100..=100: does he understand how he is being used?
    clarity_pct: i8,
    /// Weeks he has been left out without anyone telling him why.
    /// Nothing corrodes a player faster than being frozen out in silence.
    pub unexplained_weeks: u8,
}

impl ProfessionalMind {
    /// How far one interaction moves his read. Slower than form: a
    /// relationship with the manager is not rebuilt or destroyed in an
    /// afternoon.
    pub const SHIFT: f32 = 0.14;

    /// Weeks of silence after which he stops assuming there is a reason.
    pub const SILENCE_LIMIT: u8 = 6;

    /// Standing below which he concludes the man will never rate him.
    pub const WRITTEN_OFF: f32 = -0.45;

    #[inline]
    pub fn feels_rated(&self) -> f32 {
        self.rated_pct as f32 / 100.0
    }

    #[inline]
    pub fn role_clarity(&self) -> f32 {
        self.clarity_pct as f32 / 100.0
    }

    fn shift_rated(&mut self, delta: f32) {
        let value = (self.feels_rated() + delta).clamp(-1.0, 1.0);
        self.rated_pct = (value * 100.0).round() as i8;
    }

    fn shift_clarity(&mut self, delta: f32) {
        let value = (self.role_clarity() + delta).clamp(-1.0, 1.0);
        self.clarity_pct = (value * 100.0).round() as i8;
    }

    /// A new man is in charge. The read starts again — which is exactly
    /// what a change of manager means to a player who was out of favour,
    /// and the reason it is one of the few things that can rescue a
    /// career at a club.
    pub fn on_manager_change(&mut self, manager: ActorRef) {
        self.manager = manager;
        self.rated_pct = 0;
        self.clarity_pct = 0;
        self.unexplained_weeks = 0;
    }
}

impl SubMind for ProfessionalMind {
    fn domain(&self) -> GoalDomain {
        GoalDomain::Professional
    }

    fn observe(&mut self, episode: &MindEpisode, _organs: &mut MindOrgans) {
        // The first time a manager does something to him, that is who
        // the manager is. Without this the read would be wiped by the
        // next think — which sees a manager it has no record of and
        // treats him as a new arrival — and everything learned before
        // the faculty's first reflect would be lost.
        if self.manager.is_none() && episode.who.kind == ActorKind::Staff {
            self.manager = episode.who;
        }

        match episode.kind {
            EpisodeKind::ManagerPromiseKept | EpisodeKind::ManagerPrivateBacking => {
                self.shift_rated(Self::SHIFT);
                self.shift_clarity(Self::SHIFT);
                self.unexplained_weeks = 0;
            }
            EpisodeKind::ManagerPublicPraise => self.shift_rated(Self::SHIFT * 0.6),
            EpisodeKind::ManagerPromiseBroken => {
                self.shift_rated(-Self::SHIFT * 1.5);
                self.shift_clarity(-Self::SHIFT);
            }
            EpisodeKind::ManagerPublicCriticism => self.shift_rated(-Self::SHIFT),
            EpisodeKind::ManagerFrozenOut => {
                self.shift_rated(-Self::SHIFT * 1.5);
                self.shift_clarity(-Self::SHIFT * 1.5);
            }
            EpisodeKind::ManagerSignedARival => self.shift_rated(-Self::SHIFT * 0.8),
            EpisodeKind::RoleUpgraded => {
                self.shift_rated(Self::SHIFT);
                self.shift_clarity(Self::SHIFT);
            }
            EpisodeKind::RoleDowngraded => self.shift_rated(-Self::SHIFT),
            // Being played out of position is a clarity problem, not a
            // trust one — he may well believe the manager rates him and
            // still have no idea what he is being asked to do.
            EpisodeKind::SubbedOffEarly => self.shift_clarity(-Self::SHIFT * 0.5),
            EpisodeKind::ManagerArrived => self.on_manager_change(episode.who),
            _ => {}
        }
    }

    fn reflect(&mut self, view: &MindView<'_>, organs: &mut MindOrgans) {
        let s = view.situation;
        let today = view.today();

        // A different man is picking the team now.
        if s.manager.is_some() && s.manager != self.manager {
            self.on_manager_change(s.manager);
            // A fresh start is a reason to stay and fight rather than to
            // agitate — a new manager is the commonest way an out-of-favour
            // player gets his career back.
            organs.goals.pursue(
                GoalKind::WinTheManagersTrust,
                GoalOrigin::SelfDrive,
                GoalEvidence::EMPTY,
                0.45,
                today,
            );
            return;
        }
        if s.manager.is_none() || !s.is_settled() {
            return;
        }

        // Being left out with nobody explaining why.
        if s.playing_time_gap() < -0.2 {
            self.unexplained_weeks = self.unexplained_weeks.saturating_add(1);
        } else {
            self.unexplained_weeks = 0;
        }

        // What memory says about this man, independently of how the
        // player currently feels. A conviction that his word is worthless
        // outlives any amount of recent politeness.
        let standing = organs.memory.standing_with(s.manager, today);
        let word_is_worthless = organs
            .memory
            .believes(FactClaim::HisWordIsWorthless, s.manager);
        let never_trusted = organs.memory.believes(FactClaim::NeverTrustedMe, s.manager);

        let mut evidence = GoalEvidence::EMPTY;
        if word_is_worthless > 0.2 {
            evidence.insert(GoalEvidence::PROMISE_BROKEN);
        }
        if never_trusted > 0.2 || self.feels_rated() < Self::WRITTEN_OFF {
            evidence.insert(GoalEvidence::MANAGER_DOES_NOT_RATE_HIM);
        }
        if self.unexplained_weeks >= Self::SILENCE_LIMIT {
            evidence.insert(GoalEvidence::PUBLICLY_CRITICISED);
        }

        // Written off by a man whose word he no longer believes. There is
        // nothing left to win here.
        let irreparable = word_is_worthless > 0.4
            || (self.feels_rated() < Self::WRITTEN_OFF && standing < Self::WRITTEN_OFF as f32);

        if irreparable {
            organs.goals.pursue(
                GoalKind::LeaveThisClub,
                GoalOrigin::Grievance,
                evidence,
                (-self.feels_rated()).clamp(0.2, 1.0),
                today,
            );
            // And he stops trying to win over a man he has written off.
            organs.goals.resolve(GoalKind::WinTheManagersTrust, false);
            return;
        }

        // Out of favour, but not beyond saving. This is the ordinary
        // case, and the ordinary response is to try harder.
        if self.feels_rated() < 0.0 || self.unexplained_weeks >= Self::SILENCE_LIMIT {
            organs.goals.pursue(
                GoalKind::WinTheManagersTrust,
                GoalOrigin::SelfDrive,
                evidence,
                (-self.feels_rated()).clamp(0.15, 0.8),
                today,
            );
        } else if self.feels_rated() > 0.3 {
            // He is trusted. Whatever he wanted there is answered.
            organs.goals.advance(GoalKind::WinTheManagersTrust, 0.2);
        }

        // Not knowing what he is being asked to do is its own grievance,
        // separate from minutes — a misused player can be playing every
        // week and still want out of the role.
        if self.role_clarity() < -0.3 {
            organs.goals.pursue(
                GoalKind::PlayInMyBestRole,
                GoalOrigin::Grievance,
                GoalEvidence::of(&[GoalEvidence::PLAYED_OUT_OF_POSITION]),
                (-self.role_clarity()).clamp(0.0, 1.0),
                today,
            );
        }
    }

    fn appraise(&self, _organs: &MindOrgans) -> MoodContribution {
        // Feeling rated and understanding the role are separate axes;
        // both weigh, trust the heavier of the two.
        let value = self.feels_rated() * 6.0 + self.role_clarity() * 3.0;
        // No manager, no view.
        let confidence = if self.manager.is_none() { 0.0 } else { 0.8 };
        MoodContribution::new(GoalDomain::Professional, value, confidence)
    }
}

#[cfg(test)]
mod tests {
    use super::super::MindTickContext;
    use super::super::organs::memory::{Ledger, LedgerEntry, MemoryContext};
    use super::super::situation::MindSituation;
    use super::*;
    use crate::club::person::PersonAttributes;
    use chrono::NaiveDate;

    const COACH: u32 = 412;

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

    fn reflect(mind: &mut ProfessionalMind, situation: &MindSituation, organs: &mut MindOrgans) {
        let tick = MindTickContext::new(
            NaiveDate::from_ymd_opt(2030, 6, 1).unwrap(),
            7,
            &attrs(),
            50.0,
        );
        let view = MindView {
            tick: &tick,
            situation,
        };
        mind.reflect(&view, organs);
    }

    fn episode(kind: EpisodeKind, who: ActorRef) -> MindEpisode {
        MindEpisode::new(kind, who, 7, 100, kind.spec().valence, 0.8)
    }

    fn under(manager: u32) -> MindSituation {
        MindSituation {
            manager: ActorRef::staff(manager),
            days_at_club: 500,
            starter_ratio: 0.6,
            expected_start_share: 0.6,
            ..MindSituation::neutral()
        }
    }

    #[test]
    fn a_broken_promise_costs_more_than_praise_buys() {
        let mut organs = MindOrgans::new();
        let coach = ActorRef::staff(COACH);

        let mut praised = ProfessionalMind::default();
        praised.observe(
            &episode(EpisodeKind::ManagerPublicPraise, coach),
            &mut organs,
        );

        let mut betrayed = ProfessionalMind::default();
        betrayed.observe(
            &episode(EpisodeKind::ManagerPromiseBroken, coach),
            &mut organs,
        );

        assert!(betrayed.feels_rated().abs() > praised.feels_rated().abs());
    }

    #[test]
    fn a_new_manager_wipes_the_slate() {
        let mut organs = MindOrgans::new();
        let mut mind = ProfessionalMind::default();
        mind.manager = ActorRef::staff(COACH);
        for _ in 0..5 {
            mind.observe(
                &episode(EpisodeKind::ManagerFrozenOut, ActorRef::staff(COACH)),
                &mut organs,
            );
        }
        assert!(mind.feels_rated() < -0.5);

        reflect(&mut mind, &under(999), &mut organs);
        assert_eq!(mind.feels_rated(), 0.0, "a fresh start really is fresh");
        assert!(
            organs.goals.pressure_of(GoalKind::WinTheManagersTrust) > 0.0,
            "and a reason to fight for it"
        );
    }

    #[test]
    fn a_grudge_never_follows_a_manager_onto_his_successor() {
        let mut organs = MindOrgans::new();
        let mut mind = ProfessionalMind::default();
        mind.manager = ActorRef::staff(COACH);
        for _ in 0..6 {
            mind.observe(
                &episode(EpisodeKind::ManagerPromiseBroken, ActorRef::staff(COACH)),
                &mut organs,
            );
        }
        reflect(&mut mind, &under(999), &mut organs);
        assert_eq!(organs.goals.pressure_of(GoalKind::LeaveThisClub), 0.0);
    }

    #[test]
    fn being_out_of_favour_makes_him_try_harder_first() {
        let mut organs = MindOrgans::new();
        let mut mind = ProfessionalMind::default();
        mind.manager = ActorRef::staff(COACH);
        mind.observe(
            &episode(EpisodeKind::ManagerPublicCriticism, ActorRef::staff(COACH)),
            &mut organs,
        );

        reflect(&mut mind, &under(COACH), &mut organs);
        assert!(organs.goals.pressure_of(GoalKind::WinTheManagersTrust) > 0.0);
        assert_eq!(organs.goals.pressure_of(GoalKind::LeaveThisClub), 0.0);
    }

    #[test]
    fn a_man_whose_word_is_worthless_is_not_worth_winning_over() {
        let mut organs = MindOrgans::new();
        let coach = ActorRef::staff(COACH);
        let ctx = MemoryContext::neutral(100, 7);

        // Three broken promises, banked into a conviction.
        for _ in 0..3 {
            organs
                .memory
                .record_plain(EpisodeKind::ManagerPromiseBroken, coach, &ctx);
        }
        organs
            .memory
            .maybe_consolidate(&MemoryContext::neutral(140, 7));
        assert!(organs.memory.believes(FactClaim::HisWordIsWorthless, coach) > 0.4);

        let mut mind = ProfessionalMind::default();
        mind.manager = coach;
        for _ in 0..3 {
            mind.observe(
                &episode(EpisodeKind::ManagerPromiseBroken, coach),
                &mut organs,
            );
        }
        reflect(&mut mind, &under(COACH), &mut organs);

        assert!(
            organs.goals.pressure_of(GoalKind::LeaveThisClub) > 0.0,
            "there is nothing left to win here"
        );
        assert_eq!(
            organs.goals.pressure_of(GoalKind::WinTheManagersTrust),
            0.0,
            "and he stops trying"
        );
    }

    #[test]
    fn silence_is_its_own_grievance() {
        let mut organs = MindOrgans::new();
        let mut mind = ProfessionalMind::default();
        mind.manager = ActorRef::staff(COACH);

        let benched = MindSituation {
            starter_ratio: 0.1,
            expected_start_share: 0.7,
            ..under(COACH)
        };
        for _ in 0..ProfessionalMind::SILENCE_LIMIT {
            reflect(&mut mind, &benched, &mut organs);
        }
        assert!(mind.unexplained_weeks >= ProfessionalMind::SILENCE_LIMIT);
        assert!(
            organs
                .goals
                .get(GoalKind::WinTheManagersTrust)
                .unwrap()
                .evidence
                .contains(GoalEvidence::PUBLICLY_CRITICISED)
        );
    }

    #[test]
    fn playing_again_resets_the_silence() {
        let mut organs = MindOrgans::new();
        let mut mind = ProfessionalMind::default();
        mind.manager = ActorRef::staff(COACH);
        reflect(
            &mut mind,
            &MindSituation {
                starter_ratio: 0.1,
                expected_start_share: 0.7,
                ..under(COACH)
            },
            &mut organs,
        );
        assert_eq!(mind.unexplained_weeks, 1);

        reflect(&mut mind, &under(COACH), &mut organs);
        assert_eq!(mind.unexplained_weeks, 0);
    }

    #[test]
    fn being_misused_is_a_separate_grievance_from_being_left_out() {
        let mut organs = MindOrgans::new();
        let mut mind = ProfessionalMind::default();
        mind.manager = ActorRef::staff(COACH);
        for _ in 0..8 {
            mind.observe(
                &episode(EpisodeKind::SubbedOffEarly, ActorRef::staff(COACH)),
                &mut organs,
            );
        }

        // Playing every week, and still with no idea what he is meant to do.
        reflect(&mut mind, &under(COACH), &mut organs);
        assert!(organs.goals.pressure_of(GoalKind::PlayInMyBestRole) > 0.0);
    }

    #[test]
    fn being_trusted_answers_the_want() {
        let mut organs = MindOrgans::new();
        let mut mind = ProfessionalMind::default();
        mind.manager = ActorRef::staff(COACH);
        organs.goals.pursue(
            GoalKind::WinTheManagersTrust,
            GoalOrigin::SelfDrive,
            GoalEvidence::EMPTY,
            0.6,
            100,
        );

        for _ in 0..4 {
            mind.observe(
                &episode(EpisodeKind::ManagerPrivateBacking, ActorRef::staff(COACH)),
                &mut organs,
            );
        }
        reflect(&mut mind, &under(COACH), &mut organs);
        assert!(
            organs
                .goals
                .get(GoalKind::WinTheManagersTrust)
                .unwrap()
                .progress()
                > 0.0
        );
    }

    #[test]
    fn no_manager_means_no_view() {
        let mind = ProfessionalMind::default();
        let organs = MindOrgans::new();
        assert!(mind.appraise(&organs).is_silent());
    }

    #[test]
    fn the_ledger_and_the_read_can_disagree() {
        // Theory of mind: what he believes about the manager is his own,
        // and the standing account is separate evidence the faculty
        // consults rather than a mirror of it.
        let mut organs = MindOrgans::new();
        let coach = ActorRef::staff(COACH);
        Ledger::post(
            &mut organs.memory.ledger,
            coach,
            LedgerEntry::warmth(0.9),
            100,
        );

        let mut mind = ProfessionalMind::default();
        mind.manager = coach;
        mind.observe(&episode(EpisodeKind::ManagerFrozenOut, coach), &mut organs);

        assert!(mind.feels_rated() < 0.0, "he does not feel rated");
        assert!(
            organs.memory.standing_with(coach, 100) > 0.0,
            "and still likes the man"
        );
    }
}
