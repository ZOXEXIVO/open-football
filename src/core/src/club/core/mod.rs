//! The club itself: the aggregate that owns the teams, the board, the books,
//! the academy and the ground, plus the daily tick that drives them.
//!
//! The sibling modules under [`crate::club`] each own one domain and are
//! reachable on their own — the squad-asset classifier, the finance ledger,
//! the board's decision machinery. What lives here is the club *acting*: the
//! passes that need more than one of those domains in hand at once, which is
//! why they cannot live in any single one of them.

mod academy;
mod boardroom;
mod club;
mod squad;
mod tick;
mod treasury;

pub use boardroom::LeagueStanding;
pub use club::{Club, ClubColors, ClubPhilosophy};
pub use treasury::WageReliefSale;
