use super::facts::{ClubLoanWatch, LoanWatchEntry};
use crate::club::news::types::{NewsStory, NewsStoryKind};
use chrono::NaiveDate;

/// The loan column — how the club's players away on loan are getting
/// on, and what they say about coming home.
///
/// Loaned-out players do not live on the parent club's roster, so
/// without a gathered [`ClubLoanWatch`] the paper simply stops mentioning
/// a third of the professional squad the day it leaves the building.
/// That silence is what makes a generated page feel thin, and it is the
/// gap this desk exists to close.
pub struct LoanDesk;

impl LoanDesk {
    /// Below this many appearances there is nothing to write yet — a
    /// loanee who has been at his new club a fortnight has no record.
    const MIN_APPEARANCES_FOR_FORM: i32 = 3;
    /// Goals at a borrowing club start to matter from here.
    const GOALS_WORTH_A_LINE: i32 = 3;
    /// The same line, when one of them landed in the last fortnight. A
    /// live run is worth printing earlier than a cold one — but every
    /// phrasing of this story is about repetition ("the goals keep
    /// coming"), so it still takes more than one. A single goal, however
    /// recent, is a result rather than a run.
    const GOALS_WORTH_A_LINE_ON_A_RUN: i32 = 2;
    /// Calling a spell a failure needs a bigger sample than describing
    /// it does — a verdict is harder to take back than a scoreline.
    const MIN_APPEARANCES_FOR_VERDICT: i32 = 6;
    /// The positional-neutral rating × 100. How far a struggling loanee
    /// sits below it is how loudly the piece runs.
    const WATER_LINE: i32 = 680;
    /// How close the agreement has to be to running out before the
    /// paper starts asking what happens next.
    const SPELL_ENDING_DAYS: i32 = 45;

    pub fn file(out: &mut Vec<NewsStory>, watch: &ClubLoanWatch, date: NaiveDate) {
        for entry in &watch.players {
            Self::file_player(out, entry, date);
        }
    }

    /// One loanee, at most two lines: how the spell is going, and — if
    /// he or the club has said anything about it — what he wants next.
    fn file_player(out: &mut Vec<NewsStory>, entry: &LoanWatchEntry, date: NaiveDate) {
        let spoke = Self::file_intentions(out, entry, date);
        if Self::file_verdict(out, entry, date) {
            return;
        }
        Self::file_form(out, entry, date, spoke);
        Self::file_deadline(out, entry, date, spoke);
    }

    /// The loan that is not working — the other half of the column, and
    /// the half a parent club reads first. Two ways for a spell to fail
    /// with the player on the pitch every week: he is playing badly, or
    /// he was sent somewhere he cannot cope with. Both are distinct
    /// from the benched case, which is a failure of minutes rather than
    /// of the move itself.
    ///
    /// Returns whether a verdict was filed — it replaces the ordinary
    /// watch line rather than running alongside it.
    fn file_verdict(out: &mut Vec<NewsStory>, entry: &LoanWatchEntry, date: NaiveDate) -> bool {
        // A verdict needs a sample and a rating behind it: "an average
        // of 0.00" is what a player who has never been rated reads.
        if entry.appearances() < Self::MIN_APPEARANCES_FOR_VERDICT || entry.rating_x100 <= 0 {
            return false;
        }

        if entry.form_concern {
            out.push(
                NewsStory::new(NewsStoryKind::LoanFlop, date)
                    .about(entry.player_id)
                    .against(entry.loan_club_id)
                    .with_numbers(entry.appearances(), entry.rating_x100)
                    .weighted((Self::WATER_LINE - entry.rating_x100).max(0) / 2),
            );
            return true;
        }

        if entry.level_mismatch {
            out.push(
                NewsStory::new(NewsStoryKind::LoanStepTooBig, date)
                    .about(entry.player_id)
                    .against(entry.loan_club_id)
                    .with_numbers(entry.appearances(), entry.starts),
            );
            return true;
        }

        false
    }

    /// The interview. A loanee who cannot get on the pitch says he wants
    /// to come back and fight for his place; one who is playing every
    /// week says he wants to stay. Both are the oldest quotes in
    /// football, and both are earned here rather than rolled for: the
    /// player only speaks when his record gives him something to say.
    ///
    /// Returns whether a quote was filed, so the factual lines can stand
    /// down rather than repeat it in flatter language.
    fn file_intentions(out: &mut Vec<NewsStory>, entry: &LoanWatchEntry, date: NaiveDate) -> bool {
        // The one the parent club's supporters actually argue about: a
        // player they keep lending out, playing every week somewhere
        // else, saying he has done his time and wants a pre-season at
        // home. It leads the column over the ordinary "get me out of
        // here" line — that one is about the borrowing club, this one is
        // about theirs — and over the permanence longing, because a
        // player can carry both moods and this is the later word.
        if entry.wants_to_prove_himself {
            out.push(
                NewsStory::new(NewsStoryKind::LoanFedUp, date)
                    .about(entry.player_id)
                    .against(entry.loan_club_id)
                    .with_numbers(entry.starts, entry.goals)
                    .weighted(entry.starts * 3),
            );
            return true;
        }

        // Thriving, and the parent club's bench is not where he wants to
        // be in the summer.
        if entry.wants_permanent && entry.is_first_choice() {
            out.push(
                NewsStory::new(NewsStoryKind::LoanWantsPermanent, date)
                    .about(entry.player_id)
                    .against(entry.loan_club_id)
                    .with_numbers(entry.appearances(), entry.goals)
                    .weighted(if entry.permanent_option { 40 } else { 0 }),
            );
            return true;
        }

        // Not playing, and old enough — or well thought-of enough — to
        // say so out loud. The parent club's own concern counts as the
        // same story: somebody is briefing, and it reaches print either
        // way.
        let stalled = entry.is_frozen_out() || entry.development_concern || entry.level_mismatch;
        if stalled && (entry.recall_requested || entry.is_prospect) {
            out.push(
                NewsStory::new(NewsStoryKind::LoanWantsReturn, date)
                    .about(entry.player_id)
                    .against(entry.loan_club_id)
                    .with_numbers(entry.appearances(), entry.expected_appearances())
                    .weighted(if entry.recall_requested { 50 } else { 0 }),
            );
            return true;
        }

        // The club talking rather than the player: a recall is on the
        // table but nobody has committed to it.
        if entry.recall_requested || entry.form_concern {
            out.push(
                NewsStory::new(NewsStoryKind::LoanRecallTalk, date)
                    .about(entry.player_id)
                    .against(entry.loan_club_id)
                    .with_numbers(entry.appearances(), entry.rating_x100)
                    .weighted(if entry.recall_available { 30 } else { 0 }),
            );
            return true;
        }

        false
    }

    /// The loan-watch line proper: the numbers a supporter checks on a
    /// Sunday to see whether the kid is playing.
    fn file_form(out: &mut Vec<NewsStory>, entry: &LoanWatchEntry, date: NaiveDate, spoke: bool) {
        if entry.appearances() < Self::MIN_APPEARANCES_FOR_FORM {
            return;
        }

        // Goals travel. A loanee scoring at his borrowing club is the
        // one loan story that reaches the front half of the paper, and
        // it runs whether or not he has also given an interview.
        //
        // Both bars read the tally, never the recent-goal flag alone:
        // the headline the reader gets may be the one phrasing that
        // names no figure ("keeps scoring at Danubio"), so a story filed
        // on nothing does not even print the 0 that would give it away.
        if entry.goals >= Self::GOALS_WORTH_A_LINE
            || (entry.scored_recently && entry.goals >= Self::GOALS_WORTH_A_LINE_ON_A_RUN)
        {
            out.push(
                NewsStory::new(NewsStoryKind::LoanWatchGoals, date)
                    .about(entry.player_id)
                    .against(entry.loan_club_id)
                    .with_numbers(entry.goals, entry.appearances())
                    .weighted(entry.goals * 12),
            );
            return;
        }

        if spoke {
            return;
        }

        if entry.is_first_choice() {
            out.push(
                NewsStory::new(NewsStoryKind::LoanWatchStarter, date)
                    .about(entry.player_id)
                    .against(entry.loan_club_id)
                    .with_numbers(entry.starts, entry.rating_x100)
                    .weighted(entry.starts * 4),
            );
            return;
        }

        if entry.is_frozen_out() {
            out.push(
                NewsStory::new(NewsStoryKind::LoanWatchBenched, date)
                    .about(entry.player_id)
                    .against(entry.loan_club_id)
                    .with_numbers(entry.appearances(), entry.expected_appearances()),
            );
        }
    }

    /// What happens in the summer. Only worth a line once the date is
    /// close enough that somebody has to decide.
    fn file_deadline(
        out: &mut Vec<NewsStory>,
        entry: &LoanWatchEntry,
        date: NaiveDate,
        spoke: bool,
    ) {
        if spoke || entry.days_left <= 0 || entry.days_left > Self::SPELL_ENDING_DAYS {
            return;
        }

        out.push(
            NewsStory::new(NewsStoryKind::LoanSpellEnds, date)
                .about(entry.player_id)
                .against(entry.loan_club_id)
                .with_numbers(entry.days_left, entry.appearances())
                .weighted(if entry.permanent_option { 40 } else { 0 }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::LoanDesk;
    use crate::club::news::desk::facts::{ClubLoanWatch, LoanWatchEntry};
    use crate::club::news::types::{NewsStory, NewsStoryKind};
    use chrono::NaiveDate;

    struct Spell;

    impl Spell {
        fn day() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 3, 2).unwrap()
        }

        /// A loanee 100 days into a spell, with the appearance record
        /// the caller wants to test.
        fn entry(starts: i32, subs: i32, goals: i32) -> LoanWatchEntry {
            LoanWatchEntry {
                player_id: 7,
                loan_club_id: 90,
                starts,
                sub_appearances: subs,
                goals,
                assists: 0,
                rating_x100: 690,
                days_left: 120,
                days_elapsed: 98,
                recall_available: false,
                permanent_option: false,
                is_prospect: true,
                wants_permanent: false,
                wants_to_prove_himself: false,
                recall_requested: false,
                development_concern: false,
                form_concern: false,
                level_mismatch: false,
                scored_recently: false,
            }
        }

        fn file(entry: LoanWatchEntry) -> Vec<NewsStory> {
            let mut out = Vec::new();
            LoanDesk::file(
                &mut out,
                &ClubLoanWatch {
                    players: vec![entry],
                },
                Self::day(),
            );
            out
        }

        fn kinds(stories: &[NewsStory]) -> Vec<NewsStoryKind> {
            stories.iter().map(|story| story.kind).collect()
        }
    }

    #[test]
    fn a_loanee_who_cannot_get_a_game_asks_to_come_home() {
        // 14 fixtures' worth of spell, two appearances.
        let stories = Spell::file(Spell::entry(1, 1, 0));

        assert!(
            Spell::kinds(&stories).contains(&NewsStoryKind::LoanWantsReturn),
            "a frozen-out prospect is the loan column's whole reason to exist: {:?}",
            Spell::kinds(&stories)
        );
    }

    #[test]
    fn a_loanee_playing_every_week_wants_the_move_made_permanent() {
        let mut entry = Spell::entry(12, 0, 1);
        entry.wants_permanent = true;

        assert_eq!(
            Spell::kinds(&Spell::file(entry)),
            vec![NewsStoryKind::LoanWantsPermanent]
        );
    }

    /// The carousel line outranks the permanence one when a player is
    /// carrying both: he is thriving, and he still wants his own club's
    /// pre-season. Printing "he wants to stay" that week would report
    /// the older half of the story.
    #[test]
    fn a_loanee_who_has_had_enough_of_loans_says_so_first() {
        let mut entry = Spell::entry(14, 0, 3);
        entry.wants_permanent = true;
        entry.wants_to_prove_himself = true;

        let kinds = Spell::kinds(&Spell::file(entry));

        assert!(kinds.contains(&NewsStoryKind::LoanFedUp));
        assert!(
            !kinds.contains(&NewsStoryKind::LoanWantsPermanent),
            "a player only gives one interview a week: {:?}",
            kinds
        );
    }

    #[test]
    fn goals_at_the_borrowing_club_always_reach_the_page() {
        let mut entry = Spell::entry(11, 1, 6);
        entry.wants_permanent = true;

        let kinds = Spell::kinds(&Spell::file(entry));

        assert!(kinds.contains(&NewsStoryKind::LoanWantsPermanent));
        assert!(
            kinds.contains(&NewsStoryKind::LoanWatchGoals),
            "six goals on loan is news even when he has already spoken"
        );
    }

    /// The line that says a man is scoring is filed off the tally and
    /// nothing else.
    ///
    /// `scored_recently` is read from a happiness event that fires on an
    /// assist as readily as on a goal, and the last pass before a winner
    /// is quite often the goalkeeper's kick. A borrowed keeper therefore
    /// arrived at this desk flagged, and the column printed "keeps
    /// scoring at Danubio" about a man with no goals in his career —
    /// under the one phrasing that names no figure, so the story did not
    /// even print the 0 that would have shown the reader what happened.
    #[test]
    fn a_loanee_who_has_not_scored_is_never_said_to_be_scoring() {
        let mut keeper = Spell::entry(12, 0, 0);
        keeper.scored_recently = true;

        let kinds = Spell::kinds(&Spell::file(keeper));

        assert!(
            !kinds.contains(&NewsStoryKind::LoanWatchGoals),
            "a goalless loanee was reported as scoring: {:?}",
            kinds
        );
        assert!(
            kinds.contains(&NewsStoryKind::LoanWatchStarter),
            "he is still playing every week, and that is the true story: {:?}",
            kinds
        );
    }

    /// One goal is a result; the copy on this line is about a run. Every
    /// phrasing of it says so — "the goals keep coming", "keeps scoring"
    /// — and the numbers are set plural, so a tally of one would print
    /// "1 goals" underneath a headline claiming a streak.
    #[test]
    fn a_single_goal_is_not_yet_a_scoring_run() {
        let mut once = Spell::entry(12, 0, 1);
        once.scored_recently = true;
        assert!(!Spell::kinds(&Spell::file(once)).contains(&NewsStoryKind::LoanWatchGoals));

        let mut twice = Spell::entry(12, 0, 2);
        twice.scored_recently = true;
        assert!(
            Spell::kinds(&Spell::file(twice)).contains(&NewsStoryKind::LoanWatchGoals),
            "a live run of two is what the early bar is for"
        );
    }

    #[test]
    fn the_column_says_nothing_about_a_spell_that_has_barely_started() {
        let mut entry = Spell::entry(1, 0, 0);
        entry.days_elapsed = 9;
        entry.is_prospect = true;

        assert!(
            Spell::file(entry).is_empty(),
            "nine days is not a loan record, and inventing one is exactly the filler to avoid"
        );
    }

    #[test]
    fn an_expiring_agreement_is_only_news_once_it_is_close() {
        let mut early = Spell::entry(6, 2, 0);
        early.days_left = 200;
        assert!(!Spell::kinds(&Spell::file(early)).contains(&NewsStoryKind::LoanSpellEnds));

        let mut late = Spell::entry(6, 2, 0);
        late.days_left = 20;
        assert!(Spell::kinds(&Spell::file(late)).contains(&NewsStoryKind::LoanSpellEnds));
    }
}
