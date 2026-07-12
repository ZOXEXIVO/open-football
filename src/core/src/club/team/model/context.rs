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
    /// Official club captain as of the last monthly review
    /// (`Team::captain_id`). Threaded in so captain-centric behaviour
    /// passes (mediation, morale propagation) act through the appointed
    /// armband holder instead of re-electing an ad-hoc leader that can
    /// disagree with the one the club — and the player — sees. `None`
    /// when the constructing site didn't know it, or the team genuinely
    /// has no captain.
    pub captain_id: Option<u32>,
    /// Official vice-captain (`Team::vice_captain_id`); same contract as
    /// [`Self::captain_id`].
    pub vice_captain_id: Option<u32>,
}

impl TeamContext {
    pub fn new(id: u32) -> Self {
        TeamContext {
            id,
            reputation: 0.0,
            formation: None,
            team_type: None,
            captain_id: None,
            vice_captain_id: None,
        }
    }

    pub fn with_reputation(id: u32, reputation: f32) -> Self {
        TeamContext {
            id,
            reputation,
            formation: None,
            team_type: None,
            captain_id: None,
            vice_captain_id: None,
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

    pub fn with_captaincy(
        mut self,
        captain_id: Option<u32>,
        vice_captain_id: Option<u32>,
    ) -> Self {
        self.captain_id = captain_id;
        self.vice_captain_id = vice_captain_id;
        self
    }
}
