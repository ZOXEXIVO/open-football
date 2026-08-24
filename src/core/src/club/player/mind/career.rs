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
use super::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin, GoalStatus};
use super::organs::memory::{ActorRef, EpisodeKind, FactClaim, MindEpisode};
use super::situation::MindSituation;
use super::submind::{MindOption, MindView, MoodContribution, ReasonSet, SubMind};

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
    /// The old rule gated the whole branch on ambition ≥ 12 and then
    /// chose by club size, so a loyal, ambitious club man and a
    /// rootless one produced identical wants, and everybody below the
    /// bar produced none at all. Football has at least three answers to
    /// having been somewhere a long time, and which one a player gives
    /// is the most legible thing about his character.
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
        // A settled man plainly bigger than his club wants a bigger one;
        // one who is not wants a new test. The same restlessness,
        // pointed by where he already is — but pointed by what he has
        // actually become rather than by the club's badge.
        let goal = if outgrown > 0.2 {
            GoalKind::StepUpToABiggerClub
        } else {
            GoalKind::FindANewChallenge
        };
        // Loyalty is the brake. A man who wants to stay still feels the
        // ceiling; he simply feels it less, and it takes him longer to
        // act on it.
        let restlessness = drive * 0.5 * (1.0 - s.loyalty_drive() * 0.6);
        organs
            .goals
            .pursue(goal, GoalOrigin::SelfDrive, evidence, restlessness, today);

        // Time running out sharpens it.
        organs.goals.set_urgency(goal, s.career_spent());
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
                organs.goals.advance(GoalKind::GoOutOnLoan, 0.3);
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
