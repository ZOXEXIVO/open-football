use super::types::{ClubNewsroom, NewsRecurrence, NewsStory, NewspaperIssue};
use rustc_hash::FxHashSet;
use std::collections::VecDeque;

/// Identity the editor uses to decide whether a story is the same story
/// it ran last week: the kind, who it is about, the other party, and —
/// for progressing numbers only — the figure itself.
type StoryKey = (&'static str, u32, u32, i32);

/// Turns a week's raw candidates into the edition that actually goes to
/// print: ranked by newsworthiness, one voice per subject, and with the
/// themes the paper has already covered held back.
pub struct NewsEditor;

impl NewsEditor {
    /// How many past editions are consulted before a standing theme is
    /// allowed back onto the page. Three weeks is long enough that a
    /// paper never reads like a stuck record, short enough that a real
    /// crisis stays in front of the reader.
    const MEMORY_ISSUES: usize = 3;

    pub fn compile(
        mut candidates: Vec<NewsStory>,
        recent: &VecDeque<NewspaperIssue>,
    ) -> Vec<NewsStory> {
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
        let mut edition = Vec::with_capacity(ClubNewsroom::MAX_STORIES);

        for story in candidates {
            if edition.len() >= ClubNewsroom::MAX_STORIES {
                break;
            }

            let key = Self::key(&story);

            if story.kind.recurrence() != NewsRecurrence::Event && printed.contains(&key) {
                continue;
            }
            if !used.insert(key) {
                continue;
            }
            if !story.kind.allows_repeat() && !single_run.insert(story.kind.key_stem()) {
                continue;
            }

            edition.push(story);
        }

        edition
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
            figure,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::NewsEditor;
    use crate::club::news::types::{
        ClubNewsroom, IssueResult, NewsStory, NewsStoryKind, NewspaperIssue, PressMood,
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
            Desk::story(NewsStoryKind::NewSigning, 3, 0),
            Desk::story(NewsStoryKind::NewSigning, 4, 0),
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
            .map(|i| Desk::story(NewsStoryKind::NewSigning, i, 0))
            .collect();

        let edition = NewsEditor::compile(candidates, &VecDeque::new());

        assert_eq!(edition.len(), ClubNewsroom::MAX_STORIES);
    }
}
