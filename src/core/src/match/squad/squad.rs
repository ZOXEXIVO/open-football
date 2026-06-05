use crate::Tactics;
use crate::club::staff::CoachMatchSnapshot;
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
    /// Live-match snapshot of the head coach's persistent state —
    /// memory store, perception profile, and strategy for this
    /// fixture. Populated at squad-construction time by the team
    /// builders so the match engine's substitution layer can build a
    /// [`CoachDecisionEngine`] without reaching back to the league
    /// pipeline. `None` for tests / dev_match harnesses that don't
    /// stand up a real club — the substitution path falls back to
    /// the legacy (memory-less) scoring in that case.
    pub coach_snapshot: Option<CoachMatchSnapshot>,
}
