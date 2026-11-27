pub mod selection;
pub(crate) mod squad;

// Squad construction and the selector surface the club domain calls directly.
// The selection submodules stay public because the development harnesses and
// the club matchday code name them (`selection::helpers`, `selection::model`).
pub use selection::{
    CoachStrategyForSelection, FixtureRoom, OmittedPlayer, PlayerSelectionResult,
    SelectionCompetition, SelectionContext, SelectionReading, SquadSelector,
};
pub use squad::MatchSquad;
