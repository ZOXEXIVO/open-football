use crate::Tactics;
use crate::r#match::MatchPlayer;
use crate::r#match::squad::OmittedPlayer;

#[derive(Debug, Clone)]
pub struct MatchSquad {
    pub team_id: u32,
    pub team_name: String,
    pub tactics: Tactics,
    pub main_squad: Vec<MatchPlayer>,
    pub substitutes: Vec<MatchPlayer>,
    pub captain_id: Option<MatchPlayer>,
    pub vice_captain_id: Option<MatchPlayer>,
    pub penalty_taker_id: Option<MatchPlayer>,
    pub free_kick_taker_id: Option<MatchPlayer>,
    /// Important omissions surfaced by the squad selector, with the
    /// structured context the player-events feed needs to explain who
    /// the manager picked instead and why. Empty for rotation /
    /// friendly squads, or when nothing notable happened.
    pub selection_omissions: Vec<OmittedPlayer>,
}
