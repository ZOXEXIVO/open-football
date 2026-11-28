//! The club press: what a local football paper would print this week.
//!
//! | Module     | Concern                                                    |
//! |------------|------------------------------------------------------------|
//! | [`types`]  | The printed artefacts — stories, editions, the newsroom     |
//! | [`affairs`]| The club's own diary of dated, institution-level happenings |
//! | [`desk`]   | Detectors that read club state and file candidate stories  |
//! | [`editor`] | Ranks, de-duplicates, balances and lays out one edition     |
//!
//! Stories carry the facts known at publication. The web layer resolves
//! display names, money formats and translated prose at render time.
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
mod angle;
pub mod desk;
pub mod editor;
pub(crate) mod sightings;
pub mod types;

pub use affairs::{ClauseWindfallKind, ClubAffair, ClubAffairEntry, ClubAffairLog};
pub use angle::NewsAngle;
pub use desk::{
    Absorbing, BoardroomDesk, CareerRecord, ClubDugoutWatch, ClubLoanWatch, ClubTargetsWeek,
    ClubTransferWeek, ContinentalNight, CupTie, DugoutDesk, FansDesk, KeeperMatchFacts, LoanDesk,
    LoanWatchEntry, ManagerPursuit, MarketDesk, MatchDesk, MatchDramaFacts, MatchStarFacts,
    NextFixture, OutfieldMatchFacts, PlayerStanding, PlayoffTie, PreviewDesk, PursuitStage,
    RecentEvents, RumourDesk, SquadDesk, SquadPulse, StandingSnapshot, TableDesk, TargetPursuit,
    TargetsDesk, TownMood, TransferMotive, TransferMove, TransferMoveKind, WeeklyMatchFacts,
    WindowWeek,
};
pub use editor::NewsEditor;
pub use sightings::{ScoutSighting, ScoutVisit};
pub use types::{
    IssueResult, MatchPerformance, NewsDesk, NewsRecurrence, NewsStory, NewsStoryDetail,
    NewsStoryKind, NewspaperIssue, PlayingTimeConcern, PressMood, ResultCompetition, TeamNewsroom,
};
