//! The social mind — whether he belongs here.
//!
//! The faculty that decides if a place ever became home. It owns
//! settling, isolation, the dressing room, language and the pull of
//! where he came from — and the wants that follow from those: going
//! home, learning the language, finding someone to lean on, or simply
//! staying somewhere he has been happy.
//!
//! It is also the counterweight to the career mind. Everything the
//! career faculty forms points a player *out*; this one is where
//! `StayAtThisClub` comes from, and without it a simulated league is
//! nothing but churn.

use super::organs::MindOrgans;
use super::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin};
use super::organs::memory::{ActorRef, EpisodeKind, FactClaim, MindEpisode};
use super::submind::{MindView, MoodContribution, SubMind};

/// His sense of belonging.
#[derive(Debug, Clone, Copy, Default)]
pub struct SocialMind {
    /// −100..=100. How much this feels like his place.
    belonging_pct: i8,
    /// Consecutive thinks spent feeling alone. Saturates.
    pub isolation_weeks: u8,
    /// Somebody in the dressing room he leans on.
    pub mentor: ActorRef,
    /// People he has fallen out with, minus the ones he is close to.
    pub friction: i8,
}

impl SocialMind {
    pub const SHIFT: f32 = 0.10;

    /// Weeks of isolation after which it stops being homesickness and
    /// becomes a reason to move.
    pub const ISOLATION_LIMIT: u8 = 10;

    /// Days at one club after which he is not a signing any more, he is
    /// one of them. Four years.
    pub const BELONGS_DAYS: u16 = 1460;

    #[inline]
    pub fn belonging(&self) -> f32 {
        self.belonging_pct as f32 / 100.0
    }

    fn shift(&mut self, delta: f32) {
        let value = (self.belonging() + delta).clamp(-1.0, 1.0);
        self.belonging_pct = (value * 100.0).round() as i8;
    }

    #[inline]
    pub fn has_mentor(&self) -> bool {
        self.mentor.is_some()
    }

    /// A move wipes the social slate. Unlike memory — which keeps
    /// everything — belonging is *about a place*, and it does not
    /// travel.
    pub fn on_club_change(&mut self) {
        self.belonging_pct = 0;
        self.isolation_weeks = 0;
        self.mentor = ActorRef::NONE;
        self.friction = 0;
    }
}

impl SubMind for SocialMind {
    fn domain(&self) -> GoalDomain {
        GoalDomain::Social
    }

    fn observe(&mut self, episode: &MindEpisode, _organs: &mut MindOrgans) {
        match episode.kind {
            EpisodeKind::WelcomedBySquad | EpisodeKind::SquadBackedHim => {
                self.shift(Self::SHIFT * 1.5);
                self.isolation_weeks = 0;
            }
            EpisodeKind::TeammateBefriended => {
                self.shift(Self::SHIFT);
                self.friction = self.friction.saturating_sub(1);
                self.isolation_weeks = 0;
            }
            EpisodeKind::MentorSupport => {
                self.shift(Self::SHIFT);
                self.mentor = episode.who;
                self.isolation_weeks = 0;
            }
            EpisodeKind::FansAdoration => self.shift(Self::SHIFT),
            EpisodeKind::FeltIsolated => {
                self.shift(-Self::SHIFT);
                self.isolation_weeks = self.isolation_weeks.saturating_add(1);
            }
            EpisodeKind::TeammateConflict => {
                self.shift(-Self::SHIFT);
                self.friction = self.friction.saturating_add(1);
            }
            EpisodeKind::SquadTurnedOnHim => self.shift(-Self::SHIFT * 2.0),
            EpisodeKind::FansHostility => self.shift(-Self::SHIFT * 1.5),
            EpisodeKind::MediaAttack => self.shift(-Self::SHIFT * 0.5),
            // Life outside the game reaches this faculty and no other.
            EpisodeKind::FamilySettled | EpisodeKind::ChildBorn => self.shift(Self::SHIFT),
            EpisodeKind::FamilyUnsettled => self.shift(-Self::SHIFT * 1.5),
            EpisodeKind::Bereavement => self.shift(-Self::SHIFT),
            EpisodeKind::ClubServantMilestone => self.shift(Self::SHIFT * 2.0),
            _ => {}
        }
    }

    fn reflect(&mut self, view: &MindView<'_>, organs: &mut MindOrgans) {
        let s = view.situation;
        let today = view.today();

        // ── The practical business of settling in ───────────────
        if s.is_abroad && !s.speaks_local_language {
            organs.goals.pursue(
                GoalKind::LearnTheLanguage,
                GoalOrigin::Survival,
                GoalEvidence::of(&[GoalEvidence::LANGUAGE_BARRIER]),
                0.35,
                today,
            );
        } else {
            organs.goals.advance(GoalKind::LearnTheLanguage, 0.25);
        }

        if !self.has_mentor() && !s.is_settled() && s.adaptability < 10.0 {
            organs.goals.pursue(
                GoalKind::FindAMentor,
                GoalOrigin::Survival,
                GoalEvidence::of(&[GoalEvidence::ISOLATED_IN_THE_SQUAD]),
                0.30,
                today,
            );
        } else if self.has_mentor() {
            organs.goals.advance(GoalKind::FindAMentor, 1.0);
        }

        if !s.is_settled() {
            return;
        }

        // ── Adrift ──────────────────────────────────────────────
        if s.is_culturally_isolated() && self.belonging() < 0.0 {
            self.isolation_weeks = self.isolation_weeks.saturating_add(1);
        }

        let mut evidence = GoalEvidence::EMPTY;
        if s.is_culturally_isolated() {
            evidence.insert(GoalEvidence::LANGUAGE_BARRIER);
            evidence.insert(GoalEvidence::ISOLATED_IN_THE_SQUAD);
        }
        if organs
            .memory
            .believes(FactClaim::IStruggleAbroad, ActorRef::NONE)
            > 0.3
        {
            evidence.insert(GoalEvidence::HOMESICK);
        }

        if self.isolation_weeks >= Self::ISOLATION_LIMIT {
            evidence.insert(GoalEvidence::HOMESICK);
            organs.goals.pursue(
                GoalKind::GoHome,
                GoalOrigin::Attachment,
                evidence,
                (-self.belonging()).clamp(0.2, 1.0),
                today,
            );
        }

        // The fans have turned. A player will drop a level to get out of
        // a cauldron, and a thin-skinned one much sooner than a hard one.
        let here = ActorRef::club(view.tick.club_id);
        let fans_hostile = organs
            .memory
            .believes(FactClaim::FansTurnedOnMe, ActorRef::fans(view.tick.club_id));
        if fans_hostile > 0.3 {
            let thin_skinned = ((14.0 - s.pressure) / 14.0).clamp(0.0, 1.0);
            organs.goals.pursue(
                GoalKind::EscapeThePressure,
                GoalOrigin::Grievance,
                GoalEvidence::of(&[GoalEvidence::FANS_HOSTILE, GoalEvidence::MEDIA_PRESSURE]),
                fans_hostile * thin_skinned,
                today,
            );
        }

        // ── Home ────────────────────────────────────────────────
        //
        // The counterweight. Long service somewhere he is happy, and a
        // memory that agrees, and he actively wants to stay — which
        // pushes back on everything the career faculty is forming.
        let long_service = s.days_at_club >= Self::BELONGS_DAYS;
        let fondness = organs
            .memory
            .club_sentiment(view.tick.club_id, &view.tick.recall());
        let spiritual_home = organs.memory.believes(FactClaim::SpiritualHome, here);

        if (long_service && self.belonging() > 0.2 && fondness > 0.0) || spiritual_home > 0.4 {
            let attachment =
                (self.belonging() * 0.5 + fondness * 0.3 + spiritual_home * 0.4).clamp(0.2, 1.0);
            organs.goals.pursue(
                GoalKind::StayAtThisClub,
                GoalOrigin::Attachment,
                GoalEvidence::of(&[GoalEvidence::LONG_SERVICE, GoalEvidence::HERITAGE_PULL]),
                attachment,
                today,
            );
        }
    }

    fn appraise(&self, _organs: &MindOrgans) -> MoodContribution {
        let value = self.belonging() * 6.0 - self.friction.max(0) as f32 * 0.8;
        // He needs to have been somewhere a while to have a view on
        // whether he belongs.
        let confidence = if self.belonging_pct == 0 && self.isolation_weeks == 0 {
            0.2
        } else {
            0.8
        };
        MoodContribution::new(GoalDomain::Social, value, confidence)
    }
}

#[cfg(test)]
mod tests {
    use super::super::MindTickContext;
    use super::super::organs::memory::MemoryContext;
    use super::super::situation::MindSituation;
    use super::*;
    use crate::club::person::PersonAttributes;
    use chrono::NaiveDate;

    const CLUB: u32 = 7;

    fn attrs() -> PersonAttributes {
        PersonAttributes {
            adaptability: 12.0,
            ambition: 12.0,
            controversy: 5.0,
            loyalty: 14.0,
            pressure: 12.0,
            professionalism: 12.0,
            sportsmanship: 12.0,
            temperament: 10.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    fn reflect(mind: &mut SocialMind, situation: &MindSituation, organs: &mut MindOrgans) {
        let tick = MindTickContext::new(
            NaiveDate::from_ymd_opt(2030, 6, 1).unwrap(),
            CLUB,
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
        MindEpisode::new(kind, who, CLUB, 100, kind.spec().valence, 0.8)
    }

    fn abroad_and_alone() -> MindSituation {
        MindSituation {
            days_at_club: 400,
            is_abroad: true,
            speaks_local_language: false,
            familiar_teammates: 0,
            adaptability: 6.0,
            ..MindSituation::neutral()
        }
    }

    #[test]
    fn a_man_abroad_without_the_language_sets_about_learning_it() {
        let mut mind = SocialMind::default();
        let mut organs = MindOrgans::new();
        reflect(&mut mind, &abroad_and_alone(), &mut organs);
        assert!(organs.goals.pressure_of(GoalKind::LearnTheLanguage) > 0.0);
    }

    #[test]
    fn one_compatriot_changes_everything() {
        let build = |familiar: u8| {
            let mut mind = SocialMind::default();
            let mut organs = MindOrgans::new();
            for _ in 0..SocialMind::ISOLATION_LIMIT + 4 {
                mind.observe(
                    &episode(EpisodeKind::FeltIsolated, ActorRef::NONE),
                    &mut organs,
                );
                reflect(
                    &mut mind,
                    &MindSituation {
                        familiar_teammates: familiar,
                        ..abroad_and_alone()
                    },
                    &mut organs,
                );
            }
            organs.goals.pressure_of(GoalKind::GoHome)
        };

        assert!(build(0) > 0.0, "adrift, he wants to go home");
        assert!(
            build(3) < build(0),
            "with people around him who share his background, much less so"
        );
    }

    #[test]
    fn a_settling_in_period_is_not_isolation() {
        let mut mind = SocialMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                days_at_club: 20,
                ..abroad_and_alone()
            },
            &mut organs,
        );
        assert_eq!(organs.goals.pressure_of(GoalKind::GoHome), 0.0);
    }

    #[test]
    fn a_mentor_answers_the_want_and_lifts_belonging() {
        let mut mind = SocialMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                days_at_club: 30,
                adaptability: 5.0,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert!(organs.goals.pressure_of(GoalKind::FindAMentor) > 0.0);

        mind.observe(
            &episode(EpisodeKind::MentorSupport, ActorRef::player(99)),
            &mut organs,
        );
        assert!(mind.has_mentor());
        assert!(mind.belonging() > 0.0);

        reflect(
            &mut mind,
            &MindSituation {
                days_at_club: 40,
                adaptability: 5.0,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert!(
            organs.goals.get(GoalKind::FindAMentor).unwrap().progress() >= 1.0,
            "the want is met; the next review is what closes it"
        );
    }

    #[test]
    fn long_service_somewhere_he_is_happy_makes_him_want_to_stay() {
        let mut mind = SocialMind::default();
        let mut organs = MindOrgans::new();
        for _ in 0..4 {
            mind.observe(
                &episode(EpisodeKind::FansAdoration, ActorRef::fans(CLUB)),
                &mut organs,
            );
        }
        // And a memory that agrees.
        let ctx = MemoryContext::neutral(100, CLUB);
        organs
            .memory
            .record_plain(EpisodeKind::SeniorDebut, ActorRef::NONE, &ctx);
        organs
            .memory
            .record_plain(EpisodeKind::WonLeagueTitle, ActorRef::NONE, &ctx);

        reflect(
            &mut mind,
            &MindSituation {
                days_at_club: 2000,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert!(organs.goals.pressure_of(GoalKind::StayAtThisClub) > 0.0);
    }

    #[test]
    fn the_want_to_stay_pushes_back_on_the_want_to_go() {
        // The counterweight, asserted: without it a simulated league is
        // nothing but churn.
        let mut organs = MindOrgans::new();
        organs.goals.pursue(
            GoalKind::LeaveThisClub,
            GoalOrigin::Grievance,
            GoalEvidence::EMPTY,
            1.0,
            100,
        );
        let before = organs.goals.pressure_of(GoalKind::LeaveThisClub);

        organs.goals.pursue(
            GoalKind::StayAtThisClub,
            GoalOrigin::Attachment,
            GoalEvidence::EMPTY,
            1.0,
            100,
        );
        organs.goals.review(107);

        assert!(organs.goals.pressure_of(GoalKind::LeaveThisClub) < before);
    }

    #[test]
    fn a_thin_skinned_player_flees_a_hostile_crowd_sooner() {
        let build = |pressure: f32| {
            let mut mind = SocialMind::default();
            let mut organs = MindOrgans::new();
            let ctx = MemoryContext::neutral(100, CLUB);
            for _ in 0..6 {
                organs
                    .memory
                    .record_plain(EpisodeKind::FansHostility, ActorRef::fans(CLUB), &ctx);
            }
            organs
                .memory
                .maybe_consolidate(&MemoryContext::neutral(140, CLUB));

            reflect(
                &mut mind,
                &MindSituation {
                    days_at_club: 500,
                    pressure,
                    ..MindSituation::neutral()
                },
                &mut organs,
            );
            organs.goals.pressure_of(GoalKind::EscapeThePressure)
        };

        assert!(build(4.0) > build(18.0));
        assert!(build(4.0) > 0.0);
    }

    #[test]
    fn belonging_does_not_travel() {
        let mut organs = MindOrgans::new();
        let mut mind = SocialMind::default();
        for _ in 0..5 {
            mind.observe(
                &episode(EpisodeKind::SquadBackedHim, ActorRef::NONE),
                &mut organs,
            );
        }
        assert!(mind.belonging() > 0.0);

        mind.on_club_change();
        assert_eq!(
            mind.belonging(),
            0.0,
            "a place he belonged to is not the place he has moved to"
        );
        assert!(!mind.has_mentor());
    }

    #[test]
    fn appraisal_is_quiet_before_he_has_a_view() {
        let mind = SocialMind::default();
        let organs = MindOrgans::new();
        assert!(mind.appraise(&organs).confidence < 0.5);
    }

    #[test]
    fn falling_out_with_the_dressing_room_weighs() {
        let mut organs = MindOrgans::new();
        let mut mind = SocialMind::default();
        for id in 0..4 {
            mind.observe(
                &episode(EpisodeKind::TeammateConflict, ActorRef::player(id)),
                &mut organs,
            );
        }
        assert!(mind.appraise(&organs).weighted() < 0.0);
    }
}
