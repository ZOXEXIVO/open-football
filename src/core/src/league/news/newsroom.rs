use crate::club::news::NewspaperIssue;
use chrono::NaiveDate;
use std::collections::VecDeque;

/// A division's own paper: a monthly review of the league, kept a year
/// deep.
///
/// The club presses run every Monday and keep a long archive, because a
/// supporter goes back looking for the week a signing was announced. A
/// league paper is a different publication with different habits — it
/// comes out once a month, and what it is for is the season it is in.
/// So the shelf is exactly twelve: one edition per month, and the month
/// that pushes a new season's first issue on is the month that pushes
/// the oldest one off. A reader scrolling this tab is always looking at
/// a rolling year of the division, never at a stub and never at an
/// archive nobody asked for.
#[derive(Debug, Clone)]
pub struct LeagueNewsroom {
    /// Which masthead noun this division's paper uses. Stable for the
    /// life of the league so the title never changes under the reader.
    pub masthead: u8,
    /// Number the next edition will carry.
    pub next_number: u32,
    /// Newest edition first.
    pub issues: VecDeque<NewspaperIssue>,
    /// Last month actually put to bed, as the first of that month.
    ///
    /// Its own field rather than a read of `issues.front()`: a month
    /// with nothing in it at all prints nothing, and without this the
    /// tick would try that same empty month again on every later run
    /// and walk every player in the division for nothing.
    pub last_month: Option<NaiveDate>,
}

impl LeagueNewsroom {
    /// Masthead nouns available in the translation bundles. The same
    /// six the clubs use — a division's paper is still a paper, and
    /// "The Serie A Chronicle" is exactly what one is called.
    pub const MASTHEAD_COUNT: u8 = 6;

    /// Editions kept on the shelf: one per month, one year deep.
    ///
    /// How many STORIES an edition holds is not set here — the paper is
    /// compiled by `NewsEditor`, so it is the same page the clubs print
    /// and it is bounded by the same `TeamNewsroom::MAX_STORIES`.
    pub const MAX_ISSUES: usize = 12;

    /// Assign a masthead deterministically from the league id so the
    /// same world always prints the same paper titles.
    pub fn for_league(league_id: u32) -> Self {
        LeagueNewsroom {
            masthead: (league_id.wrapping_mul(2_654_435_761) >> 13) as u8 % Self::MASTHEAD_COUNT,
            next_number: 1,
            issues: VecDeque::new(),
            last_month: None,
        }
    }

    /// File a finished edition, dropping the oldest once the shelf is
    /// full.
    pub fn publish(&mut self, issue: NewspaperIssue) {
        self.issues.push_front(issue);
        while self.issues.len() > Self::MAX_ISSUES {
            self.issues.pop_back();
        }
        self.next_number = self.next_number.saturating_add(1);
    }

    /// Whether this month has already been put to bed — printed or
    /// found to have nothing in it.
    pub fn has_covered(&self, month_start: NaiveDate) -> bool {
        self.last_month.is_some_and(|last| last >= month_start)
    }

    /// Mark a month as dealt with whether or not it produced an issue.
    pub fn close_month(&mut self, month_start: NaiveDate) {
        self.last_month = Some(month_start);
    }

    pub fn latest(&self) -> Option<&NewspaperIssue> {
        self.issues.front()
    }

    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    /// i18n key for this newsroom's masthead pattern.
    pub fn masthead_key(&self) -> &'static str {
        const KEYS: [&str; LeagueNewsroom::MASTHEAD_COUNT as usize] = [
            "masthead_gazette",
            "masthead_chronicle",
            "masthead_herald",
            "masthead_courier",
            "masthead_post",
            "masthead_sentinel",
        ];
        KEYS[(self.masthead as usize) % KEYS.len()]
    }
}

impl Default for LeagueNewsroom {
    fn default() -> Self {
        LeagueNewsroom::for_league(0)
    }
}

#[cfg(test)]
mod tests {
    use super::LeagueNewsroom;
    use crate::club::news::{NewspaperIssue, PressMood};
    use chrono::NaiveDate;

    struct Shelf;

    impl Shelf {
        fn month(year: i32, month: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(year, month, 1).unwrap()
        }

        fn issue(number: u32, date: NaiveDate) -> NewspaperIssue {
            NewspaperIssue {
                number,
                date,
                mood: PressMood::Steady,
                stories: Vec::new(),
                results: Vec::new(),
            }
        }
    }

    /// The shelf is a rolling year. Once it is full, every new month
    /// pushes exactly one older month off the back — which is what
    /// makes a new season replace the last one edition by edition
    /// rather than all at once.
    #[test]
    fn a_thirteenth_month_pushes_the_first_one_off_the_shelf() {
        let mut room = LeagueNewsroom::for_league(11);

        for month in 1..=12 {
            room.publish(Shelf::issue(month, Shelf::month(2026, month)));
        }
        assert_eq!(room.issues.len(), LeagueNewsroom::MAX_ISSUES);
        assert_eq!(room.issues.back().unwrap().date, Shelf::month(2026, 1));

        room.publish(Shelf::issue(13, Shelf::month(2027, 1)));

        assert_eq!(
            room.issues.len(),
            LeagueNewsroom::MAX_ISSUES,
            "the shelf never grows past a year"
        );
        assert_eq!(
            room.issues.front().unwrap().date,
            Shelf::month(2027, 1),
            "newest edition first"
        );
        assert_eq!(
            room.issues.back().unwrap().date,
            Shelf::month(2026, 2),
            "the oldest month came off, and only the oldest"
        );
    }

    /// A month with nothing in it still has to be closed, or the tick
    /// re-walks every player in the division every day for the rest of
    /// the save looking for news that was never there.
    #[test]
    fn an_empty_month_is_closed_without_printing() {
        let mut room = LeagueNewsroom::for_league(3);
        let july = Shelf::month(2026, 7);

        assert!(!room.has_covered(july));
        room.close_month(july);

        assert!(room.has_covered(july));
        assert!(room.is_empty(), "closing a month is not publishing one");
        assert!(
            !room.has_covered(Shelf::month(2026, 8)),
            "closing July says nothing about August"
        );
    }

    /// Two leagues should not share a nameplate by accident, and one
    /// league's nameplate must never move.
    #[test]
    fn a_masthead_is_stable_for_the_life_of_the_league() {
        let first = LeagueNewsroom::for_league(4711);
        let again = LeagueNewsroom::for_league(4711);

        assert_eq!(first.masthead_key(), again.masthead_key());
        assert!(first.masthead_key().starts_with("masthead_"));
    }
}
