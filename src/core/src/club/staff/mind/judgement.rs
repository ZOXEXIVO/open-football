//! The judgement mind — his read of players.
//!
//! The faculty that owns the [`judgements`] organ, and the one place in
//! either mind where being **wrong** is modelled explicitly.
//!
//! `CoachDecisionEngine` is not replaced by any of this. It already does
//! the right thing — scored, *explained* per-player assessments — and it
//! stays exactly where it is. What changes is what feeds it: a
//! persistent judgement that travels with the manager instead of a store
//! that starts empty at every club.
//!
//! Two properties this adds that neither `CoachMemory` nor
//! `CoachDecisionState` has:
//!
//! * **Judgements survive the job.** A manager who rated a player at one
//!   club still rates him at the next — which is how managers sign the
//!   same players repeatedly, and something the sim currently cannot
//!   express at all.
//! * **Judgements can be scored.** [`Self::settle`] closes the loop, and
//!   a coach who keeps being wrong grows more patient rather than
//!   staying at whatever `CoachProfile` seeded.
//!
//! [`judgements`]: super::organs::judgements

use super::organs::{JudgementOutcome, Judgements, StaffOrgans};
use super::submind::{MindOption, ReasonSet, StaffSubMind, StaffView};
use crate::club::mind::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin};
use crate::club::mind::organs::memory::{
    ActorKind, ActorRef, EpisodeKind, EpochDay, FactClaim, MindEpisode, Semantic,
};
use crate::club::mind::verdict::MoodContribution;

/// His read of the people he picks.
#[derive(Debug, Clone, Copy)]
pub struct JudgementMind {
    /// −100..=100: how long he gives a player before writing him off.
    /// Starts neutral and moves with the verdicts — a coach who has been
    /// wrong about people grows patient, and one who keeps being right
    /// grows decisive.
    patience_pct: i8,
    /// 0..=100: how good he believes his own eye to be. Rises with
    /// vindication, falls with being wrong. Not the same as being
    /// accurate — this is the confidence he *brings*, and a coach whose
    /// self-belief outruns his record is a real coach.
    eye_pct: u8,
    /// Verdicts his players' careers have handed him.
    pub vindicated: u16,
    pub wrong: u16,
    /// Players he wanted and did not get.
    pub targets_missed: u8,
}

impl Default for JudgementMind {
    fn default() -> Self {
        JudgementMind {
            patience_pct: 0,
            eye_pct: Self::EYE_BASE,
            vindicated: 0,
            wrong: 0,
            targets_missed: 0,
        }
    }
}

impl JudgementMind {
    /// How far one settled verdict moves his patience.
    pub const LESSON: f32 = 0.10;

    /// Starting self-belief in his own eye.
    pub const EYE_BASE: u8 = 50;

    #[inline]
    pub fn patience(&self) -> f32 {
        self.patience_pct as f32 / 100.0
    }

    /// How good he thinks his eye is, 0..1. Starts at
    /// [`Self::EYE_BASE`] — a coach with no record still brings a view
    /// of himself to the job.
    #[inline]
    pub fn eye(&self) -> f32 {
        self.eye_pct as f32 / 100.0
    }

    /// His actual record, 0..1 — the thing his self-belief is supposed
    /// to track and is allowed not to. `None` until a career has
    /// settled enough questions to have one.
    pub fn record(&self) -> Option<f32> {
        let total = self.vindicated + self.wrong;
        if total < 3 {
            return None;
        }
        Some(self.vindicated as f32 / total as f32)
    }

    /// How far his self-belief has run ahead of his record. Positive
    /// means he trusts an eye that has not earned it.
    pub fn overconfidence(&self) -> f32 {
        match self.record() {
            Some(record) => self.eye() - record,
            None => 0.0,
        }
    }

    fn shift_patience(&mut self, delta: f32) {
        let value = (self.patience() + delta).clamp(-1.0, 1.0);
        self.patience_pct = (value * 100.0).round() as i8;
    }

    fn shift_eye(&mut self, delta: f32) {
        let value = (self.eye() + delta).clamp(0.0, 1.0);
        self.eye_pct = (value * 100.0).round() as u8;
    }

    /// A player's career has answered a question this coach had a view
    /// on. Returns the verdict when there was a firm enough view to be
    /// right or wrong about.
    ///
    /// This is the monthly audit §5 of `docs/staff_mind.md` asks for,
    /// and it is what makes a coach's perception improve over a career.
    pub fn settle(
        &mut self,
        organs: &mut StaffOrgans,
        player: ActorRef,
        true_level: f32,
        today: EpochDay,
    ) -> Option<JudgementOutcome> {
        let outcome = Judgements::settle(&mut organs.judgements, player, true_level)?;

        match outcome {
            JudgementOutcome::Vindicated => {
                self.vindicated = self.vindicated.saturating_add(1);
                self.shift_eye(Self::LESSON * 0.5);
                // Being right makes a man quicker to decide next time.
                self.shift_patience(-Self::LESSON * 0.4);
            }
            JudgementOutcome::Wrong => {
                self.wrong = self.wrong.saturating_add(1);
                self.shift_eye(-Self::LESSON * 0.8);
                self.shift_patience(Self::LESSON);
                // The only claim in either catalog whose subject is the
                // holder's own past judgement.
                Semantic::assert(
                    &mut organs.shared.memory.semantic,
                    FactClaim::IWasWrongAboutHim,
                    player,
                    today,
                    0.5,
                );
            }
            JudgementOutcome::Open => {}
        }

        Some(outcome)
    }
}

impl StaffSubMind for JudgementMind {
    fn domain(&self) -> GoalDomain {
        GoalDomain::Squad
    }

    fn observe(&mut self, episode: &MindEpisode, organs: &mut StaffOrgans) {
        let about_a_player = episode.who.kind == ActorKind::Player;

        match episode.kind {
            EpisodeKind::PlayerRepaidMyFaith if about_a_player => {
                if let Some(view) = Judgements::of_mut(&mut organs.judgements, episode.who) {
                    view.character_signal(0.7, episode.when);
                }
            }
            EpisodeKind::PlayerRefusedToPlayForMe if about_a_player => {
                if let Some(view) = Judgements::of_mut(&mut organs.judgements, episode.who) {
                    view.character_signal(-1.0, episode.when);
                }
            }
            EpisodeKind::SignedAPlayerIWanted => {
                self.targets_missed = 0;
                organs
                    .shared
                    .goals
                    .advance(GoalKind::SignThePlayerIWant, 1.0);
                organs.shared.goals.advance(GoalKind::GetMyOwnSquad, 0.12);
            }
            EpisodeKind::SignedAPlayerIDidNotWant => {
                self.targets_missed = self.targets_missed.saturating_add(1);
            }
            EpisodeKind::BoardSoldMyBestPlayer => {
                organs
                    .shared
                    .goals
                    .resolve(GoalKind::KeepMyBestPlayer, false);
            }
            _ => {}
        }
    }

    fn reflect(&mut self, view: &StaffView<'_>, organs: &mut StaffOrgans) {
        let s = view.situation;
        let today = view.today();

        // A squad that is not his is the want that keeps a rebuilding
        // manager at a club he could otherwise leave.
        if s.squad_is_his < 0.5 && s.months_in_the_job < 48 {
            let mut evidence = GoalEvidence::EMPTY;
            if s.squad_is_his < 0.25 {
                evidence.insert(GoalEvidence::SQUAD_BELOW_HIS_LEVEL);
            }
            if self.targets_missed > 0 {
                evidence.insert(GoalEvidence::RIVAL_SIGNED);
            }
            organs.shared.goals.pursue(
                GoalKind::GetMyOwnSquad,
                GoalOrigin::SelfDrive,
                evidence,
                (0.5 - s.squad_is_his) * 0.5,
                today,
            );
        } else if s.squad_is_his > 0.7 {
            organs.shared.goals.advance(GoalKind::GetMyOwnSquad, 0.35);
        }

        // Keeping a player he rates. The want only exists while somebody
        // is actually asking for him — a manager does not spend the
        // season wanting to keep players nobody wants.
        if s.best_player_wanted {
            let core: [Option<ActorRef>; 1] = Judgements::core(&organs.judgements, today);
            let has_someone_to_lose = core[0].is_some();
            if has_someone_to_lose {
                organs.shared.goals.pursue(
                    GoalKind::KeepMyBestPlayer,
                    GoalOrigin::Circumstance,
                    GoalEvidence::of(&[GoalEvidence::CLUBS_ARE_INTERESTED]),
                    0.45,
                    today,
                );
            }
        }

        // Wanting a specific signing, once the window has given him
        // something to want and the board has not delivered it.
        if self.targets_missed > 0 {
            organs.shared.goals.pursue(
                GoalKind::SignThePlayerIWant,
                GoalOrigin::SelfDrive,
                GoalEvidence::of(&[GoalEvidence::SQUAD_BELOW_HIS_LEVEL]),
                0.20,
                today,
            );
        }
    }

    fn appraise(&self, organs: &StaffOrgans) -> MoodContribution {
        let held = organs.judgements.len();
        if held == 0 {
            return MoodContribution::silent(GoalDomain::Squad);
        }

        // How he feels about the squad is mostly how much of it he
        // rates, discounted by the wants he cannot satisfy about it.
        let rated = organs
            .judgements
            .iter()
            .map(|view| view.level())
            .sum::<f32>()
            / held as f32;
        let value = (rated - 0.5) * 10.0 - organs.pressure_in(GoalDomain::Squad) * 3.0;
        let confidence = (held as f32 / 18.0).clamp(0.0, 1.0);
        MoodContribution::new(GoalDomain::Squad, value, confidence)
    }

    fn weigh(&self, option: MindOption, organs: &StaffOrgans) -> ReasonSet {
        let mut reasons = ReasonSet::new();

        match option {
            MindOption::SignThisPlayer(player_id) | MindOption::DropThisPlayer(player_id) => {
                let player = ActorRef::player(player_id);
                let Some(view) = Judgements::of(&organs.judgements, player) else {
                    return reasons;
                };
                // Confidence is the whole point of the read: a coach
                // argues from a firm view and shrugs at a vague one.
                let sure = view.confidence(view.last_seen);
                let opinion = (view.level() - 0.5) * 2.0 * sure;

                // Being dropped is the negation of being signed, so the
                // same opinion points the other way.
                let sign = matches!(option, MindOption::SignThisPlayer(_));
                let weight = if sign { opinion } else { -opinion };
                reasons.push(GoalKind::GetMyOwnSquad, weight);

                // A coach who has learned he was wrong about this
                // particular player argues against himself.
                let humbled = organs
                    .memory()
                    .believes(FactClaim::IWasWrongAboutHim, player);
                if humbled > 0.1 {
                    reasons.push(GoalKind::SignThePlayerIWant, -weight * humbled);
                }
            }

            MindOption::SellThisPlayer(player_id) => {
                let player = ActorRef::player(player_id);
                let Some(view) = Judgements::of(&organs.judgements, player) else {
                    return reasons;
                };
                let sure = view.confidence(view.last_seen);
                if view.is_worth_building_around(view.last_seen) {
                    reasons.push(GoalKind::KeepMyBestPlayer, -view.level() * sure);
                }
                let let_me_down = organs.memory().believes(FactClaim::HeLetMeDown, player);
                if let_me_down > 0.1 {
                    reasons.push(GoalKind::RestoreOrderInTheRoom, let_me_down);
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
    use crate::club::staff::mind::{StaffMind, StaffTickContext};
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
        /// A coach with a firm, low view of a player.
        fn wrote_him_off(mind: &mut StaffMind, player_id: u32) {
            let start = Self::date(2030, 8, 1);
            let player = ActorRef::player(player_id);
            mind.form_judgement(player, 0.30, 0.35, &Self::context(start, 7));
            for week in 0..16 {
                let date = start + Duration::days(week * 7);
                mind.watched(player, 5.4, false, &Self::context(date, 7));
            }
        }
    }

    #[test]
    fn being_wrong_about_a_player_makes_a_coach_more_patient() {
        let mut mind = StaffMind::new();
        Fixture::wrote_him_off(&mut mind, 21);
        let before = mind.judgement.patience();

        let later = Fixture::context(Fixture::date(2033, 6, 1), 7);
        let verdict = mind.settle_judgement(ActorRef::player(21), 0.92, &later);

        assert_eq!(verdict, Some(JudgementOutcome::Wrong));
        assert!(
            mind.judgement.patience() > before,
            "a coach who has been wrong gives the next one longer"
        );
        assert!(
            mind.believes(FactClaim::IWasWrongAboutHim, ActorRef::player(21)) > 0.0,
            "and he knows it about that player specifically"
        );
    }

    #[test]
    fn being_right_makes_him_quicker_and_surer() {
        let mut mind = StaffMind::new();
        let start = Fixture::date(2030, 8, 1);
        mind.form_judgement(
            ActorRef::player(30),
            0.60,
            0.88,
            &Fixture::context(start, 7),
        );
        for week in 0..16 {
            let date = start + Duration::days(week * 7);
            mind.watched(ActorRef::player(30), 7.4, false, &Fixture::context(date, 7));
        }
        let eye_before = mind.judgement.eye();

        let later = Fixture::context(Fixture::date(2033, 6, 1), 7);
        assert_eq!(
            mind.settle_judgement(ActorRef::player(30), 0.90, &later),
            Some(JudgementOutcome::Vindicated)
        );
        assert!(mind.judgement.eye() > eye_before);
        assert_eq!(mind.judgement.vindicated, 1);
    }

    #[test]
    fn a_judgement_travels_with_the_man() {
        let mut mind = StaffMind::new();
        Fixture::wrote_him_off(&mut mind, 21);
        let held = mind.judgement_of(ActorRef::player(21)).map(|v| v.level());

        mind.on_club_change(7);

        assert_eq!(
            mind.judgement_of(ActorRef::player(21)).map(|v| v.level()),
            held,
            "what he thinks of a player is not the club's opinion"
        );
    }

    #[test]
    fn self_belief_is_allowed_to_outrun_the_record() {
        let mut mind = StaffMind::new();
        assert_eq!(
            mind.judgement.record(),
            None,
            "no record until enough questions have been answered"
        );
        assert_eq!(mind.judgement.overconfidence(), 0.0);

        mind.judgement.vindicated = 1;
        mind.judgement.wrong = 4;
        mind.judgement.eye_pct = 85;
        assert!(
            mind.judgement.overconfidence() > 0.4,
            "a coach whose eye has not earned its confidence is a real coach"
        );
    }

    #[test]
    fn a_coach_who_knows_nobody_has_nothing_to_say_about_the_squad() {
        let mind = StaffMind::new();
        assert!(mind.judgement.appraise(&mind.organs).is_silent());
    }

    #[test]
    fn he_argues_against_signing_a_player_he_does_not_rate() {
        let mut mind = StaffMind::new();
        Fixture::wrote_him_off(&mut mind, 21);

        let against = mind
            .judgement
            .weigh(MindOption::SignThisPlayer(21), &mind.organs);
        assert!(against.net() < 0.0);

        let dropping = mind
            .judgement
            .weigh(MindOption::DropThisPlayer(21), &mind.organs);
        assert!(
            dropping.net() > 0.0,
            "the same opinion points the other way when the question is reversed"
        );
    }
}
