//! The competitive mind — his belief in himself as a player, and his
//! standing in the side.
//!
//! The faculty that thinks in matches. It owns self-belief, the run of
//! form, the drought, and the wants that are about the shirt: winning a
//! place back, getting first-team football, ending a barren spell.
//!
//! This is also the seam to the match engine. `PsychologyState`
//! (`match/engine/psychology/`) is correctly separate — it models one
//! afternoon — but it should be *seeded* by a player who has been
//! dropped four times running rather than starting neutral every week.
//! [`CompetitiveMind::self_belief`] is that seed.

use super::organs::MindOrgans;
use super::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin};
use super::organs::memory::{EpisodeKind, MindEpisode};
use super::submind::{MindView, MoodContribution, SubMind};

/// His belief in himself as a player.
#[derive(Debug, Clone, Copy, Default)]
pub struct CompetitiveMind {
    /// −100..=100. Built from what actually happens on the pitch.
    belief_pct: i8,
    /// Consecutive matches started, or (negative) consecutive matches
    /// watched. The run, not the season total — a man who has started
    /// the last six feels different from one who started six in August.
    pub run: i8,
    /// Big matches he rose to, minus the ones he shrank in.
    pub big_match_record: i8,
    /// Weeks since anything went right. Saturates.
    pub barren_weeks: u8,
}

impl CompetitiveMind {
    /// How far one event moves self-belief, as a fraction of the gap to
    /// the extreme. Asymmetric on purpose: a mistake costs more belief
    /// than a good game buys, which is both true and what makes a slump
    /// something a player has to climb out of.
    pub const LIFT: f32 = 0.12;
    pub const KNOCK: f32 = 0.20;

    /// Consecutive matches out of the side after which he stops assuming
    /// it is temporary.
    pub const DROPPED_RUN: i8 = -4;

    #[inline]
    pub fn self_belief(&self) -> f32 {
        self.belief_pct as f32 / 100.0
    }

    fn shift(&mut self, delta: f32) {
        let belief = self.self_belief();
        let gap = if delta > 0.0 {
            1.0 - belief
        } else {
            belief + 1.0
        };
        let next = (belief + delta * gap).clamp(-1.0, 1.0);
        self.belief_pct = (next * 100.0).round() as i8;
    }

    fn extend_run(&mut self, started: bool) {
        self.run = if started {
            self.run.max(0).saturating_add(1)
        } else {
            self.run.min(0).saturating_sub(1)
        };
    }

    /// Is he out of the side long enough to have concluded something
    /// about it?
    #[inline]
    pub fn is_out_of_the_side(&self) -> bool {
        self.run <= Self::DROPPED_RUN
    }
}

impl SubMind for CompetitiveMind {
    fn domain(&self) -> GoalDomain {
        GoalDomain::Competitive
    }

    fn observe(&mut self, episode: &MindEpisode, _organs: &mut MindOrgans) {
        match episode.kind {
            EpisodeKind::DecisiveGoal | EpisodeKind::ManOfTheMatch => {
                self.shift(Self::LIFT);
                self.barren_weeks = 0;
            }
            EpisodeKind::StartedBigMatch | EpisodeKind::DerbyWin => {
                self.shift(Self::LIFT);
                self.big_match_record = self.big_match_record.saturating_add(1);
                self.extend_run(true);
                self.barren_weeks = 0;
            }
            EpisodeKind::WonStartingPlace => {
                self.shift(Self::LIFT);
                self.extend_run(true);
            }
            EpisodeKind::CostlyError | EpisodeKind::MissedDecisivePenalty => {
                self.shift(-Self::KNOCK);
            }
            EpisodeKind::LeftOutOfBigMatch => {
                self.shift(-Self::KNOCK);
                self.big_match_record = self.big_match_record.saturating_sub(1);
                self.extend_run(false);
            }
            EpisodeKind::DroppedToBench | EpisodeKind::LostStartingPlace => {
                self.shift(-Self::KNOCK);
                self.extend_run(false);
            }
            EpisodeKind::SentOff | EpisodeKind::DerbyDefeat | EpisodeKind::HeavyDefeat => {
                self.shift(-Self::LIFT);
            }
            // The body speaks to this faculty too — a long lay-off is a
            // blow to a player's belief in himself, not just his fitness.
            EpisodeKind::CareerThreateningInjury => self.shift(-Self::KNOCK * 1.5),
            EpisodeKind::SeriousInjury => self.shift(-Self::KNOCK),
            EpisodeKind::ReturnedFromLongInjury => self.shift(Self::LIFT),
            _ => {}
        }
    }

    fn reflect(&mut self, view: &MindView<'_>, organs: &mut MindOrgans) {
        let s = view.situation;
        let today = view.today();

        // A goal drought, for the players it means anything to.
        if s.apps_since_goal >= 8 {
            self.barren_weeks = self.barren_weeks.saturating_add(1);
            organs.goals.pursue(
                GoalKind::EndTheDrought,
                GoalOrigin::SelfDrive,
                GoalEvidence::EMPTY,
                (s.apps_since_goal as f32 / 20.0).clamp(0.0, 1.0),
                today,
            );
        }

        // Playing less than the role he was given implies. The gap is
        // measured against what he was told, so identical minutes are a
        // grievance for a key player and nothing at all for a backup.
        let gap = s.playing_time_gap();
        if gap >= 0.0 {
            // He is playing. Whatever he wanted about his place is
            // answered, gradually.
            organs.goals.advance(GoalKind::WinBackMyPlace, 0.15);
            organs.goals.advance(GoalKind::PlayFirstTeamFootball, 0.10);
            return;
        }

        // A settling-in period is not a grievance yet.
        if !s.is_settled() {
            return;
        }

        let shortfall = (-gap).clamp(0.0, 1.0);
        let mut evidence = GoalEvidence::of(&[GoalEvidence::LOST_HIS_PLACE]);
        if self.is_out_of_the_side() {
            evidence.insert(GoalEvidence::NO_FIRST_TEAM_FOOTBALL);
        }
        if s.career_spent() > 0.5 {
            evidence.insert(GoalEvidence::PRIME_YEARS_PASSING);
        }

        // First he means to win it back. That is what a player does.
        organs.goals.pursue(
            GoalKind::WinBackMyPlace,
            GoalOrigin::SelfDrive,
            evidence,
            shortfall,
            today,
        );

        // Only once he has stopped believing he will — a long run out of
        // the side, and a career burning down — does it become wanting to
        // be somewhere he would play. This is the one want in the whole
        // catalog that points at a *smaller* club, and it must be earned
        // rather than assumed.
        if self.is_out_of_the_side() && self.self_belief() < 0.0 {
            let resignation = shortfall * s.career_spent().max(0.3);
            organs.goals.pursue(
                GoalKind::PlayFirstTeamFootball,
                GoalOrigin::Survival,
                evidence,
                resignation,
                today,
            );
            organs
                .goals
                .set_urgency(GoalKind::PlayFirstTeamFootball, s.career_spent());
        }
    }

    fn appraise(&self, _organs: &MindOrgans) -> MoodContribution {
        // Self-belief is the axis; the run gives the confidence, because
        // a player who has not been near the side has less to go on than
        // one in the middle of a run either way.
        let evidence = (self.run.unsigned_abs() as f32 / 6.0).clamp(0.0, 1.0);
        MoodContribution::new(
            GoalDomain::Competitive,
            self.self_belief() * 6.0,
            evidence.max(0.3),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::MindTickContext;
    use super::super::organs::memory::ActorRef;
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

    fn reflect(mind: &mut CompetitiveMind, situation: &MindSituation, organs: &mut MindOrgans) {
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

    fn episode(kind: EpisodeKind) -> MindEpisode {
        MindEpisode::new(kind, ActorRef::NONE, 7, 100, kind.spec().valence, 0.8)
    }

    fn settled_but_benched() -> MindSituation {
        MindSituation {
            days_at_club: 400,
            starter_ratio: 0.05,
            expected_start_share: 0.70,
            age: 29,
            ..MindSituation::neutral()
        }
    }

    #[test]
    fn a_mistake_costs_more_belief_than_a_good_game_buys() {
        let mut organs = MindOrgans::new();

        let mut lifted = CompetitiveMind::default();
        lifted.observe(&episode(EpisodeKind::ManOfTheMatch), &mut organs);

        let mut knocked = CompetitiveMind::default();
        knocked.observe(&episode(EpisodeKind::CostlyError), &mut organs);

        assert!(knocked.self_belief().abs() > lifted.self_belief().abs());
    }

    #[test]
    fn belief_saturates_rather_than_running_away() {
        let mut organs = MindOrgans::new();
        let mut mind = CompetitiveMind::default();
        for _ in 0..80 {
            mind.observe(&episode(EpisodeKind::ManOfTheMatch), &mut organs);
        }
        assert!(mind.self_belief() <= 1.0);
        assert!(mind.self_belief() > 0.9);
    }

    #[test]
    fn the_run_tracks_the_last_stretch_not_the_season() {
        let mut organs = MindOrgans::new();
        let mut mind = CompetitiveMind::default();

        for _ in 0..5 {
            mind.observe(&episode(EpisodeKind::StartedBigMatch), &mut organs);
        }
        assert_eq!(mind.run, 5);

        mind.observe(&episode(EpisodeKind::DroppedToBench), &mut organs);
        assert_eq!(mind.run, -1, "the run breaks, it does not decrement");
    }

    #[test]
    fn a_benched_player_first_means_to_win_his_place_back() {
        let mut mind = CompetitiveMind::default();
        let mut organs = MindOrgans::new();
        reflect(&mut mind, &settled_but_benched(), &mut organs);

        assert!(organs.goals.pressure_of(GoalKind::WinBackMyPlace) > 0.0);
        assert_eq!(
            organs.goals.pressure_of(GoalKind::PlayFirstTeamFootball),
            0.0,
            "he has not given up yet"
        );
    }

    #[test]
    fn only_a_man_who_has_stopped_believing_looks_for_a_smaller_club() {
        let mut mind = CompetitiveMind::default();
        let mut organs = MindOrgans::new();

        // A long run out of the side, and the belief to match.
        for _ in 0..6 {
            mind.observe(&episode(EpisodeKind::DroppedToBench), &mut organs);
        }
        assert!(mind.is_out_of_the_side());
        assert!(mind.self_belief() < 0.0);

        reflect(&mut mind, &settled_but_benched(), &mut organs);
        assert!(
            organs.goals.pressure_of(GoalKind::PlayFirstTeamFootball) > 0.0,
            "the only want in the catalog that points downward, and it is earned"
        );
    }

    #[test]
    fn identical_minutes_are_a_grievance_for_one_man_and_not_another() {
        let build = |expected: f32| {
            let mut mind = CompetitiveMind::default();
            let mut organs = MindOrgans::new();
            reflect(
                &mut mind,
                &MindSituation {
                    days_at_club: 400,
                    starter_ratio: 0.20,
                    expected_start_share: expected,
                    ..MindSituation::neutral()
                },
                &mut organs,
            );
            organs.goals.pressure_of(GoalKind::WinBackMyPlace)
        };

        assert!(build(0.70) > 0.0, "a key player left out has a grievance");
        assert_eq!(
            build(0.15),
            0.0,
            "a backup playing a fifth of the games does not"
        );
    }

    #[test]
    fn a_new_signing_is_given_time_before_it_is_a_grievance() {
        let mut mind = CompetitiveMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                days_at_club: 20,
                ..settled_but_benched()
            },
            &mut organs,
        );
        assert!(organs.goals.is_empty());
    }

    #[test]
    fn playing_again_answers_the_want() {
        let mut mind = CompetitiveMind::default();
        let mut organs = MindOrgans::new();
        reflect(&mut mind, &settled_but_benched(), &mut organs);
        let goal = organs.goals.get(GoalKind::WinBackMyPlace).unwrap();
        assert_eq!(goal.progress(), 0.0);

        // He gets back in the side.
        reflect(
            &mut mind,
            &MindSituation {
                starter_ratio: 0.9,
                ..settled_but_benched()
            },
            &mut organs,
        );
        assert!(
            organs
                .goals
                .get(GoalKind::WinBackMyPlace)
                .unwrap()
                .progress()
                > 0.0
        );
    }

    #[test]
    fn a_drought_becomes_a_want_of_its_own() {
        let mut mind = CompetitiveMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                apps_since_goal: 14,
                starter_ratio: 0.9,
                expected_start_share: 0.7,
                ..MindSituation::neutral()
            },
            &mut organs,
        );
        assert!(organs.goals.pressure_of(GoalKind::EndTheDrought) > 0.0);
    }

    #[test]
    fn an_injury_dents_belief_and_coming_back_restores_some_of_it() {
        let mut organs = MindOrgans::new();
        let mut mind = CompetitiveMind::default();
        mind.observe(&episode(EpisodeKind::CareerThreateningInjury), &mut organs);
        let hurt = mind.self_belief();
        assert!(hurt < 0.0);

        mind.observe(&episode(EpisodeKind::ReturnedFromLongInjury), &mut organs);
        assert!(mind.self_belief() > hurt);
    }

    #[test]
    fn appraisal_follows_belief() {
        let mut organs = MindOrgans::new();
        let mut confident = CompetitiveMind::default();
        for _ in 0..6 {
            confident.observe(&episode(EpisodeKind::StartedBigMatch), &mut organs);
        }
        assert!(confident.appraise(&organs).weighted() > 0.0);

        let mut shot = CompetitiveMind::default();
        for _ in 0..6 {
            shot.observe(&episode(EpisodeKind::DroppedToBench), &mut organs);
        }
        assert!(shot.appraise(&organs).weighted() < 0.0);
    }
}
