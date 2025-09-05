use crate::r#match::MatchPlayer;
use crate::Tactics;

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
}