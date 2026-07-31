//! The club press: what a local football paper would print this week.
//!
//! | Module     | Concern                                                    |
//! |------------|------------------------------------------------------------|
//! | [`types`]  | The printed artefacts — stories, editions, the newsroom     |
//! | [`affairs`]| The club's own diary of dated, institution-level happenings |
//! | [`desk`]   | Detectors that read club state and file candidate stories  |
//! | [`editor`] | Ranks, de-duplicates, balances and lays out one edition     |
//!
//! Stories carry identifiers and numbers only. Names, money formats and
//! translated prose are resolved by the web layer at render time, so an
//! edition costs the same handful of bytes in every language and the
//! whole world's back catalogue stays cheap to keep in memory.
//!
//! A paper belongs to a TEAM, not to a club. Every side that competes
//! under its own brand — the first team, the B team, the "{Club} 2"
//! reserve side — plays its own season in its own league, and a page
//! that reported only the first team's Saturday was no use to anybody
//! reading about the other two. Squads with no brand of their own
//! (Reserve, U18..U23) have no paper and are covered by the first
//! team's, which is also where the club-wide desks (boardroom,
//! accounts, the loan column, the transfer market) file.

pub mod affairs;
pub mod desk;
pub mod editor;
pub mod types;

pub use affairs::{ClubAffair, ClubAffairEntry, ClubAffairLog, ClauseWindfallKind};
pub use desk::{
    Absorbing, BoardroomDesk, CareerRecord, ClubDugoutWatch, ClubLoanWatch, ClubTransferWeek,
    ContinentalNight, CupTie, DugoutDesk, FansDesk, KeeperMatchFacts, LoanDesk, LoanWatchEntry, ManagerPursuit,
    MarketDesk, MatchDesk, MatchDramaFacts, MatchStarFacts, OutfieldMatchFacts, PlayerStanding,
    RecentEvents, RumourDesk, SquadDesk, SquadPulse, StandingSnapshot, TableDesk, TransferMove,
    TransferMotive, TransferMoveKind, TownMood, WeeklyMatchFacts,
};
pub use editor::NewsEditor;
pub use types::{
    IssueResult, NewsDesk, NewsRecurrence, NewsStory, NewsStoryKind, NewspaperIssue, PressMood,
    ResultCompetition, TeamNewsroom,
};
