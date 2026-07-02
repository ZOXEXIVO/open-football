use crate::club::player::position::PlayerPositionType;
use crate::club::team::TeamType;

#[derive(Clone)]
pub struct TeamContext {
    pub id: u32,
    pub reputation: f32,
    /// Snapshot of the team's current formation so per-player processing
    /// (role fit, position coverage checks) can reason about its fit
    /// without reaching back into the team object.
    pub formation: Option<[PlayerPositionType; 11]>,
    /// Which squad tier this team is (Main / B / Reserve / Second /
    /// U18..U23). Lets squad-level passes (team behaviour audits) reason
    /// about "life below the first team" without reaching back into the
    /// team object. `None` when the constructing site didn't know it.
    pub team_type: Option<TeamType>,
}

impl TeamContext {
    pub fn new(id: u32) -> Self {
        TeamContext {
            id,
            reputation: 0.0,
            formation: None,
            team_type: None,
        }
    }

    pub fn with_reputation(id: u32, reputation: f32) -> Self {
        TeamContext {
            id,
            reputation,
            formation: None,
            team_type: None,
        }
    }

    pub fn with_formation(mut self, formation: [PlayerPositionType; 11]) -> Self {
        self.formation = Some(formation);
        self
    }

    pub fn with_type(mut self, team_type: TeamType) -> Self {
        self.team_type = Some(team_type);
        self
    }
}
