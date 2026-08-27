mod club;
mod compiled;
mod continent;
pub mod country;
mod data_tree;
mod domestic_cup;
mod league;
mod names;
pub mod national;
pub mod players;

pub use club::*;
pub use continent::*;
pub use country::*;
pub use data_tree::*;
pub use domestic_cup::*;
pub use league::*;
pub use names::*;
pub use national::*;
pub use players::{
    OdbContract, OdbHistoryItem, OdbLoan, OdbPlayer, OdbPosition, OdbReputation, PlayersOdb,
};

/// id -> display name for clubs that only appear in player career history.
/// Built once from the compiled database; empty when the database predates
/// the section, which just restores the old blank-club rendering.
pub fn history_club_names() -> std::collections::HashMap<u32, String> {
    compiled::compiled()
        .history_clubs
        .iter()
        .map(|c| (c.id, c.name.clone()))
        .collect()
}
