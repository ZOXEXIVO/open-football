//! **Standalone match types** — the plain data the engine produces and
//! consumes. Nothing here carries `FootballEngine` behaviour; every
//! module is a set of values the simulation passes around, grouped by
//! what it describes.
//!
//! * [`pitch`] — the playing area and the two ends of it: [`BallSide`],
//!   [`MatchFieldSize`], and the [`TeamsTactics`] snapshot taken off the
//!   field.
//! * [`players`] — [`MatchPlayerCollection`], the match's player store,
//!   and the compact [`PlayerEntry`] the hot loops iterate.
//! * [`clock`] — [`MatchTime`] and the period lengths it runs against.
//! * [`outcome`] — what a match hands back: [`MatchEvent`] and
//!   [`PlayMatchStateResult`].
//!
//! Every item is re-exported flat by the parent, so `crate::r#match::<Name>`
//! keeps resolving exactly as it did before the grouping.

pub mod clock;
pub mod outcome;
pub mod pitch;
pub mod players;

pub use clock::{MATCH_EXTRA_TIME_MS, MATCH_HALF_TIME_MS, MATCH_TIME_MS, MatchTime};
pub use outcome::{MatchEvent, PlayMatchStateResult};
pub use pitch::{BallSide, MatchFieldSize, TeamsTactics};
pub use players::{MatchPlayerCollection, PlayerEntry};
