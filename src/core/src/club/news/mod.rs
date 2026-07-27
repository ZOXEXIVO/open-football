//! The club press: what a local football paper would print this week.
//!
//! | Module    | Concern                                                     |
//! |-----------|-------------------------------------------------------------|
//! | [`types`] | The printed artefacts — stories, editions, the newsroom      |
//! | [`desk`]  | Detectors that read club state and file candidate stories   |
//! | [`editor`]| Ranks, de-duplicates and lays out one edition                |
//!
//! Stories carry identifiers and numbers only. Names, money formats and
//! translated prose are resolved by the web layer at render time, so an
//! edition costs the same handful of bytes in every language and the
//! whole world's back catalogue stays cheap to keep in memory.

pub mod desk;
pub mod editor;
pub mod types;

pub use desk::{
    BoardroomDesk, CareerRecord, ClubTransferWeek, MarketDesk, MatchDesk, SquadDesk,
    StandingSnapshot, TableDesk, WeeklyMatchFacts,
};
pub use editor::NewsEditor;
pub use types::{
    ClubNewsroom, IssueResult, NewsDesk, NewsRecurrence, NewsStory, NewsStoryKind, NewspaperIssue,
    PressMood,
};
