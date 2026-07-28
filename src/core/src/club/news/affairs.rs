//! The club's own diary — dated things that happened to the institution
//! rather than to one of its players.
//!
//! Everything else the press reads is either a fact it can recompute
//! (the table, a scoreline, a squad list) or an entry in a player's own
//! event feed. The club's week has neither: a sacking leaves behind an
//! empty seat, a takeover leaves behind a different owner, and a
//! stadium expansion leaves behind a bigger number. All three are
//! *states*, and a state cannot tell a paper **when** it changed.
//!
//! That gap produced the system's oldest dugout bug. The only signal
//! the boardroom desk had was the squad-wide `ManagerDeparture` /
//! `NewManagerBounce` pair, fired whenever the head-coach id moved. A
//! sacking moves it twice — once when the caretaker steps up, once when
//! the permanent appointment lands a month later — so the paper ran
//! "part company with the manager" and "new man in the dugout" side by
//! side on both Mondays, and could not have said which of the two
//! actually happened, nor whether he was sacked, poached, or promoted
//! from within.
//!
//! So the club keeps a short log instead. Each entry is written at the
//! moment the thing happens, by the code that does it, and carries the
//! date and the people involved. The desks read the week's entries and
//! nothing else — no state scraping, no inference, and no story that
//! cannot say when it happened.

use crate::club::board::decision::BoardFacility;
use chrono::NaiveDate;
use std::collections::VecDeque;

/// One dated happening in the life of the club.
///
/// Every variant is written from exactly one place in the simulation —
/// the line that performs the act — so a variant existing in this enum
/// means the paper can genuinely report it. Nothing here is inferred
/// from a persistent field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClubAffair {
    /// The board dismissed the head coach.
    ManagerSacked { staff_id: u32 },
    /// Another club came for him, paid the compensation, and got him.
    ManagerPoached { staff_id: u32, to_club_id: u32 },
    /// A permanent appointment. `from_club_id` is `0` when the new man
    /// was out of work — the ordinary case, and a different story from
    /// prising him out of a job he already had.
    ManagerAppointed { staff_id: u32, from_club_id: u32 },
    /// Somebody is minding the shop until the search concludes.
    CaretakerAppointed { staff_id: u32 },
    /// The interim got the job for keeps.
    CaretakerConfirmed { staff_id: u32 },
    /// A new deal for the man already in post.
    ManagerContractExtended { staff_id: u32 },
    /// The board went public with the final warning. The one stage of
    /// the sacking ladder a supporter gets to live through in advance.
    ManagerUltimatum { staff_id: u32 },
    /// Somebody is circling the club's ownership.
    TakeoverRumour,
    /// The sale went through, and the money that came with it.
    TakeoverCompleted,
    /// The sale fell over. Whoever was waiting on it is still waiting.
    TakeoverCollapsed,
    /// The ground itself gets bigger. Carries the new capacity.
    StadiumExpansion { capacity: u32 },
    /// Bricks and mortar behind the scenes — training pitches, the
    /// academy building, the scouting network.
    FacilityUpgrade { facility: BoardFacility },
    /// Money put on the table for the manager to spend.
    WarChest { amount: i64 },
    /// Money taken back off it.
    BudgetCut { amount: i64 },
    /// A promise the board made to the manager and did not keep.
    PromiseBroken,
}

impl ClubAffair {
    /// The staff member this affair is about, when it is about one.
    pub fn staff_id(self) -> u32 {
        match self {
            ClubAffair::ManagerSacked { staff_id }
            | ClubAffair::ManagerPoached { staff_id, .. }
            | ClubAffair::ManagerAppointed { staff_id, .. }
            | ClubAffair::CaretakerAppointed { staff_id }
            | ClubAffair::CaretakerConfirmed { staff_id }
            | ClubAffair::ManagerContractExtended { staff_id }
            | ClubAffair::ManagerUltimatum { staff_id } => staff_id,
            _ => 0,
        }
    }

    /// True when this entry changed who is picking the team. The
    /// boardroom desk holds its speculation about the manager's future
    /// on any week that does — a piece about whether he will survive is
    /// not a piece worth printing beside the news that he did not.
    pub fn is_dugout_change(self) -> bool {
        matches!(
            self,
            ClubAffair::ManagerSacked { .. }
                | ClubAffair::ManagerPoached { .. }
                | ClubAffair::ManagerAppointed { .. }
                | ClubAffair::CaretakerAppointed { .. }
                | ClubAffair::CaretakerConfirmed { .. }
        )
    }
}

/// A happening with the day it happened on.
#[derive(Debug, Clone, Copy)]
pub struct ClubAffairEntry {
    pub date: NaiveDate,
    pub affair: ClubAffair,
}

/// The club's short diary. Bounded, because every club in the world
/// keeps one and nothing reads further back than the last press run.
#[derive(Debug, Clone, Default)]
pub struct ClubAffairLog {
    entries: VecDeque<ClubAffairEntry>,
}

impl ClubAffairLog {
    /// Roughly a season's worth of boardroom drama at a club having a
    /// bad one. The press only ever reads the last seven days; the rest
    /// is kept so a future page (a board history tab) has somewhere to
    /// read from without another plumbing pass.
    const MAX_ENTRIES: usize = 24;

    pub fn new() -> Self {
        Self::default()
    }

    /// File a happening. Oldest entries fall off the back.
    pub fn record(&mut self, affair: ClubAffair, date: NaiveDate) {
        if self.entries.len() >= Self::MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(ClubAffairEntry { date, affair });
    }

    /// Everything filed inside `[start, end)`, oldest first.
    ///
    /// Half-open, the same window contract the transfer ledger and the
    /// match log are read through: an edition covers the seven days
    /// behind it and nothing else. Without the upper bound a press run
    /// would pull in whatever landed on the morning it went out, and
    /// that day's news belongs to next week's paper.
    pub fn between(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> impl Iterator<Item = &ClubAffairEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.date >= start && entry.date < end)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ClubAffairEntry> {
        self.entries.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, d).unwrap()
    }

    /// An edition covers the seven days behind it and nothing else: not
    /// the fortnight before it, and not the morning it goes to press.
    #[test]
    fn the_log_reads_back_exactly_one_week() {
        let mut log = ClubAffairLog::new();
        log.record(ClubAffair::ManagerSacked { staff_id: 7 }, day(1));
        log.record(ClubAffair::TakeoverRumour, day(9));
        log.record(ClubAffair::TakeoverCompleted, day(15));

        let week: Vec<ClubAffair> = log.between(day(8), day(15)).map(|e| e.affair).collect();
        assert_eq!(
            week,
            vec![ClubAffair::TakeoverRumour],
            "a sacking a fortnight ago is old news, and press day itself belongs to the next edition"
        );
    }

    #[test]
    fn the_log_stays_bounded() {
        let mut log = ClubAffairLog::new();
        for _ in 0..(ClubAffairLog::MAX_ENTRIES + 10) {
            log.record(ClubAffair::TakeoverRumour, day(1));
        }
        assert_eq!(log.len(), ClubAffairLog::MAX_ENTRIES);
    }

    /// Every club in the world carries one of these, so an unbounded
    /// diary would be an unbounded leak — and the dugout variants have
    /// to be able to name the man they are about.
    #[test]
    fn a_dugout_entry_knows_who_it_is_about() {
        assert_eq!(
            ClubAffair::ManagerPoached {
                staff_id: 42,
                to_club_id: 9
            }
            .staff_id(),
            42
        );
        assert_eq!(ClubAffair::TakeoverCompleted.staff_id(), 0);
        assert!(ClubAffair::CaretakerAppointed { staff_id: 3 }.is_dugout_change());
        assert!(!ClubAffair::ManagerUltimatum { staff_id: 3 }.is_dugout_change());
    }
}
