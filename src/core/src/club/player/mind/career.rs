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

use super::organs::MindOrgans;
use super::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin};
use super::organs::memory::{ActorRef, EpisodeKind, FactClaim, MindEpisode};
use super::submind::{MindView, MoodContribution, SubMind};

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
}

impl CareerMind {
    /// Ambition at or above which he actively wants more, on the 0–20
    /// personality scale. Below it he is content to be where he is, and
    /// the faculty forms no upward wants at all.
    pub const AMBITIOUS: f32 = 12.0;

    /// Days at one level after which an ambitious player starts feeling
    /// the ceiling. Two full seasons.
    pub const PLATEAU_DAYS: u16 = 730;

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

        // ── Wanting more ────────────────────────────────────────
        if s.ambition < Self::AMBITIOUS {
            return;
        }
        let drive = ((s.ambition - Self::AMBITIOUS) / (20.0 - Self::AMBITIOUS)).clamp(0.0, 1.0);

        // He has been at this level long enough to feel the ceiling. The
        // runway matters: the same plateau reads very differently to a
        // 25-year-old and a 32-year-old.
        if s.days_at_club >= Self::PLATEAU_DAYS {
            let mut evidence = GoalEvidence::of(&[GoalEvidence::HIGH_AMBITION]);
            if s.club_reputation < 0.5 {
                evidence.insert(GoalEvidence::OUTGROWN_CLUB);
            }
            if self.honours > 0 {
                evidence.insert(GoalEvidence::NOTHING_LEFT_TO_PROVE);
            }
            if s.career_spent() > 0.5 {
                evidence.insert(GoalEvidence::PRIME_YEARS_PASSING);
            }

            // A settled man at a small club wants a bigger one; a settled
            // man at a big one wants a new test. The same restlessness,
            // pointed by where he already is.
            let goal = if s.club_reputation < 0.6 {
                GoalKind::StepUpToABiggerClub
            } else {
                GoalKind::FindANewChallenge
            };
            organs
                .goals
                .pursue(goal, GoalOrigin::SelfDrive, evidence, drive * 0.5, today);

            // Time running out sharpens it.
            organs.goals.set_urgency(goal, s.career_spent());
        }

        // A club he has concluded is going nowhere.
        let here = ActorRef::club(view.tick.club_id);
        if organs.memory.believes(FactClaim::RelegatedWithThem, here) > 0.4 {
            organs.goals.pursue(
                GoalKind::KeepPlayingAtThisLevel,
                GoalOrigin::Circumstance,
                GoalEvidence::of(&[GoalEvidence::RELEGATED]),
                drive * 0.6,
                today,
            );
        }
    }

    fn appraise(&self, _organs: &MindOrgans) -> MoodContribution {
        // A career going the way he wanted lifts him; one going wrong
        // weighs. Confidence rises with how much career there is to
        // judge — a teenager with two appearances has no trajectory yet.
        let evidence = (self.honours as f32 + self.step_ups as f32 + self.setbacks as f32) / 4.0;
        let confidence = evidence.clamp(0.0, 1.0);
        MoodContribution::new(GoalDomain::Career, self.trajectory() * 5.0, confidence)
    }
}

#[cfg(test)]
mod tests {
    use super::super::MindTickContext;
    use super::super::organs::memory::MindClock;
    use super::super::situation::MindSituation;
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
        assert!(
            organs
                .goals
                .get(GoalKind::StepUpToABiggerClub)
                .unwrap()
                .evidence
                .contains(GoalEvidence::OUTGROWN_CLUB)
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
}
