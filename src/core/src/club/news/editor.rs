use super::types::{NewsDesk, NewsRecurrence, NewsStory, NewspaperIssue, TeamNewsroom};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

/// Identity the editor uses to decide whether a story is the same story
/// it ran last week: the kind, who it is about (player and, for the
/// dugout, staff), the other party, and — for progressing numbers only
/// — the figure itself.
///
/// The staff slot is load-bearing for the boardroom desk, whose stories
/// carry no player at all: without it, "the club have appointed a new
/// manager" would be the same key whichever man it was about, and a
/// club that changed manager twice inside three editions would print
/// the second appointment as a rerun of the first.
type StoryKey = (&'static str, u32, u32, u32, i32);

/// Turns a week's raw candidates into the edition that actually goes to
/// print: ranked by newsworthiness, one voice per subject, balanced
/// across the desks, and with the themes the paper has already covered
/// held back.
pub struct NewsEditor;

impl NewsEditor {
    /// How many past editions are consulted before a standing theme is
    /// allowed back onto the page. Three weeks is long enough that a
    /// paper never reads like a stuck record, short enough that a real
    /// crisis stays in front of the reader.
    const MEMORY_ISSUES: usize = 3;

    /// The most any single desk may take. Without it, a club with a
    /// twenty-man squad out on loan would print a page of loan briefs
    /// and no football — the rumour and loan desks generate far more
    /// candidates than the match desk ever can, and raw priority alone
    /// would let a quiet week be swamped by speculation.
    const MAX_PER_DESK: usize = 4;

    /// …except the match desk, which is what the paper is for. A club
    /// that played twice, won a cup tie and extended a run should never
    /// have a report cut to make room for a scouting note.
    const MAX_MATCH_STORIES: usize = 6;

    /// …and the charts, which are a table with a fixed number of places
    /// on it. Four is the generic desk allowance and it would print
    /// four fifths of a five-man list, which reads as a page that ran
    /// out of room rather than as a chart. Only the league's own
    /// monthly files here.
    ///
    /// Sized against what a monthly review page actually is. The desk
    /// files five distinct columns — the scorers, the two individual
    /// awards, the providers, the marks and the team of the month — and
    /// they are correctly ranked in that order, so a small allowance
    /// does not trim the page evenly: it prints the top of every column
    /// and none of the rest, which is exactly the "all leaders, no
    /// charts" front page a division's own paper must not be. Nine
    /// leaves room for a leader plus a field behind him.
    const MAX_CHART_PLACES: usize = 9;

    /// At most one player may be the subject of this many stories. A
    /// striker who scored a hat-trick, signed a new deal and was linked
    /// with a move is three separate pieces; four is an obsession.
    const MAX_PER_PLAYER: usize = 3;

    pub fn compile(
        mut candidates: Vec<NewsStory>,
        recent: &VecDeque<NewspaperIssue>,
    ) -> Vec<NewsStory> {
        // Nothing that cannot be set goes any further. A story whose
        // copy quotes a figure it does not carry renders as "$0.00" or
        // "a season average of 0.00" — the single loudest tell that a
        // page was generated rather than written, and a mistake every
        // desk is one missing guard away from making. The check lives
        // here, once, rather than in each detector.
        candidates.retain(Self::is_printable);

        // Highest newsworthiness leads. Ties break on recency, then on
        // stable identifiers so the same world always prints the same
        // page — no ordering is left to hash iteration.
        candidates.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| right.date.cmp(&left.date))
                .then_with(|| left.kind.key_stem().cmp(right.kind.key_stem()))
                .then_with(|| left.player_id.cmp(&right.player_id))
                .then_with(|| left.other_id.cmp(&right.other_id))
                .then_with(|| left.a.cmp(&right.a))
        });

        let printed: FxHashSet<StoryKey> = recent
            .iter()
            .take(Self::MEMORY_ISSUES)
            .flat_map(|issue| issue.stories.iter())
            .filter(|story| story.kind.recurrence() != NewsRecurrence::Event)
            .map(Self::key)
            .collect();

        let mut used: FxHashSet<StoryKey> = FxHashSet::default();
        let mut single_run: FxHashSet<&'static str> = FxHashSet::default();
        let mut per_desk: FxHashMap<NewsDesk, usize> = FxHashMap::default();
        let mut per_player: FxHashMap<u32, usize> = FxHashMap::default();
        let mut edition = Vec::with_capacity(TeamNewsroom::MAX_STORIES);

        for story in candidates {
            if edition.len() >= TeamNewsroom::MAX_STORIES {
                break;
            }

            let key = Self::key(&story);

            if story.kind.recurrence() != NewsRecurrence::Event && printed.contains(&key) {
                continue;
            }
            if used.contains(&key) {
                continue;
            }
            if !story.kind.allows_repeat() && single_run.contains(story.kind.key_stem()) {
                continue;
            }

            let desk = story.desk();
            let desk_used = per_desk.get(&desk).copied().unwrap_or(0);
            if desk_used >= Self::desk_allowance(desk) {
                continue;
            }

            // A match report is the team's story; the scorer it names is
            // a detail of the afternoon, not its subject. It neither
            // counts against his allowance nor gets cut by it — a
            // hat-trick hero must not lose his own piece because the
            // report of the same match already carries his name.
            if story.player_id != 0 && desk != NewsDesk::Match {
                let player_used = per_player.get(&story.player_id).copied().unwrap_or(0);
                if player_used >= Self::MAX_PER_PLAYER {
                    continue;
                }
                per_player.insert(story.player_id, player_used + 1);
            }

            used.insert(key);
            single_run.insert(story.kind.key_stem());
            per_desk.insert(desk, desk_used + 1);
            edition.push(story);
        }

        edition
    }

    /// Whether the numbers behind a story support the sentence its copy
    /// will make of them.
    fn is_printable(story: &NewsStory) -> bool {
        if story.kind.quotes_a_rating() && story.b <= 0 {
            return false;
        }
        if story.kind.quotes_a_fee() && story.money <= 0 {
            return false;
        }
        true
    }

    fn desk_allowance(desk: NewsDesk) -> usize {
        match desk {
            NewsDesk::Match => Self::MAX_MATCH_STORIES,
            NewsDesk::Charts => Self::MAX_CHART_PLACES,
            _ => Self::MAX_PER_DESK,
        }
    }

    fn key(story: &NewsStory) -> StoryKey {
        let figure = match story.kind.recurrence() {
            NewsRecurrence::Progress => story.a,
            NewsRecurrence::Event | NewsRecurrence::Standing => 0,
        };
        (
            story.kind.key_stem(),
            story.player_id,
            story.other_id,
            story.staff_id,
            figure,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::NewsEditor;
    use crate::club::news::types::{
        IssueResult, NewsStory, NewsStoryKind, NewspaperIssue, PressMood, TeamNewsroom,
    };
    use chrono::NaiveDate;
    use std::collections::VecDeque;

    /// Builds the fixtures the editor tests need without leaking loose
    /// helpers into the module.
    struct Desk;

    impl Desk {
        fn day(day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 3, day).unwrap()
        }

        fn story(kind: NewsStoryKind, player_id: u32, a: i32) -> NewsStory {
            NewsStory::new(kind, Self::day(2))
                .about(player_id)
                .with_numbers(a, 0)
        }

        /// A story with the figures its copy will actually quote, so
        /// the editor's well-formedness gate lets it through. Fixtures
        /// that skip this are testing the gate, not the ranking.
        fn printable(kind: NewsStoryKind, player_id: u32, a: i32) -> NewsStory {
            let story = Self::story(kind, player_id, a);
            let story = if kind.quotes_a_rating() {
                story.with_numbers(a, 715)
            } else {
                story
            };
            if kind.quotes_a_fee() {
                story.with_money(4_000_000)
            } else {
                story
            }
        }

        fn back_issue(stories: Vec<NewsStory>) -> VecDeque<NewspaperIssue> {
            let mut issues = VecDeque::new();
            issues.push_front(NewspaperIssue {
                number: 1,
                date: Self::day(1),
                mood: PressMood::Steady,
                stories,
                results: Vec::<IssueResult>::new(),
            });
            issues
        }
    }

    #[test]
    fn leads_with_the_most_newsworthy_story() {
        let candidates = vec![
            Desk::story(NewsStoryKind::InjuryReturn, 1, 0),
            Desk::story(NewsStoryKind::DerbyWin, 0, 2),
            Desk::story(NewsStoryKind::LeagueDraw, 0, 1),
        ];

        let edition = NewsEditor::compile(candidates, &VecDeque::new());

        assert_eq!(edition[0].kind, NewsStoryKind::DerbyWin);
    }

    #[test]
    fn a_standing_theme_waits_its_turn() {
        let recent = Desk::back_issue(vec![Desk::story(NewsStoryKind::MoneyWorries, 0, 130)]);

        let edition = NewsEditor::compile(
            vec![Desk::story(NewsStoryKind::MoneyWorries, 0, 130)],
            &recent,
        );

        assert!(
            edition.is_empty(),
            "a persisting condition must not run two weeks running"
        );
    }

    #[test]
    fn a_moving_number_earns_a_fresh_run() {
        let recent = Desk::back_issue(vec![Desk::story(NewsStoryKind::WinningRun, 0, 3)]);

        let edition =
            NewsEditor::compile(vec![Desk::story(NewsStoryKind::WinningRun, 0, 4)], &recent);

        assert_eq!(edition.len(), 1);
        assert_eq!(edition[0].a, 4);
    }

    #[test]
    fn a_dated_event_is_never_held_back() {
        let recent = Desk::back_issue(vec![Desk::story(NewsStoryKind::HatTrick, 7, 3)]);

        let edition =
            NewsEditor::compile(vec![Desk::story(NewsStoryKind::HatTrick, 7, 3)], &recent);

        assert_eq!(edition.len(), 1);
    }

    #[test]
    fn one_voice_per_kind_unless_the_kind_allows_repeats() {
        let candidates = vec![
            Desk::story(NewsStoryKind::ContractStandoff, 1, 4),
            Desk::story(NewsStoryKind::ContractStandoff, 2, 6),
            Desk::printable(NewsStoryKind::NewSigning, 3, 0),
            Desk::printable(NewsStoryKind::NewSigning, 4, 0),
        ];

        let edition = NewsEditor::compile(candidates, &VecDeque::new());

        let standoffs = edition
            .iter()
            .filter(|s| s.kind == NewsStoryKind::ContractStandoff)
            .count();
        let signings = edition
            .iter()
            .filter(|s| s.kind == NewsStoryKind::NewSigning)
            .count();

        assert_eq!(standoffs, 1);
        assert_eq!(signings, 2);
    }

    #[test]
    fn an_edition_never_overflows_the_page() {
        let candidates: Vec<NewsStory> = (0..40)
            .map(|i| Desk::printable(NewsStoryKind::NewSigning, i, 0))
            .collect();

        let edition = NewsEditor::compile(candidates, &VecDeque::new());

        assert!(edition.len() <= TeamNewsroom::MAX_STORIES);
        assert_eq!(
            edition.len(),
            NewsEditor::MAX_PER_DESK,
            "forty signings is still one desk, and one desk cannot have the whole page"
        );
    }

    /// The failure mode a bigger story vocabulary introduces: the loan
    /// column and the rumour mill can file dozens of candidates on a
    /// week the club barely played, and without a cap they would push
    /// the football off its own front page.
    #[test]
    fn no_single_desk_takes_over_the_paper() {
        let mut candidates: Vec<NewsStory> = (0..12)
            .map(|i| Desk::story(NewsStoryKind::LoanWatchGoals, 100 + i, 4))
            .collect();
        candidates.push(Desk::story(NewsStoryKind::LeagueDefeat, 0, 1));

        let edition = NewsEditor::compile(candidates, &VecDeque::new());

        let loan_stories = edition
            .iter()
            .filter(|story| story.kind == NewsStoryKind::LoanWatchGoals)
            .count();

        assert_eq!(loan_stories, NewsEditor::MAX_PER_DESK);
        assert!(
            edition
                .iter()
                .any(|story| story.kind == NewsStoryKind::LeagueDefeat),
            "the match report must survive a busy loan week"
        );
    }

    /// A match report now carries the afternoon's top scorer, but it is
    /// the team's story — it must neither spend his allowance nor be
    /// cut by it. A hat-trick hero keeps his own piece alongside the
    /// report of the match he scored it in.
    #[test]
    fn a_match_report_does_not_cost_its_star_his_own_news() {
        let candidates = vec![
            Desk::story(NewsStoryKind::LeagueWin, 9, 3).against(40),
            Desk::story(NewsStoryKind::HatTrick, 9, 3),
            Desk::printable(NewsStoryKind::StarForm, 9, 12),
            Desk::story(NewsStoryKind::RumourInterest, 9, 0),
            Desk::story(NewsStoryKind::MilestoneGoals, 9, 100),
        ];

        let edition = NewsEditor::compile(candidates, &VecDeque::new());

        assert!(
            edition
                .iter()
                .any(|story| story.kind == NewsStoryKind::LeagueWin),
            "the match report must survive its star's busy week"
        );
        assert_eq!(
            edition.len(),
            1 + NewsEditor::MAX_PER_PLAYER,
            "the report is on top of his allowance, not part of it"
        );
    }

    /// One player having a big week is a story; one player having four
    /// stories is a page that has stopped being about the club.
    #[test]
    fn no_single_player_takes_over_the_paper() {
        let candidates = vec![
            Desk::story(NewsStoryKind::HatTrick, 9, 3),
            Desk::printable(NewsStoryKind::StarForm, 9, 12),
            Desk::story(NewsStoryKind::RumourInterest, 9, 0),
            Desk::story(NewsStoryKind::ContractStandoff, 9, 5),
            Desk::story(NewsStoryKind::MilestoneGoals, 9, 100),
        ];

        let edition = NewsEditor::compile(candidates, &VecDeque::new());

        assert_eq!(edition.len(), NewsEditor::MAX_PER_PLAYER);
    }

    /// A rating of zero is not a bad rating, it is no rating — the
    /// number a player who has never been on the pitch reads. Copy
    /// built on one ("a season average of 0.00") is the loudest
    /// possible tell that nobody wrote the page, and the same is true
    /// of a fee of nothing. Neither reaches the presses.
    #[test]
    fn a_story_never_quotes_a_figure_it_does_not_have() {
        let candidates = vec![
            Desk::story(NewsStoryKind::RisingStar, 1, 16),
            Desk::story(NewsStoryKind::StarForm, 2, 9),
            Desk::story(NewsStoryKind::LoanWatchStarter, 3, 11),
            Desk::story(NewsStoryKind::NewSigning, 4, 0),
            Desk::story(NewsStoryKind::PlayerSold, 5, 0),
        ];

        assert!(
            NewsEditor::compile(candidates, &VecDeque::new()).is_empty(),
            "every one of these would have printed a 0.00 on the page"
        );
    }

    /// The reported bug, reproduced end to end: a player carrying a
    /// transfer request is re-detected every Monday, so before the fix
    /// the same headline led two consecutive editions. The desks are
    /// allowed to keep filing it — deciding what has already been said
    /// is the editor's job, not theirs.
    #[test]
    fn a_standing_request_does_not_lead_two_weeks_running() {
        let candidate = || Desk::story(NewsStoryKind::TransferRequestFiled, 4, 0);

        let first = NewsEditor::compile(vec![candidate()], &VecDeque::new());
        assert_eq!(first[0].kind, NewsStoryKind::TransferRequestFiled);

        let second = NewsEditor::compile(vec![candidate()], &Desk::back_issue(first));

        assert!(
            second.is_empty(),
            "the same request led the paper twice: {:?}",
            second.iter().map(|s| s.kind).collect::<Vec<_>>()
        );
    }

    /// …and the same kinds print perfectly well once the numbers are
    /// really there, so the gate is not quietly swallowing the desks.
    #[test]
    fn the_same_stories_print_once_the_numbers_are_real() {
        let candidates = vec![
            Desk::printable(NewsStoryKind::RisingStar, 1, 16),
            Desk::printable(NewsStoryKind::NewSigning, 4, 0),
        ];

        assert_eq!(NewsEditor::compile(candidates, &VecDeque::new()).len(), 2);
    }
}
